use super::install_smoke_release::{Failure, SanitizedOutcome, VerifiedMacOsUpdaterArtifact};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use flate2::bufread::GzDecoder;
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tar::{Archive, EntryType};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const APP_NAME: &str = "BatCave Monitor.app";
const TAURI_CONFIG: &str = include_str!("../../../tauri.conf.json");
const COPY_CHUNK_BYTES: usize = 64 * 1024;
const CLEANUP_RETRY_LIMIT: usize = 3;

#[derive(Clone, Copy)]
struct Limits {
    max_compressed_bytes: usize,
    max_decompressed_tar_bytes: usize,
    max_member_count: usize,
    max_path_depth: usize,
    max_path_bytes: usize,
    max_path_bookkeeping_bytes: usize,
    max_canonical_prefixes: usize,
    max_file_bytes: u64,
    max_expanded_bytes: u64,
}

impl Limits {
    const fn production() -> Self {
        Self {
            max_compressed_bytes: 256 * 1024 * 1024,
            max_decompressed_tar_bytes: 1152 * 1024 * 1024,
            max_member_count: 50_000,
            max_path_depth: 64,
            max_path_bytes: 4_096,
            max_path_bookkeeping_bytes: 32 * 1024 * 1024,
            max_canonical_prefixes: 100_000,
            max_file_bytes: 256 * 1024 * 1024,
            max_expanded_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageFailure {
    Signature,
    Preflight,
    Materialization,
    Verification,
    Cleanup,
}

#[derive(Debug)]
struct StageError {
    primary: Option<StageFailure>,
    cleanup: Option<StageFailure>,
    retained_root: Option<PrivateRoot>,
}

impl StageError {
    fn primary(primary: StageFailure) -> Self {
        Self {
            primary: Some(primary),
            cleanup: None,
            retained_root: None,
        }
    }

    fn retained(primary: Option<StageFailure>, cleanup: StageFailure, root: PrivateRoot) -> Self {
        Self {
            primary,
            cleanup: Some(cleanup),
            retained_root: Some(root),
        }
    }

    fn residue_retained(&self) -> bool {
        self.retained_root.is_some()
    }

    fn retry_cleanup(&mut self) -> Result<(), StageFailure> {
        let Some(root) = self.retained_root.as_mut() else {
            return Ok(());
        };
        match root.cleanup() {
            Ok(()) => {
                self.retained_root.take();
                Ok(())
            }
            Err(cleanup) => {
                self.cleanup = Some(cleanup);
                Err(cleanup)
            }
        }
    }

    fn retry_cleanup_bounded(&mut self) -> Result<(), StageFailure> {
        for _ in 0..CLEANUP_RETRY_LIMIT {
            match self.retry_cleanup() {
                Ok(()) => return Ok(()),
                Err(StageFailure::Cleanup) => {}
                Err(other) => return Err(other),
            }
        }
        Err(StageFailure::Cleanup)
    }

    fn into_public_failure(mut self) -> Failure {
        if self.residue_retained() {
            let _ = self.retry_cleanup_bounded();
        }
        match (self.primary, self.cleanup) {
            (Some(StageFailure::Materialization), Some(StageFailure::Cleanup)) => {
                Failure::MaterializationAndCleanupFailed
            }
            (Some(StageFailure::Verification), Some(StageFailure::Cleanup)) => {
                Failure::VerificationAndCleanupFailed
            }
            (_, Some(StageFailure::Cleanup)) => Failure::CleanupFailed,
            _ => Failure::AuthorityRejected,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecordKind {
    Directory,
    File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArchiveRecord {
    parts: Vec<String>,
    kind: RecordKind,
    size: u64,
    mode: u32,
    digest: Option<[u8; 32]>,
}

struct Preflight {
    records: Vec<ArchiveRecord>,
    expected_paths: BTreeMap<Vec<String>, RecordKind>,
}

struct CanonicalEntry {
    original: Vec<String>,
    kind: RecordKind,
    explicit: bool,
}

pub(super) fn run(artifact: VerifiedMacOsUpdaterArtifact) -> Result<SanitizedOutcome, Failure> {
    artifact.revalidate()?;
    verify_updater_signature(artifact.bytes.as_ref(), &artifact.signature)
        .map_err(|_| Failure::AuthorityRejected)?;
    let member_count = stage_and_cleanup(artifact.bytes.as_ref(), Limits::production())
        .map_err(StageError::into_public_failure)?;
    Ok(SanitizedOutcome::macos_updater_observed(
        &artifact.identity,
        member_count,
    ))
}

fn verify_updater_signature(archive: &[u8], wrapped_signature: &[u8]) -> Result<(), StageFailure> {
    let config: Value = serde_json::from_str(TAURI_CONFIG).map_err(|_| StageFailure::Signature)?;
    let wrapped_public_key = config
        .pointer("/plugins/updater/pubkey")
        .and_then(Value::as_str)
        .ok_or(StageFailure::Signature)?;
    verify_updater_signature_with_key(archive, wrapped_signature, wrapped_public_key)
}

fn verify_updater_signature_with_key(
    archive: &[u8],
    wrapped_signature: &[u8],
    wrapped_public_key: &str,
) -> Result<(), StageFailure> {
    let decoded_key = decode_wrapped(wrapped_public_key.as_bytes())?;
    let decoded_signature = decode_wrapped(wrapped_signature)?;
    let public_key = PublicKey::decode(&decoded_key).map_err(|_| StageFailure::Signature)?;
    let signature = Signature::decode(&decoded_signature).map_err(|_| StageFailure::Signature)?;
    public_key
        .verify(archive, &signature, true)
        .map_err(|_| StageFailure::Signature)
}

fn decode_wrapped(value: &[u8]) -> Result<String, StageFailure> {
    let value = std::str::from_utf8(value).map_err(|_| StageFailure::Signature)?;
    let decoded = STANDARD
        .decode(value.trim())
        .map_err(|_| StageFailure::Signature)?;
    String::from_utf8(decoded).map_err(|_| StageFailure::Signature)
}

fn stage_and_cleanup(owned_archive: &[u8], limits: Limits) -> Result<usize, StageError> {
    let preflight = preflight(owned_archive, limits).map_err(StageError::primary)?;
    let root = PrivateRoot::create().map_err(StageError::primary)?;
    let staged = materialize(owned_archive, &preflight, root.path(), limits)
        .and_then(|()| verify_staged_tree(root.path(), &preflight));
    settle_staging(root, staged, preflight.records.len())
}

fn settle_staging(
    mut root: PrivateRoot,
    staged: Result<(), StageFailure>,
    member_count: usize,
) -> Result<usize, StageError> {
    match staged {
        Err(primary) => match root.cleanup() {
            Ok(()) => Err(StageError::primary(primary)),
            Err(cleanup) => Err(StageError::retained(Some(primary), cleanup, root)),
        },
        Ok(()) => match root.cleanup() {
            Ok(()) => Ok(member_count),
            Err(cleanup) => Err(StageError::retained(None, cleanup, root)),
        },
    }
}

#[derive(Debug)]
struct PrivateRoot {
    path: Option<PathBuf>,
    #[cfg(test)]
    cleanup_failures_remaining: usize,
}

impl PrivateRoot {
    fn create() -> Result<Self, StageFailure> {
        Self::create_with_cleanup_failures(0)
    }

    fn create_with_cleanup_failures(
        cleanup_failures_remaining: usize,
    ) -> Result<Self, StageFailure> {
        #[cfg(not(test))]
        let _ = cleanup_failures_remaining;
        let root = tempfile::Builder::new()
            .prefix("batcave-macos-updater-")
            .tempdir()
            .map_err(|_| StageFailure::Materialization)?;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .map_err(|_| StageFailure::Materialization)?;
        let metadata =
            fs::symlink_metadata(root.path()).map_err(|_| StageFailure::Materialization)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err(StageFailure::Materialization);
        }
        Ok(Self {
            path: Some(root.keep()),
            #[cfg(test)]
            cleanup_failures_remaining,
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("private root is retained")
    }

    fn cleanup(&mut self) -> Result<(), StageFailure> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if self.cleanup_failures_remaining > 0 {
            self.cleanup_failures_remaining -= 1;
            return Err(StageFailure::Cleanup);
        }
        make_tree_removable(path)?;
        fs::remove_dir_all(path).map_err(|_| StageFailure::Cleanup)?;
        if path.exists() {
            return Err(StageFailure::Cleanup);
        }
        self.path.take();
        Ok(())
    }
}

fn make_tree_removable(path: &Path) -> Result<(), StageFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| StageFailure::Cleanup)?;
    if metadata.file_type().is_symlink() {
        return fs::remove_file(path).map_err(|_| StageFailure::Cleanup);
    }
    if metadata.is_file() {
        return fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| StageFailure::Cleanup);
    }
    if !metadata.is_dir() {
        return fs::remove_file(path).map_err(|_| StageFailure::Cleanup);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| StageFailure::Cleanup)?;
    for entry in fs::read_dir(path).map_err(|_| StageFailure::Cleanup)? {
        make_tree_removable(&entry.map_err(|_| StageFailure::Cleanup)?.path())?;
    }
    Ok(())
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        for _ in 0..CLEANUP_RETRY_LIMIT {
            if self.cleanup().is_ok() {
                break;
            }
        }
    }
}

fn preflight(owned_archive: &[u8], limits: Limits) -> Result<Preflight, StageFailure> {
    if owned_archive.is_empty() || owned_archive.len() > limits.max_compressed_bytes {
        return Err(StageFailure::Preflight);
    }
    let decoder = GzDecoder::new(Cursor::new(owned_archive));
    let reader = BudgetReader::new(decoder, limits.max_decompressed_tar_bytes);
    let mut archive = Archive::new(reader);
    let entries = archive.entries().map_err(|_| StageFailure::Preflight)?;
    let mut records = Vec::new();
    let mut canonical = BTreeMap::<Vec<String>, CanonicalEntry>::new();
    let mut expanded_bytes = 0_u64;
    let mut path_budget = PathBudget::new(limits.max_path_bookkeeping_bytes);

    for entry in entries {
        if records.len() >= limits.max_member_count {
            return Err(StageFailure::Preflight);
        }
        let mut entry = entry.map_err(|_| StageFailure::Preflight)?;
        let parts = checked_parts(&entry.path_bytes(), limits)?;
        if parts[0] != APP_NAME
            || parts
                .iter()
                .skip(1)
                .any(|part| part.to_lowercase().ends_with(".app"))
        {
            return Err(StageFailure::Preflight);
        }
        let kind = supported_kind(entry.header().entry_type())?;
        let size = entry.header().size().map_err(|_| StageFailure::Preflight)?;
        let mode = entry.header().mode().map_err(|_| StageFailure::Preflight)? & 0o777;
        let digest = if kind == RecordKind::File {
            if size > limits.max_file_bytes {
                return Err(StageFailure::Preflight);
            }
            expanded_bytes = expanded_bytes
                .checked_add(size)
                .ok_or(StageFailure::Preflight)?;
            if expanded_bytes > limits.max_expanded_bytes {
                return Err(StageFailure::Preflight);
            }
            Some(hash_entry(&mut entry, size)?)
        } else {
            if size != 0 {
                return Err(StageFailure::Preflight);
            }
            None
        };
        path_budget.charge(retained_path_bytes(&parts, parts.capacity())?)?;
        register_prefixes(&mut canonical, &parts, &kind, limits, &mut path_budget)?;
        records.push(ArchiveRecord {
            parts,
            kind,
            size,
            mode,
            digest,
        });
    }

    let mut reader = archive.into_inner();
    validate_zero_tar_tail(&mut reader)?;
    let decoder = reader.into_inner();
    if decoder.get_ref().position() != owned_archive.len() as u64 {
        return Err(StageFailure::Preflight);
    }
    let root_key = collision_key(&[APP_NAME.to_owned()]);
    match canonical.get(&root_key) {
        Some(root) if root.kind == RecordKind::Directory => {}
        _ => return Err(StageFailure::Preflight),
    }
    let expected_paths = canonical
        .into_values()
        .map(|entry| (entry.original, entry.kind))
        .collect();
    Ok(Preflight {
        records,
        expected_paths,
    })
}

fn checked_parts(raw: &[u8], limits: Limits) -> Result<Vec<String>, StageFailure> {
    if raw.is_empty() || raw[0] == b'/' || raw.contains(&b'\\') || raw.len() > limits.max_path_bytes
    {
        return Err(StageFailure::Preflight);
    }
    let name = std::str::from_utf8(raw)
        .map_err(|_| StageFailure::Preflight)?
        .trim_end_matches('/');
    let parts = name.split('/').map(str::to_owned).collect::<Vec<_>>();
    if parts.is_empty()
        || parts.len() > limits.max_path_depth
        || parts
            .iter()
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\0'))
    {
        return Err(StageFailure::Preflight);
    }
    Ok(parts)
}

fn supported_kind(entry_type: EntryType) -> Result<RecordKind, StageFailure> {
    if entry_type.is_dir() {
        Ok(RecordKind::Directory)
    } else if entry_type.is_file() {
        Ok(RecordKind::File)
    } else {
        Err(StageFailure::Preflight)
    }
}

fn register_prefixes(
    canonical: &mut BTreeMap<Vec<String>, CanonicalEntry>,
    parts: &[String],
    member_kind: &RecordKind,
    limits: Limits,
    path_budget: &mut PathBudget,
) -> Result<(), StageFailure> {
    for length in 1..=parts.len() {
        let prefix = parts[..length].to_vec();
        let key = collision_key(&prefix);
        let kind = if length == parts.len() {
            member_kind.clone()
        } else {
            RecordKind::Directory
        };
        let explicit = length == parts.len();
        match canonical.get_mut(&key) {
            None => {
                if canonical.len() >= limits.max_canonical_prefixes {
                    return Err(StageFailure::Preflight);
                }
                path_budget.charge(
                    retained_path_bytes(&prefix, prefix.capacity())?
                        .checked_add(retained_path_bytes(&key, key.capacity())?)
                        .ok_or(StageFailure::Preflight)?,
                )?;
                canonical.insert(
                    key,
                    CanonicalEntry {
                        original: prefix,
                        kind,
                        explicit,
                    },
                );
            }
            Some(existing) => {
                if existing.kind != kind
                    || existing.original != prefix
                    || (explicit && existing.explicit)
                {
                    return Err(StageFailure::Preflight);
                }
                existing.explicit |= explicit;
            }
        }
    }
    Ok(())
}

fn collision_key(parts: &[String]) -> Vec<String> {
    parts
        .iter()
        .map(|part| part.nfd().case_fold().collect())
        .collect()
}

fn retained_path_bytes(parts: &[String], capacity: usize) -> Result<usize, StageFailure> {
    let slots = capacity
        .checked_mul(std::mem::size_of::<String>())
        .ok_or(StageFailure::Preflight)?;
    parts.iter().try_fold(
        std::mem::size_of::<Vec<String>>()
            .checked_add(slots)
            .ok_or(StageFailure::Preflight)?,
        |total, part| {
            total
                .checked_add(part.capacity())
                .ok_or(StageFailure::Preflight)
        },
    )
}

struct PathBudget {
    limit: usize,
    consumed: usize,
}

impl PathBudget {
    fn new(limit: usize) -> Self {
        Self { limit, consumed: 0 }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), StageFailure> {
        self.consumed = self
            .consumed
            .checked_add(bytes)
            .ok_or(StageFailure::Preflight)?;
        if self.consumed > self.limit {
            return Err(StageFailure::Preflight);
        }
        Ok(())
    }
}

fn hash_entry(entry: &mut impl Read, expected_size: u64) -> Result<[u8; 32], StageFailure> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; COPY_CHUNK_BYTES];
    let mut consumed = 0_u64;
    loop {
        let read = entry
            .read(&mut buffer)
            .map_err(|_| StageFailure::Preflight)?;
        if read == 0 {
            break;
        }
        consumed = consumed
            .checked_add(read as u64)
            .ok_or(StageFailure::Preflight)?;
        digest.update(&buffer[..read]);
    }
    if consumed != expected_size {
        return Err(StageFailure::Preflight);
    }
    Ok(digest.finalize().into())
}

fn materialize(
    owned_archive: &[u8],
    preflight: &Preflight,
    root: &Path,
    limits: Limits,
) -> Result<(), StageFailure> {
    let decoder = GzDecoder::new(Cursor::new(owned_archive));
    let reader = BudgetReader::new(decoder, limits.max_decompressed_tar_bytes);
    let mut archive = Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|_| StageFailure::Materialization)?;
    let mut index = 0_usize;
    for entry in entries {
        let mut entry = entry.map_err(|_| StageFailure::Materialization)?;
        let record = preflight
            .records
            .get(index)
            .ok_or(StageFailure::Materialization)?;
        index += 1;
        let parts = checked_parts(&entry.path_bytes(), limits)
            .map_err(|_| StageFailure::Materialization)?;
        let kind = supported_kind(entry.header().entry_type())
            .map_err(|_| StageFailure::Materialization)?;
        let size = entry
            .header()
            .size()
            .map_err(|_| StageFailure::Materialization)?;
        let mode = entry
            .header()
            .mode()
            .map_err(|_| StageFailure::Materialization)?
            & 0o777;
        if parts != record.parts
            || kind != record.kind
            || size != record.size
            || mode != record.mode
        {
            return Err(StageFailure::Materialization);
        }
        let destination = join_parts(root, &parts);
        match kind {
            RecordKind::Directory => {
                ensure_directory(root, &parts)?;
                set_mode(&destination, safe_directory_mode(mode))?;
            }
            RecordKind::File => {
                ensure_directory(root, &parts[..parts.len() - 1])?;
                let mut output = create_new_file(&destination)?;
                let digest = copy_file_entry(&mut entry, &mut output, size)?;
                output
                    .sync_all()
                    .map_err(|_| StageFailure::Materialization)?;
                if Some(digest) != record.digest {
                    return Err(StageFailure::Materialization);
                }
                set_mode(&destination, safe_file_mode(mode))?;
            }
        }
    }
    if index != preflight.records.len() {
        return Err(StageFailure::Materialization);
    }
    let mut reader = archive.into_inner();
    validate_zero_tar_tail(&mut reader).map_err(|_| StageFailure::Materialization)?;
    if reader.into_inner().get_ref().position() != owned_archive.len() as u64 {
        return Err(StageFailure::Materialization);
    }
    Ok(())
}

