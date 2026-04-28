use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use sysinfo::{Networks, System};

use crate::contracts::{
    AccessState, MetricQuality, MetricQualityInfo, MetricSource, ProcessMetricQuality,
    ProcessSample, SystemMetricQuality, SystemMetricsSnapshot,
};
use crate::network_attribution::NetworkAttributionSample;

#[cfg(windows)]
use crate::windows_process;
#[cfg(target_os = "linux")]
use crate::{
    linux_network::LinuxNetworkAttributionMonitor, linux_process::LinuxProcessCollector,
    linux_system::LinuxSystemCollector,
};
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
    #[cfg(windows)]
    previous_cpu_times: Mutex<Option<windows_system::CpuTimes>>,
    #[cfg(windows)]
    pdh_disk: Mutex<PdhDiskState>,
    #[cfg(windows)]
    network_attribution: Mutex<NetworkAttributionState>,
}

impl TelemetryCollector {
    pub fn new() -> Self {
        Self {
            system: Mutex::new(System::new_all()),
            networks: Mutex::new(Networks::new_with_refreshed_list()),
            #[cfg(target_os = "linux")]
            linux_system: Mutex::new(LinuxSystemCollector::new()),
            #[cfg(target_os = "linux")]
            linux_processes: Mutex::new(LinuxProcessCollector::new()),
            #[cfg(target_os = "linux")]
            linux_network_attribution: Mutex::new(LinuxNetworkAttributionState::new()),
            #[cfg(windows)]
            previous_cpu_times: Mutex::new(None),
            #[cfg(windows)]
            pdh_disk: Mutex::new(PdhDiskState::new()),
            #[cfg(windows)]
            network_attribution: Mutex::new(NetworkAttributionState::new()),
        }
    }

    pub fn collect(&self) -> Result<TelemetrySample, String> {
        let started = Instant::now();
        let mut warnings = Vec::new();

        let mut sysinfo_system = self
            .system
            .lock()
            .map_err(|_| "system telemetry lock is poisoned".to_string())?;
        sysinfo_system.refresh_all();

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
        let mut system_snapshot = collect_system_snapshot(
            &sysinfo_system,
            &sysinfo_networks,
            &logical_cpu_percent,
            &mut warnings,
            self,
        )?;
        let mut processes =
            collect_processes(&sysinfo_processes, &sysinfo_cpu_by_pid, &mut warnings, self)?;
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

        if system_snapshot.disk_read_total_bytes == 0 && system_snapshot.disk_write_total_bytes == 0
        {
            system_snapshot.disk_read_total_bytes = disk_read_total_bytes;
            system_snapshot.disk_write_total_bytes = disk_write_total_bytes;
        }
        system_snapshot.process_count = processes.len();

        Ok(TelemetrySample {
            latency_ms: started.elapsed().as_millis() as u64,
            system: system_snapshot,
            processes,
            warnings,
        })
    }
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
            Ok(native_processes) => Ok(native_processes
                .into_iter()
                .map(|process| enrich_native_process(process, sysinfo_cpu_by_pid))
                .collect()),
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

    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        let _ = (sysinfo_cpu_by_pid, warnings, collector);
        Ok(sysinfo_processes.to_vec())
    }
}

