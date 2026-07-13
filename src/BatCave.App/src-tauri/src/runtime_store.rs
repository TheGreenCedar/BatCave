use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::elevation::{ElevatedHelperClient, ElevatedPoll};
use crate::{
    atomic_json::{write_json_atomic, AtomicJsonErrorLabels},
    contracts::{
        AccessState, MetricQuality, MetricQualityInfo, MetricSource, ProcessContributorSummary,
        ProcessFocusMode, ProcessSample, ProcessViewRow, ProcessViewRowKind, RuntimeAdminModeState,
        RuntimeAdminModeStatus, RuntimeEnvironment, RuntimeHealth, RuntimeInstallKind,
        RuntimePlatform, RuntimePrivilegedSource, RuntimeQuery, RuntimeSettings, RuntimeSnapshot,
        RuntimeWarning, SortColumn, SortDirection, SystemMemoryAccounting, SystemMetricsSnapshot,
        WarmCache,
    },
    runtime_provenance::RuntimeProvenance,
    telemetry::{now_ms, TelemetryCollector},
};

const SETTINGS_FILE: &str = "settings.json";
const WARM_CACHE_FILE: &str = "warm-cache.json";
const DIAGNOSTICS_FILE: &str = "diagnostics.jsonl";
const MAX_WARNINGS: usize = 16;
const WARM_CACHE_WRITE_INTERVAL_TICKS: u64 = 10;
const APP_CPU_DEGRADE_PCT: f64 = 25.0;
const APP_RSS_DEGRADE_BYTES: u64 = 350 * 1024 * 1024;
const ATTENTION_CPU_PERCENT: f64 = 10.0;
const ATTENTION_MEMORY_BYTES: u64 = 900 * 1024 * 1024;
const ATTENTION_IO_BPS: u64 = 500 * 1024;
const ATTENTION_NETWORK_BPS: u64 = 1024 * 1024;
const PROCESS_IO_BASELINE_PENDING: &str = "Process read/write I/O rates need a fresh prior sample.";
const PROCESS_OTHER_IO_BASELINE_PENDING: &str =
    "Process Other I/O rates need a fresh prior sample.";
const PERSISTENCE_JSON_ERRORS: AtomicJsonErrorLabels = AtomicJsonErrorLabels {
    write_failed: "persistence_write_failed",
    serialize_failed: "persistence_serialize_failed",
    replace_failed: "persistence_replace_failed",
    rename_failed: "persistence_rename_failed",
    serialize_error_includes_path: true,
};

pub struct RuntimeState {
    store: Arc<Mutex<RuntimeStore>>,
    worker_started: AtomicBool,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self::from_base_dir(default_base_dir())
    }

    pub(crate) fn from_base_dir(base_dir: PathBuf) -> Self {
        Self {
            store: Arc::new(Mutex::new(RuntimeStore::from_base_dir(base_dir))),
            worker_started: AtomicBool::new(false),
        }
    }

    pub fn start(&self) {
        if self.worker_started.swap(true, AtomicOrdering::AcqRel) {
            return;
        }
        let store = Arc::downgrade(&self.store);
        std::thread::spawn(move || {
            while let Some(store) = store.upgrade() {
                let delay = if let Ok(mut store) = store.lock() {
                    if !store.settings.paused {
                        store.tick();
                    } else {
                        if store.resolve_elevated_request() {
                            store.publish_snapshot_only(None);
                        }
                    }
                    Duration::from_millis(store.settings.sample_interval_ms.into())
                } else {
                    Duration::from_secs(1)
                };
                std::thread::sleep(delay);
            }
        });
    }

    pub fn snapshot(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.snapshot())
    }

    pub fn refresh_now(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.refresh_now())
    }

    pub fn pause(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_paused(true))
    }

    pub fn resume(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_paused(false))
    }

    pub fn set_query(&self, query: RuntimeQuery) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_query(query))
    }

    pub fn set_sample_interval(&self, sample_interval_ms: u32) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_sample_interval(sample_interval_ms))
    }

    pub fn set_admin_mode(&self, enabled: bool) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_admin_mode(enabled))
    }

    pub fn has_process_exe(&self, exe: &str) -> Result<bool, String> {
        self.with_store(|store| store.has_process_exe(exe))
    }

    fn with_store<T>(&self, action: impl FnOnce(&mut RuntimeStore) -> T) -> Result<T, String> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| "runtime store lock is poisoned".to_string())?;
        Ok(action(&mut store))
    }
}

struct RuntimeStore {
    collector: TelemetryCollector,
    base_dir: PathBuf,
    provenance: RuntimeProvenance,
    settings: RuntimeSettings,
    admin_mode: RuntimeAdminModeStatus,
    elevated: Option<ElevatedHelperClient>,
    elevated_request: Option<Receiver<Result<ElevatedHelperClient, String>>>,
    snapshot: RuntimeSnapshot,
    warnings: VecDeque<RuntimeWarning>,
    previous_totals: Option<TelemetryTotals>,
    previous_processes: Vec<ProcessSample>,
    live_process_snapshot: bool,
    tick_p95: P95Window,
    sort_p95: P95Window,
    jitter_p95: P95Window,
    last_tick_at: Option<Instant>,
    dropped_ticks: u64,
    publication_seq: u64,
    sample_seq: u64,
    sampled_at_ms: Option<u64>,
}

impl RuntimeStore {
    #[cfg(test)]
    fn new() -> Self {
        Self::from_base_dir(default_base_dir())
    }

