use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{cli_args, runtime_store::RuntimeState};

const FORMAT_VERSION: u32 = 3;
const HOST: &str = "core";
const MEASUREMENT_ORIGIN: &str = "runtime_state_refresh_and_json_serialization";
const DEFAULT_MIN_SPEED_RATIO: f64 = 0.90;
const MAX_APP_CPU_PERCENT: f64 = 25.0;
const MAX_APP_RSS_BYTES: u64 = 350 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkRepeat {
    tick_p95_ms: f64,
    peak_app_cpu_percent: f64,
    peak_app_rss_bytes: u64,
    samples_advanced: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkSummary {
    format_version: u32,
    host: String,
    measurement_origin: String,
    platform: String,
    architecture: String,
    machine_class: String,
    workload_profile: String,
    warmup_ticks: usize,
    measured_ticks: usize,
    sleep_ms: u64,
    repeat_count: usize,
    repeats: Vec<BenchmarkRepeat>,
    median_tick_p95_ms: f64,
    peak_app_cpu_percent: f64,
    peak_app_rss_bytes: u64,
    max_p95_passed: bool,
    baseline_metadata_matched: bool,
    speed_ratio: Option<f64>,
    speed_ratio_passed: bool,
    resource_budget_passed: bool,
    sample_quality_passed: bool,
    strict_passed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkBaseline {
    format_version: u32,
    host: String,
    measurement_origin: String,
    platform: String,
    architecture: String,
    machine_class: String,
    workload_profile: String,
    warmup_ticks: usize,
    measured_ticks: usize,
    sleep_ms: u64,
    repeat_count: usize,
    median_tick_p95_ms: f64,
}

#[derive(Debug, Clone)]
struct BenchmarkConfig {
    platform: String,
    architecture: String,
    machine_class: String,
    workload_profile: String,
    warmup_ticks: usize,
    measured_ticks: usize,
    sleep_ms: u64,
    repeat_count: usize,
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
    let requested_platform = parse_string(args, "--platform", std::env::consts::OS)?;
    let requested_architecture = parse_string(args, "--architecture", std::env::consts::ARCH)?;
    let platform = std::env::consts::OS.to_string();
    let architecture = canonical_architecture(std::env::consts::ARCH).to_string();
    if canonical_platform(&requested_platform) != platform {
        return Err(format!(
            "invalid_argument:--platform:does_not_match_binary:{platform}"
        ));
    }
    if canonical_architecture(&requested_architecture) != architecture {
        return Err(format!(
            "invalid_argument:--architecture:does_not_match_binary:{architecture}"
        ));
    }
    let config = BenchmarkConfig {
        platform,
        architecture,
        machine_class: parse_string(args, "--machine-class", default_machine_class())?,
        workload_profile: parse_string(args, "--workload-profile", "fixed-default")?,
        warmup_ticks: parse_usize(args, "--warmup-ticks", 30)?,
        measured_ticks: parse_usize(args, "--ticks", 120)?,
        sleep_ms: parse_u64(args, "--sleep-ms", 1000)?,
        repeat_count: parse_usize(args, "--repeats", 5)?,
    };
    validate_config(&config)?;

    let strict = args.iter().any(|arg| arg == "--strict");
    let max_p95_ms = parse_optional_f64(args, "--max-p95-ms")?;
    let requested_min_speed_ratio = parse_optional_f64(args, "--min-speedup-multiplier")?;
    reject_unknown_args(args)?;

    if requested_min_speed_ratio.is_some() && !has_value(args, "--baseline-json") {
        return Err("invalid_benchmark_gate:min_speedup_requires_baseline".to_string());
    }
    if strict && !has_value(args, "--baseline-json") && max_p95_ms.is_none() {
        return Err("invalid_benchmark_gate:strict_requires_baseline_or_max_p95_ms".to_string());
    }
    validate_positive_finite("--max-p95-ms", max_p95_ms)?;
    validate_positive_finite("--min-speedup-multiplier", requested_min_speed_ratio)?;

    let baseline = parse_baseline(args, &config)?;
    let min_speed_ratio = requested_min_speed_ratio
        .or_else(|| (strict && baseline.is_some()).then_some(DEFAULT_MIN_SPEED_RATIO));

    let _runtime_dir = BenchmarkRuntimeDir::new()?;
    let state = RuntimeState::new();
    if config.warmup_ticks > 0 {
        measure_window(&state, config.warmup_ticks, config.sleep_ms)?;
    }

    let mut repeats = Vec::with_capacity(config.repeat_count);
    for _ in 0..config.repeat_count {
        repeats.push(measure_window(
            &state,
            config.measured_ticks,
            config.sleep_ms,
        )?);
    }

    let median_tick_p95_ms = median(
        &repeats
            .iter()
            .map(|repeat| repeat.tick_p95_ms)
            .collect::<Vec<_>>(),
    );
    let peak_app_cpu_percent = repeats
        .iter()
        .map(|repeat| repeat.peak_app_cpu_percent)
        .fold(0.0, f64::max);
    let peak_app_rss_bytes = repeats
        .iter()
        .map(|repeat| repeat.peak_app_rss_bytes)
        .max()
        .unwrap_or(0);
    let max_p95_passed = max_p95_ms
        .map(|maximum| median_tick_p95_ms <= maximum)
        .unwrap_or(true);
    let speed_ratio = baseline
        .as_ref()
        .map(|baseline| calculate_speed_ratio(baseline.median_tick_p95_ms, median_tick_p95_ms));
    let speed_ratio_passed = min_speed_ratio
        .map(|minimum| speed_ratio.is_some_and(|ratio| ratio >= minimum))
        .unwrap_or(true);
    let resource_budget_passed =
        peak_app_cpu_percent <= MAX_APP_CPU_PERCENT && peak_app_rss_bytes <= MAX_APP_RSS_BYTES;
    let sample_quality_passed = repeats.iter().all(|repeat| repeat.samples_advanced);
    let strict_passed = !strict
        || (max_p95_passed
            && speed_ratio_passed
            && resource_budget_passed
            && sample_quality_passed);

    Ok(BenchmarkSummary {
        format_version: FORMAT_VERSION,
        host: HOST.to_string(),
        measurement_origin: MEASUREMENT_ORIGIN.to_string(),
        platform: config.platform,
        architecture: config.architecture,
        machine_class: config.machine_class,
        workload_profile: config.workload_profile,
        warmup_ticks: config.warmup_ticks,
        measured_ticks: config.measured_ticks,
        sleep_ms: config.sleep_ms,
        repeat_count: config.repeat_count,
        repeats,
        median_tick_p95_ms: round1(median_tick_p95_ms),
        peak_app_cpu_percent: round1(peak_app_cpu_percent),
        peak_app_rss_bytes,
        max_p95_passed,
        baseline_metadata_matched: baseline.is_some(),
        speed_ratio: speed_ratio.map(round3),
        speed_ratio_passed,
        resource_budget_passed,
        sample_quality_passed,
        strict_passed,
    })
}

fn measure_window(
    state: &RuntimeState,
    ticks: usize,
    sleep_ms: u64,
) -> Result<BenchmarkRepeat, String> {
    let mut durations = Vec::with_capacity(ticks);
    let mut peak_app_cpu_percent = 0.0_f64;
    let mut peak_app_rss_bytes = 0_u64;
    let mut previous_sample_seq = None;
    let mut samples_advanced = true;

    for _ in 0..ticks {
        let started = Instant::now();
        let snapshot = state.refresh_now()?;
        serde_json::to_vec(&snapshot)
            .map_err(|error| format!("benchmark_snapshot_serialize_failed:{error}"))?;
        durations.push(started.elapsed().as_secs_f64() * 1000.0);
        peak_app_cpu_percent = peak_app_cpu_percent.max(snapshot.health.app_cpu_percent);
        peak_app_rss_bytes = peak_app_rss_bytes.max(snapshot.health.app_rss_bytes);
        samples_advanced &=
            previous_sample_seq.is_none_or(|previous| snapshot.sample_seq > previous);
        previous_sample_seq = Some(snapshot.sample_seq);

        if sleep_ms > 0 {
            thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    Ok(BenchmarkRepeat {
        tick_p95_ms: round1(p95(&durations)),
        peak_app_cpu_percent: round1(peak_app_cpu_percent),
        peak_app_rss_bytes,
        samples_advanced,
    })
}

fn canonical_platform(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "win32" | "win64" | "windows" => "windows".to_string(),
        "linux" => "linux".to_string(),
        other => other.to_string(),
    }
}

fn canonical_architecture(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "x64" | "amd64" | "x86_64" => "x86_64".to_string(),
        "arm64" | "aarch64" => "aarch64".to_string(),
        "x86" | "i686" => "x86".to_string(),
        other => other.to_string(),
    }
}

fn parse_baseline(
    args: &[String],
    expected: &BenchmarkConfig,
) -> Result<Option<BenchmarkBaseline>, String> {
    let Some(path) = parse_optional_string(args, "--baseline-json")? else {
        return Ok(None);
    };

    let payload = fs::read_to_string(&path)
        .map_err(|error| format!("baseline_json_read_failed path={path} error={error}"))?;
    let baseline = serde_json::from_str::<BenchmarkBaseline>(&payload)
        .map_err(|error| format!("baseline_json_parse_failed path={path} error={error}"))?;
    if baseline.format_version != FORMAT_VERSION {
        return Err(format!(
            "baseline_metadata_mismatch field=format_version expected={} actual={}",
            FORMAT_VERSION, baseline.format_version
        ));
    }
    require_metadata("host", HOST, &baseline.host)?;
    require_metadata(
        "measurement_origin",
        MEASUREMENT_ORIGIN,
        &baseline.measurement_origin,
    )?;
    require_metadata("platform", &expected.platform, &baseline.platform)?;
    require_metadata(
        "architecture",
        &expected.architecture,
        &baseline.architecture,
    )?;
    require_metadata(
        "machine_class",
        &expected.machine_class,
        &baseline.machine_class,
    )?;
    require_metadata(
        "workload_profile",
        &expected.workload_profile,
        &baseline.workload_profile,
    )?;
    require_number("warmup_ticks", expected.warmup_ticks, baseline.warmup_ticks)?;
    require_number(
        "measured_ticks",
        expected.measured_ticks,
        baseline.measured_ticks,
    )?;
    require_number("sleep_ms", expected.sleep_ms, baseline.sleep_ms)?;
    require_number("repeat_count", expected.repeat_count, baseline.repeat_count)?;
    validate_positive_finite(
        "baseline.median_tick_p95_ms",
        Some(baseline.median_tick_p95_ms),
    )?;

    Ok(Some(baseline))
}

fn require_metadata(field: &str, expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "baseline_metadata_mismatch field={field} expected={expected:?} actual={actual:?}"
        ))
    }
}

