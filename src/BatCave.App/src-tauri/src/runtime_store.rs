use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Condvar, Mutex, RwLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(test)]
use serde::de::DeserializeOwned;
#[cfg(test)]
use std::env;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;

use crate::{
    collector_engine::{
        CollectorCadence, CollectorEngine, CollectorEngineConfig, CollectorEngineHandle,
        CollectorEvent, CollectorPublication, RawCollector,
    },
    contracts::{
        AccessState, GroupDetail, GroupDetailKind, GroupMetricCoverage, GroupMetricQuality,
        MetricCoverage, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
        ProcessContributorIdentity, ProcessContributorSummary, ProcessDetail, ProcessDetailKind,
        ProcessFocusMode, ProcessSample, ProcessViewRow, RuntimeAdminModeState,
        RuntimeAdminModeStatus, RuntimeCollectorServiceState, RuntimeCollectorState,
        RuntimeEngineState, RuntimeEnvironment, RuntimeFatalError, RuntimeHealth,
        RuntimePersistence, RuntimePersistenceState, RuntimePrivilegedSource, RuntimeQuery,
        RuntimeSettings, RuntimeSnapshot, RuntimeUiPreferences, RuntimeWarning, SortColumn,
        SortDirection, SystemMemoryAccounting, SystemMetricsSnapshot, WarmCache,
    },
    persistence::{
        DiagnosticWriteOutcome, JsonMigration, RuntimePersistenceCoordinator, UserStorageComponent,
    },
    runtime_provenance::RuntimeProvenance,
    telemetry::{now_ms, TelemetrySampleProvenance},
};

#[cfg(test)]
const SETTINGS_FILE: &str = "settings.json";
#[cfg(test)]
const WARM_CACHE_FILE: &str = "warm-cache.json";
const MAX_WARNINGS: usize = 16;
const WARM_CACHE_WRITE_INTERVAL_TICKS: u64 = 10;
const APP_CPU_DEGRADE_PCT: f64 = 25.0;
const APP_RSS_DEGRADE_BYTES: u64 = 350 * 1024 * 1024;
const ATTENTION_CPU_PERCENT: f64 = 10.0;
const ATTENTION_MEMORY_BYTES: u64 = 900 * 1024 * 1024;
const ATTENTION_IO_BPS: u64 = 500 * 1024;
const ATTENTION_NETWORK_BPS: u64 = 1024 * 1024;
const CONTROL_QUEUE_CAPACITY: usize = 32;
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const ENGINE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_IO_BASELINE_PENDING: &str = "Process read/write I/O rates need a fresh prior sample.";
const PROCESS_OTHER_IO_BASELINE_PENDING: &str =
    "Process Other I/O rates need a fresh prior sample.";
type SnapshotReply = mpsc::Sender<Result<Arc<RuntimeSnapshot>, String>>;

struct PublishedRuntime {
    snapshot: Arc<RuntimeSnapshot>,
    process_exe_authoritative: bool,
}

struct MonotonicWireClock {
    origin: Instant,
    wire_origin_ms: u64,
}

impl MonotonicWireClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            wire_origin_ms: now_ms(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.at_ms(Instant::now())
    }

    fn at_ms(&self, instant: Instant) -> u64 {
        if let Some(elapsed) = instant.checked_duration_since(self.origin) {
            self.wire_origin_ms.saturating_add(duration_ms(elapsed))
        } else {
            self.wire_origin_ms
                .saturating_sub(duration_ms(self.origin.duration_since(instant)))
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[derive(Clone)]
struct EngineRefreshMeasurement {
    snapshot: Arc<RuntimeSnapshot>,
    collection_latency_ms: f64,
    publication_latency_ms: f64,
}

pub(crate) struct RefreshMeasurement {
    pub snapshot: RuntimeSnapshot,
    pub collection_latency_ms: f64,
    pub publication_latency_ms: f64,
}

#[derive(Default)]
struct RefreshGateState {
    requested_generation: u64,
    started_generation: u64,
    completed_generation: u64,
    waiting_callers: usize,
    wake_queued: bool,
    completed_measurement: Option<(u64, EngineRefreshMeasurement)>,
    failed_generation: Option<(u64, String)>,
    terminal_error: Option<String>,
}

#[derive(Default)]
struct RefreshGate {
    state: Mutex<RefreshGateState>,
    changed: Condvar,
}

enum EngineControl {
    CollectorPublished,
    Refresh,
    Pause(SnapshotReply),
    Resume(SnapshotReply),
    SetQuery(RuntimeQuery, QueryWriteIntent, SnapshotReply),
    SetSampleInterval(u32, SnapshotReply),
    SetUiPreferences(RuntimeUiPreferences, SnapshotReply),
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryWriteIntent {
    RuntimeOnly,
    UserMutation,
}

pub struct RuntimeState {
    control: SyncSender<EngineControl>,
    published: Arc<RwLock<PublishedRuntime>>,
    refresh_gate: Arc<RefreshGate>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown_started: AtomicBool,
    shutdown_requested: Arc<AtomicBool>,
    clock: Arc<MonotonicWireClock>,
    shutdown_lock: Mutex<()>,
    completion: Mutex<Receiver<Result<(), String>>>,
    shutdown_result: Mutex<Option<Result<(), String>>>,
}

impl RuntimeState {
    pub fn new() -> Result<Self, String> {
        Self::from_store(RuntimeStore::from_current_process(), true)
    }

    pub(crate) fn from_base_dir_manual(base_dir: PathBuf) -> Result<Self, String> {
        Self::from_store(RuntimeStore::from_base_dir(base_dir), false)
    }

    fn from_store(store: RuntimeStore, automatic_sampling: bool) -> Result<Self, String> {
        Self::from_store_with_collector(store, automatic_sampling, None)
    }

    fn from_store_with_collector(
        store: RuntimeStore,
        automatic_sampling: bool,
        collector: Option<Box<dyn RawCollector>>,
    ) -> Result<Self, String> {
        let clock = Arc::clone(&store.clock);
        let published = Arc::new(RwLock::new(PublishedRuntime {
            snapshot: Arc::new(store.snapshot.clone()),
            process_exe_authoritative: store.live_process_snapshot,
        }));
        let refresh_gate = Arc::new(RefreshGate::default());
        let (control, receiver) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let collector_notification_queued = Arc::new(AtomicBool::new(false));
        let notifier_control = control.clone();
        let notifier_queued = Arc::clone(&collector_notification_queued);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if !notifier_queued.swap(true, AtomicOrdering::AcqRel)
                && notifier_control
                    .try_send(EngineControl::CollectorPublished)
                    .is_err()
            {
                notifier_queued.store(false, AtomicOrdering::Release);
            }
        });
        let collector_config = CollectorEngineConfig {
            interval: sample_interval(store.settings.sample_interval_ms),
            metric_window: metric_window(store.settings.metric_window_seconds),
            paused: store.settings.paused,
            automatic: automatic_sampling,
        };
        let collector_engine = match collector {
            Some(collector) => CollectorEngine::start(collector, collector_config, notify)?,
            None => CollectorEngine::start_default(collector_config, notify)?,
        };
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let worker_published = Arc::clone(&published);
        let worker_refresh_gate = Arc::clone(&refresh_gate);
        let worker_shutdown = Arc::clone(&shutdown_requested);
        let worker_notification_queued = Arc::clone(&collector_notification_queued);
        let (completion_sender, completion) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("batcave-sampling-engine".to_string())
            .spawn(move || {
                let result = run_sampling_engine(
                    store,
                    collector_engine,
                    receiver,
                    worker_published,
                    worker_refresh_gate,
                    worker_shutdown,
                    worker_notification_queued,
                );
                let _ = completion_sender.send(result);
            })
            .map_err(|error| format!("runtime_engine_spawn_failed:{error}"))?;

        Ok(Self {
            control,
            published,
            refresh_gate,
            worker: Mutex::new(Some(worker)),
            shutdown_started: AtomicBool::new(false),
            shutdown_requested,
            clock,
            shutdown_lock: Mutex::new(()),
            completion: Mutex::new(completion),
            shutdown_result: Mutex::new(None),
        })
    }

    pub fn start(&self) {
        // Construction starts the owned engine so no caller can reach a mutable
        // collector before the worker owns it. Kept for the existing app setup API.
    }

    pub fn snapshot(&self) -> Result<RuntimeSnapshot, String> {
        self.published_snapshot()
    }

    pub fn refresh_now(&self) -> Result<RuntimeSnapshot, String> {
        self.refresh_now_measured()
            .map(|measurement| measurement.snapshot)
    }

    pub(crate) fn refresh_now_measured(&self) -> Result<RefreshMeasurement, String> {
        if self.shutdown_started.load(AtomicOrdering::Acquire) {
            return Err("runtime_engine_shutting_down".to_string());
        }
        let (target_generation, schedule) = {
            let mut gate = self
                .refresh_gate
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(error) = &gate.terminal_error {
                return Err(error.clone());
            }
            if gate.requested_generation == gate.started_generation {
                gate.requested_generation = gate.requested_generation.saturating_add(1);
            }
            let target_generation = gate.requested_generation;
            let schedule = !gate.wake_queued;
            gate.wake_queued = true;
            if schedule {
                gate.failed_generation = None;
            }
            gate.waiting_callers = gate.waiting_callers.saturating_add(1);
            (target_generation, schedule)
        };

        if schedule {
            if let Err(error) = self.control.try_send(EngineControl::Refresh) {
                let message = control_send_error(error);
                fail_refresh_generation(&self.refresh_gate, target_generation, message.clone());
                finish_refresh_wait(&self.refresh_gate);
                return Err(message);
            }
        }
        let result = wait_for_refresh_generation(&self.refresh_gate, target_generation);
        finish_refresh_wait(&self.refresh_gate);
        let measurement = result?;
        let mut snapshot = (*measurement.snapshot).clone();
        evaluate_snapshot_health(&mut snapshot, self.clock.now_ms());
        Ok(RefreshMeasurement {
            snapshot,
            collection_latency_ms: measurement.collection_latency_ms,
            publication_latency_ms: measurement.publication_latency_ms,
        })
    }

    pub fn pause(&self) -> Result<RuntimeSnapshot, String> {
        self.request_snapshot(EngineControl::Pause)
    }

    pub fn resume(&self) -> Result<RuntimeSnapshot, String> {
        self.request_snapshot(EngineControl::Resume)
    }

    pub fn set_query(&self, query: RuntimeQuery) -> Result<RuntimeSnapshot, String> {
        self.request_query(query, QueryWriteIntent::UserMutation)
    }

    pub(crate) fn set_query_runtime_only(
        &self,
        query: RuntimeQuery,
    ) -> Result<RuntimeSnapshot, String> {
        self.request_query(query, QueryWriteIntent::RuntimeOnly)
    }

    pub fn set_sample_interval(&self, sample_interval_ms: u32) -> Result<RuntimeSnapshot, String> {
        self.request_snapshot(|reply| EngineControl::SetSampleInterval(sample_interval_ms, reply))
    }

    pub fn set_ui_preferences(
        &self,
        preferences: RuntimeUiPreferences,
    ) -> Result<RuntimeSnapshot, String> {
        self.request_snapshot(|reply| EngineControl::SetUiPreferences(preferences, reply))
    }

    pub fn has_process_exe(&self, exe: &str) -> Result<bool, String> {
        let published = self
            .published
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = Arc::clone(&published.snapshot);
        let authoritative = published.process_exe_authoritative;
        drop(published);
        let exe = exe.trim();
        Ok(authoritative
            && !exe.is_empty()
            && snapshot
                .processes
                .iter()
                .any(|process| process.exe.eq_ignore_ascii_case(exe)))
    }

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let _shutdown = self
            .shutdown_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(result) = self
            .shutdown_result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return result;
        }
        if !self.shutdown_started.swap(true, AtomicOrdering::AcqRel) {
            self.shutdown_requested.store(true, AtomicOrdering::Release);
            terminate_refresh_gate(
                &self.refresh_gate,
                "runtime_engine_shutting_down".to_string(),
            );
            let _ = self.control.try_send(EngineControl::Shutdown);
        }
        let cleanup_result = self
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv_timeout(ENGINE_SHUTDOWN_TIMEOUT)
            .map_err(|_| "runtime_engine_shutdown_timeout".to_string())?;
        let join_result = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map_or(Ok(()), |worker| {
                worker
                    .join()
                    .map_err(|_| "runtime_engine_join_failed".to_string())
            });
        let result = cleanup_result.and(join_result);
        *self
            .shutdown_result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result.clone());
        result
    }

    fn request_snapshot(
        &self,
        control: impl FnOnce(SnapshotReply) -> EngineControl,
    ) -> Result<RuntimeSnapshot, String> {
        if self.shutdown_started.load(AtomicOrdering::Acquire) {
            return Err("runtime_engine_shutting_down".to_string());
        }
        let (reply, receiver) = mpsc::channel();
        self.control
            .try_send(control(reply))
            .map_err(control_send_error)?;
        receive_snapshot(receiver, self.clock.now_ms())
    }

    fn request_query(
        &self,
        query: RuntimeQuery,
        intent: QueryWriteIntent,
    ) -> Result<RuntimeSnapshot, String> {
        self.request_snapshot(|reply| EngineControl::SetQuery(query, intent, reply))
    }

    fn published_snapshot(&self) -> Result<RuntimeSnapshot, String> {
        let published = self
            .published
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = Arc::clone(&published.snapshot);
        drop(published);
        let mut snapshot = (*snapshot).clone();
        evaluate_snapshot_health(&mut snapshot, self.clock.now_ms());
        Ok(snapshot)
    }
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn receive_snapshot(
    receiver: Receiver<Result<Arc<RuntimeSnapshot>, String>>,
    evaluated_at_ms: u64,
) -> Result<RuntimeSnapshot, String> {
    let snapshot = receiver
        .recv_timeout(CONTROL_RESPONSE_TIMEOUT)
        .map_err(|_| "runtime_control_timeout".to_string())??;
    let mut snapshot = (*snapshot).clone();
    evaluate_snapshot_health(&mut snapshot, evaluated_at_ms);
    Ok(snapshot)
}

fn control_send_error(error: TrySendError<EngineControl>) -> String {
    match error {
        TrySendError::Full(_) => "runtime_control_busy".to_string(),
        TrySendError::Disconnected(_) => "runtime_engine_unavailable".to_string(),
    }
}

fn run_sampling_engine(
    mut store: RuntimeStore,
    collector_engine: CollectorEngine,
    receiver: Receiver<EngineControl>,
    published: Arc<RwLock<PublishedRuntime>>,
    refresh_gate: Arc<RefreshGate>,
    shutdown_requested: Arc<AtomicBool>,
    collector_notification_queued: Arc<AtomicBool>,
) -> Result<(), String> {
    let clock = Arc::clone(&store.clock);
    let collector = collector_engine.handle();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        sampling_engine_loop(
            &mut store,
            &collector,
            &receiver,
            &published,
            &refresh_gate,
            &shutdown_requested,
            &collector_notification_queued,
        )
    }));
    let fatal = match result {
        Ok(false) => false,
        Ok(true) => true,
        Err(_) => {
            let error = "runtime sampling engine panicked".to_string();
            publish_fatal_from_latest(
                &published,
                "sampling_engine_panicked",
                &error,
                clock.now_ms(),
            );
            terminate_refresh_gate(&refresh_gate, "runtime_engine_fatal".to_string());
            true
        }
    };
    let collector_cleanup = collector_engine.shutdown();
    let runtime_cleanup = store.shutdown_owned_resources();
    if fatal {
        fatal_control_loop(&receiver, &shutdown_requested);
    }
    match (collector_cleanup, runtime_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(collector), Ok(())) => Err(collector),
        (Ok(()), Err(runtime)) => Err(runtime),
        (Err(collector), Err(runtime)) => Err(format!("{collector};{runtime}")),
    }
}

