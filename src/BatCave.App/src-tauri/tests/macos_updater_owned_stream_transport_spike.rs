//! Non-production issue #114 probe for macOS updater archive transport.
//!
//! The accepted install-smoke architecture has no production Rust entry that
//! can independently establish public release identity yet. This integration
//! test therefore exercises only the safe transport primitive: exact bytes are
//! copied into a Rust-owned immutable stream, preflighted completely, staged in
//! a private root, verified, and removed. It does not verify or launch an app,
//! mint a native receipt, or emit release evidence.

use flate2::{write::GzEncoder, Compression, GzBuilder};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use tar::{Builder, EntryType, Header};

const APP_NAME: &str = "BatCave Monitor.app";
const FIXTURE_FILE: &str = "BatCave Monitor.app/Contents/fixture";
const FIXTURE_BYTES: &[u8] = b"BatCave updater owned-stream fixture\n";

mod adapter {
    use flate2::bufread::GzDecoder;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Cursor, Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tar::{Archive, EntryType};
    use unicode_casefold::UnicodeCaseFold;
    use unicode_normalization::UnicodeNormalization;

    const COPY_CHUNK_BYTES: usize = 64 * 1024;
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy)]
    pub(super) struct Limits {
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
        pub(super) const fn production() -> Self {
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

        #[cfg(test)]
        pub(super) fn with_compressed_limit(mut self, value: usize) -> Self {
            self.max_compressed_bytes = value;
            self
        }

        #[cfg(test)]
        pub(super) fn with_member_limit(mut self, value: usize) -> Self {
            self.max_member_count = value;
            self
        }

        #[cfg(test)]
        pub(super) fn with_expanded_limit(mut self, value: u64) -> Self {
            self.max_expanded_bytes = value;
            self
        }

        #[cfg(test)]
        pub(super) fn with_path_bookkeeping_limit(mut self, value: usize) -> Self {
            self.max_path_bookkeeping_bytes = value;
            self
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum FailureBoundary {
        Acquisition,
        Authority,
        Preflight,
        Materialization,
        Verification,
        Cleanup,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct AdapterError {
        boundary: FailureBoundary,
        code: &'static str,
    }

    impl AdapterError {
        fn new(boundary: FailureBoundary, code: &'static str) -> Self {
            Self { boundary, code }
        }

        pub(super) fn boundary(&self) -> FailureBoundary {
            self.boundary
        }

        pub(super) fn code(&self) -> &'static str {
            self.code
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum Disposition {
        PrototypeStaged,
        RetainedCleanupFailed,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct Outcome {
        pub(super) disposition: Disposition,
        pub(super) archive_sha256: String,
        pub(super) member_count: usize,
        pub(super) archive_stream_consumed: bool,
        pub(super) preflight_passed: bool,
        pub(super) app_staged: bool,
        pub(super) staged_tree_verified: bool,
        pub(super) cleanup_passed: bool,
        pub(super) residue_retained: bool,
        pub(super) package_installed: bool,
        pub(super) app_launched: bool,
        pub(super) trust_verified: bool,
        pub(super) native_proven: bool,
        pub(super) receipt_emitted: bool,
        pub(super) evidence_emitted: bool,
        pub(super) limitations: [&'static str; 2],
    }

    struct CompletionSeal;

    pub(super) struct Completion {
        seal: Arc<CompletionSeal>,
        outcome: Outcome,
    }

    impl Completion {
        pub(super) fn outcome(&self) -> &Outcome {
            &self.outcome
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AuthorityPhase {
        Acquired,
        Staged,
    }

    pub(super) struct ClosedPlan {
        expected_app_name: &'static str,
        expected_size: usize,
        expected_sha256: [u8; 32],
        limits: Limits,
        fail_cleanup_once: bool,
        fail_stage_cleanup_once: bool,
        create_materialization_collision: bool,
        create_verification_collision: bool,
    }

    pub(super) fn closed_fixture_plan(archive: &[u8]) -> ClosedPlan {
        plan_with_options(archive, Limits::production(), false, false, false, false)
    }

    #[cfg(test)]
    pub(super) fn limited_fixture_plan(archive: &[u8], limits: Limits) -> ClosedPlan {
        plan_with_options(archive, limits, false, false, false, false)
    }

    #[cfg(test)]
    pub(super) fn cleanup_failure_fixture_plan(archive: &[u8]) -> ClosedPlan {
        plan_with_options(archive, Limits::production(), true, false, false, false)
    }

    #[cfg(test)]
    pub(super) fn materialization_collision_fixture_plan(archive: &[u8]) -> ClosedPlan {
        plan_with_options(archive, Limits::production(), false, false, true, false)
    }

    #[cfg(test)]
    pub(super) fn materialization_cleanup_failure_fixture_plan(archive: &[u8]) -> ClosedPlan {
        plan_with_options(archive, Limits::production(), false, true, true, false)
    }

    #[cfg(test)]
    pub(super) fn verification_cleanup_failure_fixture_plan(archive: &[u8]) -> ClosedPlan {
        plan_with_options(archive, Limits::production(), false, true, false, true)
    }

    fn plan_with_options(
        archive: &[u8],
        limits: Limits,
        fail_cleanup_once: bool,
        fail_stage_cleanup_once: bool,
        create_materialization_collision: bool,
        create_verification_collision: bool,
    ) -> ClosedPlan {
        ClosedPlan {
            expected_app_name: super::APP_NAME,
            expected_size: archive.len(),
            expected_sha256: Sha256::digest(archive).into(),
            limits,
            fail_cleanup_once,
            fail_stage_cleanup_once,
            create_materialization_collision,
            create_verification_collision,
        }
    }

    pub(super) struct UpdaterStreamAuthority {
        phase: AuthorityPhase,
        owned_archive: Arc<[u8]>,
        plan: ClosedPlan,
        seal: Arc<CompletionSeal>,
    }

    pub(super) fn acquire(
        plan: ClosedPlan,
        source: &[u8],
    ) -> Result<UpdaterStreamAuthority, AdapterError> {
        if source.len() != plan.expected_size
            || <[u8; 32]>::from(Sha256::digest(source)) != plan.expected_sha256
        {
            return Err(AdapterError::new(
                FailureBoundary::Acquisition,
                "selected_archive_mismatch",
            ));
        }
        Ok(UpdaterStreamAuthority {
            phase: AuthorityPhase::Acquired,
            owned_archive: Arc::from(source.to_vec()),
            plan,
            seal: Arc::new(CompletionSeal),
        })
    }

    impl UpdaterStreamAuthority {
        pub(super) fn stage(&mut self) -> Result<StagedUpdater, AdapterError> {
            if self.phase != AuthorityPhase::Acquired {
                return Err(AdapterError::new(
                    FailureBoundary::Authority,
                    "archive_authority_replayed",
                ));
            }
            self.phase = AuthorityPhase::Staged;
            let preflight = preflight(
                &self.owned_archive,
                self.plan.expected_app_name,
                self.plan.limits,
            )?;
            let mut root = PrivateRoot::create(self.plan.fail_stage_cleanup_once)?;
            let staged = (|| {
                if self.plan.create_materialization_collision {
                    let collision = root.path().join(self.plan.expected_app_name);
                    File::create(collision).map_err(|_| {
                        AdapterError::new(
                            FailureBoundary::Materialization,
                            "fixture_collision_setup_failed",
                        )
                    })?;
                }
                materialize(
                    &self.owned_archive,
                    &preflight,
                    root.path(),
                    self.plan.limits,
                )?;
                if self.plan.create_verification_collision {
                    File::create(
                        root.path()
                            .join(self.plan.expected_app_name)
                            .join("unexpected"),
                    )
                    .map_err(|_| materialization_error())?;
                }
                verify_staged_tree(root.path(), &preflight)
            })();
            if let Err(error) = staged {
                return Err(root.abort(error));
            }
            Ok(StagedUpdater {
                root: root.take(),
                seal: Arc::clone(&self.seal),
                archive_sha256: hex_digest(self.plan.expected_sha256),
                member_count: preflight.records.len(),
                fail_cleanup_once: self.plan.fail_cleanup_once,
            })
        }

        pub(super) fn accepts_completion(&self, completion: &Completion) -> bool {
            self.phase == AuthorityPhase::Staged && Arc::ptr_eq(&self.seal, &completion.seal)
        }
    }

    pub(super) struct StagedUpdater {
        root: Option<PathBuf>,
        seal: Arc<CompletionSeal>,
        archive_sha256: String,
        member_count: usize,
        fail_cleanup_once: bool,
    }

    impl StagedUpdater {
        pub(super) fn complete(&mut self) -> Completion {
            let cleanup_passed = if self.fail_cleanup_once {
                self.fail_cleanup_once = false;
                false
            } else {
                self.cleanup().is_ok()
            };
            self.completion(cleanup_passed)
        }

        pub(super) fn retry_cleanup(&mut self) -> Completion {
            let cleanup_passed = self.cleanup().is_ok();
            self.completion(cleanup_passed)
        }

        fn cleanup(&mut self) -> Result<(), AdapterError> {
            let Some(root) = self.root.as_ref() else {
                return Ok(());
            };
            fs::remove_dir_all(root).map_err(|_| {
                AdapterError::new(FailureBoundary::Cleanup, "staging_cleanup_failed")
            })?;
            self.root.take();
            Ok(())
        }

        fn completion(&self, cleanup_passed: bool) -> Completion {
            Completion {
                seal: Arc::clone(&self.seal),
                outcome: Outcome {
                    disposition: if cleanup_passed {
                        Disposition::PrototypeStaged
                    } else {
                        Disposition::RetainedCleanupFailed
                    },
                    archive_sha256: self.archive_sha256.clone(),
                    member_count: self.member_count,
                    archive_stream_consumed: true,
                    preflight_passed: true,
                    app_staged: true,
                    staged_tree_verified: true,
                    cleanup_passed,
                    residue_retained: !cleanup_passed,
                    package_installed: false,
                    app_launched: false,
                    trust_verified: false,
                    native_proven: false,
                    receipt_emitted: false,
                    evidence_emitted: false,
                    limitations: ["fixture_only", "macos_updater_staging_only"],
                },
            }
        }
    }

    impl Drop for StagedUpdater {
        fn drop(&mut self) {
            let _ = self.cleanup();
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

    fn preflight(
        owned_archive: &[u8],
        expected_app_name: &str,
        limits: Limits,
    ) -> Result<Preflight, AdapterError> {
        if owned_archive.len() > limits.max_compressed_bytes {
            return Err(preflight_error("compressed_budget_exceeded"));
        }
        let decoder = GzDecoder::new(Cursor::new(owned_archive));
        let reader = BudgetReader::new(decoder, limits.max_decompressed_tar_bytes);
        let mut archive = Archive::new(reader);
        let entries = archive
            .entries()
            .map_err(|_| preflight_error("archive_stream_invalid"))?;
        let mut records = Vec::new();
        let mut canonical = BTreeMap::<Vec<String>, CanonicalEntry>::new();
        let mut expanded_bytes = 0_u64;
        let mut path_budget = PathBudget::new(limits.max_path_bookkeeping_bytes);

        for entry in entries {
            if records.len() >= limits.max_member_count {
                return Err(preflight_error("member_budget_exceeded"));
            }
            let mut entry = entry.map_err(|_| preflight_error("archive_stream_invalid"))?;
            let raw_path = entry.path_bytes();
            let parts = checked_parts(&raw_path, limits)?;
            if parts[0] != expected_app_name {
                return Err(preflight_error("unexpected_archive_root"));
            }
            if parts
                .iter()
                .skip(1)
                .any(|part| part.to_lowercase().ends_with(".app"))
            {
                return Err(preflight_error("nested_app_rejected"));
            }

            let kind = supported_kind(entry.header().entry_type())?;
            let size = entry
                .header()
                .size()
                .map_err(|_| preflight_error("member_size_invalid"))?;
            let mode = entry
                .header()
                .mode()
                .map_err(|_| preflight_error("member_mode_invalid"))?
                & 0o777;
            let digest = if kind == RecordKind::File {
                if size > limits.max_file_bytes {
                    return Err(preflight_error("file_budget_exceeded"));
                }
                expanded_bytes = expanded_bytes
                    .checked_add(size)
                    .ok_or_else(|| preflight_error("expanded_budget_exceeded"))?;
                if expanded_bytes > limits.max_expanded_bytes {
                    return Err(preflight_error("expanded_budget_exceeded"));
                }
                Some(hash_entry(&mut entry, size)?)
            } else {
                if size != 0 {
                    return Err(preflight_error("directory_size_invalid"));
                }
                None
            };

            path_budget.charge(path_payload_bytes(&parts))?;
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
        io::copy(&mut reader, &mut io::sink())
            .map_err(|_| preflight_error("archive_stream_invalid"))?;
        let decoder = reader.into_inner();
        if decoder.get_ref().position() != owned_archive.len() as u64 {
            return Err(preflight_error("archive_trailing_data"));
        }
        let root_key = collision_key(&[expected_app_name.to_owned()]);
        match canonical.get(&root_key) {
            Some(root) if root.kind == RecordKind::Directory => {}
            _ => return Err(preflight_error("app_root_not_directory")),
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

    fn checked_parts(raw: &[u8], limits: Limits) -> Result<Vec<String>, AdapterError> {
        if raw.is_empty() || raw[0] == b'/' || raw.contains(&b'\\') {
            return Err(preflight_error("archive_path_not_relative"));
        }
        if raw.len() > limits.max_path_bytes {
            return Err(preflight_error("path_budget_exceeded"));
        }
        let name = std::str::from_utf8(raw)
            .map_err(|_| preflight_error("archive_path_not_utf8"))?
            .trim_end_matches('/');
        let parts = name.split('/').map(str::to_owned).collect::<Vec<_>>();
        if parts.is_empty()
            || parts.len() > limits.max_path_depth
            || parts
                .iter()
                .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\0'))
        {
            return Err(preflight_error("archive_path_not_canonical"));
        }
        Ok(parts)
    }

    fn supported_kind(entry_type: EntryType) -> Result<RecordKind, AdapterError> {
        if entry_type.is_dir() {
            Ok(RecordKind::Directory)
        } else if entry_type.is_file() {
            Ok(RecordKind::File)
        } else if entry_type.is_symlink() {
            Err(preflight_error("symbolic_link_rejected"))
        } else if entry_type.is_hard_link() {
            Err(preflight_error("hard_link_rejected"))
        } else {
            Err(preflight_error("special_entry_rejected"))
        }
    }

    fn register_prefixes(
        canonical: &mut BTreeMap<Vec<String>, CanonicalEntry>,
        parts: &[String],
        member_kind: &RecordKind,
        limits: Limits,
        path_budget: &mut PathBudget,
    ) -> Result<(), AdapterError> {
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
                        return Err(preflight_error("canonical_prefix_budget_exceeded"));
                    }
                    path_budget.charge(
                        path_payload_bytes(&prefix)
                            .checked_add(path_payload_bytes(&key))
                            .ok_or_else(|| preflight_error("path_bookkeeping_budget_exceeded"))?,
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
                    if existing.kind != kind {
                        return Err(preflight_error("file_directory_conflict"));
                    }
                    if existing.original != prefix {
                        return Err(preflight_error("filesystem_collision"));
                    }
                    if explicit && existing.explicit {
                        return Err(preflight_error("duplicate_path"));
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

    fn path_payload_bytes(parts: &[String]) -> usize {
        parts.iter().map(String::len).sum()
    }

    struct PathBudget {
        limit: usize,
        consumed: usize,
    }

    impl PathBudget {
        fn new(limit: usize) -> Self {
            Self { limit, consumed: 0 }
        }

        fn charge(&mut self, bytes: usize) -> Result<(), AdapterError> {
            self.consumed = self
                .consumed
                .checked_add(bytes)
                .ok_or_else(|| preflight_error("path_bookkeeping_budget_exceeded"))?;
            if self.consumed > self.limit {
                return Err(preflight_error("path_bookkeeping_budget_exceeded"));
            }
            Ok(())
        }
    }

    fn hash_entry(entry: &mut impl Read, expected_size: u64) -> Result<[u8; 32], AdapterError> {
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; COPY_CHUNK_BYTES];
        let mut consumed = 0_u64;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|_| preflight_error("member_read_failed"))?;
            if read == 0 {
                break;
            }
            consumed = consumed
                .checked_add(read as u64)
                .ok_or_else(|| preflight_error("member_size_invalid"))?;
            digest.update(&buffer[..read]);
        }
        if consumed != expected_size {
            return Err(preflight_error("member_size_invalid"));
        }
        Ok(digest.finalize().into())
    }

    fn materialize(
        owned_archive: &[u8],
        preflight: &Preflight,
        root: &Path,
        limits: Limits,
    ) -> Result<(), AdapterError> {
        let decoder = GzDecoder::new(Cursor::new(owned_archive));
        let reader = BudgetReader::new(decoder, limits.max_decompressed_tar_bytes);
        let mut archive = Archive::new(reader);
        let entries = archive.entries().map_err(|_| materialization_error())?;
        let mut record_index = 0_usize;
        for entry in entries {
            let mut entry = entry.map_err(|_| materialization_error())?;
            let record = preflight
                .records
                .get(record_index)
                .ok_or_else(materialization_error)?;
            record_index += 1;
            let parts =
                checked_parts(&entry.path_bytes(), limits).map_err(|_| materialization_error())?;
            let kind =
                supported_kind(entry.header().entry_type()).map_err(|_| materialization_error())?;
            let size = entry.header().size().map_err(|_| materialization_error())?;
            let mode = entry.header().mode().map_err(|_| materialization_error())? & 0o777;
            if parts != record.parts
                || kind != record.kind
                || size != record.size
                || mode != record.mode
            {
                return Err(materialization_error());
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
                    output.sync_all().map_err(|_| materialization_error())?;
                    if Some(digest) != record.digest {
                        return Err(materialization_error());
                    }
                    set_mode(&destination, safe_file_mode(mode))?;
                }
            }
        }
        if record_index != preflight.records.len() {
            return Err(materialization_error());
        }
        let mut reader = archive.into_inner();
        io::copy(&mut reader, &mut io::sink()).map_err(|_| materialization_error())?;
        let decoder = reader.into_inner();
        if decoder.get_ref().position() != owned_archive.len() as u64 {
            return Err(materialization_error());
        }
        Ok(())
    }

    fn ensure_directory(root: &Path, parts: &[String]) -> Result<(), AdapterError> {
        let mut current = root.to_path_buf();
        for part in parts {
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(materialization_error());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|_| materialization_error())?;
                    set_mode(&current, 0o700)?;
                }
                Err(_) => return Err(materialization_error()),
            }
        }
        Ok(())
    }

    fn copy_file_entry(
        entry: &mut impl Read,
        output: &mut File,
        expected_size: u64,
    ) -> Result<[u8; 32], AdapterError> {
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; COPY_CHUNK_BYTES];
        let mut copied = 0_u64;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|_| materialization_error())?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or_else(materialization_error)?;
            output
                .write_all(&buffer[..read])
                .map_err(|_| materialization_error())?;
            digest.update(&buffer[..read]);
        }
        if copied != expected_size {
            return Err(materialization_error());
        }
        Ok(digest.finalize().into())
    }

    fn verify_staged_tree(root: &Path, preflight: &Preflight) -> Result<(), AdapterError> {
        let mut actual = BTreeMap::new();
        collect_paths(root, root, &mut actual)?;
        if actual != preflight.expected_paths {
            return Err(AdapterError::new(
                FailureBoundary::Verification,
                "staged_tree_identity_mismatch",
            ));
        }
        for record in &preflight.records {
            let path = join_parts(root, &record.parts);
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                AdapterError::new(
                    FailureBoundary::Verification,
                    "staged_tree_identity_mismatch",
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AdapterError::new(
                    FailureBoundary::Verification,
                    "staged_tree_identity_mismatch",
                ));
            }
            if record.kind == RecordKind::File {
                let bytes = fs::read(path).map_err(|_| {
                    AdapterError::new(
                        FailureBoundary::Verification,
                        "staged_tree_identity_mismatch",
                    )
                })?;
                if metadata.len() != record.size
                    || Some(<[u8; 32]>::from(Sha256::digest(bytes))) != record.digest
                {
                    return Err(AdapterError::new(
                        FailureBoundary::Verification,
                        "staged_tree_identity_mismatch",
                    ));
                }
            }
        }
        Ok(())
    }

    fn collect_paths(
        root: &Path,
        directory: &Path,
        paths: &mut BTreeMap<Vec<String>, RecordKind>,
    ) -> Result<(), AdapterError> {
        for entry in fs::read_dir(directory).map_err(|_| verification_error())? {
            let entry = entry.map_err(|_| verification_error())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| verification_error())?;
            if metadata.file_type().is_symlink() {
                return Err(verification_error());
            }
            let relative = path.strip_prefix(root).map_err(|_| verification_error())?;
            let parts = relative
                .components()
                .map(|component| {
                    component
                        .as_os_str()
                        .to_str()
                        .map(str::to_owned)
                        .ok_or_else(verification_error)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let kind = if metadata.is_dir() {
                RecordKind::Directory
            } else if metadata.is_file() {
                RecordKind::File
            } else {
                return Err(verification_error());
            };
            if paths.insert(parts, kind.clone()).is_some() {
                return Err(verification_error());
            }
            if kind == RecordKind::Directory {
                collect_paths(root, &path, paths)?;
            }
        }
        Ok(())
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

    struct PrivateRoot {
        path: Option<PathBuf>,
        fail_cleanup_once: bool,
    }

    impl PrivateRoot {
        fn create(fail_cleanup_once: bool) -> Result<Self, AdapterError> {
            for _ in 0..32 {
                let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let root = std::env::temp_dir().join(format!(
                    "batcave-updater-stream-{}-{nanos}-{sequence}",
                    std::process::id()
                ));
                match create_private_directory(&root) {
                    Ok(()) => {
                        return Ok(Self {
                            path: Some(root),
                            fail_cleanup_once,
                        })
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        return Err(AdapterError::new(
                            FailureBoundary::Materialization,
                            "private_staging_root_failed",
                        ));
                    }
                }
            }
            Err(AdapterError::new(
                FailureBoundary::Materialization,
                "private_staging_root_failed",
            ))
        }

        fn path(&self) -> &Path {
            self.path.as_deref().expect("private root is owned")
        }

        fn take(&mut self) -> Option<PathBuf> {
            self.path.take()
        }

        fn cleanup(&mut self) -> Result<(), AdapterError> {
            let Some(root) = self.path.as_ref() else {
                return Ok(());
            };
            if self.fail_cleanup_once {
                self.fail_cleanup_once = false;
                return Err(AdapterError::new(
                    FailureBoundary::Cleanup,
                    "staging_cleanup_after_failure_failed",
                ));
            }
            fs::remove_dir_all(root).map_err(|_| {
                AdapterError::new(
                    FailureBoundary::Cleanup,
                    "staging_cleanup_after_failure_failed",
                )
            })?;
            self.path.take();
            Ok(())
        }

        fn abort(mut self, primary: AdapterError) -> AdapterError {
            match self.cleanup() {
                Ok(()) => primary,
                Err(cleanup) => cleanup,
            }
        }
    }

    impl Drop for PrivateRoot {
        fn drop(&mut self) {
            let _ = self.cleanup();
        }
    }

    #[cfg(unix)]
    fn create_private_directory(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(path)
    }

    #[cfg(not(unix))]
    fn create_private_directory(path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn join_parts(root: &Path, parts: &[String]) -> PathBuf {
        parts
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn safe_directory_mode(mode: u32) -> u32 {
        (mode & 0o755) | 0o500
    }

    fn safe_file_mode(mode: u32) -> u32 {
        (mode & 0o755) | 0o400
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) -> Result<(), AdapterError> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|_| materialization_error())
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) -> Result<(), AdapterError> {
        Ok(())
    }

    #[cfg(unix)]
    fn create_new_file(path: &Path) -> Result<File, AdapterError> {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| materialization_error())
    }

    #[cfg(not(unix))]
    fn create_new_file(path: &Path) -> Result<File, AdapterError> {
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| materialization_error())
    }

    fn preflight_error(code: &'static str) -> AdapterError {
        AdapterError::new(FailureBoundary::Preflight, code)
    }

    fn materialization_error() -> AdapterError {
        AdapterError::new(
            FailureBoundary::Materialization,
            "staging_materialization_failed",
        )
    }

    fn verification_error() -> AdapterError {
        AdapterError::new(
            FailureBoundary::Verification,
            "staged_tree_identity_mismatch",
        )
    }

    fn hex_digest(digest: [u8; 32]) -> String {
        let mut output = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }
}

#[derive(Clone, Copy)]
enum FixtureKind {
    Directory,
    File,
    Symlink,
    Hardlink,
    Device,
}

struct FixtureEntry<'a> {
    name: &'a str,
    kind: FixtureKind,
    bytes: &'a [u8],
}

fn valid_archive() -> Vec<u8> {
    archive(&[
        FixtureEntry {
            name: APP_NAME,
            kind: FixtureKind::Directory,
            bytes: b"",
        },
        FixtureEntry {
            name: "BatCave Monitor.app/Contents",
            kind: FixtureKind::Directory,
            bytes: b"",
        },
        FixtureEntry {
            name: FIXTURE_FILE,
            kind: FixtureKind::File,
            bytes: FIXTURE_BYTES,
        },
    ])
}

fn archive(entries: &[FixtureEntry<'_>]) -> Vec<u8> {
    let encoder: GzEncoder<Vec<u8>> = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    for fixture in entries {
        let mut header = Header::new_gnu();
        let entry_type = match fixture.kind {
            FixtureKind::Directory => EntryType::Directory,
            FixtureKind::File => EntryType::Regular,
            FixtureKind::Symlink => EntryType::Symlink,
            FixtureKind::Hardlink => EntryType::Link,
            FixtureKind::Device => EntryType::Char,
        };
        header.set_entry_type(entry_type);
        header.set_mode(if matches!(fixture.kind, FixtureKind::Directory) {
            0o755
        } else {
            0o644
        });
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(if matches!(fixture.kind, FixtureKind::File) {
            fixture.bytes.len() as u64
        } else {
            0
        });
        set_raw_name(&mut header, fixture.name.as_bytes());
        if matches!(fixture.kind, FixtureKind::Symlink | FixtureKind::Hardlink) {
            header
                .set_link_name("outside")
                .expect("fixture link name is valid");
        }
        if matches!(fixture.kind, FixtureKind::Device) {
            header.set_device_major(1).expect("fixture device major");
            header.set_device_minor(3).expect("fixture device minor");
        }
        header.set_cksum();
        builder
            .append(&header, Cursor::new(fixture.bytes))
            .expect("append fixture archive entry");
    }
    let encoder = builder.into_inner().expect("finish fixture tar stream");
    encoder.finish().expect("finish fixture gzip stream")
}

fn gzip_member(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("write gzip member");
    encoder.finish().expect("finish gzip member")
}

fn set_raw_name(header: &mut Header, name: &[u8]) {
    assert!(
        name.len() <= 100,
        "fixture path must fit the GNU name field"
    );
    let bytes = header.as_mut_bytes();
    bytes[..100].fill(0);
    bytes[..name.len()].copy_from_slice(name);
}

fn assert_non_claims(outcome: &adapter::Outcome) {
    assert!(!outcome.package_installed);
    assert!(!outcome.app_launched);
    assert!(!outcome.trust_verified);
    assert!(!outcome.native_proven);
    assert!(!outcome.receipt_emitted);
    assert!(!outcome.evidence_emitted);
    assert_eq!(
        outcome.limitations,
        ["fixture_only", "macos_updater_staging_only"]
    );
}

#[test]
fn owned_stream_preflights_stages_verifies_and_cleans() {
    let archive = valid_archive();
    let expected_digest = format!("{:x}", Sha256::digest(&archive));
    let mut authority =
        adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("acquire");
    let mut staged = authority.stage().expect("stage owned stream");
    let completion = staged.complete();
    assert!(authority.accepts_completion(&completion));

    let outcome = completion.outcome();
    assert_eq!(outcome.disposition, adapter::Disposition::PrototypeStaged);
    assert_eq!(outcome.archive_sha256, expected_digest);
    assert_eq!(outcome.member_count, 3);
    assert!(outcome.archive_stream_consumed);
    assert!(outcome.preflight_passed);
    assert!(outcome.app_staged);
    assert!(outcome.staged_tree_verified);
    assert!(outcome.cleanup_passed);
    assert!(!outcome.residue_retained);
    assert_non_claims(outcome);
}

#[test]
fn source_replacement_cannot_change_the_owned_archive() {
    let mut source = valid_archive();
    let plan = adapter::closed_fixture_plan(&source);
    let mut authority = adapter::acquire(plan, &source).expect("acquire owned copy");
    source.fill(0);

    let mut staged = authority.stage().expect("stage original owned bytes");
    let completion = staged.complete();
    assert_eq!(
        completion.outcome().disposition,
        adapter::Disposition::PrototypeStaged
    );
    assert_non_claims(completion.outcome());
}

#[test]
fn mismatched_bytes_replay_and_cross_operation_completion_fail_closed() {
    let archive = valid_archive();
    let mut different = archive.clone();
    different[0] ^= 0xff;
    let error = match adapter::acquire(adapter::closed_fixture_plan(&archive), &different) {
        Ok(_) => panic!("mismatched archive must fail"),
        Err(error) => error,
    };
    assert_eq!(error.boundary(), adapter::FailureBoundary::Acquisition);
    assert_eq!(error.code(), "selected_archive_mismatch");

    let mut first =
        adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("first");
    let mut staged = first.stage().expect("first stage");
    let replay = match first.stage() {
        Ok(_) => panic!("replay must fail"),
        Err(error) => error,
    };
    assert_eq!(replay.boundary(), adapter::FailureBoundary::Authority);
    assert_eq!(replay.code(), "archive_authority_replayed");
    let completion = staged.complete();

    let second =
        adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("second");
    assert!(!second.accepts_completion(&completion));
}

#[test]
fn hostile_archive_entries_fail_before_staging() {
    let cases = [
        (
            "BatCave Monitor.app/../outside",
            FixtureKind::File,
            "archive_path_not_canonical",
        ),
        (
            "/BatCave Monitor.app/Contents/file",
            FixtureKind::File,
            "archive_path_not_relative",
        ),
        (
            "BatCave Monitor.app/Contents\\file",
            FixtureKind::File,
            "archive_path_not_relative",
        ),
        (
            "BatCave Monitor.app/Contents/link",
            FixtureKind::Symlink,
            "symbolic_link_rejected",
        ),
        (
            "BatCave Monitor.app/Contents/link",
            FixtureKind::Hardlink,
            "hard_link_rejected",
        ),
        (
            "BatCave Monitor.app/Contents/device",
            FixtureKind::Device,
            "special_entry_rejected",
        ),
    ];
    for (name, kind, expected_code) in cases {
        let archive = archive(&[
            FixtureEntry {
                name: APP_NAME,
                kind: FixtureKind::Directory,
                bytes: b"",
            },
            FixtureEntry {
                name,
                kind,
                bytes: b"fixture",
            },
        ]);
        let mut authority =
            adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("acquire");
        let error = match authority.stage() {
            Ok(_) => panic!("hostile archive must fail: {name}"),
            Err(error) => error,
        };
        assert_eq!(error.boundary(), adapter::FailureBoundary::Preflight);
        assert_eq!(error.code(), expected_code);
    }
}

#[test]
fn roots_nested_apps_duplicates_and_macos_collisions_fail_closed() {
    let cases: Vec<(Vec<FixtureEntry<'_>>, &str)> = vec![
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "README.txt",
                    kind: FixtureKind::File,
                    bytes: b"readme",
                },
            ],
            "unexpected_archive_root",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Other.app/file",
                    kind: FixtureKind::File,
                    bytes: b"nested",
                },
            ],
            "nested_app_rejected",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: FIXTURE_FILE,
                    kind: FixtureKind::File,
                    bytes: b"one",
                },
                FixtureEntry {
                    name: FIXTURE_FILE,
                    kind: FixtureKind::File,
                    bytes: b"two",
                },
            ],
            "duplicate_path",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Contents/A",
                    kind: FixtureKind::File,
                    bytes: b"one",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/contents/B",
                    kind: FixtureKind::File,
                    bytes: b"two",
                },
            ],
            "filesystem_collision",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Stra\u{df}e/A",
                    kind: FixtureKind::File,
                    bytes: b"one",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/STRASSE/B",
                    kind: FixtureKind::File,
                    bytes: b"two",
                },
            ],
            "filesystem_collision",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Re\u{301}sources/A",
                    kind: FixtureKind::File,
                    bytes: b"one",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/R\u{e9}sources/B",
                    kind: FixtureKind::File,
                    bytes: b"two",
                },
            ],
            "filesystem_collision",
        ),
        (
            vec![
                FixtureEntry {
                    name: APP_NAME,
                    kind: FixtureKind::Directory,
                    bytes: b"",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Contents/node",
                    kind: FixtureKind::File,
                    bytes: b"file",
                },
                FixtureEntry {
                    name: "BatCave Monitor.app/Contents/Node/child",
                    kind: FixtureKind::File,
                    bytes: b"child",
                },
            ],
            "file_directory_conflict",
        ),
    ];
    for (entries, expected_code) in cases {
        let archive = archive(&entries);
        let mut authority =
            adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("acquire");
        let error = match authority.stage() {
            Ok(_) => panic!("hostile archive must fail with {expected_code}"),
            Err(error) => error,
        };
        assert_eq!(error.boundary(), adapter::FailureBoundary::Preflight);
        assert_eq!(error.code(), expected_code);
    }
}