fn require_number<T>(field: &str, expected: T, actual: T) -> Result<(), String>
where
    T: std::fmt::Display + PartialEq,
{
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "baseline_metadata_mismatch field={field} expected={expected} actual={actual}"
        ))
    }
}

fn validate_config(config: &BenchmarkConfig) -> Result<(), String> {
    if config.measured_ticks == 0 {
        return Err("invalid_argument:--ticks:must_be_greater_than_zero".to_string());
    }
    if config.repeat_count == 0 {
        return Err("invalid_argument:--repeats:must_be_greater_than_zero".to_string());
    }
    for (name, value) in [
        ("--platform", &config.platform),
        ("--architecture", &config.architecture),
        ("--machine-class", &config.machine_class),
        ("--workload-profile", &config.workload_profile),
    ] {
        if value.trim().is_empty() {
            return Err(format!("invalid_argument:{name}:must_not_be_empty"));
        }
    }
    Ok(())
}

fn validate_positive_finite(name: &str, value: Option<f64>) -> Result<(), String> {
    if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        Err(format!(
            "invalid_argument:{name}:must_be_positive_and_finite"
        ))
    } else {
        Ok(())
    }
}

fn reject_unknown_args(args: &[String]) -> Result<(), String> {
    let known_with_value = [
        "--platform",
        "--architecture",
        "--machine-class",
        "--workload-profile",
        "--warmup-ticks",
        "--ticks",
        "--sleep-ms",
        "--repeats",
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

fn parse_string(args: &[String], name: &str, default: impl Into<String>) -> Result<String, String> {
    Ok(parse_optional_string(args, name)?.unwrap_or_else(|| default.into()))
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

fn has_value(args: &[String], name: &str) -> bool {
    args.iter()
        .position(|arg| arg == name)
        .is_some_and(|index| {
            args.get(index + 1)
                .is_some_and(|value| !value.starts_with("--"))
        })
}

fn default_machine_class() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string())
}

fn p95(values: &[f64]) -> f64 {
    percentile(values, 0.95)
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut values = values.to_vec();
    values.sort_by(|left, right| left.total_cmp(right));
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut values = values.to_vec();
    values.sort_by(|left, right| left.total_cmp(right));
    let index = ((values.len() as f64 * quantile).ceil() as usize).saturating_sub(1);
    values[index.min(values.len() - 1)]
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn calculate_speed_ratio(baseline_p95_ms: f64, candidate_p95_ms: f64) -> f64 {
    baseline_p95_ms / candidate_p95_ms.max(f64::EPSILON)
}

struct BenchmarkRuntimeDir {
    path: PathBuf,
    env_key: &'static str,
    previous_value: Option<OsString>,
}

impl BenchmarkRuntimeDir {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("benchmark_temp_clock_failed:{error}"))?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("batcave-benchmark-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).map_err(|error| {
            format!(
                "benchmark_temp_directory_create_failed path={} error={error}",
                path.display()
            )
        })?;

        #[cfg(windows)]
        let env_key = "LOCALAPPDATA";
        #[cfg(not(windows))]
        let env_key = "XDG_DATA_HOME";
        let previous_value = std::env::var_os(env_key);
        std::env::set_var(env_key, &path);

        Ok(Self {
            path,
            env_key,
            previous_value,
        })
    }
}

