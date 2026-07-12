#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use crate::contracts::{
    KernelPoolKind, KernelPoolTag, MetricQuality, MetricQualityInfo, MetricSource,
    SystemMemoryAccounting, SystemMetricQuality, SystemMetricsSnapshot,
};

#[cfg(windows)]
use std::{
    mem::{align_of, size_of},
    ptr::{null_mut, read_unaligned},
    slice,
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{ERROR_SUCCESS, FILETIME, NTSTATUS},
    NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL, MIB_IF_ROW2,
        MIB_IF_TABLE2,
    },
    NetworkManagement::Ndis::NET_IF_OPER_STATUS_UP,
    System::{
        ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::GetSystemTimes,
    },
};

#[cfg(windows)]
use windows_sys::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS,
};

#[cfg(windows)]
const SYSTEM_POOL_TAG_INFORMATION: SYSTEM_INFORMATION_CLASS = 22;
#[cfg(windows)]
const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = 0xC000_0004_u32 as i32;
#[cfg(windows)]
const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005_u32 as i32;
#[cfg(windows)]
const STATUS_BUFFER_TOO_SMALL: NTSTATUS = 0xC000_0023_u32 as i32;
#[cfg(windows)]
const INITIAL_POOL_TAG_BUFFER_BYTES: usize = 128 * 1024;
#[cfg(windows)]
const MAX_POOL_TAG_BUFFER_BYTES: usize = 16 * 1024 * 1024;
#[cfg(windows)]
const MAX_KERNEL_POOL_TAGS: usize = 8;
#[cfg(windows)]
#[cfg(windows)]
pub fn sample_system() -> Result<SystemMetricsSnapshot, String> {
    let memory = sample_memory()?;
    let _cpu_times = sample_cpu_times()?;
    let network = sample_network_totals()?;
    let performance = sample_performance_metrics().ok();
    let process_count = performance
        .as_ref()
        .map(|metrics| metrics.process_count)
        .unwrap_or_default();

    Ok(SystemMetricsSnapshot {
        cpu_percent: 0.0,
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: Vec::new(),
        memory_used_bytes: memory.used_bytes,
        memory_total_bytes: memory.total_bytes,
        memory_available_bytes: Some(memory.available_bytes),
        swap_used_bytes: None,
        swap_total_bytes: None,
        process_count,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: network.received_bytes,
        network_transmitted_total_bytes: network.transmitted_bytes,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        memory_accounting: performance.map(|metrics| metrics.memory_accounting),
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
            swap: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
                    .with_message("Windows reports commit accounting, not swap usage."),
            ),
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
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PerformanceMetrics {
    process_count: usize,
    memory_accounting: SystemMemoryAccounting,
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
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return Err("GlobalMemoryStatusEx failed".to_string());
    }

    Ok(memory_metrics_from_status(
        status.ullTotalPhys,
        status.ullAvailPhys,
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
fn sample_performance_metrics() -> Result<PerformanceMetrics, String> {
    let mut performance = PERFORMANCE_INFORMATION {
        cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };

    let ok = unsafe { GetPerformanceInfo(&mut performance, performance.cb) };
    if ok == 0 {
        return Err("GetPerformanceInfo failed".to_string());
    }

    let mut metrics = performance_metrics_from_info(performance);
    metrics.memory_accounting.kernel_pool_tags = sample_kernel_pool_tags().unwrap_or_default();

    Ok(metrics)
}

#[cfg(windows)]
fn sample_network_totals() -> Result<NetworkTotals, String> {
    let mut table: *mut MIB_IF_TABLE2 = null_mut();
    let result = unsafe { GetIfTable2(&mut table) };
    if result != ERROR_SUCCESS {
        return Err(format!("GetIfTable2 failed with error code {result}"));
    }
    if table.is_null() {
        return Ok(NetworkTotals::default());
    }

    let totals = unsafe {
        let row_count = (*table).NumEntries as usize;
        let first_row = (*table).Table.as_ptr();
        network_totals_from_if_rows(slice::from_raw_parts(first_row, row_count))
    };
    unsafe {
        FreeMibTable(table.cast());
    }

    Ok(totals)
}

#[cfg(windows)]
fn network_totals_from_if_rows(rows: &[MIB_IF_ROW2]) -> NetworkTotals {
    rows.iter()
        .filter(|row| include_network_interface(row))
        .fold(NetworkTotals::default(), |totals, row| {
            add_network_totals(totals, row.InOctets, row.OutOctets)
        })
}

#[cfg(windows)]
fn include_network_interface(row: &MIB_IF_ROW2) -> bool {
    let is_up = row.OperStatus == NET_IF_OPER_STATUS_UP;
    let is_loopback_or_tunnel = matches!(row.Type, IF_TYPE_SOFTWARE_LOOPBACK | IF_TYPE_TUNNEL);

    is_up && !is_loopback_or_tunnel
}

fn memory_metrics_from_status(total_phys: u64, avail_phys: u64) -> MemoryMetrics {
    MemoryMetrics {
        used_bytes: total_phys.saturating_sub(avail_phys),
        total_bytes: total_phys,
        available_bytes: avail_phys,
    }
}

#[cfg(windows)]
fn performance_metrics_from_info(performance: PERFORMANCE_INFORMATION) -> PerformanceMetrics {
    let page_size = performance.PageSize;
    PerformanceMetrics {
        process_count: performance.ProcessCount as usize,
        memory_accounting: SystemMemoryAccounting {
            commit_used_bytes: Some(page_count_bytes(performance.CommitTotal, page_size)),
            commit_limit_bytes: Some(page_count_bytes(performance.CommitLimit, page_size)),
            system_cache_bytes: Some(page_count_bytes(performance.SystemCache, page_size)),
            kernel_total_bytes: Some(page_count_bytes(performance.KernelTotal, page_size)),
            kernel_paged_pool_bytes: Some(page_count_bytes(performance.KernelPaged, page_size)),
            kernel_nonpaged_pool_bytes: Some(page_count_bytes(
                performance.KernelNonpaged,
                page_size,
            )),
            ..SystemMemoryAccounting::default()
        },
    }
}

#[cfg(windows)]
fn page_count_bytes(pages: usize, page_size: usize) -> u64 {
    (pages as u64).saturating_mul(page_size as u64)
}

#[cfg(windows)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SystemPoolTagRaw {
    tag: [u8; 4],
    paged_allocs: u32,
    paged_frees: u32,
    paged_used: usize,
    nonpaged_allocs: u32,
    nonpaged_frees: u32,
    nonpaged_used: usize,
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct KernelPoolTagWithBytes {
    tag: KernelPoolTag,
}

#[cfg(windows)]
fn sample_kernel_pool_tags() -> Result<Vec<KernelPoolTag>, String> {
    let rows = query_kernel_pool_tag_rows()?;
    let mut tags = kernel_pool_tags_from_rows(&rows, MAX_KERNEL_POOL_TAGS);
    annotate_driver_candidates(&mut tags);

    Ok(tags.into_iter().map(|entry| entry.tag).collect())
}

#[cfg(windows)]
fn query_kernel_pool_tag_rows() -> Result<Vec<SystemPoolTagRaw>, String> {
    let mut buffer_len = INITIAL_POOL_TAG_BUFFER_BYTES;

    loop {
        let mut buffer = vec![0_u8; buffer_len];
        let mut return_len = 0_u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_POOL_TAG_INFORMATION,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut return_len,
            )
        };

        if nt_success(status) {
            return pool_tag_rows_from_buffer(&buffer);
        }

        if is_pool_tag_buffer_too_small(status) && buffer_len < MAX_POOL_TAG_BUFFER_BYTES {
            let next_len = (return_len as usize).max(buffer_len.saturating_mul(2));
            buffer_len = next_len.min(MAX_POOL_TAG_BUFFER_BYTES);
            continue;
        }

        return Err(format!(
            "NtQuerySystemInformation(SystemPoolTagInformation) failed with NTSTATUS 0x{:08X}",
            status as u32
        ));
    }
}

#[cfg(windows)]
fn nt_success(status: NTSTATUS) -> bool {
    status >= 0
}

#[cfg(windows)]
fn is_pool_tag_buffer_too_small(status: NTSTATUS) -> bool {
    matches!(
        status,
        STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_OVERFLOW | STATUS_BUFFER_TOO_SMALL
    )
}

#[cfg(windows)]
fn pool_tag_rows_from_buffer(buffer: &[u8]) -> Result<Vec<SystemPoolTagRaw>, String> {
    if buffer.len() < size_of::<u32>() {
        return Err("SystemPoolTagInformation returned a short header".to_string());
    }

    let count = unsafe { read_unaligned(buffer.as_ptr().cast::<u32>()) as usize };
    let row_offset = align_up(size_of::<u32>(), align_of::<SystemPoolTagRaw>());
    let row_size = size_of::<SystemPoolTagRaw>();
    let required_len = count
        .checked_mul(row_size)
        .and_then(|rows_len| row_offset.checked_add(rows_len))
        .ok_or_else(|| "SystemPoolTagInformation row count overflowed".to_string())?;

    if required_len > buffer.len() {
        return Err(format!(
            "SystemPoolTagInformation returned {count} rows but only {} bytes are available",
            buffer.len()
        ));
    }

    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let row_ptr = unsafe { buffer.as_ptr().add(row_offset + index * row_size) };
        rows.push(unsafe { read_unaligned(row_ptr.cast::<SystemPoolTagRaw>()) });
    }

    Ok(rows)
}

#[cfg(windows)]
fn align_up(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }

    (value + alignment - 1) & !(alignment - 1)
}

