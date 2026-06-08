use std::{
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{cli_args, telemetry::TelemetryCollector};

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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkBaseline {
    ticks: usize,
    sleep_ms: u64,
    tick_p95_ms: f64,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--benchmark") {
        return None;
    }

    match run_benchmark_from_args(args) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(payload) => {
                println!("{payload}");
                Some(if summary.strict_passed { 0 } else { 1 })
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
    let min_speedup = parse_optional_f64(args, "--min-speedup-multiplier")?;
    reject_unknown_args(args)?;
    let baseline = parse_baseline(args, ticks, sleep_ms)?;

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
    let max_p95_passed = max_p95_ms.map(|max| tick_p95_ms <= max).unwrap_or(true);
    let core_speedup_passed = speedup_passed(baseline.as_ref(), tick_p95_ms, min_speedup);
    let strict_passed = !strict || (max_p95_passed && core_speedup_passed);

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
        baseline_metadata_matched: baseline.is_some(),
        core_speedup_passed,
    })
}

fn parse_baseline(
    args: &[String],
    expected_ticks: usize,
    expected_sleep_ms: u64,
) -> Result<Option<BenchmarkBaseline>, String> {
    let Some(path) = parse_optional_string(args, "--baseline-json")? else {
        return Ok(None);
    };

    let payload = std::fs::read_to_string(&path)
        .map_err(|error| format!("baseline_json_read_failed path={path} error={error}"))?;
    let baseline = serde_json::from_str::<BenchmarkBaseline>(&payload)
        .map_err(|error| format!("baseline_json_parse_failed path={path} error={error}"))?;
    if baseline.ticks != expected_ticks || baseline.sleep_ms != expected_sleep_ms {
        return Err(format!(
            "baseline_metadata_mismatch expected_ticks={} expected_sleep_ms={} actual_ticks={} actual_sleep_ms={}",
            expected_ticks, expected_sleep_ms, baseline.ticks, baseline.sleep_ms
        ));
    }

    Ok(Some(baseline))
}

fn speedup_passed(
    baseline: Option<&BenchmarkBaseline>,
    current_tick_p95_ms: f64,
    min_speedup: Option<f64>,
) -> bool {
    let Some(min_speedup) = min_speedup else {
        return true;
    };
    let Some(baseline) = baseline else {
        return false;
    };
    if current_tick_p95_ms <= 0.0 {
        return true;
    }

    baseline.tick_p95_ms / current_tick_p95_ms >= min_speedup
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
    cli_args::reject_unknown_args(args, &known_with_value, &known_flags)
}

fn parse_usize(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    parse_value(args, name, default)
}

fn parse_u64(args: &[String], name: &str, default: u64) -> Result<u64, String> {
    parse_value(args, name, default)
}

fn parse_optional_f64(args: &[String], name: &str) -> Result<Option<f64>, String> {
    parse_optional_value(args, name)
}

fn parse_value<T>(args: &[String], name: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    Ok(parse_optional_value(args, name)?.unwrap_or(default))
}

fn parse_optional_value<T>(args: &[String], name: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    parse_optional_string(args, name)?
        .map(|value| {
            value
                .parse::<T>()
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

    #[test]
    fn missing_baseline_file_fails() {
        let path = std::env::temp_dir().join(format!(
            "batcave-missing-baseline-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];

        let error = parse_baseline(&args, 2, 0).expect_err("missing baseline fails");

        assert!(error.contains("baseline_json_read_failed"));
    }

    #[test]
    fn baseline_metadata_mismatch_fails() {
        let path = std::env::temp_dir().join(format!(
            "batcave-mismatch-baseline-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"ticks":120,"sleep_ms":1000,"tick_p95_ms":10.0}"#)
            .expect("baseline fixture writes");
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];

        let error = parse_baseline(&args, 2, 0).expect_err("metadata mismatch fails");

        std::fs::remove_file(&path).expect("baseline fixture cleanup");
        assert!(error.contains("baseline_metadata_mismatch"));
    }

    #[test]
    fn speedup_gate_uses_baseline_over_current_ratio() {
        let baseline = BenchmarkBaseline {
            ticks: 2,
            sleep_ms: 0,
            tick_p95_ms: 100.0,
        };

        assert!(speedup_passed(Some(&baseline), 10.0, Some(2.0)));
        assert!(!speedup_passed(Some(&baseline), 80.0, Some(2.0)));
        assert!(speedup_passed(None, 80.0, None));
    }

    #[test]
    fn speedup_gate_fails_when_min_speedup_lacks_baseline() {
        assert!(!speedup_passed(None, 80.0, Some(2.0)));
    }
}
