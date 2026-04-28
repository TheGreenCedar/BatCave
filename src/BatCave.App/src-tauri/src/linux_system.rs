#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{fs, path::Path, time::Instant};

use crate::contracts::{
    MetricQuality, MetricQualityInfo, MetricSource, SystemMetricQuality, SystemMetricsSnapshot,
};

const SECTOR_SIZE_BYTES: u64 = 512;

#[derive(Debug, Default)]
pub struct LinuxSystemCollector {
    previous: Option<LinuxSystemCounters>,
}

impl LinuxSystemCollector {
    pub fn new() -> Self {
        Self { previous: None }
    }

    pub fn sample(&mut self) -> Result<SystemMetricsSnapshot, String> {
        let cpu_times = read_cpu_times()?;
        let memory = read_meminfo()?;
        let (disk, disk_quality) = match read_block_device_totals() {
            Ok(disk) => (
                disk,
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs),
            ),
            Err(error) => (
                IoTotals::default(),
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Procfs)
                    .with_message(&error),
            ),
        };
        let (network, network_quality) = match read_network_totals() {
            Ok(network) => (
                network,
                MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs),
            ),
            Err(error) => (
                IoTotals::default(),
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Procfs)
                    .with_message(&error),
            ),
        };
        let current = LinuxSystemCounters {
            cpu_times,
            disk,
            network,
            sampled_at: Instant::now(),
        };
        let previous = self.previous.as_ref();

        let aggregate_cpu = current.cpu_times.first().copied().unwrap_or_default();
        let (cpu_percent, kernel_cpu_percent, cpu_quality) = previous
            .and_then(|previous| previous.cpu_times.first().copied())
            .map_or(
                (
                    0.0,
                    0.0,
                    MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                        .with_message("Linux CPU counters need a second /proc/stat sample."),
                ),
                |previous_cpu| {
                    let (cpu, kernel) = cpu_load(previous_cpu, aggregate_cpu);
                    (
                        cpu,
                        kernel,
                        MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs),
                    )
                },
            );

        let logical_cpu_percent = previous.map_or_else(Vec::new, |previous| {
            current
                .cpu_times
                .iter()
                .skip(1)
                .zip(previous.cpu_times.iter().skip(1))
                .map(|(current, previous)| cpu_load(*previous, *current).0)
                .collect()
        });

        let elapsed_seconds = previous
            .map(|previous| {
                current
                    .sampled_at
                    .duration_since(previous.sampled_at)
                    .as_secs_f64()
            })
            .unwrap_or(1.0);

        let (disk_read_bps, disk_write_bps) = previous.map_or((0, 0), |previous| {
            (
                byte_rate(
                    current.disk.read_total_bytes,
                    previous.disk.read_total_bytes,
                    elapsed_seconds,
                ),
                byte_rate(
                    current.disk.write_total_bytes,
                    previous.disk.write_total_bytes,
                    elapsed_seconds,
                ),
            )
        });
        let (network_received_bps, network_transmitted_bps) = previous.map_or((0, 0), |previous| {
            (
                byte_rate(
                    current.network.read_total_bytes,
                    previous.network.read_total_bytes,
                    elapsed_seconds,
                ),
                byte_rate(
                    current.network.write_total_bytes,
                    previous.network.write_total_bytes,
                    elapsed_seconds,
                ),
            )
        });

        let snapshot = SystemMetricsSnapshot {
            cpu_percent,
            kernel_cpu_percent,
            logical_cpu_percent,
            memory_used_bytes: memory.used_bytes,
            memory_total_bytes: memory.total_bytes,
            memory_available_bytes: Some(memory.available_bytes),
            swap_used_bytes: memory.swap_used_bytes,
            swap_total_bytes: memory.swap_total_bytes,
            process_count: count_process_dirs(),
            disk_read_total_bytes: current.disk.read_total_bytes,
            disk_write_total_bytes: current.disk.write_total_bytes,
            disk_read_bps,
            disk_write_bps,
            network_received_total_bytes: current.network.read_total_bytes,
            network_transmitted_total_bytes: current.network.write_total_bytes,
            network_received_bps,
            network_transmitted_bps,
            quality: Some(SystemMetricQuality {
                cpu: Some(cpu_quality.clone()),
                kernel_cpu: Some(cpu_quality.clone()),
                logical_cpu: Some(cpu_quality),
                memory: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::Procfs,
                )),
                swap: Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::Procfs,
                )),
                disk: Some(disk_quality),
                network: Some(network_quality),
            }),
        };

        self.previous = Some(current);
        Ok(snapshot)
    }
}

