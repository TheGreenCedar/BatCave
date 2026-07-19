use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use foundry_local_sdk::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestUserMessage, FoundryLocalConfig, FoundryLocalManager,
};
use serde::{Deserialize, Serialize};

use super::{
    NarrativeAvailability, NarrativeFactPacket, NarrativeModelDownloadState, NarrativeModelStatus,
    NarrativeProvider, NarrativeProviderBackend, NarrativeProviderRequest, NarrativeSurface,
    ProviderGeneration,
};
use crate::{
    atomic_json::write_bytes_atomic,
    persistence::{resolve_current_user_root, CurrentUserEnvironment, StoragePlatform},
};

const RECEIPT_SCHEMA: &str = "batcave_foundry_receipt_v1";
const RECEIPT_FILE: &str = "foundry-model-receipt.json";
const MAX_RECEIPT_BYTES: u64 = 8 * 1024;
const MODEL_ID: &str = "qwen2.5-0.5b-instruct-generic-cpu:4";
const MODEL_NAME: &str = "Qwen 2.5 0.5B (CPU)";
const MODEL_VERSION: u64 = 4;
const MODEL_DOWNLOAD_BYTES: u64 = 822 * 1024 * 1024;
const MODEL_LICENSE: &str = "Apache-2.0";
const MODEL_LICENSE_URL: &str =
    "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/blob/main/LICENSE";

#[cfg(target_os = "windows")]
const CORE_LIBRARY: &str = "Microsoft.AI.Foundry.Local.Core.dll";
#[cfg(target_os = "linux")]
const CORE_LIBRARY: &str = "Microsoft.AI.Foundry.Local.Core.so";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ModelReceipt {
    schema_version: String,
    model_id: String,
    model_version: u64,
    model_path: PathBuf,
}

pub(super) fn provider(resource_dir: Option<&Path>) -> Arc<dyn NarrativeProviderBackend> {
    Arc::new(FoundryProvider::new(resource_dir))
}

#[derive(Debug)]
struct FoundryProvider {
    app_data_dir: Option<PathBuf>,
    model_cache_dir: Option<PathBuf>,
    receipt_path: Option<PathBuf>,
    library_path: Option<PathBuf>,
    downloading: AtomicBool,
    download_cancelled: Arc<AtomicBool>,
    downloaded_bytes: Arc<AtomicU64>,
    download_failed: AtomicBool,
}

impl FoundryProvider {
    fn new(resource_dir: Option<&Path>) -> Self {
        let storage_root = resolve_current_user_root(
            StoragePlatform::current(),
            &CurrentUserEnvironment::from_current_process(),
        )
        .ok()
        .map(|root| root.directory.join("narratives/foundry"));
        let app_data_dir = storage_root.as_ref().map(|root| root.join("runtime"));
        let model_cache_dir = storage_root.as_ref().map(|root| root.join("models"));
        let receipt_path = storage_root.as_ref().map(|root| root.join(RECEIPT_FILE));
        let library_path = resolve_library_path(resource_dir);
        Self {
            app_data_dir,
            model_cache_dir,
            receipt_path,
            library_path,
            downloading: AtomicBool::new(false),
            download_cancelled: Arc::new(AtomicBool::new(false)),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            download_failed: AtomicBool::new(false),
        }
    }

    fn runtime_ready(&self) -> bool {
        self.library_path.as_deref().is_some_and(valid_regular_file)
            && self.app_data_dir.is_some()
            && self.model_cache_dir.is_some()
            && self.receipt_path.is_some()
    }

    fn valid_receipt(&self) -> Option<ModelReceipt> {
        let receipt_path = self.receipt_path.as_deref()?;
        let model_cache_dir = self.model_cache_dir.as_deref()?;
        read_valid_receipt(receipt_path, model_cache_dir).ok()
    }

