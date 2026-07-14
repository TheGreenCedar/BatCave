#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{collections::HashMap, fmt::Display, fs, path::Path, process::Command, str::FromStr};

use crate::contracts::{
    AccessState, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
    ProcessMetricQuality, ProcessSample,
};

#[derive(Debug)]
pub struct LinuxProcessCollector {
    previous: HashMap<ProcessKey, ProcessCpuTicks>,
    ticks_per_second: u64,
    page_size: u64,
    boot_time_ms: u64,
}

impl LinuxProcessCollector {
    pub fn new() -> Self {
        Self {
            previous: HashMap::new(),
            ticks_per_second: getconf_u64("CLK_TCK", 100),
            page_size: getconf_u64("PAGESIZE", 4096),
            boot_time_ms: read_boot_time_ms().unwrap_or_default(),
        }
    }

    pub fn collect(&mut self) -> Result<Vec<ProcessSample>, String> {
        if self.boot_time_ms == 0 {
            self.boot_time_ms = read_boot_time_ms().unwrap_or_default();
        }

        let cpu_snapshot = read_cpu_snapshot()?;
        let mut current_ticks = HashMap::new();
        let mut processes = Vec::new();

        for entry in
            fs::read_dir("/proc").map_err(|error| format!("linux_proc_read_failed:{error}"))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Ok(pid) = file_name.parse::<u32>() else {
                continue;
            };
            let Ok(raw) = read_process(
                pid,
                self.ticks_per_second,
                self.page_size,
                self.boot_time_ms,
            ) else {
                continue;
            };

            let key = ProcessKey {
                pid,
                start_time_ticks: raw.start_time_ticks,
            };
            let mut process = raw.sample;
            if let Some(previous) = self.previous.get(&key) {
                let cpu_total_delta = cpu_snapshot
                    .total_ticks
                    .saturating_sub(previous.cpu_total_ticks);
                let process_delta = raw
                    .cpu_ticks
                    .total()
                    .saturating_sub(previous.process_ticks.total());
                let kernel_delta = raw
                    .cpu_ticks
                    .kernel
                    .saturating_sub(previous.process_ticks.kernel);
                if cpu_total_delta > 0 {
                    let logical_factor = cpu_snapshot.logical_cpu_count.max(1) as f64;
                    process.cpu_percent = round1(
                        (process_delta as f64 / cpu_total_delta as f64) * logical_factor * 100.0,
                    );
                    process.kernel_cpu_percent = Some(round1(
                        (kernel_delta as f64 / cpu_total_delta as f64) * logical_factor * 100.0,
                    ));
                    process_quality(&mut process).cpu = Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::Procfs,
                    ));
                }
            }

            current_ticks.insert(
                key,
                ProcessCpuTicks {
                    process_ticks: raw.cpu_ticks,
                    cpu_total_ticks: cpu_snapshot.total_ticks,
                },
            );
            processes.push(process);
        }

        self.previous = current_ticks;
        Ok(processes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProcessKey {
    pid: u32,
    start_time_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
struct ProcessCpuTicks {
    process_ticks: ProcessTicks,
    cpu_total_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
struct ProcessTicks {
    user: u64,
    kernel: u64,
}

impl ProcessTicks {
    fn total(self) -> u64 {
        self.user.saturating_add(self.kernel)
    }
}

#[derive(Debug)]
struct RawLinuxProcess {
    sample: ProcessSample,
    cpu_ticks: ProcessTicks,
    start_time_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
struct CpuSnapshot {
    total_ticks: u64,
    logical_cpu_count: usize,
}

#[derive(Debug)]
struct ProcessStat {
    pid: u32,
    name: String,
    status: String,
    parent_pid: Option<String>,
    user_ticks: u64,
    kernel_ticks: u64,
    threads: u32,
    start_time_ticks: u64,
    virtual_memory_bytes: u64,
    rss_pages: i64,
}

#[derive(Debug, Default)]
struct ProcessIo {
    read_bytes: u64,
    write_bytes: u64,
}

fn read_process(
    pid: u32,
    ticks_per_second: u64,
    page_size: u64,
    boot_time_ms: u64,
) -> Result<RawLinuxProcess, String> {
    let proc_dir = Path::new("/proc").join(pid.to_string());
    let stat = parse_process_stat(
        &fs::read_to_string(proc_dir.join("stat"))
            .map_err(|error| format!("linux_proc_process_stat_read_failed:{pid}:{error}"))?,
    )?;
    let io_result = read_process_io(&proc_dir);
    let has_io = io_result.is_ok();
    let io = io_result.unwrap_or_default();
    let exe = fs::read_link(proc_dir.join("exe"))
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let measured_private_bytes = read_private_bytes(&proc_dir);
    let private_bytes =
        measured_private_bytes.unwrap_or_else(|| rss_bytes(stat.rss_pages, page_size));
    let handles_result = fs::read_dir(proc_dir.join("fd"))
        .map(|entries| u32::try_from(entries.filter_map(Result::ok).count()).unwrap_or(u32::MAX));
    let has_fd = handles_result.is_ok();
    let handles = handles_result.unwrap_or_default();
    let has_exe = !exe.is_empty();
    let access_state = if has_exe && has_io && has_fd {
        AccessState::Full
    } else {
        AccessState::Partial
    };
    let start_time_ms = boot_time_ms.saturating_add(
        stat.start_time_ticks
            .saturating_mul(1000)
            .checked_div(ticks_per_second.max(1))
            .unwrap_or_default(),
    );

    Ok(RawLinuxProcess {
        cpu_ticks: ProcessTicks {
            user: stat.user_ticks,
            kernel: stat.kernel_ticks,
        },
        start_time_ticks: stat.start_time_ticks,
        sample: ProcessSample {
            pid: stat.pid.to_string(),
            parent_pid: stat.parent_pid,
            start_time_ms,
            name: stat.name,
            exe,
            status: stat.status,
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: rss_bytes(stat.rss_pages, page_size),
            private_bytes,
            virtual_memory_bytes: Some(stat.virtual_memory_bytes),
            io_read_total_bytes: io.read_bytes,
            io_write_total_bytes: io.write_bytes,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: stat.threads,
            handles,
            access_state,
            quality: Some(linux_process_quality(
                access_state,
                has_io,
                measured_private_bytes.is_some(),
            )),
        },
    })
}

fn parse_process_stat(content: &str) -> Result<ProcessStat, String> {
    let open = content
        .find('(')
        .ok_or_else(|| "linux_proc_stat_missing_name_open".to_string())?;
    let close = content
        .rfind(')')
        .ok_or_else(|| "linux_proc_stat_missing_name_close".to_string())?;
    if close <= open {
        return Err("linux_proc_stat_invalid_name_bounds".to_string());
    }
    let pid = content[..open]
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("linux_proc_stat_pid_parse_failed:{error}"))?;
    let name = content[open + 1..close].to_string();
    let fields = content[close + 1..].split_whitespace().collect::<Vec<_>>();
    if fields.len() < 22 {
        return Err("linux_proc_stat_too_short".to_string());
    }

    let parent_pid = parse_process_stat_field::<u32>(&fields, 1, "parent_pid")?;
    Ok(ProcessStat {
        pid,
        name,
        status: process_status_label(fields[0]),
        parent_pid: (parent_pid != 0).then(|| parent_pid.to_string()),
        user_ticks: parse_process_stat_field(&fields, 11, "user_ticks")?,
        kernel_ticks: parse_process_stat_field(&fields, 12, "kernel_ticks")?,
        threads: parse_process_stat_field(&fields, 17, "threads")?,
        start_time_ticks: parse_process_stat_field(&fields, 19, "start_time_ticks")?,
        virtual_memory_bytes: parse_process_stat_field(&fields, 20, "virtual_memory_bytes")?,
        rss_pages: parse_process_stat_field(&fields, 21, "rss_pages")?,
    })
}

fn parse_process_stat_field<T>(fields: &[&str], index: usize, name: &str) -> Result<T, String>
where
    T: FromStr,
    T::Err: Display,
{
    fields[index]
        .parse::<T>()
        .map_err(|error| format!("linux_proc_stat_{name}_parse_failed:{error}"))
}

fn read_process_io(proc_dir: &Path) -> Result<ProcessIo, String> {
    parse_process_io(
        &fs::read_to_string(proc_dir.join("io"))
            .map_err(|error| format!("linux_proc_process_io_read_failed:{proc_dir:?}:{error}"))?,
    )
}

fn parse_process_io(content: &str) -> Result<ProcessIo, String> {
    let mut read_bytes = None;
    let mut write_bytes = None;
    for line in content.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let slot = match key {
            "read_bytes" => &mut read_bytes,
            "write_bytes" => &mut write_bytes,
            _ => continue,
        };
        if slot.is_some() {
            return Err(format!("linux_proc_process_io_duplicate_field:{key}"));
        }
        *slot = Some(
            value
                .trim()
                .parse::<u64>()
                .map_err(|error| format!("linux_proc_process_io_{key}_parse_failed:{error}"))?,
        );
    }
    Ok(ProcessIo {
        read_bytes: read_bytes
            .ok_or_else(|| "linux_proc_process_io_missing_read_bytes".to_string())?,
        write_bytes: write_bytes
            .ok_or_else(|| "linux_proc_process_io_missing_write_bytes".to_string())?,
    })
}

fn read_private_bytes(proc_dir: &Path) -> Option<u64> {
    parse_status_value_kib(
        &fs::read_to_string(proc_dir.join("status")).ok()?,
        "RssAnon",
    )
    .map(|kib| kib.saturating_mul(1024))
}

fn parse_status_value_kib(content: &str, key: &str) -> Option<u64> {
    for line in content.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name != key {
            continue;
        }
        let mut fields = rest.split_whitespace();
        let value = fields.next()?.parse::<u64>().ok()?;
        if fields.next()? != "kB" || fields.next().is_some() {
            return None;
        }
        return Some(value);
    }
    None
}

fn read_cpu_snapshot() -> Result<CpuSnapshot, String> {
    parse_cpu_snapshot(
        &fs::read_to_string("/proc/stat")
            .map_err(|error| format!("linux_proc_stat_read_failed:{error}"))?,
    )
}

fn parse_cpu_snapshot(content: &str) -> Result<CpuSnapshot, String> {
    let mut total_ticks = None;
    let mut logical_cpu_count = 0_usize;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            let mut parsed_any = false;
            let mut total = 0_u64;
            for value in rest.split_whitespace() {
                parsed_any = true;
                total =
                    total.saturating_add(value.parse::<u64>().map_err(|error| {
                        format!("linux_proc_stat_cpu_total_parse_failed:{error}")
                    })?);
            }
            if !parsed_any {
                return Err("linux_proc_stat_cpu_total_empty".to_string());
            }
            total_ticks = Some(total);
        } else if line.split_whitespace().next().is_some_and(|label| {
            label.strip_prefix("cpu").is_some_and(|index| {
                !index.is_empty() && index.chars().all(|value| value.is_ascii_digit())
            })
        }) {
            logical_cpu_count = logical_cpu_count.saturating_add(1);
        }
    }

    Ok(CpuSnapshot {
        total_ticks: total_ticks.ok_or_else(|| "linux_proc_stat_missing_cpu_total".to_string())?,
        logical_cpu_count,
    })
}

