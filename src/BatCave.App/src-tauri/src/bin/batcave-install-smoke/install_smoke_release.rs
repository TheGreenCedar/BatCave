use reqwest::blocking::{Client, Response};
use reqwest::header::HeaderMap;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore_verify::trust_root::{
    SigstoreInstance, TrustedRoot, SIGSTORE_PRODUCTION_TRUSTED_ROOT,
};
use sigstore_verify::types::{Bundle, Sha256Hash, SignatureContent};
use sigstore_verify::{verify, VerificationPolicy};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::{fs::OpenOptionsExt, prelude::FileExt, prelude::MetadataExt},
};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const REPOSITORY: &str = "TheGreenCedar/BatCave";
const REPOSITORY_ID: &str = "1196390305";
const OWNER_ID: &str = "14635636";
const SOURCE_REF: &str = "refs/heads/main";
const WORKFLOW_PATH: &str = ".github/workflows/release.yml";
const WORKFLOW_IDENTITY: &str =
    "https://github.com/TheGreenCedar/BatCave/.github/workflows/release.yml@refs/heads/main";
const ACTIONS_ISSUER: &str = "https://token.actions.githubusercontent.com";
const RELEASE_IDENTITY: &str = "https://dotcom.releases.github.com";
const CHECKSUMS: &str = "SHA256SUMS.txt";
const MAX_RELEASE_BYTES: u64 = 1_073_741_824;
const MAX_API_JSON_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ATTESTATION_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ATTESTATION_ENTRIES: usize = 100;
const MAX_REDIRECTS: usize = 5;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Profile {
    WindowsNsis,
    LinuxDeb,
    LinuxAppImage,
    MacOsDmg,
    MacOsUpdater,
}