impl Drop for BenchmarkRuntimeDir {
    fn drop(&mut self) {
        if let Some(value) = &self.previous_value {
            std::env::set_var(self.env_key, value);
        } else {
            std::env::remove_var(self.env_key);
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BenchmarkConfig {
        BenchmarkConfig {
            platform: "test-os".to_string(),
            architecture: "test-arch".to_string(),
            machine_class: "test-machine".to_string(),
            workload_profile: "fixed-default".to_string(),
            warmup_ticks: 0,
            measured_ticks: 2,
            sleep_ms: 0,
            repeat_count: 1,
        }
    }

    fn baseline_json(overrides: &str) -> String {
        format!(
            r#"{{
                "format_version": 3,
                "host": "core",
                "measurement_origin": "runtime_state_refresh_and_json_serialization",
                "platform": "test-os",
                "architecture": "test-arch",
                "machine_class": "test-machine",
                "workload_profile": "fixed-default",
                "warmup_ticks": 0,
                "measured_ticks": 2,
                "sleep_ms": 0,
                "repeat_count": 1,
                "median_tick_p95_ms": 10.0{}
            }}"#,
            overrides
        )
    }

    fn write_baseline(payload: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "batcave-benchmark-baseline-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::write(&path, payload).expect("baseline fixture writes");
        path
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        assert_eq!(p95(&[1.0, 2.0, 3.0, 4.0]), 4.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0, 5.0]), 3.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn speed_ratio_is_baseline_over_candidate() {
        assert_eq!(calculate_speed_ratio(90.0, 100.0), 0.9);
        assert_eq!(calculate_speed_ratio(100.0, 50.0), 2.0);
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
    fn benchmark_rejects_zero_ticks_and_repeats() {
        let mut invalid = config();
        invalid.measured_ticks = 0;
        assert_eq!(
            validate_config(&invalid),
            Err("invalid_argument:--ticks:must_be_greater_than_zero".to_string())
        );
        invalid.measured_ticks = 1;
        invalid.repeat_count = 0;
        assert_eq!(
            validate_config(&invalid),
            Err("invalid_argument:--repeats:must_be_greater_than_zero".to_string())
        );
    }

    #[test]
    fn strict_gate_requires_baseline_or_ceiling() {
        let args = vec![
            "--benchmark".to_string(),
            "--strict".to_string(),
            "--warmup-ticks".to_string(),
            "0".to_string(),
            "--ticks".to_string(),
            "1".to_string(),
            "--sleep-ms".to_string(),
            "0".to_string(),
            "--repeats".to_string(),
            "1".to_string(),
        ];
        assert_eq!(
            run_benchmark_from_args(&args).expect_err("missing gate fails"),
            "invalid_benchmark_gate:strict_requires_baseline_or_max_p95_ms"
        );
    }

    #[test]
    fn speed_ratio_without_baseline_is_configuration_error() {
        let args = vec![
            "--benchmark".to_string(),
            "--min-speedup-multiplier".to_string(),
            "0.9".to_string(),
        ];
        assert_eq!(
            run_benchmark_from_args(&args).expect_err("missing baseline fails"),
            "invalid_benchmark_gate:min_speedup_requires_baseline"
        );
    }

    #[test]
    fn baseline_requires_v3_protocol_metadata() {
        let path = write_baseline(&baseline_json(""));
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];
        let parsed = parse_baseline(&args, &config()).expect("matching baseline parses");
        fs::remove_file(&path).expect("baseline fixture cleanup");
        assert_eq!(parsed.expect("baseline").median_tick_p95_ms, 10.0);
    }

    #[test]
    fn baseline_metadata_mismatch_fails() {
        let payload = baseline_json("").replace("test-machine", "other-machine");
        let path = write_baseline(&payload);
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];
        let error = parse_baseline(&args, &config()).expect_err("metadata mismatch fails");
        fs::remove_file(&path).expect("baseline fixture cleanup");
        assert!(error.contains("field=machine_class"));
    }
}