    fn from_base_dir(base_dir: PathBuf) -> Self {
        ElevatedHelperClient::remove_stale_artifacts(&base_dir);
        let provenance = RuntimeProvenance::detect(&base_dir);
        let mut warnings = VecDeque::new();
        if let Some(warning) = provenance.privilege_warning() {
            push_warning(&mut warnings, 0, "admin_mode", warning.to_string());
        }
        let settings =
            read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE)).unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                RuntimeSettings::default()
            });
        let settings_requested_admin = settings.admin_mode_requested;
        let mut warm_cache = read_json::<WarmCache>(&base_dir.join(WARM_CACHE_FILE))
            .unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                WarmCache {
                    seq: 0,
                    rows: Vec::new(),
                }
            });
        let publication_seq = warm_cache.seq;
        let mut settings = normalize_settings(settings);
        settings.admin_mode_enabled = provenance.process_is_elevated();
        if settings_requested_admin {
            warm_cache.rows.clear();
            let _ = fs::remove_file(base_dir.join(WARM_CACHE_FILE));
        } else {
            warm_cache.rows = hold_process_rates(warm_cache.rows);
        }
        let admin_mode = provenance.admin_mode_status();
        let snapshot = build_snapshot(
            publication_seq,
            now_ms(),
            0,
            None,
            provenance.environment(),
            &settings,
            &admin_mode,
            RuntimeHealth::default(),
            empty_system(),
            &warm_cache.rows,
            shape_rows(&warm_cache.rows, &settings.query),
            warnings.iter().cloned().collect(),
        );

        Self {
            collector: TelemetryCollector::new(),
            base_dir,
            provenance,
            settings,
            admin_mode,
            elevated: None,
            elevated_request: None,
            snapshot,
            warnings,
            previous_totals: None,
            previous_processes: warm_cache.rows,
            live_process_snapshot: false,
            tick_p95: P95Window::new(120),
            sort_p95: P95Window::new(120),
            jitter_p95: P95Window::new(120),
            last_tick_at: None,
            dropped_ticks: 0,
            publication_seq,
            sample_seq: 0,
            sampled_at_ms: None,
        }
    }

    fn snapshot(&mut self) -> RuntimeSnapshot {
        self.snapshot.clone()
    }

    fn refresh_now(&mut self) -> RuntimeSnapshot {
        self.tick()
    }

    fn set_paused(&mut self, paused: bool) -> RuntimeSnapshot {
        self.settings.paused = paused;
        if paused {
            self.live_process_snapshot = false;
        }
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_admin_mode(&mut self, enabled: bool) -> RuntimeSnapshot {
        let enabled = enabled && self.provenance.environment().admin_mode_available;
        let was_admin_enabled = self.settings.admin_mode_enabled;
        if enabled && self.provenance.process_is_elevated() {
            self.settings.admin_mode_requested = false;
            self.settings.admin_mode_enabled = true;
            self.admin_mode = self.provenance.admin_mode_status();
            self.clear_warning("admin_mode");
        } else if enabled {
            self.settings.admin_mode_requested = true;
            self.settings.admin_mode_enabled = false;
            self.admin_mode.state = RuntimeAdminModeState::Requesting;
            self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
            self.admin_mode.detail = None;
            self.clear_warning("admin_mode");
            if self.elevated.is_none() && self.elevated_request.is_none() {
                let collect_process_network =
                    !self.collector.process_network_ready().unwrap_or(false);
                let base_dir = self.base_dir.clone();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = sender.send(ElevatedHelperClient::start(
                        &base_dir,
                        collect_process_network,
                    ));
                });
                self.elevated_request = Some(receiver);
            }
        } else {
            self.settings.admin_mode_requested = false;
            self.settings.admin_mode_enabled = self.provenance.process_is_elevated();
            self.admin_mode = self.provenance.admin_mode_status();
            self.clear_warning("admin_mode");
            if let Some(mut client) = self.elevated.take() {
                if let Err(error) = client.stop() {
                    self.add_warning("admin_mode", error);
                }
            }
            let _ = self.collector.retry_process_network();
            if was_admin_enabled {
                self.previous_processes.clear();
                self.previous_totals = None;
                self.live_process_snapshot = false;
            }
            self.purge_warm_cache();
            ElevatedHelperClient::remove_stale_artifacts(&self.base_dir);
        }
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_query(&mut self, query: RuntimeQuery) -> RuntimeSnapshot {
        self.settings.query = normalize_query(query);
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_sample_interval(&mut self, sample_interval_ms: u32) -> RuntimeSnapshot {
        self.settings.sample_interval_ms = sample_interval_ms.clamp(500, 5_000);
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn has_process_exe(&mut self, exe: &str) -> bool {
        let exe = exe.trim();
        self.live_process_snapshot
            && !exe.is_empty()
            && self
                .snapshot
                .processes
                .iter()
                .any(|process| process.exe.eq_ignore_ascii_case(exe))
    }

    fn tick(&mut self) -> RuntimeSnapshot {
        let previous_process_baseline_live = self.live_process_snapshot;
        self.live_process_snapshot = false;
        self.resolve_elevated_request();
        let tick_started = Instant::now();
        let previous_tick_at = self.last_tick_at.replace(tick_started);
        let sample = match self.collector.collect() {
            Ok(sample) => sample,
            Err(error) => {
                self.publish_snapshot_only(Some(("collector", error)));
                return self.snapshot.clone();
            }
        };
        let tick_ms = tick_started.elapsed().as_secs_f64() * 1000.0;
        self.tick_p95.add(tick_ms);

        if let Some(previous) = previous_tick_at {
            let elapsed_ms = tick_started.duration_since(previous).as_secs_f64() * 1000.0;
            let jitter = (elapsed_ms - 1000.0).abs();
            self.jitter_p95.add(jitter);
            if jitter > 1500.0 {
                self.dropped_ticks = self.dropped_ticks.saturating_add((jitter / 1000.0) as u64);
            }
        }

        let sample_ts_ms = now_ms();
        let mut active_collector_warnings = sample.warnings;

        let elapsed_seconds = self
            .previous_totals
            .as_ref()
            .and_then(|previous| {
                let delta_ms = sample_ts_ms.saturating_sub(previous.ts_ms);
                (delta_ms > 0).then_some(delta_ms as f64 / 1000.0)
            })
            .unwrap_or(1.0)
            .max(0.5);
        let mut system = sample.system;
        let mut sample_processes = sample.processes;
        let mut process_rows_fresh = true;
        if self.settings.admin_mode_requested {
            let was_admin_enabled = self.settings.admin_mode_enabled;
            let mut admin_warning = None;
            let mut admin_recovered = false;
            let mut stop_elevated = false;
            if let Some(elevated) = &mut self.elevated {
                let collects_process_network = elevated.collects_process_network();
                match elevated.poll_rows() {
                    Ok(ElevatedPoll::Fresh { mut rows, warnings }) => {
                        merge_main_network_attribution(&sample_processes, &mut rows);
                        sample_processes = rows;
                        self.settings.admin_mode_enabled = true;
                        self.admin_mode.state = RuntimeAdminModeState::Active;
                        self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
                        self.admin_mode.detail = None;
                        self.admin_mode.last_success_at_ms = Some(sample_ts_ms);
                        admin_recovered = true;
                        if collects_process_network {
                            active_collector_warnings.retain(|warning| {
                                warning_key("collector", warning) != "collector.network_attribution"
                            });
                        }
                        active_collector_warnings.extend(warnings);
                    }
                    Ok(ElevatedPoll::Held { warnings }) => {
                        sample_processes = self.previous_processes.clone();
                        process_rows_fresh = false;
                        self.settings.admin_mode_enabled = true;
                        self.admin_mode.state = RuntimeAdminModeState::Active;
                        self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
                        self.admin_mode.detail = None;
                        if collects_process_network {
                            active_collector_warnings.retain(|warning| {
                                warning_key("collector", warning) != "collector.network_attribution"
                            });
                        }
                        active_collector_warnings.extend(warnings);
                    }
                    Ok(ElevatedPoll::Recovering(detail)) => {
                        self.settings.admin_mode_enabled = false;
                        self.admin_mode.state = RuntimeAdminModeState::Recovering;
                        self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
                        self.admin_mode.detail = Some(detail);
                    }
                    Ok(ElevatedPoll::Pending) => {
                        self.settings.admin_mode_enabled = false;
                        self.admin_mode.state = RuntimeAdminModeState::Requesting;
                        self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
                        self.admin_mode.detail = None;
                    }
                    Err(error) => {
                        self.settings.admin_mode_enabled = false;
                        admin_warning = Some(error);
                        stop_elevated = true;
                    }
                }
            } else {
                self.settings.admin_mode_enabled = false;
            }
            if was_admin_enabled != self.settings.admin_mode_enabled {
                self.previous_processes.clear();
            }
            if admin_recovered {
                self.clear_warning("admin_mode");
            }
            if stop_elevated {
                self.settings.admin_mode_requested = false;
                if let Some(mut client) = self.elevated.take() {
                    if let Err(error) = client.stop() {
                        self.add_warning("admin_mode", error);
                    }
                }
            }
            if let Some(error) = admin_warning {
                self.fail_admin_mode(error);
            }
        } else {
            self.settings.admin_mode_enabled = self.provenance.process_is_elevated();
            if self.admin_mode.source == RuntimePrivilegedSource::CurrentProcess
                && self.admin_mode.state == RuntimeAdminModeState::Active
            {
                self.admin_mode.last_success_at_ms = Some(sample_ts_ms);
            }
        }
        self.sync_collector_warnings(active_collector_warnings);
        if let Some(previous) = &self.previous_totals {
            let disk_rates_are_native = system
                .quality
                .as_ref()
                .and_then(|quality| quality.disk.as_ref())
                .and_then(|quality| quality.source)
                == Some(MetricSource::Procfs);
            if !disk_rates_are_native && system.disk_read_bps == 0 && system.disk_write_bps == 0 {
                system.disk_read_bps = byte_rate(
                    system.disk_read_total_bytes,
                    previous.disk_read_total_bytes,
                    elapsed_seconds,
                );
                system.disk_write_bps = byte_rate(
                    system.disk_write_total_bytes,
                    previous.disk_write_total_bytes,
                    elapsed_seconds,
                );
            }
            let network_rates_are_native = system
                .quality
                .as_ref()
                .and_then(|quality| quality.network.as_ref())
                .and_then(|quality| quality.source)
                == Some(MetricSource::Procfs);
            if !network_rates_are_native {
                system.network_received_bps = byte_rate(
                    system.network_received_total_bytes,
                    previous.network_received_total_bytes,
                    elapsed_seconds,
                );
                system.network_transmitted_bps = byte_rate(
                    system.network_transmitted_total_bytes,
                    previous.network_transmitted_total_bytes,
                    elapsed_seconds,
                );
            }
        }

        let processes = if process_rows_fresh {
            add_process_rates(
                sample_processes,
                &self.previous_processes,
                elapsed_seconds,
                previous_process_baseline_live,
            )
        } else {
            hold_process_rates(sample_processes)
        };
        add_process_memory_accounting(&mut system, &processes);
        self.previous_processes = processes;
        self.live_process_snapshot = process_rows_fresh;
        self.previous_totals = Some(TelemetryTotals::from_system(&system, sample_ts_ms));
        self.publication_seq = self.publication_seq.saturating_add(1);
        self.sample_seq = self.sample_seq.saturating_add(1);
        self.sampled_at_ms = Some(sample_ts_ms);

        let sort_started = Instant::now();
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        let sort_ms = sort_started.elapsed().as_secs_f64() * 1000.0;
        self.sort_p95.add(sort_ms);

        let app_metrics = current_app_metrics(&self.previous_processes);
        let health = self.build_health(
            sample.latency_ms,
            app_metrics.cpu_percent,
            app_metrics.rss_bytes,
        );
        self.snapshot = build_snapshot(
            self.publication_seq,
            now_ms(),
            self.sample_seq,
            self.sampled_at_ms,
            self.provenance.environment(),
            &self.settings,
            &self.admin_mode,
            health,
            system,
            &self.previous_processes,
            rows,
            self.warnings.iter().cloned().collect(),
        );

        if self
            .sample_seq
            .is_multiple_of(WARM_CACHE_WRITE_INTERVAL_TICKS)
        {
            self.persist_warm_cache();
        }

        self.snapshot.clone()
    }

    fn fail_admin_mode(&mut self, error: String) {
        self.settings.admin_mode_requested = false;
        self.settings.admin_mode_enabled = false;
        self.admin_mode.state = RuntimeAdminModeState::Failed;
        self.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
        self.admin_mode.detail = Some(error.clone());
        self.add_warning("admin_mode", error);
        let _ = self.collector.retry_process_network();
        self.persist_settings();
    }

    fn resolve_elevated_request(&mut self) -> bool {
        let result = match self.elevated_request.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Disconnected)) => {
                Some(Err("admin_mode_launch_channel_closed".to_string()))
            }
            Some(Err(TryRecvError::Empty)) | None => None,
        };
        let Some(result) = result else {
            return false;
        };
        self.elevated_request = None;

        match (self.settings.admin_mode_requested, result) {
            (true, Ok(client)) => self.elevated = Some(client),
            (true, Err(error)) => self.fail_admin_mode(error),
            (false, Ok(mut client)) => {
                if let Err(error) = client.stop() {
                    self.fail_admin_mode(error);
                }
            }
            (false, Err(_)) => {}
        }
        true
    }

    fn publish_snapshot_only(&mut self, warning: Option<(&str, String)>) {
        self.publication_seq = self.publication_seq.saturating_add(1);
        if let Some((category, message)) = warning {
            self.add_warning(category, message);
        }

        let app_metrics = current_app_metrics(&self.previous_processes);
        let health = self.build_health(0, app_metrics.cpu_percent, app_metrics.rss_bytes);
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        self.snapshot = build_snapshot(
            self.publication_seq,
            now_ms(),
            self.sample_seq,
            self.sampled_at_ms,
            self.provenance.environment(),
            &self.settings,
            &self.admin_mode,
            health,
            self.snapshot.system.clone(),
            &self.previous_processes,
            rows,
            self.warnings.iter().cloned().collect(),
        );
    }

    fn build_health(
        &self,
        latency_ms: u64,
        app_cpu_percent: f64,
        app_rss_bytes: u64,
    ) -> RuntimeHealth {
        let cpu_degraded = app_cpu_percent >= APP_CPU_DEGRADE_PCT;
        let rss_degraded = app_rss_bytes >= APP_RSS_DEGRADE_BYTES;
        let warning_degraded_count = self
            .warnings
            .iter()
            .filter(|warning| warning_degrades_health(&warning.category))
            .count();
        let warning_degraded = warning_degraded_count > 0;
        let last_warning = self.warnings.back().map(|warning| warning.message.clone());
        let status_summary = if self.settings.paused {
            "Paused.".to_string()
        } else if self.admin_mode.state == RuntimeAdminModeState::Requesting {
            "Waiting for Windows approval.".to_string()
        } else if self.admin_mode.state == RuntimeAdminModeState::Recovering {
            "Privileged collection is recovering; standard monitoring remains current.".to_string()
        } else if warning_degraded {
            format!(
                "{warning_degraded_count} telemetry limitation{}.",
                if warning_degraded_count == 1 { "" } else { "s" }
            )
        } else if cpu_degraded && rss_degraded {
            "Degraded: app CPU and RSS above budget.".to_string()
        } else if cpu_degraded {
            "Degraded: app CPU above budget.".to_string()
        } else if rss_degraded {
            "Degraded: app RSS above budget.".to_string()
        } else {
            "Healthy.".to_string()
        };

        RuntimeHealth {
            tick_count: self.sample_seq,
            snapshot_latency_ms: latency_ms,
            degraded: cpu_degraded || rss_degraded || warning_degraded,
            collector_warnings: warning_degraded_count,
            runtime_loop_enabled: true,
            runtime_loop_running: !self.settings.paused,
            status_summary,
            updated_at_ms: now_ms(),
            tick_p95_ms: round1(self.tick_p95.value()),
            sort_p95_ms: round1(self.sort_p95.value()),
            jitter_p95_ms: round1(self.jitter_p95.value()),
            dropped_ticks: self.dropped_ticks,
            app_cpu_percent: round1(app_cpu_percent),
            app_rss_bytes,
            last_warning,
        }
    }

    fn add_warning(&mut self, category: &str, message: String) {
        let key = warning_key(category, &message);
        if self
            .warnings
            .iter()
            .any(|warning| warning.key == key && warning.message == message)
        {
            return;
        }
        if let Some(index) = self.warnings.iter().position(|warning| warning.key == key) {
            self.warnings.remove(index);
        }
        push_warning(
            &mut self.warnings,
            self.publication_seq,
            category,
            message.clone(),
        );
        self.append_diagnostic(category, &message);
    }

    fn sync_collector_warnings(&mut self, messages: Vec<String>) {
        let active = messages
            .iter()
            .map(|message| warning_key("collector", message))
            .collect::<HashSet<_>>();
        let stale = self
            .warnings
            .iter()
            .filter(|warning| warning.category == "collector" && !active.contains(&warning.key))
            .map(|warning| warning.key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            self.clear_warning(&key);
        }
        for message in messages {
            self.add_warning("collector", message);
        }
    }

    fn clear_warning(&mut self, key: &str) {
        if let Some(index) = self.warnings.iter().position(|warning| warning.key == key) {
            let warning = self.warnings.remove(index).expect("warning index exists");
            self.append_diagnostic("recovery", &format!("{} resolved", warning.key));
        }
    }

    fn persist_settings(&mut self) {
        let mut persisted = self.settings.clone();
        persisted.admin_mode_requested = false;
        persisted.admin_mode_enabled = false;
        if let Err(error) = write_json_atomic(
            &self.base_dir.join(SETTINGS_FILE),
            &persisted,
            PERSISTENCE_JSON_ERRORS,
        ) {
            self.add_warning("persistence", error);
        }
    }

    fn persist_warm_cache(&mut self) {
        if self.settings.admin_mode_enabled {
            return;
        }
        let cache = WarmCache {
            seq: self.publication_seq,
            rows: self.previous_processes.clone(),
        };
        if let Err(error) = write_json_atomic(
            &self.base_dir.join(WARM_CACHE_FILE),
            &cache,
            PERSISTENCE_JSON_ERRORS,
        ) {
            self.add_warning("persistence", error);
        }
    }

    fn purge_warm_cache(&mut self) {
        let path = self.base_dir.join(WARM_CACHE_FILE);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.add_warning(
                "persistence",
                format!(
                    "persistence_remove_failed path={} error={}",
                    path.display(),
                    error
                ),
            ),
        }
    }

    fn append_diagnostic(&self, category: &str, message: &str) {
        let path = self.base_dir.join(DIAGNOSTICS_FILE);
        let payload = serde_json::json!({
            "ts_ms": now_ms(),
            "category": category,
            "payload": { "message": message },
        });
        let _ = fs::create_dir_all(&self.base_dir);
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= 1024 * 1024) {
            let backup = self.base_dir.join("diagnostics.jsonl.1");
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(&path, backup);
        }
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                use std::io::Write;
                writeln!(file, "{payload}")
            });
    }
}

impl Drop for RuntimeStore {
    fn drop(&mut self) {
        if let Some(mut client) = self.elevated.take() {
            let _ = client.stop();
        }
        ElevatedHelperClient::remove_stale_artifacts(&self.base_dir);
    }
}

fn warning_degrades_health(category: &str) -> bool {
    matches!(category, "collector" | "admin_mode" | "persistence")
}

#[derive(Debug, Clone)]
struct TelemetryTotals {
    ts_ms: u64,
    disk_read_total_bytes: u64,
    disk_write_total_bytes: u64,
    network_received_total_bytes: u64,
    network_transmitted_total_bytes: u64,
}