impl Profile {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "windows-nsis" => Some(Self::WindowsNsis),
            "linux-deb" => Some(Self::LinuxDeb),
            "linux-appimage" => Some(Self::LinuxAppImage),
            "macos-dmg" => Some(Self::MacOsDmg),
            "macos-updater" => Some(Self::MacOsUpdater),
            _ => None,
        }
    }

    fn selected_name(self, version: &str) -> String {
        match self {
            Self::WindowsNsis => format!("BatCave.Monitor_{version}_x64-setup.exe"),
            Self::LinuxDeb => format!("BatCave.Monitor_{version}_amd64.deb"),
            Self::LinuxAppImage => format!("BatCave.Monitor_{version}_amd64.AppImage"),
            Self::MacOsDmg => format!("BatCave.Monitor_{version}_universal.dmg"),
            Self::MacOsUpdater => "BatCave.Monitor.app.tar.gz".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub(super) struct SanitizedOutcome {
    disposition: &'static str,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<MacOsUpdaterObservation>,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
struct MacOsUpdaterObservation {
    schema_version: u8,
    result_kind: &'static str,
    proof_scope: &'static str,
    release_evidence_eligible: bool,
    repository: &'static str,
    release: MacOsUpdaterReleaseIdentity,
    artifact: MacOsUpdaterArtifactObservation,
    observed_checks: MacOsUpdaterObservedChecks,
    limitations: [&'static str; 7],
}

#[derive(Debug, Serialize, Eq, PartialEq)]
struct MacOsUpdaterReleaseIdentity {
    tag: String,
    source_sha: String,
    app_version: String,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
struct MacOsUpdaterArtifactObservation {
    name: String,
    size_bytes: u64,
    sha256: String,
    updater_signature_name: String,
    updater_signature_sha256: String,
    staged_member_count: usize,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
struct MacOsUpdaterObservedChecks {
    anonymous_public_bytes: &'static str,
    checksum_manifest: &'static str,
    source_bound_attestations: &'static str,
    updater_signature: &'static str,
    exact_owned_stream: &'static str,
    archive_preflight: &'static str,
    staged_tree_reverification: &'static str,
    private_root_cleanup: &'static str,
}

impl SanitizedOutcome {
    fn failed(reason: &'static str) -> Self {
        Self {
            disposition: "failed",
            reason,
            observation: None,
        }
    }

    pub(super) fn skipped() -> Self {
        Self {
            disposition: "skipped",
            reason: "native_platform_not_implemented",
            observation: None,
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn macos_updater_observed(
        identity: &MacOsUpdaterArtifactIdentity,
        staged_member_count: usize,
    ) -> Self {
        Self {
            disposition: "observation_complete",
            reason: "exact_updater_archive_staged_and_cleaned",
            observation: Some(MacOsUpdaterObservation {
                schema_version: 1,
                result_kind: "macos_updater_post_public_observation",
                proof_scope: "post_public_macos_updater_staging_observation_only",
                release_evidence_eligible: false,
                repository: REPOSITORY,
                release: MacOsUpdaterReleaseIdentity {
                    tag: identity.tag.clone(),
                    source_sha: identity.source_sha.clone(),
                    app_version: identity.version.clone(),
                },
                artifact: MacOsUpdaterArtifactObservation {
                    name: identity.asset_name.clone(),
                    size_bytes: identity.size,
                    sha256: identity.digest.clone(),
                    updater_signature_name: identity.signature_name.clone(),
                    updater_signature_sha256: identity.signature_digest.clone(),
                    staged_member_count,
                },
                observed_checks: MacOsUpdaterObservedChecks {
                    anonymous_public_bytes: "passed",
                    checksum_manifest: "passed",
                    source_bound_attestations: "passed",
                    updater_signature: "passed",
                    exact_owned_stream: "passed",
                    archive_preflight: "passed",
                    staged_tree_reverification: "passed",
                    private_root_cleanup: "passed",
                },
                limitations: [
                    "github_hosted_macos_15",
                    "universal_updater_archive_staging_only",
                    "application_not_installed_or_launched",
                    "developer_id_notarization_and_staple_not_rechecked",
                    "runtime_settings_telemetry_and_degradation_not_exercised",
                    "updater_a_to_b_not_exercised",
                    "not_release_evidence",
                ],
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ReleaseAsset {
    id: u64,
    name: String,
    size: u64,
    digest: String,
    browser_download_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ReleaseReadback {
    id: u64,
    tag_name: String,
    target_commitish: String,
    draft: bool,
    prerelease: bool,
    immutable: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Failure {
    InvalidRequest,
    Offline,
    Timeout,
    Redirect,
    ReleaseRejected,
    InventoryRejected,
    DownloadRejected,
    ChecksumRejected,
    AttestationRejected,
    ReadbackDrift,
    CleanupFailed,
    AuthorityRejected,
}

trait PublicSource {
    fn release(&self, tag: &str) -> Result<ReleaseReadback, Failure>;
    fn tag_commit(&self, tag: &str) -> Result<String, Failure>;
    fn release_attestations(&self, commit: &str) -> Result<Vec<Vec<u8>>, Failure>;
    fn download(&self, url: &str, limit: u64, output: &mut File) -> Result<u64, Failure>;
}

trait AttestationTrust {
    fn verify_build(
        &self,
        bundle: &[u8],
        subjects: &BTreeMap<String, String>,
        source_sha: &str,
    ) -> Result<(), Failure>;

    fn verify_release(
        &self,
        bundles: &[Vec<u8>],
        tag: &str,
        commit: &str,
        release: &ReleaseReadback,
        selected_digest: &str,
    ) -> Result<(), Failure>;
}

pub(super) fn run(selectors: &[String]) -> (SanitizedOutcome, i32) {
    if selectors.len() != 2 {
        return (SanitizedOutcome::failed("invalid_request"), 2);
    }
    let source = match GitHubSource::new() {
        Ok(source) => source,
        Err(failure) => return public_failure(failure),
    };
    let trust = SigstoreTrust;
    match verify_and_bind(&selectors[0], &selectors[1], &source, &trust, false)
        .and_then(dispatch_verified)
    {
        Ok(outcome) => (outcome, 0),
        Err(failure) => public_failure(failure),
    }
}

fn dispatch_verified(verified: VerifiedArtifact) -> Result<SanitizedOutcome, Failure> {
    match verified.profile {
        #[cfg(target_os = "linux")]
        Profile::LinuxDeb | Profile::LinuxAppImage => {
            super::install_smoke_linux::run(verified.into_linux()?)
        }
        #[cfg(target_os = "macos")]
        Profile::MacOsUpdater => {
            super::install_smoke_macos_updater::run(verified.into_macos_updater()?)
        }
        Profile::WindowsNsis | Profile::MacOsDmg => verified.finish_without_native(),
        #[cfg(not(target_os = "macos"))]
        Profile::MacOsUpdater => verified.finish_without_native(),
        #[cfg(not(target_os = "linux"))]
        Profile::LinuxDeb | Profile::LinuxAppImage => verified.finish_without_native(),
    }
}

fn public_failure(failure: Failure) -> (SanitizedOutcome, i32) {
    let reason = match failure {
        Failure::InvalidRequest => "invalid_request",
        Failure::Offline => "offline",
        Failure::Timeout => "timeout",
        Failure::CleanupFailed => "cleanup_failed",
        _ => "public_release_verification_failed",
    };
    (SanitizedOutcome::failed(reason), 1)
}

#[cfg(test)]
fn execute(
    tag: &str,
    profile: &str,
    source: &dyn PublicSource,
    trust: &dyn AttestationTrust,
    force_cleanup_failure: bool,
) -> Result<SanitizedOutcome, Failure> {
    verify_and_bind(tag, profile, source, trust, force_cleanup_failure)
        .and_then(VerifiedArtifact::finish_without_native)
}

struct VerifiedArtifact {
    profile: Profile,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    tag: String,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    version: String,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    source_sha: String,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    release: ReleaseReadback,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    selected: ReleaseAsset,
    updater_signature: Option<BoundUpdaterSignature>,
    artifact: BoundArtifact,
}

struct BoundUpdaterSignature {
    asset: ReleaseAsset,
    bytes: Vec<u8>,
}

impl VerifiedArtifact {
    pub(super) fn finish_without_native(self) -> Result<SanitizedOutcome, Failure> {
        let Self {
            profile: _,
            tag: _,
            version: _,
            source_sha: _,
            release: _,
            selected: _,
            updater_signature: _,
            artifact,
        } = self;
        artifact.finish_without_native()
    }

    #[cfg(target_os = "linux")]
    fn into_linux(mut self) -> Result<VerifiedLinuxArtifact, Failure> {
        let profile = match self.profile {
            Profile::LinuxDeb => LinuxProfile::Deb,
            Profile::LinuxAppImage => LinuxProfile::AppImage,
            _ => return Err(Failure::AuthorityRejected),
        };
        let expected_name = self.profile.selected_name(&self.version);
        if self.release.tag_name != self.tag
            || self.release.target_commitish != self.source_sha
            || self.selected.name != expected_name
            || self.selected.size == 0
            || self.selected.digest != format!("sha256:{}", digest_value(&self.selected.digest)?)
        {
            return Err(Failure::AuthorityRejected);
        }
        let seal = Arc::clone(&self.artifact.seal);
        let bytes = self.artifact.take(&seal)?;
        if bytes.len() as u64 != self.selected.size
            || digest_hex(bytes.as_ref()) != digest_value(&self.selected.digest)?
        {
            return Err(Failure::AuthorityRejected);
        }
        let identity = LinuxArtifactIdentity {
            tag: self.tag,
            version: self.version,
            source_sha: self.source_sha,
            release_id: self.release.id,
            asset_id: self.selected.id,
            asset_name: self.selected.name,
            size: self.selected.size,
            digest: self.selected.digest,
        };
        VerifiedLinuxArtifact::seal(profile, identity, bytes)
    }

    #[cfg(target_os = "macos")]
    fn into_macos_updater(mut self) -> Result<VerifiedMacOsUpdaterArtifact, Failure> {
        if self.profile != Profile::MacOsUpdater {
            return Err(Failure::AuthorityRejected);
        }
        let signature = self
            .updater_signature
            .take()
            .ok_or(Failure::AuthorityRejected)?;
        let expected_name = self.profile.selected_name(&self.version);
        let expected_signature_name = format!("{expected_name}.sig");
        if self.release.id == 0
            || self.selected.id == 0
            || signature.asset.id == 0
            || self.release.tag_name != self.tag
            || self.release.target_commitish != self.source_sha
            || self.selected.name != expected_name
            || signature.asset.name != expected_signature_name
            || self.selected.size == 0
            || signature.asset.size == 0
            || self.selected.digest != format!("sha256:{}", digest_value(&self.selected.digest)?)
            || signature.asset.digest
                != format!("sha256:{}", digest_value(&signature.asset.digest)?)
            || signature.bytes.len() as u64 != signature.asset.size
            || digest_hex(&signature.bytes) != digest_value(&signature.asset.digest)?
        {
            return Err(Failure::AuthorityRejected);
        }
        let seal = Arc::clone(&self.artifact.seal);
        let bytes = self.artifact.take(&seal)?;
        if bytes.len() as u64 != self.selected.size
            || digest_hex(bytes.as_ref()) != digest_value(&self.selected.digest)?
        {
            return Err(Failure::AuthorityRejected);
        }
        Ok(VerifiedMacOsUpdaterArtifact {
            identity: MacOsUpdaterArtifactIdentity {
                tag: self.tag,
                version: self.version,
                source_sha: self.source_sha,
                release_id: self.release.id,
                asset_id: self.selected.id,
                asset_name: self.selected.name,
                size: self.selected.size,
                digest: self.selected.digest,
                signature_asset_id: signature.asset.id,
                signature_name: signature.asset.name,
                signature_size: signature.asset.size,
                signature_digest: signature.asset.digest,
            },
            bytes,
            signature: signature.bytes,
        })
    }
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsUpdaterArtifactIdentity {
    tag: String,
    version: String,
    source_sha: String,
    release_id: u64,
    asset_id: u64,
    asset_name: String,
    size: u64,
    digest: String,
    signature_asset_id: u64,
    signature_name: String,
    signature_size: u64,
    signature_digest: String,
}

#[cfg(target_os = "macos")]
pub(super) struct VerifiedMacOsUpdaterArtifact {
    pub(super) identity: MacOsUpdaterArtifactIdentity,
    pub(super) bytes: Arc<[u8]>,
    pub(super) signature: Vec<u8>,
}

#[cfg(target_os = "macos")]
impl VerifiedMacOsUpdaterArtifact {
    pub(super) fn revalidate(&self) -> Result<(), Failure> {
        if self.identity.release_id == 0
            || self.identity.asset_id == 0
            || self.identity.signature_asset_id == 0
            || self.identity.asset_name != "BatCave.Monitor.app.tar.gz"
            || self.identity.signature_name != "BatCave.Monitor.app.tar.gz.sig"
            || self.identity.tag != format!("v{}", self.identity.version)
            || !is_sha(&self.identity.source_sha)
            || self.identity.size != self.bytes.len() as u64
            || self.identity.signature_size != self.signature.len() as u64
            || self.identity.digest != format!("sha256:{}", digest_hex(self.bytes.as_ref()))
            || self.identity.signature_digest != format!("sha256:{}", digest_hex(&self.signature))
        {
            return Err(Failure::AuthorityRejected);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinuxProfile {
    Deb,
    AppImage,
}

#[cfg(target_os = "linux")]
impl LinuxProfile {
    fn expected_name(self, version: &str) -> String {
        match self {
            Self::Deb => format!("BatCave.Monitor_{version}_amd64.deb"),
            Self::AppImage => format!("BatCave.Monitor_{version}_amd64.AppImage"),
        }
    }

    fn mode(self) -> libc::mode_t {
        match self {
            Self::Deb => 0o400,
            Self::AppImage => 0o500,
        }
    }
}

#[cfg(target_os = "linux")]
struct LinuxArtifactIdentity {
    tag: String,
    version: String,
    source_sha: String,
    release_id: u64,
    asset_id: u64,
    asset_name: String,
    size: u64,
    digest: String,
}

#[cfg(target_os = "linux")]
pub(super) struct VerifiedLinuxArtifact {
    profile: LinuxProfile,
    identity: LinuxArtifactIdentity,
    descriptor: File,
    bytes: Arc<[u8]>,
    sha256: [u8; 32],
    device: u64,
    inode: u64,
    required_seals: libc::c_int,
}

#[cfg(target_os = "linux")]
impl VerifiedLinuxArtifact {
    fn seal(
        profile: LinuxProfile,
        identity: LinuxArtifactIdentity,
        bytes: Arc<[u8]>,
    ) -> Result<Self, Failure> {
        if bytes.is_empty() || bytes.len() as u64 != identity.size {
            return Err(Failure::AuthorityRejected);
        }
        let name = CString::new("batcave-install-smoke-artifact")
            .expect("fixed memfd name contains no NUL");
        let raw_fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if raw_fd < 0 {
            return Err(Failure::AuthorityRejected);
        }
        let mut writable = unsafe { File::from_raw_fd(raw_fd) };
        writable
            .write_all(&bytes)
            .map_err(|_| Failure::AuthorityRejected)?;
        writable.flush().map_err(|_| Failure::AuthorityRejected)?;
        if unsafe { libc::fchmod(raw_fd, profile.mode()) } != 0 {
            return Err(Failure::AuthorityRejected);
        }
        let required_seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(raw_fd, libc::F_ADD_SEALS, required_seals) } != 0 {
            return Err(Failure::AuthorityRejected);
        }
        let writable_metadata = writable
            .metadata()
            .map_err(|_| Failure::AuthorityRejected)?;
        let descriptor = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{raw_fd}"))
            .map_err(|_| Failure::AuthorityRejected)?;
        let descriptor_metadata = descriptor
            .metadata()
            .map_err(|_| Failure::AuthorityRejected)?;
        if descriptor_metadata.dev() != writable_metadata.dev()
            || descriptor_metadata.ino() != writable_metadata.ino()
            || descriptor_metadata.len() != identity.size
        {
            return Err(Failure::AuthorityRejected);
        }
        verify_linux_descriptor(&descriptor, required_seals)?;
        drop(writable);
        let artifact = Self {
            profile,
            identity,
            descriptor,
            sha256: Sha256::digest(bytes.as_ref()).into(),
            bytes,
            device: descriptor_metadata.dev(),
            inode: descriptor_metadata.ino(),
            required_seals,
        };
        artifact.revalidate()?;
        Ok(artifact)
    }

    pub(super) fn revalidate(&self) -> Result<(), Failure> {
        if self.identity.release_id == 0
            || self.identity.asset_id == 0
            || self.identity.asset_name != self.profile.expected_name(&self.identity.version)
            || self.identity.tag != format!("v{}", self.identity.version)
            || !is_sha(&self.identity.source_sha)
            || self.identity.size != self.bytes.len() as u64
            || self.identity.digest != format!("sha256:{}", digest_hex(self.bytes.as_ref()))
        {
            return Err(Failure::AuthorityRejected);
        }
        let metadata = self
            .descriptor
            .metadata()
            .map_err(|_| Failure::AuthorityRejected)?;
        if metadata.dev() != self.device
            || metadata.ino() != self.inode
            || metadata.len() != self.identity.size
            || !metadata.is_file()
            || metadata.mode() & 0o777 != self.profile.mode()
        {
            return Err(Failure::AuthorityRejected);
        }
        verify_linux_descriptor(&self.descriptor, self.required_seals)?;
        let before = unsafe { libc::lseek(self.descriptor.as_raw_fd(), 0, libc::SEEK_CUR) };
        if before < 0 {
            return Err(Failure::AuthorityRejected);
        }
        let mut hash = Sha256::new();
        let mut offset = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        while offset < self.identity.size {
            let read = self
                .descriptor
                .read_at(&mut buffer, offset)
                .map_err(|_| Failure::AuthorityRejected)?;
            if read == 0 {
                return Err(Failure::AuthorityRejected);
            }
            offset = offset
                .checked_add(read as u64)
                .ok_or(Failure::AuthorityRejected)?;
            hash.update(&buffer[..read]);
        }
        let after = unsafe { libc::lseek(self.descriptor.as_raw_fd(), 0, libc::SEEK_CUR) };
        let observed: [u8; 32] = hash.finalize().into();
        if offset != self.identity.size || observed != self.sha256 || before != after {
            return Err(Failure::AuthorityRejected);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn verify_linux_descriptor(descriptor: &File, required_seals: libc::c_int) -> Result<(), Failure> {
    let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFL) };
    let seals = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GET_SEALS) };
    if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY || seals != required_seals {
        return Err(Failure::AuthorityRejected);
    }
    Ok(())
}

fn verify_and_bind(
    tag: &str,
    profile: &str,
    source: &dyn PublicSource,
    trust: &dyn AttestationTrust,
    force_cleanup_failure: bool,
) -> Result<VerifiedArtifact, Failure> {
    let (version, prerelease) = parse_tag(tag)?;
    let profile = Profile::parse(profile).ok_or(Failure::InvalidRequest)?;
    let selected_name = profile.selected_name(&version);
    let root = tempfile::Builder::new()
        .prefix("batcave-release-verifier-")
        .tempdir()
        .map_err(|_| Failure::AuthorityRejected)?;

    let operation = (|| {
        let before = source.release(tag)?;
        validate_release(&before, tag, prerelease, &version)?;
        let commit = source.tag_commit(tag)?;
        if commit != before.target_commitish {
            return Err(Failure::ReleaseRejected);
        }

        download_inventory(source, &before, root.path())?;
        let subjects = verify_checksums(&before, root.path(), tag)?;
        let provenance_name = format!("BatCave-{tag}-provenance.json");
        let provenance = fs::read(root.path().join(&provenance_name))
            .map_err(|_| Failure::AttestationRejected)?;
        trust.verify_build(&provenance, &subjects, &before.target_commitish)?;

        let selected = before
            .assets
            .iter()
            .find(|asset| asset.name == selected_name)
            .cloned()
            .ok_or(Failure::InventoryRejected)?;
        let selected_bytes =
            fs::read(root.path().join(&selected.name)).map_err(|_| Failure::AuthorityRejected)?;
        if digest_hex(&selected_bytes) != digest_value(&selected.digest)? {
            return Err(Failure::AuthorityRejected);
        }
        let updater_signature = if profile == Profile::MacOsUpdater {
            let signature_name = format!("{}.sig", selected.name);
            let asset = before
                .assets
                .iter()
                .find(|asset| asset.name == signature_name)
                .cloned()
                .ok_or(Failure::InventoryRejected)?;
            let bytes =
                fs::read(root.path().join(&asset.name)).map_err(|_| Failure::AuthorityRejected)?;
            if bytes.is_empty()
                || bytes.len() as u64 != asset.size
                || digest_hex(&bytes) != digest_value(&asset.digest)?
            {
                return Err(Failure::AuthorityRejected);
            }
            Some(BoundUpdaterSignature { asset, bytes })
        } else {
            None
        };

        let release_bundles = source.release_attestations(&commit)?;
        trust.verify_release(&release_bundles, tag, &commit, &before, &selected.digest)?;

        let after = source.release(tag)?;
        if canonical_release(&before) != canonical_release(&after) {
            return Err(Failure::ReadbackDrift);
        }

        Ok(VerifiedArtifact {
            profile,
            tag: tag.to_string(),
            version,
            source_sha: commit,
            release: before,
            selected,
            updater_signature,
            artifact: BoundArtifact::new(selected_bytes),
        })
    })();

    let cleanup = if force_cleanup_failure {
        Err(Failure::CleanupFailed)
    } else {
        root.close().map_err(|_| Failure::CleanupFailed)
    };
    match (operation, cleanup) {
        (_, Err(failure)) => Err(failure),
        (Err(failure), Ok(())) => Err(failure),
        (Ok(verified), Ok(())) => Ok(verified),
    }
}

fn parse_tag(tag: &str) -> Result<(String, bool), Failure> {
    let version = tag.strip_prefix('v').ok_or(Failure::InvalidRequest)?;
    if version.is_empty()
        || version.len() > 80
        || version.ends_with('-')
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(Failure::InvalidRequest);
    }
    let core = version.split('-').next().ok_or(Failure::InvalidRequest)?;
    if core.split('.').count() != 3
        || core
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(Failure::InvalidRequest);
    }
    Ok((version.to_string(), version.contains('-')))
}

fn expected_names(tag: &str, version: &str) -> BTreeSet<String> {
    [
        "batcave-monitor.exe".to_string(),
        "batcave-monitor-cli.exe".to_string(),
        format!("BatCave.Monitor_{version}_x64-setup.exe"),
        format!("BatCave.Monitor_{version}_x64-setup.exe.sig"),
        format!("BatCave.Monitor_{version}_amd64.deb"),
        format!("BatCave.Monitor_{version}_amd64.AppImage"),
        format!("BatCave.Monitor_{version}_amd64.AppImage.sig"),
        format!("BatCave.Monitor_{version}_universal.dmg"),
        "BatCave.Monitor.app.tar.gz".to_string(),
        "BatCave.Monitor.app.tar.gz.sig".to_string(),
        "latest.json".to_string(),
        CHECKSUMS.to_string(),
        format!("BatCave-{tag}-provenance.json"),
    ]
    .into_iter()
    .collect()
}

fn validate_release(
    release: &ReleaseReadback,
    tag: &str,
    prerelease: bool,
    version: &str,
) -> Result<(), Failure> {
    if release.tag_name != tag
        || !is_sha(&release.target_commitish)
        || release.draft
        || !release.immutable
        || release.prerelease != prerelease
    {
        return Err(Failure::ReleaseRejected);
    }
    let expected = expected_names(tag, version);
    let mut actual = BTreeSet::new();
    let mut canonical = BTreeSet::new();
    let mut total = 0u64;
    for asset in &release.assets {
        require_safe_name(&asset.name)?;
        if !canonical.insert(asset.name.to_ascii_lowercase()) || !actual.insert(asset.name.clone())
        {
            return Err(Failure::InventoryRejected);
        }
        total = total
            .checked_add(asset.size)
            .ok_or(Failure::InventoryRejected)?;
        digest_value(&asset.digest)?;
        let expected_url = format!(
            "https://github.com/{REPOSITORY}/releases/download/{tag}/{}",
            asset.name
        );
        if asset.browser_download_url != expected_url {
            return Err(Failure::InventoryRejected);
        }
    }
    if actual != expected || total > MAX_RELEASE_BYTES {
        return Err(Failure::InventoryRejected);
    }
    Ok(())
}

fn canonical_release(release: &ReleaseReadback) -> ReleaseReadback {
    let mut release = release.clone();
    release
        .assets
        .sort_by(|left, right| left.name.cmp(&right.name));
    release
}

fn download_inventory(
    source: &dyn PublicSource,
    release: &ReleaseReadback,
    root: &Path,
) -> Result<(), Failure> {
    for asset in &release.assets {
        let path = root.join(&asset.name);
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| Failure::DownloadRejected)?;
        let written = source.download(&asset.browser_download_url, asset.size, &mut output)?;
        output.sync_all().map_err(|_| Failure::DownloadRejected)?;
        if written != asset.size {
            return Err(Failure::DownloadRejected);
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::DownloadRejected)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != asset.size
        {
            return Err(Failure::DownloadRejected);
        }
        if sha256_file(&path)? != digest_value(&asset.digest)? {
            return Err(Failure::DownloadRejected);
        }
    }
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|_| Failure::DownloadRejected)? {
        let entry = entry.map_err(|_| Failure::DownloadRejected)?;
        if !entry
            .file_type()
            .map_err(|_| Failure::DownloadRejected)?
            .is_file()
        {
            return Err(Failure::DownloadRejected);
        }
        actual.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| Failure::DownloadRejected)?,
        );
    }
    let expected = release
        .assets
        .iter()
        .map(|asset| asset.name.clone())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(Failure::DownloadRejected);
    }
    Ok(())
}

fn verify_checksums(
    release: &ReleaseReadback,
    root: &Path,
    tag: &str,
) -> Result<BTreeMap<String, String>, Failure> {
    let bytes = fs::read(root.join(CHECKSUMS)).map_err(|_| Failure::ChecksumRejected)?;
    let contents = std::str::from_utf8(&bytes).map_err(|_| Failure::ChecksumRejected)?;
    if contents.is_empty() || !contents.ends_with('\n') || contents.contains('\r') {
        return Err(Failure::ChecksumRejected);
    }
    let mut subjects = BTreeMap::new();
    for line in contents[..contents.len() - 1].split('\n') {
        let (digest, name) = line.split_once("  ./").ok_or(Failure::ChecksumRejected)?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(Failure::ChecksumRejected);
        }
        require_safe_name(name)?;
        if subjects
            .insert(name.to_string(), digest.to_string())
            .is_some()
        {
            return Err(Failure::ChecksumRejected);
        }
    }
    let provenance = format!("BatCave-{tag}-provenance.json");
    let expected = release
        .assets
        .iter()
        .filter(|asset| asset.name != CHECKSUMS && asset.name != provenance)
        .map(|asset| Ok((asset.name.clone(), digest_value(&asset.digest)?)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if subjects != expected {
        return Err(Failure::ChecksumRejected);
    }
    Ok(subjects)
}

fn require_safe_name(name: &str) -> Result<(), Failure> {
    if name.is_empty()
        || !name.is_ascii()
        || name == "."
        || name == ".."
        || name.bytes().any(|byte| byte <= 0x1f || byte == 0x7f)
        || name.contains(['/', '\\'])
    {
        return Err(Failure::InventoryRejected);
    }
    Ok(())
}

fn is_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest_value(value: &str) -> Result<String, Failure> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or(Failure::InventoryRejected)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Failure::InventoryRejected);
    }
    Ok(digest.to_string())
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String, Failure> {
    let mut file = File::open(path).map_err(|_| Failure::DownloadRejected)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| Failure::DownloadRejected)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

struct Seal;

struct BoundArtifact {
    bytes: Option<Arc<[u8]>>,
    seal: Arc<Seal>,
}

impl BoundArtifact {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Some(Arc::from(bytes)),
            seal: Arc::new(Seal),
        }
    }

    fn take(&mut self, seal: &Arc<Seal>) -> Result<Arc<[u8]>, Failure> {
        if !Arc::ptr_eq(&self.seal, seal) {
            return Err(Failure::AuthorityRejected);
        }
        self.bytes.take().ok_or(Failure::AuthorityRejected)
    }

    fn finish_without_native(mut self) -> Result<SanitizedOutcome, Failure> {
        let seal = Arc::clone(&self.seal);
        let retained = self.take(&seal)?;
        if retained.is_empty() {
            return Err(Failure::AuthorityRejected);
        }
        Ok(SanitizedOutcome::skipped())
    }
}

struct GitHubSource {
    client: Client,
    deadline: OperationDeadline,
}

#[derive(Clone, Copy)]
struct OperationDeadline {
    expires_at: Instant,
}

impl OperationDeadline {
    fn starting_at(now: Instant, timeout: Duration) -> Result<Self, Failure> {
        let expires_at = now.checked_add(timeout).ok_or(Failure::Timeout)?;
        Ok(Self { expires_at })
    }

    fn remaining_at(self, now: Instant) -> Result<Duration, Failure> {
        self.expires_at
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .ok_or(Failure::Timeout)
    }

    fn remaining(self) -> Result<Duration, Failure> {
        self.remaining_at(Instant::now())
    }
}

impl GitHubSource {
    fn new() -> Result<Self, Failure> {
        let deadline = OperationDeadline::starting_at(Instant::now(), OPERATION_TIMEOUT)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Failure::Offline)?;
        Ok(Self { client, deadline })
    }

    fn get(&self, url: &str, api: bool) -> Result<Response, Failure> {
        let mut current = Url::parse(url).map_err(|_| Failure::Redirect)?;
        for _ in 0..=MAX_REDIRECTS {
            validate_url(&current, api)?;
            let remaining = self.deadline.remaining()?;
            let mut request = self
                .client
                .get(current.clone())
                .timeout(remaining)
                .header("User-Agent", "batcave-rust-release-verifier/1")
                .header("Accept", "application/vnd.github+json");
            if api {
                request = request.header("X-GitHub-Api-Version", "2022-11-28");
            }
            let response = request.send().map_err(map_reqwest)?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(Failure::Redirect)?;
                current = current.join(location).map_err(|_| Failure::Redirect)?;
                continue;
            }
            if response.status() != StatusCode::OK {
                return Err(Failure::Offline);
            }
            self.deadline.remaining()?;
            return Ok(response);
        }
        Err(Failure::Redirect)
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, Failure> {
        self.json_response(url, Failure::ReleaseRejected)
            .map(|(value, _)| value)
    }

    fn json_response<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        failure: Failure,
    ) -> Result<(T, HeaderMap), Failure> {
        let response = self.get(url, true)?;
        let headers = response.headers().clone();
        let bytes = self.read_response(response, MAX_API_JSON_BYTES, failure.clone())?;
        let value = parse_api_json(&bytes, failure)?;
        Ok((value, headers))
    }

    fn read_response(
        &self,
        mut response: Response,
        limit: u64,
        failure: Failure,
    ) -> Result<Vec<u8>, Failure> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            self.deadline.remaining()?;
            let read = response
                .read(&mut buffer)
                .map_err(|_| self.deadline.remaining().err().unwrap_or(failure.clone()))?;
            if read == 0 {
                break;
            }
            if bytes.len() as u64 + read as u64 > limit {
                return Err(failure);
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        self.deadline.remaining()?;
        Ok(bytes)
    }
}

impl PublicSource for GitHubSource {
    fn release(&self, tag: &str) -> Result<ReleaseReadback, Failure> {
        self.json(&format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/tags/{tag}"
        ))
    }

    fn tag_commit(&self, tag: &str) -> Result<String, Failure> {
        #[derive(Deserialize)]
        struct RefObject {
            object: GitObject,
        }
        #[derive(Deserialize)]
        struct GitObject {
            #[serde(rename = "type")]
            kind: String,
            sha: String,
        }
        let mut object = self
            .json::<RefObject>(&format!(
                "https://api.github.com/repos/{REPOSITORY}/git/ref/tags/{tag}"
            ))?
            .object;
        for _ in 0..2 {
            if object.kind == "commit" && is_sha(&object.sha) {
                return Ok(object.sha);
            }
            if object.kind != "tag" || !is_sha(&object.sha) {
                return Err(Failure::ReleaseRejected);
            }
            object = self
                .json::<RefObject>(&format!(
                    "https://api.github.com/repos/{REPOSITORY}/git/tags/{}",
                    object.sha
                ))?
                .object;
        }
        Err(Failure::ReleaseRejected)
    }

    fn release_attestations(&self, commit: &str) -> Result<Vec<Vec<u8>>, Failure> {
        #[derive(Deserialize)]
        struct Attestations {
            attestations: Vec<Attestation>,
        }
        #[derive(Deserialize)]
        struct Attestation {
            bundle: Option<serde_json::Value>,
            bundle_url: Option<String>,
        }
        let (response, headers): (Attestations, _) =
            self.json_response(&attestation_api_url(commit), Failure::AttestationRejected)?;
        if link_has_next_page(&headers)? {
            return Err(Failure::AttestationRejected);
        }
        validate_attestation_count(response.attestations.len())?;
        let mut bundles = Vec::new();
        for attestation in response.attestations {
            if let (Some(bundle), None) = (attestation.bundle.as_ref(), &attestation.bundle_url) {
                bundles.push(serialize_inline_bundle(bundle)?);
            } else if let (None, Some(url)) = (attestation.bundle, attestation.bundle_url) {
                let response = self.get(&url, true)?;
                let compressed = self.read_response(
                    response,
                    MAX_ATTESTATION_BYTES,
                    Failure::AttestationRejected,
                )?;
                bundles.push(decode_release_bundle_blob(&compressed)?);
            } else {
                return Err(Failure::AttestationRejected);
            }
        }
        validate_attestation_count(bundles.len())?;
        Ok(bundles)
    }

    fn download(&self, url: &str, limit: u64, output: &mut File) -> Result<u64, Failure> {
        let mut response = self.get(url, false)?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            self.deadline.remaining()?;
            let read = response.read(&mut buffer).map_err(|_| {
                self.deadline
                    .remaining()
                    .err()
                    .unwrap_or(Failure::DownloadRejected)
            })?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .ok_or(Failure::DownloadRejected)?;
            if total > limit {
                return Err(Failure::DownloadRejected);
            }
            output
                .write_all(&buffer[..read])
                .map_err(|_| Failure::DownloadRejected)?;
        }
        self.deadline.remaining()?;
        Ok(total)
    }
}

fn decode_release_bundle_blob(compressed: &[u8]) -> Result<Vec<u8>, Failure> {
    if compressed.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(Failure::AttestationRejected);
    }

    let decoded_len =
        snap::raw::decompress_len(compressed).map_err(|_| Failure::AttestationRejected)?;
    if decoded_len > MAX_ATTESTATION_BYTES as usize {
        return Err(Failure::AttestationRejected);
    }
    let decoded = snap::raw::Decoder::new()
        .decompress_vec(compressed)
        .map_err(|_| Failure::AttestationRejected)?;
    if decoded.len() != decoded_len || decoded.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(Failure::AttestationRejected);
    }
    Ok(decoded)
}

fn attestation_api_url(commit: &str) -> String {
    format!(
        "https://api.github.com/repos/{REPOSITORY}/attestations/sha1:{commit}?predicate_type=release&per_page={MAX_ATTESTATION_ENTRIES}"
    )
}

fn validate_attestation_count(count: usize) -> Result<(), Failure> {
    (count <= MAX_ATTESTATION_ENTRIES)
        .then_some(())
        .ok_or(Failure::AttestationRejected)
}

fn parse_api_json<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    failure: Failure,
) -> Result<T, Failure> {
    if bytes.len() as u64 > MAX_API_JSON_BYTES {
        return Err(failure);
    }
    serde_json::from_slice(bytes).map_err(|_| failure)
}