#[test]
fn compressed_member_and_expanded_budgets_fail_closed() {
    let archive = valid_archive();
    let cases = [
        (
            adapter::Limits::production().with_compressed_limit(archive.len() - 1),
            "compressed_budget_exceeded",
        ),
        (
            adapter::Limits::production().with_member_limit(2),
            "member_budget_exceeded",
        ),
        (
            adapter::Limits::production().with_expanded_limit(FIXTURE_BYTES.len() as u64 - 1),
            "expanded_budget_exceeded",
        ),
    ];
    for (limits, expected_code) in cases {
        let mut authority =
            adapter::acquire(adapter::limited_fixture_plan(&archive, limits), &archive)
                .expect("acquire");
        let error = match authority.stage() {
            Ok(_) => panic!("budget must fail with {expected_code}"),
            Err(error) => error,
        };
        assert_eq!(error.boundary(), adapter::FailureBoundary::Preflight);
        assert_eq!(error.code(), expected_code);
    }
}

#[test]
fn gzip_trailer_trailing_bytes_and_second_members_fail_before_staging() {
    let valid = valid_archive();
    let mut invalid_trailer = valid.clone();
    let trailer = invalid_trailer.len() - 8;
    invalid_trailer[trailer] ^= 0xff;

    let mut trailing_bytes = valid.clone();
    trailing_bytes.extend_from_slice(b"trailing");

    let mut second_member = valid;
    second_member.extend_from_slice(&gzip_member(b"second gzip member"));

    for (archive, expected_code) in [
        (invalid_trailer, "archive_stream_invalid"),
        (trailing_bytes, "archive_trailing_data"),
        (second_member, "archive_trailing_data"),
    ] {
        let mut authority =
            adapter::acquire(adapter::closed_fixture_plan(&archive), &archive).expect("acquire");
        let error = match authority.stage() {
            Ok(_) => panic!("gzip integrity failure must not stage"),
            Err(error) => error,
        };
        assert_eq!(error.boundary(), adapter::FailureBoundary::Preflight);
        assert_eq!(error.code(), expected_code);
    }
}