fn collect_system_snapshot(
    sysinfo_system: &System,
    sysinfo_networks: &Networks,
    logical_cpu_percent: &[f64],
    warnings: &mut Vec<String>,
    collector: &TelemetryCollector,
) -> Result<SystemMetricsSnapshot, String> {
    let sysinfo_snapshot = collect_sysinfo_system(sysinfo_system, sysinfo_networks);

    #[cfg(windows)]
    {
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
        let _ = (sysinfo_system, sysinfo_networks, logical_cpu_percent);
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

    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        let _ = (logical_cpu_percent, warnings, collector);
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
                    *state = PdhDiskState::Failed(error.clone());
                    Ok(DiskQualityState::Unavailable(error))
                }
            },
            PdhDiskState::Failed(error) => Ok(DiskQualityState::Unavailable(error.clone())),
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
            NetworkAttributionState::Ready(monitor) => {
                let sample = monitor.sample();
                if let NetworkAttributionSample::Failed(message) = &sample {
                    warnings.push(format!("network_attribution_failed:{message}"));
                    *state = NetworkAttributionState::Failed {
                        message: message.clone(),
                        warned: true,
                    };
                }
                Ok(sample)
            }
            NetworkAttributionState::Failed { message, warned } => {
                if !*warned {
                    warnings.push(format!("network_attribution_failed:{message}"));
                    *warned = true;
                }
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
                        warned: true,
                    };
                }
                Ok(sample)
            }
            LinuxNetworkAttributionState::Failed { message, warned } => {
                if !*warned {
                    warnings.push(format!("linux_network_attribution_failed:{message}"));
                    *warned = true;
                }
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
        memory_available_bytes: Some(system.total_memory().saturating_sub(system.used_memory())),
        swap_used_bytes: system.used_swap(),
        swap_total_bytes: system.total_swap(),
        process_count: system.processes().len(),
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes,
        network_transmitted_total_bytes,
        network_received_bps: 0,
        network_transmitted_bps: 0,
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
            swap: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            disk: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
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
                virtual_memory_bytes: process.virtual_memory(),
                disk_read_total_bytes: disk_usage.total_read_bytes,
                disk_write_total_bytes: disk_usage.total_written_bytes,
                other_io_total_bytes: None,
                disk_read_bps: 0,
                disk_write_bps: 0,
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
                    disk: Some(MetricQualityInfo::new(
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

#[cfg(not(windows))]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
        .with_message("Per-process network attribution is unavailable from the sysinfo fallback.")
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
) -> ProcessSample {
    let has_cpu = sysinfo_cpu_by_pid
        .get(&process.pid)
        .map(|cpu| {
            process.cpu_percent = *cpu;
            true
        })
        .unwrap_or(false);
    process.quality = Some(native_process_quality(process.access_state, has_cpu));
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
        swap: Some(MetricQualityInfo::new(
            MetricQuality::Native,
            MetricSource::DirectApi,
        )),
        disk: Some(match disk_quality {
            DiskQualityState::Native => {
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::Pdh)
            }
            DiskQualityState::Held(message) => {
                MetricQualityInfo::new(MetricQuality::Held, MetricSource::Pdh)
                    .with_message(&message)
            }
            DiskQualityState::Unavailable(message) => {
                MetricQualityInfo::new(MetricQuality::Partial, MetricSource::ProcessAggregate)
                    .with_message(&format!(
                        "PDH disk rates unavailable; using process I/O totals. {message}"
                    ))
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
    Failed(String),
}

#[cfg(windows)]
impl PdhDiskState {
    fn new() -> Self {
        match PdhDiskSampler::new() {
            Ok(sampler) => Self::Ready(sampler),
            Err(error) => Self::Failed(error),
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
    Ready(NetworkAttributionMonitor),
    Failed { message: String, warned: bool },
}

#[cfg(windows)]
impl NetworkAttributionState {
    fn new() -> Self {
        match NetworkAttributionMonitor::new() {
            Ok(monitor) => Self::Ready(monitor),
            Err(message) => Self::Failed {
                message,
                warned: false,
            },
        }
    }
}

#[cfg(target_os = "linux")]
enum LinuxNetworkAttributionState {
    Ready(LinuxNetworkAttributionMonitor),
    Failed { message: String, warned: bool },
}

#[cfg(target_os = "linux")]
impl LinuxNetworkAttributionState {
    fn new() -> Self {
        match LinuxNetworkAttributionMonitor::start() {
            Ok(monitor) => Self::Ready(monitor),
            Err(message) => Self::Failed {
                message,
                warned: false,
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
        disk: Some(direct(None)),
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
            virtual_memory_bytes: 0,
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
}
