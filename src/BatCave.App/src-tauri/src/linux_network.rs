#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::network_attribution::{NetworkAttributionSample, ProcessNetworkRates};

const IPV4_SOCKET_FAMILY: u16 = 2;
const IPV6_SOCKET_FAMILY: u16 = 10;
const BPFTRACE_INTERVAL: Duration = Duration::from_secs(1);
const STARTUP_LIVENESS_TIMEOUT: Duration = Duration::from_secs(10);
const INTERVAL_LIVENESS_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_START_ATTEMPTS: u8 = 3;
const EPOCH_GRACE_INTERVALS: u64 = 1;
const BPFTRACE_MAX_MAP_KEYS: usize = 16_384;
const MAX_READER_ERROR_CHARS: usize = 2_048;
const INTERVAL_MARKER: &str = "BATCAVE_NETWORK_INTERVAL";
const RX_BEGIN: &str = "BATCAVE_NETWORK_RX_BEGIN";
const RX_ENTRY: &str = "BATCAVE_NETWORK_RX";
const RX_END: &str = "BATCAVE_NETWORK_RX_END";
const TX_BEGIN: &str = "BATCAVE_NETWORK_TX_BEGIN";
const TX_ENTRY: &str = "BATCAVE_NETWORK_TX";
const TX_END: &str = "BATCAVE_NETWORK_TX_END";
const BPFTRACE_SCRIPT: &str = include_str!("../bpftrace/linux-network-attribution.bt");

fn bpftrace_script() -> &'static str {
    BPFTRACE_SCRIPT
}

fn bpftrace_command() -> Command {
    let mut command = Command::new("bpftrace");
    command
        .arg("-q")
        .arg("-B")
        .arg("line")
        .arg("-e")
        .arg(bpftrace_script());
    command
}

#[derive(Debug)]
pub struct LinuxNetworkAttribution {
    state: LinuxNetworkAttributionState,
}

impl LinuxNetworkAttribution {
    pub fn new() -> Self {
        let now = Instant::now();
        Self::from_start_result(LinuxNetworkAttributionMonitor::start(), now)
    }

    fn from_start_result(
        result: Result<LinuxNetworkAttributionMonitor, String>,
        now: Instant,
    ) -> Self {
        let state = match result {
            Ok(monitor) => LinuxNetworkAttributionState::Ready(ReadyState {
                monitor,
                attempts: 1,
            }),
            Err(message) => {
                LinuxNetworkAttributionState::Failed(FailureState::after_attempt(message, 1, now))
            }
        };
        Self { state }
    }

    pub fn sample(&mut self) -> NetworkAttributionSample {
        let mut start = LinuxNetworkAttributionMonitor::start;
        self.sample_at_with(Instant::now(), &mut start)
    }

    fn sample_at_with<F>(&mut self, now: Instant, start: &mut F) -> NetworkAttributionSample
    where
        F: FnMut() -> Result<LinuxNetworkAttributionMonitor, String>,
    {
        let runtime_failure = match &mut self.state {
            LinuxNetworkAttributionState::Ready(ready) => match ready.monitor.sample() {
                NetworkAttributionSample::Ready { rates_by_pid } => {
                    ready.attempts = 0;
                    return NetworkAttributionSample::Ready { rates_by_pid };
                }
                NetworkAttributionSample::Failed(message) => Some((message, ready.attempts.max(1))),
                sample => return sample,
            },
            LinuxNetworkAttributionState::Failed(_) => None,
        };

        if let Some((message, attempts)) = runtime_failure {
            self.state = LinuxNetworkAttributionState::Failed(FailureState::after_runtime_failure(
                message, attempts, now,
            ));
        }
        self.retry_failed_with(now, start)
    }

    fn retry_failed_with<F>(&mut self, now: Instant, start: &mut F) -> NetworkAttributionSample
    where
        F: FnMut() -> Result<LinuxNetworkAttributionMonitor, String>,
    {
        let failure = match &self.state {
            LinuxNetworkAttributionState::Ready(_) => {
                return NetworkAttributionSample::Held(
                    "Linux eBPF network attribution is warming up.".to_string(),
                );
            }
            LinuxNetworkAttributionState::Failed(failure) => failure.clone(),
        };

        if !failure.retry_due(now) {
            return NetworkAttributionSample::Failed(failure.observable_message(now));
        }

        let attempt = failure.attempts.saturating_add(1);
        match start() {
            Ok(monitor) => {
                self.state = LinuxNetworkAttributionState::Ready(ReadyState {
                    monitor,
                    attempts: attempt,
                });
                NetworkAttributionSample::Held(format!(
                    "Linux eBPF network attribution restarted on attempt {attempt}/{MAX_START_ATTEMPTS} and is warming up."
                ))
            }
            Err(message) => {
                let failure = FailureState::after_attempt(message, attempt, now);
                let sample = NetworkAttributionSample::Failed(failure.observable_message(now));
                self.state = LinuxNetworkAttributionState::Failed(failure);
                sample
            }
        }
    }
}

#[derive(Debug)]
enum LinuxNetworkAttributionState {
    Ready(ReadyState),
    Failed(FailureState),
}

#[derive(Debug)]
struct ReadyState {
    monitor: LinuxNetworkAttributionMonitor,
    attempts: u8,
}

#[derive(Debug, Clone)]
struct FailureState {
    message: String,
    attempts: u8,
    retry_at: Option<Instant>,
}

impl FailureState {
    fn after_runtime_failure(message: String, attempts: u8, now: Instant) -> Self {
        Self::after_attempt(message, attempts.max(1), now)
    }

    fn after_attempt(message: String, attempts: u8, now: Instant) -> Self {
        let attempts = attempts.min(MAX_START_ATTEMPTS);
        Self {
            message,
            attempts,
            retry_at: (attempts < MAX_START_ATTEMPTS).then_some(now + RETRY_DELAY),
        }
    }