fn validate_zero_tar_tail(reader: &mut impl Read) -> Result<(), StageFailure> {
    let mut buffer = [0_u8; COPY_CHUNK_BYTES];
    let mut consumed = 0_usize;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| StageFailure::Preflight)?;
        if read == 0 {
            break;
        }
        if buffer[..read].iter().any(|byte| *byte != 0) {
            return Err(StageFailure::Preflight);
        }
        consumed = consumed.checked_add(read).ok_or(StageFailure::Preflight)?;
    }
    if consumed < 512 || !consumed.is_multiple_of(512) {
        return Err(StageFailure::Preflight);
    }
    Ok(())
}

fn ensure_directory(root: &Path, parts: &[String]) -> Result<(), StageFailure> {
    let mut current = root.to_path_buf();
    for part in parts {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(StageFailure::Materialization);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder
                    .mode(0o700)
                    .create(&current)
                    .map_err(|_| StageFailure::Materialization)?;
            }
            Err(_) => return Err(StageFailure::Materialization),
        }
    }
    Ok(())
}

fn create_new_file(path: &Path) -> Result<File, StageFailure> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| StageFailure::Materialization)
}

fn copy_file_entry(
    entry: &mut impl Read,
    output: &mut File,
    expected_size: u64,
) -> Result<[u8; 32], StageFailure> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; COPY_CHUNK_BYTES];
    let mut copied = 0_u64;
    loop {
        let read = entry
            .read(&mut buffer)
            .map_err(|_| StageFailure::Materialization)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or(StageFailure::Materialization)?;
        output
            .write_all(&buffer[..read])
            .map_err(|_| StageFailure::Materialization)?;
        digest.update(&buffer[..read]);
    }
    if copied != expected_size {
        return Err(StageFailure::Materialization);
    }
    Ok(digest.finalize().into())
}

