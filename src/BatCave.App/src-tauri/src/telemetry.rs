use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System,
    UpdateKind,
};

use crate::contracts::{
    AccessState, MetricQuality, MetricQualityInfo, MetricSource, ProcessMetricQuality,
    ProcessSample, SystemMetricQuality, SystemMetricsSnapshot,
};
#[cfg(any(windows, target_os = "linux", test))]
use crate::network_attribution::NetworkAttributionSample;

#[cfg(windows)]
use crate::windows_process;
#[cfg(target_os = "linux")]
use crate::{
    linux_network::LinuxNetworkAttributionMonitor, linux_process::LinuxProcessCollector,
    linux_system::LinuxSystemCollector,
};
#[cfg(target_os = "macos")]
use crate::{macos_process::MacosProcessCollector, macos_system::MacosSystemCollector};
#[cfg(windows)]
use crate::{
    windows_network::NetworkAttributionMonitor,
    windows_pdh::{PdhDiskSampler, PdhSample},
    windows_system,
};

#[derive(Debug, Clone)]
pub struct TelemetrySample {
    pub latency_ms: u64,
    pub system: SystemMetricsSnapshot,
    pub processes: Vec<ProcessSample>,
    pub warnings: Vec<String>,
}

pub struct TelemetryCollector {
    system: Mutex<System>,
    networks: Mutex<Networks>,
    #[cfg(target_os = "linux")]
    linux_system: Mutex<LinuxSystemCollector>,
    #[cfg(target_os = "linux")]
    linux_processes: Mutex<LinuxProcessCollector>,
    #[cfg(target_os = "linux")]
    linux_network_attribution: Mutex<LinuxNetworkAttributionState>,
    #[cfg(target_os = "macos")]
    macos_system: Mutex<MacosSystemCollector>,
    #[cfg(target_os = "macos")]
    macos_processes: Mutex<MacosProcessCollector>,
    #[cfg(windows)]
    previous_cpu_times: Mutex<Option<windows_system::CpuTimes>>,
    #[cfg(windows)]
    pdh_disk: Mutex<PdhDiskState>,
    #[cfg(windows)]
    network_attribution: Mutex<NetworkAttributionState>,
}

impl TelemetryCollector {
    pub fn new() -> Self {
        Self::new_with_process_network(true)
    }

    #[cfg(all(windows, test))]
    pub(crate) fn for_elevated_helper(process_network: bool) -> Self {
        Self::new_with_process_network(process_network)
    }

    #[cfg(test)]
    pub(crate) fn process_network_ready(&self) -> Result<bool, String> {
        #[cfg(windows)]
        {
            let state = self
                .network_attribution
                .lock()
                .map_err(|_| "network attribution telemetry lock is poisoned".to_string())?;
            Ok(matches!(&*state, NetworkAttributionState::Ready(_)))
        }
        #[cfg(not(windows))]
        {
            Ok(false)
        }
    }

