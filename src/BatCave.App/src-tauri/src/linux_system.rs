#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{collections::BTreeMap, fs, path::Path, time::Instant};

use crate::contracts::{
    MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource, SystemMetricQuality,
    SystemMetricsSnapshot,
};

const SECTOR_SIZE_BYTES: u64 = 512;
const REQUIRED_CPU_COUNTERS: usize = 8;

#[derive(Debug, Default)]
pub struct LinuxSystemCollector {
    previous_cpu: Option<BTreeMap<String, CpuTimes>>,
    previous_disk: Option<TimedIoCounters>,
    previous_network: Option<TimedIoCounters>,
}

impl LinuxSystemCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(&mut self) -> Result<SystemMetricsSnapshot, String> {
        let cpu_times = read_cpu_times()?;
        let memory = read_meminfo()?;
        let sampled_at = Instant::now();
        let (disk, disk_read_bps, disk_write_bps, disk_quality) = resolve_io_sample(
            read_block_device_totals(),
            &mut self.previous_disk,
            sampled_at,
            "Linux disk counters need a second /sys/block sample.",
        );
        let (network, network_received_bps, network_transmitted_bps, network_quality) =
            resolve_io_sample(
                read_network_totals(),
                &mut self.previous_network,
                sampled_at,
                "Linux network counters need a second /proc/net/dev sample.",
            );

