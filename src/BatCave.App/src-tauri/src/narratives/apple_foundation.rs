use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    NarrativeAvailability, NarrativeFactPacket, NarrativeModelDownloadState, NarrativeModelStatus,
    NarrativeProvider, NarrativeProviderBackend, NarrativeProviderRequest, NarrativeResult,
    ProviderGeneration,
};

const SIDECAR_NAME: &str = "batcave-foundation-models";
const SIDECAR_PROTOCOL_VERSION: u8 = 1;
const MAX_SIDECAR_INPUT_BYTES: usize = 32 * 1024;
const MAX_SIDECAR_OUTPUT_BYTES: u64 = 4 * 1024;
const SIDECAR_TIMEOUT: Duration = Duration::from_secs(10);
const SIDECAR_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    Arc::new(AppleFoundationProvider::new(
        bundled_sidecar_path(resource_dir),
        SIDECAR_TIMEOUT,
    ))
}

#[derive(Debug)]
struct AppleFoundationProvider {
    sidecar_path: PathBuf,
    timeout: Duration,
    in_flight: AtomicBool,
    shutdown: AtomicBool,
}

impl AppleFoundationProvider {
    fn new(sidecar_path: PathBuf, timeout: Duration) -> Self {
        Self {
            sidecar_path,
            timeout,
            in_flight: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        }
    }

    fn availability(&self, request: &SidecarRequest<'_>) -> SidecarResponse {
        if self.shutdown.load(Ordering::SeqCst) {
            return SidecarResponse::unavailable(NarrativeAvailability::Unsupported);
        }
        let Some(_lease) = InvocationLease::acquire(&self.in_flight) else {
            return SidecarResponse::unavailable(NarrativeAvailability::Busy);
        };
        let cancelled = AtomicBool::new(false);
        self.invoke(request, &cancelled)
            .unwrap_or_else(SidecarResponse::unavailable)
    }

    fn invoke(
        &self,
        request: &SidecarRequest<'_>,
        cancelled: &AtomicBool,
    ) -> Result<SidecarResponse, NarrativeAvailability> {
        if cancelled.load(Ordering::SeqCst) {
            return Err(NarrativeAvailability::Busy);
        }
        let mut payload =
            serde_json::to_vec(request).map_err(|_| NarrativeAvailability::RuntimeMissing)?;
        if payload.len() + 1 > MAX_SIDECAR_INPUT_BYTES {
            return Err(NarrativeAvailability::Unsupported);
        }
        payload.push(b'\n');

        let mut child = SidecarChild::spawn(&self.sidecar_path)?;
        child.write_request(&payload)?;
        let started = Instant::now();
        loop {
            if cancelled.load(Ordering::SeqCst) || self.shutdown.load(Ordering::SeqCst) {
                return Err(NarrativeAvailability::Busy);
            }
            if started.elapsed() >= self.timeout {
                return Err(NarrativeAvailability::Busy);
            }
            match child.try_wait()? {
                Some(status) if status.success() => break,
                Some(_) => return Err(NarrativeAvailability::RuntimeMissing),
                None => thread::sleep(SIDECAR_POLL_INTERVAL),
            }
        }

        let output = child.read_output()?;
        let response: SidecarResponse =
            serde_json::from_slice(&output).map_err(|_| NarrativeAvailability::RuntimeMissing)?;
        if response.version != SIDECAR_PROTOCOL_VERSION {
            return Err(NarrativeAvailability::RuntimeMissing);
        }
        Ok(response)
    }
}

impl NarrativeProviderBackend for AppleFoundationProvider {
    fn provider(&self) -> NarrativeProvider {
        NarrativeProvider::AppleFoundation
    }

    fn model_status(&self) -> NarrativeModelStatus {
        let response = self.availability(&SidecarRequest::status());
        let availability = if response.result.is_none() {
            response.availability
        } else {
            NarrativeAvailability::RuntimeMissing
        };
        model_status(availability)
    }

