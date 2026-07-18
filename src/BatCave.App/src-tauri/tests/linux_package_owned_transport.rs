use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fmt;

const MAX_PACKAGED_BENCHMARK_BYTES: usize = 4096;
const PACKAGED_BENCHMARK_MACHINE_CLASS: &str = "owned-package-payload";
const PACKAGED_BENCHMARK_WORKLOAD_PROFILE: &str = "fixed-owned-package-launch";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostSupport {
    LinuxPackageTransport,
    UnsupportedHost,
}

fn host_support() -> HostSupport {
    if cfg!(target_os = "linux") {
        HostSupport::LinuxPackageTransport
    } else {
        HostSupport::UnsupportedHost
    }
}

#[test]
fn package_transport_probe_is_explicitly_linux_only() {
    if cfg!(target_os = "linux") {
        assert_eq!(host_support(), HostSupport::LinuxPackageTransport);
    } else {
        assert_eq!(host_support(), HostSupport::UnsupportedHost);
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PackagedBenchmarkObservation {
    app_version: String,
    source_commit_sha: Option<String>,
}

struct DuplicateRejectingValue(Value);

impl<'de> Deserialize<'de> for DuplicateRejectingValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateRejectingValueVisitor)
    }
}

struct DuplicateRejectingValueVisitor;

impl<'de> Visitor<'de> for DuplicateRejectingValueVisitor {
    type Value = DuplicateRejectingValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("one JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(DuplicateRejectingValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DuplicateRejectingValue(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateRejectingValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        DuplicateRejectingValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(DuplicateRejectingValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(DuplicateRejectingValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate object key {key}")));
            }
            let DuplicateRejectingValue(value) = map.next_value()?;
            values.insert(key, value);
        }
        Ok(DuplicateRejectingValue(Value::Object(values)))
    }
}

fn parse_json_without_duplicate_keys(bytes: &[u8]) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let DuplicateRejectingValue(value) = DuplicateRejectingValue::deserialize(&mut deserializer)
        .map_err(|error| {
            format!("packaged benchmark output was not one closed JSON value: {error}")
        })?;
    deserializer
        .end()
        .map_err(|_| "packaged benchmark output was not one JSON value".to_string())?;
    Ok(value)
}

fn expected_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" | "amd64" | "x64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        "x86" | "i686" | "i586" => "x86",
        _ => "unknown",
    }
}

fn expected_source_commit_sha() -> Option<&'static str> {
    option_env!("BATCAVE_SOURCE_COMMIT_SHA").filter(|value| !value.is_empty())
}

fn exact_keys(object: &Map<String, Value>, expected: &[&str], label: &str) -> Result<(), String> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("packaged benchmark {label} keys were not exact"))
    }
}

fn exact_string(object: &Map<String, Value>, field: &str, expected: &str) -> Result<(), String> {
    if object.get(field).and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(format!("packaged benchmark field {field} did not match"))
    }
}

fn exact_u64(object: &Map<String, Value>, field: &str, expected: u64) -> Result<(), String> {
    if object.get(field).and_then(Value::as_u64) == Some(expected) {
        Ok(())
    } else {
        Err(format!("packaged benchmark field {field} did not match"))
    }
}

fn exact_bool(object: &Map<String, Value>, field: &str, expected: bool) -> Result<(), String> {
    if object.get(field).and_then(Value::as_bool) == Some(expected) {
        Ok(())
    } else {
        Err(format!("packaged benchmark field {field} did not match"))
    }
}

fn require_nonnegative_number(object: &Map<String, Value>, field: &str) -> Result<(), String> {
    if object
        .get(field)
        .and_then(Value::as_f64)
        .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        Ok(())
    } else {
        Err(format!(
            "packaged benchmark field {field} was not a nonnegative number"
        ))
    }
}

fn parse_packaged_benchmark(
    stdout: &[u8],
    output_bounded: bool,
) -> Result<PackagedBenchmarkObservation, String> {
    const SUMMARY_KEYS: [&str; 32] = [
        "format_version",
        "release_identity",
        "host",
        "measurement_origin",
        "evidence_scope",
        "whole_app_measured",
        "live_command",
        "command_transport",
        "serialization_scope",
        "latency_gate_metric",
        "platform",
        "architecture",
        "machine_class",
        "workload_profile",
        "warmup_ticks",
        "measured_ticks",
        "inter_command_delay_ms",
        "repeat_count",
        "repeats",
        "median_collection_p95_ms",
        "median_publication_p95_ms",
        "median_serialization_p95_ms",
        "median_live_command_p95_ms",
        "peak_app_cpu_percent",
        "peak_app_rss_bytes",
        "max_p95_passed",
        "baseline_metadata_matched",
        "speed_ratio",
        "speed_ratio_passed",
        "resource_budget_passed",
        "sample_quality_passed",
        "strict_passed",
    ];
    const REPEAT_KEYS: [&str; 7] = [
        "collection_p95_ms",
        "publication_p95_ms",
        "serialization_p95_ms",
        "live_command_p95_ms",
        "peak_app_cpu_percent",
        "peak_app_rss_bytes",
        "samples_advanced",
    ];

    if !output_bounded || stdout.len() > MAX_PACKAGED_BENCHMARK_BYTES {
        return Err("packaged benchmark output exceeded the fixed bound".to_string());
    }
    let value = parse_json_without_duplicate_keys(stdout)?;
    let summary = value
        .as_object()
        .ok_or_else(|| "packaged benchmark output was not an object".to_string())?;
    exact_keys(summary, &SUMMARY_KEYS, "summary")?;
    exact_u64(summary, "format_version", 4)?;
    exact_string(summary, "host", "core")?;
    exact_string(
        summary,
        "measurement_origin",
        "owned_sampling_engine_refresh_and_protocol_serialization",
    )?;
    exact_string(summary, "evidence_scope", "core_runtime_host_only")?;
    exact_bool(summary, "whole_app_measured", false)?;
    exact_string(summary, "live_command", "refresh_now")?;
    exact_string(summary, "command_transport", "in_process_bounded_channel")?;
    exact_string(
        summary,
        "serialization_scope",
        "runtime_protocol_v3_encode_and_json",
    )?;
    exact_string(summary, "latency_gate_metric", "median_live_command_p95_ms")?;
    exact_string(summary, "platform", "linux")?;
    exact_string(summary, "architecture", expected_architecture())?;
    exact_string(summary, "machine_class", PACKAGED_BENCHMARK_MACHINE_CLASS)?;
    exact_string(
        summary,
        "workload_profile",
        PACKAGED_BENCHMARK_WORKLOAD_PROFILE,
    )?;
    exact_u64(summary, "warmup_ticks", 0)?;
    exact_u64(summary, "measured_ticks", 1)?;
    exact_u64(summary, "inter_command_delay_ms", 0)?;
    exact_u64(summary, "repeat_count", 1)?;

    for field in [
        "median_collection_p95_ms",
        "median_publication_p95_ms",
        "median_serialization_p95_ms",
        "median_live_command_p95_ms",
        "peak_app_cpu_percent",
        "peak_app_rss_bytes",
    ] {
        require_nonnegative_number(summary, field)?;
    }
    exact_bool(summary, "max_p95_passed", true)?;
    exact_bool(summary, "baseline_metadata_matched", false)?;
    if !summary.get("speed_ratio").is_some_and(Value::is_null) {
        return Err("packaged benchmark field speed_ratio did not match".to_string());
    }
    exact_bool(summary, "speed_ratio_passed", true)?;
    if !summary
        .get("resource_budget_passed")
        .is_some_and(Value::is_boolean)
    {
        return Err("packaged benchmark field resource_budget_passed was not boolean".to_string());
    }
    exact_bool(summary, "sample_quality_passed", true)?;
    exact_bool(summary, "strict_passed", true)?;

    let repeats = summary
        .get("repeats")
        .and_then(Value::as_array)
        .filter(|repeats| repeats.len() == 1)
        .ok_or_else(|| "packaged benchmark repeats were not exact".to_string())?;
    let repeat = repeats[0]
        .as_object()
        .ok_or_else(|| "packaged benchmark repeat was not an object".to_string())?;
    exact_keys(repeat, &REPEAT_KEYS, "repeat")?;
    for field in [
        "collection_p95_ms",
        "publication_p95_ms",
        "serialization_p95_ms",
        "live_command_p95_ms",
        "peak_app_cpu_percent",
        "peak_app_rss_bytes",
    ] {
        require_nonnegative_number(repeat, field)?;
    }
    exact_bool(repeat, "samples_advanced", true)?;

    let identity = summary
        .get("release_identity")
        .and_then(Value::as_object)
        .ok_or_else(|| "packaged benchmark release identity was not an object".to_string())?;
    exact_keys(
        identity,
        &["app_version", "source_commit_sha"],
        "release identity",
    )?;
    exact_string(identity, "app_version", env!("CARGO_PKG_VERSION"))?;
    let source_commit_sha = match (
        identity.get("source_commit_sha"),
        expected_source_commit_sha(),
    ) {
        (Some(Value::Null), None) => None,
        (Some(Value::String(actual)), Some(expected)) if actual == expected => Some(actual.clone()),
        _ => return Err("packaged benchmark source identity did not match".to_string()),
    };

    Ok(PackagedBenchmarkObservation {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        source_commit_sha,
    })
}