#[derive(Debug, Clone)]
struct LinuxSystemCounters {
    cpu_times: Vec<CpuTimes>,
    disk: IoTotals,
    network: IoTotals,
    sampled_at: Instant,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CpuTimes {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

impl CpuTimes {
    fn total(self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
            .saturating_add(self.steal)
    }

    fn idle_total(self) -> u64 {
        self.idle.saturating_add(self.iowait)
    }

    fn kernel_total(self) -> u64 {
        self.system
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MemoryTotals {
    total_bytes: u64,
    available_bytes: u64,
    used_bytes: u64,
    swap_total_bytes: u64,
    swap_used_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct IoTotals {
    read_total_bytes: u64,
    write_total_bytes: u64,
}

fn read_cpu_times() -> Result<Vec<CpuTimes>, String> {
    parse_cpu_times(
        &fs::read_to_string("/proc/stat")
            .map_err(|error| format!("linux_proc_stat_read_failed:{error}"))?,
    )
}

fn read_meminfo() -> Result<MemoryTotals, String> {
    parse_meminfo(
        &fs::read_to_string("/proc/meminfo")
            .map_err(|error| format!("linux_proc_meminfo_read_failed:{error}"))?,
    )
}

fn read_network_totals() -> Result<IoTotals, String> {
    parse_network_totals(
        &fs::read_to_string("/proc/net/dev")
            .map_err(|error| format!("linux_proc_net_dev_read_failed:{error}"))?,
    )
}

fn read_block_device_totals() -> Result<IoTotals, String> {
    let mut totals = IoTotals::default();
    for entry in fs::read_dir("/sys/block")
        .map_err(|error| format!("linux_sys_block_read_failed:{error}"))?
    {
        let entry = entry.map_err(|error| format!("linux_sys_block_entry_failed:{error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_block_device(&name) {
            continue;
        }
        let stat_path = entry.path().join("stat");
        let stat = fs::read_to_string(&stat_path)
            .map_err(|error| format!("linux_sys_block_stat_read_failed:{stat_path:?}:{error}"))?;
        let device_totals = parse_block_stat(&stat)?;
        totals.read_total_bytes = totals
            .read_total_bytes
            .saturating_add(device_totals.read_total_bytes);
        totals.write_total_bytes = totals
            .write_total_bytes
            .saturating_add(device_totals.write_total_bytes);
    }
    Ok(totals)
}

fn parse_cpu_times(content: &str) -> Result<Vec<CpuTimes>, String> {
    let mut cpus = Vec::new();
    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let Some(label) = fields.next() else {
            continue;
        };
        if label != "cpu"
            && !label
                .strip_prefix("cpu")
                .is_some_and(|suffix| suffix.chars().all(|value| value.is_ascii_digit()))
        {
            continue;
        }
        let values = fields
            .filter_map(|value| value.parse::<u64>().ok())
            .collect::<Vec<_>>();
        if values.len() < 4 {
            continue;
        }
        cpus.push(CpuTimes {
            user: values.first().copied().unwrap_or_default(),
            nice: values.get(1).copied().unwrap_or_default(),
            system: values.get(2).copied().unwrap_or_default(),
            idle: values.get(3).copied().unwrap_or_default(),
            iowait: values.get(4).copied().unwrap_or_default(),
            irq: values.get(5).copied().unwrap_or_default(),
            softirq: values.get(6).copied().unwrap_or_default(),
            steal: values.get(7).copied().unwrap_or_default(),
        });
    }

    if cpus.is_empty() {
        Err("linux_proc_stat_missing_cpu_lines".to_string())
    } else {
        Ok(cpus)
    }
}

fn parse_meminfo(content: &str) -> Result<MemoryTotals, String> {
    let value_kib = |key: &str| -> Option<u64> {
        content.lines().find_map(|line| {
            let (name, rest) = line.split_once(':')?;
            (name == key).then(|| {
                rest.split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
            })?
        })
    };

    let total_bytes = value_kib("MemTotal")
        .ok_or_else(|| "linux_proc_meminfo_missing_mem_total".to_string())?
        .saturating_mul(1024);
    let available_bytes = value_kib("MemAvailable")
        .or_else(|| value_kib("MemFree"))
        .unwrap_or_default()
        .saturating_mul(1024);
    let swap_total_bytes = value_kib("SwapTotal")
        .unwrap_or_default()
        .saturating_mul(1024);
    let swap_free_bytes = value_kib("SwapFree")
        .unwrap_or_default()
        .saturating_mul(1024);

    Ok(MemoryTotals {
        total_bytes,
        available_bytes,
        used_bytes: total_bytes.saturating_sub(available_bytes),
        swap_total_bytes,
        swap_used_bytes: swap_total_bytes.saturating_sub(swap_free_bytes),
    })
}

fn parse_network_totals(content: &str) -> Result<IoTotals, String> {
    let mut totals = IoTotals::default();
    for line in content.lines().skip(2) {
        let Some((name, values)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == "lo" {
            continue;
        }
        let fields = values.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 16 {
            continue;
        }
        totals.read_total_bytes = totals
            .read_total_bytes
            .saturating_add(fields[0].parse::<u64>().unwrap_or_default());
        totals.write_total_bytes = totals
            .write_total_bytes
            .saturating_add(fields[8].parse::<u64>().unwrap_or_default());
    }
    Ok(totals)
}

fn parse_block_stat(content: &str) -> Result<IoTotals, String> {
    let fields = content.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 7 {
        return Err("linux_sys_block_stat_too_short".to_string());
    }
    let sectors_read = fields[2].parse::<u64>().unwrap_or_default();
    let sectors_written = fields[6].parse::<u64>().unwrap_or_default();
    Ok(IoTotals {
        read_total_bytes: sectors_read.saturating_mul(SECTOR_SIZE_BYTES),
        write_total_bytes: sectors_written.saturating_mul(SECTOR_SIZE_BYTES),
    })
}

fn should_skip_block_device(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("fd")
        || name.starts_with("sr")
}

fn count_process_dirs() -> usize {
    fs::read_dir(Path::new("/proc"))
        .ok()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().parse::<u32>().is_ok())
                .count()
        })
        .unwrap_or_default()
}

fn cpu_load(previous: CpuTimes, current: CpuTimes) -> (f64, f64) {
    let total_delta = current.total().saturating_sub(previous.total());
    if total_delta == 0 {
        return (0.0, 0.0);
    }
    let idle_delta = current.idle_total().saturating_sub(previous.idle_total());
    let kernel_delta = current
        .kernel_total()
        .saturating_sub(previous.kernel_total());
    (
        round1(percent(total_delta.saturating_sub(idle_delta), total_delta)),
        round1(percent(kernel_delta, total_delta)),
    )
}

fn byte_rate(current: u64, previous: u64, elapsed_seconds: f64) -> u64 {
    if current < previous {
        return 0;
    }
    ((current - previous) as f64 / elapsed_seconds.max(0.001)).round() as u64
}

fn percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        (numerator as f64 / denominator as f64) * 100.0
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_times_reads_aggregate_and_logical_cpus() {
        let cpus = parse_cpu_times(
            "cpu  100 2 30 400 5 6 7 8 0 0\ncpu0 50 1 10 200 2 3 4 0 0 0\nbtime 1\n",
        )
        .unwrap();

        assert_eq!(cpus.len(), 2);
        assert_eq!(cpus[0].system, 30);
        assert_eq!(cpus[1].idle, 200);
    }

    #[test]
    fn cpu_load_reports_busy_and_kernel_percent() {
        let previous = CpuTimes {
            user: 100,
            system: 20,
            idle: 80,
            ..CpuTimes::default()
        };
        let current = CpuTimes {
            user: 150,
            system: 40,
            idle: 110,
            ..CpuTimes::default()
        };

        let (cpu, kernel) = cpu_load(previous, current);

        assert_eq!(cpu, 70.0);
        assert_eq!(kernel, 20.0);
    }

    #[test]
    fn parse_meminfo_uses_available_memory_and_swap_free() {
        let memory = parse_meminfo(
            "MemTotal:       1000 kB\nMemFree:         200 kB\nMemAvailable:    750 kB\nSwapTotal:       500 kB\nSwapFree:        125 kB\n",
        )
        .unwrap();

        assert_eq!(memory.total_bytes, 1000 * 1024);
        assert_eq!(memory.used_bytes, 250 * 1024);
        assert_eq!(memory.swap_used_bytes, 375 * 1024);
    }

    #[test]
    fn parse_network_totals_skips_loopback() {
        let totals = parse_network_totals(
            "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\neth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        assert_eq!(totals.read_total_bytes, 100);
        assert_eq!(totals.write_total_bytes, 200);
    }

    #[test]
    fn parse_block_stat_converts_sectors_to_bytes() {
        let totals = parse_block_stat("1 0 8 0 2 0 16 0 0 0 0").unwrap();

        assert_eq!(totals.read_total_bytes, 4096);
        assert_eq!(totals.write_total_bytes, 8192);
    }
}