fn sampling_engine_loop(
    store: &mut RuntimeStore,
    collector: &CollectorEngineHandle,
    receiver: &Receiver<EngineControl>,
    published: &Arc<RwLock<PublishedRuntime>>,
    refresh_gate: &Arc<RefreshGate>,
    shutdown_requested: &AtomicBool,
    collector_notification_queued: &AtomicBool,
) -> bool {
    let mut last_collector_revision = 0_u64;
    store.set_engine_state(if store.settings.paused {
        crate::contracts::RuntimeEngineState::Paused
    } else {
        crate::contracts::RuntimeEngineState::Starting
    });
    publish_store(store, published);

    loop {
        if shutdown_requested.load(AtomicOrdering::Acquire) {
            return false;
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(EngineControl::Shutdown) => return false,
            Ok(EngineControl::CollectorPublished) => {
                collector_notification_queued.store(false, AtomicOrdering::Release);
                if let Err(error) = apply_latest_collector_publication(
                    store,
                    collector,
                    published,
                    &mut last_collector_revision,
                ) {
                    terminate_refresh_gate(refresh_gate, error);
                    return true;
                }
            }
            Ok(EngineControl::Refresh) => {
                let Some(generation) = begin_refresh_generation(refresh_gate) else {
                    continue;
                };
                match collector.refresh_now() {
                    Ok(publication) => match apply_collector_publication(
                        store,
                        published,
                        &mut last_collector_revision,
                        publication,
                    ) {
                        Ok(Some(measurement)) => {
                            complete_refresh_generation(refresh_gate, generation, measurement)
                        }
                        Ok(None) => fail_refresh_generation(
                            refresh_gate,
                            generation,
                            "runtime_refresh_result_unavailable".to_string(),
                        ),
                        Err(error) => {
                            terminate_refresh_gate(refresh_gate, error);
                            return true;
                        }
                    },
                    Err(error) => {
                        if let Err(fatal) = apply_latest_collector_publication(
                            store,
                            collector,
                            published,
                            &mut last_collector_revision,
                        ) {
                            terminate_refresh_gate(refresh_gate, fatal);
                            return true;
                        }
                        fail_refresh_generation(refresh_gate, generation, error);
                    }
                }
            }
            Ok(EngineControl::Pause(reply)) => {
                if let Err(error) = collector.pause() {
                    let _ = reply.send(Err(error));
                    continue;
                }
                if let Err(error) = apply_latest_collector_publication(
                    store,
                    collector,
                    published,
                    &mut last_collector_revision,
                ) {
                    let _ = reply.send(Err(error.clone()));
                    terminate_refresh_gate(refresh_gate, error);
                    return true;
                }
                store.note_heartbeat();
                store.set_engine_state(crate::contracts::RuntimeEngineState::Paused);
                store.set_paused(true);
                reply_with_publication(reply, store, published);
            }
            Ok(EngineControl::Resume(reply)) => {
                store.note_heartbeat();
                store.set_engine_state(crate::contracts::RuntimeEngineState::Running);
                store.set_paused(false);
                match collector.resume() {
                    Ok(publication) => match apply_collector_publication(
                        store,
                        published,
                        &mut last_collector_revision,
                        publication,
                    ) {
                        Ok(Some(measurement)) => {
                            let _ = reply.send(Ok(measurement.snapshot));
                        }
                        Ok(None) => {
                            let _ =
                                reply.send(Err("runtime_refresh_result_unavailable".to_string()));
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error.clone()));
                            terminate_refresh_gate(refresh_gate, error);
                            return true;
                        }
                    },
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Ok(EngineControl::SetQuery(query, intent, reply)) => {
                store.note_heartbeat();
                store.set_query_with_intent(query, intent);
                reply_with_publication(reply, store, published);
            }
            Ok(EngineControl::SetSampleInterval(sample_interval_ms, reply)) => {
                store.note_heartbeat();
                let normalized = sample_interval_ms.clamp(500, 5_000);
                if normalized != store.settings.sample_interval_ms {
                    if let Err(error) = collector.set_interval(sample_interval(normalized)) {
                        let _ = reply.send(Err(error));
                        continue;
                    }
                }
                store.set_sample_interval(normalized);
                reply_with_publication(reply, store, published);
            }
            Ok(EngineControl::SetUiPreferences(preferences, reply)) => {
                store.note_heartbeat();
                store.set_ui_preferences(preferences);
                reply_with_publication(reply, store, published);
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = apply_latest_collector_publication(
                    store,
                    collector,
                    published,
                    &mut last_collector_revision,
                ) {
                    terminate_refresh_gate(refresh_gate, error);
                    return true;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn apply_latest_collector_publication(
    store: &mut RuntimeStore,
    collector: &CollectorEngineHandle,
    published: &Arc<RwLock<PublishedRuntime>>,
    last_collector_revision: &mut u64,
) -> Result<(), String> {
    let Some(publication) = collector.snapshot() else {
        return Ok(());
    };
    if publication.revision <= *last_collector_revision {
        return Ok(());
    }
    apply_collector_publication(store, published, last_collector_revision, publication).map(|_| ())
}

fn apply_collector_publication(
    store: &mut RuntimeStore,
    published: &Arc<RwLock<PublishedRuntime>>,
    last_collector_revision: &mut u64,
    publication: Arc<CollectorPublication>,
) -> Result<Option<EngineRefreshMeasurement>, String> {
    if publication.revision <= *last_collector_revision {
        return Ok(None);
    }
    *last_collector_revision = publication.revision;
    let completed_at_ms = store.clock.at_ms(publication.completed_at);
    store.note_heartbeat_at(completed_at_ms);
    store.update_schedule_health(publication.cadence);
    if !store.settings.paused && !matches!(&publication.event, CollectorEvent::PausedHeartbeat) {
        store.set_engine_state(crate::contracts::RuntimeEngineState::Running);
    }

    let publication_started = match &publication.event {
        CollectorEvent::PausedHeartbeat => {
            store.publish_snapshot_only(None);
            publish_store(store, published);
            return Ok(None);
        }
        CollectorEvent::Sample(sample) => store.apply_raw_sample(
            (**sample).clone(),
            publication.collection_latency_ms,
            completed_at_ms,
        ),
        CollectorEvent::Unavailable(error) => {
            store.record_collection_latency_ms(publication.collection_latency_ms);
            let publication_started = Instant::now();
            store.publish_collector_unavailable(error.clone());
            publication_started
        }
        CollectorEvent::Fatal { code, message } => {
            store.record_collection_latency_ms(publication.collection_latency_ms);
            let publication_started = Instant::now();
            store.mark_engine_fatal(code, message.clone());
            publish_store_measured(store, published, publication_started);
            return Err(if code == "collector_fatal" {
                format!("runtime_engine_fatal:{message}")
            } else {
                "runtime_engine_fatal".to_string()
            });
        }
    };

    let (snapshot, publication_latency_ms) =
        publish_store_measured(store, published, publication_started);
    Ok(Some(EngineRefreshMeasurement {
        snapshot,
        collection_latency_ms: store.collection_latency_ms.unwrap_or_default(),
        publication_latency_ms,
    }))
}

fn reply_with_publication(
    reply: SnapshotReply,
    store: &mut RuntimeStore,
    published: &Arc<RwLock<PublishedRuntime>>,
) {
    let _ = reply.send(Ok(publish_store(store, published)));
}

fn publish_store(
    store: &mut RuntimeStore,
    published: &Arc<RwLock<PublishedRuntime>>,
) -> Arc<RuntimeSnapshot> {
    store.refresh_snapshot_health();
    let snapshot = Arc::new(store.snapshot.clone());
    let mut target = published
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *target = PublishedRuntime {
        snapshot: Arc::clone(&snapshot),
        process_exe_authoritative: store.live_process_snapshot,
    };
    drop(target);
    snapshot
}

fn publish_store_measured(
    store: &mut RuntimeStore,
    published: &Arc<RwLock<PublishedRuntime>>,
    publication_started: Instant,
) -> (Arc<RuntimeSnapshot>, f64) {
    store.refresh_snapshot_health();
    let mut snapshot = store.snapshot.clone();
    store.record_publication_latency(publication_started.elapsed());
    snapshot.health = store.snapshot.health.clone();
    let snapshot = Arc::new(snapshot);
    let mut target = published
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *target = PublishedRuntime {
        snapshot: Arc::clone(&snapshot),
        process_exe_authoritative: store.live_process_snapshot,
    };
    drop(target);
    let publication_latency_ms = publication_started.elapsed().as_secs_f64() * 1000.0;
    (snapshot, publication_latency_ms)
}

fn publish_fatal_from_latest(
    published: &Arc<RwLock<PublishedRuntime>>,
    code: &str,
    message: &str,
    occurred_at_ms: u64,
) {
    let current = {
        let current = published
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(&current.snapshot)
    };
    let mut snapshot = (*current).clone();
    snapshot.publication_seq = snapshot.publication_seq.saturating_add(1);
    snapshot.published_at_ms = occurred_at_ms.max(snapshot.sampled_at_ms.unwrap_or_default());
    snapshot.health.engine_state = Some(crate::contracts::RuntimeEngineState::Fatal);
    snapshot.health.collector_state = Some(crate::contracts::RuntimeCollectorState::Unavailable);
    snapshot.health.degraded = true;
    snapshot.health.status_summary = "Sampling engine stopped after a fatal error.".to_string();
    snapshot.health.updated_at_ms = snapshot.published_at_ms;
    snapshot.health.fatal_error = Some(crate::contracts::RuntimeFatalError {
        code: code.to_string(),
        message: message.to_string(),
        occurred_at_ms: snapshot.published_at_ms,
    });
    let mut target = published
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *target = PublishedRuntime {
        snapshot: Arc::new(snapshot),
        process_exe_authoritative: false,
    };
}

fn fatal_control_loop(receiver: &Receiver<EngineControl>, shutdown_requested: &AtomicBool) {
    while !shutdown_requested.load(AtomicOrdering::Acquire) {
        let Ok(control) = receiver.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        match control {
            EngineControl::Shutdown => return,
            EngineControl::CollectorPublished | EngineControl::Refresh => {}
            EngineControl::Pause(reply) | EngineControl::Resume(reply) => {
                let _ = reply.send(Err("runtime_engine_fatal".to_string()));
            }
            EngineControl::SetQuery(_, _, reply)
            | EngineControl::SetSampleInterval(_, reply)
            | EngineControl::SetUiPreferences(_, reply) => {
                let _ = reply.send(Err("runtime_engine_fatal".to_string()));
            }
        }
    }
}

fn begin_refresh_generation(refresh_gate: &RefreshGate) -> Option<u64> {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.wake_queued = false;
    if gate.requested_generation <= gate.started_generation {
        return None;
    }
    gate.started_generation = gate.requested_generation;
    Some(gate.started_generation)
}

fn complete_refresh_generation(
    refresh_gate: &RefreshGate,
    generation: u64,
    measurement: EngineRefreshMeasurement,
) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.completed_generation = gate.completed_generation.max(generation);
    gate.completed_measurement = Some((generation, measurement));
    refresh_gate.changed.notify_all();
}

fn fail_refresh_generation(refresh_gate: &RefreshGate, generation: u64, error: String) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.wake_queued = false;
    gate.started_generation = gate.started_generation.max(generation);
    gate.failed_generation = Some((generation, error));
    refresh_gate.changed.notify_all();
}

fn finish_refresh_wait(refresh_gate: &RefreshGate) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.waiting_callers = gate.waiting_callers.saturating_sub(1);
}

fn terminate_refresh_gate(refresh_gate: &RefreshGate, error: String) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.terminal_error = Some(error);
    refresh_gate.changed.notify_all();
}

fn wait_for_refresh_generation(
    refresh_gate: &RefreshGate,
    target_generation: u64,
) -> Result<EngineRefreshMeasurement, String> {
    let deadline = Instant::now() + CONTROL_RESPONSE_TIMEOUT;
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        if gate.completed_generation >= target_generation {
            return gate
                .completed_measurement
                .as_ref()
                .map(|(_, measurement)| measurement.clone())
                .ok_or_else(|| "runtime_refresh_result_unavailable".to_string());
        }
        if let Some(error) = &gate.terminal_error {
            return Err(error.clone());
        }
        if let Some((generation, error)) = &gate.failed_generation {
            if *generation >= target_generation {
                return Err(error.clone());
            }
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err("runtime_control_timeout".to_string());
        }
        let (next, result) = refresh_gate
            .changed
            .wait_timeout(gate, timeout)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gate = next;
        if result.timed_out() {
            return Err("runtime_control_timeout".to_string());
        }
    }
}

fn sample_interval(sample_interval_ms: u32) -> Duration {
    Duration::from_millis(u64::from(sample_interval_ms.clamp(500, 5_000)))
}

fn metric_window(metric_window_seconds: u32) -> Duration {
    Duration::from_secs(u64::from(metric_window_seconds.clamp(15, 600)))
}

