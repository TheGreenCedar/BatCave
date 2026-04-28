use std::{
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::telemetry::TelemetryCollector;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkSummary {
    host: String,
    measurement_origin: String,
    ticks: usize,
    sleep_ms: u64,
    tick_p95_ms: f64,
    sort_p95_ms: f64,
    cpu_budget_pct: f64,
    rss_budget_bytes: u64,
    strict_passed: bool,
    baseline_metadata_matched: bool,
    core_speedup_passed: bool,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--benchmark") {
        return None;
    }

    match run_benchmark_from_args(args) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(payload) => {
                println!("{payload}");
                Some(0)
            }
            Err(error) => {
                eprintln!("benchmark_serialize_failed:{error}");
                Some(1)
            }
        },
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

fn run_benchmark_from_args(args: &[String]) -> Result<BenchmarkSummary, String> {
    let ticks = parse_usize(args, "--ticks", 120)?;
    let sleep_ms = parse_u64(args, "--sleep-ms", 1000)?;
    let strict = args.iter().any(|arg| arg == "--strict");
    let max_p95_ms = parse_optional_f64(args, "--max-p95-ms")?;
    let _baseline_json = parse_optional_string(args, "--baseline-json")?;
    let _min_speedup = parse_optional_f64(args, "--min-speedup-multiplier")?;
    reject_unknown_args(args)?;

    let collector = TelemetryCollector::new();
    let mut tick_values = Vec::with_capacity(ticks);
    let mut sort_values = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let started = Instant::now();
        let sample = collector.collect()?;
        tick_values.push(started.elapsed().as_secs_f64() * 1000.0);

        let sort_started = Instant::now();
        let mut rows = sample.processes;
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        sort_values.push(sort_started.elapsed().as_secs_f64() * 1000.0);

        if sleep_ms > 0 {
            thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    let tick_p95_ms = round1(p95(&tick_values));
    let sort_p95_ms = round1(p95(&sort_values));
    let strict_passed = !strict || max_p95_ms.map(|max| tick_p95_ms <= max).unwrap_or(true);

    Ok(BenchmarkSummary {
        host: "core".to_string(),
        measurement_origin: "rust_tauri_runtime".to_string(),
        ticks,
        sleep_ms,
        tick_p95_ms,
        sort_p95_ms,
        cpu_budget_pct: 6.0,
        rss_budget_bytes: 350 * 1024 * 1024,
        strict_passed,
        baseline_metadata_matched: false,
        core_speedup_passed: !strict,
    })
}

fn reject_unknown_args(args: &[String]) -> Result<(), String> {
    let known_with_value = [
        "--ticks",
        "--sleep-ms",
        "--baseline-json",
        "--min-speedup-multiplier",
        "--max-p95-ms",
    ];
    let known_flags = ["--benchmark", "--strict"];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if known_flags.contains(&arg.as_str()) {
            index += 1;
        } else if known_with_value.contains(&arg.as_str()) {
            if index + 1 >= args.len() {
                return Err(format!("missing_value_for_argument:{arg}"));
            }
            if args[index + 1].starts_with("--") {
                return Err(format!("missing_value_for_argument:{arg}"));
            }
            index += 2;
        } else {
            return Err(format!("unknown_argument:{arg}"));
        }
    }

    Ok(())
}

fn parse_usize(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    parse_optional_string(args, name)?
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|error| format!("invalid_argument:{name}:{error}"))
        })
        .unwrap_or(Ok(default))
}

fn parse_u64(args: &[String], name: &str, default: u64) -> Result<u64, String> {
    parse_optional_string(args, name)?
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| format!("invalid_argument:{name}:{error}"))
        })
        .unwrap_or(Ok(default))
}

fn parse_optional_f64(args: &[String], name: &str) -> Result<Option<f64>, String> {
    parse_optional_string(args, name)?
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|error| format!("invalid_argument:{name}:{error}"))
        })
        .transpose()
}

fn parse_optional_string(args: &[String], name: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("missing_value_for_argument:{name}"))
        .map(Some)
}

fn p95(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut values = values.to_vec();
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    values[index.min(values.len() - 1)]
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p95_uses_nearest_rank() {
        assert_eq!(p95(&[1.0, 2.0, 3.0, 4.0]), 4.0);
    }

    #[test]
    fn benchmark_cli_rejects_unknown_arguments() {
        let args = vec!["--benchmark".to_string(), "--wat".to_string()];
        assert_eq!(
            reject_unknown_args(&args),
            Err("unknown_argument:--wat".to_string())
        );
    }
}