    fn retry_due(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|retry_at| now >= retry_at)
    }

    fn observable_message(&self, now: Instant) -> String {
        match self.retry_at {
            Some(retry_at) => format!(
                "{}; retry_state=waiting retry_attempts={} retry_limit={} retry_in_ms={}",
                self.message,
                self.attempts,
                MAX_START_ATTEMPTS,
                retry_at.saturating_duration_since(now).as_millis()
            ),
            None => format!(
                "{}; retry_state=exhausted retry_attempts={} retry_limit={}",
                self.message, self.attempts, MAX_START_ATTEMPTS
            ),
        }
    }
}

#[derive(Debug)]
struct LinuxNetworkAttributionMonitor {
    child: Child,
    shared: Arc<Mutex<LinuxNetworkShared>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    stopped: bool,
}

impl LinuxNetworkAttributionMonitor {
    fn start() -> Result<Self, String> {
        ensure_bpftrace_available()?;
        Self::spawn(bpftrace_command())
    }

    fn spawn(mut command: Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("linux_network_ebpf_start_failed:{error}"))?;

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_child(&mut child);
                return Err("linux_network_ebpf_stdout_unavailable".to_string());
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_child(&mut child);
                return Err("linux_network_ebpf_stderr_unavailable".to_string());
            }
        };

        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let stdout_shared = Arc::clone(&shared);
        let stdout_thread = match thread::Builder::new()
            .name("batcave-linux-network-ebpf-stdout".to_string())
            .spawn(move || read_bpftrace_stdout(stdout, stdout_shared))
        {
            Ok(thread) => thread,
            Err(error) => {
                terminate_child(&mut child);
                return Err(format!("linux_network_ebpf_stdout_thread_failed:{error}"));
            }
        };

        let stderr_shared = Arc::clone(&shared);
        let stderr_thread = match thread::Builder::new()
            .name("batcave-linux-network-ebpf-stderr".to_string())
            .spawn(move || read_bpftrace_stderr(stderr, stderr_shared))
        {
            Ok(thread) => thread,
            Err(error) => {
                terminate_child(&mut child);
                let _ = stdout_thread.join();
                return Err(format!("linux_network_ebpf_stderr_thread_failed:{error}"));
            }
        };

        Ok(Self {
            child,
            shared,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            stopped: false,
        })
    }

    fn sample(&mut self) -> NetworkAttributionSample {
        if self.stopped {
            return NetworkAttributionSample::Failed(
                "linux_network_ebpf_monitor_stopped".to_string(),
            );
        }

        match self.child.try_wait() {
            Ok(Some(status)) => {
                let message = self
                    .shared
                    .lock()
                    .ok()
                    .and_then(|shared| shared.last_error.clone())
                    .unwrap_or_else(|| "bpftrace exited without stderr output".to_string());
                return NetworkAttributionSample::Failed(format!(
                    "linux_network_ebpf_exited:{status}; {message}"
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return NetworkAttributionSample::Failed(format!(
                    "linux_network_ebpf_status_failed:{error}"
                ));
            }
        }

        let stdout_finished = self
            .stdout_thread
            .as_ref()
            .is_none_or(JoinHandle::is_finished);
        let stderr_finished = self
            .stderr_thread
            .as_ref()
            .is_none_or(JoinHandle::is_finished);
        let mut shared = match self.shared.lock() {
            Ok(shared) => shared,
            Err(_) => {
                return NetworkAttributionSample::Failed(
                    "linux_network_ebpf_state_lock_poisoned".to_string(),
                );
            }
        };
        if let Some(message) =
            shared.reader_failure(Instant::now(), stdout_finished, stderr_finished)
        {
            return NetworkAttributionSample::Failed(message);
        }

        match shared.completed.take_rates() {
            Some(completed) => {
                debug_assert!(completed.interval_count > 0);
                NetworkAttributionSample::Ready {
                    rates_by_pid: completed.rates_by_pid,
                }
            }
            None if shared.last_interval_at.is_none() => NetworkAttributionSample::Held(
                "Linux eBPF network attribution is warming up.".to_string(),
            ),
            None => NetworkAttributionSample::Held(
                "Linux eBPF network attribution is waiting for a completed interval.".to_string(),
            ),
        }
    }

    fn shutdown(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;

        let mut errors = Vec::new();
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = self.child.kill() {
                    errors.push(format!("linux_network_ebpf_kill_failed:{error}"));
                }
                if let Err(error) = self.child.wait() {
                    errors.push(format!("linux_network_ebpf_wait_failed:{error}"));
                }
            }
            Err(error) => {
                errors.push(format!("linux_network_ebpf_status_failed:{error}"));
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }

        if self
            .stdout_thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            errors.push("linux_network_ebpf_stdout_join_failed".to_string());
        }
        if self
            .stderr_thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            errors.push("linux_network_ebpf_stderr_join_failed".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join(";"))
        }
    }
}

impl Drop for LinuxNetworkAttributionMonitor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReaderState {
    Starting,
    Running,
    Closed,
    Failed(String),
}

#[derive(Debug, Clone, Copy)]
enum ReaderKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
struct LinuxNetworkShared {
    started_at: Instant,
    last_interval_at: Option<Instant>,
    completed: CompletedIntervalAccumulator,
    stdout_state: ReaderState,
    stderr_state: ReaderState,
    last_error: Option<String>,
}