    fn generate(
        &self,
        request: &NarrativeProviderRequest,
        facts: &NarrativeFactPacket,
        cancelled: &AtomicBool,
    ) -> ProviderGeneration {
        if self.shutdown.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) {
            return ProviderGeneration::Unavailable(NarrativeAvailability::Busy);
        }
        let Some(_lease) = InvocationLease::acquire(&self.in_flight) else {
            return ProviderGeneration::Unavailable(NarrativeAvailability::Busy);
        };
        let response = match self.invoke(&SidecarRequest::generate(request, facts), cancelled) {
            Ok(response) => response,
            Err(availability) => return ProviderGeneration::Unavailable(availability),
        };
        generation_from_response(response, request)
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn model_status(availability: NarrativeAvailability) -> NarrativeModelStatus {
    NarrativeModelStatus {
        provider: NarrativeProvider::AppleFoundation,
        availability,
        model_id: Some("system_language_model_default".to_string()),
        model_name: Some("Apple Foundation Model".to_string()),
        download_state: NarrativeModelDownloadState::NotRequired,
        download_size_bytes: None,
        downloaded_bytes: None,
        license_name: None,
        license_url: None,
        can_download: false,
        can_cancel_download: false,
        detail_code: match availability {
            NarrativeAvailability::Available => None,
            NarrativeAvailability::Unsupported => Some("apple_foundation_unsupported".to_string()),
            NarrativeAvailability::ModelNotReady => {
                Some("apple_foundation_model_not_ready".to_string())
            }
            NarrativeAvailability::RuntimeMissing => {
                Some("apple_foundation_runtime_missing".to_string())
            }
            NarrativeAvailability::Busy => Some("apple_foundation_busy".to_string()),
        },
    }
}

fn generation_from_response(
    response: SidecarResponse,
    request: &NarrativeProviderRequest,
) -> ProviderGeneration {
    match (response.availability, response.result) {
        (NarrativeAvailability::Available, Some(result))
            if result.provider == NarrativeProvider::AppleFoundation
                && result.publication_seq == request.publication_seq
                && result.fact_digest == request.fact_digest =>
        {
            ProviderGeneration::Completed(result.text)
        }
        (NarrativeAvailability::Available, _) => {
            ProviderGeneration::Unavailable(NarrativeAvailability::RuntimeMissing)
        }
        (availability, None) => ProviderGeneration::Unavailable(availability),
        (_, Some(_)) => ProviderGeneration::Unavailable(NarrativeAvailability::RuntimeMissing),
    }
}

fn bundled_sidecar_path(resource_dir: Option<&Path>) -> PathBuf {
    if let Some(resource_dir) = resource_dir {
        let resource_candidate = resource_dir.join(SIDECAR_NAME);
        if resource_candidate.is_file() {
            return resource_candidate;
        }
        if let Some(contents_dir) = resource_dir.parent() {
            let macos_candidate = contents_dir.join("MacOS").join(SIDECAR_NAME);
            if macos_candidate.is_file() {
                return macos_candidate;
            }
        }
    }

    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(SIDECAR_NAME));
    let directory = executable.parent().unwrap_or_else(|| Path::new("."));
    let direct = directory.join(SIDECAR_NAME);
    if direct.is_file() {
        return direct;
    }
    if directory.file_name().is_some_and(|name| name == "deps") {
        if let Some(parent) = directory.parent() {
            let test_candidate = parent.join(SIDECAR_NAME);
            if test_candidate.is_file() {
                return test_candidate;
            }
        }
    }
    direct
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SidecarRequest<'a> {
    version: u8,
    operation: SidecarOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<&'a NarrativeProviderRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facts: Option<&'a NarrativeFactPacket>,
}

impl<'a> SidecarRequest<'a> {
    fn status() -> Self {
        Self {
            version: SIDECAR_PROTOCOL_VERSION,
            operation: SidecarOperation::Status,
            request: None,
            facts: None,
        }
    }

    fn generate(request: &'a NarrativeProviderRequest, facts: &'a NarrativeFactPacket) -> Self {
        Self {
            version: SIDECAR_PROTOCOL_VERSION,
            operation: SidecarOperation::Generate,
            request: Some(request),
            facts: Some(facts),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SidecarOperation {
    Status,
    Generate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct SidecarResponse {
    version: u8,
    availability: NarrativeAvailability,
    #[serde(default)]
    result: Option<NarrativeResult>,
}

impl SidecarResponse {
    fn unavailable(availability: NarrativeAvailability) -> Self {
        Self {
            version: SIDECAR_PROTOCOL_VERSION,
            availability,
            result: None,
        }
    }
}

struct InvocationLease<'a> {
    in_flight: &'a AtomicBool,
}

impl<'a> InvocationLease<'a> {
    fn acquire(in_flight: &'a AtomicBool) -> Option<Self> {
        in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self { in_flight })
    }
}

impl Drop for InvocationLease<'_> {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }
}

struct SidecarChild {
    child: Child,
}

impl SidecarChild {
    fn spawn(path: &Path) -> Result<Self, NarrativeAvailability> {
        let child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| NarrativeAvailability::RuntimeMissing)?;
        Ok(Self { child })
    }