impl TelemetryTotals {
    fn from_system(system: &SystemMetricsSnapshot, ts_ms: u64) -> Self {
        Self {
            ts_ms,
            disk_read_total_bytes: system.disk_read_total_bytes,
            disk_write_total_bytes: system.disk_write_total_bytes,
            network_received_total_bytes: system.network_received_total_bytes,
            network_transmitted_total_bytes: system.network_transmitted_total_bytes,
        }
    }
}

struct P95Window {
    values: VecDeque<f64>,
    capacity: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct CurrentAppMetrics {
    cpu_percent: f64,
    rss_bytes: u64,
}

impl P95Window {
    fn new(capacity: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn add(&mut self, value: f64) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value.max(0.0));
    }

    fn value(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }

        let mut values = self.values.iter().copied().collect::<Vec<_>>();
        values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
        let index = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        values[index.min(values.len() - 1)]
    }
}

#[allow(clippy::too_many_arguments)]
fn build_snapshot(
    publication_seq: u64,
    published_at_ms: u64,
    sample_seq: u64,
    sampled_at_ms: Option<u64>,
    environment: &RuntimeEnvironment,
    settings: &RuntimeSettings,
    admin_mode: &RuntimeAdminModeStatus,
    health: RuntimeHealth,
    system: SystemMetricsSnapshot,
    all_processes: &[ProcessSample],
    processes: Vec<ProcessSample>,
    warnings: Vec<RuntimeWarning>,
) -> RuntimeSnapshot {
    let process_view_rows = shape_process_view(&processes, &settings.query);
    RuntimeSnapshot {
        event_kind: "runtime_snapshot".to_string(),
        publication_seq,
        published_at_ms,
        sample_seq,
        sampled_at_ms,
        source: "tauri_runtime".to_string(),
        environment: environment.clone(),
        admin_mode: admin_mode.clone(),
        settings: settings.clone(),
        health,
        system,
        process_contributors: summarize_process_contributors(all_processes),
        processes,
        process_view_rows,
        total_process_count: all_processes.len(),
        warnings,
    }
}

fn add_process_rates(
    mut processes: Vec<ProcessSample>,
    previous_processes: &[ProcessSample],
    elapsed_seconds: f64,
    baseline_is_live: bool,
) -> Vec<ProcessSample> {
    let previous_by_identity = previous_processes
        .iter()
        .filter(|process| process.start_time_ms != 0)
        .map(|process| ((process.pid.as_str(), process.start_time_ms), process))
        .collect::<HashMap<_, _>>();
    for process in &mut processes {
        let previous = (baseline_is_live && process.start_time_ms != 0)
            .then(|| previous_by_identity.get(&(process.pid.as_str(), process.start_time_ms)))
            .flatten()
            .copied();
        let current_io_quality = process
            .quality
            .as_ref()
            .and_then(|quality| quality.io.as_ref())
            .cloned();
        let current_other_io_quality = process
            .quality
            .as_ref()
            .and_then(|quality| quality.other_io.as_ref())
            .cloned();
        let previous_io_quality = previous
            .and_then(|process| process.quality.as_ref())
            .and_then(|quality| quality.io.as_ref());
        let previous_other_io_quality = previous
            .and_then(|process| process.quality.as_ref())
            .and_then(|quality| quality.other_io.as_ref());

        let io_baseline_is_valid = previous.is_some_and(|previous| {
            cumulative_baseline_is_compatible(
                current_io_quality.as_ref(),
                previous_io_quality,
                PROCESS_IO_BASELINE_PENDING,
            ) && process.io_read_total_bytes >= previous.io_read_total_bytes
                && process.io_write_total_bytes >= previous.io_write_total_bytes
        });
        if let Some(previous) = previous.filter(|_| io_baseline_is_valid) {
            process.io_read_bps = byte_rate(
                process.io_read_total_bytes,
                previous.io_read_total_bytes,
                elapsed_seconds,
            );
            process.io_write_bps = byte_rate(
                process.io_write_total_bytes,
                previous.io_write_total_bytes,
                elapsed_seconds,
            );
        } else {
            process.io_read_bps = 0;
            process.io_write_bps = 0;
            if cumulative_sample_is_valid(current_io_quality.as_ref()) {
                hold_process_io_rate_quality(process);
            }
        }

        let other_io_baseline_is_valid = match (
            process.other_io_total_bytes,
            previous.and_then(|process| process.other_io_total_bytes),
        ) {
            (Some(current), Some(previous)) => {
                current >= previous
                    && cumulative_baseline_is_compatible(
                        current_other_io_quality.as_ref(),
                        previous_other_io_quality,
                        PROCESS_OTHER_IO_BASELINE_PENDING,
                    )
            }
            _ => false,
        };
        if let (Some(current_total), Some(previous_total), true) = (
            process.other_io_total_bytes,
            previous.and_then(|process| process.other_io_total_bytes),
            other_io_baseline_is_valid,
        ) {
            process.other_io_bps = Some(byte_rate(current_total, previous_total, elapsed_seconds));
        } else {
            process.other_io_bps = None;
            if process.other_io_total_bytes.is_some()
                && cumulative_sample_is_valid(current_other_io_quality.as_ref())
            {
                hold_process_other_io_rate_quality(process);
            }
        }
    }

    processes
}

fn hold_process_rates(mut processes: Vec<ProcessSample>) -> Vec<ProcessSample> {
    for process in &mut processes {
        process.io_read_bps = 0;
        process.io_write_bps = 0;
        process.other_io_bps = None;
        hold_process_rate_quality(process);
    }
    processes
}

fn hold_process_rate_quality(process: &mut ProcessSample) {
    hold_process_io_rate_quality(process);
    if process.other_io_total_bytes.is_some() {
        hold_process_other_io_rate_quality(process);
    }
}

fn hold_process_io_rate_quality(process: &mut ProcessSample) {
    if let Some(quality) = process.quality.as_mut() {
        let Some(current) = quality.io.as_ref() else {
            return;
        };
        if !cumulative_sample_is_valid(Some(current)) {
            return;
        }
        let mut pending = current.clone();
        pending.quality = MetricQuality::Held;
        pending.message = Some(PROCESS_IO_BASELINE_PENDING.to_string());
        quality.io = Some(pending);
    }
}

fn hold_process_other_io_rate_quality(process: &mut ProcessSample) {
    if let Some(quality) = process.quality.as_mut() {
        let Some(current) = quality.other_io.as_ref() else {
            return;
        };
        if !cumulative_sample_is_valid(Some(current)) {
            return;
        }
        let mut pending = current.clone();
        pending.quality = MetricQuality::Held;
        pending.message = Some(PROCESS_OTHER_IO_BASELINE_PENDING.to_string());
        quality.other_io = Some(pending);
    }
}

fn cumulative_sample_is_valid(quality: Option<&MetricQualityInfo>) -> bool {
    quality.is_some_and(|quality| {
        matches!(
            quality.quality,
            MetricQuality::Native | MetricQuality::Estimated | MetricQuality::Partial
        )
    })
}

fn metric_is_pending_baseline(quality: &MetricQualityInfo, pending_message: &str) -> bool {
    // These Held markers are written only after a valid cumulative sample, so that sample may
    // become the baseline without treating collector-held or unavailable rows as counters.
    quality.quality == MetricQuality::Held && quality.message.as_deref() == Some(pending_message)
}

fn cumulative_baseline_is_compatible(
    current: Option<&MetricQualityInfo>,
    previous: Option<&MetricQualityInfo>,
    pending_message: &str,
) -> bool {
    let (Some(current), Some(previous)) = (current, previous) else {
        return false;
    };
    let sources_are_compatible = match (current.source, previous.source) {
        (Some(current), Some(previous)) => current == previous,
        _ => false,
    };
    cumulative_sample_is_valid(Some(current))
        && (cumulative_sample_is_valid(Some(previous))
            || metric_is_pending_baseline(previous, pending_message))
        && sources_are_compatible
}

fn merge_main_network_attribution(main: &[ProcessSample], elevated: &mut [ProcessSample]) {
    let by_identity = main
        .iter()
        .map(|process| ((process.pid.as_str(), process.start_time_ms), process))
        .collect::<HashMap<_, _>>();
    for process in elevated {
        let already_native = process.network_received_bps.is_some()
            && process.network_transmitted_bps.is_some()
            && process
                .quality
                .as_ref()
                .and_then(|quality| quality.network.as_ref())
                .is_some_and(|quality| quality.quality == MetricQuality::Native);
        if already_native {
            continue;
        }
        let Some(main) = by_identity.get(&(process.pid.as_str(), process.start_time_ms)) else {
            continue;
        };
        process.network_received_bps = main.network_received_bps;
        process.network_transmitted_bps = main.network_transmitted_bps;
        let main_quality = main
            .quality
            .as_ref()
            .and_then(|quality| quality.network.clone());
        process.quality.get_or_insert_with(Default::default).network = main_quality;
    }
}

fn shape_rows(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessSample> {
    let needle = query.filter_text.trim().to_lowercase();
    let mut rows = processes
        .iter()
        .filter(|process| {
            needle.is_empty()
                || process.name.to_lowercase().contains(&needle)
                || process.pid.contains(&needle)
                || process.exe.to_lowercase().contains(&needle)
        })
        .filter(|process| matches_focus_mode(process, query.focus_mode))
        .cloned()
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| compare_process(left, right, query));
    rows.truncate(query.limit.max(1));
    rows
}

#[derive(Debug, Clone)]
struct ProcessIdentity {
    icon_kind: &'static str,
    category: &'static str,
    is_child: bool,
}

#[derive(Debug, Clone)]
struct ProcessAppGroup {
    key: String,
    label: String,
    category: String,
    icon_kind: String,
    representative: ProcessSample,
    processes: Vec<ProcessSample>,
    cpu_percent: f64,
    memory_bytes: u64,
    io_bps: u64,
    network_bps: u64,
    threads: u64,
}

