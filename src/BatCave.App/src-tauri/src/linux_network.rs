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
const INTERVAL_MARKER: &str = "BATCAVE_NETWORK_INTERVAL";

fn bpftrace_script() -> String {
    format!(
        r#"
kprobe:sock_sendmsg {{
  $socket = (struct socket *)arg0;
  @batcave_tx_family[tid] = $socket->sk->__sk_common.skc_family;
}}
kretprobe:sock_sendmsg {{
  $family = @batcave_tx_family[tid];
  if ((int64)retval > 0 && ($family == {IPV4_SOCKET_FAMILY} || $family == {IPV6_SOCKET_FAMILY})) {{
    @batcave_tx[pid] = sum((int64)retval);
  }}
  delete(@batcave_tx_family[tid]);
}}
kprobe:sock_recvmsg {{
  $socket = (struct socket *)arg0;
  @batcave_rx_family[tid] = $socket->sk->__sk_common.skc_family;
}}
kretprobe:sock_recvmsg {{
  $family = @batcave_rx_family[tid];
  if ((int64)retval > 0 && ($family == {IPV4_SOCKET_FAMILY} || $family == {IPV6_SOCKET_FAMILY})) {{
    @batcave_rx[pid] = sum((int64)retval);
  }}
  delete(@batcave_rx_family[tid]);
}}
interval:s:1 {{
  print(@batcave_rx);
  print(@batcave_tx);
  printf("{INTERVAL_MARKER}\n");
  clear(@batcave_rx);
  clear(@batcave_tx);
}}
"#
    )
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
            Ok(monitor) => LinuxNetworkAttributionState::Ready(monitor),
            Err(message) => {
                LinuxNetworkAttributionState::Failed(FailureState::after_attempt(message, 1, now))
            }
        };
        Self { state }
    }

    pub fn sample(&mut self) -> NetworkAttributionSample {
        let runtime_failure = match &mut self.state {
            LinuxNetworkAttributionState::Ready(monitor) => match monitor.sample() {
                NetworkAttributionSample::Failed(message) => Some(message),
                sample => return sample,
            },
            LinuxNetworkAttributionState::Failed(_) => None,
        };

        let now = Instant::now();
        if let Some(message) = runtime_failure {
            self.state = LinuxNetworkAttributionState::Failed(FailureState::after_runtime_failure(
                message, now,
            ));
        }
        self.retry_failed(now)
    }

    fn retry_failed(&mut self, now: Instant) -> NetworkAttributionSample {
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
        match LinuxNetworkAttributionMonitor::start() {
            Ok(monitor) => {
                self.state = LinuxNetworkAttributionState::Ready(monitor);
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
    Ready(LinuxNetworkAttributionMonitor),
    Failed(FailureState),
}

#[derive(Debug, Clone)]
struct FailureState {
    message: String,
    attempts: u8,
    retry_at: Option<Instant>,
}

impl FailureState {
    fn after_runtime_failure(message: String, now: Instant) -> Self {
        Self {
            message,
            attempts: 0,
            retry_at: Some(now + RETRY_DELAY),
        }
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

        let mut command = Command::new("bpftrace");
        command.arg("-q").arg("-e").arg(bpftrace_script());
        Self::spawn(command)
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

#[derive(Debug, Default)]
struct PendingRates {
    received_by_pid: HashMap<u32, u64>,
    transmitted_by_pid: HashMap<u32, u64>,
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
    let mut pending = PendingRates::default();
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
                if let Err(message) = ingest_bpftrace_stdout_line(&line, &mut pending, &shared) {
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
    pending: &mut PendingRates,
    shared: &Arc<Mutex<LinuxNetworkShared>>,
) -> Result<(), String> {
    let line = line.trim();
    if line.is_empty() || matches!(line, "@batcave_rx: {}" | "@batcave_tx: {}") {
        return Ok(());
    }
    if line == INTERVAL_MARKER {
        let mut shared = shared
            .lock()
            .map_err(|_| "linux_network_ebpf_state_lock_poisoned".to_string())?;
        shared.completed.push(pending);
        shared.last_interval_at = Some(Instant::now());
        return Ok(());
    }

    let Some((direction, pid, bytes)) = parse_bpftrace_map_line(line) else {
        let excerpt = line.chars().take(160).collect::<String>();
        return Err(format!("linux_network_ebpf_stdout_malformed:{excerpt}"));
    };
    let previous = match direction {
        NetworkDirection::Received => pending.received_by_pid.insert(pid, bytes),
        NetworkDirection::Transmitted => pending.transmitted_by_pid.insert(pid, bytes),
    };
    if previous.is_some() {
        return Err(format!(
            "linux_network_ebpf_stdout_duplicate_pid:{}:{pid}",
            direction.label()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkDirection {
    Received,
    Transmitted,
}

impl NetworkDirection {
    const fn label(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Transmitted => "transmitted",
        }
    }
}

fn parse_bpftrace_map_line(line: &str) -> Option<(NetworkDirection, u32, u64)> {
    let (name, bytes) = line.split_once(':')?;
    let bytes = bytes.trim().parse::<u64>().ok()?;
    let (prefix, pid) = name.rsplit_once('[')?;
    let pid = pid.strip_suffix(']')?.parse::<u32>().ok()?;
    let direction = if prefix == "@batcave_rx" {
        NetworkDirection::Received
    } else if prefix == "@batcave_tx" {
        NetworkDirection::Transmitted
    } else {
        return None;
    };
    Some((direction, pid, bytes))
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
    }

    #[test]
    fn parse_bpftrace_map_line_reads_receive_and_transmit_entries() {
        assert_eq!(
            parse_bpftrace_map_line("@batcave_rx[1234]: 4096"),
            Some((NetworkDirection::Received, 1234, 4096))
        );
        assert_eq!(
            parse_bpftrace_map_line("@batcave_tx[42]: 8192"),
            Some((NetworkDirection::Transmitted, 42, 8192))
        );
    }

    #[test]
    fn parse_bpftrace_map_line_rejects_unrelated_and_hostile_output() {
        for line in [
            "Attaching 3 probes...",
            "@other[42]: 1",
            "@batcave_rx[-1]: 1",
            "@batcave_tx[42]: -1",
            "@batcave_rx[42]: 1:2",
            "@batcave_rx[4294967296]: 1",
        ] {
            assert_eq!(parse_bpftrace_map_line(line), None, "line={line}");
        }
    }

    #[test]
    fn completed_intervals_accumulate_both_directions() {
        let mut completed = CompletedIntervalAccumulator::default();
        let mut first = PendingRates::default();
        first.received_by_pid.insert(10, 100);
        first.transmitted_by_pid.insert(10, 200);
        first.transmitted_by_pid.insert(20, 300);
        completed.push(&mut first);

        let mut second = PendingRates::default();
        second.received_by_pid.insert(10, 300);
        second.transmitted_by_pid.insert(10, 400);
        completed.push(&mut second);

        let sample = completed.take_rates().expect("completed sample");
        assert_eq!(sample.interval_count, 2);
        assert_eq!(sample.rates_by_pid[&10].received_bps, 200);
        assert_eq!(sample.rates_by_pid[&10].transmitted_bps, 300);
        assert_eq!(sample.rates_by_pid[&20].received_bps, 0);
        assert_eq!(sample.rates_by_pid[&20].transmitted_bps, 150);
        assert!(completed.take_rates().is_none());
    }

    #[test]
    fn supported_app_cadences_consume_every_completed_interval_once() {
        for cadence_ms in [500_u64, 1_000, 2_000, 5_000] {
            let mut completed = CompletedIntervalAccumulator::default();
            let mut consumed_intervals = 0;
            let mut ready_samples = 0;
            for elapsed_ms in (500_u64..=10_000).step_by(500) {
                if elapsed_ms % 1_000 == 0 {
                    let mut interval = PendingRates::default();
                    interval.received_by_pid.insert(7, 1_000);
                    interval.transmitted_by_pid.insert(7, 2_000);
                    completed.push(&mut interval);
                }
                if elapsed_ms % cadence_ms == 0 {
                    if let Some(sample) = completed.take_rates() {
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
            assert!(completed.take_rates().is_none());
        }
    }

    #[test]
    fn malformed_and_duplicate_stdout_fail_the_reader_protocol() {
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let mut pending = PendingRates::default();
        ingest_bpftrace_stdout_line("@batcave_rx[4]: 10", &mut pending, &shared).unwrap();
        let duplicate = ingest_bpftrace_stdout_line("@batcave_rx[4]: 20", &mut pending, &shared)
            .expect_err("duplicate PID fails");
        assert!(duplicate.contains("duplicate_pid:received:4"));

        let malformed = ingest_bpftrace_stdout_line("not protocol", &mut pending, &shared)
            .expect_err("unknown stdout fails");
        assert!(malformed.contains("stdout_malformed"));
    }

    #[test]
    fn empty_maps_and_interval_marker_publish_a_truthful_zero_interval() {
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::new(Instant::now())));
        let mut pending = PendingRates::default();
        ingest_bpftrace_stdout_line("@batcave_rx: {}", &mut pending, &shared).unwrap();
        ingest_bpftrace_stdout_line("@batcave_tx: {}", &mut pending, &shared).unwrap();
        ingest_bpftrace_stdout_line(INTERVAL_MARKER, &mut pending, &shared).unwrap();

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

    struct AlwaysFails;

    impl Read for AlwaysFails {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture read failure"))
        }
    }
}