fn verify_staged_tree(root: &Path, preflight: &Preflight) -> Result<(), StageFailure> {
    let mut actual = BTreeMap::new();
    collect_paths(root, root, &mut actual)?;
    if actual != preflight.expected_paths {
        return Err(StageFailure::Verification);
    }
    for record in &preflight.records {
        let path = join_parts(root, &record.parts);
        let metadata = fs::symlink_metadata(&path).map_err(|_| StageFailure::Verification)?;
        if metadata.file_type().is_symlink() {
            return Err(StageFailure::Verification);
        }
        if record.kind == RecordKind::File {
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| StageFailure::Verification)?;
            let mut digest = Sha256::new();
            let mut copied = 0_u64;
            let mut buffer = [0_u8; COPY_CHUNK_BYTES];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|_| StageFailure::Verification)?;
                if read == 0 {
                    break;
                }
                copied = copied
                    .checked_add(read as u64)
                    .ok_or(StageFailure::Verification)?;
                digest.update(&buffer[..read]);
            }
            if copied != record.size || Some(<[u8; 32]>::from(digest.finalize())) != record.digest {
                return Err(StageFailure::Verification);
            }
        }
    }
    Ok(())
}

fn collect_paths(
    root: &Path,
    directory: &Path,
    paths: &mut BTreeMap<Vec<String>, RecordKind>,
) -> Result<(), StageFailure> {
    for entry in fs::read_dir(directory).map_err(|_| StageFailure::Verification)? {
        let entry = entry.map_err(|_| StageFailure::Verification)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| StageFailure::Verification)?;
        if metadata.file_type().is_symlink() {
            return Err(StageFailure::Verification);
        }
        let parts = path
            .strip_prefix(root)
            .map_err(|_| StageFailure::Verification)?
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .map(str::to_owned)
                    .ok_or(StageFailure::Verification)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let kind = if metadata.is_dir() {
            RecordKind::Directory
        } else if metadata.is_file() {
            RecordKind::File
        } else {
            return Err(StageFailure::Verification);
        };
        if paths.insert(parts, kind.clone()).is_some() {
            return Err(StageFailure::Verification);
        }
        if kind == RecordKind::Directory {
            collect_paths(root, &path, paths)?;
        }
    }
    Ok(())
}