    fn status(&self) -> NarrativeModelStatus {
        let runtime_ready = self.runtime_ready();
        let downloading = self.downloading.load(Ordering::SeqCst);
        let receipt_ready = runtime_ready && self.valid_receipt().is_some();
        let failed = self.download_failed.load(Ordering::SeqCst);
        NarrativeModelStatus {
            provider: NarrativeProvider::FoundryLocal,
            availability: if downloading {
                NarrativeAvailability::Busy
            } else if !runtime_ready {
                NarrativeAvailability::RuntimeMissing
            } else if receipt_ready {
                NarrativeAvailability::Available
            } else {
                NarrativeAvailability::ModelNotReady
            },
            model_id: Some(MODEL_ID.to_string()),
            model_name: Some(MODEL_NAME.to_string()),
            download_state: if downloading {
                NarrativeModelDownloadState::Downloading
            } else if receipt_ready {
                NarrativeModelDownloadState::Ready
            } else if failed {
                NarrativeModelDownloadState::Failed
            } else {
                NarrativeModelDownloadState::NotDownloaded
            },
            download_size_bytes: Some(MODEL_DOWNLOAD_BYTES),
            downloaded_bytes: if downloading {
                Some(self.downloaded_bytes.load(Ordering::SeqCst))
            } else if receipt_ready {
                Some(MODEL_DOWNLOAD_BYTES)
            } else {
                None
            },
            license_name: Some(MODEL_LICENSE.to_string()),
            license_url: Some(MODEL_LICENSE_URL.to_string()),
            can_download: runtime_ready && !downloading && !receipt_ready,
            can_cancel_download: runtime_ready && downloading,
            detail_code: if downloading {
                Some("narrative_model_downloading".to_string())
            } else if !runtime_ready {
                Some("narrative_foundry_runtime_missing".to_string())
            } else if receipt_ready {
                None
            } else if failed {
                Some("narrative_model_download_failed".to_string())
            } else {
                Some("narrative_model_download_required".to_string())
            },
        }
    }

    fn manager(&self) -> Result<&'static FoundryLocalManager, ()> {
        let app_data_dir = self.app_data_dir.as_deref().ok_or(())?;
        let model_cache_dir = self.model_cache_dir.as_deref().ok_or(())?;
        let library_path = self.library_path.as_deref().ok_or(())?;
        if !valid_regular_file(library_path) {
            return Err(());
        }
        fs::create_dir_all(app_data_dir).map_err(|_| ())?;
        fs::create_dir_all(model_cache_dir).map_err(|_| ())?;
        let logs_dir = app_data_dir.join("logs");
        fs::create_dir_all(&logs_dir).map_err(|_| ())?;
        FoundryLocalManager::create(
            FoundryLocalConfig::new("BatCave Monitor")
                .app_data_dir(app_data_dir.to_string_lossy())
                .model_cache_dir(model_cache_dir.to_string_lossy())
                .logs_dir(logs_dir.to_string_lossy())
                .library_path(library_path.to_string_lossy()),
        )
        .map_err(|_| ())
    }

    fn clear_receipt(&self) {
        if let Some(path) = &self.receipt_path {
            let _ = fs::remove_file(path);
        }
    }

    fn write_receipt(&self, model_path: &Path) -> Result<(), ()> {
        let receipt_path = self.receipt_path.as_deref().ok_or(())?;
        let model_cache_dir = self.model_cache_dir.as_deref().ok_or(())?;
        let canonical_model = model_path.canonicalize().map_err(|_| ())?;
        let canonical_cache = model_cache_dir.canonicalize().map_err(|_| ())?;
        if !canonical_model.starts_with(&canonical_cache) || !canonical_model.is_dir() {
            return Err(());
        }
        let receipt = ModelReceipt {
            schema_version: RECEIPT_SCHEMA.to_string(),
            model_id: MODEL_ID.to_string(),
            model_version: MODEL_VERSION,
            model_path: canonical_model,
        };
        let payload = serde_json::to_vec(&receipt).map_err(|_| ())?;
        write_bytes_atomic(receipt_path, &payload).map_err(|_| ())?;
        read_valid_receipt(receipt_path, model_cache_dir).map_err(|_| ())?;
        Ok(())
    }

    fn generate_inner(
        &self,
        request: &NarrativeProviderRequest,
        facts: &NarrativeFactPacket,
        cancelled: &AtomicBool,
    ) -> ProviderGeneration {
        if cancelled.load(Ordering::SeqCst) || self.valid_receipt().is_none() {
            return ProviderGeneration::Unavailable(NarrativeAvailability::ModelNotReady);
        }
        let manager = match self.manager() {
            Ok(manager) => manager,
            Err(()) => {
                return ProviderGeneration::Unavailable(NarrativeAvailability::RuntimeMissing)
            }
        };
        let result = tauri::async_runtime::block_on(async {
            let model = manager
                .catalog()
                .get_model_variant(MODEL_ID)
                .await
                .map_err(|_| NarrativeAvailability::RuntimeMissing)?;
            if !model
                .is_cached()
                .await
                .map_err(|_| NarrativeAvailability::RuntimeMissing)?
            {
                return Err(NarrativeAvailability::ModelNotReady);
            }
            if cancelled.load(Ordering::SeqCst) {
                return Err(NarrativeAvailability::Busy);
            }
            model
                .load()
                .await
                .map_err(|_| NarrativeAvailability::RuntimeMissing)?;
            let response = async {
                let messages = narrative_messages(request, facts)
                    .map_err(|_| NarrativeAvailability::RuntimeMissing)?;
                model
                    .create_chat_client()
                    .temperature(0.0)
                    .random_seed(0)
                    .max_tokens(64)
                    .complete_chat(&messages, None)
                    .await
                    .map_err(|_| NarrativeAvailability::RuntimeMissing)
            }
            .await;
            let _ = model.unload().await;
            let response = response?;
            if cancelled.load(Ordering::SeqCst) {
                return Err(NarrativeAvailability::Busy);
            }
            response
                .choices
                .first()
                .and_then(|choice| choice.message.content.as_deref())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned)
                .ok_or(NarrativeAvailability::RuntimeMissing)
        });
        match result {
            Ok(text) => ProviderGeneration::Completed(text),
            Err(NarrativeAvailability::ModelNotReady) => {
                self.clear_receipt();
                ProviderGeneration::Unavailable(NarrativeAvailability::ModelNotReady)
            }
            Err(availability) => ProviderGeneration::Unavailable(availability),
        }
    }

    fn download_inner(&self) -> NarrativeModelStatus {
        if !self.runtime_ready() {
            return self.status();
        }
        if self
            .downloading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return self.status();
        }
        self.download_cancelled.store(false, Ordering::SeqCst);
        self.download_failed.store(false, Ordering::SeqCst);
        self.downloaded_bytes.store(0, Ordering::SeqCst);
        self.clear_receipt();
        let outcome = self.download_model_explicitly();
        self.downloading.store(false, Ordering::SeqCst);
        if outcome.is_err() && !self.download_cancelled.load(Ordering::SeqCst) {
            self.download_failed.store(true, Ordering::SeqCst);
        }
        self.status()
    }

    fn download_model_explicitly(&self) -> Result<(), ()> {
        let manager = self.manager()?;
        let cancel = Arc::clone(&self.download_cancelled);
        let progress = Arc::clone(&self.downloaded_bytes);
        let model = tauri::async_runtime::block_on(async {
            let model = manager
                .catalog()
                .get_model_variant(MODEL_ID)
                .await
                .map_err(|_| ())?;
            validate_catalog_model(&model)?;
            if !model.is_cached().await.map_err(|_| ())? {
                model
                    .download_builder()
                    .progress(move |percent| {
                        let fraction = percent.clamp(0.0, 100.0) / 100.0;
                        progress.store(
                            (MODEL_DOWNLOAD_BYTES as f64 * fraction).round() as u64,
                            Ordering::SeqCst,
                        );
                    })
                    .cancel(cancel)
                    .run()
                    .await
                    .map_err(|_| ())?;
            }
            if self.download_cancelled.load(Ordering::SeqCst)
                || !model.is_cached().await.map_err(|_| ())?
            {
                return Err(());
            }
            let model_path = model.path().await.map_err(|_| ())?;
            Ok(model_path)
        })?;
        self.write_receipt(&model)?;
        self.downloaded_bytes
            .store(MODEL_DOWNLOAD_BYTES, Ordering::SeqCst);
        Ok(())
    }
}

