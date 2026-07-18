use std::{
    cmp::Ordering,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(not(target_os = "linux"))]
use std::collections::HashMap;

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind, System,
    UpdateKind,
};

use crate::contracts::{
    AccessState, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
    ProcessMetricQuality, ProcessSample, RuntimeCollectorServiceStatus, RuntimeCollectorState,
    SystemMetricQuality, SystemMetricsSnapshot,
};
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
use crate::network_attribution::{
    NetworkAttributionBinder, NetworkAttributionSample, ProcessGeneration,
};

#[cfg(windows)]
use crate::windows_process;
#[cfg(target_os = "linux")]
use crate::{
    linux_network::LinuxNetworkAttribution, linux_process::LinuxProcessCollector,
    linux_system::LinuxSystemCollector,
};
#[cfg(target_os = "macos")]
use crate::{
    macos_network::MacosNetworkAttribution, macos_process::MacosProcessCollector,
    macos_system::MacosSystemCollector,
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
    pub collector_state: RuntimeCollectorState,
    pub system: SystemMetricsSnapshot,
    pub processes: Vec<ProcessSample>,
    pub warnings: Vec<String>,
    pub collector_service: Option<RuntimeCollectorServiceStatus>,
    pub source_provenance: Option<TelemetrySampleProvenance>,
    pub standard_fallback_process_etw_disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySampleProvenance {
    pub source_instance_id: String,
    pub source_sample_seq: u64,
    pub sampled_at_ms: u64,
}

pub struct TelemetryCollector {
    system: System,
    networks: Networks,
    #[cfg(target_os = "linux")]
    linux_system: LinuxSystemCollector,
    #[cfg(target_os = "linux")]
    linux_processes: LinuxProcessCollector,
    #[cfg(target_os = "linux")]
    linux_network_attribution: LinuxNetworkAttribution,
    #[cfg(target_os = "macos")]
    macos_system: MacosSystemCollector,
    #[cfg(target_os = "macos")]
    macos_processes: MacosProcessCollector,
    #[cfg(target_os = "macos")]
    macos_network_attribution: MacosNetworkAttribution,
    #[cfg(windows)]
    previous_cpu_times: Option<windows_system::CpuTimes>,
    #[cfg(windows)]
    windows_network: windows_system::WindowsNetworkSampler,
    #[cfg(windows)]
    pdh_disk: PdhDiskState,
    #[cfg(windows)]
    network_attribution: NetworkAttributionState,
    network_generation_binder: NetworkAttributionBinder,
    standard_fallback_process_etw_disabled: bool,
}

impl TelemetryCollector {
    #[cfg(not(windows))]
    pub fn new() -> Self {
        Self::new_inner(false)
    }

    #[cfg(windows)]
    pub(crate) fn for_collector_service(monitor: NetworkAttributionMonitor) -> Self {
        Self::new_inner(
            NetworkAttributionState::ServiceReady(Box::new(monitor)),
            false,
        )
    }

    #[cfg(windows)]
    pub(crate) fn for_standard_fallback() -> Self {
        // The standard-user fallback never competes with the SCM service for
        // machine-global ETW ownership. #70 owns the eventual service lease.
        Self::new_inner(NetworkAttributionState::Disabled, true)
    }

    #[cfg(windows)]
    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let prior = std::mem::replace(
            &mut self.network_attribution,
            NetworkAttributionState::Disabled,
        );
        match prior {
            NetworkAttributionState::ServiceReady(mut monitor) => monitor.shutdown(),
            NetworkAttributionState::Disabled => Ok(()),
        }
    }

    fn new_inner(
        #[cfg(windows)] network_attribution: NetworkAttributionState,
        standard_fallback_process_etw_disabled: bool,
    ) -> Self {
        Self {
            system: System::new_with_specifics(sysinfo_refresh_kind()),
            networks: Networks::new_with_refreshed_list(),
            #[cfg(target_os = "linux")]
            linux_system: LinuxSystemCollector::new(),
            #[cfg(target_os = "linux")]
            linux_processes: LinuxProcessCollector::new(),
            #[cfg(target_os = "linux")]
            linux_network_attribution: LinuxNetworkAttribution::new(),
            #[cfg(target_os = "macos")]
            macos_system: MacosSystemCollector::new(),
            #[cfg(target_os = "macos")]
            macos_processes: MacosProcessCollector::new(),
            #[cfg(target_os = "macos")]
            macos_network_attribution: MacosNetworkAttribution::new(),
            #[cfg(windows)]
            previous_cpu_times: None,
            #[cfg(windows)]
            windows_network: windows_system::WindowsNetworkSampler::new(),
            #[cfg(windows)]
            pdh_disk: PdhDiskState::new(),
            #[cfg(windows)]
            network_attribution,
            network_generation_binder: NetworkAttributionBinder::default(),
            standard_fallback_process_etw_disabled,
        }
    }

    pub fn collect(&mut self) -> Result<TelemetrySample, String> {
        #[cfg(target_os = "linux")]
        {
            self.collect_linux()
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.collect_sysinfo_seeded()
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn collect_sysinfo_seeded(&mut self) -> Result<TelemetrySample, String> {
        let started = Instant::now();
        let mut warnings = Vec::new();

        self.system.refresh_specifics(sysinfo_refresh_kind());
        self.networks.refresh(true);

        let sysinfo_processes = collect_sysinfo_processes(&self.system);
        let sysinfo_cpu_by_generation = sysinfo_processes
            .iter()
            .filter_map(|process| {
                ProcessGeneration::from_process(process)
                    .map(|generation| (generation, process.cpu_percent))
            })
            .collect::<HashMap<_, _>>();
        let logical_cpu_percent = logical_cpu_percent(&self.system, &mut warnings);
        let sysinfo_snapshot = collect_sysinfo_system(&self.system, &self.networks);
        let mut processes = collect_processes(
            &sysinfo_processes,
            &sysinfo_cpu_by_generation,
            &mut warnings,
            self,
        )?;
        let mut system_snapshot = collect_system_snapshot(
            sysinfo_snapshot,
            &logical_cpu_percent,
            &processes,
            &mut warnings,
            self,
        )?;
        #[cfg(windows)]
        {
            let network_attribution = self.network_attribution_sample(&mut warnings);
            apply_network_attribution(
                &mut processes,
                network_attribution,
                MetricSource::Etw,
                &mut self.network_generation_binder,
            );
        }
        #[cfg(target_os = "macos")]
        {
            let network_attribution = self.macos_network_attribution_sample(&mut warnings);
            apply_network_attribution(
                &mut processes,
                network_attribution,
                MetricSource::Nstat,
                &mut self.network_generation_binder,
            );
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
        let collector_state = collector_state(&system_snapshot, &processes, &warnings);

        Ok(TelemetrySample {
            latency_ms: started.elapsed().as_millis() as u64,
            collector_state,
            system: system_snapshot,
            processes,
            warnings,
            collector_service: None,
            source_provenance: None,
            standard_fallback_process_etw_disabled: self.standard_fallback_process_etw_disabled,
        })
    }

    #[cfg(target_os = "linux")]
    fn collect_linux(&mut self) -> Result<TelemetrySample, String> {
        let started = Instant::now();
        let process_result = self.linux_processes.collect();
        let system_result = self.linux_system.sample();

        let needs_fallback = process_result.is_err() || system_result.is_err();
        let (fallback_processes, fallback_system) = if needs_fallback {
            self.system.refresh_specifics(sysinfo_refresh_kind());
            self.networks.refresh(true);
            (
                Some(collect_sysinfo_processes(&self.system)),
                Some(collect_sysinfo_system(&self.system, &self.networks)),
            )
        } else {
            (None, None)
        };

        let mut warnings = Vec::new();
        let mut processes = match process_result {
            Ok(processes) => processes,
            Err(error) => {
                warnings.push(format!(
                    "linux_process_collector_failed:{error}; using sysinfo fallback"
                ));
                fallback_processes.unwrap_or_default()
            }
        };
        let mut system = match system_result {
            Ok(system) => system,
            Err(error) => {
                warnings.push(format!(
                    "linux_system_collector_failed:{error}; using sysinfo fallback"
                ));
                fallback_system.expect("sysinfo fallback is collected after native failure")
            }
        };
        let network_attribution = self.linux_network_attribution_sample(&mut warnings);
        apply_network_attribution(
            &mut processes,
            network_attribution,
            MetricSource::Ebpf,
            &mut self.network_generation_binder,
        );
        processes.sort_by(|left, right| {
            right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
                .then_with(|| left.name.cmp(&right.name))
        });
        system.process_count = processes.len();
        let collector_state = collector_state(&system, &processes, &warnings);
        Ok(TelemetrySample {
            latency_ms: started.elapsed().as_millis() as u64,
            collector_state,
            system,
            processes,
            warnings,
            collector_service: None,
            source_provenance: None,
            standard_fallback_process_etw_disabled: self.standard_fallback_process_etw_disabled,
        })
    }
}

fn collector_state(
    system: &SystemMetricsSnapshot,
    processes: &[ProcessSample],
    warnings: &[String],
) -> RuntimeCollectorState {
    let system_limited = system.quality.as_ref().is_none_or(|quality| {
        [
            quality.cpu.as_ref(),
            quality.kernel_cpu.as_ref(),
            quality.logical_cpu.as_ref(),
            quality.memory.as_ref(),
            quality.swap.as_ref(),
            quality.disk.as_ref(),
            quality.network.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(metric_degrades_collector)
    });
    let process_limited = processes.iter().any(|process| {
        process.quality.as_ref().is_none_or(|quality| {
            [
                quality.cpu.as_ref(),
                quality.memory.as_ref(),
                quality.io.as_ref(),
                quality.other_io.as_ref(),
                quality.network.as_ref(),
                quality.threads.as_ref(),
                quality.handles.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(metric_degrades_collector)
        })
    });

    if warnings.is_empty() && !system_limited && !process_limited {
        RuntimeCollectorState::Healthy
    } else {
        RuntimeCollectorState::Limited
    }
}

fn metric_degrades_collector(quality: &MetricQualityInfo) -> bool {
    quality.limitation_code.is_some_and(|limitation| {
        matches!(
            limitation,
            MetricLimitationCode::CollectorFailure
                | MetricLimitationCode::DataLoss
                | MetricLimitationCode::MissingMetadata
                | MetricLimitationCode::NumericRange
        )
    }) || (quality.quality == MetricQuality::Unavailable && quality.limitation_code.is_none())
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

#[cfg(not(target_os = "linux"))]
fn collect_processes(
    sysinfo_processes: &[ProcessSample],
    sysinfo_cpu_by_generation: &HashMap<ProcessGeneration, f64>,
    warnings: &mut Vec<String>,
    collector: &mut TelemetryCollector,
) -> Result<Vec<ProcessSample>, String> {
    #[cfg(windows)]
    let _ = collector;

    #[cfg(windows)]
    {
        match windows_process::collect_processes(0) {
            Ok(native_processes) => {
                let sysinfo_by_generation = sysinfo_processes
                    .iter()
                    .filter_map(|process| {
                        ProcessGeneration::from_process(process)
                            .map(|generation| (generation, process))
                    })
                    .collect::<HashMap<_, _>>();
                Ok(native_processes
                    .into_iter()
                    .map(|process| {
                        enrich_native_process(
                            process,
                            sysinfo_cpu_by_generation,
                            &sysinfo_by_generation,
                        )
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

    #[cfg(target_os = "macos")]
    {
        let _ = sysinfo_cpu_by_generation;
        let mut processes = sysinfo_processes.to_vec();
        let process_count = processes.len();
        let collection = collector.macos_processes.enrich(&mut processes);
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
        let _ = (sysinfo_cpu_by_generation, warnings, collector);
        Ok(sysinfo_processes.to_vec())
    }
}

#[cfg(not(target_os = "linux"))]
fn collect_system_snapshot(
    sysinfo_snapshot: SystemMetricsSnapshot,
    logical_cpu_percent: &[f64],
    processes: &[ProcessSample],
    warnings: &mut Vec<String>,
    collector: &mut TelemetryCollector,
) -> Result<SystemMetricsSnapshot, String> {
    #[cfg(windows)]
    {
        let _ = processes;
        let cpu_load = collector.native_cpu_load(warnings);

        match windows_system::sample_system(&mut collector.windows_network) {
            Ok(mut snapshot) => {
                snapshot.logical_cpu_percent = logical_cpu_percent.to_vec();
                if let Some(load) = cpu_load {
                    snapshot.cpu_percent = load.cpu_percent;
                    snapshot.kernel_cpu_percent = load.kernel_cpu_percent;
                } else {
                    snapshot.cpu_percent = sysinfo_snapshot.cpu_percent;
                    snapshot.kernel_cpu_percent = 0.0;
                }
                let disk_quality = collector.apply_pdh_disk_rates(&mut snapshot, warnings);
                let network_quality = snapshot
                    .quality
                    .as_ref()
                    .and_then(|quality| quality.network.clone())
                    .unwrap_or_else(|| {
                        MetricQualityInfo::new(
                            MetricQuality::Unavailable,
                            MetricSource::InterfaceAggregate,
                        )
                        .with_limitation(
                            MetricLimitationCode::CollectorFailure,
                            "Windows interface quality metadata is unavailable.",
                        )
                    });
                snapshot.quality = Some(system_quality(
                    cpu_load.is_some(),
                    disk_quality,
                    network_quality,
                ));
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

    #[cfg(target_os = "macos")]
    {
        let _ = (logical_cpu_percent, warnings);
        let mut snapshot = sysinfo_snapshot;
        collector.macos_system.enrich(&mut snapshot, processes);
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
        &mut self,
        snapshot: &mut SystemMetricsSnapshot,
        warnings: &mut Vec<String>,
    ) -> DiskQualityState {
        if matches!(&self.pdh_disk, PdhDiskState::Failed(_, failed_at) if failed_at.elapsed() >= std::time::Duration::from_secs(30))
        {
            self.pdh_disk = PdhDiskState::new();
        }
        match &mut self.pdh_disk {
            PdhDiskState::Ready(sampler) => match sampler.sample() {
                Ok(PdhSample::Ready(rates)) => {
                    snapshot.disk_read_bps = rates.read_bps;
                    snapshot.disk_write_bps = rates.write_bps;
                    DiskQualityState::Native
                }
                Ok(PdhSample::Held(message)) => DiskQualityState::Held(message),
                Err(error) => {
                    warnings.push(format!("pdh_disk_collector_failed:{error}"));
                    self.pdh_disk = PdhDiskState::Failed(error.clone(), Instant::now());
                    DiskQualityState::Unavailable(error)
                }
            },
            PdhDiskState::Failed(error, _) => {
                warnings.push(format!("pdh_disk_collector_failed:{error}"));
                DiskQualityState::Unavailable(error.clone())
            }
        }
    }

    #[cfg(windows)]
    fn network_attribution_sample(
        &mut self,
        warnings: &mut Vec<String>,
    ) -> NetworkAttributionSample {
        match &mut self.network_attribution {
            NetworkAttributionState::Disabled => NetworkAttributionSample::Held(
                "Network attribution is owned by the installed collector service.".to_string(),
            ),
            NetworkAttributionState::ServiceReady(monitor) => {
                let sample = monitor.sample();
                if let NetworkAttributionSample::Failed(message) = &sample {
                    warnings.push(format!("network_attribution_failed:{message}"));
                }
                sample
            }
        }
    }

    #[cfg(windows)]
    fn native_cpu_load(&mut self, warnings: &mut Vec<String>) -> Option<windows_system::CpuLoad> {
        let current = match windows_system::sample_cpu_times() {
            Ok(current) => current,
            Err(error) => {
                warnings.push(format!(
                    "native_cpu_collector_failed:{error}; using sysinfo fallback"
                ));
                return None;
            }
        };
        let load = self
            .previous_cpu_times
            .map(|previous| windows_system::calculate_cpu_load(previous, current));
        self.previous_cpu_times = Some(current);
        load
    }

    #[cfg(target_os = "linux")]
    fn linux_network_attribution_sample(
        &mut self,
        warnings: &mut Vec<String>,
    ) -> NetworkAttributionSample {
        let sample = self.linux_network_attribution.sample();
        if let NetworkAttributionSample::Failed(message) = &sample {
            warnings.push(format!("linux_network_attribution_failed:{message}"));
        }
        sample
    }

    #[cfg(target_os = "macos")]
    fn macos_network_attribution_sample(
        &mut self,
        warnings: &mut Vec<String>,
    ) -> NetworkAttributionSample {
        let sample = self.macos_network_attribution.sample();
        if let NetworkAttributionSample::Failed(message) = &sample {
            warnings.push(format!("macos_network_attribution_failed:{message}"));
        }
        sample
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
                    .with_limitation(
                        MetricLimitationCode::UnsupportedMetric,
                        "Kernel CPU is unavailable from the sysinfo fallback.",
                    ),
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
                    .with_limitation(
                        MetricLimitationCode::UnsupportedMetric,
                        "Windows reports commit accounting, not swap usage.",
                    )
            } else {
                MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
            }),
            disk: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_limitation(
                        MetricLimitationCode::UnsupportedMetric,
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
                    io: Some(process_io_seed_quality()),
                    other_io: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_limitation(
                                MetricLimitationCode::UnsupportedMetric,
                                "Other I/O is unavailable from the sysinfo fallback.",
                            ),
                    ),
                    network: Some(process_network_quality_unavailable()),
                    threads: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_limitation(
                                MetricLimitationCode::UnsupportedMetric,
                                "Thread counts require the native process collector.",
                            ),
                    ),
                    handles: Some(
                        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                            .with_limitation(
                                MetricLimitationCode::UnsupportedMetric,
                                "Handle counts require the native process collector.",
                            ),
                    ),
                }),
            }
        })
        .collect()
}

#[cfg(windows)]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Held, MetricSource::Etw).with_limitation(
        MetricLimitationCode::PendingBaseline,
        "Waiting for ETW network attribution.",
    )
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo).with_limitation(
        MetricLimitationCode::UnsupportedMetric,
        "Per-process network attribution is unavailable from the sysinfo fallback.",
    )
}

#[cfg(target_os = "macos")]
fn process_network_quality_unavailable() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
        MetricLimitationCode::UnsupportedMetric,
        "Per-process network attribution is unavailable on macOS.",
    )
}

#[cfg(target_os = "macos")]
fn process_io_seed_quality() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
        MetricLimitationCode::CollectorFailure,
        "Native process read/write totals have not been collected.",
    )
}

#[cfg(not(target_os = "macos"))]
fn process_io_seed_quality() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
}

