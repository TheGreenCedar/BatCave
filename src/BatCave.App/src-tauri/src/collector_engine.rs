use std::{
    collections::VecDeque,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Condvar, Mutex, RwLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::telemetry::{TelemetryCollector, TelemetrySample};

const CONTROL_QUEUE_CAPACITY: usize = 8;
const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

type PublicationReply = mpsc::Sender<Result<Arc<CollectorPublication>, String>>;
type UnitReply = mpsc::Sender<Result<(), String>>;
type BoolReply = mpsc::Sender<Result<bool, String>>;

#[derive(Clone, Debug)]
pub(crate) enum CollectionFailure {
    Unavailable(String),
    Fatal(String),
}

pub(crate) trait RawCollector: Send {
    fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure>;
    fn process_network_ready(&self) -> Result<bool, String>;
    fn retry_process_network(&mut self) -> Result<(), String>;
    fn shutdown(&mut self) -> Result<(), String> {
        Ok(())
    }
}

impl RawCollector for TelemetryCollector {
    fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure> {
        TelemetryCollector::collect(self).map_err(|error| {
            if error.contains("lock is poisoned") {
                CollectionFailure::Fatal(error)
            } else {
                CollectionFailure::Unavailable(error)
            }
        })
    }

    fn process_network_ready(&self) -> Result<bool, String> {
        TelemetryCollector::process_network_ready(self)
    }

    fn retry_process_network(&mut self) -> Result<(), String> {
        TelemetryCollector::retry_process_network(self)
    }

    #[cfg(windows)]
    fn shutdown(&mut self) -> Result<(), String> {
        TelemetryCollector::shutdown(self)
    }
}

#[cfg(windows)]
impl RawCollector for crate::collector_service::client::DesktopCollector {
    fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure> {
        crate::collector_service::client::DesktopCollector::collect(self)
    }

    fn process_network_ready(&self) -> Result<bool, String> {
        crate::collector_service::client::DesktopCollector::process_network_ready(self)
    }

    fn retry_process_network(&mut self) -> Result<(), String> {
        crate::collector_service::client::DesktopCollector::retry_process_network(self)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CollectorCadence {
    pub deadline_misses: u64,
    pub recent_deadline_misses: u64,
    pub deadline_lateness_p95_ms: f64,
}

#[derive(Clone, Debug)]
pub(crate) enum CollectorEvent {
    Sample(Arc<TelemetrySample>),
    Unavailable(String),
    Fatal { code: String, message: String },
    PausedHeartbeat,
}

#[derive(Clone, Debug)]
pub(crate) struct CollectorPublication {
    pub revision: u64,
    pub completed_at: Instant,
    pub event: CollectorEvent,
    pub collection_latency_ms: f64,
    pub cadence: CollectorCadence,
}

#[derive(Clone, Copy)]
pub(crate) struct CollectorEngineConfig {
    pub interval: Duration,
    pub metric_window: Duration,
    pub paused: bool,
    pub automatic: bool,
}

#[derive(Default)]
struct RefreshGateState {
    requested_generation: u64,
    started_generation: u64,
    completed_generation: u64,
    waiting_callers: usize,
    wake_queued: bool,
    completed_publication: Option<(u64, Arc<CollectorPublication>)>,
    failed_generation: Option<(u64, String)>,
    terminal_error: Option<String>,
}

#[derive(Default)]
struct RefreshGate {
    state: Mutex<RefreshGateState>,
    changed: Condvar,
}

enum CollectorControl {
    Refresh,
    Pause(UnitReply),
    Resume(PublicationReply),
    SetInterval(Duration, UnitReply),
    ProcessNetworkReady(BoolReply),
    RetryProcessNetwork(UnitReply),
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct CollectorEngineHandle {
    control: SyncSender<CollectorControl>,
    published: Arc<RwLock<Option<Arc<CollectorPublication>>>>,
    refresh_gate: Arc<RefreshGate>,
    shutdown_started: Arc<AtomicBool>,
}

impl CollectorEngineHandle {
    pub fn snapshot(&self) -> Option<Arc<CollectorPublication>> {
        self.published
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn refresh_now(&self) -> Result<Arc<CollectorPublication>, String> {
        if self.shutdown_started.load(AtomicOrdering::Acquire) {
            return Err("collector_engine_shutting_down".to_string());
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
            if let Err(error) = self.control.try_send(CollectorControl::Refresh) {
                let message = control_send_error(error);
                fail_refresh_generation(&self.refresh_gate, target_generation, message.clone());
                finish_refresh_wait(&self.refresh_gate);
                return Err(message);
            }
        }
        let result = wait_for_refresh_generation(&self.refresh_gate, target_generation);
        finish_refresh_wait(&self.refresh_gate);
        result
    }

    pub fn pause(&self) -> Result<(), String> {
        self.request_unit(CollectorControl::Pause)
    }

    pub fn resume(&self) -> Result<Arc<CollectorPublication>, String> {
        self.request_publication(CollectorControl::Resume)
    }

    pub fn set_interval(&self, interval: Duration) -> Result<(), String> {
        validate_interval(interval)?;
        self.request_unit(|reply| CollectorControl::SetInterval(interval, reply))
    }

    pub fn process_network_ready(&self) -> Result<bool, String> {
        let (reply, receiver) = mpsc::channel();
        self.send(CollectorControl::ProcessNetworkReady(reply))?;
        receiver
            .recv_timeout(CONTROL_RESPONSE_TIMEOUT)
            .map_err(|_| "collector_control_timeout".to_string())?
    }

    pub fn retry_process_network(&self) -> Result<(), String> {
        self.request_unit(CollectorControl::RetryProcessNetwork)
    }

    fn request_unit(
        &self,
        control: impl FnOnce(UnitReply) -> CollectorControl,
    ) -> Result<(), String> {
        let (reply, receiver) = mpsc::channel();
        self.send(control(reply))?;
        receiver
            .recv_timeout(CONTROL_RESPONSE_TIMEOUT)
            .map_err(|_| "collector_control_timeout".to_string())?
    }

    fn request_publication(
        &self,
        control: impl FnOnce(PublicationReply) -> CollectorControl,
    ) -> Result<Arc<CollectorPublication>, String> {
        let (reply, receiver) = mpsc::channel();
        self.send(control(reply))?;
        receiver
            .recv_timeout(CONTROL_RESPONSE_TIMEOUT)
            .map_err(|_| "collector_control_timeout".to_string())?
    }

    fn send(&self, control: CollectorControl) -> Result<(), String> {
        if self.shutdown_started.load(AtomicOrdering::Acquire) {
            return Err("collector_engine_shutting_down".to_string());
        }
        self.control.try_send(control).map_err(control_send_error)
    }
}

pub(crate) struct CollectorEngine {
    handle: CollectorEngineHandle,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown_lock: Mutex<()>,
    completion: Mutex<Receiver<Result<(), String>>>,
    shutdown_result: Mutex<Option<Result<(), String>>>,
}

impl CollectorEngine {
    pub fn start_default(
        config: CollectorEngineConfig,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        let collector = Box::new(crate::collector_service::client::DesktopCollector::new());
        #[cfg(not(windows))]
        let collector = Box::new(TelemetryCollector::new());
        Self::start(collector, config, notify)
    }

    pub fn start(
        collector: Box<dyn RawCollector>,
        config: CollectorEngineConfig,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        validate_interval(config.interval)?;
        let (control, receiver) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let published = Arc::new(RwLock::new(None));
        let refresh_gate = Arc::new(RefreshGate::default());
        let shutdown_started = Arc::new(AtomicBool::new(false));
        let worker_published = Arc::clone(&published);
        let worker_gate = Arc::clone(&refresh_gate);
        let worker_shutdown = Arc::clone(&shutdown_started);
        let (completion_sender, completion) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("batcave-raw-collector".to_string())
            .spawn(move || {
                let result = run_collector_engine(
                    collector,
                    receiver,
                    worker_published,
                    worker_gate,
                    worker_shutdown,
                    config,
                    notify,
                );
                let _ = completion_sender.send(result);
            })
            .map_err(|error| format!("collector_engine_spawn_failed:{error}"))?;

        Ok(Self {
            handle: CollectorEngineHandle {
                control,
                published,
                refresh_gate,
                shutdown_started,
            },
            worker: Mutex::new(Some(worker)),
            shutdown_lock: Mutex::new(()),
            completion: Mutex::new(completion),
            shutdown_result: Mutex::new(None),
        })
    }

    pub fn handle(&self) -> CollectorEngineHandle {
        self.handle.clone()
    }

    pub fn shutdown(&self) -> Result<(), String> {
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
        if !self
            .handle
            .shutdown_started
            .swap(true, AtomicOrdering::AcqRel)
        {
            terminate_refresh_gate(
                &self.handle.refresh_gate,
                "collector_engine_shutting_down".to_string(),
            );
            let _ = self.handle.control.try_send(CollectorControl::Shutdown);
        }
        let completion = self
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv_timeout(SHUTDOWN_TIMEOUT)
            .map_err(|_| "collector_engine_shutdown_timeout".to_string())?;
        let join = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map_or(Ok(()), |worker| {
                worker
                    .join()
                    .map_err(|_| "collector_engine_join_failed".to_string())
            });
        let result = completion.and(join);
        *self
            .shutdown_result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result.clone());
        result
    }
}

impl Drop for CollectorEngine {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn control_send_error(error: TrySendError<CollectorControl>) -> String {
    match error {
        TrySendError::Full(_) => "collector_control_busy".to_string(),
        TrySendError::Disconnected(_) => "collector_engine_unavailable".to_string(),
    }
}

fn validate_interval(interval: Duration) -> Result<(), String> {
    if interval.is_zero() {
        Err("collector_interval_must_be_positive".to_string())
    } else {
        Ok(())
    }
}

struct AbsoluteScheduler {
    interval: Duration,
    next_deadline: Instant,
    deadline_misses: u64,
    recent_deadline_misses: TimedCountWindow,
    lateness_p95: TimedP95Window,
}

impl AbsoluteScheduler {
    fn new(now: Instant, config: CollectorEngineConfig) -> Self {
        Self {
            interval: config.interval,
            next_deadline: if config.paused {
                now + config.interval
            } else {
                now
            },
            deadline_misses: 0,
            recent_deadline_misses: TimedCountWindow::new(config.metric_window),
            lateness_p95: TimedP95Window::new(config.metric_window),
        }
    }

    fn wait_from(&self, now: Instant) -> Duration {
        self.next_deadline.saturating_duration_since(now)
    }

    fn deadline_is_due(&self, now: Instant) -> bool {
        now >= self.next_deadline
    }

    fn record_start(&mut self, now: Instant) {
        self.lateness_p95.add_at(
            now,
            now.saturating_duration_since(self.next_deadline)
                .as_secs_f64()
                * 1_000.0,
        );
    }

    fn complete_scheduled_work(&mut self, completed_at: Instant) {
        let mut next = self.next_deadline + self.interval;
        let mut missed = 0_u64;
        while next <= completed_at {
            self.deadline_misses = self.deadline_misses.saturating_add(1);
            missed = missed.saturating_add(1);
            next += self.interval;
        }
        if missed > 0 {
            self.recent_deadline_misses.add_at(completed_at, missed);
        }
        self.next_deadline = next;
    }

    fn complete_paused_work(&mut self, completed_at: Instant) {
        let mut next = self.next_deadline + self.interval;
        while next <= completed_at {
            next += self.interval;
        }
        self.next_deadline = next;
    }

    fn reanchor(&mut self, now: Instant, interval: Duration, due_now: bool) {
        self.interval = interval;
        self.next_deadline = if due_now { now } else { now + interval };
    }

    fn cadence(&mut self, now: Instant) -> CollectorCadence {
        CollectorCadence {
            deadline_misses: self.deadline_misses,
            recent_deadline_misses: self.recent_deadline_misses.value_at(now),
            deadline_lateness_p95_ms: self.lateness_p95.value_at(now),
        }
    }
}

fn run_collector_engine(
    mut collector: Box<dyn RawCollector>,
    receiver: Receiver<CollectorControl>,
    published: Arc<RwLock<Option<Arc<CollectorPublication>>>>,
    refresh_gate: Arc<RefreshGate>,
    shutdown_started: Arc<AtomicBool>,
    config: CollectorEngineConfig,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> Result<(), String> {
    let mut scheduler = AbsoluteScheduler::new(Instant::now(), config);
    let mut paused = config.paused;
    let mut revision = 0_u64;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        collector_loop(
            collector.as_mut(),
            &receiver,
            &published,
            &refresh_gate,
            &shutdown_started,
            &mut scheduler,
            &mut paused,
            &mut revision,
            config.automatic,
            &notify,
        )
    }));
    let fatal = match result {
        Ok(fatal) => fatal,
        Err(_) => {
            let publication = publish(
                &published,
                &notify,
                revision.saturating_add(1),
                Instant::now(),
                CollectorEvent::Fatal {
                    code: "sampling_engine_panicked".to_string(),
                    message: "raw collector engine panicked".to_string(),
                },
                0.0,
                scheduler.cadence(Instant::now()),
            );
            revision = publication.revision;
            terminate_refresh_gate(&refresh_gate, "collector_engine_fatal".to_string());
            true
        }
    };
    if fatal {
        fatal_control_loop(&receiver, &shutdown_started);
    }
    let _ = revision;
    collector.shutdown()
}

#[allow(clippy::too_many_arguments)]
fn collector_loop(
    collector: &mut dyn RawCollector,
    receiver: &Receiver<CollectorControl>,
    published: &Arc<RwLock<Option<Arc<CollectorPublication>>>>,
    refresh_gate: &Arc<RefreshGate>,
    shutdown_started: &AtomicBool,
    scheduler: &mut AbsoluteScheduler,
    paused: &mut bool,
    revision: &mut u64,
    automatic: bool,
    notify: &Arc<dyn Fn() + Send + Sync>,
) -> bool {
    loop {
        if shutdown_started.load(AtomicOrdering::Acquire) {
            return false;
        }
        if automatic && scheduler.deadline_is_due(Instant::now()) {
            if *paused {
                scheduler.complete_paused_work(Instant::now());
                *revision = publish(
                    published,
                    notify,
                    revision.saturating_add(1),
                    Instant::now(),
                    CollectorEvent::PausedHeartbeat,
                    0.0,
                    scheduler.cadence(Instant::now()),
                )
                .revision;
            } else if collect_and_publish(collector, published, scheduler, revision, notify, true) {
                terminate_refresh_gate(refresh_gate, "collector_engine_fatal".to_string());
                return true;
            }
            continue;
        }

        let received = if automatic {
            receiver.recv_timeout(scheduler.wait_from(Instant::now()))
        } else {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match received {
            Ok(CollectorControl::Shutdown) => return false,
            Ok(CollectorControl::Refresh) => {
                let Some(generation) = begin_refresh_generation(refresh_gate) else {
                    continue;
                };
                let fatal =
                    collect_and_publish(collector, published, scheduler, revision, notify, false);
                let publication = published
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
                    .expect("collection publishes an immutable snapshot");
                if fatal {
                    terminate_refresh_gate(refresh_gate, "collector_engine_fatal".to_string());
                    return true;
                }
                complete_refresh_generation(refresh_gate, generation, publication);
            }
            Ok(CollectorControl::Pause(reply)) => {
                *paused = true;
                scheduler.reanchor(Instant::now(), scheduler.interval, false);
                let _ = reply.send(Ok(()));
            }
            Ok(CollectorControl::Resume(reply)) => {
                *paused = false;
                scheduler.reanchor(Instant::now(), scheduler.interval, true);
                let fatal =
                    collect_and_publish(collector, published, scheduler, revision, notify, true);
                let publication = published
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
                    .expect("resume publishes an immutable snapshot");
                if fatal {
                    let _ = reply.send(Err("collector_engine_fatal".to_string()));
                    terminate_refresh_gate(refresh_gate, "collector_engine_fatal".to_string());
                    return true;
                }
                let _ = reply.send(Ok(publication));
            }
            Ok(CollectorControl::SetInterval(interval, reply)) => {
                if let Err(error) = validate_interval(interval) {
                    let _ = reply.send(Err(error));
                } else {
                    scheduler.reanchor(Instant::now(), interval, false);
                    let _ = reply.send(Ok(()));
                }
            }
            Ok(CollectorControl::ProcessNetworkReady(reply)) => {
                let _ = reply.send(collector.process_network_ready());
            }
            Ok(CollectorControl::RetryProcessNetwork(reply)) => {
                let _ = reply.send(collector.retry_process_network());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn collect_and_publish(
    collector: &mut dyn RawCollector,
    published: &Arc<RwLock<Option<Arc<CollectorPublication>>>>,
    scheduler: &mut AbsoluteScheduler,
    revision: &mut u64,
    notify: &Arc<dyn Fn() + Send + Sync>,
    account_deadline: bool,
) -> bool {
    let started = Instant::now();
    if account_deadline {
        scheduler.record_start(started);
    }
    let outcome = collector.collect();
    let completed = Instant::now();
    if account_deadline {
        scheduler.complete_scheduled_work(completed);
    }
    let event = match outcome {
        Ok(sample) => CollectorEvent::Sample(Arc::new(sample)),
        Err(CollectionFailure::Unavailable(error)) => CollectorEvent::Unavailable(error),
        Err(CollectionFailure::Fatal(message)) => CollectorEvent::Fatal {
            code: "collector_fatal".to_string(),
            message,
        },
    };
    let fatal = matches!(event, CollectorEvent::Fatal { .. });
    *revision = publish(
        published,
        notify,
        revision.saturating_add(1),
        completed,
        event,
        completed.saturating_duration_since(started).as_secs_f64() * 1_000.0,
        scheduler.cadence(completed),
    )
    .revision;
    fatal
}

fn publish(
    published: &Arc<RwLock<Option<Arc<CollectorPublication>>>>,
    notify: &Arc<dyn Fn() + Send + Sync>,
    revision: u64,
    completed_at: Instant,
    event: CollectorEvent,
    collection_latency_ms: f64,
    cadence: CollectorCadence,
) -> Arc<CollectorPublication> {
    let publication = Arc::new(CollectorPublication {
        revision,
        completed_at,
        event,
        collection_latency_ms,
        cadence,
    });
    *published
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&publication));
    notify();
    publication
}

fn fatal_control_loop(receiver: &Receiver<CollectorControl>, shutdown_started: &AtomicBool) {
    while !shutdown_started.load(AtomicOrdering::Acquire) {
        let control = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(control) => control,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match control {
            CollectorControl::Shutdown => return,
            CollectorControl::Refresh => {}
            CollectorControl::Pause(reply)
            | CollectorControl::SetInterval(_, reply)
            | CollectorControl::RetryProcessNetwork(reply) => {
                let _ = reply.send(Err("collector_engine_fatal".to_string()));
            }
            CollectorControl::Resume(reply) => {
                let _ = reply.send(Err("collector_engine_fatal".to_string()));
            }
            CollectorControl::ProcessNetworkReady(reply) => {
                let _ = reply.send(Err("collector_engine_fatal".to_string()));
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
    gate.started_generation = gate.started_generation.saturating_add(1);
    Some(gate.started_generation)
}

fn complete_refresh_generation(
    refresh_gate: &RefreshGate,
    generation: u64,
    publication: Arc<CollectorPublication>,
) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.completed_generation = generation;
    gate.completed_publication = Some((generation, publication));
    refresh_gate.changed.notify_all();
}

fn fail_refresh_generation(refresh_gate: &RefreshGate, generation: u64, error: String) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.failed_generation = Some((generation, error));
    gate.wake_queued = false;
    refresh_gate.changed.notify_all();
}

fn finish_refresh_wait(refresh_gate: &RefreshGate) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.waiting_callers = gate.waiting_callers.saturating_sub(1);
    if gate.waiting_callers == 0 {
        gate.completed_publication = None;
        gate.failed_generation = None;
    }
}

fn terminate_refresh_gate(refresh_gate: &RefreshGate, error: String) {
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gate.terminal_error = Some(error);
    gate.wake_queued = false;
    refresh_gate.changed.notify_all();
}

fn wait_for_refresh_generation(
    refresh_gate: &RefreshGate,
    target_generation: u64,
) -> Result<Arc<CollectorPublication>, String> {
    let deadline = Instant::now() + CONTROL_RESPONSE_TIMEOUT;
    let mut gate = refresh_gate
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        if let Some(error) = &gate.terminal_error {
            return Err(error.clone());
        }
        if gate.completed_generation >= target_generation {
            return gate
                .completed_publication
                .as_ref()
                .filter(|(generation, _)| *generation >= target_generation)
                .map(|(_, publication)| Arc::clone(publication))
                .ok_or_else(|| "collector_refresh_result_unavailable".to_string());
        }
        if let Some((generation, error)) = &gate.failed_generation {
            if *generation >= target_generation {
                return Err(error.clone());
            }
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err("collector_refresh_timeout".to_string());
        }
        let (next, result) = refresh_gate
            .changed
            .wait_timeout(gate, timeout)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gate = next;
        if result.timed_out() {
            return Err("collector_refresh_timeout".to_string());
        }
    }
}

struct TimedCountWindow {
    window: Duration,
    values: VecDeque<(Instant, u64)>,
}

impl TimedCountWindow {
    fn new(window: Duration) -> Self {
        Self {
            window,
            values: VecDeque::new(),
        }
    }

    fn add_at(&mut self, at: Instant, value: u64) {
        self.values.push_back((at, value));
        self.prune(at);
    }

    fn value_at(&mut self, now: Instant) -> u64 {
        self.prune(now);
        self.values.iter().map(|(_, value)| value).sum()
    }

    fn prune(&mut self, now: Instant) {
        while self
            .values
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > self.window)
        {
            self.values.pop_front();
        }
    }
}

struct TimedP95Window {
    window: Duration,
    values: VecDeque<(Instant, f64)>,
}

impl TimedP95Window {
    fn new(window: Duration) -> Self {
        Self {
            window,
            values: VecDeque::new(),
        }
    }

    fn add_at(&mut self, at: Instant, value: f64) {
        self.values.push_back((at, value));
        self.prune(at);
    }

    fn value_at(&mut self, now: Instant) -> f64 {
        self.prune(now);
        let mut values = self
            .values
            .iter()
            .map(|(_, value)| *value)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(f64::total_cmp);
        let index = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        values[index.min(values.len() - 1)]
    }

    fn prune(&mut self, now: Instant) {
        while self
            .values
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > self.window)
        {
            self.values.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{RuntimeCollectorState, SystemMetricsSnapshot},
        runtime_store::empty_system,
    };
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
        shutdown_error: Option<&'static str>,
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
                    shutdown_error: None,
                },
                collect_count,
            )
        }
    }

    impl RawCollector for FakeCollector {
        fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure> {
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
                FakeOutcome::Sample => Ok(TelemetrySample {
                    latency_ms: 0,
                    collector_state: RuntimeCollectorState::Healthy,
                    system: SystemMetricsSnapshot {
                        process_count: count,
                        ..empty_system()
                    },
                    processes: Vec::new(),
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

        fn process_network_ready(&self) -> Result<bool, String> {
            Ok(false)
        }

        fn retry_process_network(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), String> {
            self.shutdown_error
                .take()
                .map_or(Ok(()), |error| Err(error.to_string()))
        }
    }

    impl Drop for FakeCollector {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    fn manual_engine(collector: FakeCollector) -> CollectorEngine {
        CollectorEngine::start(
            Box::new(collector),
            CollectorEngineConfig {
                interval: Duration::from_millis(500),
                metric_window: Duration::from_secs(15),
                paused: false,
                automatic: false,
            },
            Arc::new(|| {}),
        )
        .expect("collector engine starts")
    }

    #[test]
    fn zero_interval_is_rejected_at_start_and_update_without_stranding_shutdown() {
        let (collector, start_collect_count) = FakeCollector::new([FakeOutcome::Sample]);
        let start_error = CollectorEngine::start(
            Box::new(collector),
            CollectorEngineConfig {
                interval: Duration::ZERO,
                metric_window: Duration::from_secs(15),
                paused: false,
                automatic: true,
            },
            Arc::new(|| {}),
        )
        .err()
        .expect("zero interval is rejected before the worker starts");
        assert_eq!(start_error, "collector_interval_must_be_positive");
        assert_eq!(start_collect_count.load(TestOrdering::SeqCst), 0);

        let (collector, update_collect_count) = FakeCollector::new([FakeOutcome::Sample]);
        let engine = manual_engine(collector);
        assert_eq!(
            engine.handle().set_interval(Duration::ZERO),
            Err("collector_interval_must_be_positive".to_string())
        );
        engine
            .handle()
            .refresh_now()
            .expect("rejected update leaves the engine usable");
        assert_eq!(update_collect_count.load(TestOrdering::SeqCst), 1);
        let shutdown_started = Instant::now();
        engine
            .shutdown()
            .expect("rejected zero interval cannot strand shutdown");
        assert!(shutdown_started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn collector_shutdown_failure_is_propagated_after_the_worker_settles() {
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Sample]);
        collector.shutdown_error = Some("scripted_collector_shutdown_failed");
        let engine = manual_engine(collector);

        assert_eq!(
            engine.shutdown(),
            Err("scripted_collector_shutdown_failed".to_string())
        );
    }

    #[test]
    fn absolute_scheduler_skips_missed_deadlines_without_work_sleep_drift() {
        let base = Instant::now();
        for interval_ms in [500_u64, 1_000, 2_000, 5_000] {
            let interval = Duration::from_millis(interval_ms);
            let mut scheduler = AbsoluteScheduler::new(
                base,
                CollectorEngineConfig {
                    interval,
                    metric_window: Duration::from_secs(600),
                    paused: false,
                    automatic: true,
                },
            );
            scheduler.record_start(base);
            scheduler.complete_scheduled_work(base + Duration::from_millis(123));
            assert_eq!(scheduler.next_deadline, base + interval);
            scheduler.record_start(base + interval);
            scheduler.complete_scheduled_work(base + interval + Duration::from_millis(77));
            assert_eq!(scheduler.next_deadline, base + interval + interval);
            assert_eq!(scheduler.deadline_misses, 0);
        }

        let mut slow = AbsoluteScheduler::new(
            base,
            CollectorEngineConfig {
                interval: Duration::from_millis(500),
                metric_window: Duration::from_secs(600),
                paused: false,
                automatic: true,
            },
        );
        slow.record_start(base);
        slow.complete_scheduled_work(base + Duration::from_millis(1_250));
        assert_eq!(slow.next_deadline, base + Duration::from_millis(1_500));
        assert_eq!(slow.deadline_misses, 2);
    }

    #[test]
    fn immutable_snapshot_reads_do_not_wait_for_a_slow_collector() {
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Sample, FakeOutcome::Sample]);
        collector.first_started = Some(started_tx);
        collector.first_release = Some(Arc::clone(&release));
        let engine = Arc::new(manual_engine(collector));
        let handle = engine.handle();
        let refresh_handle = handle.clone();
        let refresh = std::thread::spawn(move || refresh_handle.refresh_now());
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("collector starts");

        let read_started = Instant::now();
        assert!(handle.snapshot().is_none());
        assert!(read_started.elapsed() < Duration::from_millis(100));
        let (ready, changed) = &*release;
        *ready.lock().expect("release lock") = true;
        changed.notify_all();
        refresh
            .join()
            .expect("refresh joins")
            .expect("refresh succeeds");
        assert!(handle.snapshot().is_some());
        engine.shutdown().expect("engine joins");
    }

    #[test]
    fn refreshes_arriving_during_collection_coalesce_to_one_next_generation() {
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (mut collector, collect_count) =
            FakeCollector::new([FakeOutcome::Sample, FakeOutcome::Sample]);
        collector.first_started = Some(started_tx);
        collector.first_release = Some(Arc::clone(&release));
        let engine = Arc::new(manual_engine(collector));
        let first_handle = engine.handle();
        let first = std::thread::spawn(move || first_handle.refresh_now());
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first collection starts");

        let barrier = Arc::new(Barrier::new(9));
        let callers = (0..8)
            .map(|_| {
                let handle = engine.handle();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    handle.refresh_now()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let registration_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let waiting = engine
                .handle
                .refresh_gate
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .waiting_callers;
            if waiting == 9 {
                break;
            }
            assert!(Instant::now() < registration_deadline);
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
        engine.shutdown().expect("engine joins");
    }

    #[test]
    fn unavailable_recovers_fatal_is_durable_and_shutdown_joins_owner() {
        let (mut collector, _) = FakeCollector::new([
            FakeOutcome::Unavailable("collector offline"),
            FakeOutcome::Sample,
            FakeOutcome::Fatal("collector poisoned"),
        ]);
        let (dropped_tx, dropped_rx) = mpsc::channel();
        collector.dropped = Some(dropped_tx);
        let engine = manual_engine(collector);
        let handle = engine.handle();

        assert!(matches!(
            handle.refresh_now().expect("unavailable publishes").event,
            CollectorEvent::Unavailable(_)
        ));
        assert!(matches!(
            handle.refresh_now().expect("collector recovers").event,
            CollectorEvent::Sample(_)
        ));
        assert_eq!(
            handle.refresh_now().expect_err("fatal stops refresh"),
            "collector_engine_fatal"
        );
        assert!(matches!(
            handle.snapshot().expect("fatal snapshot").event,
            CollectorEvent::Fatal { .. }
        ));
        engine.shutdown().expect("fatal engine joins");
        dropped_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("collector drops before shutdown returns");
    }

    #[test]
    fn collector_panic_is_published_and_joined() {
        let (mut collector, _) = FakeCollector::new([FakeOutcome::Panic]);
        let (dropped_tx, dropped_rx) = mpsc::channel();
        collector.dropped = Some(dropped_tx);
        let engine = manual_engine(collector);
        let handle = engine.handle();
        assert_eq!(
            handle.refresh_now().expect_err("panic stops refresh"),
            "collector_engine_fatal"
        );
        let fatal = handle.snapshot().expect("panic snapshot remains readable");
        assert!(matches!(
            &fatal.event,
            CollectorEvent::Fatal { code, .. } if code == "sampling_engine_panicked"
        ));
        engine.shutdown().expect("panic engine joins");
        dropped_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("collector drops before shutdown returns");
    }
}