    #[cfg(test)]
    pub(crate) fn retry_process_network(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            let mut state = self
                .network_attribution
                .lock()
                .map_err(|_| "network attribution telemetry lock is poisoned".to_string())?;
            *state = NetworkAttributionState::Disabled;
            *state = NetworkAttributionState::new();
        }
        Ok(())
    }

    fn new_with_process_network(process_network: bool) -> Self {
        #[cfg(not(windows))]
        let _ = process_network;
        Self {
            system: Mutex::new(System::new_with_specifics(sysinfo_refresh_kind())),
            networks: Mutex::new(Networks::new_with_refreshed_list()),
            #[cfg(target_os = "linux")]
            linux_system: Mutex::new(LinuxSystemCollector::new()),
            #[cfg(target_os = "linux")]
            linux_processes: Mutex::new(LinuxProcessCollector::new()),
            #[cfg(target_os = "linux")]
            linux_network_attribution: Mutex::new(LinuxNetworkAttributionState::new()),
            #[cfg(target_os = "macos")]
            macos_system: Mutex::new(MacosSystemCollector::new()),
            #[cfg(target_os = "macos")]
            macos_processes: Mutex::new(MacosProcessCollector::new()),
            #[cfg(windows)]
            previous_cpu_times: Mutex::new(None),
            #[cfg(windows)]
            pdh_disk: Mutex::new(PdhDiskState::new()),
            #[cfg(windows)]
            network_attribution: Mutex::new(if process_network {
                NetworkAttributionState::new()
            } else {
                NetworkAttributionState::Disabled
            }),
        }
    }

    pub fn collect(&self) -> Result<TelemetrySample, String> {
        let started = Instant::now();
        let mut warnings = Vec::new();

        let mut sysinfo_system = self
            .system
            .lock()
            .map_err(|_| "system telemetry lock is poisoned".to_string())?;
        sysinfo_system.refresh_specifics(sysinfo_refresh_kind());

        let mut sysinfo_networks = self
            .networks
            .lock()
            .map_err(|_| "network telemetry lock is poisoned".to_string())?;
        sysinfo_networks.refresh(true);

        let sysinfo_processes = collect_sysinfo_processes(&sysinfo_system);
        let sysinfo_cpu_by_pid = sysinfo_processes
            .iter()
            .map(|process| (process.pid.clone(), process.cpu_percent))
            .collect::<HashMap<_, _>>();
        let logical_cpu_percent = logical_cpu_percent(&sysinfo_system, &mut warnings);
        let mut processes =
            collect_processes(&sysinfo_processes, &sysinfo_cpu_by_pid, &mut warnings, self)?;
        let mut system_snapshot = collect_system_snapshot(
            &sysinfo_system,
            &sysinfo_networks,
            &logical_cpu_percent,
            &processes,
            &mut warnings,
            self,
        )?;
        #[cfg(windows)]
        {
            let network_attribution = self.network_attribution_sample(&mut warnings)?;
            apply_network_attribution(&mut processes, network_attribution, MetricSource::Etw);
        }
        #[cfg(target_os = "linux")]
        {
            let network_attribution = self.linux_network_attribution_sample(&mut warnings)?;
            apply_network_attribution(&mut processes, network_attribution, MetricSource::Ebpf);
        }

        processes.sort_by(|left, right| {
            right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
                .then_with(|| left.name.cmp(&right.name))
        });

        system_snapshot.process_count = processes.len();

        Ok(TelemetrySample {
            latency_ms: started.elapsed().as_millis() as u64,
            system: system_snapshot,
            processes,
            warnings,
        })
    }
}