        let aggregate_cpu = cpu_times
            .get("cpu")
            .copied()
            .ok_or_else(|| "linux_proc_stat_missing_aggregate_cpu".to_string())?;
        let (cpu_percent, kernel_cpu_percent, cpu_quality) = self
            .previous_cpu
            .as_ref()
            .and_then(|previous| previous.get("cpu").copied())
            .map_or(
                (
                    0.0,
                    0.0,
                    MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                        .with_limitation(
                            MetricLimitationCode::PendingBaseline,
                            "Linux CPU counters need a second /proc/stat sample.",
                        ),
                ),
                |previous_cpu| match cpu_load(previous_cpu, aggregate_cpu) {
                    Some((cpu, kernel)) => (
                        cpu,
                        kernel,
                        MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs),
                    ),
                    None => (
                        0.0,
                        0.0,
                        MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                            .with_limitation(
                                MetricLimitationCode::PendingBaseline,
                                "Linux aggregate CPU counters reset; waiting for a fresh baseline.",
                            ),
                    ),
                },
            );

        let mut logical_warming = 0_usize;
        let mut logical_reset = 0_usize;
        let mut logical_cpu_times = cpu_times
            .iter()
            .filter(|(label, _)| label.as_str() != "cpu")
            .collect::<Vec<_>>();
        logical_cpu_times.sort_by_key(|(label, _)| {
            label
                .strip_prefix("cpu")
                .and_then(|index| index.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        let logical_cpu_percent = logical_cpu_times
            .into_iter()
            .map(|(label, current)| {
                let Some(previous) = self
                    .previous_cpu
                    .as_ref()
                    .and_then(|previous| previous.get(label))
                else {
                    logical_warming += 1;
                    return 0.0;
                };
                match cpu_load(*previous, *current) {
                    Some(load) => load.0,
                    None => {
                        logical_reset += 1;
                        0.0
                    }
                }
            })
            .collect();
        let logical_cpu_quality = if self.previous_cpu.is_none() {
            MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs).with_limitation(
                MetricLimitationCode::PendingBaseline,
                "Linux logical CPU counters need a second keyed /proc/stat sample.",
            )
        } else if logical_warming > 0 || logical_reset > 0 {
            MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Procfs).with_limitation(
                MetricLimitationCode::PendingBaseline,
                "New or reset logical CPUs are warming up; stable CPU identities remain native.",
            )
        } else {
            MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs)
        };
        self.previous_cpu = Some(cpu_times);

        let snapshot = SystemMetricsSnapshot {
            cpu_percent,
            kernel_cpu_percent,
            logical_cpu_percent,
            memory_used_bytes: memory.used_bytes,
            memory_total_bytes: memory.total_bytes,
            memory_available_bytes: Some(memory.available_bytes),
            swap_used_bytes: Some(memory.swap_used_bytes),
            swap_total_bytes: Some(memory.swap_total_bytes),
            process_count: count_process_dirs(),
            disk_read_total_bytes: disk.read_total_bytes,
            disk_write_total_bytes: disk.write_total_bytes,
            disk_read_bps,
            disk_write_bps,
            network_received_total_bytes: network.read_total_bytes,
            network_transmitted_total_bytes: network.write_total_bytes,
            network_received_bps,
            network_transmitted_bps,
            memory_accounting: None,
            quality: Some(SystemMetricQuality {
                cpu: Some(cpu_quality.clone()),
                kernel_cpu: Some(cpu_quality.clone()),
                logical_cpu: Some(logical_cpu_quality),
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

        Ok(snapshot)
    }
}

#[derive(Debug, Clone)]
struct TimedIoCounters {
    counters: BTreeMap<String, IoTotals>,
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

fn resolve_io_sample(
    current: Result<BTreeMap<String, IoTotals>, String>,
    previous: &mut Option<TimedIoCounters>,
    sampled_at: Instant,
    initial_message: &str,
) -> (IoTotals, u64, u64, MetricQualityInfo) {
    match current {
        Ok(counters) => {
            let totals = sum_io_totals(counters.values().copied());
            let (read_bps, write_bps, quality) = previous.as_ref().map_or_else(
                || {
                    (
                        0,
                        0,
                        MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                            .with_limitation(
                                MetricLimitationCode::PendingBaseline,
                                initial_message,
                            ),
                    )
                },
                |previous| {
                    let elapsed = sampled_at.duration_since(previous.sampled_at).as_secs_f64();
                    keyed_io_rates(&counters, &previous.counters, elapsed)
                },
            );
            *previous = Some(TimedIoCounters {
                counters,
                sampled_at,
            });
            (totals, read_bps, write_bps, quality)
        }
        Err(error) => previous.as_ref().map_or_else(
            || {
                (
                    IoTotals::default(),
                    0,
                    0,
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Procfs)
                        .with_limitation(MetricLimitationCode::CollectorFailure, &error),
                )
            },
            |previous| {
                (
                    sum_io_totals(previous.counters.values().copied()),
                    0,
                    0,
                    MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                        .with_limitation(MetricLimitationCode::CollectorFailure, &error),
                )
            },
        ),
    }
}

fn keyed_io_rates(
    current: &BTreeMap<String, IoTotals>,
    previous: &BTreeMap<String, IoTotals>,
    elapsed_seconds: f64,
) -> (u64, u64, MetricQualityInfo) {
    let mut read_bps = 0_u64;
    let mut write_bps = 0_u64;
    let mut matched = 0_usize;
    let mut warming = 0_usize;
    let mut reset = 0_usize;

    for (identity, current) in current {
        let Some(previous) = previous.get(identity) else {
            warming += 1;
            continue;
        };
        if current.read_total_bytes < previous.read_total_bytes
            || current.write_total_bytes < previous.write_total_bytes
        {
            reset += 1;
            continue;
        }
        matched += 1;
        read_bps = read_bps.saturating_add(byte_rate(
            current.read_total_bytes,
            previous.read_total_bytes,
            elapsed_seconds,
        ));
        write_bps = write_bps.saturating_add(byte_rate(
            current.write_total_bytes,
            previous.write_total_bytes,
            elapsed_seconds,
        ));
    }

    let quality = if matched == 0 && (!current.is_empty() || !previous.is_empty()) {
        MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs).with_limitation(
            MetricLimitationCode::PendingBaseline,
            "Linux counter identities changed or reset; waiting for stable per-device baselines.",
        )
    } else if warming > 0 || reset > 0 {
        MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Procfs).with_limitation(
            MetricLimitationCode::PendingBaseline,
            "New or reset Linux counter identities are warming up; stable identities remain native.",
        )
    } else {
        MetricQualityInfo::new(MetricQuality::Native, MetricSource::Procfs)
    };
    (read_bps, write_bps, quality)
}