fn join_parts(root: &Path, parts: &[String]) -> PathBuf {
    parts
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn set_mode(path: &Path, mode: u32) -> Result<(), StageFailure> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|_| StageFailure::Materialization)
}

fn safe_directory_mode(mode: u32) -> u32 {
    (mode & 0o755) | 0o500
}

fn safe_file_mode(mode: u32) -> u32 {
    (mode & 0o755) | 0o400
}

struct BudgetReader<R> {
    inner: R,
    limit: usize,
    consumed: usize,
}

impl<R> BudgetReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            consumed: 0,
        }
    }

    fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for BudgetReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.consumed == self.limit {
            let mut probe = [0_u8; 1];
            if self.inner.read(&mut probe)? == 0 {
                return Ok(0);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decompressed archive budget exceeded",
            ));
        }
        let allowed = buffer.len().min(self.limit - self.consumed);
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.consumed += read;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, GzBuilder};
    use std::io::Write;
    use tar::{Builder, EntryType, Header};

    const FIXTURE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgRTc2MjBGMTg0MkI0RTgxRgpSV1FmNkxSQ0dBOWk1M21sWWVjTzRJelQ1MVRHUHB2V3VjTlNDaDFDQk0wUVRhTG43M1k3R0ZPMw==";
    const FIXTURE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUldRZjZMUkNHQTlpNTlTTE9GeHo2Tnh2QVNYREplUnR1Wnlrd1FlcGJERUd0ODdpZzFCTnBXYVZXdU5ybTczWWlJaUpicTcxV2krZFA5ZUtMOE9DMzUxdndJYXNTU2JYeHdBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNTU1Nzc5OTY2CWZpbGU6dGVzdApRdEtNWFd5WWN3ZHBaQWxQRjd0RTJFTkprUmQxdWp2S2psajFtOVJ0SFRCblpQYTVXS1U1dVdSczVHb1A1TS9WcUU4MVFGdU1LSTVrL1NmTlFVYU9BQT09";

    fn archive(entries: &[(&str, EntryType, &[u8])]) -> Vec<u8> {
        let mut tar = Builder::new(Vec::new());
        tar.mode(tar::HeaderMode::Deterministic);
        for (name, kind, bytes) in entries {
            let mut header = Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(if kind.is_dir() { 0o755 } else { 0o644 });
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_size(if kind.is_file() {
                bytes.len() as u64
            } else {
                0
            });
            set_raw_name(&mut header, name.as_bytes());
            if kind.is_symlink() {
                header.set_link_name("outside").expect("fixture link name");
            }
            header.set_cksum();
            tar.append(&header, Cursor::new(*bytes))
                .expect("append fixture");
        }
        let raw = tar.into_inner().expect("finish tar");
        let mut gzip = GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), Compression::default());
        gzip.write_all(&raw).expect("write gzip");
        gzip.finish().expect("finish gzip")
    }

    fn set_raw_name(header: &mut Header, name: &[u8]) {
        assert!(name.len() <= 100, "fixture path must fit the name field");
        let bytes = header.as_mut_bytes();
        bytes[..100].fill(0);
        bytes[..name.len()].copy_from_slice(name);
    }

    fn valid_archive() -> Vec<u8> {
        archive(&[
            (APP_NAME, EntryType::Directory, b""),
            ("BatCave Monitor.app/Contents", EntryType::Directory, b""),
            (
                "BatCave Monitor.app/Contents/fixture",
                EntryType::Regular,
                b"fixture\n",
            ),
        ])
    }

    fn assert_stage_failure(result: Result<usize, StageError>, expected: StageFailure) {
        let error = match result {
            Ok(_) => panic!("staging unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.primary, Some(expected));
        assert_eq!(error.cleanup, None);
        assert!(!error.residue_retained());
    }

    #[test]
    fn known_minisign_fixture_verifies_and_tamper_fails() {
        verify_updater_signature_with_key(
            b"test",
            FIXTURE_SIGNATURE.as_bytes(),
            FIXTURE_PUBLIC_KEY,
        )
        .expect("known fixture verifies");
        assert_eq!(
            verify_updater_signature_with_key(
                b"tampered",
                FIXTURE_SIGNATURE.as_bytes(),
                FIXTURE_PUBLIC_KEY,
            ),
            Err(StageFailure::Signature)
        );
    }

    #[test]
    fn exact_stream_stages_reverifies_and_cleans() {
        let member_count = stage_and_cleanup(&valid_archive(), Limits::production())
            .expect("valid archive stages and cleans");
        assert_eq!(member_count, 3);
    }

    #[test]
    fn hostile_paths_links_collisions_and_trailing_members_fail_closed() {
        let cases = [
            archive(&[(
                "BatCave Monitor.app/../outside",
                EntryType::Regular,
                b"outside",
            )]),
            archive(&[("BatCave Monitor.app/Contents/link", EntryType::Symlink, b"")]),
            archive(&[
                (APP_NAME, EntryType::Directory, b""),
                ("BatCave Monitor.app/Contents/A", EntryType::Regular, b"one"),
                ("BatCave Monitor.app/contents/a", EntryType::Regular, b"two"),
            ]),
        ];
        for hostile in cases {
            assert_stage_failure(
                stage_and_cleanup(&hostile, Limits::production()),
                StageFailure::Preflight,
            );
        }

        let mut trailing = valid_archive();
        trailing.extend_from_slice(&valid_archive());
        assert_stage_failure(
            stage_and_cleanup(&trailing, Limits::production()),
            StageFailure::Preflight,
        );
    }

    #[test]
    fn compressed_and_expanded_limits_fail_before_retained_output() {
        let bytes = valid_archive();
        let mut compressed = Limits::production();
        compressed.max_compressed_bytes = bytes.len() - 1;
        assert_stage_failure(
            stage_and_cleanup(&bytes, compressed),
            StageFailure::Preflight,
        );

        let mut expanded = Limits::production();
        expanded.max_expanded_bytes = 1;
        assert_stage_failure(stage_and_cleanup(&bytes, expanded), StageFailure::Preflight);
    }

    #[test]
    fn combined_materialization_and_cleanup_failure_retains_both_until_retry() {
        let bytes = valid_archive();
        let limits = Limits::production();
        let preflight = preflight(&bytes, limits).expect("preflight");
        let root =
            PrivateRoot::create_with_cleanup_failures(2).expect("create retained fixture root");
        let retained_path = root.path().to_path_buf();
        File::create(root.path().join(APP_NAME)).expect("create materialization collision");
        let staged = materialize(&bytes, &preflight, root.path(), limits);
        assert_eq!(staged, Err(StageFailure::Materialization));

        let mut error = settle_staging(root, staged, preflight.records.len())
            .expect_err("combined failure cannot complete staging");
        assert_eq!(error.primary, Some(StageFailure::Materialization));
        assert_eq!(error.cleanup, Some(StageFailure::Cleanup));
        assert!(error.residue_retained());
        assert!(retained_path.exists());
        assert_eq!(error.retry_cleanup(), Err(StageFailure::Cleanup));
        assert!(error.residue_retained());
        error
            .retry_cleanup_bounded()
            .expect("bounded retry removes retained staging root");
        assert!(!error.residue_retained());
        assert!(!retained_path.exists());
        assert_eq!(
            error.into_public_failure(),
            Failure::MaterializationAndCleanupFailed
        );
    }

    #[test]
    fn combined_verification_and_cleanup_failure_never_becomes_success() {
        let bytes = valid_archive();
        let limits = Limits::production();
        let preflight = preflight(&bytes, limits).expect("preflight");
        let root =
            PrivateRoot::create_with_cleanup_failures(1).expect("create retained fixture root");
        let retained_path = root.path().to_path_buf();
        materialize(&bytes, &preflight, root.path(), limits).expect("materialize fixture");
        File::create(root.path().join(APP_NAME).join("unexpected"))
            .expect("create verification collision");
        let staged = verify_staged_tree(root.path(), &preflight);
        assert_eq!(staged, Err(StageFailure::Verification));

        let mut error = settle_staging(root, staged, preflight.records.len())
            .expect_err("combined failure cannot emit a completion");
        assert_eq!(error.primary, Some(StageFailure::Verification));
        assert_eq!(error.cleanup, Some(StageFailure::Cleanup));
        assert!(error.residue_retained());
        error
            .retry_cleanup_bounded()
            .expect("bounded retry removes retained staging root");
        assert!(!retained_path.exists());
        assert_eq!(
            error.into_public_failure(),
            Failure::VerificationAndCleanupFailed
        );
    }
}
