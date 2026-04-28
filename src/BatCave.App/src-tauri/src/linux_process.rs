#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{collections::HashMap, fs, path::Path, process::Command};

use crate::contracts::{
    AccessState, MetricQuality, MetricQualityInfo, MetricSource, ProcessMetricQuality,
    ProcessSample,
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
    let private_bytes =
        read_private_bytes(&proc_dir).unwrap_or_else(|| rss_bytes(stat.rss_pages, page_size));
    let handles = fs::read_dir(proc_dir.join("fd"))
        .map(|entries| entries.filter_map(Result::ok).count() as u32)
        .unwrap_or_default();
    let has_exe = !exe.is_empty();
    let has_fd = handles > 0;
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
            virtual_memory_bytes: stat.virtual_memory_bytes,
            disk_read_total_bytes: io.read_bytes,
            disk_write_total_bytes: io.write_bytes,
            other_io_total_bytes: None,
            disk_read_bps: 0,
            disk_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: stat.threads,
            handles,
            access_state,
            quality: Some(linux_process_quality(access_state, has_io)),
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
    let pid = content[..open]
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("linux_proc_stat_pid_parse_failed:{error}"))?;
    let name = content[open + 1..close].to_string();
    let fields = content[close + 1..].split_whitespace().collect::<Vec<_>>();
    if fields.len() < 22 {
        return Err("linux_proc_stat_too_short".to_string());
    }

    Ok(ProcessStat {
        pid,
        name,
        status: process_status_label(fields[0]),
        parent_pid: fields[1]
            .parse::<u32>()
            .ok()
            .filter(|parent| *parent != 0)
            .map(|parent| parent.to_string()),
        user_ticks: fields[11].parse::<u64>().unwrap_or_default(),
        kernel_ticks: fields[12].parse::<u64>().unwrap_or_default(),
        threads: fields[17].parse::<u32>().unwrap_or_default(),
        start_time_ticks: fields[19].parse::<u64>().unwrap_or_default(),
        virtual_memory_bytes: fields[20].parse::<u64>().unwrap_or_default(),
        rss_pages: fields[21].parse::<i64>().unwrap_or_default(),
    })
}

fn read_process_io(proc_dir: &Path) -> Result<ProcessIo, String> {
    parse_process_io(
        &fs::read_to_string(proc_dir.join("io"))
            .map_err(|error| format!("linux_proc_process_io_read_failed:{proc_dir:?}:{error}"))?,
    )
}

fn parse_process_io(content: &str) -> Result<ProcessIo, String> {
    let mut io = ProcessIo::default();
    for line in content.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().parse::<u64>().unwrap_or_default();
        match key {
            "read_bytes" => io.read_bytes = value,
            "write_bytes" => io.write_bytes = value,
            _ => {}
        }
    }
    Ok(io)
}

fn read_private_bytes(proc_dir: &Path) -> Option<u64> {
    parse_status_value_kib(
        &fs::read_to_string(proc_dir.join("status")).ok()?,
        "RssAnon",
    )
    .map(|kib| kib.saturating_mul(1024))
}

fn parse_status_value_kib(content: &str, key: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        (name == key).then(|| {
            rest.split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
        })?
    })
}

fn read_cpu_snapshot() -> Result<CpuSnapshot, String> {
    parse_cpu_snapshot(
        &fs::read_to_string("/proc/stat")
            .map_err(|error| format!("linux_proc_stat_read_failed:{error}"))?,
    )
}

fn parse_cpu_snapshot(content: &str) -> Result<CpuSnapshot, String> {
    let mut total_ticks = None;
    let mut logical_cpu_count = 0;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            total_ticks = Some(
                rest.split_whitespace()
                    .filter_map(|value| value.parse::<u64>().ok())
                    .fold(0_u64, u64::saturating_add),
            );
        } else if line
            .strip_prefix("cpu")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|value| value.is_ascii_digit())
        {
            logical_cpu_count += 1;
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

fn linux_process_quality(access_state: AccessState, has_io: bool) -> ProcessMetricQuality {
    let direct_quality = match access_state {
        AccessState::Full => MetricQuality::Native,
        AccessState::Partial => MetricQuality::Partial,
        AccessState::Denied => MetricQuality::Unavailable,
    };
    let procfs = |quality| MetricQualityInfo::new(quality, MetricSource::Procfs);

    ProcessMetricQuality {
        cpu: Some(
            MetricQualityInfo::new(MetricQuality::Held, MetricSource::Procfs)
                .with_message("Linux process CPU needs a second /proc sample."),
        ),
        memory: Some(procfs(direct_quality)),
        disk: Some(if has_io {
            procfs(direct_quality)
        } else {
            procfs(MetricQuality::Unavailable)
                .with_message("Linux process I/O requires /proc/<pid>/io access.")
        }),
        other_io: Some(
            procfs(MetricQuality::Unavailable)
                .with_message("Linux /proc does not expose Windows-style other I/O totals."),
        ),
        network: Some(
            procfs(MetricQuality::Unavailable)
                .with_message("Linux per-process network attribution is not exposed by /proc."),
        ),
        threads: Some(procfs(direct_quality)),
        handles: Some(procfs(direct_quality)),
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
    fn parse_process_io_extracts_disk_bytes() {
        let io = parse_process_io("rchar: 10\nwchar: 20\nread_bytes: 4096\nwrite_bytes: 8192\n")
            .unwrap();

        assert_eq!(io.read_bytes, 4096);
        assert_eq!(io.write_bytes, 8192);
    }

    #[test]
    fn parse_status_value_reads_kib_values() {
        assert_eq!(
            parse_status_value_kib("Name:\ttest\nRssAnon:\t  128 kB\n", "RssAnon"),
            Some(128)
        );
    }

    #[test]
    fn parse_cpu_snapshot_counts_logical_cpus() {
        let snapshot = parse_cpu_snapshot("cpu  1 2 3 4\ncpu0 1 1 1 1\ncpu1 1 1 1 1\n").unwrap();

        assert_eq!(snapshot.total_ticks, 10);
        assert_eq!(snapshot.logical_cpu_count, 2);
    }

    #[test]
    fn rss_bytes_saturates_negative_pages_to_zero() {
        assert_eq!(rss_bytes(-1, 4096), 0);
        assert_eq!(rss_bytes(3, 4096), 12_288);
    }
}