impl NarrativeProviderBackend for FoundryProvider {
    fn provider(&self) -> NarrativeProvider {
        NarrativeProvider::FoundryLocal
    }

    fn model_status(&self) -> NarrativeModelStatus {
        self.status()
    }

    fn generate(
        &self,
        request: &NarrativeProviderRequest,
        facts: &NarrativeFactPacket,
        cancelled: &AtomicBool,
    ) -> ProviderGeneration {
        self.generate_inner(request, facts, cancelled)
    }

    fn download_model(&self, _cancelled: &AtomicBool) -> NarrativeModelStatus {
        self.download_inner()
    }

    fn cancel_download(&self) {
        self.download_cancelled.store(true, Ordering::SeqCst);
    }
}

fn resolve_library_path(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let packaged = resource_dir
        .map(|root| root.join("foundry-native").join(CORE_LIBRARY))
        .filter(|path| valid_regular_file(path));
    if packaged.is_some() {
        return packaged;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".generated/foundry-native")
        .join(CORE_LIBRARY)
        .is_file()
        .then(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".generated/foundry-native")
                .join(CORE_LIBRARY)
        })
}

fn valid_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
}

fn read_valid_receipt(receipt_path: &Path, model_cache_dir: &Path) -> Result<ModelReceipt, ()> {
    let metadata = fs::symlink_metadata(receipt_path).map_err(|_| ())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_RECEIPT_BYTES {
        return Err(());
    }
    let receipt: ModelReceipt =
        serde_json::from_slice(&fs::read(receipt_path).map_err(|_| ())?).map_err(|_| ())?;
    if receipt.schema_version != RECEIPT_SCHEMA
        || receipt.model_id != MODEL_ID
        || receipt.model_version != MODEL_VERSION
        || !receipt.model_path.is_absolute()
    {
        return Err(());
    }
    let canonical_cache = model_cache_dir.canonicalize().map_err(|_| ())?;
    let canonical_model = receipt.model_path.canonicalize().map_err(|_| ())?;
    if canonical_model != receipt.model_path
        || !canonical_model.starts_with(canonical_cache)
        || !canonical_model.is_dir()
    {
        return Err(());
    }
    Ok(receipt)
}

