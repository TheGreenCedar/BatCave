#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use crate::contracts::{
    MetricQuality, MetricQualityInfo, MetricSource, SystemMetricQuality, SystemMetricsSnapshot,
};

#[cfg(windows)]
use std::{mem::size_of, ptr::null_mut, slice};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, FILETIME},
    NetworkManagement::IpHelper::{
        GetIfTable, IF_OPER_STATUS_CONNECTED, IF_OPER_STATUS_OPERATIONAL,
        IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL, MIB_IFROW, MIB_IFTABLE,
    },
    System::{
        ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::GetSystemTimes,
    },
};

#[cfg(windows)]
pub fn sample_system() -> Result<SystemMetricsSnapshot, String> {
    let memory = sample_memory()?;
    let _cpu_times = sample_cpu_times()?;
    let network = sample_network_totals()?;
    let process_count = sample_process_count().unwrap_or_default();

    Ok(SystemMetricsSnapshot {
        cpu_percent: 0.0,
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: Vec::new(),
        memory_used_bytes: memory.used_bytes,
        memory_total_bytes: memory.total_bytes,
        memory_available_bytes: Some(memory.available_bytes),
        swap_used_bytes: memory.pagefile_used_bytes,
        swap_total_bytes: memory.pagefile_total_bytes,
        process_count,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: network.received_bytes,
        network_transmitted_total_bytes: network.transmitted_bytes,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        quality: Some(SystemMetricQuality {
            cpu: Some(
                MetricQualityInfo::new(MetricQuality::Held, MetricSource::DirectApi).with_message(
                    "CPU requires a second sample before native deltas are available.",
                ),
            ),
            kernel_cpu: Some(
                MetricQualityInfo::new(MetricQuality::Held, MetricSource::DirectApi).with_message(
                    "Kernel CPU requires a second sample before native deltas are available.",
                ),
            ),
            logical_cpu: None,
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            )),
            swap: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            )),
            disk: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Pdh)
                    .with_message("Disk counters need the PDH collector layer."),
            ),
            network: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::InterfaceAggregate,
            )),
        }),
    })
}