#[cfg(not(target_os = "linux"))]
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
    sysinfo_cpu_by_generation: &HashMap<ProcessGeneration, f64>,
    sysinfo_by_generation: &HashMap<ProcessGeneration, &ProcessSample>,
) -> ProcessSample {
    let generation = ProcessGeneration::from_process(&process);
    let has_cpu = generation
        .and_then(|generation| sysinfo_cpu_by_generation.get(&generation))
        .map(|cpu| {
            process.cpu_percent = *cpu;
            true
        })
        .unwrap_or(false);
    let memory_from_sysinfo = generation
        .and_then(|generation| sysinfo_by_generation.get(&generation))
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
    apply_native_process_enrichment_quality(&mut process, has_cpu, memory_from_sysinfo);
    process
}

#[cfg(any(windows, test))]
fn apply_native_process_enrichment_quality(
    process: &mut ProcessSample,
    has_cpu: bool,
    memory_from_sysinfo: bool,
) {
    let quality = process_quality(process);
    quality.cpu = Some(if has_cpu {
        MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
    } else {
        MetricQualityInfo::new(MetricQuality::Held, MetricSource::Sysinfo).with_limitation(
            MetricLimitationCode::PendingBaseline,
            "Process CPU needs a second Rust-native timing pass.",
        )
    });
    quality.network = Some(process_network_quality_unavailable());
    if memory_from_sysinfo {
        quality.memory = Some(
            MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
                .with_limitation(
                    MetricLimitationCode::AccessDenied,
                    "Native process memory counters were denied; using sysinfo fallback memory.",
                ),
        );
    }
}