impl LinuxNetworkShared {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            last_interval_at: None,
            completed: CompletedIntervalAccumulator::default(),
            stdout_state: ReaderState::Starting,
            stderr_state: ReaderState::Starting,
            last_error: None,
        }
    }

    fn reader_state_mut(&mut self, kind: ReaderKind) -> &mut ReaderState {
        match kind {
            ReaderKind::Stdout => &mut self.stdout_state,
            ReaderKind::Stderr => &mut self.stderr_state,
        }
    }

    fn mark_reader_running(&mut self, kind: ReaderKind) {
        *self.reader_state_mut(kind) = ReaderState::Running;
    }

    fn mark_reader_closed(&mut self, kind: ReaderKind) {
        let state = self.reader_state_mut(kind);
        if !matches!(state, ReaderState::Failed(_)) {
            *state = ReaderState::Closed;
        }
    }

    fn mark_reader_failed(&mut self, kind: ReaderKind, message: String) {
        let message = match self.reader_state_mut(kind) {
            ReaderState::Failed(existing) => format!("{existing}; {message}"),
            _ => message,
        }
        .chars()
        .take(MAX_READER_ERROR_CHARS)
        .collect::<String>();
        self.last_error = Some(message.clone());
        *self.reader_state_mut(kind) = ReaderState::Failed(message);
    }

    fn reader_failure(
        &self,
        now: Instant,
        stdout_finished: bool,
        stderr_finished: bool,
    ) -> Option<String> {
        for (name, state, finished) in [
            ("stdout", &self.stdout_state, stdout_finished),
            ("stderr", &self.stderr_state, stderr_finished),
        ] {
            match state {
                ReaderState::Failed(message) => return Some(message.clone()),
                ReaderState::Closed => {
                    return Some(format!("linux_network_ebpf_{name}_closed"));
                }
                ReaderState::Starting | ReaderState::Running if finished => {
                    return Some(format!("linux_network_ebpf_{name}_thread_stopped"));
                }
                ReaderState::Starting | ReaderState::Running => {}
            }
        }

        match self.last_interval_at {
            Some(last_interval_at)
                if now.saturating_duration_since(last_interval_at) > INTERVAL_LIVENESS_TIMEOUT =>
            {
                Some("linux_network_ebpf_stdout_stalled".to_string())
            }
            None if now.saturating_duration_since(self.started_at) > STARTUP_LIVENESS_TIMEOUT => {
                Some("linux_network_ebpf_stdout_startup_stalled".to_string())
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PendingRates {
    received_by_pid: HashMap<u32, u64>,
    transmitted_by_pid: HashMap<u32, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolStage {
    AwaitingRxBegin,
    ReadingRx,
    AwaitingTxBegin,
    ReadingTx,
    AwaitingMarker,
}

impl ProtocolStage {
    const fn label(self) -> &'static str {
        match self {
            Self::AwaitingRxBegin => "awaiting_rx_begin",
            Self::ReadingRx => "reading_rx",
            Self::AwaitingTxBegin => "awaiting_tx_begin",
            Self::ReadingTx => "reading_tx",
            Self::AwaitingMarker => "awaiting_marker",
        }
    }
}

#[derive(Debug)]
struct ProtocolAssembler {
    expected_epoch: u64,
    stage: ProtocolStage,
    section_count: u64,
    section_total: u64,
    pending: PendingRates,
}

impl Default for ProtocolAssembler {
    fn default() -> Self {
        Self {
            expected_epoch: 1,
            stage: ProtocolStage::AwaitingRxBegin,
            section_count: 0,
            section_total: 0,
            pending: PendingRates::default(),
        }
    }
}

#[derive(Debug, Default)]
struct CompletedIntervalAccumulator {
    interval_count: u64,
    received_by_pid: HashMap<u32, u64>,
    transmitted_by_pid: HashMap<u32, u64>,
}

impl CompletedIntervalAccumulator {
    fn push(&mut self, pending: &mut PendingRates) {
        self.interval_count = self.interval_count.saturating_add(1);
        for (pid, bytes) in pending.received_by_pid.drain() {
            let total = self.received_by_pid.entry(pid).or_default();
            *total = total.saturating_add(bytes);
        }
        for (pid, bytes) in pending.transmitted_by_pid.drain() {
            let total = self.transmitted_by_pid.entry(pid).or_default();
            *total = total.saturating_add(bytes);
        }
    }

    fn take_rates(&mut self) -> Option<CompletedRates> {
        let interval_count = std::mem::take(&mut self.interval_count);
        if interval_count == 0 {
            return None;
        }

        let mut rates_by_pid = HashMap::new();
        for (pid, bytes) in self.received_by_pid.drain() {
            rates_by_pid
                .entry(pid)
                .or_insert_with(ProcessNetworkRates::default)
                .received_bps = average_bytes_per_second(bytes, interval_count);
        }
        for (pid, bytes) in self.transmitted_by_pid.drain() {
            rates_by_pid
                .entry(pid)
                .or_insert_with(ProcessNetworkRates::default)
                .transmitted_bps = average_bytes_per_second(bytes, interval_count);
        }

        Some(CompletedRates {
            interval_count,
            rates_by_pid,
        })
    }
}

#[derive(Debug)]
struct CompletedRates {
    interval_count: u64,
    rates_by_pid: HashMap<u32, ProcessNetworkRates>,
}

fn average_bytes_per_second(bytes: u64, interval_count: u64) -> u64 {
    let interval_ms = BPFTRACE_INTERVAL
        .as_millis()
        .saturating_mul(interval_count.into());
    let rate = u128::from(bytes)
        .saturating_mul(1_000)
        .checked_div(interval_ms.max(1))
        .unwrap_or_default();
    u64::try_from(rate).unwrap_or(u64::MAX)
}

fn ensure_bpftrace_available() -> Result<(), String> {
    match Command::new("bpftrace")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("linux_network_ebpf_bpftrace_unavailable:{status}")),
        Err(error) => Err(format!("linux_network_ebpf_bpftrace_not_found:{error}")),
    }
}

fn read_bpftrace_stdout(stdout: impl Read, shared: Arc<Mutex<LinuxNetworkShared>>) {
    mark_reader_running(&shared, ReaderKind::Stdout);
    let mut protocol = ProtocolAssembler::default();
    let mut reader = BufReader::new(stdout);
    let mut accepting_protocol = true;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                mark_reader_closed(&shared, ReaderKind::Stdout);
                return;
            }
            Ok(_) if accepting_protocol => {
                if let Err(message) = ingest_bpftrace_stdout_line(&line, &mut protocol, &shared) {
                    mark_reader_failed(&shared, ReaderKind::Stdout, message);
                    accepting_protocol = false;
                }
            }
            Ok(_) => {}
            Err(error) => {
                mark_reader_failed(
                    &shared,
                    ReaderKind::Stdout,
                    format!("linux_network_ebpf_stdout_read_failed:{error}"),
                );
                return;
            }
        }
    }
}

