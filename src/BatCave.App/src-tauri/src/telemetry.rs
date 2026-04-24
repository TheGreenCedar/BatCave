use std::{
    cmp::Ordering,
    sync::{
        atomic::{AtomicU64, Ordering as AtomicOrdering},
        Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use sysinfo::{Networks, System};

use crate::contracts::{ProcessSample, RuntimeHealth, RuntimeSnapshot, SystemMetricsSnapshot};

pub struct TelemetryState {
    sequence: AtomicU64,
    system: Mutex<System>,
    networks: Mutex<Networks>,
}

impl TelemetryState {
    pub fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            system: Mutex::new(System::new_all()),
            networks: Mutex::new(Networks::new_with_refreshed_list()),
        }
    }
}

pub fn collect_snapshot(state: &TelemetryState) -> Result<RuntimeSnapshot, String> {
    let started = Instant::now();
    let seq = state.sequence.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    let mut warnings = Vec::new();

    let mut system = state
        .system
        .lock()
        .map_err(|_| "system telemetry lock is poisoned".to_string())?;
    system.refresh_all();

    let mut networks = state
        .networks
        .lock()
        .map_err(|_| "network telemetry lock is poisoned".to_string())?;
    networks.refresh(true);

    let logical_cpu_percent = system
        .cpus()
        .iter()
        .map(|cpu| round1(cpu.cpu_usage() as f64))
        .collect::<Vec<_>>();
    let (network_received_total_bytes, network_transmitted_total_bytes) =
        networks
            .iter()
            .fold((0_u64, 0_u64), |(received, transmitted), (_, data)| {
                (
                    received.saturating_add(data.total_received()),
                    transmitted.saturating_add(data.total_transmitted()),
                )
            });

    let mut processes = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let disk_usage = process.disk_usage();
            ProcessSample {
                pid: pid.to_string(),
                parent_pid: process.parent().map(|parent| parent.to_string()),
                name: process.name().to_string_lossy().into_owned(),
                exe: process
                    .exe()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                status: format!("{:?}", process.status()),
                cpu_percent: round1(process.cpu_usage() as f64),
                memory_bytes: process.memory(),
                virtual_memory_bytes: process.virtual_memory(),
                disk_read_total_bytes: disk_usage.total_read_bytes,
                disk_write_total_bytes: disk_usage.total_written_bytes,
            }
        })
        .collect::<Vec<_>>();

    let (disk_read_total_bytes, disk_write_total_bytes) =
        processes
            .iter()
            .fold((0_u64, 0_u64), |(read_total, write_total), process| {
                (
                    read_total.saturating_add(process.disk_read_total_bytes),
                    write_total.saturating_add(process.disk_write_total_bytes),
                )
            });

    processes.sort_by(|left, right| {
        right
            .cpu_percent
            .partial_cmp(&left.cpu_percent)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
            .then_with(|| left.name.cmp(&right.name))
    });
    if logical_cpu_percent.is_empty() {
        warnings.push(
            "logical_cpu_percent was empty; frontend will fall back to aggregate CPU.".to_string(),
        );
    }

    let health = RuntimeHealth {
        tick_count: seq,
        snapshot_latency_ms: started.elapsed().as_millis() as u64,
        degraded: false,
        collector_warnings: warnings.len(),
    };

    Ok(RuntimeSnapshot {
        event_kind: "runtime_snapshot",
        seq,
        ts_ms: now_ms(),
        source: "tauri_sysinfo",
        health,
        system: SystemMetricsSnapshot {
            cpu_percent: round1(system.global_cpu_usage() as f64),
            kernel_cpu_percent: 0.0,
            logical_cpu_percent,
            memory_used_bytes: system.used_memory(),
            memory_total_bytes: system.total_memory(),
            swap_used_bytes: system.used_swap(),
            swap_total_bytes: system.total_swap(),
            process_count: system.processes().len(),
            disk_read_total_bytes,
            disk_write_total_bytes,
            network_received_total_bytes,
            network_transmitted_total_bytes,
        },
        processes,
        warnings,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