fn read_boot_time_ms() -> Option<u64> {
    fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("btime ")
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1000))
        })
}

fn linux_process_quality(
    access_state: AccessState,
    has_io: bool,
    has_private_memory: bool,
) -> ProcessMetricQuality {
    let procfs = |quality| MetricQualityInfo::new(quality, MetricSource::Procfs);
    let direct = || match access_state {
        AccessState::Full => procfs(MetricQuality::Native),
        AccessState::Partial => procfs(MetricQuality::Partial).with_limitation(
            MetricLimitationCode::PartialCoverage,
            "Some /proc files were unavailable for this process.",
        ),
        AccessState::Denied => procfs(MetricQuality::Unavailable).with_limitation(
            MetricLimitationCode::AccessDenied,
            "Process telemetry was denied by /proc permissions.",
        ),
    };

    ProcessMetricQuality {
        cpu: Some(
            MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs).with_limitation(
                MetricLimitationCode::PendingBaseline,
                "Linux process CPU needs a second /proc sample.",
            ),
        ),
        memory: Some(if has_private_memory {
            direct()
        } else {
            procfs(MetricQuality::Estimated).with_limitation(
                MetricLimitationCode::UnsupportedMetric,
                "RssAnon is unavailable; private memory uses RSS as an estimate.",
            )
        }),
        io: Some(if has_io {
            direct()
        } else {
            procfs(MetricQuality::Unavailable).with_limitation(
                if access_state == AccessState::Denied {
                    MetricLimitationCode::AccessDenied
                } else {
                    MetricLimitationCode::UnsupportedMetric
                },
                "Linux process I/O requires /proc/<pid>/io access.",
            )
        }),
        other_io: Some(procfs(MetricQuality::Unavailable).with_limitation(
            MetricLimitationCode::UnsupportedMetric,
            "Linux /proc does not expose Windows-style other I/O totals.",
        )),
        network: Some(procfs(MetricQuality::Unavailable).with_limitation(
            MetricLimitationCode::UnsupportedMetric,
            "Linux per-process network attribution is not exposed by /proc.",
        )),
        threads: Some(direct()),
        handles: Some(direct()),
    }
}