#[test]
fn path_budget_counts_retained_record_and_canonical_prefix_clones() {
    let archive = archive(&[
        FixtureEntry {
            name: APP_NAME,
            kind: FixtureKind::Directory,
            bytes: b"",
        },
        FixtureEntry {
            name: "BatCave Monitor.app/a/b/c/d/e/f/g/h/i/j/k/l/m/n/fixture",
            kind: FixtureKind::File,
            bytes: b"fixture",
        },
    ]);
    let limits = adapter::Limits::production().with_path_bookkeeping_limit(128);
    let mut authority = adapter::acquire(adapter::limited_fixture_plan(&archive, limits), &archive)
        .expect("acquire");
    let error = match authority.stage() {
        Ok(_) => panic!("retained canonical-prefix clones must consume the path budget"),
        Err(error) => error,
    };
    assert_eq!(error.boundary(), adapter::FailureBoundary::Preflight);
    assert_eq!(error.code(), "path_bookkeeping_budget_exceeded");
}

#[test]
fn materialization_collision_fails_without_a_completion() {
    let archive = valid_archive();
    let mut authority = adapter::acquire(
        adapter::materialization_collision_fixture_plan(&archive),
        &archive,
    )
    .expect("acquire");
    let error = match authority.stage() {
        Ok(_) => panic!("materialization collision must fail"),
        Err(error) => error,
    };
    assert_eq!(error.boundary(), adapter::FailureBoundary::Materialization);
    assert_eq!(error.code(), "staging_materialization_failed");
}