fn read_bpftrace_stderr(stderr: impl Read, shared: Arc<Mutex<LinuxNetworkShared>>) {
    mark_reader_running(&shared, ReaderKind::Stderr);
    let mut reader = BufReader::new(stderr);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                mark_reader_closed(&shared, ReaderKind::Stderr);
                return;
            }
            Ok(_) => {
                let message = line.trim();
                if !message.is_empty() {
                    mark_reader_failed(
                        &shared,
                        ReaderKind::Stderr,
                        format!("linux_network_ebpf_stderr:{message}"),
                    );
                }
            }
            Err(error) => {
                mark_reader_failed(
                    &shared,
                    ReaderKind::Stderr,
                    format!("linux_network_ebpf_stderr_read_failed:{error}"),
                );
                return;
            }
        }
    }
}

fn mark_reader_running(shared: &Arc<Mutex<LinuxNetworkShared>>, kind: ReaderKind) {
    if let Ok(mut shared) = shared.lock() {
        shared.mark_reader_running(kind);
    }
}

fn mark_reader_closed(shared: &Arc<Mutex<LinuxNetworkShared>>, kind: ReaderKind) {
    if let Ok(mut shared) = shared.lock() {
        shared.mark_reader_closed(kind);
    }
}

fn mark_reader_failed(shared: &Arc<Mutex<LinuxNetworkShared>>, kind: ReaderKind, message: String) {
    if let Ok(mut shared) = shared.lock() {
        shared.mark_reader_failed(kind, message);
    }
}

fn ingest_bpftrace_stdout_line(
    line: &str,
    protocol: &mut ProtocolAssembler,
    shared: &Arc<Mutex<LinuxNetworkShared>>,
) -> Result<(), String> {
    let line = line.trim();
    let completed = protocol.ingest(line)?;
    if let Some(mut completed) = completed {
        let mut shared = shared
            .lock()
            .map_err(|_| "linux_network_ebpf_state_lock_poisoned".to_string())?;
        shared.completed.push(&mut completed);
        shared.last_interval_at = Some(Instant::now());
    }
    Ok(())
}

impl ProtocolAssembler {
    fn ingest(&mut self, line: &str) -> Result<Option<PendingRates>, String> {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(kind) = fields.first().copied() else {
            return Err(self.error("empty_line"));
        };

        match kind {
            RX_BEGIN => {
                self.require_field_count(&fields, 2)?;
                let epoch = self.parse_epoch(fields[1])?;
                self.require_stage(ProtocolStage::AwaitingRxBegin, kind)?;
                self.require_epoch(epoch)?;
                self.stage = ProtocolStage::ReadingRx;
                self.reset_section();
                Ok(None)
            }
            RX_ENTRY => {
                self.require_field_count(&fields, 4)?;
                let epoch = self.parse_epoch(fields[1])?;
                let pid = fields[2]
                    .parse::<u32>()
                    .map_err(|_| self.error("invalid_rx_pid"))?;
                let bytes = fields[3]
                    .parse::<u64>()
                    .map_err(|_| self.error("invalid_rx_bytes"))?;
                self.require_stage(ProtocolStage::ReadingRx, kind)?;
                self.require_epoch(epoch)?;
                if self.pending.received_by_pid.insert(pid, bytes).is_some() {
                    return Err(self.error(&format!("duplicate_rx_pid:{pid}")));
                }
                self.add_section_entry(bytes)?;
                Ok(None)
            }
            RX_END => {
                self.require_field_count(&fields, 5)?;
                let epoch = self.parse_epoch(fields[1])?;
                let count = self.parse_summary_value(fields[2], "invalid_rx_count")?;
                let total = self.parse_summary_value(fields[3], "invalid_rx_total")?;
                let stale = self.parse_summary_value(fields[4], "invalid_rx_stale_count")?;
                self.require_stage(ProtocolStage::ReadingRx, kind)?;
                self.require_epoch(epoch)?;
                self.require_section_summary(count, total, "rx")?;
                self.require_no_stale_entries(stale, "rx")?;
                self.stage = ProtocolStage::AwaitingTxBegin;
                self.reset_section();
                Ok(None)
            }
            TX_BEGIN => {
                self.require_field_count(&fields, 2)?;
                let epoch = self.parse_epoch(fields[1])?;
                self.require_stage(ProtocolStage::AwaitingTxBegin, kind)?;
                self.require_epoch(epoch)?;
                self.stage = ProtocolStage::ReadingTx;
                Ok(None)
            }
            TX_ENTRY => {
                self.require_field_count(&fields, 4)?;
                let epoch = self.parse_epoch(fields[1])?;
                let pid = fields[2]
                    .parse::<u32>()
                    .map_err(|_| self.error("invalid_tx_pid"))?;
                let bytes = fields[3]
                    .parse::<u64>()
                    .map_err(|_| self.error("invalid_tx_bytes"))?;
                self.require_stage(ProtocolStage::ReadingTx, kind)?;
                self.require_epoch(epoch)?;
                if self.pending.transmitted_by_pid.insert(pid, bytes).is_some() {
                    return Err(self.error(&format!("duplicate_tx_pid:{pid}")));
                }
                self.add_section_entry(bytes)?;
                Ok(None)
            }
            TX_END => {
                self.require_field_count(&fields, 5)?;
                let epoch = self.parse_epoch(fields[1])?;
                let count = self.parse_summary_value(fields[2], "invalid_tx_count")?;
                let total = self.parse_summary_value(fields[3], "invalid_tx_total")?;
                let stale = self.parse_summary_value(fields[4], "invalid_tx_stale_count")?;
                self.require_stage(ProtocolStage::ReadingTx, kind)?;
                self.require_epoch(epoch)?;
                self.require_section_summary(count, total, "tx")?;
                self.require_no_stale_entries(stale, "tx")?;
                self.stage = ProtocolStage::AwaitingMarker;
                self.reset_section();
                Ok(None)
            }
            INTERVAL_MARKER => {
                self.require_field_count(&fields, 2)?;
                let epoch = self.parse_epoch(fields[1])?;
                self.require_stage(ProtocolStage::AwaitingMarker, kind)?;
                self.require_epoch(epoch)?;
                self.expected_epoch = self
                    .expected_epoch
                    .checked_add(1)
                    .ok_or_else(|| self.error("epoch_overflow"))?;
                self.stage = ProtocolStage::AwaitingRxBegin;
                Ok(Some(std::mem::take(&mut self.pending)))
            }
            _ => {
                let excerpt = line.chars().take(160).collect::<String>();
                Err(self.error(&format!("malformed:{excerpt}")))
            }
        }
    }