fn sysinfo_refresh_kind() -> RefreshKind {
    RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
        .with_memory(MemoryRefreshKind::everything())
        .with_processes(
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        )
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn collect_processes(
    sysinfo_processes: &[ProcessSample],
    sysinfo_cpu_by_pid: &HashMap<String, f64>,
    warnings: &mut Vec<String>,
    collector: &TelemetryCollector,
) -> Result<Vec<ProcessSample>, String> {
    #[cfg(windows)]
    let _ = collector;

    #[cfg(windows)]
    {
        match windows_process::collect_processes(0) {
            Ok(native_processes) => {
                let sysinfo_by_pid = sysinfo_processes
                    .iter()
                    .map(|process| (process.pid.clone(), process))
                    .collect::<HashMap<_, _>>();
                Ok(native_processes
                    .into_iter()
                    .map(|process| {
                        enrich_native_process(process, sysinfo_cpu_by_pid, &sysinfo_by_pid)
                    })
                    .collect())
            }
            Err(error) => {
                warnings.push(format!(
                    "native_process_collector_failed:{error}; using sysinfo fallback"
                ));
                Ok(sysinfo_processes.to_vec())
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = sysinfo_cpu_by_pid;
        let mut linux_processes = collector
            .linux_processes
            .lock()
            .map_err(|_| "linux process telemetry lock is poisoned".to_string())?;
        match linux_processes.collect() {
            Ok(processes) => Ok(processes),
            Err(error) => {
                warnings.push(format!(
                    "linux_process_collector_failed:{error}; using sysinfo fallback"
                ));
                Ok(sysinfo_processes.to_vec())
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = sysinfo_cpu_by_pid;
        let mut macos_processes = collector
            .macos_processes
            .lock()
            .map_err(|_| "macOS process telemetry lock is poisoned".to_string())?;
        let mut processes = sysinfo_processes.to_vec();
        let process_count = processes.len();
        let collection = macos_processes.enrich(&mut processes);
        if process_count > 0
            && collection
                .denied_count
                .saturating_add(collection.partial_count)
                == process_count
        {
            warnings.push(format!(
                "macos_process_collector_no_full_access:denied={} partial={}",
                collection.denied_count, collection.partial_count
            ));
        }
        Ok(processes)
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        let _ = (sysinfo_cpu_by_pid, warnings, collector);
        Ok(sysinfo_processes.to_vec())
    }
}

fn collect_system_snapshot(
    sysinfo_system: &System,
    sysinfo_networks: &Networks,
    logical_cpu_percent: &[f64],
    processes: &[ProcessSample],
    warnings: &mut Vec<String>,
    collector: &TelemetryCollector,
) -> Result<SystemMetricsSnapshot, String> {
    let sysinfo_snapshot = collect_sysinfo_system(sysinfo_system, sysinfo_networks);

    #[cfg(windows)]
    {
        let _ = processes;
        let cpu_load = collector.native_cpu_load(warnings)?;

        match windows_system::sample_system() {
            Ok(mut snapshot) => {
                snapshot.logical_cpu_percent = logical_cpu_percent.to_vec();
                if let Some(load) = cpu_load {
                    snapshot.cpu_percent = load.cpu_percent;
                    snapshot.kernel_cpu_percent = load.kernel_cpu_percent;
                } else {
                    snapshot.cpu_percent = sysinfo_snapshot.cpu_percent;
                    snapshot.kernel_cpu_percent = 0.0;
                }
                let disk_quality = collector.apply_pdh_disk_rates(&mut snapshot, warnings)?;
                snapshot.quality = Some(system_quality(cpu_load.is_some(), disk_quality));
                Ok(snapshot)
            }
            Err(error) => {
                warnings.push(format!(
                    "native_system_collector_failed:{error}; using sysinfo fallback"
                ));
                Ok(sysinfo_snapshot)
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = (
            sysinfo_system,
            sysinfo_networks,
            logical_cpu_percent,
            processes,
        );
        let mut linux_system = collector
            .linux_system
            .lock()
            .map_err(|_| "linux system telemetry lock is poisoned".to_string())?;
        match linux_system.sample() {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                warnings.push(format!(
                    "linux_system_collector_failed:{error}; using sysinfo fallback"
                ));
                Ok(sysinfo_snapshot)
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = (
            sysinfo_system,
            sysinfo_networks,
            logical_cpu_percent,
            warnings,
        );
        let mut snapshot = sysinfo_snapshot;
        let mut macos_system = collector
            .macos_system
            .lock()
            .map_err(|_| "macOS system telemetry lock is poisoned".to_string())?;
        macos_system.enrich(&mut snapshot, processes);
        Ok(snapshot)
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        let _ = (logical_cpu_percent, processes, warnings, collector);
        Ok(sysinfo_snapshot)
    }
}

impl TelemetryCollector {
    #[cfg(windows)]
    fn apply_pdh_disk_rates(
        &self,
        snapshot: &mut SystemMetricsSnapshot,
        warnings: &mut Vec<String>,
    ) -> Result<DiskQualityState, String> {
        let mut state = self
            .pdh_disk
            .lock()
            .map_err(|_| "pdh disk telemetry lock is poisoned".to_string())?;
        if matches!(&*state, PdhDiskState::Failed(_, failed_at) if failed_at.elapsed() >= std::time::Duration::from_secs(30))
        {
            *state = PdhDiskState::new();
        }
        match &mut *state {
            PdhDiskState::Ready(sampler) => match sampler.sample() {
                Ok(PdhSample::Ready(rates)) => {
                    snapshot.disk_read_bps = rates.read_bps;
                    snapshot.disk_write_bps = rates.write_bps;
                    Ok(DiskQualityState::Native)
                }
                Ok(PdhSample::Held(message)) => Ok(DiskQualityState::Held(message)),
                Err(error) => {
                    warnings.push(format!("pdh_disk_collector_failed:{error}"));
                    *state = PdhDiskState::Failed(error.clone(), Instant::now());
                    Ok(DiskQualityState::Unavailable(error))
                }
            },
            PdhDiskState::Failed(error, _) => {
                warnings.push(format!("pdh_disk_collector_failed:{error}"));
                Ok(DiskQualityState::Unavailable(error.clone()))
            }
        }
    }

    #[cfg(windows)]
    fn network_attribution_sample(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<NetworkAttributionSample, String> {
        let mut state = self
            .network_attribution
            .lock()
            .map_err(|_| "network attribution telemetry lock is poisoned".to_string())?;
        match &mut *state {
            NetworkAttributionState::Disabled => Ok(NetworkAttributionSample::Held(
                "Network attribution is owned by the main runtime.".to_string(),
            )),
            NetworkAttributionState::Ready(monitor) => {
                let sample = monitor.sample();
                if let NetworkAttributionSample::Failed(message) = &sample {
                    warnings.push(format!("network_attribution_failed:{message}"));
                    *state = NetworkAttributionState::Failed {
                        message: message.clone(),
                        failed_at: Instant::now(),
                    };
                }
                Ok(sample)
            }
            NetworkAttributionState::Failed { message, failed_at } => {
                if failed_at.elapsed() >= std::time::Duration::from_secs(30) {
                    *state = NetworkAttributionState::new();
                    return Ok(NetworkAttributionSample::Held(
                        "ETW network attribution is retrying.".to_string(),
                    ));
                }
                warnings.push(format!("network_attribution_failed:{message}"));
                Ok(NetworkAttributionSample::Failed(message.clone()))
            }
        }
    }

    #[cfg(windows)]
    fn native_cpu_load(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<Option<windows_system::CpuLoad>, String> {
        let current = match windows_system::sample_cpu_times() {
            Ok(current) => current,
            Err(error) => {
                warnings.push(format!(
                    "native_cpu_collector_failed:{error}; using sysinfo fallback"
                ));
                return Ok(None);
            }
        };
        let mut previous = self
            .previous_cpu_times
            .lock()
            .map_err(|_| "cpu telemetry lock is poisoned".to_string())?;
        let load = previous.map(|previous| windows_system::calculate_cpu_load(previous, current));
        *previous = Some(current);
        Ok(load)
    }

    #[cfg(target_os = "linux")]
    fn linux_network_attribution_sample(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<NetworkAttributionSample, String> {
        let mut state = self
            .linux_network_attribution
            .lock()
            .map_err(|_| "linux network attribution telemetry lock is poisoned".to_string())?;
        match &mut *state {
            LinuxNetworkAttributionState::Ready(monitor) => {
                let sample = monitor.sample();
                if let NetworkAttributionSample::Failed(message) = &sample {
                    warnings.push(format!("linux_network_attribution_failed:{message}"));
                    *state = LinuxNetworkAttributionState::Failed {
                        message: message.clone(),
                        failed_at: Instant::now(),
                    };
                }
                Ok(sample)
            }
            LinuxNetworkAttributionState::Failed { message, failed_at } => {
                if failed_at.elapsed() >= std::time::Duration::from_secs(30) {
                    *state = LinuxNetworkAttributionState::new();
                    return Ok(NetworkAttributionSample::Held(
                        "Linux network attribution is retrying.".to_string(),
                    ));
                }
                warnings.push(format!("linux_network_attribution_failed:{message}"));
                Ok(NetworkAttributionSample::Failed(message.clone()))
            }
        }
    }
}

fn collect_sysinfo_system(system: &System, networks: &Networks) -> SystemMetricsSnapshot {
    let (network_received_total_bytes, network_transmitted_total_bytes) =
        networks
            .iter()
            .fold((0_u64, 0_u64), |(received, transmitted), (_, data)| {
                (
                    received.saturating_add(data.total_received()),
                    transmitted.saturating_add(data.total_transmitted()),
                )
            });

    SystemMetricsSnapshot {
        cpu_percent: round1(system.global_cpu_usage() as f64),
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: system
            .cpus()
            .iter()
            .map(|cpu| round1(cpu.cpu_usage() as f64))
            .collect(),
        memory_used_bytes: system.used_memory(),
        memory_total_bytes: system.total_memory(),
        memory_available_bytes: Some(system.available_memory()),
        swap_used_bytes: (!cfg!(windows)).then_some(system.used_swap()),
        swap_total_bytes: (!cfg!(windows)).then_some(system.total_swap()),
        process_count: system.processes().len(),
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes,
        network_transmitted_total_bytes,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        memory_accounting: None,
        quality: Some(SystemMetricQuality {
            cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            kernel_cpu: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_message("Kernel CPU is unavailable from the sysinfo fallback."),
            ),
            logical_cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            swap: Some(if cfg!(windows) {
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_message("Windows reports commit accounting, not swap usage.")
            } else {
                MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
            }),
            disk: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_message(
                        "Physical-disk throughput is unavailable because the sysinfo fallback has no device-level rate source.",
                    ),
            ),
            network: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
        }),
    }
}

fn collect_sysinfo_processes(system: &System) -> Vec<ProcessSample> {
    system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let disk_usage = process.disk_usage();
            let virtual_memory_bytes = process.virtual_memory();
            ProcessSample {
                pid: pid.to_string(),
                parent_pid: process.parent().map(|parent| parent.to_string()),
                start_time_ms: process.start_time().saturating_mul(1000),
                name: process.name().to_string_lossy().into_owned(),
                exe: process
                    .exe()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                status: format!("{:?}", process.status()),
                cpu_percent: round1(process.cpu_usage() as f64),
                kernel_cpu_percent: None,
                memory_bytes: process.memory(),
                private_bytes: process.memory(),
                virtual_memory_bytes: (!cfg!(windows) && virtual_memory_bytes > 0)
                    .then_some(virtual_memory_bytes),
                io_read_total_bytes: disk_usage.total_read_bytes,
                io_write_total_bytes: disk_usage.total_written_bytes,
                other_io_total_bytes: None,
                io_read_bps: 0,
                io_write_bps: 0,
                other_io_bps: None,
                network_received_bps: None,
                network_transmitted_bps: None,
                threads: 0,
                handles: 0,
                access_state: AccessState::Partial,
                quality: Some(ProcessMetricQuality {
                    cpu: Some(MetricQualityInfo::new(
                        MetricQuality::Estimated,
                        MetricSource::Sysinfo,
                    )),
                    memory: Some(MetricQualityInfo::new(
                        MetricQuality::Estimated,
                        MetricSource::Sysinfo,
                    )),
                    io: Some(MetricQualityInfo::new(
                        MetricQuality::Estimated,
                        MetricSource::Sysinfo,
                    )),
                    other_io: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_message("Other I/O is unavailable from the sysinfo fallback."),
                    ),
                    network: Some(process_network_quality_unavailable()),
                    threads: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_message("Thread counts require the native process collector."),
                    ),
                    handles: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_message("Handle counts require the native process collector."),
                    ),
                }),
            }
        })
        .collect()
}

#[cfg(windows)]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Etw)
        .with_message("Waiting for ETW network attribution.")
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
        .with_message("Per-process network attribution is unavailable from the sysinfo fallback.")
}

#[cfg(target_os = "macos")]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
        .with_message("Per-process network attribution is unavailable on macOS.")
}

fn logical_cpu_percent(system: &System, warnings: &mut Vec<String>) -> Vec<f64> {
    let values = system
        .cpus()
        .iter()
        .map(|cpu| round1(cpu.cpu_usage() as f64))
        .collect::<Vec<_>>();
    if values.is_empty() {
        warnings.push(
            "logical_cpu_percent was empty; frontend will fall back to aggregate CPU.".into(),
        );
    }
    values
}

#[cfg(windows)]
fn enrich_native_process(
    mut process: ProcessSample,
    sysinfo_cpu_by_pid: &HashMap<String, f64>,
    sysinfo_by_pid: &HashMap<String, &ProcessSample>,
) -> ProcessSample {
    let has_cpu = sysinfo_cpu_by_pid
        .get(&process.pid)
        .map(|cpu| {
            process.cpu_percent = *cpu;
            true
        })
        .unwrap_or(false);
    let memory_from_sysinfo = sysinfo_by_pid
        .get(&process.pid)
        .map(|fallback| {
            let mut enriched = false;
            if process.memory_bytes == 0 && process.private_bytes == 0 {
                process.memory_bytes = fallback.memory_bytes;
                process.private_bytes = fallback.private_bytes;
                enriched = fallback.memory_bytes > 0 || fallback.private_bytes > 0;
            }
            if !cfg!(windows) && process.virtual_memory_bytes.is_none() {
                process.virtual_memory_bytes = fallback.virtual_memory_bytes;
                enriched |= fallback.virtual_memory_bytes.is_some();
            }
            enriched
        })
        .unwrap_or(false);
    process.quality = Some(native_process_quality(process.access_state, has_cpu));
    if memory_from_sysinfo {
        process_quality(&mut process).memory = Some(
            MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo).with_message(
                "Native process memory counters were denied; using sysinfo fallback memory.",
            ),
        );
    }
    process
}

#[cfg(windows)]
fn system_quality(has_native_cpu: bool, disk_quality: DiskQualityState) -> SystemMetricQuality {
    let cpu = if has_native_cpu {
        MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi)
    } else {
        MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
            .with_message("First sample uses sysinfo until native CPU deltas are available.")
    };

    SystemMetricQuality {
        cpu: Some(cpu.clone()),
        kernel_cpu: Some(cpu),
        logical_cpu: Some(MetricQualityInfo::new(
            MetricQuality::Estimated,
            MetricSource::Sysinfo,
        )),
        memory: Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        )),
        swap: Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
                .with_message("Windows reports commit accounting, not swap usage."),
        ),
        disk: Some(match disk_quality {
            DiskQualityState::Native => {
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::Pdh)
            }
            DiskQualityState::Held(message) => {
                MetricQualityInfo::new(MetricQuality::Held, MetricSource::Pdh)
                    .with_message(&message)
            }
            DiskQualityState::Unavailable(message) => {
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Pdh).with_message(
                    &format!("PDH physical disk telemetry is unavailable. {message}"),
                )
            }
        }),
        network: Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::InterfaceAggregate,
        )),
    }
}

