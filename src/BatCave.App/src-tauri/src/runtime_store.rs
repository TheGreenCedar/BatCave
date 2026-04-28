use std::{
    cmp::Ordering, collections::VecDeque, env, fs, path::PathBuf, sync::Mutex, time::Instant,
};

use serde::de::DeserializeOwned;

use crate::{
    atomic_json::{write_json_atomic, AtomicJsonErrorLabels},
    contracts::{
        ProcessSample, RuntimeHealth, RuntimeQuery, RuntimeSettings, RuntimeSnapshot,
        RuntimeWarning, SortColumn, SortDirection, SystemMetricsSnapshot, WarmCache,
    },
    elevation::ElevatedHelperClient,
    telemetry::{now_ms, TelemetryCollector},
};

const SETTINGS_FILE: &str = "settings.json";
const WARM_CACHE_FILE: &str = "warm-cache.json";
const DIAGNOSTICS_FILE: &str = "diagnostics.jsonl";
const MAX_WARNINGS: usize = 16;
const WARM_CACHE_WRITE_INTERVAL_TICKS: u64 = 10;
const APP_CPU_DEGRADE_PCT: f64 = 6.0;
const APP_RSS_DEGRADE_BYTES: u64 = 350 * 1024 * 1024;
const PERSISTENCE_JSON_ERRORS: AtomicJsonErrorLabels = AtomicJsonErrorLabels {
    write_failed: "persistence_write_failed",
    serialize_failed: "persistence_serialize_failed",
    replace_failed: "persistence_replace_failed",
    rename_failed: "persistence_rename_failed",
    serialize_error_includes_path: true,
};

pub struct RuntimeState {
    store: Mutex<RuntimeStore>,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(RuntimeStore::new()),
        }
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

    pub fn set_admin_mode(&self, enabled: bool) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_admin_mode(enabled))
    }

    pub fn set_query(&self, query: RuntimeQuery) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_query(query))
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
    settings: RuntimeSettings,
    elevated: Option<ElevatedHelperClient>,
    snapshot: RuntimeSnapshot,
    warnings: VecDeque<RuntimeWarning>,
    previous_totals: Option<TelemetryTotals>,
    previous_processes: Vec<ProcessSample>,
    tick_p95: P95Window,
    sort_p95: P95Window,
    jitter_p95: P95Window,
    last_tick_at: Option<Instant>,
    dropped_ticks: u64,
    seq: u64,
}