#[cfg(windows)]
fn kernel_pool_tags_from_rows(
    rows: &[SystemPoolTagRaw],
    limit: usize,
) -> Vec<KernelPoolTagWithBytes> {
    let mut tags = Vec::new();
    for row in rows {
        if row.paged_used > 0 {
            tags.push(KernelPoolTagWithBytes {
                tag: KernelPoolTag {
                    tag: pool_tag_display(row.tag),
                    kind: KernelPoolKind::Paged,
                    bytes: row.paged_used as u64,
                    allocations: row.paged_allocs as u64,
                    frees: row.paged_frees as u64,
                    driver_candidates: Vec::new(),
                    driver_candidates_pending: false,
                },
            });
        }

        if row.nonpaged_used > 0 {
            tags.push(KernelPoolTagWithBytes {
                tag: KernelPoolTag {
                    tag: pool_tag_display(row.tag),
                    kind: KernelPoolKind::Nonpaged,
                    bytes: row.nonpaged_used as u64,
                    allocations: row.nonpaged_allocs as u64,
                    frees: row.nonpaged_frees as u64,
                    driver_candidates: Vec::new(),
                    driver_candidates_pending: false,
                },
            });
        }
    }

    tags.sort_by(|left, right| {
        right
            .tag
            .bytes
            .cmp(&left.tag.bytes)
            .then_with(|| left.tag.tag.cmp(&right.tag.tag))
            .then_with(|| left.tag.kind.cmp(&right.tag.kind))
    });
    tags.truncate(limit);
    tags
}