fn serialize_inline_bundle(bundle: &serde_json::Value) -> Result<Vec<u8>, Failure> {
    let bytes = serde_json::to_vec(bundle).map_err(|_| Failure::AttestationRejected)?;
    if bytes.len() as u64 > MAX_ATTESTATION_BYTES {
        return Err(Failure::AttestationRejected);
    }
    Ok(bytes)
}

fn link_has_next_page(headers: &HeaderMap) -> Result<bool, Failure> {
    for raw in headers.get_all(reqwest::header::LINK) {
        let value = raw.to_str().map_err(|_| Failure::AttestationRejected)?;
        for entry in value.split(',') {
            let mut parts = entry.split(';');
            let target = parts
                .next()
                .map(str::trim)
                .filter(|target| target.starts_with('<') && target.ends_with('>'))
                .ok_or(Failure::AttestationRejected)?;
            if target.len() <= 2 {
                return Err(Failure::AttestationRejected);
            }
            for parameter in parts {
                let (name, value) = parameter
                    .split_once('=')
                    .ok_or(Failure::AttestationRejected)?;
                if name.trim() == "rel" {
                    let relations = value
                        .trim()
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                        .ok_or(Failure::AttestationRejected)?;
                    if relations.split_ascii_whitespace().any(|rel| rel == "next") {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

fn map_reqwest(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        Failure::Timeout
    } else {
        Failure::Offline
    }
}

fn validate_url(url: &Url, api: bool) -> Result<(), Failure> {
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Failure::Redirect);
    }
    let host = url.host_str().ok_or(Failure::Redirect)?;
    let allowed = if api {
        matches!(
            host,
            "api.github.com"
                | "github.com"
                | "objects.githubusercontent.com"
                | "tmaproduction.blob.core.windows.net"
        )
    } else {
        matches!(
            host,
            "github.com"
                | "release-assets.githubusercontent.com"
                | "objects.githubusercontent.com"
                | "github-releases.githubusercontent.com"
        )
    };
    allowed.then_some(()).ok_or(Failure::Redirect)
}

struct SigstoreTrust;

impl AttestationTrust for SigstoreTrust {
    fn verify_build(
        &self,
        bundle_json: &[u8],
        subjects: &BTreeMap<String, String>,
        source_sha: &str,
    ) -> Result<(), Failure> {
        let bundle = parse_bundle(bundle_json)?;
        let statement = statement(&bundle)?;
        validate_build_statement(&statement, subjects, source_sha)?;
        let digest = subjects
            .values()
            .next()
            .ok_or(Failure::AttestationRejected)?;
        let root = TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT)
            .map_err(|_| Failure::AttestationRejected)?;
        let policy = VerificationPolicy::default()
            .require_identity(WORKFLOW_IDENTITY)
            .require_issuer(ACTIONS_ISSUER);
        verify(
            Sha256Hash::from_hex(digest).map_err(|_| Failure::AttestationRejected)?,
            &bundle,
            &policy,
            &root,
        )
        .map_err(|_| Failure::AttestationRejected)?;
        Ok(())
    }

    fn verify_release(
        &self,
        bundles: &[Vec<u8>],
        tag: &str,
        commit: &str,
        release: &ReleaseReadback,
        selected_digest: &str,
    ) -> Result<(), Failure> {
        let mut matches = Vec::new();
        for bytes in bundles {
            let bundle = parse_bundle(bytes)?;
            let statement = statement(&bundle)?;
            if statement
                .pointer("/predicate/tag")
                .and_then(serde_json::Value::as_str)
                == Some(tag)
            {
                matches.push((bundle, statement));
            }
        }
        if matches.len() != 1 {
            return Err(Failure::AttestationRejected);
        }
        let (bundle, statement) = matches.pop().expect("one release attestation");
        validate_release_statement(&statement, tag, commit, release)?;
        let root = TrustedRoot::from_embedded(SigstoreInstance::GitHub)
            .map_err(|_| Failure::AttestationRejected)?;
        let policy = VerificationPolicy::default()
            .require_identity(RELEASE_IDENTITY)
            .skip_tlog()
            .skip_sct();
        verify(
            Sha256Hash::from_hex(&digest_value(selected_digest)?)
                .map_err(|_| Failure::AttestationRejected)?,
            &bundle,
            &policy,
            &root,
        )
        .map_err(|_| Failure::AttestationRejected)?;
        Ok(())
    }
}

fn parse_bundle(bytes: &[u8]) -> Result<Bundle, Failure> {
    let json = std::str::from_utf8(bytes).map_err(|_| Failure::AttestationRejected)?;
    Bundle::from_json(json).map_err(|_| Failure::AttestationRejected)
}

fn statement(bundle: &Bundle) -> Result<serde_json::Value, Failure> {
    let SignatureContent::DsseEnvelope(envelope) = &bundle.content else {
        return Err(Failure::AttestationRejected);
    };
    if envelope.payload_type != "application/vnd.in-toto+json" {
        return Err(Failure::AttestationRejected);
    }
    serde_json::from_slice(envelope.payload.as_bytes()).map_err(|_| Failure::AttestationRejected)
}

fn validate_build_statement(
    statement: &serde_json::Value,
    expected_subjects: &BTreeMap<String, String>,
    source_sha: &str,
) -> Result<(), Failure> {
    if statement.get("_type").and_then(serde_json::Value::as_str)
        != Some("https://in-toto.io/Statement/v1")
        || statement
            .get("predicateType")
            .and_then(serde_json::Value::as_str)
            != Some("https://slsa.dev/provenance/v1")
        || statement
            .pointer("/predicate/buildDefinition/buildType")
            .and_then(serde_json::Value::as_str)
            != Some("https://actions.github.io/buildtypes/workflow/v1")
        || statement
            .pointer("/predicate/buildDefinition/externalParameters/workflow/repository")
            .and_then(serde_json::Value::as_str)
            != Some("https://github.com/TheGreenCedar/BatCave")
        || statement
            .pointer("/predicate/buildDefinition/externalParameters/workflow/ref")
            .and_then(serde_json::Value::as_str)
            != Some(SOURCE_REF)
        || statement
            .pointer("/predicate/buildDefinition/externalParameters/workflow/path")
            .and_then(serde_json::Value::as_str)
            != Some(WORKFLOW_PATH)
        || statement
            .pointer("/predicate/buildDefinition/internalParameters/github/runner_environment")
            .and_then(serde_json::Value::as_str)
            != Some("github-hosted")
        || statement
            .pointer("/predicate/buildDefinition/internalParameters/github/repository_id")
            .and_then(serde_json::Value::as_str)
            != Some(REPOSITORY_ID)
        || statement
            .pointer("/predicate/buildDefinition/internalParameters/github/repository_owner_id")
            .and_then(serde_json::Value::as_str)
            != Some(OWNER_ID)
        || statement
            .pointer("/predicate/runDetails/builder/id")
            .and_then(serde_json::Value::as_str)
            != Some(WORKFLOW_IDENTITY)
    {
        return Err(Failure::AttestationRejected);
    }
    let dependencies = statement
        .pointer("/predicate/buildDefinition/resolvedDependencies")
        .and_then(serde_json::Value::as_array)
        .ok_or(Failure::AttestationRejected)?;
    if dependencies.len() != 1
        || dependencies[0]
            .get("uri")
            .and_then(serde_json::Value::as_str)
            != Some("git+https://github.com/TheGreenCedar/BatCave@refs/heads/main")
        || dependencies[0]
            .pointer("/digest/gitCommit")
            .and_then(serde_json::Value::as_str)
            != Some(source_sha)
    {
        return Err(Failure::AttestationRejected);
    }
    let actual = exact_build_subjects(statement)?;
    if actual != *expected_subjects {
        return Err(Failure::AttestationRejected);
    }
    Ok(())
}

fn validate_release_statement(
    statement: &serde_json::Value,
    tag: &str,
    commit: &str,
    release: &ReleaseReadback,
) -> Result<(), Failure> {
    if statement.get("_type").and_then(serde_json::Value::as_str)
        != Some("https://in-toto.io/Statement/v1")
        || statement
            .get("predicateType")
            .and_then(serde_json::Value::as_str)
            != Some("https://in-toto.io/attestation/release/v0.1")
        || statement
            .pointer("/predicate/repository")
            .and_then(serde_json::Value::as_str)
            != Some(REPOSITORY)
        || statement
            .pointer("/predicate/repositoryId")
            .and_then(serde_json::Value::as_str)
            != Some(REPOSITORY_ID)
        || statement
            .pointer("/predicate/ownerId")
            .and_then(serde_json::Value::as_str)
            != Some(OWNER_ID)
        || statement
            .pointer("/predicate/tag")
            .and_then(serde_json::Value::as_str)
            != Some(tag)
        || statement
            .pointer("/predicate/releaseId")
            .and_then(serde_json::Value::as_str)
            != Some(&release.id.to_string())
    {
        return Err(Failure::AttestationRejected);
    }
    let expected = release
        .assets
        .iter()
        .map(|asset| Ok((asset.name.clone(), digest_value(&asset.digest)?)))
        .collect::<Result<BTreeMap<_, _>, Failure>>()?;
    let assets = exact_release_subjects(statement, tag, commit)?;
    if assets != expected {
        return Err(Failure::AttestationRejected);
    }
    Ok(())
}

fn exact_build_subjects(
    statement: &serde_json::Value,
) -> Result<BTreeMap<String, String>, Failure> {
    let subjects = statement
        .get("subject")
        .and_then(serde_json::Value::as_array)
        .ok_or(Failure::AttestationRejected)?;
    let mut result = BTreeMap::new();
    for subject in subjects {
        if !has_exact_keys(subject, &["digest", "name"]) {
            return Err(Failure::AttestationRejected);
        }
        let raw_name = subject
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(Failure::AttestationRejected)?;
        let name = raw_name
            .strip_prefix("./")
            .ok_or(Failure::AttestationRejected)?;
        require_safe_name(name).map_err(|_| Failure::AttestationRejected)?;
        let digest = exact_digest(subject, "sha256", 64)?;
        if result.insert(name.to_string(), digest).is_some() {
            return Err(Failure::AttestationRejected);
        }
    }
    Ok(result)
}

fn exact_release_subjects(
    statement: &serde_json::Value,
    tag: &str,
    commit: &str,
) -> Result<BTreeMap<String, String>, Failure> {
    let subjects = statement
        .get("subject")
        .and_then(serde_json::Value::as_array)
        .ok_or(Failure::AttestationRejected)?;
    let expected_uri = format!("pkg:github/{REPOSITORY}@{tag}");
    let mut source_count = 0usize;
    let mut assets = BTreeMap::new();
    for subject in subjects {
        if has_exact_keys(subject, &["digest", "uri"]) {
            if subject.get("uri").and_then(serde_json::Value::as_str) != Some(&expected_uri)
                || exact_digest(subject, "sha1", 40)? != commit
            {
                return Err(Failure::AttestationRejected);
            }
            source_count += 1;
            continue;
        }
        if !has_exact_keys(subject, &["digest", "name"]) {
            return Err(Failure::AttestationRejected);
        }
        let name = subject
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(Failure::AttestationRejected)?;
        require_safe_name(name).map_err(|_| Failure::AttestationRejected)?;
        let digest = exact_digest(subject, "sha256", 64)?;
        if assets.insert(name.to_string(), digest).is_some() {
            return Err(Failure::AttestationRejected);
        }
    }
    if source_count != 1 {
        return Err(Failure::AttestationRejected);
    }
    Ok(assets)
}

fn exact_digest(
    subject: &serde_json::Value,
    algorithm: &str,
    length: usize,
) -> Result<String, Failure> {
    let digest = subject
        .get("digest")
        .and_then(serde_json::Value::as_object)
        .ok_or(Failure::AttestationRejected)?;
    if digest.len() != 1 {
        return Err(Failure::AttestationRejected);
    }
    let value = digest
        .get(algorithm)
        .and_then(serde_json::Value::as_str)
        .ok_or(Failure::AttestationRejected)?;
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Failure::AttestationRejected);
    }
    Ok(value.to_string())
}