#[cfg(windows)]
enum PdhDiskState {
    Ready(PdhDiskSampler),
    Failed(String, Instant),
}

#[cfg(windows)]
impl PdhDiskState {
    fn new() -> Self {
        match PdhDiskSampler::new() {
            Ok(sampler) => Self::Ready(sampler),
            Err(error) => Self::Failed(error, Instant::now()),
        }
    }
}

#[cfg(windows)]
enum DiskQualityState {
    Native,
    Held(String),
    Unavailable(String),
}

#[cfg(windows)]
enum NetworkAttributionState {
    Disabled,
    Ready(NetworkAttributionMonitor),
    Failed { message: String, failed_at: Instant },
}

#[cfg(windows)]
impl NetworkAttributionState {
    fn new() -> Self {
        match NetworkAttributionMonitor::new() {
            Ok(monitor) => Self::Ready(monitor),
            Err(message) => Self::Failed {
                message,
                failed_at: Instant::now(),
            },
        }
    }
}

#[cfg(target_os = "linux")]
enum LinuxNetworkAttributionState {
    Ready(LinuxNetworkAttributionMonitor),
    Failed { message: String, failed_at: Instant },
}

#[cfg(target_os = "linux")]
impl LinuxNetworkAttributionState {
    fn new() -> Self {
        match LinuxNetworkAttributionMonitor::start() {
            Ok(monitor) => Self::Ready(monitor),
            Err(message) => Self::Failed {
                message,
                failed_at: Instant::now(),
            },
        }
    }
}