fn validate_catalog_model(model: &foundry_local_sdk::Model) -> Result<(), ()> {
    let info = model.info();
    if info.id != MODEL_ID
        || info.version != MODEL_VERSION
        || info.file_size_mb != Some(822)
        || !license_matches(info.license.as_deref())
        || info
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.execution_provider != "CPUExecutionProvider")
    {
        return Err(());
    }
    Ok(())
}

fn license_matches(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case(MODEL_LICENSE))
}

fn narrative_messages(
    request: &NarrativeProviderRequest,
    facts: &NarrativeFactPacket,
) -> Result<Vec<ChatCompletionRequestMessage>, serde_json::Error> {
    let surface = match request.surface {
        NarrativeSurface::OverviewContributor => "overview contributor explanation",
        NarrativeSurface::WorkloadInsight => "workload insight",
    };
    let system = ChatCompletionRequestSystemMessage::from(
        "Write exactly one plain-language sentence of at most 180 characters. Use only the supplied facts. Include the exact required workload name and required resource word. Do not add causes, advice, numbers, paths, IDs, or claims. Return the sentence only.",
    );
    let facts_json = serde_json::to_string(facts)?;
    let required_resource = match facts.leading_resource {
        Some(NarrativeResourceKind::Cpu) => "CPU",
        Some(NarrativeResourceKind::Memory) => "memory",
        Some(NarrativeResourceKind::Io) => "disk",
        Some(NarrativeResourceKind::Network) => "network",
        None => "activity",
    };
    let user = ChatCompletionRequestUserMessage::from(format!(
        "Surface: {surface}\nRequired workload: {}\nRequired resource: {required_resource}\nAllowed facts: {facts_json}",
        facts.display_name
    ));
    Ok(vec![system.into(), user.into()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn receipt_must_point_to_the_exact_pinned_model_inside_the_cache() {
        let temp = TempDir::new().expect("temp dir");
        let cache = temp.path().join("models");
        let model = cache.join("pinned");
        fs::create_dir_all(&model).expect("model dir");
        let receipt_path = temp.path().join(RECEIPT_FILE);
        let receipt = ModelReceipt {
            schema_version: RECEIPT_SCHEMA.to_string(),
            model_id: MODEL_ID.to_string(),
            model_version: MODEL_VERSION,
            model_path: model.canonicalize().expect("canonical model"),
        };
        fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert_eq!(
            read_valid_receipt(&receipt_path, &cache).unwrap().model_id,
            MODEL_ID
        );

        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let invalid = ModelReceipt {
            model_path: outside.canonicalize().unwrap(),
            ..receipt
        };
        fs::write(&receipt_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(read_valid_receipt(&receipt_path, &cache).is_err());
    }

    #[test]
    fn startup_status_does_not_initialize_or_download_foundry() {
        let provider = FoundryProvider {
            app_data_dir: Some(PathBuf::from("missing/runtime")),
            model_cache_dir: Some(PathBuf::from("missing/models")),
            receipt_path: Some(PathBuf::from("missing/receipt.json")),
            library_path: Some(PathBuf::from("missing/core")),
            downloading: AtomicBool::new(false),
            download_cancelled: Arc::new(AtomicBool::new(false)),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            download_failed: AtomicBool::new(false),
        };
        let status = provider.model_status();
        assert_eq!(status.availability, NarrativeAvailability::RuntimeMissing);
        assert!(!status.can_download);
        assert!(!Path::new("missing").exists());
    }

    #[test]
    fn catalog_license_matching_accepts_spdx_case_only() {
        assert!(license_matches(Some("apache-2.0")));
        assert!(license_matches(Some("Apache-2.0")));
        assert!(!license_matches(Some("MIT")));
        assert!(!license_matches(None));
    }
}