fn has_exact_keys(value: &serde_json::Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const TAG: &str = "v0.3.0";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    struct FixtureSource {
        releases: RefCell<Vec<ReleaseReadback>>,
        release_calls: Cell<usize>,
        reread_failure: Option<Failure>,
        payloads: BTreeMap<String, Vec<u8>>,
        failure: Option<Failure>,
    }

    impl PublicSource for FixtureSource {
        fn release(&self, _tag: &str) -> Result<ReleaseReadback, Failure> {
            let call = self.release_calls.get();
            self.release_calls.set(call + 1);
            if call > 0 {
                if let Some(failure) = &self.reread_failure {
                    return Err(failure.clone());
                }
            }
            let mut releases = self.releases.borrow_mut();
            if releases.len() > 1 {
                Ok(releases.remove(0))
            } else {
                Ok(releases[0].clone())
            }
        }

        fn tag_commit(&self, _tag: &str) -> Result<String, Failure> {
            Ok(SHA.to_string())
        }

        fn release_attestations(&self, _commit: &str) -> Result<Vec<Vec<u8>>, Failure> {
            Ok(vec![b"release-attestation".to_vec()])
        }

        fn download(&self, url: &str, _limit: u64, output: &mut File) -> Result<u64, Failure> {
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }
            let name = url.rsplit('/').next().ok_or(Failure::DownloadRejected)?;
            let bytes = self.payloads.get(name).ok_or(Failure::DownloadRejected)?;
            output
                .write_all(bytes)
                .map_err(|_| Failure::DownloadRejected)?;
            Ok(bytes.len() as u64)
        }
    }

    struct FixtureTrust {
        build_ok: bool,
        release_ok: bool,
        calls: Cell<usize>,
    }

    impl AttestationTrust for FixtureTrust {
        fn verify_build(
            &self,
            _bundle: &[u8],
            _subjects: &BTreeMap<String, String>,
            _source_sha: &str,
        ) -> Result<(), Failure> {
            self.calls.set(self.calls.get() + 1);
            self.build_ok
                .then_some(())
                .ok_or(Failure::AttestationRejected)
        }

        fn verify_release(
            &self,
            _bundles: &[Vec<u8>],
            _tag: &str,
            _commit: &str,
            _release: &ReleaseReadback,
            _selected_digest: &str,
        ) -> Result<(), Failure> {
            self.calls.set(self.calls.get() + 1);
            self.release_ok
                .then_some(())
                .ok_or(Failure::AttestationRejected)
        }
    }

    fn fixture() -> (FixtureSource, FixtureTrust) {
        let names = expected_names(TAG, "0.3.0");
        let provenance = format!("BatCave-{TAG}-provenance.json");
        let mut payloads = BTreeMap::new();
        for name in names
            .iter()
            .filter(|name| *name != CHECKSUMS && **name != provenance)
        {
            payloads.insert(
                name.clone(),
                format!("fixture bytes for {name}\n").into_bytes(),
            );
        }
        let checksums = payloads
            .iter()
            .map(|(name, bytes)| format!("{}  ./{name}\n", digest_hex(bytes)))
            .collect::<String>();
        payloads.insert(CHECKSUMS.to_string(), checksums.into_bytes());
        payloads.insert(provenance, b"fixture provenance".to_vec());
        let assets = payloads
            .iter()
            .enumerate()
            .map(|(index, (name, bytes))| ReleaseAsset {
                id: index as u64 + 1,
                name: name.clone(),
                size: bytes.len() as u64,
                digest: format!("sha256:{}", digest_hex(bytes)),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/{TAG}/{name}"
                ),
            })
            .collect();
        let release = ReleaseReadback {
            id: 42,
            tag_name: TAG.to_string(),
            target_commitish: SHA.to_string(),
            draft: false,
            prerelease: false,
            immutable: true,
            assets,
        };
        (
            FixtureSource {
                releases: RefCell::new(vec![release]),
                release_calls: Cell::new(0),
                reread_failure: None,
                payloads,
                failure: None,
            },
            FixtureTrust {
                build_ok: true,
                release_ok: true,
                calls: Cell::new(0),
            },
        )
    }

    fn build_statement(subjects: &BTreeMap<String, String>) -> serde_json::Value {
        serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": subjects.iter().map(|(name, digest)| serde_json::json!({
                "name": format!("./{name}"),
                "digest": { "sha256": digest },
            })).collect::<Vec<_>>(),
            "predicateType": "https://slsa.dev/provenance/v1",
            "predicate": {
                "buildDefinition": {
                    "buildType": "https://actions.github.io/buildtypes/workflow/v1",
                    "externalParameters": { "workflow": {
                        "repository": "https://github.com/TheGreenCedar/BatCave",
                        "ref": SOURCE_REF,
                        "path": WORKFLOW_PATH,
                    }},
                    "internalParameters": { "github": {
                        "runner_environment": "github-hosted",
                        "repository_id": REPOSITORY_ID,
                        "repository_owner_id": OWNER_ID,
                    }},
                    "resolvedDependencies": [{
                        "uri": "git+https://github.com/TheGreenCedar/BatCave@refs/heads/main",
                        "digest": { "gitCommit": SHA },
                    }],
                },
                "runDetails": { "builder": { "id": WORKFLOW_IDENTITY } },
            },
        })
    }

    fn release_statement(release: &ReleaseReadback) -> serde_json::Value {
        let mut subjects = vec![serde_json::json!({
            "uri": format!("pkg:github/{REPOSITORY}@{TAG}"),
            "digest": { "sha1": SHA },
        })];
        subjects.extend(release.assets.iter().map(|asset| {
            serde_json::json!({
                "name": asset.name,
                "digest": { "sha256": digest_value(&asset.digest).unwrap() },
            })
        }));
        serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": subjects,
            "predicateType": "https://in-toto.io/attestation/release/v0.1",
            "predicate": {
                "ownerId": OWNER_ID,
                "releaseId": release.id.to_string(),
                "repository": REPOSITORY,
                "repositoryId": REPOSITORY_ID,
                "tag": TAG,
            },
        })
    }

    #[test]
    fn complete_verification_retains_selected_bytes_then_stops_without_native_proof() {
        let (source, trust) = fixture();
        assert_eq!(
            execute(TAG, "linux-deb", &source, &trust, false),
            Ok(SanitizedOutcome::skipped())
        );
        assert_eq!(trust.calls.get(), 2);
    }

    #[test]
    fn selectors_cannot_supply_proof_fields() {
        let (source, trust) = fixture();
        for selector in [
            "../asset",
            "sha256:00",
            "https://example.com",
            "linux-deb --digest",
        ] {
            assert_eq!(
                execute(TAG, selector, &source, &trust, false),
                Err(Failure::InvalidRequest)
            );
        }
        assert_eq!(
            execute("--help", "linux-deb", &source, &trust, false),
            Err(Failure::InvalidRequest)
        );
    }

    #[test]
    fn release_readback_drift_fails_closed() {
        let (source, trust) = fixture();
        let before = source.releases.borrow()[0].clone();
        let mut after = before.clone();
        after.assets[0].id += 1;
        *source.releases.borrow_mut() = vec![before, after];
        assert_eq!(
            execute(TAG, "linux-appimage", &source, &trust, false),
            Err(Failure::ReadbackDrift)
        );
    }

    #[test]
    fn release_reread_failure_fails_closed() {
        let (mut source, trust) = fixture();
        source.reread_failure = Some(Failure::Timeout);
        assert_eq!(
            execute(TAG, "linux-appimage", &source, &trust, false),
            Err(Failure::Timeout)
        );
        assert_eq!(
            public_failure(Failure::Timeout).0,
            SanitizedOutcome::failed("timeout")
        );
    }

    #[test]
    fn asset_checksum_and_attestation_drift_fail_closed() {
        let (mut source, trust) = fixture();
        let selected = "BatCave.Monitor_0.3.0_amd64.deb";
        source
            .payloads
            .insert(selected.to_string(), b"replacement".to_vec());
        assert_eq!(
            execute(TAG, "linux-deb", &source, &trust, false),
            Err(Failure::DownloadRejected)
        );

        let (source, mut trust) = fixture();
        trust.build_ok = false;
        assert_eq!(
            execute(TAG, "linux-deb", &source, &trust, false),
            Err(Failure::AttestationRejected)
        );
    }

    #[test]
    fn build_attestation_rejects_extra_or_multi_algorithm_subjects() {
        let expected = BTreeMap::from([
            ("asset-a".to_string(), "a".repeat(64)),
            ("asset-b".to_string(), "b".repeat(64)),
        ]);
        let valid = build_statement(&expected);
        assert!(validate_build_statement(&valid, &expected, SHA).is_ok());

        let mut extra = valid.clone();
        extra["subject"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "./ignored",
                "digest": { "sha512": "c".repeat(128) },
            }));
        assert_eq!(
            validate_build_statement(&extra, &expected, SHA),
            Err(Failure::AttestationRejected)
        );

        let mut multi = valid;
        multi["subject"][0]["digest"]["sha512"] = serde_json::json!("d".repeat(128));
        assert_eq!(
            validate_build_statement(&multi, &expected, SHA),
            Err(Failure::AttestationRejected)
        );
    }

    #[test]
    fn release_attestation_rejects_extra_or_multi_algorithm_subjects() {
        let (source, _) = fixture();
        let release = source.releases.borrow()[0].clone();
        let valid = release_statement(&release);
        assert!(validate_release_statement(&valid, TAG, SHA, &release).is_ok());

        let mut extra = valid.clone();
        extra["subject"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "ignored",
                "digest": { "sha512": "e".repeat(128) },
            }));
        assert_eq!(
            validate_release_statement(&extra, TAG, SHA, &release),
            Err(Failure::AttestationRejected)
        );

        let mut multi = valid;
        multi["subject"][1]["digest"]["sha512"] = serde_json::json!("f".repeat(128));
        assert_eq!(
            validate_release_statement(&multi, TAG, SHA, &release),
            Err(Failure::AttestationRejected)
        );
    }

    #[test]
    fn github_release_bundle_blob_uses_bounded_raw_snappy_without_content_type() {
        let bundle = serde_json::to_vec(&serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "verificationMaterial": {
                "certificate": { "rawBytes": "fixture-certificate" },
                "tlogEntries": [],
                "timestampVerificationData": { "rfc3161Timestamps": [] },
            },
            "dsseEnvelope": {
                "payload": "fixture-release-statement",
                "payloadType": "application/vnd.in-toto+json",
                "signatures": [{ "keyid": "", "sig": "fixture-signature" }],
            },
        }))
        .unwrap();
        let compressed = snap::raw::Encoder::new().compress_vec(&bundle).unwrap();
        assert_eq!(decode_release_bundle_blob(&compressed), Ok(bundle));

        assert_eq!(
            decode_release_bundle_blob(&vec![0_u8; MAX_ATTESTATION_BYTES as usize + 1]),
            Err(Failure::AttestationRejected)
        );
        let decoded_too_large = snap::raw::Encoder::new()
            .compress_vec(&vec![0_u8; MAX_ATTESTATION_BYTES as usize + 1])
            .unwrap();
        assert_eq!(
            decode_release_bundle_blob(&decoded_too_large),
            Err(Failure::AttestationRejected)
        );
        assert_eq!(
            public_failure(Failure::AttestationRejected).0,
            SanitizedOutcome::failed("public_release_verification_failed")
        );
    }

    #[test]
    fn attestation_query_and_pagination_are_closed_to_one_hundred_entries() {
        let url = Url::parse(&attestation_api_url(SHA)).unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query.get("predicate_type").map(|value| value.as_ref()),
            Some("release")
        );
        assert_eq!(
            query.get("per_page").map(|value| value.as_ref()),
            Some("100")
        );
        assert!(!query.contains_key("limit"));

        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "<https://api.github.com/example?page=2>; rel=\"next\", <https://api.github.com/example?page=2>; rel=\"last\"",
            ),
        );
        assert_eq!(link_has_next_page(&headers), Ok(true));

        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "<https://api.github.com/example?page=1>; rel=\"last\"",
            ),
        );
        assert_eq!(link_has_next_page(&headers), Ok(false));
        assert_eq!(validate_attestation_count(100), Ok(()));
        assert_eq!(
            validate_attestation_count(101),
            Err(Failure::AttestationRejected)
        );
    }

    #[test]
    fn api_json_inline_bundle_and_attestation_count_are_bounded() {
        assert_eq!(
            parse_api_json::<serde_json::Value>(
                &vec![b' '; MAX_API_JSON_BYTES as usize + 1],
                Failure::AttestationRejected,
            ),
            Err(Failure::AttestationRejected)
        );
        let oversized_inline = serde_json::json!({
            "bundle": "x".repeat(MAX_ATTESTATION_BYTES as usize + 1),
        });
        assert_eq!(
            serialize_inline_bundle(&oversized_inline),
            Err(Failure::AttestationRejected)
        );
        assert_eq!(
            validate_attestation_count(MAX_ATTESTATION_ENTRIES + 1),
            Err(Failure::AttestationRejected)
        );
    }

    #[test]
    fn operation_deadline_is_shared_and_timeout_remains_sanitized() {
        let start = Instant::now();
        let deadline = OperationDeadline::starting_at(start, Duration::from_secs(5)).unwrap();
        assert_eq!(
            deadline.remaining_at(start + Duration::from_secs(2)),
            Ok(Duration::from_secs(3))
        );
        assert_eq!(
            deadline.remaining_at(start + Duration::from_secs(5)),
            Err(Failure::Timeout)
        );
        assert_eq!(
            public_failure(Failure::Timeout).0,
            SanitizedOutcome::failed("timeout")
        );
    }

    #[test]
    fn redirect_timeout_and_cleanup_remain_distinct_sanitized_failures() {
        let (mut source, trust) = fixture();
        source.failure = Some(Failure::Timeout);
        assert_eq!(
            execute(TAG, "macos-updater", &source, &trust, false),
            Err(Failure::Timeout)
        );
        assert_eq!(
            public_failure(Failure::Timeout).0,
            SanitizedOutcome::failed("timeout")
        );
        assert_eq!(
            public_failure(Failure::Redirect).0,
            SanitizedOutcome::failed("public_release_verification_failed")
        );

        let (source, trust) = fixture();
        assert_eq!(
            execute(TAG, "macos-dmg", &source, &trust, true),
            Err(Failure::CleanupFailed)
        );
    }

    #[cfg(target_os = "linux")]
    fn linux_artifact(selector: &str) -> VerifiedLinuxArtifact {
        let (source, trust) = fixture();
        verify_and_bind(TAG, selector, &source, &trust, false)
            .unwrap()
            .into_linux()
            .unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_dispatch_is_closed_and_non_linux_profiles_cannot_convert() {
        for (selector, expected) in [
            ("linux-deb", LinuxProfile::Deb),
            ("linux-appimage", LinuxProfile::AppImage),
        ] {
            let artifact = linux_artifact(selector);
            assert_eq!(artifact.profile, expected);
            assert_eq!(
                artifact.identity.asset_name,
                expected.expected_name(&artifact.identity.version)
            );

            let (source, trust) = fixture();
            let verified = verify_and_bind(TAG, selector, &source, &trust, false).unwrap();
            assert_eq!(dispatch_verified(verified), Ok(SanitizedOutcome::skipped()));
        }

        let (source, trust) = fixture();
        let macos = verify_and_bind(TAG, "macos-dmg", &source, &trust, false).unwrap();
        assert!(matches!(
            macos.into_linux(),
            Err(Failure::AuthorityRejected)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_descriptor_is_exactly_sealed_read_only_and_offset_stable() {
        use std::io::{Seek, SeekFrom};

        let mut artifact = linux_artifact("linux-appimage");
        let flags = unsafe { libc::fcntl(artifact.descriptor.as_raw_fd(), libc::F_GETFL) };
        let seals = unsafe { libc::fcntl(artifact.descriptor.as_raw_fd(), libc::F_GET_SEALS) };
        assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
        assert_eq!(seals, artifact.required_seals);
        let metadata = artifact.descriptor.metadata().unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o777, LinuxProfile::AppImage.mode());

        artifact.descriptor.seek(SeekFrom::Start(3)).unwrap();
        let before = artifact.descriptor.stream_position().unwrap();
        artifact.revalidate().unwrap();
        assert_eq!(artifact.descriptor.stream_position().unwrap(), before);
        assert!(artifact.descriptor.write_at(b"x", 0).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_owner_rejects_identity_size_and_digest_drift() {
        let mut artifact = linux_artifact("linux-deb");
        assert_eq!(artifact.identity.tag, TAG);
        assert_eq!(artifact.identity.source_sha, SHA);
        assert_eq!(
            artifact.descriptor.metadata().unwrap().mode() & 0o777,
            LinuxProfile::Deb.mode()
        );

        artifact.identity.size += 1;
        assert_eq!(artifact.revalidate(), Err(Failure::AuthorityRejected));
        artifact.identity.size -= 1;
        artifact.identity.digest = format!("sha256:{}", "0".repeat(64));
        assert_eq!(artifact.revalidate(), Err(Failure::AuthorityRejected));
    }

    #[test]
    fn owned_artifact_rejects_replay_and_cross_operation_seals() {
        let mut first = BoundArtifact::new(b"first".to_vec());
        let second = BoundArtifact::new(b"second".to_vec());
        let first_seal = Arc::clone(&first.seal);
        assert!(first.take(&second.seal).is_err());
        assert_eq!(&*first.take(&first_seal).unwrap(), b"first");
        assert!(first.take(&first_seal).is_err());
    }

    #[test]
    fn urls_are_https_host_closed_and_credential_free() {
        assert!(validate_url(&Url::parse("https://github.com/a").unwrap(), false).is_ok());
        for url in [
            "http://github.com/a",
            "https://user@github.com/a",
            "https://example.com/a",
            "https://github.com:444/a",
        ] {
            assert_eq!(
                validate_url(&Url::parse(url).unwrap(), false),
                Err(Failure::Redirect)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn owned_root_rejects_preexisting_link_asset() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"outside").unwrap();
        let link = root.path().join("asset");
        symlink(&outside, &link).unwrap();
        assert!(OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(link)
            .is_err());
    }
}