#[cfg(windows)]
fn system_quality(
    has_native_cpu: bool,
    disk_quality: DiskQualityState,
    network_quality: MetricQualityInfo,
) -> SystemMetricQuality {
    let cpu = if has_native_cpu {
        MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi)
    } else {
        MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
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
                .with_limitation(
                    MetricLimitationCode::UnsupportedMetric,
                    "Windows reports commit accounting, not swap usage.",
                ),
        ),
        disk: Some(match disk_quality {
            DiskQualityState::Native => {
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::Pdh)
            }
            DiskQualityState::Held(message) => {
                MetricQualityInfo::new(MetricQuality::Held, MetricSource::Pdh)
                    .with_limitation(MetricLimitationCode::HeldValue, &message)
            }
            DiskQualityState::Unavailable(message) => {
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Pdh)
                    .with_limitation(
                        MetricLimitationCode::CollectorFailure,
                        &format!("PDH physical disk telemetry is unavailable. {message}"),
                    )
            }
        }),
        network: Some(network_quality),
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
    ServiceReady(Box<NetworkAttributionMonitor>),
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn apply_network_attribution(
    processes: &mut [ProcessSample],
    attribution: NetworkAttributionSample,
    source: MetricSource,
    binder: &mut NetworkAttributionBinder,
) {
    match attribution {
        NetworkAttributionSample::Ready { rates_by_process } => {
            let bound = binder.bind(processes, rates_by_process);
            for process in processes {
                apply_bound_network_rates(process, &bound, source, None);
            }
        }
        NetworkAttributionSample::Partial {
            rates_by_process,
            message,
        } => {
            let bound = binder.bind(processes, rates_by_process);
            for process in processes {
                apply_bound_network_rates(process, &bound, source, Some(&message));
            }
        }
        NetworkAttributionSample::PendingBaseline(message) => {
            binder.observe(processes);
            for process in processes {
                process.network_received_bps = None;
                process.network_transmitted_bps = None;
                process_quality(process).network = Some(
                    MetricQualityInfo::new(MetricQuality::Held, source)
                        .with_limitation(MetricLimitationCode::PendingBaseline, &message),
                );
            }
        }
        NetworkAttributionSample::Held(message) => {
            for process in processes {
                process.network_received_bps = None;
                process.network_transmitted_bps = None;
                process_quality(process).network = Some(
                    MetricQualityInfo::new(MetricQuality::Held, source)
                        .with_limitation(MetricLimitationCode::HeldValue, &message),
                );
            }
        }
        NetworkAttributionSample::Failed(message) => {
            binder.clear();
            for process in processes {
                process.network_received_bps = None;
                process.network_transmitted_bps = None;
                process_quality(process).network = Some(
                    MetricQualityInfo::new(MetricQuality::Unavailable, source)
                        .with_limitation(MetricLimitationCode::CollectorFailure, &message),
                );
            }
        }
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn apply_bound_network_rates(
    process: &mut ProcessSample,
    bound: &crate::network_attribution::BoundNetworkRates,
    source: MetricSource,
    data_loss: Option<&str>,
) {
    let Some(generation) = ProcessGeneration::from_process(process) else {
        process.network_received_bps = None;
        process.network_transmitted_bps = None;
        process_quality(process).network = Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, source).with_limitation(
                MetricLimitationCode::MissingMetadata,
                "Process network activity cannot be joined without a stable start time.",
            ),
        );
        return;
    };
    if !bound.is_proven(generation) {
        process.network_received_bps = None;
        process.network_transmitted_bps = None;
        process_quality(process).network = Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, source).with_limitation(
                MetricLimitationCode::PartialCoverage,
                "Process generation changed during the attribution interval; activity was not assigned by PID.",
            ),
        );
        return;
    }

    let rates = bound.rates(generation);
    process.network_received_bps = Some(rates.received_bps);
    process.network_transmitted_bps = Some(rates.transmitted_bps);
    process_quality(process).network = Some(if let Some(message) = data_loss {
        MetricQualityInfo::new(MetricQuality::Partial, source)
            .with_limitation(MetricLimitationCode::DataLoss, message)
    } else {
        MetricQualityInfo::new(MetricQuality::Native, source)
    });
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn process_quality(process: &mut ProcessSample) -> &mut ProcessMetricQuality {
    process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default)
}

