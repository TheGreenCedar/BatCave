use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    cli_args,
    protocol::{encode_snapshot, release_identity, RuntimeReleaseIdentityV3},
    runtime_store::RuntimeState,
};

const FORMAT_VERSION: u32 = 4;
const HOST: &str = "core";
const MEASUREMENT_ORIGIN: &str = "owned_sampling_engine_refresh_and_protocol_serialization";
const EVIDENCE_SCOPE: &str = "core_runtime_host_only";
const LATENCY_GATE_METRIC: &str = "median_live_command_p95_ms";
const LIVE_COMMAND: &str = "refresh_now";
const COMMAND_TRANSPORT: &str = "in_process_bounded_channel";
const SERIALIZATION_SCOPE: &str = "runtime_protocol_v3_encode_and_json";
const DEFAULT_MIN_SPEED_RATIO: f64 = 0.90;
const MAX_APP_CPU_PERCENT: f64 = 25.0;
const MAX_APP_RSS_BYTES: u64 = 350 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkRepeat {
    collection_p95_ms: f64,
    publication_p95_ms: f64,
    serialization_p95_ms: f64,
    live_command_p95_ms: f64,
    peak_app_cpu_percent: f64,
    peak_app_rss_bytes: u64,
    samples_advanced: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct BenchmarkSummary {
    format_version: u32,
    release_identity: RuntimeReleaseIdentityV3,
    host: String,
    measurement_origin: String,
    evidence_scope: String,
    whole_app_measured: bool,
    live_command: String,
    command_transport: String,
    serialization_scope: String,
    latency_gate_metric: String,
    platform: String,
    architecture: String,
    machine_class: String,
    workload_profile: String,
    warmup_ticks: usize,
    measured_ticks: usize,
    inter_command_delay_ms: u64,
    repeat_count: usize,
    repeats: Vec<BenchmarkRepeat>,
    median_collection_p95_ms: f64,
    median_publication_p95_ms: f64,
    median_serialization_p95_ms: f64,
    median_live_command_p95_ms: f64,
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
    evidence_scope: String,
    whole_app_measured: bool,
    live_command: String,
    command_transport: String,
    serialization_scope: String,
    latency_gate_metric: String,
    platform: String,
    architecture: String,
    machine_class: String,
    workload_profile: String,
    warmup_ticks: usize,
    measured_ticks: usize,
    inter_command_delay_ms: u64,
    repeat_count: usize,
    median_live_command_p95_ms: f64,
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

    let runtime_dir = BenchmarkRuntimeDir::new()?;
    let state = RuntimeState::from_base_dir_manual(runtime_dir.path().to_path_buf())?;
    let measurement_result = (|| {
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
        Ok::<_, String>(repeats)
    })();
    let shutdown_result = state.shutdown();
    let repeats = match (measurement_result, shutdown_result) {
        (Ok(repeats), Ok(())) => repeats,
        (Err(error), Ok(())) => return Err(error),
        (Err(error), Err(shutdown_error)) => {
            return Err(format!(
                "{error}; benchmark_shutdown_failed:{shutdown_error}"
            ));
        }
        (Ok(_), Err(error)) => return Err(error),
    };

    let median_collection_p95_ms = median(
        &repeats
            .iter()
            .map(|repeat| repeat.collection_p95_ms)
            .collect::<Vec<_>>(),
    );
    let median_publication_p95_ms = median(
        &repeats
            .iter()
            .map(|repeat| repeat.publication_p95_ms)
            .collect::<Vec<_>>(),
    );
    let median_serialization_p95_ms = median(
        &repeats
            .iter()
            .map(|repeat| repeat.serialization_p95_ms)
            .collect::<Vec<_>>(),
    );
    let median_live_command_p95_ms = median(
        &repeats
            .iter()
            .map(|repeat| repeat.live_command_p95_ms)
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
        .map(|maximum| median_live_command_p95_ms <= maximum)
        .unwrap_or(true);
    let speed_ratio = baseline.as_ref().map(|baseline| {
        calculate_speed_ratio(
            baseline.median_live_command_p95_ms,
            median_live_command_p95_ms,
        )
    });
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
        release_identity: release_identity(),
        host: HOST.to_string(),
        measurement_origin: MEASUREMENT_ORIGIN.to_string(),
        evidence_scope: EVIDENCE_SCOPE.to_string(),
        whole_app_measured: false,
        live_command: LIVE_COMMAND.to_string(),
        command_transport: COMMAND_TRANSPORT.to_string(),
        serialization_scope: SERIALIZATION_SCOPE.to_string(),
        latency_gate_metric: LATENCY_GATE_METRIC.to_string(),
        platform: config.platform,
        architecture: config.architecture,
        machine_class: config.machine_class,
        workload_profile: config.workload_profile,
        warmup_ticks: config.warmup_ticks,
        measured_ticks: config.measured_ticks,
        inter_command_delay_ms: config.sleep_ms,
        repeat_count: config.repeat_count,
        repeats,
        median_collection_p95_ms: round1(median_collection_p95_ms),
        median_publication_p95_ms: round1(median_publication_p95_ms),
        median_serialization_p95_ms: round1(median_serialization_p95_ms),
        median_live_command_p95_ms: round1(median_live_command_p95_ms),
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
    let mut collection_durations = Vec::with_capacity(ticks);
    let mut publication_durations = Vec::with_capacity(ticks);
    let mut serialization_durations = Vec::with_capacity(ticks);
    let mut live_command_durations = Vec::with_capacity(ticks);
    let mut peak_app_cpu_percent = 0.0_f64;
    let mut peak_app_rss_bytes = 0_u64;
    let mut previous_sample_seq = state.snapshot()?.sample_seq;
    let mut samples_advanced = true;

    for _ in 0..ticks {
        let started = Instant::now();
        let measurement = state.refresh_now_measured()?;
        live_command_durations.push(started.elapsed().as_secs_f64() * 1000.0);
        collection_durations.push(measurement.collection_latency_ms);
        publication_durations.push(measurement.publication_latency_ms);
        let snapshot = measurement.snapshot;
        let serialization_started = Instant::now();
        let envelope = encode_snapshot(snapshot.clone())
            .map_err(|error| format!("benchmark_protocol_encode_failed:{error}"))?;
        serde_json::to_vec(&envelope)
            .map_err(|error| format!("benchmark_snapshot_serialize_failed:{error}"))?;
        serialization_durations.push(serialization_started.elapsed().as_secs_f64() * 1000.0);
        peak_app_cpu_percent = peak_app_cpu_percent.max(snapshot.health.app_cpu_percent);
        peak_app_rss_bytes = peak_app_rss_bytes.max(snapshot.health.app_rss_bytes);
        samples_advanced &= snapshot.sample_seq == previous_sample_seq.saturating_add(1);
        previous_sample_seq = snapshot.sample_seq;

        if sleep_ms > 0 {
            thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    Ok(BenchmarkRepeat {
        collection_p95_ms: round1(p95(&collection_durations)),
        publication_p95_ms: round1(p95(&publication_durations)),
        serialization_p95_ms: round1(p95(&serialization_durations)),
        live_command_p95_ms: round1(p95(&live_command_durations)),
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
    let value = serde_json::from_str::<serde_json::Value>(&payload)
        .map_err(|error| format!("baseline_json_parse_failed path={path} error={error}"))?;
    let actual_format_version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            format!("baseline_json_parse_failed path={path} error=missing format_version")
        })?;
    if actual_format_version != u64::from(FORMAT_VERSION) {
        return Err(format!(
            "baseline_metadata_mismatch field=format_version expected={} actual={}",
            FORMAT_VERSION, actual_format_version
        ));
    }
    let baseline = serde_json::from_value::<BenchmarkBaseline>(value)
        .map_err(|error| format!("baseline_json_parse_failed path={path} error={error}"))?;
    require_number("format_version", FORMAT_VERSION, baseline.format_version)?;
    require_metadata("host", HOST, &baseline.host)?;
    require_metadata(
        "measurement_origin",
        MEASUREMENT_ORIGIN,
        &baseline.measurement_origin,
    )?;
    require_metadata("evidence_scope", EVIDENCE_SCOPE, &baseline.evidence_scope)?;
    require_number("whole_app_measured", false, baseline.whole_app_measured)?;
    require_metadata("live_command", LIVE_COMMAND, &baseline.live_command)?;
    require_metadata(
        "command_transport",
        COMMAND_TRANSPORT,
        &baseline.command_transport,
    )?;
    require_metadata(
        "serialization_scope",
        SERIALIZATION_SCOPE,
        &baseline.serialization_scope,
    )?;
    require_metadata(
        "latency_gate_metric",
        LATENCY_GATE_METRIC,
        &baseline.latency_gate_metric,
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
    require_number(
        "inter_command_delay_ms",
        expected.sleep_ms,
        baseline.inter_command_delay_ms,
    )?;
    require_number("repeat_count", expected.repeat_count, baseline.repeat_count)?;
    validate_positive_finite(
        "baseline.median_live_command_p95_ms",
        Some(baseline.median_live_command_p95_ms),
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

        Ok(Self { path })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for BenchmarkRuntimeDir {
    fn drop(&mut self) {
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
                "format_version": 4,
                "host": "core",
                "measurement_origin": "owned_sampling_engine_refresh_and_protocol_serialization",
                "evidence_scope": "core_runtime_host_only",
                "whole_app_measured": false,
                "live_command": "refresh_now",
                "command_transport": "in_process_bounded_channel",
                "serialization_scope": "runtime_protocol_v3_encode_and_json",
                "latency_gate_metric": "median_live_command_p95_ms",
                "platform": "test-os",
                "architecture": "test-arch",
                "machine_class": "test-machine",
                "workload_profile": "fixed-default",
                "warmup_ticks": 0,
                "measured_ticks": 2,
                "inter_command_delay_ms": 0,
                "repeat_count": 1,
                "median_live_command_p95_ms": 10.0{overrides}
            }}"#
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
    fn apple_arm64_normalizes_to_contract_architecture() {
        assert_eq!(canonical_architecture("arm64"), "aarch64");
        assert_eq!(canonical_architecture("aarch64"), "aarch64");
    }

    #[test]
    fn benchmark_runtime_directory_does_not_mutate_data_environment() {
        let before_xdg = std::env::var_os("XDG_DATA_HOME");
        let before_local = std::env::var_os("LOCALAPPDATA");
        let directory = BenchmarkRuntimeDir::new().expect("benchmark directory");
        assert!(directory.path().is_dir());
        assert_eq!(std::env::var_os("XDG_DATA_HOME"), before_xdg);
        assert_eq!(std::env::var_os("LOCALAPPDATA"), before_local);
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
    fn baseline_requires_v4_owned_engine_metadata() {
        let path = write_baseline(&baseline_json(""));
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];
        let parsed = parse_baseline(&args, &config()).expect("matching baseline parses");
        fs::remove_file(&path).expect("baseline fixture cleanup");
        assert_eq!(parsed.expect("baseline").median_live_command_p95_ms, 10.0);
    }

    #[test]
    fn v3_baseline_is_rejected_with_canonical_format_mismatch() {
        let payload = baseline_json("").replace("\"format_version\": 4", "\"format_version\": 3");
        let path = write_baseline(&payload);
        let args = vec![
            "--benchmark".to_string(),
            "--baseline-json".to_string(),
            path.display().to_string(),
        ];
        let error = parse_baseline(&args, &config()).expect_err("v3 baseline fails");
        fs::remove_file(&path).expect("baseline fixture cleanup");
        assert_eq!(
            error,
            "baseline_metadata_mismatch field=format_version expected=4 actual=3"
        );
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

    #[test]
    fn owned_engine_baseline_metadata_mismatches_fail() {
        for (field, payload) in [
            (
                "measurement_origin",
                baseline_json("").replace(MEASUREMENT_ORIGIN, "other-origin"),
            ),
            (
                "evidence_scope",
                baseline_json("").replace(EVIDENCE_SCOPE, "whole-app"),
            ),
            (
                "latency_gate_metric",
                baseline_json("").replace(
                    &format!("\"latency_gate_metric\": \"{LATENCY_GATE_METRIC}\""),
                    "\"latency_gate_metric\": \"other-p95\"",
                ),
            ),
        ] {
            let path = write_baseline(&payload);
            let args = vec![
                "--benchmark".to_string(),
                "--baseline-json".to_string(),
                path.display().to_string(),
            ];
            let error = parse_baseline(&args, &config()).expect_err("metadata mismatch fails");
            fs::remove_file(&path).expect("baseline fixture cleanup");
            assert!(error.contains(&format!("field={field}")), "{error}");
        }
    }

    #[test]
    fn v4_repeat_schema_has_disjoint_phase_metrics() {
        let value = serde_json::to_value(BenchmarkRepeat {
            collection_p95_ms: 1.0,
            publication_p95_ms: 2.0,
            serialization_p95_ms: 3.0,
            live_command_p95_ms: 4.0,
            peak_app_cpu_percent: 5.0,
            peak_app_rss_bytes: 6,
            samples_advanced: true,
        })
        .expect("repeat serializes");
        for field in [
            "collection_p95_ms",
            "publication_p95_ms",
            "serialization_p95_ms",
            "live_command_p95_ms",
        ] {
            assert!(value.get(field).is_some(), "missing {field}");
        }
        assert!(value.get("tick_p95_ms").is_none());
        assert!(value.get("median_tick_p95_ms").is_none());
    }

    #[test]
    fn manual_benchmark_refresh_advances_once_and_joins() {
        let directory = BenchmarkRuntimeDir::new().expect("benchmark directory");
        let state = RuntimeState::from_base_dir_manual(directory.path().to_path_buf())
            .expect("engine starts");
        let before = state.snapshot().expect("initial snapshot").sample_seq;
        let repeat = measure_window(&state, 1, 0).expect("one measured refresh");
        let after = state.snapshot().expect("published snapshot").sample_seq;
        assert!(repeat.samples_advanced);
        assert_eq!(after, before + 1);
        state.shutdown().expect("engine joins");
        assert_eq!(
            state
                .refresh_now()
                .expect_err("stopped engine rejects refresh"),
            "runtime_engine_shutting_down"
        );
    }
}
