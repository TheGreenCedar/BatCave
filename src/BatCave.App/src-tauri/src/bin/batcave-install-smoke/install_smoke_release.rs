use reqwest::blocking::{Client, Response};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore_verify::trust_root::{
    SigstoreInstance, TrustedRoot, SIGSTORE_PRODUCTION_TRUSTED_ROOT,
};
use sigstore_verify::types::{Bundle, Sha256Hash, SignatureContent};
use sigstore_verify::{verify, VerificationPolicy};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

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
const MAX_ATTESTATION_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

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
}

impl SanitizedOutcome {
    fn failed(reason: &'static str) -> Self {
        Self {
            disposition: "failed",
            reason,
        }
    }

    fn skipped() -> Self {
        Self {
            disposition: "skipped",
            reason: "native_platform_not_implemented",
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
enum Failure {
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
    match execute(&selectors[0], &selectors[1], &source, &trust, false) {
        Ok(outcome) => (outcome, 0),
        Err(failure) => public_failure(failure),
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

fn execute(
    tag: &str,
    profile: &str,
    source: &dyn PublicSource,
    trust: &dyn AttestationTrust,
    force_cleanup_failure: bool,
) -> Result<SanitizedOutcome, Failure> {
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
            .ok_or(Failure::InventoryRejected)?;
        let selected_bytes =
            fs::read(root.path().join(&selected.name)).map_err(|_| Failure::AuthorityRejected)?;
        if digest_hex(&selected_bytes) != digest_value(&selected.digest)? {
            return Err(Failure::AuthorityRejected);
        }

        let release_bundles = source.release_attestations(&commit)?;
        trust.verify_release(&release_bundles, tag, &commit, &before, &selected.digest)?;

        let after = source.release(tag)?;
        if canonical_release(&before) != canonical_release(&after) {
            return Err(Failure::ReadbackDrift);
        }

        Ok(BoundArtifact::new(selected_bytes))
    })();

    let cleanup = if force_cleanup_failure {
        Err(Failure::CleanupFailed)
    } else {
        root.close().map_err(|_| Failure::CleanupFailed)
    };
    match (operation, cleanup) {
        (_, Err(failure)) => Err(failure),
        (Err(failure), Ok(())) => Err(failure),
        (Ok(artifact), Ok(())) => artifact.finish_without_native(),
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
}

impl GitHubSource {
    fn new() -> Result<Self, Failure> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Failure::Offline)?;
        Ok(Self { client })
    }

    fn get(&self, url: &str, api: bool) -> Result<Response, Failure> {
        let mut current = Url::parse(url).map_err(|_| Failure::Redirect)?;
        for _ in 0..=MAX_REDIRECTS {
            validate_url(&current, api)?;
            let mut request = self
                .client
                .get(current.clone())
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
            return Ok(response);
        }
        Err(Failure::Redirect)
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, Failure> {
        self.get(url, true)?
            .json()
            .map_err(|_| Failure::ReleaseRejected)
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
        let response: Attestations = self.json(&format!(
            "https://api.github.com/repos/{REPOSITORY}/attestations/sha1:{commit}?predicate_type=release&limit=100"
        ))?;
        let mut bundles = Vec::new();
        for attestation in response.attestations {
            if let Some(bundle) = attestation.bundle {
                bundles
                    .push(serde_json::to_vec(&bundle).map_err(|_| Failure::AttestationRejected)?);
            } else if let Some(url) = attestation.bundle_url {
                let response = self.get(&url, true)?;
                let snappy = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    == Some("application/x-snappy");
                let mut bytes = Vec::new();
                if snappy {
                    snap::read::FrameDecoder::new(response)
                        .take(MAX_ATTESTATION_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| Failure::AttestationRejected)?;
                } else {
                    response
                        .take(MAX_ATTESTATION_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| Failure::AttestationRejected)?;
                }
                if bytes.len() as u64 > MAX_ATTESTATION_BYTES {
                    return Err(Failure::AttestationRejected);
                }
                bundles.push(bytes);
            } else {
                return Err(Failure::AttestationRejected);
            }
        }
        Ok(bundles)
    }

    fn download(&self, url: &str, limit: u64, output: &mut File) -> Result<u64, Failure> {
        let mut response = self.get(url, false)?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|_| Failure::DownloadRejected)?;
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
        Ok(total)
    }
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
    let actual = statement_subjects(statement, "sha256")?;
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
    let subjects = statement
        .get("subject")
        .and_then(serde_json::Value::as_array)
        .ok_or(Failure::AttestationRejected)?;
    let source = subjects
        .iter()
        .filter(|subject| {
            subject
                .pointer("/digest/sha1")
                .and_then(serde_json::Value::as_str)
                == Some(commit)
        })
        .count();
    let assets = statement_subjects(statement, "sha256")?;
    let expected = release
        .assets
        .iter()
        .map(|asset| Ok((asset.name.clone(), digest_value(&asset.digest)?)))
        .collect::<Result<BTreeMap<_, _>, Failure>>()?;
    if source != 1 || assets != expected {
        return Err(Failure::AttestationRejected);
    }
    Ok(())
}

fn statement_subjects(
    statement: &serde_json::Value,
    algorithm: &str,
) -> Result<BTreeMap<String, String>, Failure> {
    let subjects = statement
        .get("subject")
        .and_then(serde_json::Value::as_array)
        .ok_or(Failure::AttestationRejected)?;
    let mut result = BTreeMap::new();
    for subject in subjects {
        let Some(digest) = subject
            .get("digest")
            .and_then(|value| value.get(algorithm))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(raw_name) = subject.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let name = raw_name.strip_prefix("./").unwrap_or(raw_name);
        require_safe_name(name).map_err(|_| Failure::AttestationRejected)?;
        if result
            .insert(name.to_string(), digest.to_string())
            .is_some()
        {
            return Err(Failure::AttestationRejected);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const TAG: &str = "v0.3.0";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    struct FixtureSource {
        releases: RefCell<Vec<ReleaseReadback>>,
        payloads: BTreeMap<String, Vec<u8>>,
        failure: Option<Failure>,
    }

    impl PublicSource for FixtureSource {
        fn release(&self, _tag: &str) -> Result<ReleaseReadback, Failure> {
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
