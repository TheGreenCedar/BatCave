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
const MAX_RESULT_CHARS: usize = 180;
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NarrativeResult {
    pub provider: NarrativeProvider,
    pub publication_seq: u64,
    pub fact_digest: String,
    pub text: String,
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

fn platform_provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    #[cfg(target_os = "macos")]
    {
        return apple_foundation::provider(resource_dir);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        return foundry_local::provider(resource_dir);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = resource_dir;
        Arc::new(UnsupportedProvider {
            provider: NarrativeProvider::FoundryLocal,
            availability: NarrativeAvailability::RuntimeMissing,
        })
    }
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
            if let Some(cached) = state.cache.get(&cache_key) {
                return Ok(NarrativeGenerationResponse {
                    availability: NarrativeAvailability::Available,
                    result: Some(cached.clone()),
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
        let text = match generated {
            ProviderGeneration::Completed(text) => text,
            ProviderGeneration::Unavailable(availability) => {
                return Ok(unavailable(availability));
            }
        };
        validate_generated_text(&text, &facts)?;
        let result = NarrativeResult {
            provider,
            publication_seq: request.publication_seq,
            fact_digest,
            text: text.trim().to_string(),
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
        if let Ok(active) = self.download_cancel.get_mut() {
            if let Some(cancelled) = active {
                cancelled.store(true, Ordering::SeqCst);
            }
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
    Ok(())
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

fn validate_generated_text(text: &str, facts: &NarrativeFactPacket) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed != text
        || trimmed.chars().count() > MAX_RESULT_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err("narrative_result_invalid".to_string());
    }
    if sentence_boundary_count(trimmed) > 1 {
        return Err("narrative_result_not_one_sentence".to_string());
    }
    let allowed_numbers = fact_label_numbers(facts);
    if numeric_tokens(trimmed)
        .into_iter()
        .any(|number| !allowed_numbers.contains(&number))
    {
        return Err("narrative_result_new_numeric_claim".to_string());
    }
    if !has_required_grounding(trimmed, facts) {
        return Err("narrative_result_ungrounded".to_string());
    }
    Ok(())
}

fn sentence_boundary_count(value: &str) -> usize {
    let characters = value.char_indices().collect::<Vec<_>>();
    characters
        .iter()
        .enumerate()
        .filter(|(index, (_, character))| {
            if !matches!(character, '.' | '!' | '?') {
                return false;
            }
            let previous_is_digit = index
                .checked_sub(1)
                .and_then(|previous| characters.get(previous))
                .is_some_and(|(_, previous)| previous.is_ascii_digit());
            let next_is_digit = characters
                .get(index + 1)
                .is_some_and(|(_, next)| next.is_ascii_digit());
            if *character == '.' && previous_is_digit && next_is_digit {
                return false;
            }
            characters
                .get(index + 1)
                .is_none_or(|(_, next)| next.is_whitespace())
        })
        .count()
}

fn fact_label_numbers(facts: &NarrativeFactPacket) -> HashSet<String> {
    let mut numbers = HashSet::new();
    numbers.extend(numeric_tokens(&facts.display_name));
    numbers.extend(numeric_tokens(&facts.category));
    numbers
}

fn has_required_grounding(text: &str, facts: &NarrativeFactPacket) -> bool {
    const GENERIC_NAME_TOKENS: &[&str] = &[
        "app",
        "application",
        "gpu",
        "helper",
        "process",
        "renderer",
        "service",
        "utility",
        "worker",
    ];
    if !text.contains(&facts.display_name) {
        return false;
    }
    let text_tokens = normalized_words(text).into_iter().collect::<HashSet<_>>();
    let has_name = normalized_words(&facts.display_name)
        .into_iter()
        .any(|token| {
            token.chars().count() >= 3
                && !token.chars().all(|character| character.is_ascii_digit())
                && !GENERIC_NAME_TOKENS.contains(&token.as_str())
                && text_tokens.contains(&token)
        });
    if !has_name {
        return false;
    }

    let aliases: &[&str] = match facts.leading_resource {
        Some(NarrativeResourceKind::Cpu) => &["cpu", "processor"],
        Some(NarrativeResourceKind::Memory) => &["memory", "ram"],
        Some(NarrativeResourceKind::Io) => &["disk", "storage", "io"],
        Some(NarrativeResourceKind::Network) => &["network"],
        None => &[],
    };
    if !aliases.is_empty() && !aliases.iter().any(|alias| text_tokens.contains(*alias)) {
        return false;
    }

    let mut allowed = [
        "a",
        "active",
        "activity",
        "an",
        "and",
        "appears",
        "as",
        "at",
        "attention",
        "category",
        "contributor",
        "contributes",
        "contributing",
        "current",
        "currently",
        "dominant",
        "driver",
        "driving",
        "elevated",
        "for",
        "from",
        "has",
        "heavy",
        "highest",
        "in",
        "is",
        "its",
        "largest",
        "leader",
        "leading",
        "load",
        "main",
        "monitoring",
        "more",
        "most",
        "normal",
        "notable",
        "now",
        "of",
        "on",
        "other",
        "pressure",
        "primary",
        "remains",
        "resource",
        "resources",
        "right",
        "showing",
        "shows",
        "source",
        "steady",
        "surface",
        "than",
        "the",
        "this",
        "to",
        "top",
        "usage",
        "use",
        "uses",
        "using",
        "with",
        "workload",
        "cpu",
        "processor",
        "memory",
        "ram",
        "disk",
        "storage",
        "io",
        "network",
        "estimated",
        "limited",
        "stale",
        "unavailable",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<HashSet<_>>();
    allowed.extend(normalized_words(&facts.display_name));
    allowed.extend(normalized_words(&facts.category));
    text_tokens.is_subset(&allowed)
}

fn normalized_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn numeric_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_ascii_digit() || (character == '.' && !current.is_empty()) {
            current.push(character);
        } else if !current.is_empty() {
            if current.ends_with('.') {
                current.pop();
            }
            if !current.is_empty() {
                tokens.push(current.clone());
            }
            current.clear();
        }
    }
    if current.ends_with('.') {
        current.pop();
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
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
            metrics: vec![NarrativeMetricFact {
                kind: NarrativeResourceKind::Cpu,
                rounded_value: 5.0,
                unit: NarrativeMetricUnit::Percent,
            }],
            leading_resource: Some(NarrativeResourceKind::Cpu),
            ranking_state: NarrativeRankingState::TopContributor,
            measurement_limitations: Vec::new(),
        }
    }

    fn request(facts: &NarrativeFactPacket, publication_seq: u64) -> NarrativeRequest {
        NarrativeRequest {
            surface: NarrativeSurface::OverviewContributor,
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
            "Safari is the main source of current CPU activity.".to_string(),
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
            "Safari is the main CPU contributor.".to_string(),
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
    fn generated_copy_is_one_short_sentence_without_metric_numbers() {
        let facts = facts();
        assert!(
            validate_generated_text("Safari is the main source of CPU activity.", &facts).is_ok()
        );
        assert_eq!(
            validate_generated_text("Safari uses 5% CPU.", &facts),
            Err("narrative_result_new_numeric_claim".to_string())
        );
        assert_eq!(
            validate_generated_text("Safari uses 7% CPU.", &facts),
            Err("narrative_result_new_numeric_claim".to_string())
        );
        assert_eq!(
            validate_generated_text(
                "The surface area of a large project depends on its components and resources.",
                &facts
            ),
            Err("narrative_result_ungrounded".to_string())
        );
        assert_eq!(
            validate_generated_text("Safari is the main source of memory activity.", &facts),
            Err("narrative_result_ungrounded".to_string())
        );
        assert_eq!(
            validate_generated_text(
                "Safari uses a powerful CPU to perform browsing tasks efficiently.",
                &facts
            ),
            Err("narrative_result_ungrounded".to_string())
        );
        assert_eq!(
            validate_generated_text("safari is the main CPU contributor right now.", &facts),
            Err("narrative_result_ungrounded".to_string())
        );
        assert_eq!(
            validate_generated_text("Safari is active. No action is needed.", &facts),
            Err("narrative_result_not_one_sentence".to_string())
        );
        assert_eq!(
            validate_generated_text(&"x".repeat(MAX_RESULT_CHARS + 1), &facts),
            Err("narrative_result_invalid".to_string())
        );
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