    fn require_field_count(&self, fields: &[&str], expected: usize) -> Result<(), String> {
        if fields.len() == expected {
            Ok(())
        } else {
            Err(self.error(&format!(
                "invalid_field_count:actual={}:expected={expected}",
                fields.len()
            )))
        }
    }

    fn parse_epoch(&self, value: &str) -> Result<u64, String> {
        value
            .parse::<u64>()
            .map_err(|_| self.error("invalid_epoch"))
    }

    fn parse_summary_value(&self, value: &str, error: &str) -> Result<u64, String> {
        value.parse::<u64>().map_err(|_| self.error(error))
    }

    fn require_stage(&self, expected: ProtocolStage, component: &str) -> Result<(), String> {
        if self.stage == expected {
            Ok(())
        } else {
            Err(self.error(&format!(
                "unexpected_component:{component}:required_stage={}",
                expected.label()
            )))
        }
    }

    fn require_epoch(&self, epoch: u64) -> Result<(), String> {
        if epoch == self.expected_epoch {
            Ok(())
        } else {
            Err(self.error(&format!("unexpected_epoch:{epoch}")))
        }
    }

    fn add_section_entry(&mut self, bytes: u64) -> Result<(), String> {
        self.section_count = self
            .section_count
            .checked_add(1)
            .ok_or_else(|| self.error("section_count_overflow"))?;
        self.section_total = self
            .section_total
            .checked_add(bytes)
            .ok_or_else(|| self.error("section_total_overflow"))?;
        Ok(())
    }

    fn require_section_summary(
        &self,
        count: u64,
        total: u64,
        direction: &str,
    ) -> Result<(), String> {
        if count == self.section_count && total == self.section_total {
            Ok(())
        } else {
            Err(self.error(&format!(
                "{direction}_summary_mismatch:received_count={}:expected_count={count}:received_total={}:expected_total={total}",
                self.section_count, self.section_total
            )))
        }
    }

    fn require_no_stale_entries(&self, stale: u64, direction: &str) -> Result<(), String> {
        if stale == 0 {
            Ok(())
        } else {
            Err(self.error(&format!("late_{direction}_epoch_entries:{stale}")))
        }
    }

    fn reset_section(&mut self) {
        self.section_count = 0;
        self.section_total = 0;
    }