#[cfg(any(windows, target_os = "linux", test))]
fn apply_network_attribution(
    processes: &mut [ProcessSample],
    attribution: NetworkAttributionSample,
    source: MetricSource,
) {
    match attribution {
        NetworkAttributionSample::Ready { rates_by_pid } => {
            for process in processes {
                let rates = process
                    .pid
                    .parse::<u32>()
                    .ok()
                    .and_then(|pid| rates_by_pid.get(&pid).copied())
                    .unwrap_or_default();
                process.network_received_bps = Some(rates.received_bps);
                process.network_transmitted_bps = Some(rates.transmitted_bps);
                process_quality(process).network =
                    Some(MetricQualityInfo::new(MetricQuality::Native, source));
            }
        }
        NetworkAttributionSample::Held(message) => {
            for process in processes {
                process_quality(process).network = Some(
                    MetricQualityInfo::new(MetricQuality::Held, source).with_message(&message),
                );
            }
        }
        NetworkAttributionSample::Failed(message) => {
            for process in processes {
                process.network_received_bps = None;
                process.network_transmitted_bps = None;
                process_quality(process).network = Some(
                    MetricQualityInfo::new(MetricQuality::Unavailable, source)
                        .with_message(&message),
                );
            }
        }
    }
}

#[cfg(any(windows, target_os = "linux", test))]
fn process_quality(process: &mut ProcessSample) -> &mut ProcessMetricQuality {
    process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default)
}