impl RuntimeStore {
    fn new() -> Self {
        let base_dir = default_base_dir();
        let mut warnings = VecDeque::new();
        let settings =
            read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE)).unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                RuntimeSettings::default()
            });
        let warm_cache =
            read_json::<WarmCache>(&base_dir.join(WARM_CACHE_FILE)).unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                WarmCache {
                    seq: 0,
                    rows: Vec::new(),
                }
            });
        let seq = warm_cache.seq;
        let settings = normalize_settings(settings);
        let snapshot = build_snapshot(
            seq,
            now_ms(),
            &settings,
            RuntimeHealth::default(),
            empty_system(),
            shape_rows(&warm_cache.rows, &settings.query),
            warm_cache.rows.len(),
            warnings.iter().cloned().collect(),
        );

        Self {
            collector: TelemetryCollector::new(),
            base_dir,
            settings,
            elevated: None,
            snapshot,
            warnings,
            previous_totals: None,
            previous_processes: warm_cache.rows,
            tick_p95: P95Window::new(120),
            sort_p95: P95Window::new(120),
            jitter_p95: P95Window::new(120),
            last_tick_at: None,
            dropped_ticks: 0,
            seq,
        }
    }

    fn snapshot(&mut self) -> RuntimeSnapshot {
        if self.settings.paused {
            return self.snapshot.clone();
        }

        self.tick()
    }

    fn refresh_now(&mut self) -> RuntimeSnapshot {
        self.tick()
    }

    fn set_paused(&mut self, paused: bool) -> RuntimeSnapshot {
        self.settings.paused = paused;
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_admin_mode(&mut self, enabled: bool) -> RuntimeSnapshot {
        self.settings.admin_mode_requested = enabled;
        self.settings.admin_mode_enabled = false;
        if enabled {
            if self.elevated.is_none() {
                match ElevatedHelperClient::start(&self.base_dir) {
                    Ok(client) => {
                        self.elevated = Some(client);
                        self.add_warning(
                            "admin_mode",
                            "admin_mode_launch_requested waiting for elevated helper data"
                                .to_string(),
                        );
                    }
                    Err(error) => {
                        self.add_warning("admin_mode", error);
                    }
                }
            }
        } else if let Some(client) = self.elevated.take() {
            client.stop();
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

    fn tick(&mut self) -> RuntimeSnapshot {
        self.seq = self.seq.saturating_add(1);
        let tick_started = Instant::now();
        let previous_tick_at = self.last_tick_at.replace(tick_started);
        let sample = match self.collector.collect() {
            Ok(sample) => sample,
            Err(error) => {
                self.add_warning("collector", error);
                self.publish_snapshot_only(None);
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
        for warning in &sample.warnings {
            self.add_warning("collector", warning.clone());
        }

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
        if self.settings.admin_mode_requested {
            let mut admin_warning = None;
            let mut stop_elevated = false;
            if let Some(elevated) = &mut self.elevated {
                match elevated.poll_rows() {
                    Ok(Some(rows)) => {
                        sample_processes = rows;
                        self.settings.admin_mode_enabled = true;
                    }
                    Ok(None) => {
                        self.settings.admin_mode_enabled = false;
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
            if stop_elevated {
                if let Some(client) = self.elevated.take() {
                    client.stop();
                }
            }
            if let Some(error) = admin_warning {
                self.add_warning("admin_mode", error);
            }
        } else {
            self.settings.admin_mode_enabled = false;
        }
        if let Some(previous) = &self.previous_totals {
            if system.disk_read_bps == 0 && system.disk_write_bps == 0 {
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

        let processes =
            add_process_rates(sample_processes, &self.previous_processes, elapsed_seconds);
        self.previous_processes = processes;
        self.previous_totals = Some(TelemetryTotals::from_system(&system, sample_ts_ms));

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
            self.seq,
            now_ms(),
            &self.settings,
            health,
            system,
            rows,
            self.previous_processes.len(),
            self.warnings.iter().cloned().collect(),
        );

        if self.seq % WARM_CACHE_WRITE_INTERVAL_TICKS == 0 {
            self.persist_warm_cache();
        }

        self.snapshot.clone()
    }

    fn publish_snapshot_only(&mut self, warning: Option<&str>) {
        self.seq = self.seq.saturating_add(1);
        if let Some(message) = warning {
            self.add_warning("runtime", message.to_string());
        }

        let app_metrics = current_app_metrics(&self.previous_processes);
        let health = self.build_health(0, app_metrics.cpu_percent, app_metrics.rss_bytes);
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        self.snapshot = build_snapshot(
            self.seq,
            now_ms(),
            &self.settings,
            health,
            self.snapshot.system.clone(),
            rows,
            self.previous_processes.len(),
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
        let last_warning = self.warnings.back().map(|warning| warning.message.clone());
        let status_summary = if self.settings.paused {
            "Paused.".to_string()
        } else if self.settings.admin_mode_requested && !self.settings.admin_mode_enabled {
            "Standard access: admin mode requested but elevation is inactive.".to_string()
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
            tick_count: self.seq,
            snapshot_latency_ms: latency_ms,
            degraded: cpu_degraded || rss_degraded,
            collector_warnings: self.warnings.len(),
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
        push_warning(&mut self.warnings, self.seq, category, message.clone());
        self.append_diagnostic(category, &message);
    }

    fn persist_settings(&mut self) {
        if let Err(error) = write_json_atomic(
            &self.base_dir.join(SETTINGS_FILE),
            &self.settings,
            PERSISTENCE_JSON_ERRORS,
        ) {
            self.add_warning("persistence", error);
        }
    }

    fn persist_warm_cache(&mut self) {
        let cache = WarmCache {
            seq: self.seq,
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

    fn append_diagnostic(&self, category: &str, message: &str) {
        let path = self.base_dir.join(DIAGNOSTICS_FILE);
        let payload = serde_json::json!({
            "ts_ms": now_ms(),
            "category": category,
            "payload": { "message": message },
        });
        let _ = fs::create_dir_all(&self.base_dir);
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

fn build_snapshot(
    seq: u64,
    ts_ms: u64,
    settings: &RuntimeSettings,
    health: RuntimeHealth,
    system: SystemMetricsSnapshot,
    processes: Vec<ProcessSample>,
    total_process_count: usize,
    warnings: Vec<RuntimeWarning>,
) -> RuntimeSnapshot {
    RuntimeSnapshot {
        event_kind: "runtime_snapshot".to_string(),
        seq,
        ts_ms,
        source: "tauri_runtime".to_string(),
        settings: settings.clone(),
        health,
        system,
        processes,
        total_process_count,
        warnings,
    }
}

fn add_process_rates(
    mut processes: Vec<ProcessSample>,
    previous_processes: &[ProcessSample],
    elapsed_seconds: f64,
) -> Vec<ProcessSample> {
    for process in &mut processes {
        if let Some(previous) = previous_processes.iter().find(|candidate| {
            candidate.pid == process.pid && candidate.start_time_ms == process.start_time_ms
        }) {
            process.disk_read_bps = byte_rate(
                process.disk_read_total_bytes,
                previous.disk_read_total_bytes,
                elapsed_seconds,
            );
            process.disk_write_bps = byte_rate(
                process.disk_write_total_bytes,
                previous.disk_write_total_bytes,
                elapsed_seconds,
            );
            process.other_io_bps =
                match (process.other_io_total_bytes, previous.other_io_total_bytes) {
                    (Some(current), Some(previous)) => {
                        Some(byte_rate(current, previous, elapsed_seconds))
                    }
                    _ => process.other_io_bps,
                };
        }
    }

    processes
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
        .cloned()
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| compare_process(left, right, query));
    rows.truncate(query.limit.max(1));
    rows
}

fn compare_process(left: &ProcessSample, right: &ProcessSample, query: &RuntimeQuery) -> Ordering {
    let ordering = match query.sort_column {
        SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortColumn::Pid => left.pid.cmp(&right.pid),
        SortColumn::MemoryBytes => left.memory_bytes.cmp(&right.memory_bytes),
        SortColumn::DiskBps => left
            .disk_read_bps
            .saturating_add(left.disk_write_bps)
            .cmp(&right.disk_read_bps.saturating_add(right.disk_write_bps)),
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

fn normalize_settings(settings: RuntimeSettings) -> RuntimeSettings {
    RuntimeSettings {
        query: normalize_query(settings.query),
        admin_mode_requested: settings.admin_mode_requested,
        admin_mode_enabled: false,
        metric_window_seconds: settings.metric_window_seconds.clamp(15, 600),
        paused: settings.paused,
    }
}

fn process_attention_score(process: &ProcessSample) -> f64 {
    let mut score = process.cpu_percent * 3.0;
    score += (process.memory_bytes as f64 / (128.0 * 1024.0 * 1024.0)).min(20.0);
    let other_io_bps = process.other_io_bps.unwrap_or_default();
    let io_bps = process
        .disk_read_bps
        .saturating_add(process.disk_write_bps)
        .saturating_add(other_io_bps);
    score += (io_bps as f64 / (512.0 * 1024.0)).min(20.0);
    let network_bps = process
        .network_received_bps
        .unwrap_or_default()
        .saturating_add(process.network_transmitted_bps.unwrap_or_default());
    score += (network_bps as f64 / (1024.0 * 1024.0)).min(20.0);
    if process.access_state != crate::contracts::AccessState::Full {
        score += 12.0;
    }

    score
}

fn normalize_query(query: RuntimeQuery) -> RuntimeQuery {
    RuntimeQuery {
        filter_text: query.filter_text.trim().to_string(),
        sort_column: query.sort_column,
        sort_direction: query.sort_direction,
        limit: query.limit.clamp(25, 20_000),
    }
}

fn push_warning(
    warnings: &mut VecDeque<RuntimeWarning>,
    seq: u64,
    category: &str,
    message: String,
) {
    warnings.push_back(RuntimeWarning {
        seq,
        ts_ms: now_ms(),
        category: category.to_string(),
        message,
    });
    while warnings.len() > MAX_WARNINGS {
        warnings.pop_front();
    }
}

fn empty_system() -> SystemMetricsSnapshot {
    SystemMetricsSnapshot {
        cpu_percent: 0.0,
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: Vec::new(),
        memory_used_bytes: 0,
        memory_total_bytes: 0,
        memory_available_bytes: None,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        process_count: 0,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: 0,
        network_transmitted_total_bytes: 0,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        quality: None,
    }
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

fn default_base_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("BatCaveMonitor")
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::AccessState;

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
    fn process_rates_use_pid_and_start_time_identity() {
        let mut previous = sample("10", "Stable", 1.0);
        previous.start_time_ms = 100;
        previous.disk_read_total_bytes = 100;
        previous.disk_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(10);

        let mut current = previous.clone();
        current.disk_read_total_bytes = 600;
        current.disk_write_total_bytes = 250;
        current.other_io_total_bytes = Some(110);

        let updated = add_process_rates(vec![current], &[previous], 1.0);

        assert_eq!(updated[0].disk_read_bps, 500);
        assert_eq!(updated[0].disk_write_bps, 200);
        assert_eq!(updated[0].other_io_bps, Some(100));

        let mut restarted_previous = sample("10", "Stable", 1.0);
        restarted_previous.start_time_ms = 1;
        let restarted = add_process_rates(
            vec![sample("10", "Stable", 1.0)],
            &[restarted_previous],
            1.0,
        );

        assert_eq!(restarted[0].disk_read_bps, 0);
        assert_eq!(restarted[0].other_io_bps, None);
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
    fn settings_normalization_clears_effective_admin_and_clamps_ranges() {
        let settings = normalize_settings(RuntimeSettings {
            admin_mode_requested: true,
            admin_mode_enabled: true,
            metric_window_seconds: 1,
            query: RuntimeQuery {
                filter_text: "  code  ".to_string(),
                limit: usize::MAX,
                ..RuntimeQuery::default()
            },
            paused: false,
        });

        assert!(settings.admin_mode_requested);
        assert!(!settings.admin_mode_enabled);
        assert_eq!(settings.metric_window_seconds, 15);
        assert_eq!(settings.query.filter_text, "code");
        assert_eq!(settings.query.limit, 20_000);
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
            virtual_memory_bytes: 128 * 1024 * 1024,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            other_io_total_bytes: None,
            disk_read_bps: 0,
            disk_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 1,
            handles: 1,
            access_state: AccessState::Full,
            quality: None,
        }
    }
}