fn process_quality(process: &mut ProcessSample) -> &mut ProcessMetricQuality {
    process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default)
}

fn process_status_label(value: &str) -> String {
    match value {
        "R" => "running",
        "S" => "sleeping",
        "D" => "disk_sleep",
        "Z" => "zombie",
        "T" | "t" => "stopped",
        "I" => "idle",
        "W" => "paging",
        "X" | "x" => "dead",
        "K" => "wakekill",
        "P" => "parked",
        _ => value,
    }
    .to_string()
}

fn rss_bytes(pages: i64, page_size: u64) -> u64 {
    u64::try_from(pages)
        .unwrap_or_default()
        .saturating_mul(page_size)
}

fn getconf_u64(name: &str, fallback: u64) -> u64 {
    Command::new("getconf")
        .arg(name)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_process_stat_handles_names_with_spaces() {
        let stat = parse_process_stat(
            "42 (bat cave worker) S 1 2 3 4 5 6 7 8 9 10 100 30 14 15 16 17 8 19 12345 4096 3",
        )
        .unwrap();

        assert_eq!(stat.pid, 42);
        assert_eq!(stat.name, "bat cave worker");
        assert_eq!(stat.parent_pid, Some("1".to_string()));
        assert_eq!(stat.user_ticks, 100);
        assert_eq!(stat.kernel_ticks, 30);
        assert_eq!(stat.threads, 8);
        assert_eq!(stat.start_time_ticks, 12345);
        assert_eq!(stat.virtual_memory_bytes, 4096);
        assert_eq!(stat.rss_pages, 3);
    }

    #[test]
    fn parse_process_stat_handles_parentheses_inside_process_name() {
        let stat = parse_process_stat(
            "42 (bat ) cave (worker)) S 0 2 3 4 5 6 7 8 9 10 100 30 14 15 16 17 8 19 12345 4096 -1",
        )
        .unwrap();

        assert_eq!(stat.name, "bat ) cave (worker)");
        assert_eq!(stat.parent_pid, None);
        assert_eq!(stat.rss_pages, -1);
    }

    #[test]
    fn parse_process_stat_rejects_invalid_bounds_and_numeric_fields() {
        assert_eq!(
            parse_process_stat(") 42 (").unwrap_err(),
            "linux_proc_stat_invalid_name_bounds"
        );
        assert!(parse_process_stat(
            "42 (worker) S nope 2 3 4 5 6 7 8 9 10 100 30 14 15 16 17 8 19 12345 4096 3",
        )
        .unwrap_err()
        .starts_with("linux_proc_stat_parent_pid_parse_failed:"));
        assert!(parse_process_stat(
            "42 (worker) S 1 2 3 4 5 6 7 8 9 10 invalid 30 14 15 16 17 8 19 12345 4096 3",
        )
        .unwrap_err()
        .starts_with("linux_proc_stat_user_ticks_parse_failed:"));
        assert_eq!(
            parse_process_stat("42 (worker) S 1 2 3").unwrap_err(),
            "linux_proc_stat_too_short"
        );
    }

    #[test]
    fn parse_process_io_extracts_disk_bytes() {
        let io = parse_process_io("rchar: 10\nwchar: 20\nread_bytes: 4096\nwrite_bytes: 8192\n")
            .unwrap();

        assert_eq!(io.read_bytes, 4096);
        assert_eq!(io.write_bytes, 8192);
    }

    #[test]
    fn parse_process_io_rejects_missing_duplicate_and_malformed_counters() {
        assert_eq!(
            parse_process_io("read_bytes: 1\n").unwrap_err(),
            "linux_proc_process_io_missing_write_bytes"
        );
        assert_eq!(
            parse_process_io("read_bytes: 1\nread_bytes: 2\nwrite_bytes: 3\n").unwrap_err(),
            "linux_proc_process_io_duplicate_field:read_bytes"
        );
        assert!(parse_process_io("read_bytes: -1\nwrite_bytes: 3\n")
            .unwrap_err()
            .starts_with("linux_proc_process_io_read_bytes_parse_failed:"));
        assert!(
            parse_process_io("read_bytes: 1\nwrite_bytes: 18446744073709551616\n")
                .unwrap_err()
                .starts_with("linux_proc_process_io_write_bytes_parse_failed:")
        );
    }

    #[test]
    fn parse_status_value_reads_kib_values() {
        assert_eq!(
            parse_status_value_kib("Name:\ttest\nRssAnon:\t  128 kB\n", "RssAnon"),
            Some(128)
        );
        assert_eq!(
            parse_status_value_kib("RssAnon:\t128 MB\n", "RssAnon"),
            None
        );
        assert_eq!(parse_status_value_kib("RssAnon:\t-1 kB\n", "RssAnon"), None);
        assert_eq!(
            parse_status_value_kib("RssAnon:\t128 kB trailing\n", "RssAnon"),
            None
        );
    }

    #[test]
    fn parse_cpu_snapshot_counts_logical_cpus() {
        let snapshot =
            parse_cpu_snapshot("cpu  1 2 3 4\ncpu0 1 1 1 1\ncpu1 1 1 1 1\ncpu1guest 1 1 1 1\n")
                .unwrap();

        assert_eq!(snapshot.total_ticks, 10);
        assert_eq!(snapshot.logical_cpu_count, 2);
    }

    #[test]
    fn parse_cpu_snapshot_rejects_partial_or_empty_totals() {
        assert!(parse_cpu_snapshot("cpu  1 nope 3\ncpu0 1 1 1\n")
            .unwrap_err()
            .starts_with("linux_proc_stat_cpu_total_parse_failed:"));
        assert_eq!(
            parse_cpu_snapshot("cpu \ncpu0 1 1 1\n").unwrap_err(),
            "linux_proc_stat_cpu_total_empty"
        );
        assert_eq!(
            parse_cpu_snapshot("intr 1\n").unwrap_err(),
            "linux_proc_stat_missing_cpu_total"
        );
    }

    #[test]
    fn rss_bytes_saturates_negative_pages_to_zero() {
        assert_eq!(rss_bytes(-1, 4096), 0);
        assert_eq!(rss_bytes(3, 4096), 12_288);
    }

    #[test]
    fn rss_private_fallback_is_marked_estimated() {
        let quality = linux_process_quality(AccessState::Full, true, false)
            .memory
            .expect("memory quality exists");

        assert_eq!(quality.quality, MetricQuality::Estimated);
        assert!(quality
            .message
            .expect("fallback message exists")
            .contains("RssAnon"));
    }

    #[test]
    fn partial_and_denied_procfs_quality_have_typed_explanations() {
        for (access, expected_quality, expected_limitation) in [
            (
                AccessState::Partial,
                MetricQuality::Partial,
                MetricLimitationCode::PartialCoverage,
            ),
            (
                AccessState::Denied,
                MetricQuality::Unavailable,
                MetricLimitationCode::AccessDenied,
            ),
        ] {
            let quality = linux_process_quality(access, true, true)
                .threads
                .expect("thread quality exists");

            assert_eq!(quality.quality, expected_quality);
            assert_eq!(quality.limitation_code, Some(expected_limitation));
            assert!(quality.message.is_some());
        }
    }
}