#[cfg(any(windows, test))]
fn native_process_quality(access_state: AccessState, has_cpu: bool) -> ProcessMetricQuality {
    let direct_quality = match access_state {
        AccessState::Full => MetricQuality::Native,
        AccessState::Partial => MetricQuality::Partial,
        AccessState::Denied => MetricQuality::Unavailable,
    };
    let direct = |message: Option<&str>| {
        let value = MetricQualityInfo::new(direct_quality, MetricSource::DirectApi);
        match message {
            Some(message) => value.with_message(message),
            None => value,
        }
    };

    ProcessMetricQuality {
        cpu: Some(if has_cpu {
            MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
        } else {
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                .with_message("Process CPU needs a second Rust-native timing pass.")
        }),
        memory: Some(direct(None)),
        io: Some(direct(Some(
            "Read/write I/O reports ReadTransferCount plus WriteTransferCount. OtherTransferCount remains a separate process field and this metric is not physical-disk attribution.",
        ))),
        other_io: Some(direct(None)),
        network: Some(process_network_quality_unavailable()),
        threads: Some(direct(None)),
        handles: Some(direct(None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_attribution::{NetworkAttributionSample, ProcessNetworkRates};
    #[cfg(windows)]
    use std::collections::HashMap;

    #[test]
    fn sysinfo_fallback_marks_physical_disk_unavailable_for_native_collector_failures() {
        let system = System::new();
        let networks = Networks::new();

        // Windows and Linux both return this shared snapshot when their native system
        // collector fails, so a zero payload must never be presented as measured disk I/O.
        let snapshot = collect_sysinfo_system(&system, &networks);
        let disk = snapshot
            .quality
            .as_ref()
            .and_then(|quality| quality.disk.as_ref())
            .expect("fallback disk quality exists");

        assert_eq!(snapshot.disk_read_total_bytes, 0);
        assert_eq!(snapshot.disk_write_total_bytes, 0);
        assert_eq!(snapshot.disk_read_bps, 0);
        assert_eq!(snapshot.disk_write_bps, 0);
        assert_eq!(disk.quality, MetricQuality::Unavailable);
        assert_eq!(disk.source, Some(MetricSource::Sysinfo));
        assert!(disk
            .message
            .as_deref()
            .is_some_and(|message| message.contains("no device-level rate source")));
    }

    #[cfg(windows)]
    #[test]
    fn enrich_native_process_uses_sysinfo_memory_when_native_memory_is_denied() {
        let mut native = sample_process("42");
        native.access_state = AccessState::Denied;
        native.memory_bytes = 0;
        native.private_bytes = 0;
        native.virtual_memory_bytes = None;
        let mut fallback = sample_process("42");
        fallback.memory_bytes = 123;
        fallback.private_bytes = 45;
        fallback.virtual_memory_bytes = Some(678);
        let sysinfo_cpu_by_pid = HashMap::from([("42".to_string(), 9.0)]);
        let sysinfo_by_pid = HashMap::from([("42".to_string(), &fallback)]);

        let enriched = enrich_native_process(native, &sysinfo_cpu_by_pid, &sysinfo_by_pid);

        assert_eq!(enriched.access_state, AccessState::Denied);
        assert_eq!(enriched.cpu_percent, 9.0);
        assert_eq!(enriched.memory_bytes, 123);
        assert_eq!(enriched.private_bytes, 45);
        assert_eq!(enriched.virtual_memory_bytes, None);
        let memory_quality = enriched
            .quality
            .and_then(|quality| quality.memory)
            .expect("memory quality exists");
        assert_eq!(memory_quality.quality, MetricQuality::Estimated);
        assert_eq!(memory_quality.source, Some(MetricSource::Sysinfo));
        assert!(memory_quality
            .message
            .expect("message exists")
            .contains("Native process memory counters were denied"));
    }

    #[cfg(windows)]
    #[test]
    fn elevated_helper_does_not_start_a_second_etw_monitor() {
        let collector = TelemetryCollector::for_elevated_helper(false);
        let state = collector.network_attribution.lock().unwrap();

        assert!(matches!(&*state, NetworkAttributionState::Disabled));
    }

    #[test]
    fn apply_network_attribution_marks_rows_native_when_monitor_is_ready() {
        let mut processes = vec![ProcessSample {
            pid: "42".to_string(),
            parent_pid: None,
            start_time_ms: 1,
            name: "Networked".to_string(),
            exe: String::new(),
            status: "running".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 0,
            private_bytes: 0,
            virtual_memory_bytes: None,
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
            quality: Some(native_process_quality(AccessState::Full, true)),
        }];
        let sample = NetworkAttributionSample::ready([(
            42,
            ProcessNetworkRates {
                received_bps: 4096,
                transmitted_bps: 2048,
            },
        )]);

        apply_network_attribution(&mut processes, sample, MetricSource::Etw);

        assert_eq!(processes[0].network_received_bps, Some(4096));
        assert_eq!(processes[0].network_transmitted_bps, Some(2048));
        let network = processes[0]
            .quality
            .as_ref()
            .unwrap()
            .network
            .as_ref()
            .unwrap();
        assert_eq!(network.quality, MetricQuality::Native);
        assert_eq!(network.source, Some(MetricSource::Etw));
        assert_eq!(network.message, None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_collector_reports_procfs_metric_sources() {
        let collector = TelemetryCollector::new();
        let sample = collector.collect().unwrap();

        let system_quality = sample.system.quality.unwrap();
        assert_eq!(
            system_quality.memory.unwrap().source,
            Some(MetricSource::Procfs)
        );
        assert_eq!(
            system_quality.network.unwrap().source,
            Some(MetricSource::Procfs)
        );
        assert!(
            sample.processes.iter().any(|process| {
                process
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.memory.as_ref())
                    .and_then(|memory| memory.source)
                    == Some(MetricSource::Procfs)
            }),
            "expected at least one Linux process row sourced from procfs"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_collector_reports_native_process_and_honest_system_sources() {
        let collector = TelemetryCollector::new();
        let sample = collector.collect().expect("macOS telemetry sample");

        assert!(sample.system.memory_available_bytes.is_some());
        let system_quality = sample.system.quality.as_ref().expect("system quality");
        assert_eq!(
            system_quality
                .memory
                .as_ref()
                .and_then(|quality| quality.source),
            Some(MetricSource::Sysinfo)
        );
        let disk = system_quality.disk.as_ref().expect("disk quality");
        assert_eq!(disk.source, Some(MetricSource::Runtime));
        assert_eq!(disk.quality, MetricQuality::Unavailable);
        assert_eq!(sample.system.disk_read_total_bytes, 0);
        assert_eq!(sample.system.disk_write_total_bytes, 0);
        assert_eq!(
            system_quality
                .network
                .as_ref()
                .and_then(|quality| quality.source),
            Some(MetricSource::InterfaceAggregate)
        );

        let current_pid = std::process::id().to_string();
        let current = sample
            .processes
            .iter()
            .find(|process| process.pid == current_pid)
            .expect("collector includes current process");
        assert!(current.private_bytes > 0);
        assert!(current.threads > 0);
        let quality = current.quality.as_ref().expect("process quality");
        assert_eq!(
            quality.memory.as_ref().and_then(|quality| quality.source),
            Some(MetricSource::DirectApi)
        );
        assert_eq!(
            quality.network.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(current.network_received_bps, None);
        assert_eq!(current.network_transmitted_bps, None);
    }

    #[cfg(windows)]
    fn sample_process(pid: &str) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1,
            name: "Process".to_string(),
            exe: String::new(),
            status: "running".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 0,
            private_bytes: 0,
            virtual_memory_bytes: None,
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
            quality: Some(native_process_quality(AccessState::Full, true)),
        }
    }
}