#[cfg(test)]
fn fully_native_process_quality() -> ProcessMetricQuality {
    let direct = || MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi);
    ProcessMetricQuality {
        cpu: Some(direct()),
        memory: Some(direct()),
        io: Some(direct()),
        other_io: Some(direct()),
        network: Some(process_network_quality_unavailable()),
        threads: Some(direct()),
        handles: Some(direct()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_attribution::{
        NetworkAttributionSample, ObservedProcessGeneration, ProcessNetworkRates,
    };
    use std::collections::HashMap;

    #[test]
    fn collector_health_separates_expected_coverage_gaps_from_failures() {
        for limitation in [
            MetricLimitationCode::UnsupportedMetric,
            MetricLimitationCode::AccessDenied,
            MetricLimitationCode::AuthorizationScope,
            MetricLimitationCode::PartialCoverage,
            MetricLimitationCode::PendingBaseline,
            MetricLimitationCode::HeldValue,
            MetricLimitationCode::GroupPartialCoverage,
        ] {
            let quality = MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Runtime)
                .with_limitation(limitation, "Expected platform coverage gap.");
            assert!(!metric_degrades_collector(&quality), "{limitation:?}");
        }

        for limitation in [
            MetricLimitationCode::CollectorFailure,
            MetricLimitationCode::DataLoss,
            MetricLimitationCode::MissingMetadata,
            MetricLimitationCode::NumericRange,
        ] {
            let quality = MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Runtime)
                .with_limitation(limitation, "Operational collector failure.");
            assert!(metric_degrades_collector(&quality), "{limitation:?}");
        }

        assert!(metric_degrades_collector(&MetricQualityInfo::new(
            MetricQuality::Unavailable,
            MetricSource::Runtime,
        )));
    }

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

    #[test]
    fn native_enrichment_preserves_unrelated_per_field_quality() {
        let mut process = ProcessSample {
            pid: "42".to_string(),
            parent_pid: None,
            start_time_ms: 1,
            name: "Mixed".to_string(),
            exe: String::new(),
            status: "running".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 123,
            private_bytes: 45,
            virtual_memory_bytes: None,
            io_read_total_bytes: 0,
            io_write_total_bytes: 0,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 3,
            handles: 0,
            access_state: AccessState::Partial,
            quality: Some(ProcessMetricQuality {
                memory: Some(MetricQualityInfo::new(
                    MetricQuality::Unavailable,
                    MetricSource::DirectApi,
                )),
                io: Some(MetricQualityInfo::new(
                    MetricQuality::Unavailable,
                    MetricSource::DirectApi,
                )),
                other_io: Some(MetricQualityInfo::new(
                    MetricQuality::Unavailable,
                    MetricSource::DirectApi,
                )),
                threads: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::DirectApi,
                )),
                handles: Some(MetricQualityInfo::new(
                    MetricQuality::Unavailable,
                    MetricSource::DirectApi,
                )),
                ..ProcessMetricQuality::default()
            }),
        };

        apply_native_process_enrichment_quality(&mut process, true, true);
        let quality = process.quality.expect("process quality");
        assert_eq!(
            quality.memory.map(|quality| quality.quality),
            Some(MetricQuality::Estimated)
        );
        assert_eq!(
            quality.io.map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.other_io.map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.threads.map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );
        assert_eq!(
            quality.handles.map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.cpu.map(|quality| quality.quality),
            Some(MetricQuality::Estimated)
        );
        assert_eq!(
            quality.network.map(|quality| quality.quality),
            Some(if cfg!(windows) {
                MetricQuality::Held
            } else {
                MetricQuality::Unavailable
            })
        );
    }

    #[test]
    fn native_enrichment_marks_first_process_cpu_pass_as_pending() {
        let mut process = sample_process("42");

        apply_native_process_enrichment_quality(&mut process, false, false);

        let cpu = process
            .quality
            .and_then(|quality| quality.cpu)
            .expect("CPU quality");
        assert_eq!(cpu.quality, MetricQuality::Held);
        assert_eq!(
            cpu.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );
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
        let generation = ProcessGeneration {
            pid: 42,
            start_time_ms: 1,
        };
        let sysinfo_cpu_by_generation = HashMap::from([(generation, 9.0)]);
        let sysinfo_by_generation = HashMap::from([(generation, &fallback)]);

        let enriched =
            enrich_native_process(native, &sysinfo_cpu_by_generation, &sysinfo_by_generation);

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
    fn enrich_native_process_rejects_pid_reuse_between_probes() {
        let mut native = sample_process("42");
        native.start_time_ms = 2;
        native.memory_bytes = 0;
        native.private_bytes = 0;
        let mut old_generation = sample_process("42");
        old_generation.start_time_ms = 1;
        old_generation.memory_bytes = 999;
        old_generation.private_bytes = 777;
        let old_identity = ProcessGeneration {
            pid: 42,
            start_time_ms: 1,
        };

        let enriched = enrich_native_process(
            native,
            &HashMap::from([(old_identity, 75.0)]),
            &HashMap::from([(old_identity, &old_generation)]),
        );

        assert_eq!(enriched.cpu_percent, 0.0);
        assert_eq!(enriched.memory_bytes, 0);
        assert_eq!(enriched.private_bytes, 0);
    }

    #[cfg(windows)]
    #[test]
    fn standard_fallback_keeps_etw_disabled() {
        let mut collector = TelemetryCollector::for_standard_fallback();

        assert!(matches!(
            collector.network_attribution,
            NetworkAttributionState::Disabled
        ));
        assert!(collector.standard_fallback_process_etw_disabled);
        assert!(
            collector
                .collect()
                .expect("standard fallback sample")
                .standard_fallback_process_etw_disabled
        );
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
            quality: Some(fully_native_process_quality()),
        }];
        let sample = NetworkAttributionSample::ready([(
            ProcessGeneration {
                pid: 42,
                start_time_ms: 1,
            },
            ProcessNetworkRates {
                received_bps: 4096,
                transmitted_bps: 2048,
            },
        )]);

        let mut binder = NetworkAttributionBinder::default();
        binder.observe(&processes);
        apply_network_attribution(&mut processes, sample, MetricSource::Etw, &mut binder);

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

    #[test]
    fn apply_network_attribution_keeps_unproven_zero_out_of_native_quality() {
        let mut processes = vec![sample_process("42")];

        apply_network_attribution(
            &mut processes,
            NetworkAttributionSample::PendingBaseline(
                "Waiting for a supported ETW event.".to_string(),
            ),
            MetricSource::Etw,
            &mut NetworkAttributionBinder::default(),
        );

        assert_eq!(processes[0].network_received_bps, None);
        assert_eq!(processes[0].network_transmitted_bps, None);
        let network = processes[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.network.as_ref())
            .expect("network quality");
        assert_eq!(network.quality, MetricQuality::Held);
        assert_eq!(
            network.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );
    }

    #[test]
    fn apply_network_attribution_marks_lossy_rows_partial() {
        let mut processes = vec![sample_process("42")];
        let mut binder = NetworkAttributionBinder::default();
        binder.observe(&processes);

        apply_network_attribution(
            &mut processes,
            NetworkAttributionSample::Partial {
                rates_by_process: HashMap::from([(
                    ObservedProcessGeneration::pid_only(42),
                    ProcessNetworkRates {
                        received_bps: 512,
                        transmitted_bps: 256,
                    },
                )]),
                message: "ETW reported data loss.".to_string(),
            },
            MetricSource::Etw,
            &mut binder,
        );

        assert_eq!(processes[0].network_received_bps, Some(512));
        assert_eq!(processes[0].network_transmitted_bps, Some(256));
        let network = processes[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.network.as_ref())
            .expect("network quality");
        assert_eq!(network.quality, MetricQuality::Partial);
        assert_eq!(
            network.limitation_code,
            Some(MetricLimitationCode::DataLoss)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_collector_reports_procfs_metric_sources() {
        let mut collector = TelemetryCollector::new();
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
    fn macos_sysinfo_seed_never_claims_process_io_provenance() {
        let quality = process_io_seed_quality();
        assert_eq!(quality.quality, MetricQuality::Unavailable);
        assert_eq!(quality.source, Some(MetricSource::Libproc));
        assert_eq!(
            quality.limitation_code,
            Some(MetricLimitationCode::CollectorFailure)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_collector_reports_native_process_and_honest_system_sources() {
        let mut collector = TelemetryCollector::new();
        let first = collector
            .collect()
            .expect("macOS baseline telemetry sample");
        let first_disk = first
            .system
            .quality
            .as_ref()
            .and_then(|quality| quality.disk.as_ref())
            .expect("baseline disk quality");
        assert_eq!(first_disk.source, Some(MetricSource::Iokit));
        assert_eq!(first_disk.quality, MetricQuality::Held);
        assert_eq!(
            first_disk.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let sample = loop {
            let sample = collector.collect().expect("macOS telemetry sample");
            let current_pid = std::process::id().to_string();
            let network_ready = sample.processes.iter().any(|process| {
                process.pid == current_pid
                    && process
                        .quality
                        .as_ref()
                        .and_then(|quality| quality.network.as_ref())
                        .is_some_and(|quality| quality.quality == MetricQuality::Native)
            });
            if network_ready || Instant::now() >= deadline {
                break sample;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(sample.collector_state, RuntimeCollectorState::Limited);

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
        assert_eq!(disk.source, Some(MetricSource::Iokit));
        assert_eq!(disk.quality, MetricQuality::Native);
        assert!(sample.system.disk_read_total_bytes > 0);
        assert_eq!(
            system_quality
                .network
                .as_ref()
                .and_then(|quality| quality.source),
            Some(MetricSource::Sysinfo)
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
            Some(MetricSource::Libproc)
        );
        assert_eq!(
            quality.network.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );
        assert_eq!(
            quality.network.as_ref().and_then(|quality| quality.source),
            Some(MetricSource::Nstat)
        );
        assert!(current.network_received_bps.is_some());
        assert!(current.network_transmitted_bps.is_some());
    }

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
            quality: Some(fully_native_process_quality()),
        }
    }
}