    fn write_request(&mut self, payload: &[u8]) -> Result<(), NarrativeAvailability> {
        let mut stdin = self
            .child
            .stdin
            .take()
            .ok_or(NarrativeAvailability::RuntimeMissing)?;
        stdin
            .write_all(payload)
            .map_err(|_| NarrativeAvailability::RuntimeMissing)
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, NarrativeAvailability> {
        self.child
            .try_wait()
            .map_err(|_| NarrativeAvailability::RuntimeMissing)
    }

    fn read_output(&mut self) -> Result<Vec<u8>, NarrativeAvailability> {
        let stdout = self
            .child
            .stdout
            .take()
            .ok_or(NarrativeAvailability::RuntimeMissing)?;
        let mut output = Vec::new();
        stdout
            .take(MAX_SIDECAR_OUTPUT_BYTES + 1)
            .read_to_end(&mut output)
            .map_err(|_| NarrativeAvailability::RuntimeMissing)?;
        if output.is_empty() || output.len() as u64 > MAX_SIDECAR_OUTPUT_BYTES {
            return Err(NarrativeAvailability::RuntimeMissing);
        }
        Ok(output)
    }
}

impl Drop for SidecarChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narratives::{
        NarrativeMeasurementLimitation, NarrativeRankingState, NarrativeResourceKind,
        NarrativeSurface,
    };

    #[test]
    fn generation_wire_excludes_subject_identity() {
        let request = NarrativeProviderRequest {
            surface: NarrativeSurface::WorkloadInsight,
            publication_seq: 9,
            fact_digest: "a".repeat(64),
        };
        let facts = fact_packet();
        let json = serde_json::to_string(&SidecarRequest::generate(&request, &facts))
            .expect("sidecar request serializes");
        assert!(json.contains("\"publication_seq\":9"));
        assert!(json.contains("\"facts\""));
        assert!(!json.contains("subject"));
        assert!(!json.contains("pid"));
        assert!(!json.contains("path"));
    }

    #[test]
    fn rejects_mismatched_generation_echoes() {
        let request = NarrativeProviderRequest {
            surface: NarrativeSurface::OverviewContributor,
            publication_seq: 7,
            fact_digest: "b".repeat(64),
        };
        let facts = fact_packet();
        let provider = AppleFoundationProvider::new(PathBuf::from("missing"), Duration::ZERO);
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            provider.generate(&request, &facts, &cancelled),
            ProviderGeneration::Unavailable(NarrativeAvailability::Busy)
        );

        let mismatched = SidecarResponse {
            version: SIDECAR_PROTOCOL_VERSION,
            availability: NarrativeAvailability::Available,
            result: Some(NarrativeResult {
                provider: NarrativeProvider::AppleFoundation,
                publication_seq: 8,
                fact_digest: request.fact_digest.clone(),
                text: "Memory is stable.".to_string(),
            }),
        };
        assert_eq!(
            generation_from_response(mismatched, &request),
            ProviderGeneration::Unavailable(NarrativeAvailability::RuntimeMissing)
        );
    }

    #[test]
    fn model_status_is_local_and_not_downloadable() {
        let status = model_status(NarrativeAvailability::ModelNotReady);
        assert_eq!(status.provider, NarrativeProvider::AppleFoundation);
        assert_eq!(
            status.download_state,
            NarrativeModelDownloadState::NotRequired
        );
        assert!(!status.can_download);
        assert!(!status.can_cancel_download);
        assert_eq!(
            status.detail_code.as_deref(),
            Some("apple_foundation_model_not_ready")
        );
    }

    fn fact_packet() -> NarrativeFactPacket {
        NarrativeFactPacket {
            display_name: "Example workload".to_string(),
            category: "Process".to_string(),
            metrics: Vec::new(),
            leading_resource: Some(NarrativeResourceKind::Memory),
            ranking_state: NarrativeRankingState::Leading,
            measurement_limitations: Vec::<NarrativeMeasurementLimitation>::new(),
        }
    }
}