fn shape_process_view(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessViewRow> {
    let mut groups = Vec::<ProcessAppGroup>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for process in processes {
        let key = process_app_key(process);
        let index = if let Some(index) = group_indexes.get(&key) {
            *index
        } else {
            let identity = process_identity(process);
            let index = groups.len();
            group_indexes.insert(key.clone(), index);
            groups.push(ProcessAppGroup {
                key,
                label: process_app_label(process),
                category: identity.category.to_string(),
                icon_kind: identity.icon_kind.to_string(),
                representative: process.clone(),
                processes: Vec::new(),
                cpu_percent: 0.0,
                memory_bytes: 0,
                io_bps: 0,
                network_bps: 0,
                threads: 0,
            });
            index
        };

        let group = &mut groups[index];
        group.cpu_percent += process.cpu_percent;
        group.memory_bytes = group.memory_bytes.saturating_add(process.memory_bytes);
        group.io_bps = group.io_bps.saturating_add(process_io_rate(process));
        group.network_bps = group
            .network_bps
            .saturating_add(process_network_rate(process));
        group.threads = group.threads.saturating_add(process.threads as u64);
        group.processes.push(process.clone());
    }

    groups.sort_by(|left, right| compare_process_group(left, right, query));

    let mut rows = Vec::with_capacity(processes.len() + groups.len());
    for group in groups {
        let grouped = group.processes.len() > 1;
        let group_count = group.processes.len();
        if grouped {
            rows.push(ProcessViewRow {
                kind: ProcessViewRowKind::Group,
                process: None,
                representative: Some(group.representative.clone()),
                group_key: Some(group.key.clone()),
                group_label: Some(group.label.clone()),
                group_category: Some(group.category.clone()),
                group_count,
                icon_kind: group.icon_kind.clone(),
                is_child: false,
                is_grouped: true,
                attention_label: attention_label(
                    group.cpu_percent,
                    group.memory_bytes,
                    group.io_bps,
                    group.network_bps,
                    group
                        .processes
                        .iter()
                        .any(|process| process.access_state != AccessState::Full),
                ),
                cpu_percent: round1(group.cpu_percent),
                memory_bytes: group.memory_bytes,
                io_bps: group.io_bps,
                network_bps: group.network_bps,
                threads: group.threads,
            });
        }

        for process in group.processes {
            let identity = process_identity(&process);
            rows.push(ProcessViewRow {
                kind: ProcessViewRowKind::Process,
                process: Some(process.clone()),
                representative: None,
                group_key: Some(group.key.clone()),
                group_label: Some(group.label.clone()),
                group_category: Some(group.category.clone()),
                group_count,
                icon_kind: identity.icon_kind.to_string(),
                is_child: identity.is_child,
                is_grouped: grouped,
                attention_label: attention_label(
                    process.cpu_percent,
                    process.memory_bytes,
                    process_io_rate(&process),
                    process_network_rate(&process),
                    process.access_state != AccessState::Full,
                ),
                cpu_percent: process.cpu_percent,
                memory_bytes: process.memory_bytes,
                io_bps: process_io_rate(&process),
                network_bps: process_network_rate(&process),
                threads: process.threads as u64,
            });
        }
    }

    rows
}

fn compare_process(left: &ProcessSample, right: &ProcessSample, query: &RuntimeQuery) -> Ordering {
    let ordering = match query.sort_column {
        SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortColumn::Pid => numeric_pid(&left.pid).cmp(&numeric_pid(&right.pid)),
        SortColumn::MemoryBytes => left.memory_bytes.cmp(&right.memory_bytes),
        SortColumn::IoBps => left
            .io_read_bps
            .saturating_add(left.io_write_bps)
            .cmp(&right.io_read_bps.saturating_add(right.io_write_bps)),
        SortColumn::NetworkBps => process_network_rate(left).cmp(&process_network_rate(right)),
        SortColumn::Threads => left.threads.cmp(&right.threads),
        SortColumn::Handles => left.handles.cmp(&right.handles),
        SortColumn::StartTimeMs => left.start_time_ms.cmp(&right.start_time_ms),
        SortColumn::Attention => process_attention_score(left)
            .partial_cmp(&process_attention_score(right))
            .unwrap_or(Ordering::Equal),
        SortColumn::CpuPct => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let directed = match query.sort_direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    directed.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
}

fn numeric_pid(pid: &str) -> u64 {
    pid.parse().unwrap_or(u64::MAX)
}

fn compare_process_group(
    left: &ProcessAppGroup,
    right: &ProcessAppGroup,
    query: &RuntimeQuery,
) -> Ordering {
    let ordering = match query.sort_column {
        SortColumn::Name => left.label.to_lowercase().cmp(&right.label.to_lowercase()),
        SortColumn::Pid => {
            numeric_pid(&left.representative.pid).cmp(&numeric_pid(&right.representative.pid))
        }
        SortColumn::MemoryBytes => left.memory_bytes.cmp(&right.memory_bytes),
        SortColumn::IoBps => left.io_bps.cmp(&right.io_bps),
        SortColumn::NetworkBps => left.network_bps.cmp(&right.network_bps),
        SortColumn::Threads => left.threads.cmp(&right.threads),
        SortColumn::Handles => left
            .representative
            .handles
            .cmp(&right.representative.handles),
        SortColumn::StartTimeMs => left
            .representative
            .start_time_ms
            .cmp(&right.representative.start_time_ms),
        SortColumn::Attention => group_attention_score(left)
            .partial_cmp(&group_attention_score(right))
            .unwrap_or(Ordering::Equal),
        SortColumn::CpuPct => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let directed = match query.sort_direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    directed.then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
}

fn matches_focus_mode(process: &ProcessSample, focus_mode: ProcessFocusMode) -> bool {
    match focus_mode {
        ProcessFocusMode::All => true,
        ProcessFocusMode::Attention => needs_attention(process),
        ProcessFocusMode::Io => process_io_rate(process) > 0,
    }
}

fn needs_attention(process: &ProcessSample) -> bool {
    process.cpu_percent >= ATTENTION_CPU_PERCENT
        || process.memory_bytes >= ATTENTION_MEMORY_BYTES
        || process_io_rate(process) >= ATTENTION_IO_BPS
        || process_network_rate(process) >= ATTENTION_NETWORK_BPS
        || process.access_state != AccessState::Full
}

fn process_io_rate(process: &ProcessSample) -> u64 {
    process.io_read_bps.saturating_add(process.io_write_bps)
}

fn process_network_rate(process: &ProcessSample) -> u64 {
    process
        .network_received_bps
        .unwrap_or_default()
        .saturating_add(process.network_transmitted_bps.unwrap_or_default())
}

fn summarize_process_contributors(processes: &[ProcessSample]) -> ProcessContributorSummary {
    let cpu = top_process_contributor(processes, |process| process.cpu_percent, cpu_quality);
    let memory = top_process_contributor(processes, |process| process.memory_bytes, memory_quality);
    let io = top_process_contributor(processes, process_io_rate, io_quality);
    let network = top_process_contributor(processes, process_network_rate, network_quality);

    ProcessContributorSummary {
        cpu: cpu.name,
        cpu_quality: cpu.quality,
        cpu_name_ambiguous: cpu.ambiguous,
        memory: memory.name,
        memory_quality: memory.quality,
        memory_name_ambiguous: memory.ambiguous,
        io: io.name,
        io_quality: io.quality,
        io_name_ambiguous: io.ambiguous,
        network: network.name,
        network_quality: network.quality,
        network_name_ambiguous: network.ambiguous,
    }
}

struct ProcessContributor {
    name: Option<String>,
    quality: Option<MetricQualityInfo>,
    ambiguous: bool,
}

type ProcessQualityAccessor = fn(&ProcessSample) -> Option<&MetricQualityInfo>;

fn top_process_contributor<T>(
    processes: &[ProcessSample],
    metric: impl Fn(&ProcessSample) -> T,
    quality: ProcessQualityAccessor,
) -> ProcessContributor
where
    T: Copy + Default + PartialOrd,
{
    let coverage_quality = process_contributor_coverage_quality(processes, quality);
    let coverage_is_publishable = processes.iter().all(|process| {
        quality(process).is_some_and(|quality| contributor_quality_is_publishable(Some(quality)))
    });
    let winner = processes
        .iter()
        .filter(|process| contributor_quality_is_publishable(quality(process)))
        .max_by(|left, right| {
            metric(left)
                .partial_cmp(&metric(right))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(process) = winner.filter(|process| metric(process) > T::default()) {
        if !coverage_is_publishable {
            return ProcessContributor {
                name: None,
                quality: coverage_quality,
                ambiguous: false,
            };
        }
        return ProcessContributor {
            name: Some(process.name.clone()),
            quality: coverage_quality,
            ambiguous: processes
                .iter()
                .filter(|candidate| candidate.name == process.name)
                .count()
                > 1,
        };
    }

    ProcessContributor {
        name: None,
        quality: coverage_quality,
        ambiguous: false,
    }
}

fn process_contributor_coverage_quality(
    processes: &[ProcessSample],
    quality: ProcessQualityAccessor,
) -> Option<MetricQualityInfo> {
    (!processes.iter().any(|process| quality(process).is_none()))
        .then(|| {
            processes
                .iter()
                .filter_map(quality)
                .max_by_key(|quality| contributor_quality_rank(quality.quality))
                .cloned()
        })
        .flatten()
}

fn contributor_quality_is_publishable(quality: Option<&MetricQualityInfo>) -> bool {
    !quality.is_some_and(|quality| {
        matches!(
            quality.quality,
            MetricQuality::Unavailable | MetricQuality::Held
        )
    })
}

fn contributor_quality_rank(quality: MetricQuality) -> u8 {
    match quality {
        MetricQuality::Native => 1,
        MetricQuality::Estimated => 2,
        MetricQuality::Partial => 3,
        MetricQuality::Held => 4,
        MetricQuality::Unavailable => 5,
    }
}

fn cpu_quality(process: &ProcessSample) -> Option<&MetricQualityInfo> {
    process.quality.as_ref()?.cpu.as_ref()
}

fn memory_quality(process: &ProcessSample) -> Option<&MetricQualityInfo> {
    process.quality.as_ref()?.memory.as_ref()
}

fn io_quality(process: &ProcessSample) -> Option<&MetricQualityInfo> {
    process.quality.as_ref()?.io.as_ref()
}

fn network_quality(process: &ProcessSample) -> Option<&MetricQualityInfo> {
    process.quality.as_ref()?.network.as_ref()
}

fn process_identity(process: &ProcessSample) -> ProcessIdentity {
    let haystack = format!("{} {}", process.name, process.exe).to_lowercase();
    let name = process.name.to_lowercase();
    let is_child = name.starts_with("--")
        || haystack.contains("--type=")
        || haystack.contains("renderer")
        || haystack.contains("gpu-process")
        || haystack.contains("utility");

    if haystack.contains("batcave") {
        return ProcessIdentity {
            icon_kind: "batcave",
            category: "BatCave",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["chrome", "msedge", "firefox", "brave", "browser"],
    ) {
        return ProcessIdentity {
            icon_kind: "browser",
            category: "Browsers",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["code.exe", "visual studio code", "\\code\\", "/code/"],
    ) {
        return ProcessIdentity {
            icon_kind: "code",
            category: "Developer tools",
            is_child,
        };
    }

    if matches_any(&haystack, &["node", "npm", "deno", "bun.exe"]) {
        return ProcessIdentity {
            icon_kind: "node",
            category: "Runtimes",
            is_child,
        };
    }

    if haystack.contains("docker") {
        return ProcessIdentity {
            icon_kind: "container",
            category: "Containers",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["postgres", "mysql", "redis", "sqlserver", "mariadb"],
    ) {
        return ProcessIdentity {
            icon_kind: "database",
            category: "Databases",
            is_child,
        };
    }

    if matches_any(&haystack, &["slack", "teams", "discord", "zoom"]) {
        return ProcessIdentity {
            icon_kind: "chat",
            category: "Communication",
            is_child,
        };
    }

    if matches_any(&haystack, &["spotify", "vlc", "media player"]) {
        return ProcessIdentity {
            icon_kind: "media",
            category: "Media",
            is_child,
        };
    }

    if matches_any(&haystack, &["dropbox", "onedrive", "googledrive"]) {
        return ProcessIdentity {
            icon_kind: "sync",
            category: "Sync",
            is_child,
        };
    }

    if matches_any(&haystack, &["nvidia", "amd", "radeon", "intel graphics"]) {
        return ProcessIdentity {
            icon_kind: "gpu",
            category: "GPU",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &[
            "applicationframehost",
            "conhost",
            "ctfmon",
            "dwm",
            "explorer.exe",
            "phoneexperiencehost",
            "searchindexer",
            "securityhealthservice",
            "shellexperiencehost",
            "sihost",
            "startmenuexperiencehost",
            "svchost",
            "textinputhost",
            "widgetservice",
            "windows",
        ],
    ) {
        return ProcessIdentity {
            icon_kind: "windows",
            category: "Windows",
            is_child,
        };
    }

    ProcessIdentity {
        icon_kind: "process",
        category: "Processes",
        is_child,
    }
}

fn process_app_key(process: &ProcessSample) -> String {
    let executable_name = normalized_process_name(executable_file_name(&process.exe));
    let process_name = normalized_process_name(&process.name);
    let key = if !executable_name.trim().is_empty() {
        executable_name
    } else if !process_name.trim().is_empty() {
        process_name
    } else {
        format!("pid:{}", process.pid)
    };

    key.to_lowercase()
}

fn process_app_label(process: &ProcessSample) -> String {
    normalized_process_name(&process.name)
}

fn executable_file_name(path: &str) -> &str {
    let trimmed = path.trim();
    trimmed
        .rsplit(['\\', '/'])
        .next()
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or(trimmed)
}

fn normalized_process_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if !lower.ends_with(".exe") {
        return name.to_string();
    }

    let extension_start = name.len().saturating_sub(4);
    let stem = &name[..extension_start];
    if let Some((base, suffix)) = stem.rsplit_once('-') {
        if !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()) {
            return format!("{base}.exe");
        }
    }

    name.to_string()
}

fn matches_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn attention_label(
    cpu_percent: f64,
    memory_bytes: u64,
    io_bps: u64,
    network_bps: u64,
    access_limited: bool,
) -> String {
    if cpu_percent >= ATTENTION_CPU_PERCENT {
        return "CPU activity".to_string();
    }

    if memory_bytes >= ATTENTION_MEMORY_BYTES {
        return "memory activity".to_string();
    }

    if io_bps >= ATTENTION_IO_BPS {
        return "I/O activity".to_string();
    }

    if network_bps >= ATTENTION_NETWORK_BPS {
        return "network activity".to_string();
    }

    if access_limited {
        return "access limited".to_string();
    }

    "steady".to_string()
}

fn normalize_settings(settings: RuntimeSettings) -> RuntimeSettings {
    RuntimeSettings {
        query: normalize_query(settings.query),
        admin_mode_requested: false,
        admin_mode_enabled: false,
        metric_window_seconds: settings.metric_window_seconds.clamp(15, 600),
        sample_interval_ms: settings.sample_interval_ms.clamp(500, 5_000),
        paused: settings.paused,
    }
}

fn process_attention_score(process: &ProcessSample) -> f64 {
    let mut score = process.cpu_percent * 3.0;
    score += (process.memory_bytes as f64 / (128.0 * 1024.0 * 1024.0)).min(20.0);
    let io_bps = process_io_rate(process);
    score += (io_bps as f64 / (512.0 * 1024.0)).min(20.0);
    let network_bps = process_network_rate(process);
    score += (network_bps as f64 / (1024.0 * 1024.0)).min(20.0);
    if process.access_state != crate::contracts::AccessState::Full {
        score += 12.0;
    }

    score
}

fn group_attention_score(group: &ProcessAppGroup) -> f64 {
    let mut score = group.cpu_percent * 3.0;
    score += (group.memory_bytes as f64 / (128.0 * 1024.0 * 1024.0)).min(20.0);
    score += (group.io_bps as f64 / (512.0 * 1024.0)).min(20.0);
    score += (group.network_bps as f64 / (1024.0 * 1024.0)).min(20.0);
    if group
        .processes
        .iter()
        .any(|process| process.access_state != crate::contracts::AccessState::Full)
    {
        score += 12.0;
    }

    score
}

fn normalize_query(query: RuntimeQuery) -> RuntimeQuery {
    RuntimeQuery {
        filter_text: query.filter_text.trim().chars().take(256).collect(),
        focus_mode: query.focus_mode,
        sort_column: query.sort_column,
        sort_direction: query.sort_direction,
        limit: query.limit.clamp(25, 20_000),
    }
}

fn push_warning(
    warnings: &mut VecDeque<RuntimeWarning>,
    publication_seq: u64,
    category: &str,
    message: String,
) {
    warnings.push_back(RuntimeWarning {
        key: warning_key(category, &message),
        publication_seq,
        occurred_at_ms: now_ms(),
        category: category.to_string(),
        message,
    });
    while warnings.len() > MAX_WARNINGS {
        warnings.pop_front();
    }
}

fn warning_key(category: &str, message: &str) -> String {
    if category == "admin_mode" {
        return "admin_mode".to_string();
    }

    let code = message
        .split([':', ';', ' '])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .trim_end_matches("_failed");
    format!("{category}.{code}")
}

fn empty_system() -> SystemMetricsSnapshot {
    SystemMetricsSnapshot {
        cpu_percent: 0.0,
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: Vec::new(),
        memory_used_bytes: 0,
        memory_total_bytes: 0,
        memory_available_bytes: None,
        swap_used_bytes: None,
        swap_total_bytes: None,
        process_count: 0,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: 0,
        network_transmitted_total_bytes: 0,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        memory_accounting: None,
        quality: None,
    }
}

fn add_process_memory_accounting(system: &mut SystemMetricsSnapshot, processes: &[ProcessSample]) {
    let process_working_set_bytes = processes
        .iter()
        .filter(|process| process_memory_is_reported(process))
        .fold(0_u64, |total, process| {
            total.saturating_add(process.memory_bytes)
        });
    let process_private_bytes = processes
        .iter()
        .filter(|process| process_memory_is_reported(process))
        .fold(0_u64, |total, process| {
            total.saturating_add(process.private_bytes)
        });
    let denied_process_count = processes
        .iter()
        .filter(|process| process.access_state == crate::contracts::AccessState::Denied)
        .count();
    let partial_process_count = processes
        .iter()
        .filter(|process| process.access_state == crate::contracts::AccessState::Partial)
        .count();

    let accounting = system
        .memory_accounting
        .get_or_insert_with(SystemMemoryAccounting::default);
    accounting.process_working_set_bytes = process_working_set_bytes;
    accounting.process_private_bytes = process_private_bytes;
    accounting.denied_process_count = denied_process_count;
    accounting.partial_process_count = partial_process_count;
}

fn process_memory_is_reported(process: &ProcessSample) -> bool {
    process
        .quality
        .as_ref()
        .and_then(|quality| quality.memory.as_ref())
        .map(|memory| memory.quality != MetricQuality::Unavailable)
        .unwrap_or(true)
}

fn current_app_metrics(processes: &[ProcessSample]) -> CurrentAppMetrics {
    let current_pid = std::process::id().to_string();
    processes
        .iter()
        .find(|process| process.pid == current_pid)
        .map(|process| CurrentAppMetrics {
            cpu_percent: process.cpu_percent,
            rss_bytes: process.memory_bytes,
        })
        .unwrap_or_default()
}

fn byte_rate(current: u64, previous: u64, elapsed_seconds: f64) -> u64 {
    if current < previous {
        return 0;
    }

    ((current - previous) as f64 / elapsed_seconds.max(0.001)).round() as u64
}

fn read_json<T: DeserializeOwned>(path: &PathBuf) -> Result<T, Option<String>> {
    if !path.exists() {
        return Err(None);
    }

    let payload = fs::read_to_string(path).map_err(|error| {
        Some(format!(
            "persistence_load_failed path={} error={}: {}",
            path.display(),
            std::any::type_name::<T>(),
            error
        ))
    })?;
    serde_json::from_str(&payload).map_err(|error| {
        Some(format!(
            "persistence_parse_failed path={} error={}",
            path.display(),
            error
        ))
    })
}

pub(crate) fn default_base_dir() -> PathBuf {
    platform_data_dir()
        .unwrap_or_else(env::temp_dir)
        .join("BatCaveMonitor")
}

fn platform_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("APPDATA"))
            .map(PathBuf::from)
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    }

    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{
        AccessState, MetricQualityInfo, MetricSource, ProcessMetricQuality, RuntimeInstallKind,
        RuntimePlatform, RuntimeProcessElevation,
    };

    #[test]
    fn shape_rows_filters_sorts_and_limits_processes() {
        let rows = shape_rows(
            &[
                sample("10", "Alpha", 1.0),
                sample("20", "Code", 3.0),
                sample("30", "Codex", 2.0),
            ],
            &RuntimeQuery {
                filter_text: "code".to_string(),
                focus_mode: ProcessFocusMode::All,
                sort_column: SortColumn::Name,
                sort_direction: SortDirection::Asc,
                limit: 1,
            },
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Code");
    }

    #[test]
    fn attention_sort_uses_attention_score_not_cpu_only() {
        let mut limited = sample("10", "Limited", 0.2);
        limited.access_state = AccessState::Partial;
        let rows = shape_rows(
            &[
                sample("20", "Quiet", 0.1),
                limited,
                sample("30", "Busy", 44.4),
            ],
            &RuntimeQuery {
                sort_column: SortColumn::Attention,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].name, "Busy");
        assert_eq!(rows[1].name, "Limited");
    }

    #[test]
    fn shape_rows_applies_focus_mode_in_runtime_query() {
        let mut idle_io = sample("20", "IdleIo", 0.0);
        idle_io.io_read_bps = 2048;
        let mut other_only = sample("40", "OtherOnly", 0.0);
        other_only.other_io_bps = Some(16 * 1024 * 1024);
        let active = sample("30", "Active", 10.0);
        let rows = shape_rows(
            &[
                sample("10", "Idle", 0.0),
                active.clone(),
                idle_io.clone(),
                other_only.clone(),
            ],
            &RuntimeQuery {
                focus_mode: ProcessFocusMode::Attention,
                sort_column: SortColumn::Name,
                sort_direction: SortDirection::Asc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Active");

        let io_rows = shape_rows(
            &[sample("10", "Idle", 0.0), active, idle_io, other_only],
            &RuntimeQuery {
                focus_mode: ProcessFocusMode::Io,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(io_rows.len(), 1);
        assert_eq!(io_rows[0].name, "IdleIo");
    }

    #[test]
    fn snapshot_contributors_ignore_search_and_focus() {
        let cpu = sample("10", "CpuWinner", 80.0);
        let mut memory = sample("20", "MemoryWinner", 0.0);
        memory.memory_bytes = 2 * 1024 * 1024 * 1024;
        let mut io = sample("30", "IoWinner", 0.0);
        io.io_read_bps = 8 * 1024 * 1024;
        let mut network = sample("40", "NetworkWinner", 0.0);
        network.network_received_bps = Some(16 * 1024 * 1024);
        let mut other_only = sample("50", "OtherOnly", 0.0);
        other_only.other_io_bps = Some(64 * 1024 * 1024);
        let all_processes = vec![cpu, memory, io, network, other_only];
        let settings = RuntimeSettings {
            query: RuntimeQuery {
                filter_text: "iowinner".to_string(),
                focus_mode: ProcessFocusMode::Io,
                ..RuntimeQuery::default()
            },
            ..RuntimeSettings::default()
        };
        let visible_processes = shape_rows(&all_processes, &settings.query);
        let provenance = RuntimeProvenance::detect(Path::new(""));

        let snapshot = build_snapshot(
            1,
            1,
            1,
            Some(1),
            provenance.environment(),
            &settings,
            &provenance.admin_mode_status(),
            RuntimeHealth::default(),
            empty_system(),
            &all_processes,
            visible_processes,
            Vec::new(),
        );

        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].name, "IoWinner");
        assert_eq!(
            snapshot.process_contributors.cpu.as_deref(),
            Some("CpuWinner")
        );
        assert_eq!(
            snapshot.process_contributors.memory.as_deref(),
            Some("MemoryWinner")
        );
        assert_eq!(
            snapshot.process_contributors.io.as_deref(),
            Some("IoWinner")
        );
        assert_eq!(
            snapshot.process_contributors.network.as_deref(),
            Some("NetworkWinner")
        );
    }

    #[test]
    fn process_contributor_quality_excludes_unpublishable_winners_and_limits_zero_coverage() {
        let quality = |value| {
            Some(crate::contracts::ProcessMetricQuality {
                cpu: Some(MetricQualityInfo::new(value, MetricSource::DirectApi)),
                ..crate::contracts::ProcessMetricQuality::default()
            })
        };
        let mut unavailable_high = sample("10", "UnavailableHigh", 90.0);
        unavailable_high.quality = quality(MetricQuality::Unavailable);
        let mut estimated_lower = sample("20", "EstimatedLower", 20.0);
        estimated_lower.quality = quality(MetricQuality::Estimated);

        let selected = summarize_process_contributors(&[unavailable_high.clone(), estimated_lower]);

        assert_eq!(selected.cpu, None);
        assert_eq!(
            selected.cpu_quality.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );

        let mut native_positive = sample("21", "NativePositive", 25.0);
        native_positive.quality = quality(MetricQuality::Native);
        let mut held_placeholder = sample("22", "HeldPlaceholder", 0.0);
        held_placeholder.quality = quality(MetricQuality::Held);
        let blocked_positive = summarize_process_contributors(&[native_positive, held_placeholder]);
        assert_eq!(blocked_positive.cpu, None);
        assert_eq!(
            blocked_positive
                .cpu_quality
                .as_ref()
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );

        let unavailable = summarize_process_contributors(&[unavailable_high.clone()]);
        assert_eq!(unavailable.cpu, None);
        assert_eq!(
            unavailable
                .cpu_quality
                .as_ref()
                .map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );

        let mut native_zero = sample("30", "NativeZero", 0.0);
        native_zero.quality = quality(MetricQuality::Native);
        let quiet = summarize_process_contributors(&[native_zero, unavailable_high]);
        assert_eq!(quiet.cpu, None);
        assert_eq!(
            quiet.cpu_quality.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );

        let mut held_zero = sample("31", "HeldZero", 0.0);
        held_zero.quality = quality(MetricQuality::Held);
        let mut native_zero_with_held = sample("32", "NativeZeroWithHeld", 0.0);
        native_zero_with_held.quality = quality(MetricQuality::Native);
        let pending = summarize_process_contributors(&[native_zero_with_held, held_zero]);
        assert_eq!(pending.cpu, None);
        assert_eq!(
            pending.cpu_quality.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );

        let mut partial_zero = sample("40", "PartialZero", 0.0);
        partial_zero.quality = quality(MetricQuality::Partial);
        let mut native_zero_again = sample("41", "NativeZeroAgain", 0.0);
        native_zero_again.quality = quality(MetricQuality::Native);
        let limited = summarize_process_contributors(&[native_zero_again, partial_zero]);
        assert_eq!(limited.cpu, None);
        assert_eq!(
            limited.cpu_quality.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Partial)
        );

        let mut known_zero = sample("42", "KnownZero", 0.0);
        known_zero.quality = quality(MetricQuality::Native);
        let mut unknown_zero = sample("50", "UnknownZero", 0.0);
        unknown_zero.quality = None;
        let unknown = summarize_process_contributors(&[known_zero, unknown_zero]);
        assert_eq!(unknown.cpu, None);
        assert_eq!(unknown.cpu_quality, None);
    }

    #[test]
    fn contributor_name_ambiguity_uses_full_sample_when_query_keeps_one_row() {
        let first = sample("10", "worker", 80.0);
        let second = sample("20", "worker", 40.0);
        let all_processes = vec![first, second];
        let settings = RuntimeSettings {
            query: RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                limit: 1,
                ..RuntimeQuery::default()
            },
            ..RuntimeSettings::default()
        };
        let visible_processes = shape_rows(&all_processes, &settings.query);

        let snapshot = build_snapshot(
            1,
            1,
            1,
            Some(1),
            Path::new(""),
            &settings,
            &initial_admin_mode_status(),
            RuntimeHealth::default(),
            empty_system(),
            &all_processes,
            visible_processes,
            Vec::new(),
        );

        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].name, "worker");
        assert_eq!(snapshot.process_contributors.cpu.as_deref(), Some("worker"));
        assert!(snapshot.process_contributors.cpu_name_ambiguous);
    }

    #[test]
    fn process_view_groups_suffixed_app_processes() {
        let mut first = sample("10", "SearchIndexer-211.exe", 12.0);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
        first.io_read_bps = 256;
        first.other_io_bps = Some(8 * 1024 * 1024);
        first.threads = 3;
        let mut second = sample("20", "SearchIndexer-223.exe", 8.0);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
        second.io_write_bps = 512;
        second.threads = 5;

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_label.as_deref(), Some("SearchIndexer.exe"));
        assert_eq!(rows[0].group_category.as_deref(), Some("Windows"));
        assert_eq!(rows[0].group_count, 2);
        assert_eq!(rows[0].cpu_percent, 20.0);
        assert_eq!(rows[0].io_bps, 768);
        assert_eq!(rows[0].threads, 8);
        assert!(rows[1].is_grouped);
        assert_eq!(rows[1].group_key, rows[0].group_key);
    }

    #[test]
    fn process_view_uses_process_name_key_when_exe_is_missing() {
        let mut first = sample("10", "Memory Compression", 12.0);
        first.exe = String::new();
        let mut second = sample("20", "Memory Compression", 8.0);
        second.exe = " ".to_string();

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_key.as_deref(), Some("memory compression"));
        assert!(rows[1].is_grouped);
        assert_eq!(rows[1].group_key, rows[0].group_key);
        assert_eq!(rows[2].group_key, rows[0].group_key);
    }

    #[test]
    fn process_view_keeps_singletons_as_process_rows() {
        let rows = shape_process_view(&[sample("10", "Code.exe", 12.0)], &RuntimeQuery::default());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Process);
        assert!(!rows[0].is_grouped);
        assert_eq!(rows[0].group_label.as_deref(), Some("Code.exe"));
    }

    #[test]
    fn shape_rows_sorts_by_network_rate() {
        let quiet = sample("10", "Quiet", 1.0);
        let mut network_busy = sample("20", "NetworkBusy", 1.0);
        network_busy.network_received_bps = Some(8_000);
        network_busy.network_transmitted_bps = Some(2_000);

        let rows = shape_rows(
            &[quiet, network_busy],
            &RuntimeQuery {
                sort_column: SortColumn::NetworkBps,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].name, "NetworkBusy");
    }

    #[test]
    fn process_view_sorts_group_rows_by_visible_aggregate() {
        let singleton = sample("10", "Code.exe", 70.0);
        let mut first = sample("20", "SearchIndexer-001.exe", 40.0);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-001.exe".to_string();
        let mut second = sample("30", "SearchIndexer-002.exe", 35.0);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-002.exe".to_string();

        let rows = shape_process_view(
            &[singleton, first, second],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_label.as_deref(), Some("SearchIndexer.exe"));
        assert_eq!(rows[0].cpu_percent, 75.0);
    }

    #[test]
    fn process_view_sorts_cpu_values_numerically() {
        let larger = sample("10", "OneTwentyFour.exe", 124.0);
        let smaller = sample("20", "TwentyFive.exe", 25.0);

        let descending = shape_process_view(
            &[smaller.clone(), larger.clone()],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );
        let ascending = shape_process_view(
            &[larger, smaller],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Asc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(descending[0].cpu_percent, 124.0);
        assert_eq!(descending[1].cpu_percent, 25.0);
        assert_eq!(ascending[0].cpu_percent, 25.0);
        assert_eq!(ascending[1].cpu_percent, 124.0);
    }

    #[test]
    fn process_rates_use_pid_and_start_time_identity() {
        let mut previous = sample("10", "Stable", 1.0);
        previous.start_time_ms = 100;
        previous.io_read_total_bytes = 100;
        previous.io_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(10);
        previous.quality.as_mut().expect("process quality").other_io = Some(
            MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi),
        );

        let mut current = previous.clone();
        current.io_read_total_bytes = 600;
        current.io_write_total_bytes = 250;
        current.other_io_total_bytes = Some(110);

        let updated = add_process_rates(vec![current], &[previous], 1.0, true);

        assert_eq!(updated[0].io_read_bps, 500);
        assert_eq!(updated[0].io_write_bps, 200);
        assert_eq!(updated[0].other_io_bps, Some(100));
        assert_eq!(
            updated[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );

        let mut restarted_previous = sample("10", "Stable", 1.0);
        restarted_previous.start_time_ms = 1;
        let restarted = add_process_rates(
            vec![sample("10", "Stable", 1.0)],
            &[restarted_previous],
            1.0,
            true,
        );

        assert_eq!(restarted[0].io_read_bps, 0);
        assert_eq!(restarted[0].other_io_bps, None);
        assert_eq!(
            restarted[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
    }

    #[test]
    fn process_rate_quality_requires_a_fresh_live_baseline_across_collectors() {
        let collector_cases = [
            ("windows", MetricSource::DirectApi, MetricQuality::Native),
            ("macos", MetricSource::DirectApi, MetricQuality::Native),
            ("linux", MetricSource::Procfs, MetricQuality::Native),
            ("sysinfo", MetricSource::Sysinfo, MetricQuality::Estimated),
        ];

        for (label, source, collector_quality) in collector_cases {
            let mut first = sample("10", label, 1.0);
            first.io_read_total_bytes = 100;
            first.io_write_total_bytes = 50;
            first.other_io_total_bytes = Some(25);
            let quality = first.quality.as_mut().expect("process quality");
            quality.io = Some(MetricQualityInfo::new(collector_quality, source));
            quality.other_io = Some(MetricQualityInfo::new(collector_quality, source));

            let held = add_process_rates(vec![first.clone()], &[], 1.0, false);
            assert_eq!(held[0].io_read_bps, 0, "{label} first read rate");
            assert_eq!(held[0].other_io_bps, None, "{label} first Other I/O rate");
            assert_eq!(
                held[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| (quality.quality, quality.source)),
                Some((MetricQuality::Held, Some(source))),
                "{label} first rate quality"
            );
            assert_eq!(
                summarize_process_contributors(&held)
                    .io_quality
                    .as_ref()
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} contributor coverage"
            );

            let mut second = first.clone();
            second.io_read_total_bytes = 300;
            second.io_write_total_bytes = 150;
            second.other_io_total_bytes = Some(125);
            let current = add_process_rates(vec![second], &held, 1.0, true);
            assert_eq!(current[0].io_read_bps, 200, "{label} second read rate");
            assert_eq!(current[0].io_write_bps, 100, "{label} second write rate");
            assert_eq!(
                current[0].other_io_bps,
                Some(100),
                "{label} second Other rate"
            );
            assert_eq!(
                current[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(collector_quality),
                "{label} restored collector quality"
            );

            let mut previous_without_other = first.clone();
            previous_without_other.other_io_total_bytes = None;
            let mut current_without_other_baseline = first.clone();
            current_without_other_baseline.other_io_total_bytes = Some(225);
            current_without_other_baseline.other_io_bps = Some(999);
            let current_without_other_baseline = add_process_rates(
                vec![current_without_other_baseline],
                &[previous_without_other],
                1.0,
                true,
            );
            assert_eq!(
                current_without_other_baseline[0].other_io_bps, None,
                "{label} missing Other I/O baseline"
            );
            assert_eq!(
                current_without_other_baseline[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.other_io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} missing Other I/O baseline quality"
            );
            assert_eq!(
                current_without_other_baseline[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(collector_quality),
                "{label} read/write I/O remains publishable"
            );

            let mut previous_without_io = first.clone();
            previous_without_io
                .quality
                .as_mut()
                .expect("process quality")
                .io = Some(MetricQualityInfo::new(MetricQuality::Unavailable, source));
            let mut current_with_independent_other = first.clone();
            current_with_independent_other.io_read_total_bytes = 300;
            current_with_independent_other.io_write_total_bytes = 150;
            current_with_independent_other.other_io_total_bytes = Some(125);
            let current_with_independent_other = add_process_rates(
                vec![current_with_independent_other],
                &[previous_without_io],
                1.0,
                true,
            );
            assert_eq!(
                current_with_independent_other[0].io_read_bps, 0,
                "{label} invalid read/write baseline"
            );
            assert_eq!(
                current_with_independent_other[0].other_io_bps,
                Some(100),
                "{label} independent Other I/O baseline"
            );
            assert_eq!(
                current_with_independent_other[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} read/write waits independently"
            );
            assert_eq!(
                current_with_independent_other[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.other_io.as_ref())
                    .map(|quality| quality.quality),
                Some(collector_quality),
                "{label} Other I/O keeps collector quality"
            );

            let after_gap = add_process_rates(vec![first.clone()], &[first], 1.0, false);
            assert_eq!(
                after_gap[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} post-gap rate quality"
            );
        }
    }

    #[test]
    fn process_rate_recovery_requires_a_valid_cumulative_baseline_across_sources() {
        let collector_cases = [
            ("direct_api", MetricSource::DirectApi, MetricQuality::Native),
            ("procfs", MetricSource::Procfs, MetricQuality::Native),
            ("sysinfo", MetricSource::Sysinfo, MetricQuality::Estimated),
        ];

        for (label, source, collector_quality) in collector_cases {
            let mut unavailable = sample("10", label, 1.0);
            unavailable.start_time_ms = 100;
            unavailable.io_read_total_bytes = 0;
            unavailable.io_write_total_bytes = 0;
            unavailable.other_io_total_bytes = Some(0);
            unavailable.io_read_bps = 999;
            unavailable.io_write_bps = 999;
            unavailable.other_io_bps = Some(999);
            let quality = unavailable.quality.as_mut().expect("process quality");
            quality.io = Some(MetricQualityInfo::new(MetricQuality::Unavailable, source));
            quality.other_io = Some(MetricQualityInfo::new(MetricQuality::Unavailable, source));

            let unavailable = add_process_rates(vec![unavailable], &[], 1.0, false);
            assert_eq!(unavailable[0].io_read_bps, 0, "{label} unavailable read");
            assert_eq!(
                unavailable[0].other_io_bps, None,
                "{label} unavailable other"
            );
            assert_eq!(
                unavailable[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Unavailable),
                "{label} unavailable read/write quality"
            );
            assert_eq!(
                unavailable[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.other_io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Unavailable),
                "{label} unavailable Other I/O quality"
            );

            let mut first_valid = sample("10", label, 1.0);
            first_valid.start_time_ms = 100;
            first_valid.io_read_total_bytes = 100;
            first_valid.io_write_total_bytes = 50;
            first_valid.other_io_total_bytes = Some(25);
            let quality = first_valid.quality.as_mut().expect("process quality");
            quality.io = Some(MetricQualityInfo::new(collector_quality, source));
            quality.other_io = Some(MetricQualityInfo::new(collector_quality, source));

            let valid_baseline = add_process_rates(vec![first_valid], &unavailable, 1.0, true);
            assert_eq!(
                valid_baseline[0].io_read_bps, 0,
                "{label} recovery baseline"
            );
            assert_eq!(
                valid_baseline[0].other_io_bps, None,
                "{label} Other baseline"
            );
            assert_eq!(
                valid_baseline[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} recovery pending"
            );
            assert_eq!(
                valid_baseline[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.other_io.as_ref())
                    .map(|quality| quality.quality),
                Some(MetricQuality::Held),
                "{label} Other recovery pending"
            );

            let mut second_valid = sample("10", label, 1.0);
            second_valid.start_time_ms = 100;
            second_valid.io_read_total_bytes = 300;
            second_valid.io_write_total_bytes = 150;
            second_valid.other_io_total_bytes = Some(125);
            let quality = second_valid.quality.as_mut().expect("process quality");
            quality.io = Some(MetricQualityInfo::new(collector_quality, source));
            quality.other_io = Some(MetricQualityInfo::new(collector_quality, source));

            let recovered = add_process_rates(vec![second_valid], &valid_baseline, 1.0, true);
            assert_eq!(recovered[0].io_read_bps, 200, "{label} recovered read");
            assert_eq!(recovered[0].io_write_bps, 100, "{label} recovered write");
            assert_eq!(
                recovered[0].other_io_bps,
                Some(100),
                "{label} recovered other"
            );
            assert_eq!(
                recovered[0]
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.io.as_ref())
                    .map(|quality| quality.quality),
                Some(collector_quality),
                "{label} recovered collector quality"
            );
        }
    }

    #[test]
    fn process_rate_source_transition_requires_a_new_compatible_baseline() {
        let mut fallback = sample("10", "Source transition", 1.0);
        fallback.io_read_total_bytes = 100;
        fallback.io_write_total_bytes = 50;
        fallback.other_io_total_bytes = Some(25);
        let quality = fallback.quality.as_mut().expect("process quality");
        quality.io = Some(MetricQualityInfo::new(
            MetricQuality::Estimated,
            MetricSource::Sysinfo,
        ));
        quality.other_io = Some(MetricQualityInfo::new(
            MetricQuality::Estimated,
            MetricSource::Sysinfo,
        ));

        let mut native = sample("10", "Source transition", 1.0);
        native.io_read_total_bytes = 500;
        native.io_write_total_bytes = 250;
        native.other_io_total_bytes = Some(125);
        native.quality.as_mut().expect("process quality").other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));

        let native_baseline = add_process_rates(vec![native], &[fallback], 1.0, true);
        assert_eq!(native_baseline[0].io_read_bps, 0);
        assert_eq!(native_baseline[0].io_write_bps, 0);
        assert_eq!(native_baseline[0].other_io_bps, None);
        assert_eq!(
            native_baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Held, Some(MetricSource::DirectApi)))
        );
        assert_eq!(
            native_baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.other_io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Held, Some(MetricSource::DirectApi)))
        );

        let mut next_native = sample("10", "Source transition", 1.0);
        next_native.io_read_total_bytes = 700;
        next_native.io_write_total_bytes = 350;
        next_native.other_io_total_bytes = Some(225);
        next_native
            .quality
            .as_mut()
            .expect("process quality")
            .other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));

        let recovered = add_process_rates(vec![next_native], &native_baseline, 1.0, true);
        assert_eq!(recovered[0].io_read_bps, 200);
        assert_eq!(recovered[0].io_write_bps, 100);
        assert_eq!(recovered[0].other_io_bps, Some(100));
        assert_eq!(
            recovered[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );
    }

    #[test]
    fn missing_prior_cumulative_quality_cannot_form_a_rate_baseline() {
        let mut previous = sample("10", "Missing quality", 1.0);
        previous.io_read_total_bytes = 100;
        previous.io_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(25);
        let quality = previous.quality.as_mut().expect("process quality");
        quality.io = None;
        quality.other_io = None;

        let mut current = sample("10", "Missing quality", 1.0);
        current.io_read_total_bytes = 500;
        current.io_write_total_bytes = 250;
        current.other_io_total_bytes = Some(125);
        current.quality.as_mut().expect("process quality").other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));

        let baseline = add_process_rates(vec![current], &[previous], 1.0, true);

        assert_eq!(baseline[0].io_read_bps, 0);
        assert_eq!(baseline[0].io_write_bps, 0);
        assert_eq!(baseline[0].other_io_bps, None);
        assert_eq!(
            baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
        assert_eq!(
            baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.other_io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
    }

    #[test]
    fn missing_cumulative_source_cannot_form_a_rate_baseline() {
        let source_less = || {
            let mut quality =
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi);
            quality.source = None;
            quality
        };
        let mut previous = sample("10", "Missing source", 1.0);
        previous.io_read_total_bytes = 100;
        previous.io_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(25);
        let quality = previous.quality.as_mut().expect("process quality");
        quality.io = Some(source_less());
        quality.other_io = Some(source_less());

        let mut current = sample("10", "Missing source", 1.0);
        current.io_read_total_bytes = 500;
        current.io_write_total_bytes = 250;
        current.other_io_total_bytes = Some(125);
        let quality = current.quality.as_mut().expect("process quality");
        quality.io = Some(source_less());
        quality.other_io = Some(source_less());

        let baseline = add_process_rates(vec![current], &[previous], 1.0, true);

        assert_eq!(baseline[0].io_read_bps, 0);
        assert_eq!(baseline[0].io_write_bps, 0);
        assert_eq!(baseline[0].other_io_bps, None);
        assert_eq!(
            baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Held, None))
        );
        assert_eq!(
            baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.other_io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Held, None))
        );
    }

    #[test]
    fn cumulative_counter_reset_requires_a_new_rate_baseline() {
        let mut previous = sample("10", "Counter reset", 1.0);
        previous.io_read_total_bytes = 500;
        previous.io_write_total_bytes = 250;
        previous.other_io_total_bytes = Some(125);
        previous.quality.as_mut().expect("process quality").other_io = Some(
            MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi),
        );

        let mut reset = sample("10", "Counter reset", 1.0);
        reset.io_read_total_bytes = 100;
        reset.io_write_total_bytes = 50;
        reset.other_io_total_bytes = Some(25);
        reset.quality.as_mut().expect("process quality").other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));
        let reset_baseline = add_process_rates(vec![reset], &[previous], 1.0, true);

        assert_eq!(reset_baseline[0].io_read_bps, 0);
        assert_eq!(reset_baseline[0].io_write_bps, 0);
        assert_eq!(reset_baseline[0].other_io_bps, None);
        assert_eq!(
            reset_baseline[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );

        let mut recovered = sample("10", "Counter reset", 1.0);
        recovered.io_read_total_bytes = 300;
        recovered.io_write_total_bytes = 150;
        recovered.other_io_total_bytes = Some(125);
        recovered
            .quality
            .as_mut()
            .expect("process quality")
            .other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));
        let recovered = add_process_rates(vec![recovered], &reset_baseline, 1.0, true);

        assert_eq!(recovered[0].io_read_bps, 200);
        assert_eq!(recovered[0].io_write_bps, 100);
        assert_eq!(recovered[0].other_io_bps, Some(100));
    }

    #[test]
    fn zero_start_time_never_forms_a_process_rate_identity() {
        let mut previous = sample("10", "Unknown identity", 1.0);
        previous.start_time_ms = 0;
        previous.io_read_total_bytes = 100;
        previous.io_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(25);

        let mut current = previous.clone();
        current.io_read_total_bytes = 500;
        current.io_write_total_bytes = 250;
        current.other_io_total_bytes = Some(125);
        let first = add_process_rates(vec![current], &[previous], 1.0, true);

        assert_eq!(first[0].io_read_bps, 0);
        assert_eq!(first[0].io_write_bps, 0);
        assert_eq!(first[0].other_io_bps, None);
        assert_eq!(
            first[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );

        let mut reused_pid = sample("10", "Reused PID", 1.0);
        reused_pid.start_time_ms = 0;
        reused_pid.io_read_total_bytes = 900;
        reused_pid.io_write_total_bytes = 450;
        reused_pid.other_io_total_bytes = Some(225);
        let second = add_process_rates(vec![reused_pid], &first, 1.0, true);

        assert_eq!(second[0].io_read_bps, 0);
        assert_eq!(second[0].io_write_bps, 0);
        assert_eq!(second[0].other_io_bps, None);
        assert_eq!(
            second[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
    }

    #[test]
    fn snapshot_reads_are_passive() {
        let state = RuntimeState::new();

        let first = state.snapshot().expect("snapshot read succeeds");
        let second = state.snapshot().expect("snapshot read succeeds");

        assert_eq!(first.sample_seq, second.sample_seq);
        assert_eq!(first.publication_seq, second.publication_seq);
    }

    #[test]
    fn process_memory_accounting_excludes_unavailable_placeholder_memory() {
        let mut system = empty_system();
        system.memory_used_bytes = 1_000;
        system.memory_accounting = Some(SystemMemoryAccounting {
            kernel_paged_pool_bytes: Some(128),
            ..SystemMemoryAccounting::default()
        });
        let mut reported = sample("10", "Reported", 1.0);
        reported.memory_bytes = 400;
        reported.private_bytes = 200;
        let mut blocked = sample("20", "Blocked", 1.0);
        blocked.access_state = AccessState::Denied;
        blocked.memory_bytes = 900;
        blocked.private_bytes = 800;
        blocked.quality = Some(ProcessMetricQuality {
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Unavailable,
                MetricSource::DirectApi,
            )),
            ..ProcessMetricQuality::default()
        });
        let mut partial = sample("30", "Partial", 1.0);
        partial.access_state = AccessState::Partial;
        partial.memory_bytes = 100;
        partial.private_bytes = 50;

        add_process_memory_accounting(&mut system, &[reported, blocked, partial]);

        let accounting = system.memory_accounting.expect("accounting exists");
        assert_eq!(accounting.process_working_set_bytes, 500);
        assert_eq!(accounting.process_private_bytes, 250);
        assert_eq!(accounting.denied_process_count, 1);
        assert_eq!(accounting.partial_process_count, 1);
        assert_eq!(accounting.kernel_paged_pool_bytes, Some(128));
    }

    #[test]
    fn attention_score_counts_network_rates() {
        let quiet = sample("10", "Quiet", 0.1);
        let mut network_busy = sample("20", "NetworkBusy", 0.1);
        network_busy.network_received_bps = Some(8 * 1024 * 1024);
        network_busy.network_transmitted_bps = Some(2 * 1024 * 1024);

        assert!(process_attention_score(&network_busy) > process_attention_score(&quiet));
    }

    #[test]
    fn attention_filter_uses_every_attention_dimension() {
        let quiet = sample("10", "Quiet", 0.1);
        let cpu = sample("20", "Cpu", ATTENTION_CPU_PERCENT);
        let mut memory = quiet.clone();
        memory.pid = "30".to_string();
        memory.memory_bytes = ATTENTION_MEMORY_BYTES;
        let mut io = quiet.clone();
        io.pid = "40".to_string();
        io.io_read_bps = ATTENTION_IO_BPS;
        let mut network = quiet.clone();
        network.pid = "50".to_string();
        network.network_received_bps = Some(ATTENTION_NETWORK_BPS);
        let mut limited = quiet.clone();
        limited.pid = "60".to_string();
        limited.access_state = AccessState::Partial;

        assert!(!needs_attention(&quiet));
        for process in [&cpu, &memory, &io, &network, &limited] {
            assert!(needs_attention(process), "{} needs attention", process.pid);
        }
    }

    #[test]
    fn elevated_rows_inherit_main_network_only_for_exact_identity() {
        let mut main = sample("10", "Main", 0.0);
        main.start_time_ms = 100;
        main.network_received_bps = Some(4_096);
        main.network_transmitted_bps = Some(2_048);
        main.quality = Some(ProcessMetricQuality {
            network: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Etw,
            )),
            ..ProcessMetricQuality::default()
        });
        let mut matching = sample("10", "Elevated", 0.0);
        matching.start_time_ms = 100;
        let mut reused_pid = matching.clone();
        reused_pid.start_time_ms = 200;
        let mut elevated = vec![matching, reused_pid];

        merge_main_network_attribution(&[main], &mut elevated);

        assert_eq!(elevated[0].network_received_bps, Some(4_096));
        assert_eq!(elevated[0].network_transmitted_bps, Some(2_048));
        assert_eq!(elevated[1].network_received_bps, None);
    }

    #[test]
    fn settings_normalization_clears_effective_admin_and_clamps_ranges() {
        let settings = normalize_settings(RuntimeSettings {
            admin_mode_requested: true,
            admin_mode_enabled: true,
            metric_window_seconds: 1,
            sample_interval_ms: 1,
            query: RuntimeQuery {
                filter_text: "  code  ".to_string(),
                focus_mode: ProcessFocusMode::Attention,
                limit: usize::MAX,
                ..RuntimeQuery::default()
            },
            paused: false,
        });

        assert!(!settings.admin_mode_requested);
        assert!(!settings.admin_mode_enabled);
        assert_eq!(settings.metric_window_seconds, 15);
        assert_eq!(settings.query.filter_text, "code");
        assert_eq!(settings.query.focus_mode, ProcessFocusMode::Attention);
        assert_eq!(settings.query.limit, 20_000);
    }

    #[test]
    fn process_icon_allowlist_requires_live_snapshot() {
        let mut store = RuntimeStore::new();
        let trusted = sample("10", "Trusted", 1.0);
        let exe = trusted.exe.clone();
        let all_processes = vec![trusted.clone()];
        store.settings = RuntimeSettings::default();
        store.snapshot = build_snapshot(
            1,
            now_ms(),
            1,
            Some(now_ms()),
            store.provenance.environment(),
            &store.settings,
            &store.admin_mode,
            RuntimeHealth::default(),
            empty_system(),
            &all_processes,
            vec![trusted],
            Vec::new(),
        );

        assert!(!store.has_process_exe(&exe));

        store.live_process_snapshot = true;

        assert!(store.has_process_exe(&exe));
    }

    #[test]
    fn pausing_revokes_process_icon_authority() {
        let base_dir = runtime_test_dir("icon-pause");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.live_process_snapshot = true;

        store.set_paused(true);

        assert!(!store.live_process_snapshot);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_admin_request_remains_disabled() {
        let base_dir = runtime_test_dir("admin-unsupported");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());

        let snapshot = store.set_admin_mode(true);

        assert!(!snapshot.settings.admin_mode_requested);
        assert!(!snapshot.settings.admin_mode_enabled);
        assert!(!snapshot.environment.admin_mode_available);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn standard_parent_and_active_helper_remain_distinct() {
        let base_dir = runtime_test_dir("admin-helper-source");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.provenance = RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Standard);
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = true;
        store.admin_mode = RuntimeAdminModeStatus {
            state: RuntimeAdminModeState::Active,
            source: RuntimePrivilegedSource::ElevatedHelper,
            detail: None,
            last_success_at_ms: Some(7),
        };

        store.publish_snapshot_only(None);

        assert_eq!(
            store.snapshot.environment.process_elevation,
            RuntimeProcessElevation::Standard
        );
        assert_eq!(
            store.snapshot.admin_mode.source,
            RuntimePrivilegedSource::ElevatedHelper
        );
        assert_eq!(
            store.snapshot.admin_mode.state,
            RuntimeAdminModeState::Active
        );
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn elevated_parent_uses_current_process_without_starting_helper() {
        let base_dir = runtime_test_dir("admin-current-process-source");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.provenance = RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Elevated);

        let snapshot = store.set_admin_mode(true);

        assert_eq!(
            snapshot.environment.process_elevation,
            RuntimeProcessElevation::Elevated
        );
        assert_eq!(
            snapshot.admin_mode.source,
            RuntimePrivilegedSource::CurrentProcess
        );
        assert_eq!(snapshot.admin_mode.state, RuntimeAdminModeState::Active);
        assert!(snapshot.settings.admin_mode_enabled);
        assert!(!snapshot.settings.admin_mode_requested);
        assert!(store.elevated_request.is_none());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_development_environment_disables_admin_mode() {
        let base_dir = PathBuf::from("/Users/test/Library/Application Support/BatCaveMonitor");
        let provenance = RuntimeProvenance::detect(&base_dir);
        let environment = provenance.environment();

        assert_eq!(environment.platform, RuntimePlatform::Macos);
        assert_eq!(environment.install_kind, RuntimeInstallKind::Development);
        assert!(!environment.admin_mode_available);
        assert_eq!(
            environment.data_directory.as_deref(),
            Some("/Users/test/Library/Application Support/BatCaveMonitor")
        );
        assert_eq!(
            provenance.admin_mode_status().state,
            RuntimeAdminModeState::Unavailable
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_default_data_directory_is_application_support() {
        let expected_root = env::var_os("HOME")
            .map(PathBuf::from)
            .expect("macOS test has a home directory")
            .join("Library")
            .join("Application Support");
        assert_eq!(default_base_dir(), expected_root.join("BatCaveMonitor"));
    }

    #[test]
    fn admin_failure_clears_requested_and_enabled_state() {
        let base_dir = runtime_test_dir("admin-failure");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = true;

        store.fail_admin_mode("admin_mode_launch_failed_or_cancelled".to_string());

        assert!(!store.settings.admin_mode_requested);
        assert!(!store.settings.admin_mode_enabled);
        assert_eq!(store.admin_mode.state, RuntimeAdminModeState::Failed);
        assert_eq!(
            store.admin_mode.source,
            RuntimePrivilegedSource::ElevatedHelper
        );
        assert_eq!(
            store.admin_mode.detail.as_deref(),
            Some("admin_mode_launch_failed_or_cancelled")
        );
        assert!(store
            .warnings
            .back()
            .is_some_and(|warning| warning.message == "admin_mode_launch_failed_or_cancelled"));
        let persisted = read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE))
            .expect("settings read succeeds");
        assert!(!persisted.admin_mode_requested);
        assert!(!persisted.admin_mode_enabled);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn failed_background_elevation_request_becomes_retryable() {
        let base_dir = runtime_test_dir("admin-background-failure");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        let (sender, receiver) = mpsc::channel();
        store.settings.admin_mode_requested = true;
        store.admin_mode.state = RuntimeAdminModeState::Requesting;
        store.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
        store.settings.paused = true;
        store.elevated_request = Some(receiver);
        sender
            .send(Err("admin_mode_launch_failed_or_cancelled".to_string()))
            .expect("test result sends");

        assert!(store.resolve_elevated_request());
        store.publish_snapshot_only(None);

        assert_eq!(store.admin_mode.state, RuntimeAdminModeState::Failed);
        assert_eq!(
            store.admin_mode.source,
            RuntimePrivilegedSource::ElevatedHelper
        );
        assert_eq!(
            store.snapshot.admin_mode.state,
            RuntimeAdminModeState::Failed
        );
        assert!(store.elevated_request.is_none());
        assert!(!store.settings.admin_mode_requested);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn persisted_admin_request_is_session_only_and_drops_warm_cache() {
        let base_dir = runtime_test_dir("admin-startup");
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        fs::create_dir_all(&helper_dir).expect("helper dir exists");
        fs::write(
            base_dir.join(SETTINGS_FILE),
            serde_json::to_string(&RuntimeSettings {
                admin_mode_requested: true,
                ..RuntimeSettings::default()
            })
            .expect("settings serialize"),
        )
        .expect("settings fixture writes");
        fs::write(
            base_dir.join(WARM_CACHE_FILE),
            serde_json::to_string(&WarmCache {
                seq: 7,
                rows: vec![sample("10", "Elevated", 0.0)],
            })
            .expect("cache serializes"),
        )
        .expect("cache fixture writes");
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture writes");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture writes");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture writes");

        let store = RuntimeStore::from_base_dir(base_dir.clone());

        assert!(!store.settings.admin_mode_requested);
        assert!(store.previous_processes.is_empty());
        assert!(store.snapshot.processes.is_empty());
        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        assert!(!helper_dir.join("snapshot.json").exists());
        assert!(!helper_dir.join("snapshot.json.tmp").exists());
        assert!(!helper_dir.join("stop.signal").exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn warm_cache_rates_are_held_until_a_fresh_live_baseline_exists() {
        let base_dir = runtime_test_dir("warm-cache-rate-baseline");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        let mut cached = sample("10", "Cached", 1.0);
        cached.io_read_total_bytes = 2_000;
        cached.io_write_total_bytes = 1_000;
        cached.other_io_total_bytes = Some(500);
        cached.io_read_bps = 900;
        cached.io_write_bps = 450;
        cached.other_io_bps = Some(225);
        cached.quality.as_mut().expect("process quality").other_io = Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        ));
        fs::write(
            base_dir.join(WARM_CACHE_FILE),
            serde_json::to_string(&WarmCache {
                seq: 7,
                rows: vec![cached],
            })
            .expect("cache serializes"),
        )
        .expect("cache fixture writes");

        let store = RuntimeStore::from_base_dir(base_dir.clone());
        let row = store.snapshot.processes.first().expect("cached row");

        assert_eq!(row.io_read_bps, 0);
        assert_eq!(row.io_write_bps, 0);
        assert_eq!(row.other_io_bps, None);
        assert_eq!(
            row.quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Held, Some(MetricSource::DirectApi)))
        );
        assert_eq!(
            row.quality
                .as_ref()
                .and_then(|quality| quality.other_io.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
        assert_eq!(
            store
                .snapshot
                .process_contributors
                .io_quality
                .as_ref()
                .map(|quality| quality.quality),
            Some(MetricQuality::Held)
        );
        assert!(!store.live_process_snapshot);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn disabling_admin_mode_purges_cached_rows_and_warm_cache() {
        let base_dir = runtime_test_dir("admin-disable");
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        fs::create_dir_all(&helper_dir).expect("helper dir exists");
        fs::write(base_dir.join(WARM_CACHE_FILE), "{}").expect("cache fixture writes");
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture writes");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture writes");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture writes");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = true;
        store.previous_processes = vec![sample("10", "Elevated", 0.0)];
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture rewrites");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture rewrites");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture rewrites");

        let snapshot = store.set_admin_mode(false);

        assert!(!snapshot.settings.admin_mode_requested);
        assert!(store.previous_processes.is_empty());
        assert!(snapshot.processes.is_empty());
        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        assert!(!helper_dir.join("snapshot.json").exists());
        assert!(!helper_dir.join("snapshot.json.tmp").exists());
        assert!(!helper_dir.join("stop.signal").exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn admin_pending_status_does_not_degrade_without_warning() {
        let base_dir = runtime_test_dir("admin-pending");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = false;
        store.admin_mode.state = RuntimeAdminModeState::Requesting;
        store.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
        store.warnings.clear();

        let health = store.build_health(0, 0.0, 0);

        assert!(!health.degraded);
        assert_eq!(health.collector_warnings, 0);
        assert!(health.status_summary.contains("Windows approval"));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn warm_cache_is_not_written_while_admin_mode_is_enabled() {
        let base_dir = runtime_test_dir("admin-cache-skip");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_enabled = true;
        store.previous_processes = vec![sample("10", "Elevated", 0.0)];
        store.publication_seq = 10;

        store.persist_warm_cache();

        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn collector_warning_marks_health_degraded() {
        let mut store = RuntimeStore::new();
        let base_dir = std::env::temp_dir().join(format!(
            "batcave-runtime-health-warning-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base_dir);
        store.base_dir = base_dir.clone();
        store.settings = RuntimeSettings::default();
        store.warnings.clear();
        store.add_warning(
            "collector",
            "network_attribution_failed:access_denied".to_string(),
        );

        let health = store.build_health(3, 0.2, 64 * 1024 * 1024);

        assert!(health.degraded);
        assert_eq!(health.collector_warnings, 1);
        assert!(health.status_summary.contains("1 telemetry limitation"));

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn collector_warnings_replace_by_key_and_clear_on_recovery() {
        let base_dir = runtime_test_dir("warning-reconcile");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.warnings.clear();

        store
            .sync_collector_warnings(vec!["network_attribution_failed: access denied".to_string()]);
        store
            .sync_collector_warnings(vec!["network_attribution_failed: retry pending".to_string()]);

        assert_eq!(store.warnings.len(), 1);
        assert_eq!(store.warnings[0].key, "collector.network_attribution");
        assert_eq!(
            store.warnings[0].message,
            "network_attribution_failed: retry pending"
        );

        store.sync_collector_warnings(Vec::new());
        assert!(store.warnings.is_empty());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn corrupt_json_returns_persistence_parse_warning() {
        let path = std::env::temp_dir().join(format!(
            "batcave-corrupt-settings-{}.json",
            std::process::id()
        ));
        fs::write(&path, "{not-json").expect("corrupt fixture writes");

        let error = read_json::<RuntimeSettings>(&path).expect_err("corrupt json fails");

        fs::remove_file(&path).expect("corrupt fixture cleanup");
        assert!(error
            .expect("parse warning exists")
            .contains("persistence_parse_failed"));
    }

    fn runtime_test_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("batcave-runtime-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn sample(pid: &str, name: &str, cpu: f64) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1_700_000_000_000,
            name: name.to_string(),
            exe: format!("C:\\Program Files\\{name}.exe"),
            status: "running".to_string(),
            cpu_percent: cpu,
            kernel_cpu_percent: None,
            memory_bytes: 64 * 1024 * 1024,
            private_bytes: 32 * 1024 * 1024,
            virtual_memory_bytes: Some(128 * 1024 * 1024),
            io_read_total_bytes: 0,
            io_write_total_bytes: 0,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 1,
            handles: 1,
            access_state: AccessState::Full,
            quality: Some(ProcessMetricQuality {
                cpu: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::DirectApi,
                )),
                memory: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::DirectApi,
                )),
                io: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::DirectApi,
                )),
                network: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::DirectApi,
                )),
                ..ProcessMetricQuality::default()
            }),
        }
    }
}