fn evaluate_snapshot_health(snapshot: &mut RuntimeSnapshot, evaluated_at_ms: u64) {
    let evaluated_at_ms = evaluated_at_ms
        .max(snapshot.published_at_ms)
        .max(snapshot.health.last_heartbeat_at_ms.unwrap_or_default());
    snapshot.health.updated_at_ms = evaluated_at_ms;
    match snapshot.health.engine_state {
        Some(RuntimeEngineState::Fatal) => {
            snapshot.health.degraded = true;
            snapshot.health.status_summary =
                "Sampling engine stopped after a fatal error.".to_string();
        }
        Some(RuntimeEngineState::Running) => {
            let interval_ms = u64::from(snapshot.settings.sample_interval_ms.clamp(500, 5_000));
            let heartbeat_stale = snapshot
                .health
                .last_heartbeat_at_ms
                .is_none_or(|heartbeat| {
                    evaluated_at_ms.saturating_sub(heartbeat) > interval_ms * 2
                });
            let publication_stale =
                evaluated_at_ms.saturating_sub(snapshot.published_at_ms) > interval_ms * 2;
            let collection_budget_ms = snapshot
                .health
                .collection_latency_ms
                .unwrap_or_default()
                .ceil()
                .max(interval_ms as f64) as u64;
            let sample_stale = snapshot.sampled_at_ms.is_none_or(|sampled_at_ms| {
                evaluated_at_ms.saturating_sub(sampled_at_ms)
                    > interval_ms.saturating_add(collection_budget_ms)
            });
            if heartbeat_stale || publication_stale || sample_stale {
                snapshot.health.degraded = true;
                snapshot.health.status_summary = if heartbeat_stale {
                    "Sampling engine heartbeat is stale.".to_string()
                } else if publication_stale {
                    "Snapshot publication is stale.".to_string()
                } else {
                    "Telemetry sample is stale.".to_string()
                };
            }
        }
        Some(RuntimeEngineState::Paused | RuntimeEngineState::Starting) | None => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsWriteIntent {
    Automatic,
    UserMutation,
}

struct RuntimeStore {
    clock: Arc<MonotonicWireClock>,
    persistence: RuntimePersistenceCoordinator,
    provenance: RuntimeProvenance,
    settings: RuntimeSettings,
    // Runtime-only shaping can differ from the last user-authored query.
    durable_query: RuntimeQuery,
    admin_mode: RuntimeAdminModeStatus,
    snapshot: RuntimeSnapshot,
    warnings: VecDeque<RuntimeWarning>,
    previous_totals: Option<TelemetryTotals>,
    previous_processes: Vec<ProcessSample>,
    live_process_snapshot: bool,
    tick_p95: P95Window,
    sort_p95: P95Window,
    publication_p95: P95Window,
    engine_state: RuntimeEngineState,
    collector_state: Option<RuntimeCollectorState>,
    last_heartbeat_at_ms: Option<u64>,
    deadline_misses: u64,
    recent_deadline_misses: u64,
    deadline_lateness_p95_ms: f64,
    collection_latency_ms: Option<f64>,
    publication_latency_ms: Option<f64>,
    fatal_error: Option<RuntimeFatalError>,
    publication_seq: u64,
    sample_seq: u64,
    sampled_at_ms: Option<u64>,
    last_sample_provenance: Option<TelemetrySampleProvenance>,
    // A failed load may hide original bytes. Only a user mutation may replace them with defaults.
    settings_rewrite_blocked: bool,
    persistence_flushed: bool,
}

impl RuntimeStore {
    #[cfg(test)]
    fn new() -> Self {
        Self::from_base_dir(default_base_dir())
    }

    fn from_base_dir(base_dir: PathBuf) -> Self {
        let clock = Arc::new(MonotonicWireClock::new());
        let persistence = RuntimePersistenceCoordinator::for_current_user_directory(
            base_dir.clone(),
            clock.now_ms(),
        );
        Self::from_base_dir_with_persistence(base_dir, clock, persistence)
    }

    fn from_current_process() -> Self {
        let clock = Arc::new(MonotonicWireClock::new());
        let persistence = RuntimePersistenceCoordinator::from_current_process(clock.now_ms());
        let base_dir = persistence.runtime_directory().to_path_buf();
        Self::from_base_dir_with_persistence(base_dir, clock, persistence)
    }

    fn from_base_dir_with_persistence(
        base_dir: PathBuf,
        clock: Arc<MonotonicWireClock>,
        mut persistence: RuntimePersistenceCoordinator,
    ) -> Self {
        let provenance = RuntimeProvenance::detect(&base_dir);
        let mut warnings = VecDeque::new();
        if let Some(warning) = provenance.privilege_warning() {
            push_startup_warning(
                &mut persistence,
                &mut warnings,
                0,
                clock.now_ms(),
                "admin_mode",
                warning.to_string(),
            );
        }
        let (settings, settings_rewrite_blocked) = match persistence.load_json_migrating(
            UserStorageComponent::Settings,
            clock.now_ms(),
            migrate_runtime_settings,
        ) {
            Ok(Some(load)) => {
                if load.migrated {
                    let event = serde_json::json!({
                        "ts_ms": clock.now_ms(),
                        "category": "persistence",
                        "payload": { "message": "settings schema migrated" },
                    });
                    if let DiagnosticWriteOutcome::Failed(failure) =
                        persistence.record_diagnostic(&event, clock.now_ms())
                    {
                        push_startup_persistence_failure(
                            &mut persistence,
                            &mut warnings,
                            0,
                            clock.now_ms(),
                            &failure,
                        );
                    }
                }
                (load.value, false)
            }
            Ok(None) => {
                let settings = RuntimeSettings::default();
                if let Err(failure) = persistence.write_json(
                    UserStorageComponent::Settings,
                    &settings,
                    clock.now_ms(),
                ) {
                    push_startup_persistence_failure(
                        &mut persistence,
                        &mut warnings,
                        0,
                        clock.now_ms(),
                        &failure,
                    );
                }
                (settings, false)
            }
            Err(failure) => {
                push_startup_persistence_failure(
                    &mut persistence,
                    &mut warnings,
                    0,
                    clock.now_ms(),
                    &failure,
                );
                (RuntimeSettings::default(), true)
            }
        };
        let mut warm_cache = match persistence
            .load_json::<WarmCache>(UserStorageComponent::WarmCache, clock.now_ms())
        {
            Ok(Some(cache)) => cache,
            Ok(None) => WarmCache {
                seq: 0,
                rows: Vec::new(),
            },
            Err(failure) => {
                push_startup_persistence_failure(
                    &mut persistence,
                    &mut warnings,
                    0,
                    clock.now_ms(),
                    &failure,
                );
                WarmCache {
                    seq: 0,
                    rows: Vec::new(),
                }
            }
        };
        let publication_seq = warm_cache.seq;
        let settings = normalize_settings(settings);
        warm_cache.rows = hold_process_rates(warm_cache.rows);
        let admin_mode = provenance.admin_mode_status();
        let engine_state = if settings.paused {
            RuntimeEngineState::Paused
        } else {
            RuntimeEngineState::Starting
        };
        let persistence_health = persistence.health();
        let persistence_degraded = persistence_health.state != RuntimePersistenceState::Healthy;
        let initial_health = RuntimeHealth {
            engine_state: Some(engine_state),
            degraded: persistence_degraded,
            status_summary: if persistence_health.state == RuntimePersistenceState::Unavailable {
                "Local persistence is unavailable; monitoring is starting with session-only state."
                    .to_string()
            } else if persistence_degraded {
                "Local persistence is degraded; monitoring is starting with visible state loss risk."
                    .to_string()
            } else {
                "Runtime starting.".to_string()
            },
            ..RuntimeHealth::default()
        };
        let health_window = metric_window(settings.metric_window_seconds);
        let durable_query = settings.query.clone();
        let snapshot = build_snapshot(
            publication_seq,
            clock.now_ms(),
            0,
            None,
            provenance.environment(),
            &settings,
            &admin_mode,
            initial_health,
            Some(persistence_health),
            empty_system(),
            &warm_cache.rows,
            shape_rows(&warm_cache.rows, &settings.query),
            warnings.iter().cloned().collect(),
        );

        Self {
            clock,
            persistence,
            provenance,
            settings,
            durable_query,
            admin_mode,
            snapshot,
            warnings,
            previous_totals: None,
            previous_processes: warm_cache.rows,
            live_process_snapshot: false,
            tick_p95: P95Window::new(health_window),
            sort_p95: P95Window::new(health_window),
            publication_p95: P95Window::new(health_window),
            engine_state,
            collector_state: None,
            last_heartbeat_at_ms: None,
            deadline_misses: 0,
            recent_deadline_misses: 0,
            deadline_lateness_p95_ms: 0.0,
            collection_latency_ms: None,
            publication_latency_ms: None,
            fatal_error: None,
            publication_seq,
            sample_seq: 0,
            sampled_at_ms: None,
            last_sample_provenance: None,
            settings_rewrite_blocked,
            persistence_flushed: false,
        }
    }

    #[cfg(test)]
    fn has_process_exe(&self, exe: &str) -> bool {
        let exe = exe.trim();
        self.live_process_snapshot
            && !exe.is_empty()
            && self
                .snapshot
                .processes
                .iter()
                .any(|process| process.exe.eq_ignore_ascii_case(exe))
    }

    fn set_paused(&mut self, paused: bool) -> RuntimeSnapshot {
        self.settings.paused = paused;
        if paused {
            self.live_process_snapshot = false;
        }
        let _ = self.persist_settings(SettingsWriteIntent::UserMutation);
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_query_with_intent(
        &mut self,
        query: RuntimeQuery,
        intent: QueryWriteIntent,
    ) -> RuntimeSnapshot {
        let query = normalize_query(query);
        self.settings.query = query.clone();
        if intent == QueryWriteIntent::UserMutation {
            self.durable_query = query;
            let _ = self.persist_settings(SettingsWriteIntent::UserMutation);
        }
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_sample_interval(&mut self, sample_interval_ms: u32) -> RuntimeSnapshot {
        self.settings.sample_interval_ms = sample_interval_ms.clamp(500, 5_000);
        let _ = self.persist_settings(SettingsWriteIntent::UserMutation);
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_ui_preferences(&mut self, preferences: RuntimeUiPreferences) -> RuntimeSnapshot {
        self.settings.ui_preferences = Some(preferences);
        let _ = self.persist_settings(SettingsWriteIntent::UserMutation);
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn apply_raw_sample(
        &mut self,
        sample: crate::telemetry::TelemetrySample,
        raw_collection_latency_ms: f64,
        completed_at_ms: u64,
    ) -> Instant {
        let processing_started = Instant::now();
        let source_provenance = sample.source_provenance.clone();
        // Service and desktop clocks have independent wall-time anchors. Keep the service time for
        // source deltas, but publish freshness only in the desktop clock domain.
        let source_sample_ts_ms = source_provenance
            .as_ref()
            .map_or(completed_at_ms, |provenance| provenance.sampled_at_ms);
        let source_unchanged = source_provenance.is_some()
            && source_provenance.as_ref() == self.last_sample_provenance.as_ref();
        let service_status_ts_ms = if source_unchanged {
            self.sampled_at_ms.unwrap_or(completed_at_ms)
        } else {
            completed_at_ms
        };
        let service_active = self
            .apply_collector_service_status(sample.collector_service.clone(), service_status_ts_ms);
        if source_unchanged {
            self.sync_collector_warnings(sample.warnings);
            self.record_collection_latency_ms(
                raw_collection_latency_ms + processing_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let publication_started = Instant::now();
            self.publish_snapshot_only(None);
            return publication_started;
        }
        self.last_sample_provenance = source_provenance;
        let previous_process_baseline_live = self.live_process_snapshot;
        self.live_process_snapshot = false;
        self.collector_state = Some(sample.collector_state);

        let active_collector_warnings = sample.warnings;

        let elapsed_seconds = self
            .previous_totals
            .as_ref()
            .and_then(|previous| {
                let delta_ms = source_sample_ts_ms.saturating_sub(previous.ts_ms);
                (delta_ms > 0).then_some(delta_ms as f64 / 1000.0)
            })
            .unwrap_or(1.0)
            .max(0.5);
        let mut system = sample.system;
        let sample_processes = sample.processes;
        let process_rows_fresh = true;
        if !service_active {
            if self.admin_mode.source == RuntimePrivilegedSource::CurrentProcess
                && self.admin_mode.state == RuntimeAdminModeState::Active
            {
                self.admin_mode.last_success_at_ms = Some(completed_at_ms);
            }
        }
        self.sync_collector_warnings(active_collector_warnings);
        let disk_source = system
            .quality
            .as_ref()
            .and_then(|quality| quality.disk.as_ref())
            .and_then(|quality| quality.source);
        if disk_source == Some(MetricSource::Iokit) {
            derive_iokit_disk_rates(&mut system, self.previous_totals.as_ref(), elapsed_seconds);
        } else if let Some(previous) = &self.previous_totals {
            let disk_rates_are_native = disk_source == Some(MetricSource::Procfs);
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
        }
        if let Some(previous) = &self.previous_totals {
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
        self.previous_totals = Some(TelemetryTotals::from_system(&system, source_sample_ts_ms));
        self.publication_seq = self.publication_seq.saturating_add(1);
        self.sample_seq = self.sample_seq.saturating_add(1);
        self.sampled_at_ms = Some(completed_at_ms);

        let sort_started = Instant::now();
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        let sort_ms = sort_started.elapsed().as_secs_f64() * 1000.0;
        self.sort_p95.add(sort_ms);

        let app_metrics = current_app_metrics(&self.previous_processes);
        if self
            .sample_seq
            .is_multiple_of(WARM_CACHE_WRITE_INTERVAL_TICKS)
        {
            let _ = self.persist_warm_cache();
        }
        self.record_collection_latency_ms(
            raw_collection_latency_ms + processing_started.elapsed().as_secs_f64() * 1000.0,
        );
        let publication_started = Instant::now();
        let health = self.build_health(
            sample.latency_ms,
            app_metrics.cpu_percent,
            app_metrics.rss_bytes,
        );
        self.snapshot = build_snapshot(
            self.publication_seq,
            self.clock.now_ms(),
            self.sample_seq,
            self.sampled_at_ms,
            self.provenance.environment(),
            &self.settings,
            &self.admin_mode,
            health,
            Some(self.persistence.health()),
            system,
            &self.previous_processes,
            rows,
            self.warnings.iter().cloned().collect(),
        );

        publication_started
    }

    fn apply_collector_service_status(
        &mut self,
        status: Option<crate::contracts::RuntimeCollectorServiceStatus>,
        sample_ts_ms: u64,
    ) -> bool {
        let Some(status) = status else {
            return false;
        };
        let previous_binding = self
            .admin_mode
            .collector_service
            .as_ref()
            .filter(|service| service.state == RuntimeCollectorServiceState::Active)
            .and_then(|service| service.instance_id.clone());
        let next_binding = (status.state == RuntimeCollectorServiceState::Active)
            .then(|| status.instance_id.clone())
            .flatten();
        if previous_binding != next_binding {
            self.previous_totals = None;
            self.previous_processes.clear();
            self.live_process_snapshot = false;
        }

        let service_active = status.state == RuntimeCollectorServiceState::Active;
        if service_active {
            self.admin_mode.state = RuntimeAdminModeState::Active;
            self.admin_mode.source = RuntimePrivilegedSource::CollectorService;
            self.admin_mode.detail = None;
            self.admin_mode.last_success_at_ms = Some(sample_ts_ms);
        } else if self.admin_mode.source == RuntimePrivilegedSource::CollectorService
            || self.admin_mode.source == RuntimePrivilegedSource::None
        {
            let last_success_at_ms = self.admin_mode.last_success_at_ms;
            self.admin_mode = self.provenance.admin_mode_status();
            self.admin_mode.detail = status.detail.clone();
            self.admin_mode.last_success_at_ms = last_success_at_ms;
        }
        self.admin_mode.collector_service = Some(status);
        service_active
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
            self.clock.now_ms(),
            self.sample_seq,
            self.sampled_at_ms,
            self.provenance.environment(),
            &self.settings,
            &self.admin_mode,
            health,
            Some(self.persistence.health()),
            self.snapshot.system.clone(),
            &self.previous_processes,
            rows,
            self.warnings.iter().cloned().collect(),
        );
    }

    fn set_engine_state(&mut self, state: RuntimeEngineState) {
        self.engine_state = state;
        if state != RuntimeEngineState::Fatal {
            self.fatal_error = None;
        }
    }

    fn note_heartbeat(&mut self) {
        self.note_heartbeat_at(self.clock.now_ms());
    }

    fn note_heartbeat_at(&mut self, occurred_at_ms: u64) {
        self.last_heartbeat_at_ms = Some(occurred_at_ms);
    }

    fn update_schedule_health(&mut self, cadence: CollectorCadence) {
        self.deadline_misses = cadence.deadline_misses;
        self.recent_deadline_misses = cadence.recent_deadline_misses;
        self.deadline_lateness_p95_ms = cadence.deadline_lateness_p95_ms;
        self.refresh_snapshot_health();
    }

    fn record_collection_latency_ms(&mut self, latency_ms: f64) {
        self.collection_latency_ms = Some(latency_ms);
        self.tick_p95.add_at(Instant::now(), latency_ms);
    }

    fn record_publication_latency(&mut self, elapsed: Duration) {
        let latency_ms = elapsed.as_secs_f64() * 1000.0;
        self.publication_latency_ms = Some(latency_ms);
        self.publication_p95.add_at(Instant::now(), latency_ms);
        self.refresh_snapshot_health();
    }

    fn publish_collector_unavailable(&mut self, error: String) {
        self.collector_state = Some(RuntimeCollectorState::Unavailable);
        self.live_process_snapshot = false;
        self.publish_snapshot_only(Some(("collector", error)));
    }

    fn mark_engine_fatal(&mut self, code: &str, message: String) {
        let occurred_at_ms = self.clock.now_ms();
        self.engine_state = RuntimeEngineState::Fatal;
        self.collector_state = Some(RuntimeCollectorState::Unavailable);
        self.live_process_snapshot = false;
        self.fatal_error = Some(RuntimeFatalError {
            code: code.to_string(),
            message: message.clone(),
            occurred_at_ms,
        });
        self.publish_snapshot_only(Some(("collector", message)));
    }

    fn refresh_snapshot_health(&mut self) {
        let app_metrics = current_app_metrics(&self.previous_processes);
        self.snapshot.health = self.build_health(
            self.collection_latency_ms.unwrap_or_default().round() as u64,
            app_metrics.cpu_percent,
            app_metrics.rss_bytes,
        );
        self.snapshot.persistence = Some(self.persistence.health());
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
        let collector_warning_count = self
            .warnings
            .iter()
            .filter(|warning| warning.category == "collector")
            .count();
        let collector_degraded = matches!(
            self.collector_state,
            Some(RuntimeCollectorState::Limited | RuntimeCollectorState::Unavailable)
        );
        let fatal = self.engine_state == RuntimeEngineState::Fatal;
        let cadence_degraded = self.recent_deadline_misses > 0;
        let persistence_state = self.persistence.health().state;
        let persistence_degraded = persistence_state != RuntimePersistenceState::Healthy;
        let last_warning = self.warnings.back().map(|warning| warning.message.clone());
        let status_summary = if fatal {
            "Sampling engine stopped after a fatal error.".to_string()
        } else if self.settings.paused {
            "Paused.".to_string()
        } else if self.admin_mode.state == RuntimeAdminModeState::Requesting {
            "Waiting for Windows approval.".to_string()
        } else if self.admin_mode.state == RuntimeAdminModeState::Recovering {
            "Privileged collection is recovering; standard monitoring remains current.".to_string()
        } else if self.collector_state == Some(RuntimeCollectorState::Unavailable) {
            "Collector unavailable; retaining the last published sample.".to_string()
        } else if self.collector_state == Some(RuntimeCollectorState::Limited) {
            "Collecting with limited telemetry quality.".to_string()
        } else if persistence_state == RuntimePersistenceState::Unavailable {
            "Local persistence is unavailable; monitoring continues with session-only state."
                .to_string()
        } else if persistence_degraded {
            "Local persistence is degraded; monitoring continues with visible state loss risk."
                .to_string()
        } else if cadence_degraded {
            format!(
                "Sampling missed {} deadline(s) in the current health window.",
                self.recent_deadline_misses
            )
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
            degraded: fatal
                || collector_degraded
                || cadence_degraded
                || cpu_degraded
                || rss_degraded
                || warning_degraded
                || persistence_degraded,
            collector_warnings: collector_warning_count,
            runtime_loop_enabled: true,
            runtime_loop_running: self.engine_state == RuntimeEngineState::Running,
            status_summary,
            updated_at_ms: self.clock.now_ms(),
            tick_p95_ms: round1(self.tick_p95.value()),
            sort_p95_ms: round1(self.sort_p95.value()),
            jitter_p95_ms: round1(self.deadline_lateness_p95_ms),
            dropped_ticks: self.deadline_misses,
            app_cpu_percent: round1(app_cpu_percent),
            app_rss_bytes,
            last_warning,
            engine_state: Some(self.engine_state),
            collector_state: self.collector_state,
            last_heartbeat_at_ms: self.last_heartbeat_at_ms,
            deadline_misses: Some(self.deadline_misses),
            deadline_lateness_p95_ms: Some(round1(self.deadline_lateness_p95_ms)),
            collection_latency_ms: self.collection_latency_ms.map(round1),
            collection_p95_ms: Some(round1(self.tick_p95.value())),
            publication_latency_ms: self.publication_latency_ms.map(round1),
            publication_p95_ms: Some(round1(self.publication_p95.value())),
            fatal_error: self.fatal_error.clone(),
        }
    }

    fn add_warning(&mut self, category: &str, message: String) {
        if self.upsert_warning(category, &message) {
            self.append_diagnostic(category, &message);
        }
    }

    fn upsert_warning(&mut self, category: &str, message: &str) -> bool {
        let key = warning_key(category, message);
        if self
            .warnings
            .iter()
            .any(|warning| warning.key == key && warning.message == message)
        {
            return false;
        }
        if let Some(index) = self.warnings.iter().position(|warning| warning.key == key) {
            self.warnings.remove(index);
        }
        push_warning(
            &mut self.warnings,
            self.publication_seq,
            self.clock.now_ms(),
            category,
            message.to_string(),
        );
        true
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

    fn persist_settings(&mut self, intent: SettingsWriteIntent) -> Result<(), String> {
        if intent == SettingsWriteIntent::UserMutation {
            self.settings_rewrite_blocked = false;
        } else if self.settings_rewrite_blocked {
            return Ok(());
        }
        let mut persisted = self.settings.clone();
        persisted.query = self.durable_query.clone();
        match self.persistence.write_json(
            UserStorageComponent::Settings,
            &persisted,
            self.clock.now_ms(),
        ) {
            Ok(()) => {
                self.persistence.retry_diagnostics();
                let event = serde_json::json!({
                    "ts_ms": self.clock.now_ms(),
                    "category": "persistence",
                    "payload": { "message": "settings persisted" },
                });
                let _ = self
                    .persistence
                    .record_diagnostic(&event, self.clock.now_ms());
                self.clear_persistence_warnings_if_healthy();
                Ok(())
            }
            Err(failure) => {
                let message = RuntimePersistenceCoordinator::failure_message(&failure);
                self.add_warning("persistence", message.clone());
                Err(message)
            }
        }
    }

    fn persist_warm_cache(&mut self) -> Result<(), String> {
        if self.admin_mode.state == RuntimeAdminModeState::Active {
            return Ok(());
        }
        let cache = WarmCache {
            seq: self.publication_seq,
            rows: self.previous_processes.clone(),
        };
        match self.persistence.write_json(
            UserStorageComponent::WarmCache,
            &cache,
            self.clock.now_ms(),
        ) {
            Ok(()) => {
                self.clear_persistence_warnings_if_healthy();
                Ok(())
            }
            Err(failure) => {
                let message = RuntimePersistenceCoordinator::failure_message(&failure);
                self.add_warning("persistence", message.clone());
                Err(message)
            }
        }
    }

    fn purge_warm_cache(&mut self) -> Result<(), String> {
        match self
            .persistence
            .remove(UserStorageComponent::WarmCache, self.clock.now_ms())
        {
            Ok(()) => {
                self.clear_persistence_warnings_if_healthy();
                Ok(())
            }
            Err(failure) => {
                let message = RuntimePersistenceCoordinator::failure_message(&failure);
                self.add_warning("persistence", message.clone());
                Err(message)
            }
        }
    }

    fn append_diagnostic(&mut self, category: &str, message: &str) {
        let payload = serde_json::json!({
            "ts_ms": self.clock.now_ms(),
            "category": category,
            "payload": { "message": message },
        });
        if let DiagnosticWriteOutcome::Failed(failure) = self
            .persistence
            .record_diagnostic(&payload, self.clock.now_ms())
        {
            let message = RuntimePersistenceCoordinator::failure_message(&failure);
            self.upsert_warning("persistence", &message);
        }
    }

    fn clear_persistence_warnings_if_healthy(&mut self) {
        if self.persistence.health().state != RuntimePersistenceState::Healthy {
            return;
        }
        let recovered = self
            .warnings
            .iter()
            .filter(|warning| warning.category == "persistence")
            .map(|warning| warning.key.clone())
            .collect::<Vec<_>>();
        for key in recovered {
            self.clear_warning(&key);
        }
    }

    fn shutdown_owned_resources(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if !self.persistence_flushed {
            self.persistence_flushed = true;
            if let Err(error) = self.persist_settings(SettingsWriteIntent::Automatic) {
                errors.push(error);
            }
            let cache_result = if self.admin_mode.state == RuntimeAdminModeState::Active {
                self.purge_warm_cache()
            } else {
                self.persist_warm_cache()
            };
            if let Err(error) = cache_result {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "runtime_resource_cleanup_failed:{}",
                errors.join(";")
            ))
        }
    }
}

impl Drop for RuntimeStore {
    fn drop(&mut self) {
        let _ = self.shutdown_owned_resources();
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
    disk_quality: Option<MetricQualityInfo>,
    network_received_total_bytes: u64,
    network_transmitted_total_bytes: u64,
}

impl TelemetryTotals {
    fn from_system(system: &SystemMetricsSnapshot, ts_ms: u64) -> Self {
        Self {
            ts_ms,
            disk_read_total_bytes: system.disk_read_total_bytes,
            disk_write_total_bytes: system.disk_write_total_bytes,
            disk_quality: system
                .quality
                .as_ref()
                .and_then(|quality| quality.disk.clone()),
            network_received_total_bytes: system.network_received_total_bytes,
            network_transmitted_total_bytes: system.network_transmitted_total_bytes,
        }
    }
}

struct P95Window {
    values: VecDeque<(Instant, f64)>,
    window: Duration,
}

#[derive(Debug, Clone, Copy, Default)]
struct CurrentAppMetrics {
    cpu_percent: f64,
    rss_bytes: u64,
}

impl P95Window {
    fn new(window: Duration) -> Self {
        Self {
            values: VecDeque::new(),
            window,
        }
    }

    fn add(&mut self, value: f64) {
        self.add_at(Instant::now(), value);
    }

    fn add_at(&mut self, now: Instant, value: f64) {
        self.values.push_back((now, value.max(0.0)));
        while self.values.front().is_some_and(|(recorded_at, _)| {
            now.saturating_duration_since(*recorded_at) > self.window
        }) {
            self.values.pop_front();
        }
    }

    fn value(&self) -> f64 {
        self.value_at(Instant::now())
    }

    fn value_at(&self, now: Instant) -> f64 {
        let mut values = self
            .values
            .iter()
            .filter(|(recorded_at, _)| now.saturating_duration_since(*recorded_at) <= self.window)
            .map(|(_, value)| *value)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return 0.0;
        }

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
    persistence: Option<RuntimePersistence>,
    system: SystemMetricsSnapshot,
    all_processes: &[ProcessSample],
    processes: Vec<ProcessSample>,
    warnings: Vec<RuntimeWarning>,
) -> RuntimeSnapshot {
    let process_view_rows = shape_process_view(all_processes, &settings.query);
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
        persistence,
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
            cumulative_baseline_is_compatible(current_io_quality.as_ref(), previous_io_quality)
                && process.io_read_total_bytes >= previous.io_read_total_bytes
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
        pending.limitation_code = Some(MetricLimitationCode::PendingBaseline);
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
        pending.limitation_code = Some(MetricLimitationCode::PendingBaseline);
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

fn metric_is_pending_baseline(quality: &MetricQualityInfo) -> bool {
    // These Held markers are written only after a valid cumulative sample, so that sample may
    // become the baseline without treating collector-held or unavailable rows as counters.
    quality.quality == MetricQuality::Held
        && quality.limitation_code == Some(MetricLimitationCode::PendingBaseline)
}

fn cumulative_baseline_is_compatible(
    current: Option<&MetricQualityInfo>,
    previous: Option<&MetricQualityInfo>,
) -> bool {
    let (Some(current), Some(previous)) = (current, previous) else {
        return false;
    };
    let sources_are_compatible = match (current.source, previous.source) {
        (Some(current), Some(previous)) => current == previous,
        _ => false,
    };
    cumulative_sample_is_valid(Some(current))
        && (cumulative_sample_is_valid(Some(previous)) || metric_is_pending_baseline(previous))
        && sources_are_compatible
}

fn derive_iokit_disk_rates(
    system: &mut SystemMetricsSnapshot,
    previous: Option<&TelemetryTotals>,
    elapsed_seconds: f64,
) {
    let current_quality = system
        .quality
        .as_ref()
        .and_then(|quality| quality.disk.as_ref())
        .cloned();
    let baseline_is_valid = previous.is_some_and(|previous| {
        cumulative_baseline_is_compatible(current_quality.as_ref(), previous.disk_quality.as_ref())
            && system.disk_read_total_bytes >= previous.disk_read_total_bytes
            && system.disk_write_total_bytes >= previous.disk_write_total_bytes
    });

    if let Some(previous) = previous.filter(|_| baseline_is_valid) {
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
        return;
    }

    system.disk_read_bps = 0;
    system.disk_write_bps = 0;
    let Some(current) = current_quality.filter(|quality| {
        matches!(
            quality.quality,
            MetricQuality::Native | MetricQuality::Estimated | MetricQuality::Partial
        )
    }) else {
        return;
    };
    let mut pending = current;
    pending.quality = MetricQuality::Held;
    pending.limitation_code = Some(MetricLimitationCode::PendingBaseline);
    pending.message = Some(
        "Waiting for a stable IOKit physical-device counter baseline before deriving disk rates."
            .to_string(),
    );
    if let Some(quality) = system.quality.as_mut() {
        quality.disk = Some(pending);
    }
}

fn shape_rows(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessSample> {
    let mut rows = rank_processes(processes, query);
    rows.truncate(query.limit.max(1));
    rows
}

fn rank_processes(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessSample> {
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
    presentation_process: ProcessSample,
    processes: Vec<ProcessSample>,
    cpu_percent: f64,
    memory_bytes: u64,
    io_bps: u64,
    network_bps: u64,
    threads: u64,
}

fn shape_process_view(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessViewRow> {
    let processes = rank_processes(processes, query);
    let mut groups = Vec::<ProcessAppGroup>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for process in &processes {
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
                presentation_process: process.clone(),
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
        if group_metric_available(process, GroupMetric::Cpu) {
            group.cpu_percent += process.cpu_percent;
        }
        if group_metric_available(process, GroupMetric::Memory) {
            group.memory_bytes = group.memory_bytes.saturating_add(process.memory_bytes);
        }
        if group_metric_available(process, GroupMetric::Io) {
            group.io_bps = group.io_bps.saturating_add(process_io_rate(process));
        }
        if group_metric_available(process, GroupMetric::Network) {
            group.network_bps = group
                .network_bps
                .saturating_add(process_network_rate(process));
        }
        if group_metric_available(process, GroupMetric::Threads) {
            group.threads = group.threads.saturating_add(process.threads as u64);
        }
        group.processes.push(process.clone());
    }

    groups.sort_by(|left, right| compare_process_group(left, right, query));

    let mut rows = Vec::with_capacity(processes.len() + groups.len());
    for group in groups.into_iter().take(query.limit.max(1)) {
        let grouped = group.processes.len() > 1;
        let group_count = group.processes.len();
        if grouped {
            let detail = group_detail(&group);
            rows.push(ProcessViewRow::Group {
                attention_label: group_attention_label(
                    &detail,
                    group
                        .processes
                        .iter()
                        .any(|process| process.access_state != AccessState::Full),
                ),
                detail: Box::new(detail),
                icon_kind: group.icon_kind.clone(),
                icon_source: (!group.presentation_process.exe.trim().is_empty())
                    .then(|| group.presentation_process.exe.clone()),
                example_label: (group.presentation_process.name != group.label)
                    .then(|| group.presentation_process.name.clone()),
            });
        }

        for process in group.processes {
            let identity = process_identity(&process);
            rows.push(ProcessViewRow::Process {
                detail: Box::new(ProcessDetail {
                    kind: ProcessDetailKind::Process,
                    workload_id: process_workload_id(&process),
                    io_bps: process_io_rate(&process),
                    network_bps: process_network_rate(&process),
                    process: process.clone(),
                }),
                group_key: group.key.clone(),
                group_label: group.label.clone(),
                group_category: group.category.clone(),
                group_count,
                icon_kind: if grouped {
                    group.icon_kind.clone()
                } else {
                    identity.icon_kind.to_string()
                },
                is_child: grouped && identity.is_child,
                is_grouped: grouped,
                attention_label: process_attention_label(&process),
            });
        }
    }

    rows
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupMetric {
    Cpu,
    Memory,
    Io,
    OtherIo,
    Network,
    Threads,
}

fn group_detail(group: &ProcessAppGroup) -> GroupDetail {
    let (cpu_quality, cpu_coverage) = group_metric_summary(&group.processes, GroupMetric::Cpu);
    let (memory_quality, memory_coverage) =
        group_metric_summary(&group.processes, GroupMetric::Memory);
    let (io_quality, io_coverage) = group_metric_summary(&group.processes, GroupMetric::Io);
    let (other_io_quality, other_io_coverage) =
        group_metric_summary(&group.processes, GroupMetric::OtherIo);
    let (network_quality, network_coverage) =
        group_metric_summary(&group.processes, GroupMetric::Network);
    let (threads_quality, threads_coverage) =
        group_metric_summary(&group.processes, GroupMetric::Threads);

    GroupDetail {
        kind: GroupDetailKind::Group,
        workload_id: group_workload_id(&group.key),
        group_key: group.key.clone(),
        label: group.label.clone(),
        category: group.category.clone(),
        process_count: group.processes.len(),
        cpu_percent: round1(group.cpu_percent),
        memory_bytes: group.memory_bytes,
        io_bps: group.io_bps,
        other_io_bps: None,
        network_bps: group.network_bps,
        threads: group.threads,
        quality: GroupMetricQuality {
            cpu: cpu_quality,
            memory: memory_quality,
            io: io_quality,
            other_io: other_io_quality,
            network: network_quality,
            threads: threads_quality,
        },
        coverage: GroupMetricCoverage {
            cpu: cpu_coverage,
            memory: memory_coverage,
            io: io_coverage,
            other_io: other_io_coverage,
            network: network_coverage,
            threads: threads_coverage,
        },
    }
}

fn group_metric_summary(
    processes: &[ProcessSample],
    metric: GroupMetric,
) -> (MetricQualityInfo, MetricCoverage) {
    let total = processes.len();
    let available = processes
        .iter()
        .filter(|process| group_metric_available(process, metric))
        .count();
    let coverage = MetricCoverage { available, total };
    let available_quality = processes
        .iter()
        .filter(|process| group_metric_available(process, metric))
        .filter_map(|process| group_metric_quality(process, metric))
        .map(|quality| quality.quality)
        .collect::<Vec<_>>();

    let reported_quality = processes
        .iter()
        .filter_map(|process| group_metric_quality(process, metric))
        .map(|quality| quality.quality)
        .collect::<Vec<_>>();
    let missing_quality = total.saturating_sub(reported_quality.len());
    let value_count = processes
        .iter()
        .filter(|process| group_metric_has_value(process, metric))
        .count();

    let includes_partial_quality = available_quality.contains(&MetricQuality::Partial);
    let quality = if metric == GroupMetric::OtherIo || value_count == 0 {
        MetricQuality::Unavailable
    } else if available == 0 {
        if missing_quality > 0 {
            MetricQuality::Partial
        } else if reported_quality
            .iter()
            .all(|quality| *quality == MetricQuality::Held)
        {
            MetricQuality::Held
        } else if reported_quality
            .iter()
            .all(|quality| *quality == MetricQuality::Unavailable)
        {
            MetricQuality::Unavailable
        } else {
            MetricQuality::Partial
        }
    } else if available < total || includes_partial_quality {
        MetricQuality::Partial
    } else if available_quality.contains(&MetricQuality::Estimated) {
        MetricQuality::Estimated
    } else {
        MetricQuality::Native
    };

    let limitation_code = if available < total {
        Some(MetricLimitationCode::GroupPartialCoverage)
    } else if includes_partial_quality {
        Some(MetricLimitationCode::PartialCoverage)
    } else {
        None
    };
    let message = if available < total {
        Some(format!(
            "{available} of {total} processes contribute to this aggregate."
        ))
    } else if includes_partial_quality {
        Some("At least one process contributes partial-quality data.".to_string())
    } else {
        None
    };
    (
        MetricQualityInfo {
            quality,
            source: Some(MetricSource::ProcessAggregate),
            updated_at_ms: None,
            age_ms: None,
            limitation_code,
            message,
        },
        coverage,
    )
}

fn group_metric_available(process: &ProcessSample, metric: GroupMetric) -> bool {
    group_metric_has_value(process, metric)
        && group_metric_quality(process, metric).is_some_and(|quality| {
            !matches!(
                quality.quality,
                MetricQuality::Unavailable | MetricQuality::Held
            )
        })
}

fn group_metric_has_value(process: &ProcessSample, metric: GroupMetric) -> bool {
    match metric {
        GroupMetric::OtherIo => false,
        GroupMetric::Network => {
            process.network_received_bps.is_some() || process.network_transmitted_bps.is_some()
        }
        GroupMetric::Cpu | GroupMetric::Memory | GroupMetric::Io | GroupMetric::Threads => true,
    }
}

fn group_metric_quality(
    process: &ProcessSample,
    metric: GroupMetric,
) -> Option<&MetricQualityInfo> {
    let quality = process.quality.as_ref()?;
    match metric {
        GroupMetric::Cpu => quality.cpu.as_ref(),
        GroupMetric::Memory => quality.memory.as_ref(),
        GroupMetric::Io => quality.io.as_ref(),
        GroupMetric::OtherIo => quality.other_io.as_ref(),
        GroupMetric::Network => quality.network.as_ref(),
        GroupMetric::Threads => quality.threads.as_ref(),
    }
}

fn process_workload_id(process: &ProcessSample) -> String {
    format!("process:{}:{}", process.pid, process.start_time_ms)
}

fn group_workload_id(group_key: &str) -> String {
    format!("group:{group_key}")
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
        SortColumn::Pid => left.key.cmp(&right.key),
        SortColumn::MemoryBytes => {
            group_memory_sort_value(left).cmp(&group_memory_sort_value(right))
        }
        SortColumn::IoBps => group_io_sort_value(left).cmp(&group_io_sort_value(right)),
        SortColumn::NetworkBps => {
            group_network_sort_value(left).cmp(&group_network_sort_value(right))
        }
        SortColumn::Threads => group_threads_sort_value(left).cmp(&group_threads_sort_value(right)),
        SortColumn::Handles | SortColumn::StartTimeMs => left.key.cmp(&right.key),
        SortColumn::Attention => group_attention_score(left)
            .partial_cmp(&group_attention_score(right))
            .unwrap_or(Ordering::Equal),
        SortColumn::CpuPct => group_cpu_sort_value(left)
            .partial_cmp(&group_cpu_sort_value(right))
            .unwrap_or(Ordering::Equal),
    };

    let directed = match query.sort_direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    directed.then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
}

fn group_cpu_sort_value(group: &ProcessAppGroup) -> f64 {
    group
        .processes
        .first()
        .filter(|_| group.processes.len() == 1)
        .map_or(group.cpu_percent, |process| process.cpu_percent)
}

fn group_memory_sort_value(group: &ProcessAppGroup) -> u64 {
    group
        .processes
        .first()
        .filter(|_| group.processes.len() == 1)
        .map_or(group.memory_bytes, |process| process.memory_bytes)
}

fn group_io_sort_value(group: &ProcessAppGroup) -> u64 {
    group
        .processes
        .first()
        .filter(|_| group.processes.len() == 1)
        .map_or(group.io_bps, process_io_rate)
}

fn group_network_sort_value(group: &ProcessAppGroup) -> u64 {
    group
        .processes
        .first()
        .filter(|_| group.processes.len() == 1)
        .map_or(group.network_bps, process_network_rate)
}

fn group_threads_sort_value(group: &ProcessAppGroup) -> u64 {
    group
        .processes
        .first()
        .filter(|_| group.processes.len() == 1)
        .map_or(group.threads, |process| process.threads as u64)
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
        cpu_identity: cpu.identity,
        cpu_coverage: Some(cpu.coverage),
        cpu_quality: cpu.quality,
        cpu_name_ambiguous: cpu.ambiguous,
        memory: memory.name,
        memory_identity: memory.identity,
        memory_coverage: Some(memory.coverage),
        memory_quality: memory.quality,
        memory_name_ambiguous: memory.ambiguous,
        io: io.name,
        io_identity: io.identity,
        io_coverage: Some(io.coverage),
        io_quality: io.quality,
        io_name_ambiguous: io.ambiguous,
        network: network.name,
        network_identity: network.identity,
        network_coverage: Some(network.coverage),
        network_quality: network.quality,
        network_name_ambiguous: network.ambiguous,
    }
}

#[cfg(test)]
pub(crate) fn shape_protocol_fixture_snapshot(snapshot: &mut RuntimeSnapshot) {
    snapshot.process_view_rows = shape_process_view(&snapshot.processes, &snapshot.settings.query);
    snapshot.process_contributors = summarize_process_contributors(&snapshot.processes);
    snapshot.total_process_count = snapshot.processes.len();
    snapshot.system.process_count = snapshot.processes.len();
    if let Some(accounting) = &mut snapshot.system.memory_accounting {
        accounting.process_working_set_bytes = snapshot
            .processes
            .iter()
            .map(|process| process.memory_bytes)
            .sum();
        accounting.process_private_bytes = snapshot
            .processes
            .iter()
            .map(|process| process.private_bytes)
            .sum();
    }
}

struct ProcessContributor {
    name: Option<String>,
    identity: Option<ProcessContributorIdentity>,
    coverage: MetricCoverage,
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
    let available = processes
        .iter()
        .filter(|process| {
            quality(process)
                .is_some_and(|quality| contributor_quality_is_publishable(Some(quality)))
        })
        .count();
    let coverage = MetricCoverage {
        available,
        total: processes.len(),
    };
    let coverage_is_publishable = coverage.available == coverage.total;
    let winner = processes
        .iter()
        .filter(|process| {
            quality(process)
                .is_some_and(|quality| contributor_quality_is_publishable(Some(quality)))
        })
        .max_by(|left, right| {
            metric(left)
                .partial_cmp(&metric(right))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(process) = winner.filter(|process| metric(process) > T::default()) {
        if !coverage_is_publishable {
            return ProcessContributor {
                name: None,
                identity: None,
                coverage,
                quality: coverage_quality,
                ambiguous: false,
            };
        }
        return ProcessContributor {
            name: Some(process.name.clone()),
            identity: Some(ProcessContributorIdentity {
                pid: process.pid.clone(),
                start_time_ms: process.start_time_ms,
            }),
            coverage,
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
        identity: None,
        coverage,
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
    let process_name = normalized_process_name(&process.name);
    let key = if !process_name.trim().is_empty() {
        process_name
    } else {
        format!("pid:{}", process.pid)
    };

    key.to_lowercase()
}

fn process_app_label(process: &ProcessSample) -> String {
    normalized_process_name(&process.name)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessAttentionMetricState {
    Native,
    Estimated,
    Partial,
    Held,
    Unavailable,
    Missing,
}

fn process_attention_metric_state(
    process: &ProcessSample,
    metric: GroupMetric,
) -> ProcessAttentionMetricState {
    let Some(quality) = group_metric_quality(process, metric) else {
        return ProcessAttentionMetricState::Missing;
    };
    match quality.quality {
        MetricQuality::Held => ProcessAttentionMetricState::Held,
        MetricQuality::Unavailable => ProcessAttentionMetricState::Unavailable,
        _ if !group_metric_has_value(process, metric) => ProcessAttentionMetricState::Unavailable,
        MetricQuality::Native => ProcessAttentionMetricState::Native,
        MetricQuality::Estimated => ProcessAttentionMetricState::Estimated,
        MetricQuality::Partial => ProcessAttentionMetricState::Partial,
    }
}

fn process_activity_attention_label(
    label: &str,
    state: ProcessAttentionMetricState,
    all_metrics_complete: bool,
) -> String {
    let mut qualifiers = Vec::new();
    if state == ProcessAttentionMetricState::Partial {
        qualifiers.push("limited");
    }
    if state == ProcessAttentionMetricState::Estimated {
        qualifiers.push("estimated");
    }
    if !all_metrics_complete && state != ProcessAttentionMetricState::Partial {
        qualifiers.push("telemetry limited");
    }

    if qualifiers.is_empty() {
        label.to_string()
    } else {
        format!("{label} · {}", qualifiers.join(" · "))
    }
}

fn process_attention_label(process: &ProcessSample) -> String {
    let metrics = [
        GroupMetric::Cpu,
        GroupMetric::Memory,
        GroupMetric::Io,
        GroupMetric::Network,
    ];
    let states = metrics.map(|metric| process_attention_metric_state(process, metric));
    let all_metrics_complete = states.iter().all(|state| {
        matches!(
            state,
            ProcessAttentionMetricState::Native | ProcessAttentionMetricState::Estimated
        )
    });
    let activities = [
        (process.cpu_percent >= ATTENTION_CPU_PERCENT, "CPU activity"),
        (
            process.memory_bytes >= ATTENTION_MEMORY_BYTES,
            "memory activity",
        ),
        (process_io_rate(process) >= ATTENTION_IO_BPS, "I/O activity"),
        (
            process_network_rate(process) >= ATTENTION_NETWORK_BPS,
            "network activity",
        ),
    ];

    for (index, (threshold_reached, label)) in activities.into_iter().enumerate() {
        let state = states[index];
        if threshold_reached
            && matches!(
                state,
                ProcessAttentionMetricState::Native
                    | ProcessAttentionMetricState::Estimated
                    | ProcessAttentionMetricState::Partial
            )
        {
            return process_activity_attention_label(label, state, all_metrics_complete);
        }
    }

    if states.contains(&ProcessAttentionMetricState::Unavailable) {
        return "Unavailable".to_string();
    }
    if states.contains(&ProcessAttentionMetricState::Held) {
        return "Pending".to_string();
    }
    if states.contains(&ProcessAttentionMetricState::Missing)
        || states.contains(&ProcessAttentionMetricState::Partial)
    {
        return "Limited".to_string();
    }
    if process.access_state != AccessState::Full {
        return "access limited".to_string();
    }
    if states.contains(&ProcessAttentionMetricState::Estimated) {
        return "steady · estimated".to_string();
    }
    "steady".to_string()
}

fn group_metric_can_display(quality: &MetricQualityInfo, coverage: MetricCoverage) -> bool {
    coverage.available > 0
        && !matches!(
            quality.quality,
            MetricQuality::Held | MetricQuality::Unavailable
        )
}

fn group_metric_is_complete(quality: &MetricQualityInfo, coverage: MetricCoverage) -> bool {
    group_metric_can_display(quality, coverage)
        && quality.quality != MetricQuality::Partial
        && coverage.available == coverage.total
}

fn group_activity_label(
    label: &str,
    quality: &MetricQualityInfo,
    coverage: MetricCoverage,
    all_metrics_complete: bool,
) -> String {
    if quality.quality == MetricQuality::Partial || coverage.available < coverage.total {
        return format!(
            "{label} · {}/{} · limited",
            coverage.available, coverage.total
        );
    }
    if !all_metrics_complete {
        return format!("{label} · telemetry limited");
    }
    if quality.quality == MetricQuality::Estimated {
        return format!("{label} · estimated");
    }
    label.to_string()
}

fn group_attention_label(detail: &GroupDetail, access_limited: bool) -> String {
    let metrics = [
        (&detail.quality.cpu, detail.coverage.cpu),
        (&detail.quality.memory, detail.coverage.memory),
        (&detail.quality.io, detail.coverage.io),
        (&detail.quality.network, detail.coverage.network),
    ];
    let all_metrics_complete = metrics
        .iter()
        .all(|(quality, coverage)| group_metric_is_complete(quality, *coverage));
    let activities = [
        (
            detail.cpu_percent >= ATTENTION_CPU_PERCENT,
            "CPU activity",
            &detail.quality.cpu,
            detail.coverage.cpu,
        ),
        (
            detail.memory_bytes >= ATTENTION_MEMORY_BYTES,
            "memory activity",
            &detail.quality.memory,
            detail.coverage.memory,
        ),
        (
            detail.io_bps >= ATTENTION_IO_BPS,
            "I/O activity",
            &detail.quality.io,
            detail.coverage.io,
        ),
        (
            detail.network_bps >= ATTENTION_NETWORK_BPS,
            "network activity",
            &detail.quality.network,
            detail.coverage.network,
        ),
    ];

    for (threshold_reached, label, quality, coverage) in activities {
        if threshold_reached && group_metric_can_display(quality, coverage) {
            return group_activity_label(label, quality, coverage, all_metrics_complete);
        }
    }

    if all_metrics_complete {
        return if access_limited {
            "access limited".to_string()
        } else {
            "steady".to_string()
        };
    }

    let state = if metrics
        .iter()
        .all(|(quality, _)| quality.quality == MetricQuality::Held)
    {
        "Pending"
    } else if metrics
        .iter()
        .all(|(quality, _)| quality.quality == MetricQuality::Unavailable)
    {
        "Unavailable"
    } else {
        "Limited"
    };
    let coverage = metrics
        .iter()
        .find(|(quality, coverage)| !group_metric_is_complete(quality, *coverage))
        .map(|(_, coverage)| *coverage)
        .unwrap_or(MetricCoverage {
            available: 0,
            total: detail.process_count,
        });
    format!(
        "{state} · {}/{} coverage",
        coverage.available, coverage.total
    )
}

fn normalize_settings(settings: RuntimeSettings) -> RuntimeSettings {
    RuntimeSettings {
        query: normalize_query(settings.query),
        metric_window_seconds: settings.metric_window_seconds.clamp(15, 600),
        sample_interval_ms: settings.sample_interval_ms.clamp(500, 5_000),
        paused: settings.paused,
        ui_preferences: settings.ui_preferences.filter(valid_ui_preferences),
    }
}

fn migrate_runtime_settings(
    value: serde_json::Value,
) -> Result<JsonMigration<RuntimeSettings>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "settings schema must be a JSON object".to_string())?;
    let retires_admin_mode =
        object.contains_key("admin_mode_requested") || object.contains_key("admin_mode_enabled");
    let unknown_field = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "query"
                | "admin_mode_requested"
                | "admin_mode_enabled"
                | "metric_window_seconds"
                | "sample_interval_ms"
                | "paused"
                | "ui_preferences"
                | "theme"
                | "history_point_limit"
        )
    });
    if let Some(field) = unknown_field {
        return Err(format!("settings schema contains unknown field `{field}`"));
    }
    for (section, allowed) in [
        (
            "query",
            &[
                "filter_text",
                "focus_mode",
                "sort_column",
                "sort_direction",
                "limit",
            ][..],
        ),
        ("ui_preferences", &["theme", "history_point_limit"][..]),
    ] {
        if let Some(fields) = object.get(section).and_then(serde_json::Value::as_object) {
            if let Some(field) = fields
                .keys()
                .find(|field| !allowed.contains(&field.as_str()))
            {
                return Err(format!(
                    "settings schema contains unknown field `{section}.{field}`"
                ));
            }
        }
    }
    let mut settings = serde_json::from_value::<RuntimeSettings>(value.clone())
        .map_err(|error| format!("settings schema is invalid: {error}"))?;
    if settings
        .ui_preferences
        .as_ref()
        .is_some_and(|preferences| !valid_ui_preferences(preferences))
    {
        return Err("settings UI preferences are invalid".to_string());
    }
    if settings.ui_preferences.is_none() {
        let legacy_theme = value.get("theme").and_then(serde_json::Value::as_str);
        let legacy_history_point_limit = value
            .get("history_point_limit")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        match (legacy_theme, legacy_history_point_limit) {
            (Some(theme), Some(history_point_limit)) => {
                let preferences = RuntimeUiPreferences {
                    theme: theme.to_string(),
                    history_point_limit,
                };
                if !valid_ui_preferences(&preferences) {
                    return Err("legacy UI preferences are invalid".to_string());
                }
                settings.ui_preferences = Some(preferences);
                return Ok(JsonMigration::Migrated(settings));
            }
            (None, None) => {}
            _ => return Err("legacy UI preferences are incomplete".to_string()),
        }
    }
    Ok(if retires_admin_mode {
        JsonMigration::Migrated(settings)
    } else {
        JsonMigration::Current(settings)
    })
}

fn valid_ui_preferences(preferences: &RuntimeUiPreferences) -> bool {
    matches!(
        preferences.theme.as_str(),
        "system" | "cave" | "aurora" | "ember" | "daylight"
    ) && matches!(preferences.history_point_limit, 30 | 72 | 180 | 360)
}

fn push_startup_persistence_failure(
    persistence: &mut RuntimePersistenceCoordinator,
    warnings: &mut VecDeque<RuntimeWarning>,
    publication_seq: u64,
    occurred_at_ms: u64,
    failure: &crate::persistence::PersistenceFailure,
) {
    push_startup_warning(
        persistence,
        warnings,
        publication_seq,
        occurred_at_ms,
        "persistence",
        RuntimePersistenceCoordinator::failure_message(failure),
    );
}

fn push_startup_warning(
    persistence: &mut RuntimePersistenceCoordinator,
    warnings: &mut VecDeque<RuntimeWarning>,
    publication_seq: u64,
    occurred_at_ms: u64,
    category: &str,
    message: String,
) {
    push_warning(
        warnings,
        publication_seq,
        occurred_at_ms,
        category,
        message.clone(),
    );
    let event = serde_json::json!({
        "ts_ms": occurred_at_ms,
        "category": category,
        "payload": { "message": message },
    });
    if let DiagnosticWriteOutcome::Failed(failure) =
        persistence.record_diagnostic(&event, occurred_at_ms)
    {
        let diagnostic_message = RuntimePersistenceCoordinator::failure_message(&failure);
        if !warnings
            .iter()
            .any(|warning| warning.message == diagnostic_message)
        {
            push_warning(
                warnings,
                publication_seq,
                occurred_at_ms,
                "persistence",
                diagnostic_message,
            );
        }
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
    if let [process] = group.processes.as_slice() {
        return process_attention_score(process);
    }

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
    occurred_at_ms: u64,
    category: &str,
    message: String,
) {
    warnings.push_back(RuntimeWarning {
        key: warning_key(category, &message),
        publication_seq,
        occurred_at_ms,
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

pub(crate) fn empty_system() -> SystemMetricsSnapshot {
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

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn default_base_dir() -> PathBuf {
    platform_data_dir()
        .unwrap_or_else(env::temp_dir)
        .join("BatCaveMonitor")
}

#[cfg(test)]
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
    use crate::collector_engine::CollectionFailure;
    use crate::contracts::{
        AccessState, MetricQualityInfo, MetricSource, ProcessMetricQuality,
        RuntimePersistencePermissionState, RuntimeProcessElevation,
    };
    #[cfg(target_os = "macos")]
    use crate::contracts::{RuntimeInstallKind, RuntimePlatform};
    use std::sync::{
        atomic::{AtomicUsize, Ordering as TestOrdering},
        Barrier,
    };

    enum FakeOutcome {
        Sample,
        Unavailable(&'static str),
        Fatal(&'static str),
        Panic,
    }

    struct FakeCollector {
        outcomes: VecDeque<FakeOutcome>,
        collect_count: Arc<AtomicUsize>,
        first_started: Option<mpsc::Sender<()>>,
        first_release: Option<Arc<(Mutex<bool>, Condvar)>>,
        dropped: Option<mpsc::Sender<()>>,
    }

    impl FakeCollector {
        fn new(outcomes: impl IntoIterator<Item = FakeOutcome>) -> (Self, Arc<AtomicUsize>) {
            let collect_count = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    outcomes: outcomes.into_iter().collect(),
                    collect_count: Arc::clone(&collect_count),
                    first_started: None,
                    first_release: None,
                    dropped: None,
                },
                collect_count,
            )
        }
    }

    impl RawCollector for FakeCollector {
        fn collect(&mut self) -> Result<crate::telemetry::TelemetrySample, CollectionFailure> {
            let count = self.collect_count.fetch_add(1, TestOrdering::SeqCst) + 1;
            if count == 1 {
                if let Some(started) = self.first_started.take() {
                    let _ = started.send(());
                }
                if let Some(release) = &self.first_release {
                    let (ready, changed) = &**release;
                    let mut ready = ready
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while !*ready {
                        ready = changed
                            .wait(ready)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                }
            }
            match self.outcomes.pop_front().unwrap_or(FakeOutcome::Sample) {
                FakeOutcome::Sample => Ok(crate::telemetry::TelemetrySample {
                    latency_ms: 0,
                    collector_state: RuntimeCollectorState::Healthy,
                    system: empty_system(),
                    processes: vec![sample("10", "Fake", count as f64)],
                    warnings: Vec::new(),
                    collector_service: None,
                    source_provenance: None,
                }),
                FakeOutcome::Unavailable(error) => {
                    Err(CollectionFailure::Unavailable(error.to_string()))
                }
                FakeOutcome::Fatal(error) => Err(CollectionFailure::Fatal(error.to_string())),
                FakeOutcome::Panic => panic!("scripted collector panic"),
            }
        }
    }

    impl Drop for FakeCollector {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    fn state_with_collector(
        name: &str,
        collector: FakeCollector,
        automatic_sampling: bool,
    ) -> (RuntimeState, PathBuf) {
        let base_dir = runtime_test_dir(name);
        let store = RuntimeStore::from_base_dir(base_dir.clone());
        (
            RuntimeState::from_store_with_collector(
                store,
                automatic_sampling,
                Some(Box::new(collector)),
            )
            .expect("engine starts"),
            base_dir,
        )
    }

    #[test]
    fn delayed_collector_consumption_preserves_sample_time_and_rate_interval() {
        let base_dir = runtime_test_dir("delayed-collector-publication");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        let (collector, _) = FakeCollector::new([]);
        let collector_engine = CollectorEngine::start(
            Box::new(collector),
            CollectorEngineConfig {
                interval: Duration::from_millis(500),
                metric_window: Duration::from_secs(15),
                paused: false,
                automatic: false,
            },
            Arc::new(|| {}),
        )
        .expect("manual collector engine starts");
        let published = Arc::new(RwLock::new(PublishedRuntime {
            snapshot: Arc::new(store.snapshot.clone()),
            process_exe_authoritative: false,
        }));
        let mut last_collector_revision = 0;
        let first_completed = Instant::now() - Duration::from_secs(5);

        let publication = |revision, completed_at, network_received_total_bytes| {
            let mut system = empty_system();
            system.network_received_total_bytes = network_received_total_bytes;
            Arc::new(CollectorPublication {
                revision,
                completed_at,
                event: CollectorEvent::Sample(Arc::new(crate::telemetry::TelemetrySample {
                    latency_ms: 0,
                    collector_state: RuntimeCollectorState::Healthy,
                    system,
                    processes: Vec::new(),
                    warnings: Vec::new(),
                    collector_service: None,
                    source_provenance: None,
                })),
                collection_latency_ms: 0.0,
                cadence: CollectorCadence::default(),
            })
        };

        apply_collector_publication(
            &mut store,
            &published,
            &mut last_collector_revision,
            publication(1, first_completed, 1_000),
        )
        .expect("first delayed publication applies");
        let first_sampled_at_ms = store.sampled_at_ms.expect("first sample time is retained");

        let second_completed = first_completed + Duration::from_secs(1);
        apply_collector_publication(
            &mut store,
            &published,
            &mut last_collector_revision,
            publication(2, second_completed, 2_000),
        )
        .expect("second delayed publication applies");
        let second_sampled_at_ms = store.sampled_at_ms.expect("second sample time is retained");

        assert_eq!(second_sampled_at_ms - first_sampled_at_ms, 1_000);
        assert_eq!(second_sampled_at_ms, store.clock.at_ms(second_completed));
        assert!(store.clock.now_ms().saturating_sub(second_sampled_at_ms) >= 3_000);
        assert_eq!(store.snapshot.system.network_received_bps, 1_000);
        assert_eq!(store.last_heartbeat_at_ms, Some(second_sampled_at_ms));
        let mut evaluated = store.snapshot.clone();
        evaluate_snapshot_health(&mut evaluated, store.clock.now_ms());
        assert!(evaluated.health.degraded);
        assert_ne!(evaluated.health.status_summary, "Healthy.");

        collector_engine.shutdown().expect("collector engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn cadence_health_reflects_engine_window_and_recovers() {
        let base_dir = runtime_test_dir("cadence-recovery");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.paused = false;
        store.engine_state = RuntimeEngineState::Running;
        store.collector_state = Some(RuntimeCollectorState::Healthy);
        store.warnings.clear();
        store.update_schedule_health(CollectorCadence {
            deadline_misses: 2,
            recent_deadline_misses: 2,
            deadline_lateness_p95_ms: 750.0,
        });

        let degraded = store.build_health(0, 0.0, 0);
        assert!(degraded.degraded);
        assert_eq!(degraded.deadline_misses, Some(2));
        assert_eq!(degraded.dropped_ticks, 2);
        assert_eq!(
            degraded.status_summary,
            "Sampling missed 2 deadline(s) in the current health window."
        );

        store.update_schedule_health(CollectorCadence {
            deadline_misses: 2,
            recent_deadline_misses: 0,
            deadline_lateness_p95_ms: 0.0,
        });

        let recovered = store.build_health(0, 0.0, 0);
        assert!(!recovered.degraded);
        assert_eq!(recovered.status_summary, "Healthy.");
        assert_eq!(recovered.deadline_misses, Some(2));
        assert_eq!(recovered.dropped_ticks, 2);

        drop(store);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn p95_window_retains_the_configured_six_hundred_seconds() {
        let base = Instant::now();
        let mut window = P95Window::new(Duration::from_secs(600));
        window.add_at(base, 1.0);
        window.add_at(base + Duration::from_secs(599), 9.0);
        assert_eq!(window.value_at(base + Duration::from_secs(600)), 9.0);
        assert_eq!(window.value_at(base + Duration::from_secs(1_199)), 9.0);
        assert_eq!(window.value_at(base + Duration::from_secs(1_200)), 0.0);
    }

    #[test]
    fn immutable_snapshot_reads_do_not_wait_for_a_slow_collector() {
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Sample]);
        collector.first_started = Some(started_tx);
        collector.first_release = Some(Arc::clone(&release));
        let (state, base_dir) = state_with_collector("slow-read", collector, false);
        let state = Arc::new(state);
        let refresh_state = Arc::clone(&state);
        let refresh = std::thread::spawn(move || refresh_state.refresh_now());
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("collector starts");

        let read_started = Instant::now();
        let snapshot = state
            .snapshot()
            .expect("immutable snapshot remains readable");
        assert_eq!(snapshot.sample_seq, 0);
        assert!(read_started.elapsed() < Duration::from_millis(100));

        let (ready, changed) = &*release;
        *ready.lock().expect("release lock") = true;
        changed.notify_all();
        refresh
            .join()
            .expect("refresh joins")
            .expect("refresh succeeds");
        state.shutdown().expect("engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn refreshes_arriving_during_collection_coalesce_to_one_next_generation() {
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mut collector, collect_count) =
            FakeCollector::new([FakeOutcome::Sample, FakeOutcome::Sample]);
        collector.first_started = Some(started_tx);
        collector.first_release = Some(Arc::clone(&release));
        let (state, base_dir) = state_with_collector("refresh-coalesce", collector, false);
        let state = Arc::new(state);
        let first_state = Arc::clone(&state);
        let first = std::thread::spawn(move || first_state.refresh_now());
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first collection starts");

        let barrier = Arc::new(Barrier::new(9));
        let callers = (0..8)
            .map(|_| {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    state.refresh_now()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let registration_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let waiting = state
                .refresh_gate
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .waiting_callers;
            if waiting == 9 {
                break;
            }
            assert!(
                Instant::now() < registration_deadline,
                "refresh callers register"
            );
            std::thread::yield_now();
        }
        let (ready, changed) = &*release;
        *ready.lock().expect("release lock") = true;
        changed.notify_all();

        first
            .join()
            .expect("first caller joins")
            .expect("first succeeds");
        for caller in callers {
            caller
                .join()
                .expect("caller joins")
                .expect("refresh succeeds");
        }
        assert_eq!(collect_count.load(TestOrdering::SeqCst), 2);
        state.shutdown().expect("engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn unavailable_collection_recovers_and_fatal_state_is_durable() {
        let (collector, _) = FakeCollector::new([
            FakeOutcome::Unavailable("collector offline"),
            FakeOutcome::Sample,
            FakeOutcome::Fatal("collector poisoned"),
        ]);
        let (state, base_dir) = state_with_collector("collector-failures", collector, false);
        let unavailable = state
            .refresh_now()
            .expect("unavailable publishes held snapshot");
        assert_eq!(unavailable.sample_seq, 0);
        assert_eq!(
            unavailable.health.engine_state,
            Some(RuntimeEngineState::Running)
        );
        assert_eq!(
            unavailable.health.collector_state,
            Some(RuntimeCollectorState::Unavailable)
        );
        let recovered = state.refresh_now().expect("collector recovers");
        assert_eq!(recovered.sample_seq, 1);
        assert_eq!(
            state
                .refresh_now()
                .expect_err("fatal collection stops engine"),
            "runtime_engine_fatal:collector poisoned"
        );
        let fatal = state.snapshot().expect("fatal snapshot remains readable");
        assert_eq!(fatal.health.engine_state, Some(RuntimeEngineState::Fatal));
        assert!(fatal.health.fatal_error.is_some());
        let envelope = crate::protocol::encode_snapshot(fatal).expect("fatal v3 payload is valid");
        let value = serde_json::to_value(envelope).expect("fatal envelope serializes");
        assert_eq!(
            value.pointer("/event/payload/health/engine_state"),
            Some(&serde_json::json!("fatal"))
        );
        assert_eq!(
            value.pointer("/event/payload/health/collector_state"),
            Some(&serde_json::json!("unavailable"))
        );
        assert_eq!(
            value.pointer("/event/payload/health/fatal_error/code"),
            Some(&serde_json::json!("collector_fatal"))
        );
        state.shutdown().expect("fatal engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn panic_is_published_as_fatal_and_shutdown_joins_collector_drop() {
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Panic]);
        let (dropped_tx, dropped_rx) = mpsc::channel();
        collector.dropped = Some(dropped_tx);
        let (state, base_dir) = state_with_collector("collector-panic", collector, false);
        assert_eq!(
            state.refresh_now().expect_err("panic fails refresh"),
            "runtime_engine_fatal"
        );
        let fatal = state
            .snapshot()
            .expect("panic publication remains readable");
        assert_eq!(fatal.health.engine_state, Some(RuntimeEngineState::Fatal));
        assert_eq!(
            fatal
                .health
                .fatal_error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("sampling_engine_panicked")
        );
        state.shutdown().expect("panic engine joins");
        dropped_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("collector drops before shutdown returns");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn pause_resume_query_and_interval_controls_publish_consistent_state() {
        let (collector, collect_count) = FakeCollector::new([FakeOutcome::Sample]);
        let (state, base_dir) = state_with_collector("engine-controls", collector, false);
        let paused = state.pause().expect("pause succeeds");
        assert!(paused.settings.paused);
        assert_eq!(paused.health.engine_state, Some(RuntimeEngineState::Paused));
        assert_eq!(paused.health.deadline_misses, Some(0));

        let interval = state
            .set_sample_interval(2_000)
            .expect("interval update succeeds");
        assert_eq!(interval.settings.sample_interval_ms, 2_000);
        let same_interval = state
            .set_sample_interval(2_000)
            .expect("same interval remains bounded");
        assert_eq!(same_interval.settings.sample_interval_ms, 2_000);
        let query = state
            .set_query(RuntimeQuery {
                filter_text: "fake".to_string(),
                ..RuntimeQuery::default()
            })
            .expect("query update succeeds");
        assert_eq!(query.settings.query.filter_text, "fake");

        let resumed = state.resume().expect("resume publishes a fresh sample");
        assert!(!resumed.settings.paused);
        assert_eq!(
            resumed.health.engine_state,
            Some(RuntimeEngineState::Running)
        );
        assert_eq!(resumed.sample_seq, 1);
        assert_eq!(collect_count.load(TestOrdering::SeqCst), 1);
        state.shutdown().expect("engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn accepted_preference_mutations_apply_and_persist_in_fifo_order() {
        let (collector, _) = FakeCollector::new([]);
        let (state, base_dir) = state_with_collector("engine-preference-fifo", collector, false);
        let first_preferences = RuntimeUiPreferences {
            theme: "cave".to_string(),
            history_point_limit: 72,
        };
        let latest_preferences = RuntimeUiPreferences {
            theme: "ember".to_string(),
            history_point_limit: 180,
        };
        let (first_reply, first_receiver) = mpsc::channel();
        let (latest_reply, latest_receiver) = mpsc::channel();

        state
            .control
            .try_send(EngineControl::SetUiPreferences(
                first_preferences,
                first_reply,
            ))
            .expect("first preference is accepted");
        state
            .control
            .try_send(EngineControl::SetUiPreferences(
                latest_preferences.clone(),
                latest_reply,
            ))
            .expect("latest preference is accepted");

        let first = receive_snapshot(first_receiver, state.clock.now_ms())
            .expect("first preference publishes");
        let latest = receive_snapshot(latest_receiver, state.clock.now_ms())
            .expect("latest preference publishes");
        assert_eq!(
            first
                .settings
                .ui_preferences
                .as_ref()
                .map(|value| value.theme.as_str()),
            Some("cave")
        );
        assert_eq!(
            latest.settings.ui_preferences,
            Some(latest_preferences.clone())
        );
        assert_eq!(
            state
                .snapshot()
                .expect("latest publication remains readable")
                .settings
                .ui_preferences,
            Some(latest_preferences.clone())
        );
        assert_eq!(
            read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE))
                .expect("latest settings persist")
                .ui_preferences,
            Some(latest_preferences)
        );

        state.shutdown().expect("engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn paused_engine_publishes_heartbeat_without_deadline_misses() {
        let (collector, collect_count) = FakeCollector::new([]);
        let base_dir = runtime_test_dir("paused-heartbeat");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.paused = true;
        store.settings.sample_interval_ms = 500;
        let state = RuntimeState::from_store_with_collector(store, true, Some(Box::new(collector)))
            .expect("engine starts");
        std::thread::sleep(Duration::from_millis(650));
        let snapshot = state.snapshot().expect("paused heartbeat snapshot");
        assert_eq!(
            snapshot.health.engine_state,
            Some(RuntimeEngineState::Paused)
        );
        assert!(snapshot.health.last_heartbeat_at_ms.is_some());
        assert_eq!(snapshot.health.deadline_misses, Some(0));
        assert_eq!(snapshot.sample_seq, 0);
        assert_eq!(collect_count.load(TestOrdering::SeqCst), 0);
        state.shutdown().expect("paused engine joins");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn concurrent_shutdown_callers_share_post_cleanup_completion() {
        let (mut collector, _) = FakeCollector::new([]);
        let (dropped_tx, dropped_rx) = mpsc::channel();
        collector.dropped = Some(dropped_tx);
        let (state, base_dir) = state_with_collector("concurrent-shutdown", collector, false);
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(5));
        let callers = (0..4)
            .map(|_| {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    state.shutdown()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        for caller in callers {
            caller
                .join()
                .expect("shutdown caller joins")
                .expect("shared shutdown succeeds");
        }
        dropped_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("collector cleanup precedes completion");
        let _ = fs::remove_dir_all(base_dir);
    }

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
            None,
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
        assert_eq!(
            snapshot
                .process_contributors
                .cpu_identity
                .as_ref()
                .map(|identity| identity.pid.as_str()),
            Some("10")
        );
        assert_eq!(
            snapshot.process_contributors.cpu_coverage,
            Some(MetricCoverage {
                available: 5,
                total: 5
            })
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
        assert_eq!(selected.cpu_identity, None);
        assert_eq!(
            selected.cpu_coverage,
            Some(MetricCoverage {
                available: 1,
                total: 2
            })
        );
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
        assert_eq!(unknown.cpu_identity, None);
        assert_eq!(
            unknown.cpu_coverage,
            Some(MetricCoverage {
                available: 1,
                total: 2
            })
        );
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
            None,
            empty_system(),
            &all_processes,
            visible_processes,
            Vec::new(),
        );

        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].name, "worker");
        assert_eq!(snapshot.process_contributors.cpu.as_deref(), Some("worker"));
        assert!(snapshot.process_contributors.cpu_name_ambiguous);
        assert_eq!(
            snapshot
                .process_contributors
                .cpu_identity
                .as_ref()
                .map(|identity| identity.pid.as_str()),
            Some("10")
        );
        assert_eq!(
            snapshot.process_contributors.cpu_coverage,
            Some(MetricCoverage {
                available: 2,
                total: 2
            })
        );
    }

    #[test]
    fn contributor_identity_retains_unknown_start_for_publication_scoping() {
        let mut process = sample("77", "unknown-start", 25.0);
        process.start_time_ms = 0;

        let summary = summarize_process_contributors(&[process]);

        assert_eq!(
            summary.cpu_identity,
            Some(ProcessContributorIdentity {
                pid: "77".to_string(),
                start_time_ms: 0,
            })
        );
        assert_eq!(
            summary.cpu_coverage,
            Some(MetricCoverage {
                available: 1,
                total: 1
            })
        );
    }

    #[test]
    fn process_view_groups_suffixed_app_processes() {
        let mut first = sample("10", "SearchIndexer-211.exe", 12.0);
        first.quality = group_test_quality(MetricQuality::Native);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
        first.io_read_bps = 256;
        first.other_io_bps = Some(8 * 1024 * 1024);
        first.threads = 3;
        let mut second = sample("20", "SearchIndexer-223.exe", 8.0);
        second.quality = group_test_quality(MetricQuality::Native);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
        second.io_write_bps = 512;
        second.threads = 5;

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        let ProcessViewRow::Group { detail, .. } = &rows[0] else {
            panic!("expected aggregate group row");
        };
        assert_eq!(detail.label, "SearchIndexer.exe");
        assert_eq!(detail.category, "Windows");
        assert_eq!(detail.process_count, 2);
        assert_eq!(detail.cpu_percent, 20.0);
        assert_eq!(detail.io_bps, 768);
        assert_eq!(detail.other_io_bps, None);
        assert_eq!(detail.threads, 8);
        assert_eq!(
            detail.coverage.cpu,
            MetricCoverage {
                available: 2,
                total: 2
            }
        );
        assert_eq!(
            detail.coverage.network,
            MetricCoverage {
                available: 0,
                total: 2
            }
        );
        assert_eq!(detail.quality.network.quality, MetricQuality::Unavailable);
        assert_eq!(detail.quality.other_io.quality, MetricQuality::Unavailable);
        assert_eq!(
            detail.coverage.other_io,
            MetricCoverage {
                available: 0,
                total: 2
            }
        );
        let ProcessViewRow::Process {
            group_key,
            is_grouped,
            ..
        } = &rows[1]
        else {
            panic!("expected process row");
        };
        assert!(*is_grouped);
        assert_eq!(group_key, &detail.group_key);
    }

    #[test]
    fn grouped_process_rows_share_the_group_icon_kind() {
        let mut first = sample("10", "Helper-1.exe", 12.0);
        first.exe = "C:\\node\\Helper-1.exe".to_string();
        let mut second = sample("20", "Helper-2.exe", 8.0);
        second.exe = "C:\\docker\\Helper-2.exe".to_string();

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());
        let ProcessViewRow::Group {
            icon_kind: group_icon,
            ..
        } = &rows[0]
        else {
            panic!("expected aggregate group row");
        };
        for row in &rows[1..] {
            let ProcessViewRow::Process {
                icon_kind,
                is_grouped,
                ..
            } = row
            else {
                panic!("expected grouped process row");
            };
            assert!(*is_grouped);
            assert_eq!(icon_kind, group_icon);
        }
    }

    #[test]
    fn process_group_identity_survives_executable_path_enrichment() {
        let mut first = sample("10", "Visual Studio Code", 12.0);
        first.exe.clear();
        let mut second = sample("20", "Visual Studio Code", 8.0);
        second.exe.clear();
        let before = shape_process_view(&[first.clone(), second.clone()], &RuntimeQuery::default());
        let ProcessViewRow::Group {
            detail: before_detail,
            ..
        } = &before[0]
        else {
            panic!("expected aggregate group row before enrichment");
        };

        first.exe = "C:\\Program Files\\Microsoft VS Code\\Code.exe".to_string();
        second.exe = "C:\\Program Files\\Microsoft VS Code\\Code.exe".to_string();
        let after = shape_process_view(&[first, second], &RuntimeQuery::default());
        let ProcessViewRow::Group {
            detail: after_detail,
            ..
        } = &after[0]
        else {
            panic!("expected aggregate group row after enrichment");
        };

        assert_eq!(before_detail.group_key, "visual studio code");
        assert_eq!(after_detail.group_key, before_detail.group_key);
        assert_eq!(after_detail.workload_id, before_detail.workload_id);
    }

    #[test]
    fn singleton_attention_labels_publish_only_quality_backed_activity() {
        let with_quality = |quality| {
            let mut process = sample("10", "worker.exe", 90.0);
            process.network_received_bps = Some(0);
            process.network_transmitted_bps = Some(0);
            process.quality = group_test_quality(quality);
            process
        };

        let mut native_quiet = with_quality(MetricQuality::Native);
        native_quiet.cpu_percent = 9.0;
        assert_eq!(process_attention_label(&native_quiet), "steady");

        let native_active = with_quality(MetricQuality::Native);
        assert_eq!(process_attention_label(&native_active), "CPU activity");
        assert_eq!(
            process_attention_label(&with_quality(MetricQuality::Held)),
            "Pending"
        );
        assert_eq!(
            process_attention_label(&with_quality(MetricQuality::Unavailable)),
            "Unavailable"
        );

        let mut missing = with_quality(MetricQuality::Native);
        missing.quality = None;
        assert_eq!(process_attention_label(&missing), "Limited");
        assert_eq!(
            process_attention_label(&with_quality(MetricQuality::Partial)),
            "CPU activity · limited"
        );
        assert_eq!(
            process_attention_label(&with_quality(MetricQuality::Estimated)),
            "CPU activity · estimated"
        );

        let mut partial_network = with_quality(MetricQuality::Partial);
        partial_network.cpu_percent = 0.0;
        partial_network.network_received_bps = Some(2 * ATTENTION_NETWORK_BPS);
        assert_eq!(
            process_attention_label(&partial_network),
            "network activity · limited"
        );
    }

    #[test]
    fn process_view_aggregates_only_publishable_group_contributors() {
        let mut native = sample("10", "SearchIndexer-211.exe", 10.0);
        native.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
        native.memory_bytes = 100;
        native.io_read_bps = 200;
        native.network_received_bps = Some(300);
        native.threads = 4;
        native.quality = group_test_quality(MetricQuality::Native);

        let mut unavailable = sample("20", "SearchIndexer-223.exe", 90.0);
        unavailable.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
        unavailable.memory_bytes = 900;
        unavailable.io_read_bps = 1_800;
        unavailable.network_received_bps = Some(2_700);
        unavailable.threads = 36;
        unavailable.quality = group_test_quality(MetricQuality::Unavailable);

        let rows = shape_process_view(&[native, unavailable], &RuntimeQuery::default());
        let ProcessViewRow::Group {
            attention_label,
            detail,
            ..
        } = &rows[0]
        else {
            panic!("expected aggregate group row");
        };

        assert_eq!(detail.cpu_percent, 10.0);
        assert_eq!(detail.memory_bytes, 100);
        assert_eq!(detail.io_bps, 200);
        assert_eq!(detail.network_bps, 300);
        assert_eq!(detail.threads, 4);
        for coverage in [
            detail.coverage.cpu,
            detail.coverage.memory,
            detail.coverage.io,
            detail.coverage.network,
            detail.coverage.threads,
        ] {
            assert_eq!(
                coverage,
                MetricCoverage {
                    available: 1,
                    total: 2
                }
            );
        }
        for quality in [
            &detail.quality.cpu,
            &detail.quality.memory,
            &detail.quality.io,
            &detail.quality.network,
            &detail.quality.threads,
        ] {
            assert_eq!(quality.quality, MetricQuality::Partial);
        }
        assert_eq!(attention_label, "CPU activity · 1/2 · limited");
    }

    #[test]
    fn process_view_marks_missing_group_quality_limited_without_publishing_values() {
        let mut first = sample("10", "SearchIndexer-211.exe", 10.0);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
        first.quality = None;
        let mut second = sample("20", "SearchIndexer-223.exe", 90.0);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
        second.quality = None;

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());
        let ProcessViewRow::Group {
            attention_label,
            detail,
            ..
        } = &rows[0]
        else {
            panic!("expected aggregate group row");
        };

        assert_eq!(detail.cpu_percent, 0.0);
        assert_eq!(detail.memory_bytes, 0);
        assert_eq!(detail.io_bps, 0);
        assert_eq!(detail.threads, 0);
        assert_eq!(
            detail.coverage.cpu,
            MetricCoverage {
                available: 0,
                total: 2
            }
        );
        assert_eq!(detail.quality.cpu.quality, MetricQuality::Partial);
        assert!(detail
            .quality
            .cpu
            .message
            .as_deref()
            .is_some_and(|message| message.contains("0 of 2")));
        assert_eq!(attention_label, "Limited · 0/2 coverage");
    }

    #[test]
    fn process_view_group_attention_never_calls_nonpublishable_zeroes_steady() {
        for (quality, expected) in [
            (MetricQuality::Held, "Pending · 0/2 coverage"),
            (MetricQuality::Unavailable, "Unavailable · 0/2 coverage"),
        ] {
            let mut first = sample("10", "SearchIndexer-211.exe", 90.0);
            first.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
            first.network_received_bps = Some(0);
            first.quality = group_test_quality(quality);
            let mut second = sample("20", "SearchIndexer-223.exe", 80.0);
            second.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
            second.network_received_bps = Some(0);
            second.quality = group_test_quality(quality);

            let rows = shape_process_view(&[first, second], &RuntimeQuery::default());
            let ProcessViewRow::Group {
                attention_label,
                detail,
                ..
            } = &rows[0]
            else {
                panic!("expected aggregate group row");
            };

            assert_eq!(detail.cpu_percent, 0.0);
            assert_eq!(detail.coverage.cpu.available, 0);
            assert_eq!(attention_label, expected);
            assert_ne!(attention_label, "steady");
        }
    }

    #[test]
    fn process_view_singleton_sort_uses_raw_values_for_unpublishable_quality() {
        let mut unavailable_high = sample("10", "UnavailableHigh.exe", 90.0);
        unavailable_high.quality = group_test_quality(MetricQuality::Unavailable);
        let native_low = sample("20", "NativeLow.exe", 10.0);

        for sort_column in [SortColumn::CpuPct, SortColumn::Attention] {
            let rows = shape_process_view(
                &[native_low.clone(), unavailable_high.clone()],
                &RuntimeQuery {
                    sort_column,
                    sort_direction: SortDirection::Desc,
                    ..RuntimeQuery::default()
                },
            );
            let ProcessViewRow::Process { detail, .. } = &rows[0] else {
                panic!("expected singleton process row");
            };
            assert_eq!(detail.process.name, "UnavailableHigh.exe");
        }

        let mut missing_high = sample("30", "MissingHigh.exe", 80.0);
        missing_high.quality = None;
        for sort_column in [SortColumn::CpuPct, SortColumn::Attention] {
            let rows = shape_process_view(
                &[native_low.clone(), missing_high.clone()],
                &RuntimeQuery {
                    sort_column,
                    sort_direction: SortDirection::Desc,
                    ..RuntimeQuery::default()
                },
            );
            let ProcessViewRow::Process { detail, .. } = &rows[0] else {
                panic!("expected singleton process row");
            };
            assert_eq!(detail.process.name, "MissingHigh.exe");
        }
    }

    #[test]
    fn process_view_limit_counts_ranked_groups_not_hidden_children() {
        let mut processes = (0..200)
            .map(|index| {
                let mut process = sample(
                    &(1_000 + index).to_string(),
                    &format!("SearchIndexer-{index:03}.exe"),
                    1.0,
                );
                process.exe = format!("C:\\Windows\\System32\\SearchIndexer-{index:03}.exe");
                process.quality = group_test_quality(MetricQuality::Native);
                process
            })
            .collect::<Vec<_>>();
        let mut later = sample("9000", "Zulu.exe", 1.0);
        later.quality = group_test_quality(MetricQuality::Native);
        processes.push(later);

        let rows = shape_process_view(
            &processes,
            &RuntimeQuery {
                sort_column: SortColumn::Name,
                sort_direction: SortDirection::Asc,
                limit: 2,
                ..RuntimeQuery::default()
            },
        );

        let ProcessViewRow::Group { detail, .. } = &rows[0] else {
            panic!("expected aggregate group row");
        };
        assert_eq!(detail.process_count, 200);
        assert_eq!(detail.coverage.cpu.total, 200);
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(
                    row,
                    ProcessViewRow::Process {
                        is_grouped: true,
                        ..
                    }
                ))
                .count(),
            detail.process_count
        );
        assert!(rows.iter().any(|row| {
            matches!(
                row,
                ProcessViewRow::Process { detail, is_grouped: false, .. }
                    if detail.process.name == "Zulu.exe"
            )
        }));
    }

    #[test]
    fn process_view_uses_process_name_key_when_exe_is_missing() {
        let mut first = sample("10", "Memory Compression", 12.0);
        first.exe = String::new();
        let mut second = sample("20", "Memory Compression", 8.0);
        second.exe = " ".to_string();

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        let ProcessViewRow::Group { detail, .. } = &rows[0] else {
            panic!("expected aggregate group row");
        };
        assert_eq!(detail.group_key, "memory compression");
        for row in &rows[1..] {
            let ProcessViewRow::Process {
                group_key,
                is_grouped,
                ..
            } = row
            else {
                panic!("expected process row");
            };
            assert!(*is_grouped);
            assert_eq!(group_key, &detail.group_key);
        }
    }

    #[test]
    fn process_view_keeps_singletons_as_process_rows() {
        let rows = shape_process_view(&[sample("10", "Code.exe", 12.0)], &RuntimeQuery::default());

        assert_eq!(rows.len(), 1);
        let ProcessViewRow::Process {
            detail,
            is_grouped,
            group_label,
            ..
        } = &rows[0]
        else {
            panic!("expected process row");
        };
        assert!(!is_grouped);
        assert_eq!(group_label, "Code.exe");
        assert_eq!(detail.workload_id, "process:10:1700000000000");
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
        first.quality = group_test_quality(MetricQuality::Native);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-001.exe".to_string();
        let mut second = sample("30", "SearchIndexer-002.exe", 35.0);
        second.quality = group_test_quality(MetricQuality::Native);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-002.exe".to_string();

        let rows = shape_process_view(
            &[singleton, first, second],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        let ProcessViewRow::Group { detail, .. } = &rows[0] else {
            panic!("expected aggregate group row");
        };
        assert_eq!(detail.label, "SearchIndexer.exe");
        assert_eq!(detail.cpu_percent, 75.0);
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

        assert_eq!(row_cpu_percent(&descending[0]), 124.0);
        assert_eq!(row_cpu_percent(&descending[1]), 25.0);
        assert_eq!(row_cpu_percent(&ascending[0]), 25.0);
        assert_eq!(row_cpu_percent(&ascending[1]), 124.0);
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
            ("macos", MetricSource::Libproc, MetricQuality::Native),
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
            ("libproc", MetricSource::Libproc, MetricQuality::Native),
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
    fn iokit_disk_topology_and_availability_transitions_cannot_create_spikes() {
        let disk_quality = |quality, limitation_code| {
            let mut value = MetricQualityInfo::new(quality, MetricSource::Iokit);
            value.limitation_code = limitation_code;
            value
        };
        let sample = |read, write, quality, limitation_code| {
            let mut system = empty_system();
            system.disk_read_total_bytes = read;
            system.disk_write_total_bytes = write;
            system.quality = Some(crate::contracts::SystemMetricQuality {
                disk: Some(disk_quality(quality, limitation_code)),
                ..crate::contracts::SystemMetricQuality::default()
            });
            system
        };

        let first = sample(
            1_000,
            500,
            MetricQuality::Held,
            Some(MetricLimitationCode::PendingBaseline),
        );
        let first_totals = TelemetryTotals::from_system(&first, 1_000);

        let mut stable = sample(1_200, 600, MetricQuality::Native, None);
        derive_iokit_disk_rates(&mut stable, Some(&first_totals), 1.0);
        assert_eq!(stable.disk_read_bps, 200);
        assert_eq!(stable.disk_write_bps, 100);
        let stable_totals = TelemetryTotals::from_system(&stable, 2_000);

        let unavailable = sample(
            0,
            0,
            MetricQuality::Unavailable,
            Some(MetricLimitationCode::CollectorFailure),
        );
        let unavailable_totals = TelemetryTotals::from_system(&unavailable, 3_000);

        let mut recovered = sample(
            9_000,
            4_000,
            MetricQuality::Held,
            Some(MetricLimitationCode::PendingBaseline),
        );
        derive_iokit_disk_rates(&mut recovered, Some(&unavailable_totals), 1.0);
        assert_eq!(recovered.disk_read_bps, 0);
        assert_eq!(recovered.disk_write_bps, 0);
        let recovered_totals = TelemetryTotals::from_system(&recovered, 4_000);

        let mut after_recovery = sample(9_100, 4_050, MetricQuality::Native, None);
        derive_iokit_disk_rates(&mut after_recovery, Some(&recovered_totals), 1.0);
        assert_eq!(after_recovery.disk_read_bps, 100);
        assert_eq!(after_recovery.disk_write_bps, 50);

        let mut topology_change = sample(
            40_000,
            20_000,
            MetricQuality::Held,
            Some(MetricLimitationCode::PendingBaseline),
        );
        derive_iokit_disk_rates(&mut topology_change, Some(&stable_totals), 1.0);
        assert_eq!(topology_change.disk_read_bps, 0);
        assert_eq!(topology_change.disk_write_bps, 0);
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
        let state = RuntimeState::new().expect("engine starts");

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
    fn settings_normalization_clamps_ranges() {
        let settings = normalize_settings(RuntimeSettings {
            metric_window_seconds: 1,
            sample_interval_ms: 1,
            query: RuntimeQuery {
                filter_text: "  code  ".to_string(),
                focus_mode: ProcessFocusMode::Attention,
                limit: usize::MAX,
                ..RuntimeQuery::default()
            },
            paused: false,
            ui_preferences: None,
        });

        assert_eq!(settings.metric_window_seconds, 15);
        assert_eq!(settings.query.filter_text, "code");
        assert_eq!(settings.query.focus_mode, ProcessFocusMode::Attention);
        assert_eq!(settings.query.limit, 20_000);
    }

    #[test]
    fn legacy_admin_settings_are_discarded_and_not_rewritten() {
        let migrated = migrate_runtime_settings(serde_json::json!({
            "admin_mode_requested": true,
            "admin_mode_enabled": true
        }))
        .expect("legacy admin settings migrate");
        let JsonMigration::Migrated(settings) = migrated else {
            panic!("legacy admin settings must be rewritten");
        };
        let encoded = serde_json::to_value(settings).expect("settings serialize");
        assert!(encoded.get("admin_mode_requested").is_none());
        assert!(encoded.get("admin_mode_enabled").is_none());
    }

    #[test]
    fn legacy_user_preferences_migrate_into_runtime_settings() {
        let migrated = migrate_runtime_settings(serde_json::json!({
            "query": {},
            "theme": "daylight",
            "history_point_limit": 360
        }))
        .expect("legacy preferences migrate");
        let JsonMigration::Migrated(settings) = migrated else {
            panic!("legacy settings should be rewritten");
        };
        assert_eq!(
            settings.ui_preferences,
            Some(RuntimeUiPreferences {
                theme: "daylight".to_string(),
                history_point_limit: 360,
            })
        );

        assert!(migrate_runtime_settings(serde_json::json!({
            "theme": "ember"
        }))
        .is_err());
        assert!(migrate_runtime_settings(serde_json::json!({
            "schema_version": 99,
            "theme": "future",
            "history_point_limit": 72
        }))
        .is_err());
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
            Some(store.persistence.health()),
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
    fn collector_service_transition_is_visible_and_resets_source_baselines() {
        let base_dir = runtime_test_dir("collector-service-transition");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.provenance = RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Standard);
        store.previous_processes = vec![sample("10", "old-source", 1.0)];
        store.live_process_snapshot = true;
        store.previous_totals = Some(TelemetryTotals::from_system(&empty_system(), 1));

        let active = crate::contracts::RuntimeCollectorServiceStatus {
            state: RuntimeCollectorServiceState::Active,
            release_identity: Some(crate::contracts::RuntimeReleaseIdentity {
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                source_commit_sha: None,
            }),
            service_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            negotiated_protocol_version: Some(
                crate::collector_service::protocol::COLLECTOR_SERVICE_PROTOCOL_VERSION,
            ),
            minimum_desktop_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            instance_id: Some("service-instance-1".to_string()),
            last_connected_at_ms: Some(7),
            detail: None,
        };
        assert!(store.apply_collector_service_status(Some(active), 7));
        assert!(store.previous_processes.is_empty());
        assert!(store.previous_totals.is_none());
        assert_eq!(
            store.admin_mode.source,
            RuntimePrivilegedSource::CollectorService
        );
        store.publish_snapshot_only(None);
        let encoded = crate::protocol::encode_snapshot(store.snapshot.clone()).unwrap();
        let encoded = serde_json::to_value(encoded).unwrap();
        assert_eq!(
            encoded.pointer("/event/payload/privileged_collection/collector_service/state"),
            Some(&serde_json::json!("active"))
        );

        let stopped = crate::contracts::RuntimeCollectorServiceStatus {
            state: RuntimeCollectorServiceState::Stopped,
            release_identity: None,
            service_version: None,
            negotiated_protocol_version: None,
            minimum_desktop_version: None,
            instance_id: None,
            last_connected_at_ms: None,
            detail: Some("collector_service_stopped".to_string()),
        };
        assert!(!store.apply_collector_service_status(Some(stopped), 8));
        assert_eq!(store.admin_mode.source, RuntimePrivilegedSource::None);
        assert_eq!(store.admin_mode.last_success_at_ms, Some(7));
        assert_eq!(
            store
                .admin_mode
                .collector_service
                .as_ref()
                .map(|service| service.state),
            Some(RuntimeCollectorServiceState::Stopped)
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn service_clock_skew_preserves_source_rates_and_desktop_freshness() {
        let service_sample = |source_sample_seq, sampled_at_ms, received_total_bytes| {
            let mut system = empty_system();
            system.network_received_total_bytes = received_total_bytes;
            crate::telemetry::TelemetrySample {
                latency_ms: 1,
                collector_state: RuntimeCollectorState::Healthy,
                system,
                processes: Vec::new(),
                warnings: Vec::new(),
                collector_service: Some(crate::contracts::RuntimeCollectorServiceStatus {
                    state: RuntimeCollectorServiceState::Active,
                    release_identity: Some(crate::contracts::RuntimeReleaseIdentity {
                        app_version: env!("CARGO_PKG_VERSION").to_string(),
                        source_commit_sha: None,
                    }),
                    service_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    negotiated_protocol_version: Some(
                        crate::collector_service::protocol::COLLECTOR_SERVICE_PROTOCOL_VERSION,
                    ),
                    minimum_desktop_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    instance_id: Some("service-instance-1".to_string()),
                    last_connected_at_ms: Some(sampled_at_ms),
                    detail: None,
                }),
                source_provenance: Some(crate::telemetry::TelemetrySampleProvenance {
                    source_instance_id: "service-instance-1".to_string(),
                    source_sample_seq,
                    sampled_at_ms,
                }),
            }
        };

        for (label, first_source_sampled_at_ms) in
            [("service-behind", 1_000), ("service-ahead", 100_000)]
        {
            let base_dir = runtime_test_dir(&format!("collector-service-clock-{label}"));
            let mut store = RuntimeStore::from_base_dir(base_dir.clone());
            store.provenance =
                RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Standard);

            store.note_heartbeat_at(10_000);
            store.apply_raw_sample(
                service_sample(1, first_source_sampled_at_ms, 10_000),
                1.0,
                10_000,
            );
            let first_publication_seq = store.publication_seq;
            let first_sample_seq = store.sample_seq;

            store.note_heartbeat_at(10_500);
            store.apply_raw_sample(
                service_sample(1, first_source_sampled_at_ms, 10_000),
                1.0,
                10_500,
            );

            assert_eq!(store.publication_seq, first_publication_seq + 1);
            assert_eq!(store.sample_seq, first_sample_seq);
            assert_eq!(store.sampled_at_ms, Some(10_000));
            assert_eq!(store.last_heartbeat_at_ms, Some(10_500));
            assert_eq!(store.snapshot.system.network_received_total_bytes, 10_000);
            assert_eq!(store.snapshot.system.network_received_bps, 0);
            assert_eq!(
                store.previous_totals.as_ref().map(|totals| totals.ts_ms),
                Some(first_source_sampled_at_ms)
            );
            crate::protocol::encode_snapshot(store.snapshot.clone())
                .expect("unchanged service publication remains protocol-valid");

            store.note_heartbeat_at(11_000);
            store.apply_raw_sample(
                service_sample(2, first_source_sampled_at_ms + 1_000, 11_000),
                1.0,
                11_000,
            );

            assert_eq!(store.sample_seq, first_sample_seq + 1);
            assert_eq!(store.sampled_at_ms, Some(11_000));
            assert_eq!(store.snapshot.system.network_received_total_bytes, 11_000);
            assert_eq!(store.snapshot.system.network_received_bps, 1_000);
            assert_eq!(
                store.previous_totals.as_ref().map(|totals| totals.ts_ms),
                Some(first_source_sampled_at_ms + 1_000)
            );
            crate::protocol::encode_snapshot(store.snapshot.clone())
                .expect("fresh service publication remains protocol-valid");

            let _ = fs::remove_dir_all(&base_dir);
        }
    }

    #[test]
    fn negotiated_not_ready_fallback_round_trips_through_runtime_protocol() {
        let base_dir = runtime_test_dir("collector-service-not-ready");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.provenance = RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Standard);
        store.admin_mode = store.provenance.admin_mode_status();
        let identity = crate::collector_service::protocol::ServiceIdentityV1 {
            service_name: crate::collector_service::protocol::COLLECTOR_SERVICE_NAME.to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            release: crate::collector_service::host::current_release_identity(),
            instance_id: "service-instance-starting".to_string(),
            protocol_version:
                crate::collector_service::protocol::COLLECTOR_SERVICE_PROTOCOL_VERSION,
            minimum_desktop_version: env!("CARGO_PKG_VERSION").to_string(),
            limits: crate::collector_service::protocol::ServiceLimitsV1::contract(),
        };
        let status = crate::collector_service::client::status_from_failure(
            &crate::collector_service::client::ClientFailure::new(
                crate::collector_service::client::ClientFailureKind::NotReady,
                "collector_service_snapshot_not_ready",
            )
            .with_service_identity(&identity),
            false,
        );
        let sampled_at_ms = store.clock.now_ms();
        store.apply_raw_sample(
            crate::telemetry::TelemetrySample {
                latency_ms: 1,
                collector_state: RuntimeCollectorState::Healthy,
                system: empty_system(),
                processes: Vec::new(),
                warnings: vec![
                    "collector_service_snapshot_not_ready; standard-access collector fallback is active"
                        .to_string(),
                ],
                collector_service: Some(status),
                source_provenance: None,
            },
            1.0,
            sampled_at_ms,
        );

        let encoded = crate::protocol::encode_snapshot(store.snapshot.clone())
            .expect("connecting fallback status encodes");
        let bytes = serde_json::to_vec(&encoded).expect("encode runtime protocol JSON");
        let decoded: crate::protocol::ProtocolEnvelope =
            serde_json::from_slice(&bytes).expect("decode runtime protocol JSON");
        let decoded = serde_json::to_value(decoded).expect("inspect decoded runtime protocol");
        assert_eq!(
            decoded.pointer("/event/payload/privileged_collection/collector_service/state"),
            Some(&serde_json::json!("connecting"))
        );
        assert_eq!(
            decoded
                .pointer("/event/payload/privileged_collection/collector_service/service_version"),
            Some(&serde_json::json!(env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(
            decoded.pointer("/event/payload/privileged_collection/source"),
            Some(&serde_json::json!("none"))
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn negotiation_failures_preserve_verified_release_for_incompatible_fallback() {
        use crate::collector_service::{
            authorization::VerifiedServicePeer,
            client::{
                status_from_failure, ClientFailure, ClientFailureKind, ClientTransport,
                ServiceClientSession,
            },
            host::current_release_identity,
            protocol::{
                ClientRequestV1, ServiceFailureCodeV1, ServiceFailureV1, ServiceOutcomeV1,
                ServiceResponseV1, COLLECTOR_SERVICE_PROTOCOL_VERSION,
            },
        };

        struct NegotiationTransport {
            peer: VerifiedServicePeer,
            response: Option<Result<ServiceResponseV1, ClientFailure>>,
        }

        impl ClientTransport for NegotiationTransport {
            fn verified_peer(&self) -> &VerifiedServicePeer {
                &self.peer
            }

            fn exchange(
                &mut self,
                _request: &ClientRequestV1,
            ) -> Result<ServiceResponseV1, ClientFailure> {
                self.response.take().unwrap_or_else(|| {
                    Err(ClientFailure::new(
                        ClientFailureKind::Failed,
                        "negotiation_response_missing",
                    ))
                })
            }
        }

        for (label, response) in [
            (
                "exchange-error",
                Err(ClientFailure::new(
                    ClientFailureKind::Incompatible,
                    "collector_service_response_protocol_incompatible",
                )),
            ),
            (
                "negotiate-error",
                Ok(ServiceResponseV1 {
                    protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                    request_id: 1,
                    outcome: ServiceOutcomeV1::Error(ServiceFailureV1 {
                        code: ServiceFailureCodeV1::Incompatible,
                        detail: "collector_service_protocol_incompatible".to_string(),
                    }),
                }),
            ),
            (
                "invalid-outcome",
                Ok(ServiceResponseV1 {
                    protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                    request_id: 1,
                    outcome: ServiceOutcomeV1::Disconnected,
                }),
            ),
        ] {
            let release = current_release_identity();
            let transport = NegotiationTransport {
                peer: VerifiedServicePeer::from_transport_verification(
                    20,
                    30,
                    [1; 32],
                    [2; 32],
                    release.clone(),
                )
                .expect("verified service peer"),
                response: Some(response),
            };
            let failure = ServiceClientSession::connect(transport)
                .err()
                .expect("negotiation must fail");
            assert_eq!(failure.kind, ClientFailureKind::Incompatible);
            assert_eq!(failure.service_release.as_ref(), Some(&release));

            let status = status_from_failure(&failure, false);
            let base_dir = runtime_test_dir(&format!("collector-service-negotiation-{label}"));
            let mut store = RuntimeStore::from_base_dir(base_dir.clone());
            store.provenance =
                RuntimeProvenance::windows_for_test(RuntimeProcessElevation::Standard);
            let sampled_at_ms = store.clock.now_ms();
            store.apply_raw_sample(
                crate::telemetry::TelemetrySample {
                    latency_ms: 1,
                    collector_state: RuntimeCollectorState::Healthy,
                    system: empty_system(),
                    processes: Vec::new(),
                    warnings: vec![format!(
                        "{}; standard-access collector fallback is active",
                        failure.detail
                    )],
                    collector_service: Some(status),
                    source_provenance: None,
                },
                1.0,
                sampled_at_ms,
            );

            let encoded = crate::protocol::encode_snapshot(store.snapshot.clone())
                .expect("incompatible fallback status validates and encodes");
            let bytes = serde_json::to_vec(&encoded).expect("encode runtime protocol JSON");
            let decoded: crate::protocol::ProtocolEnvelope =
                serde_json::from_slice(&bytes).expect("decode runtime protocol JSON");
            let decoded = serde_json::to_value(decoded).expect("inspect decoded runtime protocol");
            assert_eq!(
                decoded.pointer("/event/payload/privileged_collection/collector_service/state"),
                Some(&serde_json::json!("incompatible"))
            );
            assert_eq!(
                decoded.pointer(
                    "/event/payload/privileged_collection/collector_service/release_identity/app_version"
                ),
                Some(&serde_json::json!(env!("CARGO_PKG_VERSION")))
            );
            assert_eq!(
                decoded.pointer(
                    "/event/payload/privileged_collection/collector_service/minimum_desktop_version"
                ),
                Some(&serde_json::Value::Null)
            );
            let _ = fs::remove_dir_all(base_dir);
        }
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
    fn warm_cache_is_not_written_during_privileged_collection() {
        let base_dir = runtime_test_dir("admin-cache-skip");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.admin_mode.state = RuntimeAdminModeState::Active;
        store.previous_processes = vec![sample("10", "Elevated", 0.0)];
        store.publication_seq = 10;

        let _ = store.persist_warm_cache();

        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn standard_monitoring_starts_with_truthful_degraded_persistence() {
        let parent = runtime_test_dir("invalid-persistence-root");
        let base_dir = parent.join("not-a-directory");
        fs::create_dir_all(&parent).expect("test parent exists");
        fs::write(&base_dir, "occupied by a file").expect("invalid root fixture writes");

        let store = RuntimeStore::from_base_dir(base_dir);
        let persistence = store
            .snapshot
            .persistence
            .as_ref()
            .expect("health published");

        assert_eq!(persistence.state, RuntimePersistenceState::Degraded);
        assert!(store.snapshot.health.degraded);
        assert!(store.snapshot.health.status_summary.contains("persistence"));
        assert_eq!(store.engine_state, RuntimeEngineState::Starting);
        assert_eq!(store.settings, RuntimeSettings::default());
        assert!(store
            .warnings
            .iter()
            .any(|warning| warning.category == "persistence"));

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn runtime_only_query_does_not_leak_into_unrelated_settings_writes() {
        let base_dir = runtime_test_dir("runtime-only-query-durability");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        let runtime_only = store.set_query_with_intent(
            RuntimeQuery {
                focus_mode: ProcessFocusMode::Attention,
                ..RuntimeQuery::default()
            },
            QueryWriteIntent::RuntimeOnly,
        );
        assert_eq!(
            runtime_only.settings.query.focus_mode,
            ProcessFocusMode::Attention
        );

        store.set_ui_preferences(RuntimeUiPreferences {
            theme: "ember".to_string(),
            history_point_limit: 180,
        });
        let persisted =
            read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE)).expect("settings persist");
        assert_eq!(persisted.query, RuntimeQuery::default());
        assert_eq!(
            persisted.ui_preferences,
            Some(RuntimeUiPreferences {
                theme: "ember".to_string(),
                history_point_limit: 180,
            })
        );
        assert_eq!(store.settings.query.focus_mode, ProcessFocusMode::Attention);

        drop(store);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_settings_survive_ordinary_runtime_open_and_close() {
        use std::os::unix::fs::PermissionsExt;

        let base_dir = runtime_test_dir("corrupt-settings-preserved");
        fs::create_dir_all(&base_dir).expect("settings root exists");
        fs::set_permissions(&base_dir, fs::Permissions::from_mode(0o700))
            .expect("settings root is private");
        let path = base_dir.join(SETTINGS_FILE);
        let original = b"{not-json\n";
        fs::write(&path, original).expect("corrupt settings fixture writes");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("settings fixture is private");

        {
            let mut store = RuntimeStore::from_base_dir(base_dir.clone());
            assert!(store.settings_rewrite_blocked);
            assert!(store
                .warnings
                .iter()
                .any(|warning| warning.message.contains("persistence_corrupt_data")));
            let original_failure = serde_json::to_value(
                store
                    .snapshot
                    .persistence
                    .as_ref()
                    .and_then(|persistence| {
                        persistence.components.iter().find(|component| {
                            component.kind == crate::contracts::RuntimePersistenceKind::Settings
                        })
                    })
                    .and_then(|component| component.active_failure.as_ref()),
            )
            .expect("failure serializes");
            store.previous_processes = vec![
                sample("10", "Quiet", 0.1),
                sample("20", "Busy", ATTENTION_CPU_PERCENT),
            ];
            store.live_process_snapshot = true;
            store.publish_snapshot_only(None);
            assert_eq!(store.snapshot.processes.len(), 2);

            let runtime_only = store.set_query_with_intent(
                RuntimeQuery {
                    focus_mode: ProcessFocusMode::Attention,
                    ..RuntimeQuery::default()
                },
                QueryWriteIntent::RuntimeOnly,
            );
            assert_eq!(
                runtime_only.settings.query.focus_mode,
                ProcessFocusMode::Attention
            );
            assert_eq!(runtime_only.processes.len(), 1);
            assert_eq!(runtime_only.processes[0].pid, "20");
            assert_eq!(runtime_only.process_view_rows.len(), 1);
            assert_eq!(
                runtime_only
                    .persistence
                    .as_ref()
                    .map(|persistence| persistence.state),
                Some(RuntimePersistenceState::Degraded)
            );
            assert_eq!(
                serde_json::to_value(
                    runtime_only
                        .persistence
                        .as_ref()
                        .and_then(|persistence| {
                            persistence.components.iter().find(|component| {
                                component.kind == crate::contracts::RuntimePersistenceKind::Settings
                            })
                        })
                        .and_then(|component| component.active_failure.as_ref()),
                )
                .expect("failure serializes"),
                original_failure
            );
            assert!(store.settings_rewrite_blocked);
            assert_eq!(fs::read(&path).expect("settings remain readable"), original);
            store
                .shutdown_owned_resources()
                .expect("ordinary shutdown leaves unreadable settings untouched");
        }

        assert_eq!(fs::read(&path).expect("settings remain readable"), original);

        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.set_query_with_intent(
            RuntimeQuery {
                focus_mode: ProcessFocusMode::Io,
                ..RuntimeQuery::default()
            },
            QueryWriteIntent::UserMutation,
        );
        assert!(!store.settings_rewrite_blocked);
        let persisted =
            read_json::<RuntimeSettings>(&path).expect("user mutation replaces defaults");
        assert_eq!(persisted.query, store.settings.query);

        drop(store);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(unix)]
    #[test]
    fn failed_settings_migration_survives_ordinary_runtime_open_and_close() {
        use std::os::unix::fs::PermissionsExt;

        for (name, original) in [
            (
                "incomplete-settings-migration-preserved",
                br#"{"theme":"ember"}"#.as_slice(),
            ),
            (
                "unknown-settings-schema-preserved",
                br#"{"schema_version":99,"theme":"future"}"#.as_slice(),
            ),
        ] {
            let base_dir = runtime_test_dir(name);
            fs::create_dir_all(&base_dir).expect("settings root exists");
            fs::set_permissions(&base_dir, fs::Permissions::from_mode(0o700))
                .expect("settings root is private");
            let path = base_dir.join(SETTINGS_FILE);
            fs::write(&path, original).expect("legacy settings fixture writes");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("settings fixture is private");

            {
                let mut store = RuntimeStore::from_base_dir(base_dir.clone());
                assert!(store.settings_rewrite_blocked);
                assert!(store
                    .warnings
                    .iter()
                    .any(|warning| warning.message.contains("persistence_migration_failed")));
                store
                    .shutdown_owned_resources()
                    .expect("ordinary shutdown leaves failed migration bytes untouched");
            }

            assert_eq!(fs::read(&path).expect("settings remain readable"), original);
            let _ = fs::remove_dir_all(base_dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_snapshot_and_protocol_publish_root_invalidation_and_recovery() {
        use std::os::unix::fs::PermissionsExt;

        let base_dir = runtime_test_dir("root-health-transition");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Sample]);
        let sample = collector
            .collect()
            .expect("a valid raw sample is available");
        let sample_ts_ms = store.clock.now_ms();
        store.apply_raw_sample(sample, 0.0, sample_ts_ms);
        fs::remove_dir_all(&base_dir).expect("verified root is removed for invalidation");
        fs::write(&base_dir, "not a directory").expect("invalid root fixture writes");

        let invalid = store.set_query_with_intent(
            RuntimeQuery {
                filter_text: "invalid".to_string(),
                ..RuntimeQuery::default()
            },
            QueryWriteIntent::UserMutation,
        );
        let persistence = invalid
            .persistence
            .as_ref()
            .expect("persistence is published");
        assert_eq!(persistence.state, RuntimePersistenceState::Degraded);
        assert_eq!(
            persistence.roots[0].permission_state,
            RuntimePersistencePermissionState::Invalid
        );
        let invalid_protocol = serde_json::to_value(
            crate::protocol::encode_snapshot(invalid).expect("invalid root snapshot encodes"),
        )
        .expect("invalid root protocol serializes");
        assert_eq!(
            invalid_protocol["event"]["payload"]["persistence"]["roots"][0]["permission_state"],
            "invalid"
        );

        fs::remove_file(&base_dir).expect("invalid root fixture removes");
        fs::create_dir_all(&base_dir).expect("root is recreated");
        fs::set_permissions(&base_dir, fs::Permissions::from_mode(0o700))
            .expect("recreated root is private");
        let recovered = store.set_query_with_intent(
            RuntimeQuery {
                filter_text: "recovered".to_string(),
                ..RuntimeQuery::default()
            },
            QueryWriteIntent::UserMutation,
        );
        let persistence = recovered
            .persistence
            .as_ref()
            .expect("recovered persistence is published");
        assert_eq!(persistence.state, RuntimePersistenceState::Healthy);
        assert_eq!(
            persistence.roots[0].permission_state,
            RuntimePersistencePermissionState::Verified
        );
        assert!(!recovered.health.degraded);
        let recovered_protocol = serde_json::to_value(
            crate::protocol::encode_snapshot(recovered).expect("recovered snapshot encodes"),
        )
        .expect("recovered protocol serializes");
        assert_eq!(
            recovered_protocol["event"]["payload"]["persistence"]["roots"][0]["permission_state"],
            "verified"
        );

        drop(store);
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn shutdown_flushes_settings_and_warm_cache_once() {
        let base_dir = runtime_test_dir("shutdown-persistence-flush");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        // This fixture proves the standard-user cache flush path independently of the host token.
        store.settings.ui_preferences = Some(RuntimeUiPreferences {
            theme: "ember".to_string(),
            history_point_limit: 180,
        });
        store.previous_processes = vec![sample("10", "Cached", 0.0)];
        store.publication_seq = 12;

        store
            .shutdown_owned_resources()
            .expect("owned state flushes");
        store
            .shutdown_owned_resources()
            .expect("second shutdown is idempotent");

        let settings = read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE))
            .expect("settings persisted");
        let cache =
            read_json::<WarmCache>(&base_dir.join(WARM_CACHE_FILE)).expect("warm cache persisted");
        assert_eq!(settings.ui_preferences, store.settings.ui_preferences);
        assert_eq!(cache.seq, 12);
        assert_eq!(cache.rows.len(), 1);
        assert_eq!(cache.rows[0].name, "Cached");

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn current_user_persistence_keeps_collector_warning_health_protocol_valid() {
        let base_dir = runtime_test_dir("health-warning");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.warnings.clear();
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Sample]);
        let sample = collector
            .collect()
            .expect("a valid raw sample is available");
        let sample_ts_ms = store.clock.now_ms();
        store.apply_raw_sample(sample, 0.0, sample_ts_ms);

        let persistence = store.persistence.health();
        assert_eq!(persistence.state, RuntimePersistenceState::Healthy);
        assert_eq!(
            persistence.roots[0].permission_state,
            RuntimePersistencePermissionState::Verified
        );

        store.publish_snapshot_only(Some((
            "collector",
            "network_attribution_failed:access_denied".to_string(),
        )));

        let health = &store.snapshot.health;
        assert!(health.degraded);
        assert_eq!(health.collector_warnings, 1);
        assert!(health.status_summary.contains("1 telemetry limitation"));
        crate::protocol::encode_snapshot(store.snapshot.clone())
            .expect("collector warning snapshot remains protocol-valid");

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

    fn row_cpu_percent(row: &ProcessViewRow) -> f64 {
        match row {
            ProcessViewRow::Process { detail, .. } => detail.process.cpu_percent,
            ProcessViewRow::Group { detail, .. } => detail.cpu_percent,
        }
    }

    fn group_test_quality(quality: MetricQuality) -> Option<ProcessMetricQuality> {
        let metric = || Some(MetricQualityInfo::new(quality, MetricSource::DirectApi));
        Some(ProcessMetricQuality {
            cpu: metric(),
            memory: metric(),
            io: metric(),
            other_io: metric(),
            network: metric(),
            threads: metric(),
            handles: metric(),
        })
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