#[cfg(windows)]
fn pool_tag_display(tag: [u8; 4]) -> String {
    tag.iter()
        .map(|byte| {
            if byte.is_ascii_graphic() {
                *byte as char
            } else if *byte == b' ' {
                '_'
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(windows)]
fn annotate_driver_candidates(tags: &mut [KernelPoolTagWithBytes]) {
    for tag in tags {
        tag.tag.driver_candidates.clear();
        tag.tag.driver_candidates_pending = false;
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
    fn memory_metrics_calculate_used_bytes() {
        let metrics = memory_metrics_from_status(1_000, 250);

        assert_eq!(
            metrics,
            MemoryMetrics {
                used_bytes: 750,
                total_bytes: 1_000,
                available_bytes: 250,
            }
        );
    }

    #[test]
    fn memory_metrics_saturate_when_available_exceeds_total() {
        let metrics = memory_metrics_from_status(500, 750);

        assert_eq!(metrics.used_bytes, 0);
    }

    #[cfg(windows)]
    #[test]
    fn performance_metrics_include_commit_and_kernel_pool_bytes() {
        let metrics = performance_metrics_from_info(PERFORMANCE_INFORMATION {
            cb: 0,
            CommitTotal: 2,
            CommitLimit: 10,
            CommitPeak: 0,
            PhysicalTotal: 0,
            PhysicalAvailable: 0,
            SystemCache: 3,
            KernelTotal: 7,
            KernelPaged: 5,
            KernelNonpaged: 6,
            PageSize: 4096,
            HandleCount: 0,
            ProcessCount: 42,
            ThreadCount: 0,
        });

        assert_eq!(metrics.process_count, 42);
        assert_eq!(metrics.memory_accounting.commit_used_bytes, Some(8192));
        assert_eq!(metrics.memory_accounting.commit_limit_bytes, Some(40960));
        assert_eq!(metrics.memory_accounting.system_cache_bytes, Some(12288));
        assert_eq!(metrics.memory_accounting.kernel_total_bytes, Some(28672));
        assert_eq!(
            metrics.memory_accounting.kernel_paged_pool_bytes,
            Some(20480)
        );
        assert_eq!(
            metrics.memory_accounting.kernel_nonpaged_pool_bytes,
            Some(24576)
        );
    }

    #[cfg(windows)]
    #[test]
    fn pool_tag_rows_parse_x64_aligned_buffer() {
        let row = SystemPoolTagRaw {
            tag: *b"Leak",
            paged_allocs: 10,
            paged_frees: 4,
            paged_used: 1_024,
            nonpaged_allocs: 20,
            nonpaged_frees: 5,
            nonpaged_used: 4_096,
        };
        let row_offset = align_up(size_of::<u32>(), align_of::<SystemPoolTagRaw>());
        let mut buffer = vec![0_u8; row_offset + size_of::<SystemPoolTagRaw>()];
        buffer[..size_of::<u32>()].copy_from_slice(&1_u32.to_ne_bytes());
        let row_bytes = unsafe {
            slice::from_raw_parts(
                (&row as *const SystemPoolTagRaw).cast::<u8>(),
                size_of::<SystemPoolTagRaw>(),
            )
        };
        buffer[row_offset..row_offset + row_bytes.len()].copy_from_slice(row_bytes);

        let rows = pool_tag_rows_from_buffer(&buffer).expect("rows parse");

        assert_eq!(rows, vec![row]);
    }

    #[cfg(windows)]
    #[test]
    fn kernel_pool_tags_are_sorted_and_limited_by_bytes() {
        let rows = vec![
            SystemPoolTagRaw {
                tag: *b"Page",
                paged_allocs: 2,
                paged_frees: 1,
                paged_used: 512,
                nonpaged_allocs: 0,
                nonpaged_frees: 0,
                nonpaged_used: 0,
            },
            SystemPoolTagRaw {
                tag: *b"Leak",
                paged_allocs: 10,
                paged_frees: 2,
                paged_used: 1_024,
                nonpaged_allocs: 30,
                nonpaged_frees: 4,
                nonpaged_used: 4_096,
            },
        ];

        let tags = kernel_pool_tags_from_rows(&rows, 2);

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].tag.tag, "Leak");
        assert_eq!(tags[0].tag.kind, KernelPoolKind::Nonpaged);
        assert_eq!(tags[0].tag.bytes, 4_096);
        assert_eq!(tags[1].tag.tag, "Leak");
        assert_eq!(tags[1].tag.kind, KernelPoolKind::Paged);
        assert_eq!(tags[1].tag.bytes, 1_024);
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
    fn network_totals_use_64_bit_interface_octets() {
        let row = MIB_IF_ROW2 {
            Type: 6,
            OperStatus: NET_IF_OPER_STATUS_UP,
            InOctets: u32::MAX as u64 + 42,
            OutOctets: u32::MAX as u64 + 84,
            ..MIB_IF_ROW2::default()
        };

        let totals = network_totals_from_if_rows(&[row]);

        assert_eq!(totals.received_bytes, u32::MAX as u64 + 42);
        assert_eq!(totals.transmitted_bytes, u32::MAX as u64 + 84);
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
