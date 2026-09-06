use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    atomic_json::write_bytes_atomic,
    persistence::{resolve_current_user_root, CurrentUserEnvironment, StoragePlatform},
};

#[cfg(target_os = "macos")]
mod apple_foundation;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod foundry_local;

const PREFERENCE_FILE_NAME: &str = "narrative-preferences.json";
const PREFERENCE_SCHEMA_VERSION: u8 = 1;
const MAX_PREFERENCE_BYTES: u64 = 4 * 1024;
const MAX_DISPLAY_NAME_CHARS: usize = 120;
const MAX_CATEGORY_CHARS: usize = 80;
const MAX_FACT_PACKET_BYTES: usize = 4 * 1024;
const MAX_SUBJECT_ID_CHARS: usize = 256;
const MAX_CACHE_ENTRIES: usize = 32;
const MIN_GENERATION_INTERVAL: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeProvider {
    AppleFoundation,
    FoundryLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeSurface {
    OverviewContributor,
    WorkloadInsight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeAvailability {
    Available,
    Unsupported,
    ModelNotReady,
    RuntimeMissing,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeRequest {
    pub surface: NarrativeSurface,
    pub publication_seq: u64,
    pub subject_stable_id: Option<String>,
    pub fact_digest: String,
}

/// Provider-visible request. Subject identity is intentionally excluded because a runtime stable
/// ID can contain a PID or other implementation detail that is outside the model fact allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeProviderRequest {
    pub surface: NarrativeSurface,
    pub publication_seq: u64,
    pub fact_digest: String,
    pub candidate_ids: Vec<NarrativeExplanationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeResult {
    pub provider: NarrativeProvider,
    pub publication_seq: u64,
    pub fact_digest: String,
    pub surface: NarrativeSurface,
    pub subject_stable_id: Option<String>,
    pub explanation_id: NarrativeExplanationId,
}

/// The model may select a measured resource; it cannot author a claim or its wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeExplanationId {
    CpuUsage,
    MemoryUsage,
    DiskActivity,
    NetworkActivity,
}

impl NarrativeExplanationId {
    fn for_resource(resource: NarrativeResourceKind) -> Self {
        match resource {
            NarrativeResourceKind::Cpu => Self::CpuUsage,
            NarrativeResourceKind::Memory => Self::MemoryUsage,
            NarrativeResourceKind::Io => Self::DiskActivity,
            NarrativeResourceKind::Network => Self::NetworkActivity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeGenerationResponse {
    pub availability: NarrativeAvailability,
    pub result: Option<NarrativeResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeResourceKind {
    Cpu,
    Memory,
    Io,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeMetricUnit {
    Percent,
    Megabytes,
    KilobytesPerSecond,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeRankingState {
    TopContributor,
    Leading,
    Notable,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeMeasurementQuality {
    Estimated,
    Limited,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeMetricFact {
    pub kind: NarrativeResourceKind,
    pub rounded_value: f64,
    pub unit: NarrativeMetricUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeMeasurementLimitation {
    pub kind: NarrativeResourceKind,
    pub quality: NarrativeMeasurementQuality,
}

/// The only workload data that a provider can receive. There are deliberately no paths, PIDs,
/// collector fields, raw diagnostics, executable metadata, or other processes in this DTO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeFactPacket {
    pub display_name: String,
    pub category: String,
    pub metrics: Vec<NarrativeMetricFact>,
    pub leading_resource: Option<NarrativeResourceKind>,
    pub ranking_state: NarrativeRankingState,
    pub measurement_limitations: Vec<NarrativeMeasurementLimitation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NarrativeModelDownloadState {
    NotRequired,
    NotDownloaded,
    Downloading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeModelStatus {
    pub provider: NarrativeProvider,
    pub availability: NarrativeAvailability,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub download_state: NarrativeModelDownloadState,
    pub download_size_bytes: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub license_name: Option<String>,
    pub license_url: Option<String>,
    pub can_download: bool,
    pub can_cancel_download: bool,
    pub detail_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativePreferences {
    pub enhanced_narratives: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderGeneration {
    Completed(String),
    Unavailable(NarrativeAvailability),
}

pub(crate) trait NarrativeProviderBackend: Send + Sync {
    fn provider(&self) -> NarrativeProvider;
    fn model_status(&self) -> NarrativeModelStatus;
    fn generate(
        &self,
        request: &NarrativeProviderRequest,
        facts: &NarrativeFactPacket,
        cancelled: &AtomicBool,
    ) -> ProviderGeneration;

    fn download_model(&self, _cancelled: &AtomicBool) -> NarrativeModelStatus {
        self.model_status()
    }

    fn cancel_download(&self) {}

    fn shutdown(&self) {}
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
#[derive(Debug)]
struct UnsupportedProvider {
    provider: NarrativeProvider,
    availability: NarrativeAvailability,
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl NarrativeProviderBackend for UnsupportedProvider {
    fn provider(&self) -> NarrativeProvider {
        self.provider
    }

    fn model_status(&self) -> NarrativeModelStatus {
        NarrativeModelStatus {
            provider: self.provider,
            availability: self.availability,
            model_id: None,
            model_name: None,
            download_state: NarrativeModelDownloadState::NotRequired,
            download_size_bytes: None,
            downloaded_bytes: None,
            license_name: None,
            license_url: None,
            can_download: false,
            can_cancel_download: false,
            detail_code: Some("narrative_provider_unavailable".to_string()),
        }
    }

    fn generate(
        &self,
        _request: &NarrativeProviderRequest,
        _facts: &NarrativeFactPacket,
        _cancelled: &AtomicBool,
    ) -> ProviderGeneration {
        ProviderGeneration::Unavailable(self.availability)
    }
}

#[cfg(target_os = "macos")]
fn platform_provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    apple_foundation::provider(resource_dir)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn platform_provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    foundry_local::provider(resource_dir)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    let _ = resource_dir;
    Arc::new(UnsupportedProvider {
        provider: NarrativeProvider::FoundryLocal,
        availability: NarrativeAvailability::RuntimeMissing,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct PersistedNarrativePreferences {
    schema_version: u8,
    enhanced_narratives: bool,
}

impl From<NarrativePreferences> for PersistedNarrativePreferences {
    fn from(value: NarrativePreferences) -> Self {
        Self {
            schema_version: PREFERENCE_SCHEMA_VERSION,
            enhanced_narratives: value.enhanced_narratives,
        }
    }
}

#[derive(Debug)]
struct NarrativePreferenceStore {
    path: PathBuf,
}

impl NarrativePreferenceStore {
    fn from_current_process() -> Result<Self, String> {
        let root = resolve_current_user_root(
            StoragePlatform::current(),
            &CurrentUserEnvironment::from_current_process(),
        )
        .map_err(|_| "narrative_preferences_root_unavailable".to_string())?;
        Ok(Self {
            path: root.directory.join(PREFERENCE_FILE_NAME),
        })
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Result<NarrativePreferences, String> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(NarrativePreferences::default());
            }
            Err(_) => return Err("narrative_preferences_read_failed".to_string()),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_PREFERENCE_BYTES {
            return Err("narrative_preferences_invalid".to_string());
        }
        let payload =
            fs::read(&self.path).map_err(|_| "narrative_preferences_read_failed".to_string())?;
        let persisted: PersistedNarrativePreferences = serde_json::from_slice(&payload)
            .map_err(|_| "narrative_preferences_invalid".to_string())?;
        if persisted.schema_version != PREFERENCE_SCHEMA_VERSION {
            return Err("narrative_preferences_version_unsupported".to_string());
        }
        Ok(NarrativePreferences {
            enhanced_narratives: persisted.enhanced_narratives,
        })
    }

    fn write_and_verify(&self, value: NarrativePreferences) -> Result<(), String> {
        let payload = serde_json::to_vec(&PersistedNarrativePreferences::from(value))
            .map_err(|_| "narrative_preferences_serialize_failed".to_string())?;
        write_bytes_atomic(&self.path, &payload)
            .map_err(|_| "narrative_preferences_write_failed".to_string())?;
        let observed = self.load()?;
        if observed != value {
            return Err("narrative_preferences_verification_failed".to_string());
        }
        Ok(())
    }
}

#[derive(Debug)]
struct PreferenceState {
    value: NarrativePreferences,
    revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SubjectKey {
    surface: NarrativeSurface,
    subject_stable_id: Option<String>,
}

impl From<&NarrativeRequest> for SubjectKey {
    fn from(request: &NarrativeRequest) -> Self {
        Self {
            surface: request.surface,
            subject_stable_id: request.subject_stable_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    provider: NarrativeProvider,
    subject: SubjectKey,
    fact_digest: String,
}

#[derive(Debug)]
struct ActiveGeneration {
    id: u64,
    subject: SubjectKey,
    publication_seq: u64,
    fact_digest: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct GenerationState {
    next_id: u64,
    active: Option<ActiveGeneration>,
    latest: HashMap<SubjectKey, (u64, String)>,
    cache: HashMap<CacheKey, NarrativeResult>,
    last_provider_call: Option<Instant>,
    closed: bool,
}

struct NarrativeCoordinator {
    provider: Arc<dyn NarrativeProviderBackend>,
    preference_store: Option<NarrativePreferenceStore>,
    preferences: Mutex<PreferenceState>,
    next_preference_revision: AtomicU64,
    generation: Mutex<GenerationState>,
    download_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Clone)]
pub(crate) struct NarrativeState {
    coordinator: Arc<NarrativeCoordinator>,
}

impl NarrativeState {
    pub(crate) fn new(resource_dir: Option<PathBuf>) -> Self {
        Self::with_provider_and_store(
            platform_provider(resource_dir.as_deref()),
            NarrativePreferenceStore::from_current_process().ok(),
        )
    }

    fn with_provider_and_store(
        provider: Arc<dyn NarrativeProviderBackend>,
        preference_store: Option<NarrativePreferenceStore>,
    ) -> Self {
        let preferences = preference_store
            .as_ref()
            .and_then(|store| store.load().ok())
            .unwrap_or_default();
        Self {
            coordinator: Arc::new(NarrativeCoordinator {
                provider,
                preference_store,
                preferences: Mutex::new(PreferenceState {
                    value: preferences,
                    revision: 0,
                }),
                next_preference_revision: AtomicU64::new(0),
                generation: Mutex::new(GenerationState::default()),
                download_cancel: Mutex::new(None),
            }),
        }
    }

    pub(crate) fn preferences(&self) -> NarrativePreferences {
        self.coordinator
            .preferences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .value
    }

    pub(crate) fn preferences_path(&self) -> Option<&Path> {
        self.coordinator
            .preference_store
            .as_ref()
            .map(|store| store.path.as_path())
    }

    pub(crate) fn set_enhanced_narratives(
        &self,
        enabled: bool,
    ) -> Result<NarrativePreferences, String> {
        let revision = self
            .coordinator
            .next_preference_revision
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.set_enhanced_narratives_at_revision(enabled, revision)
    }

    fn set_enhanced_narratives_at_revision(
        &self,
        enabled: bool,
        revision: u64,
    ) -> Result<NarrativePreferences, String> {
        let mut state = self
            .coordinator
            .preferences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if revision < state.revision {
            return Ok(state.value);
        }
        let value = NarrativePreferences {
            enhanced_narratives: enabled,
        };
        let store = self
            .coordinator
            .preference_store
            .as_ref()
            .ok_or_else(|| "narrative_preferences_root_unavailable".to_string())?;
        store.write_and_verify(value)?;
        state.value = value;
        state.revision = revision;
        if !enabled {
            self.cancel_generation();
        }
        Ok(value)
    }

    pub(crate) fn capability(&self) -> NarrativeModelStatus {
        self.coordinator.provider.model_status()
    }

    pub(crate) fn fact_digest(&self, facts: &NarrativeFactPacket) -> Result<String, String> {
        canonical_fact_digest(facts)
    }

    pub(crate) fn generate(
        &self,
        request: NarrativeRequest,
        facts: NarrativeFactPacket,
    ) -> Result<NarrativeGenerationResponse, String> {
        self.generate_at(request, facts, Instant::now())
    }

    fn generate_at(
        &self,
        request: NarrativeRequest,
        facts: NarrativeFactPacket,
        now: Instant,
    ) -> Result<NarrativeGenerationResponse, String> {
        validate_request(&request)?;
        let fact_digest = canonical_fact_digest(&facts)?;
        if request.fact_digest != fact_digest {
            return Err("narrative_fact_digest_mismatch".to_string());
        }
        if !self.preferences().enhanced_narratives {
            return Ok(unavailable(NarrativeAvailability::Unsupported));
        }

        let mut candidate_ids = admitted_candidates(&facts);
        if request.surface == NarrativeSurface::OverviewContributor {
            candidate_ids.retain(|id| {
                facts
                    .leading_resource
                    .is_some_and(|resource| *id == NarrativeExplanationId::for_resource(resource))
            });
        }
        let provider = self.coordinator.provider.provider();
        let subject = SubjectKey::from(&request);
        let cache_key = CacheKey {
            provider,
            subject: subject.clone(),
            fact_digest: fact_digest.clone(),
        };
        let (generation_id, cancelled) = {
            let mut state = self
                .coordinator
                .generation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.closed {
                return Ok(unavailable(NarrativeAvailability::Unsupported));
            }
            if let Some((latest_seq, latest_digest)) = state.latest.get(&subject) {
                if request.publication_seq < *latest_seq
                    || (request.publication_seq == *latest_seq
                        && request.fact_digest != *latest_digest)
                {
                    return Err("narrative_request_stale".to_string());
                }
            }
            state.latest.insert(
                subject.clone(),
                (request.publication_seq, request.fact_digest.clone()),
            );
            // Publish invalidation even when no choices remain, so older work cannot win.
            if candidate_ids.len() < 2 {
                if let Some(active) = &state.active {
                    active.cancelled.store(true, Ordering::SeqCst);
                }
                return Ok(NarrativeGenerationResponse {
                    availability: NarrativeAvailability::Available,
                    result: None,
                });
            }
            if let Some(cached) = state.cache.get(&cache_key) {
                return Ok(NarrativeGenerationResponse {
                    availability: NarrativeAvailability::Available,
                    result: Some(NarrativeResult {
                        publication_seq: request.publication_seq,
                        ..cached.clone()
                    }),
                });
            }
            if let Some(active) = &state.active {
                if active.subject != subject
                    || active.publication_seq != request.publication_seq
                    || active.fact_digest != request.fact_digest
                {
                    active.cancelled.store(true, Ordering::SeqCst);
                }
                return Ok(unavailable(NarrativeAvailability::Busy));
            }
            if state
                .last_provider_call
                .is_some_and(|last| now.saturating_duration_since(last) < MIN_GENERATION_INTERVAL)
            {
                return Ok(unavailable(NarrativeAvailability::Busy));
            }
            state.next_id = state.next_id.saturating_add(1);
            let id = state.next_id;
            let cancelled = Arc::new(AtomicBool::new(false));
            state.active = Some(ActiveGeneration {
                id,
                subject: subject.clone(),
                publication_seq: request.publication_seq,
                fact_digest: request.fact_digest.clone(),
                cancelled: Arc::clone(&cancelled),
            });
            state.last_provider_call = Some(now);
            (id, cancelled)
        };

        let provider_request = NarrativeProviderRequest {
            surface: request.surface,
            publication_seq: request.publication_seq,
            fact_digest: request.fact_digest.clone(),
            candidate_ids,
        };
        let generated = self
            .coordinator
            .provider
            .generate(&provider_request, &facts, &cancelled);

        let mut state = self
            .coordinator
            .generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == generation_id)
        {
            state.active = None;
        }
        if cancelled.load(Ordering::SeqCst) {
            return Ok(unavailable(NarrativeAvailability::Busy));
        }
        if state.latest.get(&subject)
            != Some(&(request.publication_seq, request.fact_digest.clone()))
        {
            return Err("narrative_result_stale".to_string());
        }
        let selected = match generated {
            ProviderGeneration::Completed(selected) => selected,
            ProviderGeneration::Unavailable(availability) => {
                return Ok(unavailable(availability));
            }
        };
        let explanation_id =
            validate_selected_explanation(&selected, &provider_request.candidate_ids)?;
        let result = NarrativeResult {
            provider,
            publication_seq: request.publication_seq,
            fact_digest,
            surface: request.surface,
            subject_stable_id: request.subject_stable_id,
            explanation_id,
        };
        if state.cache.len() >= MAX_CACHE_ENTRIES {
            state.cache.clear();
        }
        state.cache.insert(cache_key, result.clone());
        Ok(NarrativeGenerationResponse {
            availability: NarrativeAvailability::Available,
            result: Some(result),
        })
    }

    pub(crate) fn cancel_generation(&self) {
        if let Some(active) = self
            .coordinator
            .generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .as_ref()
        {
            active.cancelled.store(true, Ordering::SeqCst);
        }
    }

    pub(crate) fn download_model(&self) -> NarrativeModelStatus {
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut active = self
                .coordinator
                .download_cancel
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if active.is_some() {
                let mut status = self.capability();
                status.availability = NarrativeAvailability::Busy;
                return status;
            }
            *active = Some(Arc::clone(&cancelled));
        }
        let status = self.coordinator.provider.download_model(&cancelled);
        *self
            .coordinator
            .download_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        status
    }

    pub(crate) fn cancel_model_download(&self) -> NarrativeModelStatus {
        if let Some(cancelled) = self
            .coordinator
            .download_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.coordinator.provider.cancel_download();
        self.capability()
    }

    pub(crate) fn shutdown(&self) {
        {
            let mut state = self
                .coordinator
                .generation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.closed = true;
            if let Some(active) = &state.active {
                active.cancelled.store(true, Ordering::SeqCst);
            }
        }
        self.cancel_model_download();
        self.coordinator.provider.shutdown();
    }
}

impl Drop for NarrativeCoordinator {
    fn drop(&mut self) {
        if let Ok(state) = self.generation.get_mut() {
            if let Some(active) = &state.active {
                active.cancelled.store(true, Ordering::SeqCst);
            }
        }
        if let Ok(Some(cancelled)) = self.download_cancel.get_mut() {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.provider.shutdown();
    }
}

fn unavailable(availability: NarrativeAvailability) -> NarrativeGenerationResponse {
    NarrativeGenerationResponse {
        availability,
        result: None,
    }
}

fn validate_request(request: &NarrativeRequest) -> Result<(), String> {
    if request.publication_seq == 0 {
        return Err("narrative_publication_seq_invalid".to_string());
    }
    if request.fact_digest.len() != 64
        || !request
            .fact_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("narrative_fact_digest_invalid".to_string());
    }
    if request.subject_stable_id.as_ref().is_some_and(|subject| {
        subject.is_empty()
            || subject.chars().count() > MAX_SUBJECT_ID_CHARS
            || subject.chars().any(char::is_control)
    }) {
        return Err("narrative_subject_invalid".to_string());
    }
    Ok(())
}

fn canonical_fact_digest(facts: &NarrativeFactPacket) -> Result<String, String> {
    validate_fact_packet(facts)?;
    let payload = serde_json::to_vec(facts).map_err(|_| "narrative_facts_invalid".to_string())?;
    if payload.len() > MAX_FACT_PACKET_BYTES {
        return Err("narrative_facts_too_large".to_string());
    }
    let digest = Sha256::digest(&payload);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Ok(encoded)
}

fn validate_fact_packet(facts: &NarrativeFactPacket) -> Result<(), String> {
    validate_plain_label(
        &facts.display_name,
        MAX_DISPLAY_NAME_CHARS,
        "narrative_display_name_invalid",
    )?;
    validate_plain_label(
        &facts.category,
        MAX_CATEGORY_CHARS,
        "narrative_category_invalid",
    )?;
    if facts.metrics.is_empty()
        || facts.metrics.len() > 4
        || facts.measurement_limitations.len() > 4
    {
        return Err("narrative_facts_too_many_metrics".to_string());
    }
    let mut metric_kinds = HashSet::new();
    for metric in &facts.metrics {
        if !metric.rounded_value.is_finite()
            || metric.rounded_value < 0.0
            || metric.rounded_value > 1_000_000_000.0
            || metric.unit
                != match metric.kind {
                    NarrativeResourceKind::Cpu => NarrativeMetricUnit::Percent,
                    NarrativeResourceKind::Memory => NarrativeMetricUnit::Megabytes,
                    NarrativeResourceKind::Io | NarrativeResourceKind::Network => {
                        NarrativeMetricUnit::KilobytesPerSecond
                    }
                }
            || !is_rounded_metric(metric)
            || !metric_kinds.insert(metric.kind)
        {
            return Err("narrative_metric_invalid".to_string());
        }
    }
    let mut limitation_kinds = HashSet::new();
    if facts
        .measurement_limitations
        .iter()
        .any(|limitation| !limitation_kinds.insert(limitation.kind))
    {
        return Err("narrative_limitation_invalid".to_string());
    }
    if facts
        .leading_resource
        .is_some_and(|kind| !metric_kinds.contains(&kind))
        || limitation_kinds
            .iter()
            .any(|kind| !metric_kinds.contains(kind))
    {
        return Err("narrative_metric_missing".to_string());
    }
    Ok(())
}

fn is_rounded_metric(metric: &NarrativeMetricFact) -> bool {
    let scale = if metric.kind == NarrativeResourceKind::Cpu {
        10.0
    } else {
        1.0
    };
    metric.rounded_value == (metric.rounded_value * scale).round() / scale
}

fn validate_plain_label(value: &str, max_chars: usize, code: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed != value
        || trimmed.chars().count() > max_chars
        || trimmed.chars().any(char::is_control)
    {
        return Err(code.to_string());
    }
    Ok(())
}

/// Eligibility comes only from the validated measured value and its quality. A rank or
/// display name can never manufacture a pressure, severity, causal, or advice claim.
fn admitted_candidates(facts: &NarrativeFactPacket) -> Vec<NarrativeExplanationId> {
    [
        NarrativeResourceKind::Cpu,
        NarrativeResourceKind::Memory,
        NarrativeResourceKind::Io,
        NarrativeResourceKind::Network,
    ]
    .into_iter()
    .filter(|kind| {
        facts
            .metrics
            .iter()
            .any(|metric| metric.kind == *kind && metric.rounded_value > 0.0)
            && !facts.measurement_limitations.iter().any(|limitation| {
                limitation.kind == *kind
                    && matches!(
                        limitation.quality,
                        NarrativeMeasurementQuality::Stale
                            | NarrativeMeasurementQuality::Unavailable
                    )
            })
    })
    .map(NarrativeExplanationId::for_resource)
    .collect()
}

fn validate_selected_explanation(
    selected: &str,
    candidates: &[NarrativeExplanationId],
) -> Result<NarrativeExplanationId, String> {
    let id = match selected.trim() {
        "cpu_usage" => NarrativeExplanationId::CpuUsage,
        "memory_usage" => NarrativeExplanationId::MemoryUsage,
        "disk_activity" => NarrativeExplanationId::DiskActivity,
        "network_activity" => NarrativeExplanationId::NetworkActivity,
        _ => return Err("narrative_selection_invalid".to_string()),
    };
    if !candidates.contains(&id) {
        return Err("narrative_selection_not_offered".to_string());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug)]
    struct FakeProvider {
        calls: AtomicUsize,
        response: Mutex<ProviderGeneration>,
    }

    impl FakeProvider {
        fn new(response: ProviderGeneration) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: Mutex::new(response),
            }
        }
    }

    impl NarrativeProviderBackend for FakeProvider {
        fn provider(&self) -> NarrativeProvider {
            NarrativeProvider::FoundryLocal
        }

        fn model_status(&self) -> NarrativeModelStatus {
            NarrativeModelStatus {
                provider: NarrativeProvider::FoundryLocal,
                availability: NarrativeAvailability::Available,
                model_id: Some("fixture".to_string()),
                model_name: Some("Fixture".to_string()),
                download_state: NarrativeModelDownloadState::Ready,
                download_size_bytes: None,
                downloaded_bytes: None,
                license_name: None,
                license_url: None,
                can_download: false,
                can_cancel_download: false,
                detail_code: None,
            }
        }

        fn generate(
            &self,
            _request: &NarrativeProviderRequest,
            _facts: &NarrativeFactPacket,
            _cancelled: &AtomicBool,
        ) -> ProviderGeneration {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    fn facts() -> NarrativeFactPacket {
        NarrativeFactPacket {
            display_name: "Safari".to_string(),
            category: "Web browsing".to_string(),
            metrics: vec![
                NarrativeMetricFact {
                    kind: NarrativeResourceKind::Cpu,
                    rounded_value: 5.0,
                    unit: NarrativeMetricUnit::Percent,
                },
                NarrativeMetricFact {
                    kind: NarrativeResourceKind::Memory,
                    rounded_value: 522.0,
                    unit: NarrativeMetricUnit::Megabytes,
                },
            ],
            leading_resource: Some(NarrativeResourceKind::Cpu),
            ranking_state: NarrativeRankingState::TopContributor,
            measurement_limitations: Vec::new(),
        }
    }

    fn request(facts: &NarrativeFactPacket, publication_seq: u64) -> NarrativeRequest {
        NarrativeRequest {
            surface: NarrativeSurface::WorkloadInsight,
            publication_seq,
            subject_stable_id: Some("workload:test".to_string()),
            fact_digest: canonical_fact_digest(facts).expect("digest"),
        }
    }

    fn state(provider: Arc<dyn NarrativeProviderBackend>, directory: &Path) -> NarrativeState {
        let store = NarrativePreferenceStore::at(directory.join(PREFERENCE_FILE_NAME));
        let state = NarrativeState::with_provider_and_store(provider, Some(store));
        state
            .set_enhanced_narratives(true)
            .expect("enables narratives");
        state
    }

    #[test]
    fn preferences_default_false_and_round_trip_only_after_durable_verification() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = NarrativePreferenceStore::at(directory.path().join(PREFERENCE_FILE_NAME));
        assert_eq!(
            store.load().expect("default"),
            NarrativePreferences::default()
        );
        store
            .write_and_verify(NarrativePreferences {
                enhanced_narratives: true,
            })
            .expect("write verifies");
        assert!(store.load().expect("reload").enhanced_narratives);
        let persisted = fs::read_to_string(directory.path().join(PREFERENCE_FILE_NAME))
            .expect("persisted JSON");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&persisted).expect("valid JSON"),
            serde_json::json!({"schema_version": 1, "enhanced_narratives": true})
        );
    }

    #[test]
    fn preference_writes_are_latest_wins_and_memory_changes_only_after_verification() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Unavailable(
            NarrativeAvailability::Unsupported,
        )));
        let state = NarrativeState::with_provider_and_store(
            provider,
            Some(NarrativePreferenceStore::at(
                directory.path().join(PREFERENCE_FILE_NAME),
            )),
        );
        let newest = state
            .set_enhanced_narratives_at_revision(true, 2)
            .expect("newest write persists");
        assert!(newest.enhanced_narratives);
        let stale = state
            .set_enhanced_narratives_at_revision(false, 1)
            .expect("stale write returns current value");
        assert!(stale.enhanced_narratives);
        assert!(state.preferences().enhanced_narratives);

        let blocked_parent = directory.path().join("blocked");
        fs::write(&blocked_parent, b"not a directory").expect("blocking file");
        let blocked_state = NarrativeState::with_provider_and_store(
            Arc::new(FakeProvider::new(ProviderGeneration::Unavailable(
                NarrativeAvailability::Unsupported,
            ))),
            Some(NarrativePreferenceStore::at(
                blocked_parent.join(PREFERENCE_FILE_NAME),
            )),
        );
        assert_eq!(
            blocked_state.set_enhanced_narratives(true),
            Err("narrative_preferences_write_failed".to_string())
        );
        assert!(!blocked_state.preferences().enhanced_narratives);
    }

    #[test]
    fn corrupt_or_oversized_preferences_fail_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join(PREFERENCE_FILE_NAME);
        fs::write(&path, b"{broken").expect("corrupt fixture");
        let state = NarrativeState::with_provider_and_store(
            Arc::new(FakeProvider::new(ProviderGeneration::Completed(
                "Fine.".to_string(),
            ))),
            Some(NarrativePreferenceStore::at(path.clone())),
        );
        assert!(!state.preferences().enhanced_narratives);
        fs::write(path, vec![b'x'; MAX_PREFERENCE_BYTES as usize + 1]).expect("oversized fixture");
        assert!(
            NarrativePreferenceStore::at(directory.path().join(PREFERENCE_FILE_NAME))
                .load()
                .is_err()
        );
    }

    #[test]
    fn fact_digest_is_canonical_and_allowlisted() {
        let facts = facts();
        let first = canonical_fact_digest(&facts).expect("first digest");
        let second = canonical_fact_digest(&facts).expect("second digest");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);

        let mut duplicate = facts.clone();
        duplicate.metrics.push(duplicate.metrics[0].clone());
        assert_eq!(
            canonical_fact_digest(&duplicate),
            Err("narrative_metric_invalid".to_string())
        );
    }

    #[test]
    fn generation_validates_digest_caches_and_rate_limits_provider_calls() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "cpu_usage".to_string(),
        )));
        let state = state(provider.clone(), directory.path());
        let facts = facts();
        let first_request = request(&facts, 7);
        let now = Instant::now();
        let result = state
            .generate_at(first_request.clone(), facts.clone(), now)
            .expect("generation succeeds");
        assert_eq!(result.availability, NarrativeAvailability::Available);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        let cached = state
            .generate_at(first_request, facts.clone(), now)
            .expect("cache succeeds");
        assert_eq!(cached, result);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        let mut changed = facts;
        changed.metrics[0].rounded_value = 6.0;
        let limited = state
            .generate_at(request(&changed, 8), changed, now)
            .expect("rate limit is availability");
        assert_eq!(limited.availability, NarrativeAvailability::Busy);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_publications_and_digest_mismatches_are_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "cpu_usage".to_string(),
        )));
        let state = state(provider, directory.path());
        let facts = facts();
        let now = Instant::now();
        state
            .generate_at(request(&facts, 10), facts.clone(), now)
            .expect("first result");
        assert_eq!(
            state.generate_at(
                request(&facts, 9),
                facts.clone(),
                now + MIN_GENERATION_INTERVAL
            ),
            Err("narrative_request_stale".to_string())
        );
        let mut wrong = request(&facts, 11);
        wrong.fact_digest = "a".repeat(64);
        assert_eq!(
            state.generate_at(wrong, facts, now + MIN_GENERATION_INTERVAL),
            Err("narrative_fact_digest_mismatch".to_string())
        );
    }

    #[test]
    fn a_changed_subject_cancels_the_single_in_flight_generation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "Generated.".to_string(),
        )));
        let state = state(provider, directory.path());
        let facts = facts();
        let active_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut generation = state
                .coordinator
                .generation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            generation.active = Some(ActiveGeneration {
                id: 1,
                subject: SubjectKey {
                    surface: NarrativeSurface::WorkloadInsight,
                    subject_stable_id: Some("workload:old".to_string()),
                },
                publication_seq: 4,
                fact_digest: canonical_fact_digest(&facts).expect("digest"),
                cancelled: Arc::clone(&active_cancel),
            });
        }
        let mut next = request(&facts, 5);
        next.subject_stable_id = Some("workload:new".to_string());
        let response = state.generate(next, facts).expect("busy fallback");
        assert_eq!(response.availability, NarrativeAvailability::Busy);
        assert!(active_cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn low_cpu_never_admits_heavy_pressure_from_vocabulary_overlap() {
        let mut facts = facts();
        facts.metrics[0].rounded_value = 0.1;
        let candidates = admitted_candidates(&facts);
        assert_eq!(
            candidates,
            vec![
                NarrativeExplanationId::CpuUsage,
                NarrativeExplanationId::MemoryUsage
            ]
        );
        for invented in [
            "Safari is showing heavy CPU pressure right now.",
            "cpu_heavy_pressure",
            "cpu_usage. Restart Safari.",
            "{\"explanation_id\":\"cpu_usage\",\"text\":\"heavy CPU pressure\"}",
        ] {
            assert!(validate_selected_explanation(invented, &candidates).is_err());
        }
        assert_eq!(
            validate_selected_explanation("cpu_usage", &candidates),
            Ok(NarrativeExplanationId::CpuUsage)
        );
    }

    #[test]
    fn candidates_exclude_zero_stale_unavailable_and_absent_measurements() {
        let mut facts = facts();
        facts.metrics[0].rounded_value = 0.0;
        assert_eq!(
            admitted_candidates(&facts),
            vec![NarrativeExplanationId::MemoryUsage]
        );
        for quality in [
            NarrativeMeasurementQuality::Stale,
            NarrativeMeasurementQuality::Unavailable,
        ] {
            facts.measurement_limitations = vec![NarrativeMeasurementLimitation {
                kind: NarrativeResourceKind::Memory,
                quality,
            }];
            assert!(admitted_candidates(&facts).is_empty());
        }
        assert_eq!(
            validate_selected_explanation("network_activity", &admitted_candidates(&facts)),
            Err("narrative_selection_not_offered".to_string())
        );
    }

    #[test]
    fn evidence_rejects_wrong_units_unrounded_values_and_dangling_quality() {
        let mut wrong = facts();
        wrong.metrics[0].unit = NarrativeMetricUnit::Megabytes;
        assert!(canonical_fact_digest(&wrong).is_err());
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.11, 0.100_000_000_01] {
            let mut wrong = facts();
            wrong.metrics[0].rounded_value = invalid;
            assert!(canonical_fact_digest(&wrong).is_err());
        }
        let mut wrong = facts();
        wrong.metrics[1].rounded_value = 1.5;
        assert!(canonical_fact_digest(&wrong).is_err());
        wrong = facts();
        wrong
            .measurement_limitations
            .push(NarrativeMeasurementLimitation {
                kind: NarrativeResourceKind::Network,
                quality: NarrativeMeasurementQuality::Unavailable,
            });
        assert!(canonical_fact_digest(&wrong).is_err());
    }

    #[test]
    fn no_meaningful_choice_never_calls_provider() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "cpu_usage".to_string(),
        )));
        let state = state(provider.clone(), directory.path());
        let mut facts = facts();
        facts.metrics[1].rounded_value = 0.0;
        assert!(state
            .generate(request(&facts, 1), facts)
            .unwrap()
            .result
            .is_none());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provider_cannot_return_prose_or_an_unoffered_resource() {
        for selection in [
            "Safari is showing heavy CPU pressure right now.",
            "network_activity",
            "cpu_usage. Restart Safari.",
        ] {
            let directory = tempfile::tempdir().expect("tempdir");
            let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
                selection.to_string(),
            )));
            let state = state(provider.clone(), directory.path());
            let mut facts = facts();
            facts.metrics[0].rounded_value = 0.1;
            assert!(state.generate(request(&facts, 1), facts).is_err());
            assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn overview_skips_inference_and_no_choice_invalidates_in_flight_evidence() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "cpu_usage".to_string(),
        )));
        let state = state(provider.clone(), directory.path());
        let mut facts = facts();
        facts.leading_resource = Some(NarrativeResourceKind::Memory);
        let overview = NarrativeRequest {
            surface: NarrativeSurface::OverviewContributor,
            ..request(&facts, 1)
        };
        assert!(state
            .generate(overview, facts.clone())
            .unwrap()
            .result
            .is_none());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

        let cancelled = Arc::new(AtomicBool::new(false));
        state.coordinator.generation.lock().unwrap().active = Some(ActiveGeneration {
            id: 1,
            subject: SubjectKey::from(&request(&facts, 1)),
            publication_seq: 1,
            fact_digest: canonical_fact_digest(&facts).unwrap(),
            cancelled: Arc::clone(&cancelled),
        });
        facts.metrics[0].rounded_value = 0.0;
        assert!(state
            .generate(request(&facts, 2), facts)
            .unwrap()
            .result
            .is_none());
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn result_binds_selection_to_subject_surface_evidence_and_current_publication() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "cpu_usage".to_string(),
        )));
        let state = state(provider.clone(), directory.path());
        let facts = facts();
        let request = request(&facts, 1);
        let result = state
            .generate(request.clone(), facts.clone())
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result.subject_stable_id, request.subject_stable_id);
        assert_eq!(result.surface, request.surface);
        assert_eq!(result.fact_digest, request.fact_digest);
        assert_eq!(result.explanation_id, NarrativeExplanationId::CpuUsage);
        let next = NarrativeRequest {
            publication_seq: 2,
            ..request
        };
        assert_eq!(
            state
                .generate(next, facts)
                .unwrap()
                .result
                .unwrap()
                .publication_seq,
            2
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert!(!serde_json::to_string(&result).unwrap().contains("text"));
    }

    #[test]
    fn disabled_preference_never_calls_provider() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "Generated.".to_string(),
        )));
        let state = NarrativeState::with_provider_and_store(
            provider.clone(),
            Some(NarrativePreferenceStore::at(
                directory.path().join(PREFERENCE_FILE_NAME),
            )),
        );
        let facts = facts();
        let response = state.generate(request(&facts, 1), facts).expect("fallback");
        assert_eq!(response.availability, NarrativeAvailability::Unsupported);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shutdown_marks_generation_coordinator_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let provider = Arc::new(FakeProvider::new(ProviderGeneration::Completed(
            "Generated.".to_string(),
        )));
        let state = state(provider, directory.path());
        state.shutdown();
        let facts = facts();
        let response = state
            .generate(request(&facts, 1), facts)
            .expect("closed fallback");
        assert_eq!(response.availability, NarrativeAvailability::Unsupported);
    }
}