#[cfg(not(windows))]
pub fn sample_system() -> Result<SystemMetricsSnapshot, String> {
    Err("windows_system_collector_requires_windows".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryMetrics {
    used_bytes: u64,
    total_bytes: u64,
    available_bytes: u64,
    pagefile_used_bytes: u64,
    pagefile_total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CpuLoad {
    pub cpu_percent: f64,
    pub kernel_cpu_percent: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NetworkTotals {
    received_bytes: u64,
    transmitted_bytes: u64,
}

pub(crate) fn calculate_cpu_load(previous: CpuTimes, current: CpuTimes) -> CpuLoad {
    let idle_delta = current.idle.saturating_sub(previous.idle);
    let kernel_delta = current.kernel.saturating_sub(previous.kernel);
    let user_delta = current.user.saturating_sub(previous.user);
    let total_delta = kernel_delta.saturating_add(user_delta);

    if total_delta == 0 {
        return CpuLoad {
            cpu_percent: 0.0,
            kernel_cpu_percent: 0.0,
        };
    }

    let busy_delta = total_delta.saturating_sub(idle_delta);
    let kernel_busy_delta = kernel_delta.saturating_sub(idle_delta);

    CpuLoad {
        cpu_percent: percent(busy_delta, total_delta),
        kernel_cpu_percent: percent(kernel_busy_delta, total_delta),
    }
}

#[cfg(windows)]
fn sample_memory() -> Result<MemoryMetrics, String> {
    let mut status = MEMORYSTATUSEX::default();
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return Err("GlobalMemoryStatusEx failed".to_string());
    }

    Ok(memory_metrics_from_status(
        status.ullTotalPhys,
        status.ullAvailPhys,
        status.ullTotalPageFile,
        status.ullAvailPageFile,
    ))
}

#[cfg(windows)]
pub(crate) fn sample_cpu_times() -> Result<CpuTimes, String> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();

    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    if ok == 0 {
        return Err("GetSystemTimes failed".to_string());
    }

    Ok(CpuTimes {
        idle: filetime_to_u64(idle),
        kernel: filetime_to_u64(kernel),
        user: filetime_to_u64(user),
    })
}

#[cfg(not(windows))]
pub(crate) fn sample_cpu_times() -> Result<CpuTimes, String> {
    Err("windows_system_cpu_requires_windows".to_string())
}

#[cfg(windows)]
fn sample_process_count() -> Result<usize, String> {
    let mut performance = PERFORMANCE_INFORMATION::default();
    performance.cb = size_of::<PERFORMANCE_INFORMATION>() as u32;

    let ok = unsafe { GetPerformanceInfo(&mut performance, performance.cb) };
    if ok == 0 {
        return Err("GetPerformanceInfo failed".to_string());
    }

    Ok(performance.ProcessCount as usize)
}

#[cfg(windows)]
fn sample_network_totals() -> Result<NetworkTotals, String> {
    let mut table_size = 0_u32;
    let sizing_result = unsafe { GetIfTable(null_mut(), &mut table_size, 0) };

    if sizing_result != ERROR_INSUFFICIENT_BUFFER && sizing_result != ERROR_SUCCESS {
        return Err(format!(
            "GetIfTable sizing failed with error code {sizing_result}"
        ));
    }

    if table_size == 0 {
        return Ok(NetworkTotals::default());
    }

    let mut buffer = vec![0_u8; table_size as usize];
    let table = buffer.as_mut_ptr() as *mut MIB_IFTABLE;

    let result = unsafe { GetIfTable(table, &mut table_size, 0) };
    if result != ERROR_SUCCESS {
        return Err(format!("GetIfTable failed with error code {result}"));
    }

    let row_count = unsafe { (*table).dwNumEntries as usize };
    let first_row = unsafe { (*table).table.as_ptr() };
    let rows = unsafe { slice::from_raw_parts(first_row, row_count) };

    Ok(rows
        .iter()
        .filter(|row| include_network_interface(row))
        .fold(NetworkTotals::default(), |totals, row| {
            add_network_totals(totals, row.dwInOctets as u64, row.dwOutOctets as u64)
        }))
}

#[cfg(windows)]
fn include_network_interface(row: &MIB_IFROW) -> bool {
    let is_up = matches!(
        row.dwOperStatus,
        IF_OPER_STATUS_OPERATIONAL | IF_OPER_STATUS_CONNECTED
    );
    let is_loopback_or_tunnel = matches!(row.dwType, IF_TYPE_SOFTWARE_LOOPBACK | IF_TYPE_TUNNEL);

    is_up && !is_loopback_or_tunnel
}

fn memory_metrics_from_status(
    total_phys: u64,
    avail_phys: u64,
    total_pagefile: u64,
    avail_pagefile: u64,
) -> MemoryMetrics {
    MemoryMetrics {
        used_bytes: total_phys.saturating_sub(avail_phys),
        total_bytes: total_phys,
        available_bytes: avail_phys,
        pagefile_used_bytes: total_pagefile.saturating_sub(avail_pagefile),
        pagefile_total_bytes: total_pagefile,
    }
}

#[cfg(windows)]
fn filetime_to_u64(filetime: FILETIME) -> u64 {
    filetime_parts_to_u64(filetime.dwLowDateTime, filetime.dwHighDateTime)
}

fn filetime_parts_to_u64(low: u32, high: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn add_network_totals(totals: NetworkTotals, received: u64, transmitted: u64) -> NetworkTotals {
    NetworkTotals {
        received_bytes: totals.received_bytes.saturating_add(received),
        transmitted_bytes: totals.transmitted_bytes.saturating_add(transmitted),
    }
}

fn percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }

    round1(((numerator as f64 / denominator as f64) * 100.0).clamp(0.0, 100.0))
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_metrics_calculate_used_and_pagefile_bytes() {
        let metrics = memory_metrics_from_status(1_000, 250, 2_000, 1_250);

        assert_eq!(
            metrics,
            MemoryMetrics {
                used_bytes: 750,
                total_bytes: 1_000,
                available_bytes: 250,
                pagefile_used_bytes: 750,
                pagefile_total_bytes: 2_000,
            }
        );
    }

    #[test]
    fn memory_metrics_saturate_when_available_exceeds_total() {
        let metrics = memory_metrics_from_status(500, 750, 600, 900);

        assert_eq!(metrics.used_bytes, 0);
        assert_eq!(metrics.pagefile_used_bytes, 0);
    }

    #[test]
    fn filetime_parts_are_combined_little_endian() {
        assert_eq!(
            filetime_parts_to_u64(0x89AB_CDEF, 0x0123_4567),
            0x0123_4567_89AB_CDEF
        );
    }

    #[test]
    fn cpu_load_uses_system_time_deltas() {
        let previous = CpuTimes {
            idle: 100,
            kernel: 300,
            user: 200,
        };
        let current = CpuTimes {
            idle: 125,
            kernel: 375,
            user: 225,
        };

        let load = calculate_cpu_load(previous, current);

        assert_eq!(load.cpu_percent, 75.0);
        assert_eq!(load.kernel_cpu_percent, 50.0);
    }

    #[test]
    fn cpu_load_returns_zero_when_delta_is_empty() {
        let times = CpuTimes {
            idle: 100,
            kernel: 300,
            user: 200,
        };

        let load = calculate_cpu_load(times, times);

        assert_eq!(load.cpu_percent, 0.0);
        assert_eq!(load.kernel_cpu_percent, 0.0);
    }

    #[test]
    fn network_totals_saturate_on_overflow() {
        let totals = add_network_totals(
            NetworkTotals {
                received_bytes: u64::MAX - 5,
                transmitted_bytes: u64::MAX - 3,
            },
            10,
            10,
        );

        assert_eq!(totals.received_bytes, u64::MAX);
        assert_eq!(totals.transmitted_bytes, u64::MAX);
    }

    #[cfg(windows)]
    #[test]
    fn sample_system_reports_plausible_memory_totals() {
        let snapshot = sample_system().expect("system collection succeeds");
        let available = snapshot
            .memory_available_bytes
            .expect("native memory reports available bytes");

        assert!(snapshot.memory_total_bytes > 0);
        assert!(snapshot.memory_used_bytes <= snapshot.memory_total_bytes);
        assert!(available <= snapshot.memory_total_bytes);
    }
}
