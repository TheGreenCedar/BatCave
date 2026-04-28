#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};

use crate::network_attribution::{NetworkAttributionSample, ProcessNetworkRates};

const BPFTRACE_SCRIPT: &str = r#"
kretprobe:sock_sendmsg /retval > 0/ { @batcave_tx[pid] = sum(retval); }
kretprobe:sock_recvmsg /retval > 0/ { @batcave_rx[pid] = sum(retval); }
interval:s:1 {
  print(@batcave_rx);
  print(@batcave_tx);
  printf("BATCAVE_NETWORK_INTERVAL\n");
  clear(@batcave_rx);
  clear(@batcave_tx);
}
"#;

#[derive(Debug)]
pub struct LinuxNetworkAttributionMonitor {
    child: Child,
    shared: Arc<Mutex<LinuxNetworkShared>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    last_seq: u64,
}

impl LinuxNetworkAttributionMonitor {
    pub fn start() -> Result<Self, String> {
        ensure_bpftrace_available()?;

        let mut child = Command::new("bpftrace")
            .arg("-q")
            .arg("-e")
            .arg(BPFTRACE_SCRIPT)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("linux_network_ebpf_start_failed:{error}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "linux_network_ebpf_stdout_unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "linux_network_ebpf_stderr_unavailable".to_string())?;
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::default()));
        let stdout_shared = Arc::clone(&shared);
        let stderr_shared = Arc::clone(&shared);
        let stdout_thread = thread::Builder::new()
            .name("batcave-linux-network-ebpf-stdout".to_string())
            .spawn(move || read_bpftrace_stdout(stdout, stdout_shared))
            .map_err(|error| format!("linux_network_ebpf_stdout_thread_failed:{error}"))?;
        let stderr_thread = thread::Builder::new()
            .name("batcave-linux-network-ebpf-stderr".to_string())
            .spawn(move || read_bpftrace_stderr(stderr, stderr_shared))
            .map_err(|error| format!("linux_network_ebpf_stderr_thread_failed:{error}"))?;

        Ok(Self {
            child,
            shared,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            last_seq: 0,
        })
    }

    pub fn sample(&mut self) -> NetworkAttributionSample {
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

        let shared = match self.shared.lock() {
            Ok(shared) => shared,
            Err(_) => {
                return NetworkAttributionSample::Failed(
                    "linux_network_ebpf_state_lock_poisoned".to_string(),
                );
            }
        };
        if shared.seq == 0 {
            return NetworkAttributionSample::Held(
                "Linux eBPF network attribution is warming up.".to_string(),
            );
        }
        if shared.seq == self.last_seq {
            return NetworkAttributionSample::Held(
                "Linux eBPF network attribution is waiting for the next sample.".to_string(),
            );
        }

        self.last_seq = shared.seq;
        NetworkAttributionSample::Ready {
            rates_by_pid: shared.rates_by_pid.clone(),
        }
    }
}

impl Drop for LinuxNetworkAttributionMonitor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug, Default)]
struct LinuxNetworkShared {
    seq: u64,
    rates_by_pid: HashMap<u32, ProcessNetworkRates>,
    last_error: Option<String>,
}

#[derive(Debug, Default)]
struct PendingRates {
    received_by_pid: HashMap<u32, u64>,
    transmitted_by_pid: HashMap<u32, u64>,
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

fn read_bpftrace_stdout(stdout: impl std::io::Read, shared: Arc<Mutex<LinuxNetworkShared>>) {
    let mut pending = PendingRates::default();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let line = line.trim();
        if line == "BATCAVE_NETWORK_INTERVAL" {
            publish_pending_rates(&mut pending, &shared);
            continue;
        }
        if let Some((direction, pid, bytes)) = parse_bpftrace_map_line(line) {
            match direction {
                NetworkDirection::Received => {
                    pending.received_by_pid.insert(pid, bytes);
                }
                NetworkDirection::Transmitted => {
                    pending.transmitted_by_pid.insert(pid, bytes);
                }
            }
        }
    }
}

fn read_bpftrace_stderr(stderr: impl std::io::Read, shared: Arc<Mutex<LinuxNetworkShared>>) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        if let Ok(mut shared) = shared.lock() {
            shared.last_error = Some(message.to_string());
        }
    }
}

fn publish_pending_rates(pending: &mut PendingRates, shared: &Arc<Mutex<LinuxNetworkShared>>) {
    let mut rates_by_pid = HashMap::new();
    for (&pid, &received_bps) in &pending.received_by_pid {
        rates_by_pid
            .entry(pid)
            .or_insert_with(ProcessNetworkRates::default)
            .received_bps = received_bps;
    }
    for (&pid, &transmitted_bps) in &pending.transmitted_by_pid {
        rates_by_pid
            .entry(pid)
            .or_insert_with(ProcessNetworkRates::default)
            .transmitted_bps = transmitted_bps;
    }

    pending.received_by_pid.clear();
    pending.transmitted_by_pid.clear();

    if let Ok(mut shared) = shared.lock() {
        shared.seq = shared.seq.saturating_add(1);
        shared.rates_by_pid = rates_by_pid;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkDirection {
    Received,
    Transmitted,
}

fn parse_bpftrace_map_line(line: &str) -> Option<(NetworkDirection, u32, u64)> {
    let (name, bytes) = line.split_once(':')?;
    let bytes = bytes.trim().parse::<u64>().ok()?;
    let (prefix, pid) = name.rsplit_once('[')?;
    let pid = pid.strip_suffix(']')?.parse::<u32>().ok()?;
    let direction = if prefix.ends_with("@batcave_rx") {
        NetworkDirection::Received
    } else if prefix.ends_with("@batcave_tx") {
        NetworkDirection::Transmitted
    } else {
        return None;
    };
    Some((direction, pid, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_bpftrace_map_line_ignores_unrelated_output() {
        assert_eq!(parse_bpftrace_map_line("Attaching 3 probes..."), None);
        assert_eq!(parse_bpftrace_map_line("@other[42]: 1"), None);
    }

    #[test]
    fn publish_pending_rates_combines_directions() {
        let shared = Arc::new(Mutex::new(LinuxNetworkShared::default()));
        let mut pending = PendingRates::default();
        pending.received_by_pid.insert(10, 100);
        pending.transmitted_by_pid.insert(10, 200);
        pending.transmitted_by_pid.insert(20, 300);

        publish_pending_rates(&mut pending, &shared);

        let shared = shared.lock().unwrap();
        assert_eq!(shared.seq, 1);
        assert_eq!(shared.rates_by_pid[&10].received_bps, 100);
        assert_eq!(shared.rates_by_pid[&10].transmitted_bps, 200);
        assert_eq!(shared.rates_by_pid[&20].received_bps, 0);
        assert_eq!(shared.rates_by_pid[&20].transmitted_bps, 300);
    }
}