fn packaged_benchmark_fixture() -> Value {
    serde_json::json!({
        "format_version": 4,
        "release_identity": {
            "app_version": env!("CARGO_PKG_VERSION"),
            "source_commit_sha": expected_source_commit_sha(),
        },
        "host": "core",
        "measurement_origin": "owned_sampling_engine_refresh_and_protocol_serialization",
        "evidence_scope": "core_runtime_host_only",
        "whole_app_measured": false,
        "live_command": "refresh_now",
        "command_transport": "in_process_bounded_channel",
        "serialization_scope": "runtime_protocol_v3_encode_and_json",
        "latency_gate_metric": "median_live_command_p95_ms",
        "platform": "linux",
        "architecture": expected_architecture(),
        "machine_class": PACKAGED_BENCHMARK_MACHINE_CLASS,
        "workload_profile": PACKAGED_BENCHMARK_WORKLOAD_PROFILE,
        "warmup_ticks": 0,
        "measured_ticks": 1,
        "inter_command_delay_ms": 0,
        "repeat_count": 1,
        "repeats": [{
            "collection_p95_ms": 1.0,
            "publication_p95_ms": 1.0,
            "serialization_p95_ms": 1.0,
            "live_command_p95_ms": 1.0,
            "peak_app_cpu_percent": 1.0,
            "peak_app_rss_bytes": 1,
            "samples_advanced": true,
        }],
        "median_collection_p95_ms": 1.0,
        "median_publication_p95_ms": 1.0,
        "median_serialization_p95_ms": 1.0,
        "median_live_command_p95_ms": 1.0,
        "peak_app_cpu_percent": 1.0,
        "peak_app_rss_bytes": 1,
        "max_p95_passed": true,
        "baseline_metadata_matched": false,
        "speed_ratio": null,
        "speed_ratio_passed": true,
        "resource_budget_passed": true,
        "sample_quality_passed": true,
        "strict_passed": true,
    })
}

#[test]
fn packaged_benchmark_parser_accepts_only_the_closed_source_observation() {
    let fixture = serde_json::to_vec_pretty(&packaged_benchmark_fixture()).unwrap();
    assert_eq!(
        parse_packaged_benchmark(&fixture, true).unwrap(),
        PackagedBenchmarkObservation {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            source_commit_sha: expected_source_commit_sha().map(str::to_string),
        }
    );
}

#[test]
fn packaged_benchmark_parser_rejects_hostile_or_contradictory_output() {
    let canonical = packaged_benchmark_fixture();
    let canonical_bytes = serde_json::to_vec(&canonical).unwrap();
    assert!(parse_packaged_benchmark(&canonical_bytes, false)
        .unwrap_err()
        .contains("exceeded the fixed bound"));
    assert!(
        parse_packaged_benchmark(&vec![b' '; MAX_PACKAGED_BENCHMARK_BYTES + 1], true)
            .unwrap_err()
            .contains("exceeded the fixed bound")
    );

    let mut multiple = canonical_bytes.clone();
    multiple.extend_from_slice(&canonical_bytes);
    assert!(parse_packaged_benchmark(&multiple, true)
        .unwrap_err()
        .contains("not one JSON value"));

    for (label, hostile) in [
        ("extra key", {
            let mut value = canonical.clone();
            value["caller_status"] = Value::String("passed".to_string());
            value
        }),
        ("wrong version", {
            let mut value = canonical.clone();
            value["release_identity"]["app_version"] = Value::String("9.9.9".to_string());
            value
        }),
        ("wrong platform", {
            let mut value = canonical.clone();
            value["platform"] = Value::String("fixture".to_string());
            value
        }),
        ("negative metric", {
            let mut value = canonical.clone();
            value["median_live_command_p95_ms"] = serde_json::json!(-1.0);
            value
        }),
        ("non-advancing repeat", {
            let mut value = canonical.clone();
            value["repeats"][0]["samples_advanced"] = Value::Bool(false);
            value
        }),
        ("contradictory summary", {
            let mut value = canonical.clone();
            value["sample_quality_passed"] = Value::Bool(false);
            value
        }),
    ] {
        let error = parse_packaged_benchmark(&serde_json::to_vec(&hostile).unwrap(), true)
            .expect_err(label);
        assert!(error.contains("packaged benchmark"), "{label}: {error}");
    }
}