    fn error(&self, detail: &str) -> String {
        format!(
            "linux_network_ebpf_stdout_protocol:{detail}:expected_epoch={}:stage={}",
            self.expected_epoch,
            self.stage.label()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn bpftrace_scope_accepts_only_ipv4_and_ipv6_socket_families() {
        assert_eq!(IPV4_SOCKET_FAMILY, 2);
        assert_eq!(IPV6_SOCKET_FAMILY, 10);
        assert_ne!(IPV4_SOCKET_FAMILY, 1, "AF_UNIX must stay excluded");
        assert_ne!(IPV6_SOCKET_FAMILY, 1, "AF_UNIX must stay excluded");

        let script = bpftrace_script();
        assert!(script.contains("$family == 2 || $family == 10"));
        assert_eq!(script.matches("$family == 2 || $family == 10").count(), 2);
        assert_eq!(script.matches("(int64)retval > 0").count(), 2);
        assert_eq!(script.matches("sum((int64)retval)").count(), 2);
        assert!(!script.contains("$family == 1 ||"));
        assert!(!script.contains("|| $family == 1)"));
        assert!(script.contains("delete(@batcave_tx_family[tid])"));
        assert!(script.contains("delete(@batcave_rx_family[tid])"));
        assert!(script.contains("@batcave_tx[pid, @batcave_epoch] = sum"));
        assert!(script.contains("@batcave_rx[pid, @batcave_epoch] = sum"));
        assert_eq!(EPOCH_GRACE_INTERVALS, 1);
        assert_eq!(BPFTRACE_MAX_MAP_KEYS, 16_384);
        assert!(script.contains("max_map_keys=16384"));
        assert!(script.contains("@batcave_epoch = $closing_epoch + 1"));
        assert!(script.contains("$completed_epoch = $closing_epoch - 1"));
        assert!(script.contains("$kv.0.1 < $completed_epoch"));
        assert!(script.contains("$rx_stale++"));
        assert!(script.contains("$tx_stale++"));
        assert!(script.contains("for ($kv : @batcave_rx)"));
        assert!(script.contains("for ($kv : @batcave_tx)"));
        assert!(script.contains("delete(@batcave_rx, $kv.0)"));
        assert!(script.contains("delete(@batcave_tx, $kv.0)"));
        assert!(script.contains("print_maps_on_exit=0"));
        assert!(!script.contains("print(@batcave_"));
        assert!(!script.contains("clear(@batcave_"));
        for component in [
            RX_BEGIN,
            RX_ENTRY,
            RX_END,
            TX_BEGIN,
            TX_ENTRY,
            TX_END,
            INTERVAL_MARKER,
        ] {
            assert!(
                script.contains(&format!("printf(\"{component} ")),
                "missing emitted component {component}"
            );
        }

        let flip = script.find("@batcave_epoch = $closing_epoch + 1").unwrap();
        let quarantine = script
            .find("$completed_epoch = $closing_epoch - 1")
            .unwrap();
        let first_output = script.find(&format!("printf(\"{RX_BEGIN}")).unwrap();
        assert!(
            flip < quarantine && quarantine < first_output,
            "the epoch must rotate and age before output"
        );
    }

    #[test]
    fn launcher_keeps_runtime_warnings_visible() {
        let command = bpftrace_command();
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(arguments.windows(2).any(|args| args == ["-B", "line"]));
        assert!(arguments.contains(&"-q".to_string()));
        assert!(!arguments.contains(&"--no-warnings".to_string()));
    }

    #[test]
    fn emitted_epoch_protocol_accumulates_validated_intervals() {
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let mut protocol = ProtocolAssembler::default();
        ingest_emitted_interval(
            &shared,
            &mut protocol,
            1,
            &[(10, 100)],
            &[(10, 200), (20, 300)],
        )
        .unwrap();
        ingest_emitted_interval(&shared, &mut protocol, 2, &[(10, 300)], &[(10, 400)]).unwrap();

        let sample = shared
            .lock()
            .unwrap()
            .completed
            .take_rates()
            .expect("completed sample");
        assert_eq!(sample.interval_count, 2);
        assert_eq!(sample.rates_by_pid[&10].received_bps, 200);
        assert_eq!(sample.rates_by_pid[&10].transmitted_bps, 300);
        assert_eq!(sample.rates_by_pid[&20].received_bps, 0);
        assert_eq!(sample.rates_by_pid[&20].transmitted_bps, 150);
        assert!(shared.lock().unwrap().completed.take_rates().is_none());
    }

    #[test]
    fn supported_app_cadences_consume_every_emitted_interval_once() {
        for cadence_ms in [500_u64, 1_000, 2_000, 5_000] {
            let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
            let mut protocol = ProtocolAssembler::default();
            let mut consumed_intervals = 0;
            let mut ready_samples = 0;
            for elapsed_ms in (500_u64..=10_000).step_by(500) {
                if elapsed_ms % 1_000 == 0 {
                    let epoch = elapsed_ms / 1_000;
                    ingest_emitted_interval(
                        &shared,
                        &mut protocol,
                        epoch,
                        &[(7, 1_000)],
                        &[(7, 2_000)],
                    )
                    .unwrap();
                }
                if elapsed_ms % cadence_ms == 0 {
                    let sample = shared.lock().unwrap().completed.take_rates();
                    if let Some(sample) = sample {
                        consumed_intervals += sample.interval_count;
                        ready_samples += 1;
                        assert_eq!(sample.rates_by_pid[&7].received_bps, 1_000);
                        assert_eq!(sample.rates_by_pid[&7].transmitted_bps, 2_000);
                    }
                }
            }

            assert_eq!(consumed_intervals, 10, "cadence_ms={cadence_ms}");
            assert_eq!(
                ready_samples,
                10_000 / cadence_ms.max(1_000),
                "cadence_ms={cadence_ms}"
            );
            assert!(shared.lock().unwrap().completed.take_rates().is_none());
        }
    }

    #[test]
    fn protocol_requires_one_rx_then_one_tx_section_before_marker() {
        for (name, lines) in [
            ("marker_only", vec![format!("{INTERVAL_MARKER} 1")]),
            ("missing_rx", vec![format!("{TX_BEGIN} 1")]),
            (
                "duplicate_rx",
                vec![format!("{RX_BEGIN} 1"), format!("{RX_BEGIN} 1")],
            ),
            (
                "missing_tx",
                vec![
                    format!("{RX_BEGIN} 1"),
                    format!("{RX_END} 1 0 0 0"),
                    format!("{INTERVAL_MARKER} 1"),
                ],
            ),
            (
                "duplicate_tx",
                vec![
                    format!("{RX_BEGIN} 1"),
                    format!("{RX_END} 1 0 0 0"),
                    format!("{TX_BEGIN} 1"),
                    format!("{TX_END} 1 0 0 0"),
                    format!("{TX_BEGIN} 1"),
                ],
            ),
            (
                "out_of_order_entry",
                vec![format!("{RX_BEGIN} 1"), format!("{TX_ENTRY} 1 4 10")],
            ),
        ] {
            let mut protocol = ProtocolAssembler::default();
            let error = lines
                .iter()
                .find_map(|line| protocol.ingest(line).err())
                .unwrap_or_else(|| panic!("{name} must fail"));
            assert!(error.contains("unexpected_component"), "{name}: {error}");
        }
    }

    #[test]
    fn protocol_detects_output_loss_duplicates_malformed_lines_and_epoch_gaps() {
        let cases = [
            (
                "lost_entry",
                vec![format!("{RX_BEGIN} 1"), format!("{RX_END} 1 1 50 0")],
                "summary_mismatch",
            ),
            (
                "duplicate_entry",
                vec![
                    format!("{RX_BEGIN} 1"),
                    format!("{RX_ENTRY} 1 4 10"),
                    format!("{RX_ENTRY} 1 4 10"),
                ],
                "duplicate_rx_pid",
            ),
            ("malformed", vec!["not protocol".to_string()], "malformed"),
            (
                "epoch_gap",
                vec![format!("{RX_BEGIN} 2")],
                "unexpected_epoch",
            ),
        ];

        for (name, lines, expected) in cases {
            let mut protocol = ProtocolAssembler::default();
            let error = lines
                .iter()
                .find_map(|line| protocol.ingest(line).err())
                .unwrap_or_else(|| panic!("{name} must fail"));
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn late_old_epoch_write_is_quarantined_then_fails_the_next_sweep() {
        assert_eq!(modeled_drain_epoch(1), None);
        assert_eq!(modeled_drain_epoch(2), Some(1));
        assert_eq!(modeled_drain_epoch(3), Some(2));

        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let mut protocol = ProtocolAssembler::default();
        ingest_emitted_interval(&shared, &mut protocol, 1, &[], &[]).unwrap();

        protocol.ingest(&format!("{RX_BEGIN} 2")).unwrap();
        let error = protocol
            .ingest(&format!("{RX_END} 2 0 0 1"))
            .expect_err("a late epoch-one key must fail the epoch-two sweep");
        assert!(error.contains("late_rx_epoch_entries:1"));
        assert_eq!(shared.lock().unwrap().completed.interval_count, 1);
    }

    #[test]
    fn explicit_empty_sections_publish_a_truthful_zero_interval() {
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let mut protocol = ProtocolAssembler::default();
        ingest_emitted_interval(&shared, &mut protocol, 1, &[], &[]).unwrap();

        let sample = shared
            .lock()
            .unwrap()
            .completed
            .take_rates()
            .expect("zero interval is still completed");
        assert_eq!(sample.interval_count, 1);
        assert!(sample.rates_by_pid.is_empty());
    }

    #[test]
    fn closed_malformed_and_stalled_readers_are_not_healthy_zero() {
        let started_at = Instant::now();
        let mut shared = LinuxNetworkShared::new(started_at);
        shared.stdout_state = ReaderState::Running;
        shared.stderr_state = ReaderState::Running;
        assert_eq!(
            shared.reader_failure(
                started_at + STARTUP_LIVENESS_TIMEOUT + Duration::from_millis(1),
                false,
                false,
            ),
            Some("linux_network_ebpf_stdout_startup_stalled".to_string())
        );

        shared.last_interval_at = Some(started_at);
        assert_eq!(
            shared.reader_failure(
                started_at + INTERVAL_LIVENESS_TIMEOUT + Duration::from_millis(1),
                false,
                false,
            ),
            Some("linux_network_ebpf_stdout_stalled".to_string())
        );

        shared.stdout_state = ReaderState::Closed;
        assert_eq!(
            shared.reader_failure(started_at, true, false),
            Some("linux_network_ebpf_stdout_closed".to_string())
        );

        shared.stdout_state = ReaderState::Failed("malformed".to_string());
        assert_eq!(
            shared.reader_failure(started_at, true, false),
            Some("malformed".to_string())
        );
    }

    #[test]
    fn stdout_read_error_and_stderr_output_are_explicit_failures() {
        let stdout_shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        read_bpftrace_stdout(AlwaysFails, Arc::clone(&stdout_shared));
        assert!(matches!(
            stdout_shared.lock().unwrap().stdout_state,
            ReaderState::Failed(ref message) if message.contains("stdout_read_failed")
        ));

        let stderr_shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        read_bpftrace_stderr(
            io::Cursor::new(b"permission denied\n"),
            Arc::clone(&stderr_shared),
        );
        assert!(matches!(
            stderr_shared.lock().unwrap().stderr_state,
            ReaderState::Failed(ref message) if message.contains("permission denied")
        ));
    }

    #[test]
    fn retry_budget_is_bounded_and_observable() {
        let started_at = Instant::now();
        let mut failure =
            FailureState::after_attempt("missing bpftrace".to_string(), 1, started_at);
        assert!(failure
            .observable_message(started_at)
            .contains("retry_state=waiting"));

        for attempt in 2..=MAX_START_ATTEMPTS {
            let retry_at = failure.retry_at.expect("retry remains scheduled");
            assert!(failure.retry_due(retry_at));
            failure =
                FailureState::after_attempt(format!("attempt {attempt} failed"), attempt, retry_at);
        }

        assert!(failure.retry_at.is_none());
        assert!(!failure.retry_due(started_at + Duration::from_secs(3_600)));
        let message = failure.observable_message(started_at);
        assert!(message.contains("retry_state=exhausted"));
        assert!(message.contains("retry_attempts=3"));
        assert!(message.contains("retry_limit=3"));
    }

    #[test]
    fn missing_dependency_is_immediately_explicit_and_nonfatal() {
        let mut attribution = LinuxNetworkAttribution::from_start_result(
            Err("linux_network_ebpf_bpftrace_not_found:fixture".to_string()),
            Instant::now(),
        );

        let NetworkAttributionSample::Failed(message) = attribution.sample() else {
            panic!("missing optional dependency must be unavailable");
        };
        assert!(message.contains("bpftrace_not_found"));
        assert!(message.contains("retry_state=waiting"));
        assert!(message.contains("retry_attempts=1"));
    }

    #[cfg(unix)]
    #[test]
    fn post_spawn_runtime_failures_preserve_attempts_until_retry_exhaustion() {
        let started_at = Instant::now();
        let initial = runtime_failure_monitor().expect("initial child spawns");
        let mut attribution = LinuxNetworkAttribution::from_start_result(Ok(initial), started_at);
        let mut restart = runtime_failure_monitor;

        let first = wait_for_runtime_failure(&mut attribution, started_at, &mut restart);
        assert!(first.contains("permission denied"));
        assert!(first.contains("retry_attempts=1"));

        for expected_attempt in 2..=MAX_START_ATTEMPTS {
            let retry_at = match &attribution.state {
                LinuxNetworkAttributionState::Failed(failure) => {
                    failure.retry_at.expect("retry remains")
                }
                LinuxNetworkAttributionState::Ready(_) => panic!("failure state expected"),
            };
            let restarted = attribution.sample_at_with(retry_at, &mut restart);
            assert!(matches!(restarted, NetworkAttributionSample::Held(_)));

            let failure = wait_for_runtime_failure(&mut attribution, retry_at, &mut restart);
            assert!(failure.contains(&format!("retry_attempts={expected_attempt}")));
        }

        let LinuxNetworkAttributionState::Failed(failure) = &attribution.state else {
            panic!("runtime failures must finish exhausted");
        };
        assert_eq!(failure.attempts, MAX_START_ATTEMPTS);
        assert!(failure.retry_at.is_none());
        assert!(failure
            .observable_message(started_at)
            .contains("retry_state=exhausted"));
    }

    #[cfg(unix)]
    #[test]
    fn complete_healthy_interval_resets_the_failure_episode() {
        let started_at = Instant::now();
        let monitor = healthy_interval_monitor().expect("healthy child spawns");
        let mut attribution = LinuxNetworkAttribution::from_start_result(Ok(monitor), started_at);
        let mut unexpected_restart = || Err("restart must not run".to_string());
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            match attribution.sample_at_with(started_at, &mut unexpected_restart) {
                NetworkAttributionSample::Ready { .. } => break,
                NetworkAttributionSample::Held(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                sample => panic!("healthy interval did not arrive: {sample:?}"),
            }
        }

        let LinuxNetworkAttributionState::Ready(ready) = &attribution.state else {
            panic!("healthy monitor must remain ready");
        };
        assert_eq!(ready.attempts, 0);
    }

    #[cfg(unix)]
    #[test]
    fn map_capacity_warning_fails_before_a_completed_interval_is_consumed() {
        let mut monitor = capacity_warning_monitor().expect("capacity fixture child spawns");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let ready_to_assert = monitor.shared.lock().is_ok_and(|shared| {
                shared.completed.interval_count == 1
                    && matches!(shared.stderr_state, ReaderState::Failed(_))
            });
            if ready_to_assert {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "capacity warning fixture stalled"
            );
            thread::sleep(Duration::from_millis(5));
        }

        let NetworkAttributionSample::Failed(message) = monitor.sample() else {
            panic!("capacity warnings must outrank a completed interval");
        };
        assert!(message.contains("Map full"));
        assert!(message.contains("map_update_elem"));
        assert_eq!(monitor.shared.lock().unwrap().completed.interval_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_reaps_child_and_joins_both_readers() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf 'BATCAVE_NETWORK_INTERVAL\\n'; sleep 60");
        let mut monitor = LinuxNetworkAttributionMonitor::spawn(command).unwrap();

        monitor.shutdown().expect("owned resources stop");

        assert!(monitor.child.try_wait().unwrap().is_some());
        assert!(monitor.stdout_thread.is_none());
        assert!(monitor.stderr_thread.is_none());
        assert!(monitor.stopped);
    }

    #[cfg(unix)]
    #[test]
    fn killed_child_transitions_to_failed_before_any_zero_sample() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("sleep 60");
        let mut monitor = LinuxNetworkAttributionMonitor::spawn(command).unwrap();
        monitor.child.kill().unwrap();
        monitor.child.wait().unwrap();

        let NetworkAttributionSample::Failed(message) = monitor.sample() else {
            panic!("killed child must fail attribution");
        };
        assert!(message.contains("linux_network_ebpf_exited:"));
    }

    fn ingest_emitted_interval(
        shared: &Arc<Mutex<LinuxNetworkShared>>,
        protocol: &mut ProtocolAssembler,
        epoch: u64,
        received: &[(u32, u64)],
        transmitted: &[(u32, u64)],
    ) -> Result<(), String> {
        let received_total = received.iter().map(|(_, bytes)| bytes).sum::<u64>();
        let transmitted_total = transmitted.iter().map(|(_, bytes)| bytes).sum::<u64>();
        let mut lines = vec![format!("{RX_BEGIN} {epoch}")];
        lines.extend(
            received
                .iter()
                .map(|(pid, bytes)| format!("{RX_ENTRY} {epoch} {pid} {bytes}")),
        );
        lines.push(format!(
            "{RX_END} {epoch} {} {received_total} 0",
            received.len()
        ));
        lines.push(format!("{TX_BEGIN} {epoch}"));
        lines.extend(
            transmitted
                .iter()
                .map(|(pid, bytes)| format!("{TX_ENTRY} {epoch} {pid} {bytes}")),
        );
        lines.push(format!(
            "{TX_END} {epoch} {} {transmitted_total} 0",
            transmitted.len()
        ));
        lines.push(format!("{INTERVAL_MARKER} {epoch}"));

        for line in lines {
            ingest_bpftrace_stdout_line(&line, protocol, shared)?;
        }
        Ok(())
    }

    fn modeled_drain_epoch(closing_epoch: u64) -> Option<u64> {
        closing_epoch
            .checked_sub(EPOCH_GRACE_INTERVALS)
            .filter(|epoch| *epoch > 0)
    }

    #[cfg(unix)]
    fn runtime_failure_monitor() -> Result<LinuxNetworkAttributionMonitor, String> {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf 'permission denied\\n' >&2; exec sleep 60");
        LinuxNetworkAttributionMonitor::spawn(command)
    }

    #[cfg(unix)]
    fn healthy_interval_monitor() -> Result<LinuxNetworkAttributionMonitor, String> {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(format!(
            "printf '%s\\n' '{RX_BEGIN} 1' '{RX_END} 1 0 0 0' '{TX_BEGIN} 1' '{TX_END} 1 0 0 0' '{INTERVAL_MARKER} 1'; exec sleep 60"
        ));
        LinuxNetworkAttributionMonitor::spawn(command)
    }

    #[cfg(unix)]
    fn capacity_warning_monitor() -> Result<LinuxNetworkAttributionMonitor, String> {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(format!(
            "printf '%s\\n' '{RX_BEGIN} 1' '{RX_END} 1 0 0 0' '{TX_BEGIN} 1' '{TX_END} 1 0 0 0' '{INTERVAL_MARKER} 1'; printf '%s\\n' 'WARNING: Map full; cannot update element' 'Additional Info - helper: map_update_elem, retcode: -7' >&2; exec sleep 60"
        ));
        LinuxNetworkAttributionMonitor::spawn(command)
    }

    #[cfg(unix)]
    fn wait_for_runtime_failure<F>(
        attribution: &mut LinuxNetworkAttribution,
        now: Instant,
        restart: &mut F,
    ) -> String
    where
        F: FnMut() -> Result<LinuxNetworkAttributionMonitor, String>,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match attribution.sample_at_with(now, restart) {
                NetworkAttributionSample::Failed(message) => return message,
                NetworkAttributionSample::Held(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                sample => panic!("runtime failure did not arrive: {sample:?}"),
            }
        }
    }

    struct AlwaysFails;

    impl Read for AlwaysFails {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture read failure"))
        }
    }
}