#[test]
fn materialization_and_verification_cleanup_failures_are_reported() {
    let archive = valid_archive();
    for plan in [
        adapter::materialization_cleanup_failure_fixture_plan(&archive),
        adapter::verification_cleanup_failure_fixture_plan(&archive),
    ] {
        let mut authority = adapter::acquire(plan, &archive).expect("acquire");
        let error = match authority.stage() {
            Ok(_) => panic!("stage failure with failed cleanup must not be hidden"),
            Err(error) => error,
        };
        assert_eq!(error.boundary(), adapter::FailureBoundary::Cleanup);
        assert_eq!(error.code(), "staging_cleanup_after_failure_failed");
    }
}

#[test]
fn cleanup_failure_retains_state_and_retry_removes_it() {
    let archive = valid_archive();
    let mut authority = adapter::acquire(adapter::cleanup_failure_fixture_plan(&archive), &archive)
        .expect("acquire");
    let mut staged = authority.stage().expect("stage");
    let retained = staged.complete();
    assert!(authority.accepts_completion(&retained));
    assert_eq!(
        retained.outcome().disposition,
        adapter::Disposition::RetainedCleanupFailed
    );
    assert!(!retained.outcome().cleanup_passed);
    assert!(retained.outcome().residue_retained);
    assert_non_claims(retained.outcome());

    let recovered = staged.retry_cleanup();
    assert!(authority.accepts_completion(&recovered));
    assert_eq!(
        recovered.outcome().disposition,
        adapter::Disposition::PrototypeStaged
    );
    assert!(recovered.outcome().cleanup_passed);
    assert!(!recovered.outcome().residue_retained);
    assert_non_claims(recovered.outcome());
}

#[test]
fn probe_has_no_production_javascript_or_cli_entrypoint() {
    let manifest_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let production_lib =
        std::fs::read_to_string(manifest_root.join("src/lib.rs")).expect("read production library");
    assert!(!production_lib.contains("macos_updater_owned_stream_transport_spike"));

    let repository_root = manifest_root
        .ancestors()
        .nth(3)
        .expect("manifest is nested below repository root");
    for candidate in [
        "scripts/macos-updater-owned-stream-transport.mjs",
        "scripts/macos-updater-owned-stream-transport.test.mjs",
    ] {
        assert!(!repository_root.join(candidate).exists());
    }
}