#[test]
fn packaged_benchmark_parser_rejects_duplicate_required_and_extra_keys() {
    let canonical = serde_json::to_string(&packaged_benchmark_fixture()).unwrap();
    for (label, prefix) in [
        ("duplicate required key", r#"{"format_version":4,"#),
        (
            "duplicate extra key",
            r#"{"caller_status":"passed","caller_status":"passed","#,
        ),
    ] {
        let hostile = format!("{prefix}{}", &canonical[1..]);
        let error = parse_packaged_benchmark(hostile.as_bytes(), true).expect_err(label);
        assert!(error.contains("duplicate object key"), "{label}: {error}");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::ffi::CString;
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    const CONSUMER_FD: RawFd = 198;
    const APPIMAGE_LAUNCHER_MODE: &str = "BATCAVE_APPIMAGE_TRANSPORT_LAUNCHER";
    const HOSTILE_PROBE_MODE: &str = "BATCAVE_LINUX_PACKAGE_HOSTILE_PROBE";
    const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
    const MAX_OUTPUT_BYTES: usize = 4096;
    const MAX_DRAIN_BYTES_PER_POLL: usize = 64 * 1024;
    const STEP_TIMEOUT: Duration = Duration::from_secs(120);
    const TERMINATION_GRACE: Duration = Duration::from_millis(500);
    const SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(5);
    static PROBE_LOCK: Mutex<()> = Mutex::new(());
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PackageKind {
        Deb,
        AppImage,
    }

    impl PackageKind {
        fn directory(self) -> &'static str {
            match self {
                Self::Deb => "deb",
                Self::AppImage => "appimage",
            }
        }

        fn suffix(self) -> &'static str {
            match self {
                Self::Deb => ".deb",
                Self::AppImage => ".AppImage",
            }
        }

        fn memfd_name(self) -> &'static str {
            match self {
                Self::Deb => "batcave-linux-deb-transport",
                Self::AppImage => "batcave-linux-appimage-transport",
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FixedProbe {
        DebExtract,
        DebPayloadBenchmark,
        AppImageDescriptorPath,
        AppImageExecveat,
        AppImageFexecve,
        AppImagePayloadBenchmark,
    }

    impl FixedProbe {
        fn package_kind(self) -> PackageKind {
            match self {
                Self::DebExtract | Self::DebPayloadBenchmark => PackageKind::Deb,
                Self::AppImageDescriptorPath
                | Self::AppImageExecveat
                | Self::AppImageFexecve
                | Self::AppImagePayloadBenchmark => PackageKind::AppImage,
            }
        }

        fn label(self) -> &'static str {
            match self {
                Self::DebExtract => "deb_extract",
                Self::DebPayloadBenchmark => "deb_payload_benchmark",
                Self::AppImageDescriptorPath => "appimage_descriptor_path",
                Self::AppImageExecveat => "appimage_execveat",
                Self::AppImageFexecve => "appimage_fexecve",
                Self::AppImagePayloadBenchmark => "appimage_payload_benchmark",
            }
        }

        fn launches_payload(self) -> bool {
            matches!(
                self,
                Self::DebPayloadBenchmark | Self::AppImagePayloadBenchmark
            )
        }
    }

    #[derive(Debug)]
    struct TransportOutcome {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        output_bounded: bool,
        process_group_settled: bool,
        source_kind: &'static str,
        public_artifact_verified: bool,
        native_proven: bool,
        release_evidence_emitted: bool,
    }

    struct OwnedArtifact {
        descriptor: File,
        size: u64,
        sha256: [u8; 32],
        device: u64,
        inode: u64,
        required_seals: i32,
    }

    impl OwnedArtifact {
        fn validate(&self) -> Result<(), String> {
            let metadata = self
                .descriptor
                .metadata()
                .map_err(|error| format!("owned descriptor metadata failed: {error}"))?;
            if !metadata.is_file()
                || metadata.len() != self.size
                || metadata.dev() != self.device
                || metadata.ino() != self.inode
            {
                return Err("owned descriptor identity changed".to_string());
            }

            let flags = unsafe { libc::fcntl(self.descriptor.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY {
                return Err("owned descriptor is not read-only".to_string());
            }
            let seals = unsafe { libc::fcntl(self.descriptor.as_raw_fd(), libc::F_GET_SEALS) };
            if seals < 0 || seals & self.required_seals != self.required_seals {
                return Err("owned descriptor seals changed".to_string());
            }

            let mut reader = self
                .descriptor
                .try_clone()
                .map_err(|error| format!("owned descriptor clone failed: {error}"))?;
            reader
                .seek(SeekFrom::Start(0))
                .map_err(|error| format!("owned descriptor rewind failed: {error}"))?;
            let mut hasher = Sha256::new();
            let mut copied = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = reader
                    .read(&mut buffer)
                    .map_err(|error| format!("owned descriptor rehash failed: {error}"))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                copied = copied.saturating_add(read as u64);
            }
            if copied != self.size || hasher.finalize().as_slice() != self.sha256 {
                return Err("owned descriptor bytes changed".to_string());
            }
            Ok(())
        }
    }

    struct PrivateRoot {
        path: PathBuf,
        removed: bool,
        fail_cleanup_once: bool,
    }

    impl PrivateRoot {
        fn create(label: &str) -> Result<Self, String> {
            let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "batcave-linux-package-transport-{}-{sequence}-{label}",
                std::process::id()
            ));
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(&path)
                .map_err(|error| format!("private root creation failed: {error}"))?;
            Ok(Self {
                path,
                removed: false,
                fail_cleanup_once: false,
            })
        }

        fn cleanup(&mut self) -> Result<(), String> {
            if self.fail_cleanup_once {
                self.fail_cleanup_once = false;
                return Err("private root cleanup failed: injected cleanup failure".to_string());
            }
            fs::remove_dir_all(&self.path)
                .map_err(|error| format!("private root cleanup failed: {error}"))?;
            self.removed = true;
            Ok(())
        }

        fn finish<T>(mut self, operation: Result<T, String>) -> Result<T, String> {
            let cleanup = match self.cleanup() {
                Ok(()) => Ok(()),
                Err(first) => match self.cleanup() {
                    Ok(()) => Err(format!("{first}; explicit cleanup retry removed residue")),
                    Err(retry) => Err(format!("{first}; cleanup retry also failed: {retry}")),
                },
            };
            match (operation, cleanup) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(operation), Ok(())) => Err(operation),
                (Ok(_), Err(cleanup)) => Err(format!("cleanup boundary: {cleanup}")),
                (Err(operation), Err(cleanup)) => {
                    Err(format!("{operation}; cleanup boundary: {cleanup}"))
                }
            }
        }
    }

    impl Drop for PrivateRoot {
        fn drop(&mut self) {
            if !self.removed {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    #[derive(Debug)]
    struct BoundedOutput {
        bytes: Vec<u8>,
        overflowed: bool,
    }

    struct OutputPipe<R> {
        reader: R,
        output: BoundedOutput,
        eof: bool,
    }

    impl<R: Read + AsRawFd> OutputPipe<R> {
        fn new(reader: R) -> Result<Self, String> {
            let descriptor = reader.as_raw_fd();
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(format!(
                    "package output nonblocking setup failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self {
                reader,
                output: BoundedOutput {
                    bytes: Vec::with_capacity(MAX_OUTPUT_BYTES),
                    overflowed: false,
                },
                eof: false,
            })
        }

        fn drain(&mut self) -> Result<(), String> {
            if self.eof {
                return Ok(());
            }
            let mut buffer = [0_u8; 1024];
            let mut drained = 0_usize;
            loop {
                match self.reader.read(&mut buffer) {
                    Ok(0) => {
                        self.eof = true;
                        return Ok(());
                    }
                    Ok(read) => {
                        drained = drained.saturating_add(read);
                        let remaining = MAX_OUTPUT_BYTES.saturating_sub(self.output.bytes.len());
                        let retained = remaining.min(read);
                        self.output.bytes.extend_from_slice(&buffer[..retained]);
                        self.output.overflowed |= retained < read;
                        if drained >= MAX_DRAIN_BYTES_PER_POLL {
                            return Ok(());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                    Err(error) => return Err(format!("package output read failed: {error}")),
                }
            }
        }
    }

    struct ProbeProcess {
        child: Child,
        process_group: libc::pid_t,
        stdout: OutputPipe<ChildStdout>,
        stderr: OutputPipe<ChildStderr>,
        baseline_children: BTreeSet<libc::pid_t>,
        settled: bool,
        fail_cleanup_unsettled_once: bool,
    }

    impl ProbeProcess {
        fn drain_output(&mut self) -> Result<(), String> {
            self.stdout.drain()?;
            self.stderr.drain()?;
            Ok(())
        }

        fn output_settled(&self) -> bool {
            self.stdout.eof && self.stderr.eof
        }

        fn cleanup(&mut self) -> Result<(), String> {
            if self.fail_cleanup_unsettled_once {
                self.fail_cleanup_unsettled_once = false;
                return Err(
                    "package process cleanup retained injected unsettled ownership".to_string(),
                );
            }
            let deadline = Instant::now() + TERMINATION_GRACE + SETTLEMENT_TIMEOUT;
            let original_child = self.child.id() as libc::pid_t;
            let mut failures = Vec::new();
            if let Err(error) =
                terminate_process_group(self.process_group, &mut self.child, deadline)
            {
                failures.push(error);
            }
            if let Err(error) =
                terminate_adopted_descendants(&self.baseline_children, original_child, deadline)
            {
                failures.push(error);
            }
            let group_settled = match settle_process_group(
                self.process_group,
                &mut self.child,
                &mut self.stdout,
                &mut self.stderr,
                deadline,
            ) {
                Ok(settled) => settled,
                Err(error) => {
                    failures.push(error);
                    false
                }
            };
            let descendants_settled =
                match settle_adopted_descendants(&self.baseline_children, original_child, deadline)
                {
                    Ok(settled) => settled,
                    Err(error) => {
                        failures.push(error);
                        false
                    }
                };
            let output_settled = match drain_output_until(self, deadline) {
                Ok(settled) => settled,
                Err(error) => {
                    failures.push(error);
                    false
                }
            };
            self.settled = group_settled && descendants_settled && output_settled;
            if self.settled && failures.is_empty() {
                Ok(())
            } else {
                if !self.settled {
                    failures.push(
                        "package process cleanup did not settle all owned resources".to_string(),
                    );
                }
                Err(failures.join("; "))
            }
        }
    }

    impl Drop for ProbeProcess {
        fn drop(&mut self) {
            if self.settled {
                return;
            }
            let _ = self.cleanup();
        }
    }

    struct SubreaperGuard {
        previous: i32,
    }

    impl SubreaperGuard {
        fn enable() -> Result<Self, String> {
            let mut previous = 0;
            if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut previous) } != 0 {
                return Err(format!(
                    "subreaper state read failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if previous == 0 && unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
                return Err(format!(
                    "subreaper enable failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { previous })
        }
    }

    impl Drop for SubreaperGuard {
        fn drop(&mut self) {
            if self.previous == 0 {
                let _ = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0) };
            }
        }
    }

    struct UnsettledProbe {
        diagnostic: String,
        process: ProbeProcess,
        subreaper: SubreaperGuard,
    }

    enum ProbeFailure {
        Settled(String),
        Unsettled(Box<UnsettledProbe>),
    }

    impl ProbeFailure {
        fn after_cleanup(
            operation: String,
            mut process: ProbeProcess,
            subreaper: SubreaperGuard,
        ) -> Self {
            match process.cleanup() {
                Ok(()) => Self::Settled(operation),
                Err(cleanup) => {
                    let diagnostic = combine_operation_cleanup(operation, Err(cleanup));
                    if process.settled {
                        Self::Settled(diagnostic)
                    } else {
                        Self::Unsettled(Box::new(UnsettledProbe {
                            diagnostic,
                            process,
                            subreaper,
                        }))
                    }
                }
            }
        }

        fn diagnostic(&self) -> &str {
            match self {
                Self::Settled(diagnostic) => diagnostic,
                Self::Unsettled(probe) => &probe.diagnostic,
            }
        }

        fn contains(&self, pattern: &str) -> bool {
            self.diagnostic().contains(pattern)
        }
    }

    impl From<String> for ProbeFailure {
        fn from(error: String) -> Self {
            Self::Settled(error)
        }
    }

    impl std::fmt::Debug for ProbeFailure {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("ProbeFailure")
                .field("diagnostic", &self.diagnostic())
                .field("ownership_retained", &matches!(self, Self::Unsettled(_)))
                .finish()
        }
    }

    impl std::fmt::Display for ProbeFailure {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.diagnostic())
        }
    }

    struct RetainedOperation {
        artifact: Option<OwnedArtifact>,
        root: Option<PrivateRoot>,
        process: Option<ProbeProcess>,
        subreaper: Option<SubreaperGuard>,
        diagnostic: String,
    }

    impl RetainedOperation {
        fn owns_all_authority(&self) -> bool {
            self.artifact.is_some()
                && self.root.is_some()
                && self.process.is_some()
                && self.subreaper.is_some()
        }

        fn artifact_valid(&self) -> bool {
            self.artifact
                .as_ref()
                .is_some_and(|artifact| artifact.validate().is_ok())
        }

        fn root_path(&self) -> Option<&Path> {
            self.root.as_ref().map(|root| root.path.as_path())
        }

        fn process_unsettled(&self) -> bool {
            self.process
                .as_ref()
                .is_some_and(|process| !process.settled)
        }

        fn recover(&mut self) -> Result<(), String> {
            if !self.owns_all_authority() {
                return if self.artifact.is_none()
                    && self.root.is_none()
                    && self.process.is_none()
                    && self.subreaper.is_none()
                {
                    Ok(())
                } else {
                    Err("retained package authority was internally incomplete".to_string())
                };
            }

            let process = self
                .process
                .as_mut()
                .expect("complete retained authority owns its process");
            let process_cleanup = process.cleanup();
            if !process.settled {
                return Err(match process_cleanup {
                    Ok(()) => "retained package process remained unsettled".to_string(),
                    Err(error) => error,
                });
            }

            let root = self
                .root
                .as_mut()
                .expect("complete retained authority owns its root");
            root.cleanup()?;

            drop(self.process.take());
            drop(self.root.take());
            drop(self.artifact.take());
            drop(self.subreaper.take());
            Ok(())
        }

        fn leak_unconfirmed_authority(&mut self) {
            if let Some(process) = self.process.take() {
                std::mem::forget(process);
            }
            if let Some(root) = self.root.take() {
                std::mem::forget(root);
            }
            if let Some(artifact) = self.artifact.take() {
                std::mem::forget(artifact);
            }
            if let Some(subreaper) = self.subreaper.take() {
                std::mem::forget(subreaper);
            }
        }
    }

    impl Drop for RetainedOperation {
        fn drop(&mut self) {
            for _ in 0..2 {
                if self.recover().is_ok() {
                    return;
                }
            }
            self.leak_unconfirmed_authority();
        }
    }

    enum OwnedOperationError {
        Settled(String),
        Retained(Box<RetainedOperation>),
    }

    impl OwnedOperationError {
        fn diagnostic(&self) -> &str {
            match self {
                Self::Settled(diagnostic) => diagnostic,
                Self::Retained(operation) => &operation.diagnostic,
            }
        }
    }

    impl std::fmt::Debug for OwnedOperationError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("OwnedOperationError")
                .field("diagnostic", &self.diagnostic())
                .field("ownership_retained", &matches!(self, Self::Retained(_)))
                .finish()
        }
    }

    fn lock_probes() -> MutexGuard<'static, ()> {
        PROBE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn bundle_artifact(kind: PackageKind) -> Result<PathBuf, String> {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("release")
            .join("bundle")
            .join(kind.directory());
        let mut matches = fs::read_dir(&directory)
            .map_err(|error| format!("bundle directory is unavailable: {error}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.ends_with(kind.suffix())
                            && name.contains(&format!("_{}_", env!("CARGO_PKG_VERSION")))
                    })
            })
            .collect::<Vec<_>>();
        matches.sort();
        if matches.len() != 1 {
            return Err(format!(
                "expected one version-bound {} bundle, found {}",
                kind.directory(),
                matches.len()
            ));
        }
        Ok(matches.remove(0))
    }

    fn acquire_owned_artifact(kind: PackageKind) -> Result<OwnedArtifact, String> {
        let source_path = bundle_artifact(kind)?;
        let inspected = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("bundle metadata failed: {error}"))?;
        if !inspected.is_file()
            || inspected.file_type().is_symlink()
            || inspected.len() == 0
            || inspected.len() > MAX_ARTIFACT_BYTES
        {
            return Err("bundle must be a bounded regular non-link file".to_string());
        }

        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&source_path)
            .map_err(|error| format!("bundle open failed: {error}"))?;
        let opened = source
            .metadata()
            .map_err(|error| format!("opened bundle metadata failed: {error}"))?;
        if opened.dev() != inspected.dev()
            || opened.ino() != inspected.ino()
            || opened.len() != inspected.len()
        {
            return Err("bundle identity changed while opening".to_string());
        }

        let mut bytes = Vec::with_capacity(opened.len() as usize);
        source
            .read_to_end(&mut bytes)
            .map_err(|error| format!("bundle read failed: {error}"))?;
        if bytes.len() as u64 != opened.len() {
            return Err("bundle length changed while reading".to_string());
        }
        seal_owned_artifact(kind, &bytes, Some((opened.dev(), opened.ino())))
    }

    fn seal_owned_artifact(
        kind: PackageKind,
        bytes: &[u8],
        source_identity: Option<(u64, u64)>,
    ) -> Result<OwnedArtifact, String> {
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        let name = CString::new(kind.memfd_name()).expect("fixed memfd name contains no NUL");
        let raw_fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if raw_fd < 0 {
            return Err(format!(
                "memfd_create failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut writable = unsafe { File::from_raw_fd(raw_fd) };
        writable
            .write_all(bytes)
            .map_err(|error| format!("owned artifact write failed: {error}"))?;
        writable
            .flush()
            .map_err(|error| format!("owned artifact flush failed: {error}"))?;
        let mode = if kind == PackageKind::AppImage {
            0o500
        } else {
            0o400
        };
        if unsafe { libc::fchmod(writable.as_raw_fd(), mode) } != 0 {
            return Err(format!(
                "owned artifact permissions failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let required_seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(writable.as_raw_fd(), libc::F_ADD_SEALS, required_seals) } < 0 {
            return Err(format!(
                "owned artifact sealing failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let descriptor = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", writable.as_raw_fd()))
            .map_err(|error| format!("read-only owned descriptor reopen failed: {error}"))?;
        let descriptor_metadata = descriptor
            .metadata()
            .map_err(|error| format!("owned descriptor metadata failed: {error}"))?;
        if source_identity.is_some_and(|(device, inode)| {
            descriptor_metadata.dev() == device && descriptor_metadata.ino() == inode
        }) {
            return Err("owned artifact unexpectedly aliases the source".to_string());
        }
        let owned = OwnedArtifact {
            descriptor,
            size: bytes.len() as u64,
            sha256,
            device: descriptor_metadata.dev(),
            inode: descriptor_metadata.ino(),
            required_seals,
        };
        drop(writable);
        owned.validate()?;
        Ok(owned)
    }

    fn create_private_directory(path: &Path, label: &str) -> Result<(), String> {
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|error| format!("{label} creation failed: {error}"))
    }

    fn deb_payload_executable(private_root: &Path) -> Result<PathBuf, String> {
        let payload_root = private_root.join("deb-payload");
        let canonical_root = fs::canonicalize(&payload_root)
            .map_err(|error| format!("deb payload root validation failed: {error}"))?;
        if canonical_root != payload_root {
            return Err("deb payload root traversed a link".to_string());
        }
        let executable = payload_root.join("usr/bin/batcave-monitor");
        let metadata = fs::symlink_metadata(&executable)
            .map_err(|error| format!("deb payload executable metadata failed: {error}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("deb payload executable was not a regular non-link file".to_string());
        }
        let canonical_executable = fs::canonicalize(&executable)
            .map_err(|error| format!("deb payload executable validation failed: {error}"))?;
        if canonical_executable != canonical_root.join("usr/bin/batcave-monitor") {
            return Err("deb payload executable traversed a link".to_string());
        }
        Ok(canonical_executable)
    }

    fn add_fixed_benchmark_args(command: &mut Command) {
        command.args([
            "--benchmark",
            "--platform",
            "linux",
            "--architecture",
            super::expected_architecture(),
            "--machine-class",
            super::PACKAGED_BENCHMARK_MACHINE_CLASS,
            "--workload-profile",
            super::PACKAGED_BENCHMARK_WORKLOAD_PROFILE,
            "--warmup-ticks",
            "0",
            "--ticks",
            "1",
            "--sleep-ms",
            "0",
            "--repeats",
            "1",
        ]);
    }

    fn fixed_command(probe: FixedProbe, private_root: &Path) -> Result<Command, String> {
        let mut command = match probe {
            FixedProbe::DebExtract => {
                let payload_root = private_root.join("deb-payload");
                create_private_directory(&payload_root, "deb payload root")?;
                let mut command = Command::new("/usr/bin/dpkg-deb");
                command
                    .arg("--extract")
                    .arg(format!("/proc/self/fd/{CONSUMER_FD}"))
                    .arg(payload_root);
                command
            }
            FixedProbe::DebPayloadBenchmark => {
                let mut command = Command::new(deb_payload_executable(private_root)?);
                add_fixed_benchmark_args(&mut command);
                command
            }
            FixedProbe::AppImageDescriptorPath => {
                let mut command = Command::new(format!("/proc/self/fd/{CONSUMER_FD}"));
                command.arg("--appimage-offset");
                command
            }
            FixedProbe::AppImageExecveat | FixedProbe::AppImageFexecve => {
                let executable = std::env::current_exe()
                    .map_err(|error| format!("test executable lookup failed: {error}"))?;
                let test_name = match probe {
                    FixedProbe::AppImageExecveat => "linux::fixed_execveat_launcher_entry",
                    FixedProbe::AppImageFexecve => "linux::fixed_fexecve_launcher_entry",
                    _ => unreachable!(),
                };
                let mut command = Command::new(executable);
                command.arg("--exact").arg(test_name).arg("--nocapture");
                command
            }
            FixedProbe::AppImagePayloadBenchmark => {
                let mut command = Command::new(format!("/proc/self/fd/{CONSUMER_FD}"));
                command.arg("--appimage-extract-and-run");
                add_fixed_benchmark_args(&mut command);
                command
            }
        };
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if probe.launches_payload() {
            let home = private_root.join("home");
            let data = private_root.join("xdg-data");
            let temporary = private_root.join("tmp");
            create_private_directory(&home, "payload home")?;
            create_private_directory(&data, "payload data root")?;
            create_private_directory(&temporary, "payload temporary root")?;
            command
                .env("HOME", home)
                .env("XDG_DATA_HOME", data)
                .env("TMPDIR", temporary);
        }
        if matches!(
            probe,
            FixedProbe::AppImageExecveat | FixedProbe::AppImageFexecve
        ) {
            command.env(
                APPIMAGE_LAUNCHER_MODE,
                match probe {
                    FixedProbe::AppImageExecveat => "execveat",
                    FixedProbe::AppImageFexecve => "fexecve",
                    _ => unreachable!(),
                },
            );
        }
        Ok(command)
    }

    fn spawn_probe(
        artifact: &OwnedArtifact,
        probe: FixedProbe,
        private_root: &Path,
    ) -> Result<ProbeProcess, String> {
        if probe.package_kind() == PackageKind::AppImage {
            artifact.validate()?;
        }
        let descriptor = artifact.descriptor.as_raw_fd();
        let mut command = fixed_command(probe, private_root)?;
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(descriptor, CONSUMER_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(CONSUMER_FD, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(CONSUMER_FD, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let baseline_children = direct_children()?;
        let mut child = command
            .spawn()
            .map_err(|error| format!("fixed package probe spawn failed: {error}"))?;
        let process_group = child.id() as libc::pid_t;
        let stdout = child
            .stdout
            .take()
            .expect("piped package probe stdout is present after spawn");
        let stderr = child
            .stderr
            .take()
            .expect("piped package probe stderr is present after spawn");
        let stdout = match OutputPipe::new(stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                let deadline = Instant::now() + TERMINATION_GRACE + SETTLEMENT_TIMEOUT;
                let cleanup = terminate_process_group(process_group, &mut child, deadline);
                return Err(combine_operation_cleanup(error, cleanup));
            }
        };
        let stderr = match OutputPipe::new(stderr) {
            Ok(stderr) => stderr,
            Err(error) => {
                let deadline = Instant::now() + TERMINATION_GRACE + SETTLEMENT_TIMEOUT;
                let cleanup = terminate_process_group(process_group, &mut child, deadline);
                return Err(combine_operation_cleanup(error, cleanup));
            }
        };
        Ok(ProbeProcess {
            child,
            process_group,
            stdout,
            stderr,
            baseline_children,
            settled: false,
            fail_cleanup_unsettled_once: false,
        })
    }

    fn spawn_hostile_probe(test_name: &str, mode: &str) -> Result<ProbeProcess, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("test executable lookup failed: {error}"))?;
        let mut command = Command::new(executable);
        command
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env_clear()
            .env(HOSTILE_PROBE_MODE, mode)
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let baseline_children = direct_children()?;
        let mut child = command
            .spawn()
            .map_err(|error| format!("hostile package probe spawn failed: {error}"))?;
        let process_group = child.id() as libc::pid_t;
        let stdout = child
            .stdout
            .take()
            .expect("hostile package probe owns stdout");
        let stderr = child
            .stderr
            .take()
            .expect("hostile package probe owns stderr");
        let stdout = OutputPipe::new(stdout)?;
        let stderr = OutputPipe::new(stderr)?;
        Ok(ProbeProcess {
            child,
            process_group,
            stdout,
            stderr,
            baseline_children,
            settled: false,
            fail_cleanup_unsettled_once: false,
        })
    }

    fn run_probe(
        artifact: &OwnedArtifact,
        probe: FixedProbe,
        private_root: &Path,
    ) -> Result<TransportOutcome, ProbeFailure> {
        let subreaper = SubreaperGuard::enable().map_err(ProbeFailure::from)?;
        let process = spawn_probe(artifact, probe, private_root).map_err(ProbeFailure::from)?;
        let supervised = supervise_process(process, STEP_TIMEOUT, subreaper)?;
        artifact.validate().map_err(ProbeFailure::from)?;
        Ok(TransportOutcome {
            status: supervised.status,
            stdout: supervised.stdout.bytes,
            stderr: supervised.stderr.bytes,
            output_bounded: !supervised.stdout.overflowed && !supervised.stderr.overflowed,
            process_group_settled: true,
            source_kind: "locally_built_bundle",
            public_artifact_verified: false,
            native_proven: false,
            release_evidence_emitted: false,
        })
    }

    #[derive(Debug)]
    struct SupervisedOutput {
        status: ExitStatus,
        stdout: BoundedOutput,
        stderr: BoundedOutput,
    }

    fn supervise_process(
        mut process: ProbeProcess,
        timeout: Duration,
        subreaper: SubreaperGuard,
    ) -> Result<SupervisedOutput, ProbeFailure> {
        let deadline = Instant::now() + timeout;
        let status = match wait_for_child(&mut process, deadline) {
            Ok(Some(status)) => status,
            Ok(None) => {
                return Err(ProbeFailure::after_cleanup(
                    "fixed package probe timed out".to_string(),
                    process,
                    subreaper,
                ));
            }
            Err(error) => {
                return Err(ProbeFailure::after_cleanup(error, process, subreaper));
            }
        };

        let adopted = match adopted_descendants(
            &process.baseline_children,
            process.child.id() as libc::pid_t,
        ) {
            Ok(adopted) => adopted,
            Err(error) => {
                return Err(ProbeFailure::after_cleanup(error, process, subreaper));
            }
        };
        if process_group_exists(process.process_group) || !adopted.is_empty() {
            return Err(ProbeFailure::after_cleanup(
                "unexpected package descendant required forced settlement".to_string(),
                process,
                subreaper,
            ));
        }

        let group_settled = match settle_process_group(
            process.process_group,
            &mut process.child,
            &mut process.stdout,
            &mut process.stderr,
            deadline,
        ) {
            Ok(settled) => settled,
            Err(error) => {
                return Err(ProbeFailure::after_cleanup(error, process, subreaper));
            }
        };
        let descendants_settled = match settle_adopted_descendants(
            &process.baseline_children,
            process.child.id() as libc::pid_t,
            deadline,
        ) {
            Ok(settled) => settled,
            Err(error) => {
                return Err(ProbeFailure::after_cleanup(error, process, subreaper));
            }
        };
        let output_settled = match drain_output_until(&mut process, deadline) {
            Ok(settled) => settled,
            Err(error) => {
                return Err(ProbeFailure::after_cleanup(error, process, subreaper));
            }
        };
        if !group_settled || !descendants_settled || !output_settled {
            return Err(ProbeFailure::after_cleanup(
                "package output or process ownership did not settle before the probe deadline"
                    .to_string(),
                process,
                subreaper,
            ));
        }

        process.settled = true;
        Ok(SupervisedOutput {
            status,
            stdout: BoundedOutput {
                bytes: std::mem::take(&mut process.stdout.output.bytes),
                overflowed: process.stdout.output.overflowed,
            },
            stderr: BoundedOutput {
                bytes: std::mem::take(&mut process.stderr.output.bytes),
                overflowed: process.stderr.output.overflowed,
            },
        })
    }

    fn wait_for_child(
        process: &mut ProbeProcess,
        deadline: Instant,
    ) -> Result<Option<ExitStatus>, String> {
        loop {
            process.drain_output()?;
            if let Some(status) = process
                .child
                .try_wait()
                .map_err(|error| format!("package child wait failed: {error}"))?
            {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain_output_until(process: &mut ProbeProcess, deadline: Instant) -> Result<bool, String> {
        loop {
            process.drain_output()?;
            if process.output_settled() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn combine_operation_cleanup(operation: String, cleanup: Result<(), String>) -> String {
        match cleanup {
            Ok(()) => operation,
            Err(cleanup) => format!("{operation}; cleanup boundary: {cleanup}"),
        }
    }

    fn terminate_process_group(
        process_group: libc::pid_t,
        child: &mut Child,
        deadline: Instant,
    ) -> Result<(), String> {
        signal_process_group(process_group, libc::SIGTERM)?;
        let term_deadline = (Instant::now() + TERMINATION_GRACE).min(deadline);
        while process_group_exists(process_group) && Instant::now() < term_deadline {
            let _ = child.try_wait();
            reap_process_group(process_group)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        if process_group_exists(process_group) {
            signal_process_group(process_group, libc::SIGKILL)?;
        }
        Ok(())
    }

    fn settle_process_group(
        process_group: libc::pid_t,
        child: &mut Child,
        stdout: &mut OutputPipe<ChildStdout>,
        stderr: &mut OutputPipe<ChildStderr>,
        deadline: Instant,
    ) -> Result<bool, String> {
        loop {
            stdout.drain()?;
            stderr.drain()?;
            let _ = child
                .try_wait()
                .map_err(|error| format!("package child settlement failed: {error}"))?;
            reap_process_group(process_group)?;
            if !process_group_exists(process_group) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn signal_process_group(process_group: libc::pid_t, signal: i32) -> Result<(), String> {
        let result = unsafe { libc::kill(-process_group, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("process-group signal failed: {error}"))
        }
    }

    fn process_group_exists(process_group: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn reap_process_group(process_group: libc::pid_t) -> Result<(), String> {
        loop {
            let mut status = 0;
            let result = unsafe { libc::waitpid(-process_group, &mut status, libc::WNOHANG) };
            if result > 0 {
                continue;
            }
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            return Err(format!("process-group reap failed: {error}"));
        }
    }

    fn direct_children() -> Result<BTreeSet<libc::pid_t>, String> {
        let mut children = BTreeSet::new();
        let task_root = Path::new("/proc/self/task");
        let tasks = fs::read_dir(task_root)
            .map_err(|error| format!("direct-child inventory failed: {error}"))?;
        for task in tasks {
            let task = task.map_err(|error| format!("direct-child inventory failed: {error}"))?;
            let path = task.path().join("children");
            let value = match fs::read_to_string(path) {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!("direct-child inventory failed: {error}"));
                }
            };
            for value in value.split_whitespace() {
                let pid = value
                    .parse::<libc::pid_t>()
                    .map_err(|_| "direct-child inventory was malformed".to_string())?;
                children.insert(pid);
            }
        }
        Ok(children)
    }

    fn adopted_descendants(
        baseline: &BTreeSet<libc::pid_t>,
        original_child: libc::pid_t,
    ) -> Result<BTreeSet<libc::pid_t>, String> {
        let mut children = direct_children()?;
        children.remove(&original_child);
        children.retain(|pid| !baseline.contains(pid));
        Ok(children)
    }

    fn signal_process(process: libc::pid_t, signal: i32) -> Result<(), String> {
        let result = unsafe { libc::kill(process, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("adopted-descendant signal failed: {error}"))
        }
    }

    fn reap_adopted_descendants(
        baseline: &BTreeSet<libc::pid_t>,
        original_child: libc::pid_t,
    ) -> Result<(), String> {
        for pid in adopted_descendants(baseline, original_child)? {
            let mut status = 0;
            let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if result >= 0 {
                continue;
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ECHILD) {
                return Err(format!("adopted-descendant reap failed: {error}"));
            }
        }
        Ok(())
    }

    fn terminate_adopted_descendants(
        baseline: &BTreeSet<libc::pid_t>,
        original_child: libc::pid_t,
        deadline: Instant,
    ) -> Result<(), String> {
        let term_deadline = (Instant::now() + TERMINATION_GRACE).min(deadline);
        loop {
            reap_adopted_descendants(baseline, original_child)?;
            let children = adopted_descendants(baseline, original_child)?;
            if children.is_empty() {
                return Ok(());
            }
            let signal = if Instant::now() < term_deadline {
                libc::SIGTERM
            } else {
                libc::SIGKILL
            };
            for child in children {
                signal_process(child, signal)?;
            }
            if Instant::now() >= deadline {
                return Err("adopted package descendants did not settle".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn settle_adopted_descendants(
        baseline: &BTreeSet<libc::pid_t>,
        original_child: libc::pid_t,
        deadline: Instant,
    ) -> Result<bool, String> {
        loop {
            reap_adopted_descendants(baseline, original_child)?;
            if adopted_descendants(baseline, original_child)?.is_empty() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn appimage_argv() -> (Vec<CString>, Vec<*const libc::c_char>) {
        let values = vec![
            CString::new("BatCave.Monitor.AppImage").expect("fixed argv contains no NUL"),
            CString::new("--appimage-offset").expect("fixed argv contains no NUL"),
        ];
        let mut pointers = values
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        (values, pointers)
    }

    fn appimage_env() -> (Vec<CString>, Vec<*const libc::c_char>) {
        let values = vec![
            CString::new("PATH=/usr/sbin:/usr/bin:/sbin:/bin")
                .expect("fixed environment contains no NUL"),
            CString::new("LANG=C").expect("fixed environment contains no NUL"),
            CString::new("LC_ALL=C").expect("fixed environment contains no NUL"),
        ];
        let mut pointers = values
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        (values, pointers)
    }

    #[test]
    fn fixed_execveat_launcher_entry() {
        if std::env::var(APPIMAGE_LAUNCHER_MODE).as_deref() != Ok("execveat") {
            return;
        }
        let (_argv_values, argv) = appimage_argv();
        let (_env_values, env) = appimage_env();
        let empty = CString::new("").expect("empty path contains no NUL");
        let result = unsafe {
            libc::execveat(
                CONSUMER_FD,
                empty.as_ptr(),
                argv.as_ptr().cast::<*mut libc::c_char>(),
                env.as_ptr().cast::<*mut libc::c_char>(),
                libc::AT_EMPTY_PATH,
            )
        };
        assert_eq!(
            result,
            0,
            "execveat failed: {}",
            std::io::Error::last_os_error()
        );
    }

    #[test]
    fn fixed_fexecve_launcher_entry() {
        if std::env::var(APPIMAGE_LAUNCHER_MODE).as_deref() != Ok("fexecve") {
            return;
        }
        let (_argv_values, argv) = appimage_argv();
        let (_env_values, env) = appimage_env();
        let result = unsafe { libc::fexecve(CONSUMER_FD, argv.as_ptr(), env.as_ptr()) };
        assert_eq!(
            result,
            0,
            "fexecve failed: {}",
            std::io::Error::last_os_error()
        );
    }

    fn spawn_hostile_descendant(escape_group: bool) {
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "hostile descendant fork failed");
        if child == 0 {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
            if escape_group && unsafe { libc::setpgid(0, 0) } != 0 {
                unsafe { libc::_exit(70) };
            }
            unsafe {
                libc::sleep(60);
                libc::_exit(0);
            }
        }
    }

    #[test]
    fn hostile_same_group_descendant_entry() {
        if std::env::var(HOSTILE_PROBE_MODE).as_deref() != Ok("same-group") {
            return;
        }
        spawn_hostile_descendant(false);
    }

    #[test]
    fn hostile_escaped_pipe_descendant_entry() {
        if std::env::var(HOSTILE_PROBE_MODE).as_deref() != Ok("escaped-pipe") {
            return;
        }
        spawn_hostile_descendant(true);
    }

    #[test]
    fn hostile_infinite_output_entry() {
        if std::env::var(HOSTILE_PROBE_MODE).as_deref() != Ok("infinite-output") {
            return;
        }
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
        let output = [b'x'; 4096];
        loop {
            std::io::stdout()
                .write_all(&output)
                .expect("hostile output pipe remains open");
        }
    }

    fn assert_hostile_descendant_fails_closed(test_name: &str, mode: &str) {
        let _lock = lock_probes();
        let subreaper = SubreaperGuard::enable().expect("enable hostile-probe subreaper");
        let baseline = direct_children().expect("capture direct-child baseline");
        let process = spawn_hostile_probe(test_name, mode).expect("spawn hostile probe");
        let started = Instant::now();
        let error = supervise_process(process, Duration::from_secs(2), subreaper)
            .expect_err("unexpected descendant cannot produce transport success");
        assert!(
            error.contains("unexpected package descendant required forced settlement"),
            "unexpected failure: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "hostile output ownership exceeded its deadline"
        );
        assert_eq!(
            direct_children().expect("inventory children after settlement"),
            baseline,
            "hostile descendants must be killed and reaped"
        );
    }

    #[test]
    fn surviving_same_group_descendant_is_cleanup_not_transport_success() {
        assert_hostile_descendant_fails_closed(
            "linux::hostile_same_group_descendant_entry",
            "same-group",
        );
    }

    #[test]
    fn escaped_descendant_with_inherited_pipes_fails_within_deadline() {
        assert_hostile_descendant_fails_closed(
            "linux::hostile_escaped_pipe_descendant_entry",
            "escaped-pipe",
        );
    }

    #[test]
    fn continuously_writing_child_cannot_starve_the_probe_deadline() {
        let _lock = lock_probes();
        let subreaper = SubreaperGuard::enable().expect("enable hostile-probe subreaper");
        let baseline = direct_children().expect("capture direct-child baseline");
        let process =
            spawn_hostile_probe("linux::hostile_infinite_output_entry", "infinite-output")
                .expect("spawn infinite-output probe");
        let started = Instant::now();
        let error = supervise_process(process, Duration::from_millis(100), subreaper)
            .expect_err("continuous output cannot bypass the deadline");
        assert!(
            error.contains("fixed package probe timed out"),
            "unexpected failure: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "continuous output exceeded bounded cleanup"
        );
        assert_eq!(
            direct_children().expect("inventory children after output cleanup"),
            baseline,
            "continuous-output child must be killed and reaped"
        );
    }

    #[test]
    fn unconfirmed_settlement_retains_artifact_process_and_root_until_recovery() {
        let _lock = lock_probes();
        let subreaper = SubreaperGuard::enable().expect("enable retained-probe subreaper");
        let baseline = direct_children().expect("capture retained-probe child baseline");
        let artifact = seal_owned_artifact(
            PackageKind::Deb,
            b"inert retained Linux package authority fixture\n",
            None,
        )
        .expect("create inert retained artifact");
        let root = PrivateRoot::create("retained-settlement").expect("create retained root");
        let root_path = root.path.clone();
        let mut process =
            spawn_hostile_probe("linux::hostile_infinite_output_entry", "infinite-output")
                .expect("spawn retained hostile probe");
        process.fail_cleanup_unsettled_once = true;

        let failure = supervise_process(process, Duration::from_millis(100), subreaper)
            .expect_err("injected unconfirmed settlement must retain process authority");
        let error = finish_owned_operation(artifact, root, Err::<(), _>(failure))
            .expect_err("unconfirmed settlement cannot become an ordinary error");
        let OwnedOperationError::Retained(mut retained) = error else {
            panic!("unconfirmed settlement did not retain the complete operation");
        };

        assert!(retained.owns_all_authority());
        assert!(retained.artifact_valid());
        assert_eq!(retained.root_path(), Some(root_path.as_path()));
        assert!(root_path.exists());
        assert!(retained.process_unsettled());
        assert!(retained
            .diagnostic
            .contains("fixed package probe timed out"));
        assert!(retained
            .diagnostic
            .contains("retained injected unsettled ownership"));

        retained
            .recover()
            .expect("fixed retained recovery must settle and release all authority");
        assert!(!retained.owns_all_authority());
        assert!(!root_path.exists());
        assert_eq!(
            direct_children().expect("inventory children after retained recovery"),
            baseline,
            "retained recovery must kill and reap the hostile process"
        );
    }

    #[test]
    fn dropping_retained_settlement_uses_the_same_fixed_recovery_path() {
        let _lock = lock_probes();
        let subreaper = SubreaperGuard::enable().expect("enable drop-recovery subreaper");
        let baseline = direct_children().expect("capture drop-recovery child baseline");
        let artifact = seal_owned_artifact(
            PackageKind::AppImage,
            b"inert retained AppImage drop fixture\n",
            None,
        )
        .expect("create retained drop artifact");
        let root = PrivateRoot::create("retained-drop").expect("create retained drop root");
        let root_path = root.path.clone();
        let mut process =
            spawn_hostile_probe("linux::hostile_infinite_output_entry", "infinite-output")
                .expect("spawn retained drop probe");
        process.fail_cleanup_unsettled_once = true;

        let failure = supervise_process(process, Duration::from_millis(100), subreaper)
            .expect_err("drop fixture must first retain unsettled ownership");
        let error = finish_owned_operation(artifact, root, Err::<(), _>(failure))
            .expect_err("drop fixture cannot collapse retained ownership");
        let OwnedOperationError::Retained(retained) = error else {
            panic!("drop fixture did not retain the complete operation");
        };
        assert!(retained.owns_all_authority());
        assert!(root_path.exists());

        drop(retained);
        assert!(!root_path.exists());
        assert_eq!(
            direct_children().expect("inventory children after drop recovery"),
            baseline,
            "drop recovery must kill and reap the hostile process"
        );
    }

    fn assert_non_proof(outcome: &TransportOutcome) {
        assert_eq!(outcome.source_kind, "locally_built_bundle");
        assert!(!outcome.public_artifact_verified);
        assert!(!outcome.native_proven);
        assert!(!outcome.release_evidence_emitted);
        assert!(outcome.output_bounded);
        assert!(outcome.process_group_settled);
        assert!(
            outcome.status.success(),
            "fixed probe failed: {}",
            String::from_utf8_lossy(&outcome.stderr)
        );
    }

    fn parse_appimage_offset(
        probe: FixedProbe,
        stdout: &[u8],
        artifact_size: u64,
    ) -> Result<u64, String> {
        let text = std::str::from_utf8(stdout)
            .map_err(|_| invalid_appimage_offset(probe, stdout, "output was not UTF-8"))?;
        let offset_line = match probe {
            FixedProbe::AppImageDescriptorPath => text.strip_suffix('\n').unwrap_or(text),
            FixedProbe::AppImageExecveat | FixedProbe::AppImageFexecve => text
                .strip_prefix("\nrunning 1 test\n")
                .and_then(|offset| offset.strip_suffix('\n'))
                .ok_or_else(|| {
                    invalid_appimage_offset(
                        probe,
                        stdout,
                        "transcript shape did not match the fixed mode",
                    )
                })?,
            FixedProbe::DebExtract
            | FixedProbe::DebPayloadBenchmark
            | FixedProbe::AppImagePayloadBenchmark => {
                return Err(invalid_appimage_offset(
                    probe,
                    stdout,
                    "probe is not an AppImage mode",
                ));
            }
        };
        if offset_line.is_empty() || !offset_line.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid_appimage_offset(
                probe,
                stdout,
                "offset line was not ASCII decimal",
            ));
        }
        let offset = offset_line.parse::<u64>().map_err(|_| {
            invalid_appimage_offset(probe, stdout, "offset did not fit in an unsigned integer")
        })?;
        if offset == 0 || offset >= artifact_size {
            return Err(invalid_appimage_offset(
                probe,
                stdout,
                "offset was outside the owned artifact",
            ));
        }
        Ok(offset)
    }

    fn invalid_appimage_offset(probe: FixedProbe, stdout: &[u8], reason: &str) -> String {
        const PREVIEW_BYTES: usize = 160;
        let preview = stdout
            .iter()
            .take(PREVIEW_BYTES)
            .flat_map(|byte| std::ascii::escape_default(*byte))
            .map(char::from)
            .collect::<String>();
        let truncated = if stdout.len() > PREVIEW_BYTES {
            ", truncated"
        } else {
            ""
        };
        format!(
            "{} AppImage offset output invalid: {reason}; length={}, preview=\"{preview}\"{truncated}",
            probe.label(),
            stdout.len()
        )
    }

    fn finish_owned_operation<T>(
        artifact: OwnedArtifact,
        root: PrivateRoot,
        operation: Result<T, ProbeFailure>,
    ) -> Result<T, OwnedOperationError> {
        match operation {
            Ok(value) => root.finish(Ok(value)).map_err(OwnedOperationError::Settled),
            Err(ProbeFailure::Settled(error)) => root
                .finish::<T>(Err(error))
                .map_err(OwnedOperationError::Settled),
            Err(ProbeFailure::Unsettled(unsettled)) => {
                let UnsettledProbe {
                    diagnostic,
                    process,
                    subreaper,
                } = *unsettled;
                Err(OwnedOperationError::Retained(Box::new(RetainedOperation {
                    artifact: Some(artifact),
                    root: Some(root),
                    process: Some(process),
                    subreaper: Some(subreaper),
                    diagnostic,
                })))
            }
        }
    }

    fn with_owned_operation<T>(
        kind: PackageKind,
        label: &str,
        operation: impl FnOnce(&OwnedArtifact, &Path) -> Result<T, ProbeFailure>,
    ) -> Result<T, OwnedOperationError> {
        let artifact = acquire_owned_artifact(kind).map_err(OwnedOperationError::Settled)?;
        let root = PrivateRoot::create(label).map_err(OwnedOperationError::Settled)?;
        let result = operation(&artifact, &root.path);
        finish_owned_operation(artifact, root, result)
    }

    #[test]
    fn operation_and_cleanup_failures_are_both_reported_without_residue() {
        let _lock = lock_probes();
        let mut root = PrivateRoot::create("cleanup-failure").expect("create private root");
        root.fail_cleanup_once = true;
        let path = root.path.clone();
        let error = root
            .finish::<()>(Err("fixed package probe failed".to_string()))
            .expect_err("operation and cleanup failure must be explicit");
        assert!(error.contains("fixed package probe failed"));
        assert!(error.contains("cleanup boundary: private root cleanup failed"));
        assert!(
            !path.exists(),
            "explicit cleanup retry removes injected cleanup residue"
        );
    }

    #[test]
    fn unsettled_authority_stays_out_of_line_at_error_boundaries() {
        assert!(
            std::mem::size_of::<ProbeFailure>() < std::mem::size_of::<UnsettledProbe>(),
            "probe failures must not move live process authority inline"
        );
        assert!(
            std::mem::size_of::<OwnedOperationError>() < std::mem::size_of::<RetainedOperation>(),
            "operation errors must not move retained artifact/process/root authority inline"
        );
    }

    #[test]
    fn appimage_offset_transcripts_are_strict_and_mode_specific() {
        const SIZE: u64 = 500_000;
        const OFFSET: u64 = 191_840;
        assert_eq!(
            parse_appimage_offset(FixedProbe::AppImageDescriptorPath, b"191840\n", SIZE,),
            Ok(OFFSET)
        );
        assert_eq!(
            parse_appimage_offset(FixedProbe::AppImageDescriptorPath, b"191840", SIZE),
            Ok(OFFSET)
        );
        for probe in [FixedProbe::AppImageExecveat, FixedProbe::AppImageFexecve] {
            assert_eq!(
                parse_appimage_offset(probe, b"\nrunning 1 test\n191840\n", SIZE),
                Ok(OFFSET)
            );
        }

        let hostile: &[(FixedProbe, &[u8])] = &[
            (
                FixedProbe::AppImageDescriptorPath,
                b"running 1 test\n191840\n",
            ),
            (FixedProbe::AppImageDescriptorPath, b"\n191840\n"),
            (FixedProbe::AppImageDescriptorPath, b"191840\n\n"),
            (FixedProbe::AppImageDescriptorPath, b"191840\n191840\n"),
            (FixedProbe::AppImageDescriptorPath, b"offset=191840\n"),
            (FixedProbe::AppImageDescriptorPath, b" 191840\n"),
            (
                FixedProbe::AppImageDescriptorPath,
                b"18446744073709551616\n",
            ),
            (FixedProbe::AppImageDescriptorPath, b"0\n"),
            (FixedProbe::AppImageDescriptorPath, b"500000\n"),
            (FixedProbe::AppImageDescriptorPath, b"\xff\n"),
            (FixedProbe::AppImageExecveat, b"191840\n"),
            (FixedProbe::AppImageExecveat, b"running 1 test\n191840\n"),
            (
                FixedProbe::AppImageExecveat,
                b"\n\nrunning 1 test\n191840\n",
            ),
            (
                FixedProbe::AppImageExecveat,
                b"\nrunning 1 test\n\n191840\n",
            ),
            (
                FixedProbe::AppImageExecveat,
                b"\nrunning 1 test\n191840\n\n",
            ),
            (FixedProbe::AppImageExecveat, b"\nrunning 1 test\n191840"),
            (FixedProbe::AppImageExecveat, b"running 2 tests\n191840\n"),
            (
                FixedProbe::AppImageExecveat,
                b"running 1 test\nnoise\n191840\n",
            ),
            (
                FixedProbe::AppImageFexecve,
                b"running 1 test\n191840\nextra\n",
            ),
            (
                FixedProbe::AppImageFexecve,
                b"running 1 test\n191840\n191840\n",
            ),
        ];
        for (probe, transcript) in hostile {
            let error = parse_appimage_offset(*probe, transcript, SIZE)
                .expect_err("unexpected output must not produce an offset");
            assert!(error.contains(probe.label()));
            assert!(error.contains("preview=\""));
            assert!(!error.contains(char::from(0xff)));
        }
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_deb_extracts_from_the_owned_descriptor_without_installing() {
        let _lock = lock_probes();
        let outcome = with_owned_operation(PackageKind::Deb, "deb", |artifact, root| {
            let outcome = run_probe(artifact, FixedProbe::DebExtract, root)?;
            for binary in ["batcave-monitor", "batcave-monitor-cli"] {
                let path = root.join("deb-payload/usr/bin").join(binary);
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| format!("staged binary metadata failed: {error}"))?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("staged deb binary identity was invalid".to_string().into());
                }
            }
            Ok(outcome)
        })
        .expect("run fixed dpkg-deb extraction and clean private root");
        assert_non_proof(&outcome);
        assert!(outcome.stdout.is_empty());
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_deb_payload_launches_from_the_owned_extraction_without_installing() {
        let _lock = lock_probes();
        let outcome = with_owned_operation(PackageKind::Deb, "deb-payload", |artifact, root| {
            let extraction = run_probe(artifact, FixedProbe::DebExtract, root)?;
            assert_non_proof(&extraction);
            if !extraction.stdout.is_empty() {
                return Err("fixed deb extraction emitted unexpected output"
                    .to_string()
                    .into());
            }
            run_probe(artifact, FixedProbe::DebPayloadBenchmark, root)
        })
        .expect("launch the fixed deb payload benchmark and clean its private root");
        assert_non_proof(&outcome);
        let observation = super::parse_packaged_benchmark(&outcome.stdout, outcome.output_bounded)
            .unwrap_or_else(|error| {
                panic!("{error}: {}", String::from_utf8_lossy(&outcome.stdout))
            });
        assert_eq!(observation.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_appimage_runtime_accepts_all_closed_owned_descriptor_modes() {
        let _lock = lock_probes();
        let mut expected_offset = None;
        for probe in [
            FixedProbe::AppImageDescriptorPath,
            FixedProbe::AppImageExecveat,
            FixedProbe::AppImageFexecve,
        ] {
            let (outcome, artifact_size) =
                with_owned_operation(PackageKind::AppImage, "appimage", |artifact, root| {
                    Ok((run_probe(artifact, probe, root)?, artifact.size))
                })
                .expect("run AppImage probe and clean private root");
            assert_non_proof(&outcome);
            let offset = parse_appimage_offset(probe, &outcome.stdout, artifact_size)
                .unwrap_or_else(|error| panic!("{error}"));
            if let Some(expected) = expected_offset {
                assert_eq!(offset, expected, "owned descriptor modes disagree");
            } else {
                expected_offset = Some(offset);
            }
        }
    }

    #[test]
    #[ignore = "requires Linux deb/AppImage bundles built from this checkout"]
    fn built_appimage_payload_launches_from_the_owned_descriptor() {
        let _lock = lock_probes();
        let outcome = with_owned_operation(
            PackageKind::AppImage,
            "appimage-payload",
            |artifact, root| run_probe(artifact, FixedProbe::AppImagePayloadBenchmark, root),
        )
        .expect("launch the fixed AppImage payload benchmark and clean its private root");
        assert_non_proof(&outcome);
        let observation = super::parse_packaged_benchmark(&outcome.stdout, outcome.output_bounded)
            .unwrap_or_else(|error| {
                panic!("{error}: {}", String::from_utf8_lossy(&outcome.stdout))
            });
        assert_eq!(observation.app_version, env!("CARGO_PKG_VERSION"));
    }
}