fn sum_io_totals(counters: impl IntoIterator<Item = IoTotals>) -> IoTotals {
    counters
        .into_iter()
        .fold(IoTotals::default(), |totals, counter| IoTotals {
            read_total_bytes: totals
                .read_total_bytes
                .saturating_add(counter.read_total_bytes),
            write_total_bytes: totals
                .write_total_bytes
                .saturating_add(counter.write_total_bytes),
        })
}

fn read_cpu_times() -> Result<BTreeMap<String, CpuTimes>, String> {
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

fn read_network_totals() -> Result<BTreeMap<String, IoTotals>, String> {
    parse_network_totals(
        &fs::read_to_string("/proc/net/dev")
            .map_err(|error| format!("linux_proc_net_dev_read_failed:{error}"))?,
    )
}

fn read_block_device_totals() -> Result<BTreeMap<String, IoTotals>, String> {
    read_block_device_totals_from(Path::new("/sys/block"))
}

fn read_block_device_totals_from(block_root: &Path) -> Result<BTreeMap<String, IoTotals>, String> {
    let mut counters = BTreeMap::new();
    for entry in
        fs::read_dir(block_root).map_err(|error| format!("linux_sys_block_read_failed:{error}"))?
    {
        let entry = entry.map_err(|error| format!("linux_sys_block_entry_failed:{error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_block_device(&name) || block_device_has_holders(&entry.path())? {
            continue;
        }
        let stat_path = entry.path().join("stat");
        let stat = fs::read_to_string(&stat_path)
            .map_err(|error| format!("linux_sys_block_stat_read_failed:{stat_path:?}:{error}"))?;
        let device_totals =
            parse_block_stat(&stat).map_err(|error| format!("{error}:device={name}"))?;
        counters.insert(name, device_totals);
    }
    Ok(counters)
}

fn parse_cpu_times(content: &str) -> Result<BTreeMap<String, CpuTimes>, String> {
    let mut cpus = BTreeMap::new();
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
            .enumerate()
            .map(|(index, value)| {
                value.parse::<u64>().map_err(|error| {
                    format!("linux_proc_stat_cpu_parse_failed:{label}:field={index}:{error}")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() < REQUIRED_CPU_COUNTERS {
            return Err(format!(
                "linux_proc_stat_cpu_too_short:{label}:expected={REQUIRED_CPU_COUNTERS}:actual={}",
                values.len()
            ));
        }
        cpus.insert(
            label.to_string(),
            CpuTimes {
                user: values[0],
                nice: values[1],
                system: values[2],
                idle: values[3],
                iowait: values[4],
                irq: values[5],
                softirq: values[6],
                steal: values[7],
            },
        );
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

fn parse_network_totals(content: &str) -> Result<BTreeMap<String, IoTotals>, String> {
    let mut counters = BTreeMap::new();
    for (line_index, line) in content.lines().skip(2).enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, values) = line.split_once(':').ok_or_else(|| {
            format!(
                "linux_proc_net_dev_missing_separator:line={}",
                line_index + 3
            )
        })?;
        let name = name.trim();
        if name == "lo" {
            continue;
        }
        let fields = values.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 16 {
            return Err(format!("linux_proc_net_dev_too_short:interface={name}"));
        }
        let received = fields[0].parse::<u64>().map_err(|error| {
            format!("linux_proc_net_dev_rx_parse_failed:interface={name}:{error}")
        })?;
        let transmitted = fields[8].parse::<u64>().map_err(|error| {
            format!("linux_proc_net_dev_tx_parse_failed:interface={name}:{error}")
        })?;
        counters.insert(
            name.to_string(),
            IoTotals {
                read_total_bytes: received,
                write_total_bytes: transmitted,
            },
        );
    }
    Ok(counters)
}

fn parse_block_stat(content: &str) -> Result<IoTotals, String> {
    let fields = content.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 7 {
        return Err("linux_sys_block_stat_too_short".to_string());
    }
    let sectors_read = fields[2]
        .parse::<u64>()
        .map_err(|error| format!("linux_sys_block_stat_read_parse_failed:{error}"))?;
    let sectors_written = fields[6]
        .parse::<u64>()
        .map_err(|error| format!("linux_sys_block_stat_write_parse_failed:{error}"))?;
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
        || name.starts_with("zram")
}

fn block_device_has_holders(device: &Path) -> Result<bool, String> {
    if directory_has_entries(&device.join("holders"))? {
        return Ok(true);
    }
    for entry in fs::read_dir(device)
        .map_err(|error| format!("linux_sys_block_device_read_failed:{device:?}:{error}"))?
    {
        let entry =
            entry.map_err(|error| format!("linux_sys_block_device_entry_failed:{error}"))?;
        let path = entry.path();
        if path.join("partition").exists() && directory_has_entries(&path.join("holders"))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_has_entries(path: &Path) -> Result<bool, String> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "linux_sys_block_holders_read_failed:{path:?}:{error}"
        )),
    }
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

fn cpu_load(previous: CpuTimes, current: CpuTimes) -> Option<(f64, f64)> {
    if current.user < previous.user
        || current.nice < previous.nice
        || current.system < previous.system
        || current.idle < previous.idle
        || current.iowait < previous.iowait
        || current.irq < previous.irq
        || current.softirq < previous.softirq
        || current.steal < previous.steal
    {
        return None;
    }
    let total_delta = current.total() - previous.total();
    if total_delta == 0 {
        return Some((0.0, 0.0));
    }
    let idle_delta = current.idle_total().saturating_sub(previous.idle_total());
    let kernel_delta = current
        .kernel_total()
        .saturating_sub(previous.kernel_total());
    Some((
        round1(percent(total_delta.saturating_sub(idle_delta), total_delta)),
        round1(percent(kernel_delta, total_delta)),
    ))
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
        assert_eq!(cpus["cpu"].system, 30);
        assert_eq!(cpus["cpu0"].idle, 200);
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

        let (cpu, kernel) = cpu_load(previous, current).expect("monotonic CPU counters");

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
        let counters = parse_network_totals(
            "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\neth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        assert_eq!(counters["eth0"].read_total_bytes, 100);
        assert_eq!(counters["eth0"].write_total_bytes, 200);
    }

    #[test]
    fn parse_block_stat_converts_sectors_to_bytes() {
        let totals = parse_block_stat("1 0 8 0 2 0 16 0 0 0 0").unwrap();

        assert_eq!(totals.read_total_bytes, 4096);
        assert_eq!(totals.write_total_bytes, 8192);
    }

    #[test]
    fn failed_io_sample_keeps_last_baseline_for_recovery() {
        let started = Instant::now();
        let mut previous = None;
        let _ = resolve_io_sample(
            Ok(BTreeMap::from([(
                "device".to_string(),
                IoTotals {
                    read_total_bytes: 100,
                    write_total_bytes: 200,
                },
            )])),
            &mut previous,
            started,
            "initial",
        );

        let (held, read_bps, write_bps, quality) = resolve_io_sample(
            Err("read failed".to_string()),
            &mut previous,
            started + std::time::Duration::from_secs(5),
            "initial",
        );
        assert_eq!(held.read_total_bytes, 100);
        assert_eq!((read_bps, write_bps), (0, 0));
        assert_eq!(quality.quality, MetricQuality::Held);

        let (_, read_bps, write_bps, quality) = resolve_io_sample(
            Ok(BTreeMap::from([(
                "device".to_string(),
                IoTotals {
                    read_total_bytes: 1_100,
                    write_total_bytes: 2_200,
                },
            )])),
            &mut previous,
            started + std::time::Duration::from_secs(10),
            "initial",
        );
        assert_eq!((read_bps, write_bps), (100, 200));
        assert_eq!(quality.quality, MetricQuality::Native);
    }

    #[test]
    fn block_totals_count_only_top_of_stack_devices() {
        let root = std::env::temp_dir().join(format!(
            "batcave-linux-block-topology-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        create_block_device(&root, "sda", "1 0 100 0 2 0 200 0");
        fs::create_dir_all(root.join("sda/sda1/holders/dm-0")).unwrap();
        fs::write(root.join("sda/sda1/partition"), "1").unwrap();
        create_block_device(&root, "dm-0", "1 0 8 0 2 0 16 0");
        create_block_device(&root, "nvme0n1", "1 0 2 0 2 0 4 0");
        create_block_device(&root, "zram0", "1 0 999 0 2 0 999 0");

        let counters = read_block_device_totals_from(&root).expect("topology reads");
        let totals = sum_io_totals(counters.values().copied());

        assert_eq!(totals.read_total_bytes, 5_120);
        assert_eq!(totals.write_total_bytes, 10_240);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn logical_cpu_deltas_follow_labels_across_reordering() {
        let previous = parse_cpu_times(
            "cpu 20 0 0 180 0 0 0 0\ncpu0 10 0 0 90 0 0 0 0\ncpu1 10 0 0 90 0 0 0 0\n",
        )
        .unwrap();
        let current = parse_cpu_times(
            "cpu 60 0 0 340 0 0 0 0\ncpu1 40 0 0 160 0 0 0 0\ncpu0 20 0 0 180 0 0 0 0\n",
        )
        .unwrap();

        let cpu0 = cpu_load(previous["cpu0"], current["cpu0"]).unwrap().0;
        let cpu1 = cpu_load(previous["cpu1"], current["cpu1"]).unwrap().0;

        assert_eq!(cpu0, 10.0);
        assert_eq!(cpu1, 30.0);
    }

    #[test]
    fn keyed_io_rates_ignore_new_removed_and_reset_identities() {
        let previous = BTreeMap::from([
            (
                "gone".to_string(),
                IoTotals {
                    read_total_bytes: 9_000,
                    write_total_bytes: 9_000,
                },
            ),
            (
                "reset".to_string(),
                IoTotals {
                    read_total_bytes: 8_000,
                    write_total_bytes: 8_000,
                },
            ),
            (
                "stable".to_string(),
                IoTotals {
                    read_total_bytes: 1_000,
                    write_total_bytes: 2_000,
                },
            ),
        ]);
        let current = BTreeMap::from([
            (
                "new".to_string(),
                IoTotals {
                    read_total_bytes: 50_000,
                    write_total_bytes: 60_000,
                },
            ),
            (
                "reset".to_string(),
                IoTotals {
                    read_total_bytes: 10,
                    write_total_bytes: 20,
                },
            ),
            (
                "stable".to_string(),
                IoTotals {
                    read_total_bytes: 1_500,
                    write_total_bytes: 2_250,
                },
            ),
        ]);

        let (read_bps, write_bps, quality) = keyed_io_rates(&current, &previous, 1.0);

        assert_eq!((read_bps, write_bps), (500, 250));
        assert_eq!(quality.quality, MetricQuality::Partial);
    }

    #[test]
    fn malformed_required_counters_fail_closed() {
        assert!(parse_cpu_times("cpu 100 nope 30 400\n").is_err());
        assert!(parse_cpu_times("cpu 100 2 30\n").is_err());
        assert_eq!(
            parse_cpu_times("cpu 100 2 30 400 5 6 7\n").unwrap_err(),
            "linux_proc_stat_cpu_too_short:cpu:expected=8:actual=7"
        );
        assert_eq!(
            parse_cpu_times("cpu 100 2 30 400 5 6 7 8\ncpu0 50 1 10 200 2 3 4\n").unwrap_err(),
            "linux_proc_stat_cpu_too_short:cpu0:expected=8:actual=7"
        );
        assert!(parse_network_totals(
            "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\neth0: nope 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n"
        )
        .is_err());
        assert!(parse_block_stat("1 0 nope 0 2 0 16 0").is_err());
        assert!(parse_block_stat("1 0 8 0 2 0 nope 0").is_err());
    }

    fn create_block_device(root: &Path, name: &str, stat: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.join("holders")).unwrap();
        fs::write(path.join("stat"), stat).unwrap();
    }
}
