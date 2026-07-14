#!/usr/bin/env bash
set -euo pipefail

benchmark_host="core"
architecture="$(uname -m)"
machine_class="${HOSTNAME:-local}"
workload_profile="fixed-default"
warmup_ticks=30
ticks=120
sleep_ms=1000
repeats=5
baseline_json_path=""
baseline_artifact_path=""
min_speedup_multiplier="0.90"
max_p95_ms=""
output_directory=""

case "$(uname -s)" in
  Darwin)
    runtime_platform="macos"
    ;;
  Linux)
    runtime_platform="linux"
    ;;
  *)
    echo "run-benchmark-gate.sh supports Linux and macOS. Use scripts/run-benchmark-gate.ps1 on Windows." >&2
    exit 2
    ;;
esac

while [[ $# -gt 0 ]]; do
  case "$1" in
    --benchmark-host|--host)
      benchmark_host="${2:?missing value for $1}"
      shift 2
      ;;
    --platform)
      architecture="${2:?missing value for $1}"
      shift 2
      ;;
    --machine-class)
      machine_class="${2:?missing value for $1}"
      shift 2
      ;;
    --workload-profile)
      workload_profile="${2:?missing value for $1}"
      shift 2
      ;;
    --warmup-ticks)
      warmup_ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --ticks)
      ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --sleep-ms)
      sleep_ms="${2:?missing value for $1}"
      shift 2
      ;;
    --repeats)
      repeats="${2:?missing value for $1}"
      shift 2
      ;;
    --baseline-json)
      baseline_json_path="${2:?missing value for $1}"
      shift 2
      ;;
    --baseline-artifact)
      baseline_artifact_path="${2:?missing value for $1}"
      shift 2
      ;;
    --min-speedup-multiplier)
      min_speedup_multiplier="${2:?missing value for $1}"
      shift 2
      ;;
    --max-p95-ms)
      max_p95_ms="${2:?missing value for $1}"
      shift 2
      ;;
    --output-directory)
      output_directory="${2:?missing value for $1}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$runtime_platform" == "macos" && "$architecture" == "arm64" ]]; then
  architecture="aarch64"
fi

if [[ -n "$baseline_json_path" && -n "$baseline_artifact_path" ]]; then
  echo "Specify either --baseline-json or --baseline-artifact, not both." >&2
  exit 2
fi
has_baseline=0
if [[ -n "$baseline_json_path" || -n "$baseline_artifact_path" ]]; then
  has_baseline=1
fi
if [[ "$has_baseline" -eq 0 && -z "$max_p95_ms" ]]; then
  echo "Benchmark gate requires --baseline-json, --baseline-artifact, or --max-p95-ms." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
run_benchmark="$script_dir/run-benchmark.sh"
benchmark_exe="$repo_root/src/BatCave.App/src-tauri/target/release/batcave-monitor-cli"
if [[ -z "$output_directory" ]]; then
  output_directory="$repo_root/artifacts/benchmarks"
fi
mkdir -p "$output_directory"

timestamp="$(date -u +%Y%m%d-%H%M%S)"
report_path="$output_directory/gate-${benchmark_host}-${timestamp}.json"
raw_file="$(mktemp)"
trap 'rm -f -- "$raw_file"' EXIT

benchmark_args=(
  --benchmark-host "$benchmark_host"
  --platform "$architecture"
  --machine-class "$machine_class"
  --workload-profile "$workload_profile"
  --warmup-ticks "$warmup_ticks"
  --ticks "$ticks"
  --sleep-ms "$sleep_ms"
  --repeats "$repeats"
  --strict
)
if [[ -n "$baseline_json_path" ]]; then
  benchmark_args+=(--baseline-json "$baseline_json_path")
fi
if [[ -n "$baseline_artifact_path" ]]; then
  benchmark_args+=(--baseline-artifact "$baseline_artifact_path")
fi
if [[ "$has_baseline" -eq 1 && -n "$min_speedup_multiplier" ]]; then
  benchmark_args+=(--min-speedup-multiplier "$min_speedup_multiplier")
fi
if [[ -n "$max_p95_ms" ]]; then
  benchmark_args+=(--max-p95-ms "$max_p95_ms")
fi

set +e
bash "$run_benchmark" "${benchmark_args[@]}" >"$raw_file" 2>&1
exit_code=$?
set -e
cat "$raw_file"

candidate_sha="$(git -C "$repo_root" rev-parse HEAD)"
if [[ -n "$(git -C "$repo_root" status --porcelain)" ]]; then
  candidate_sha="${candidate_sha}-dirty"
fi
binary_sha256=""
if [[ -x "$benchmark_exe" ]]; then
  if command -v sha256sum >/dev/null 2>&1; then
    binary_sha256="$(sha256sum "$benchmark_exe" | awk '{print $1}')"
  else
    binary_sha256="$(shasum -a 256 "$benchmark_exe" | awk '{print $1}')"
  fi
fi

python3 - "$raw_file" "$report_path" "$benchmark_host" "$runtime_platform" "$architecture" "$machine_class" "$workload_profile" "$warmup_ticks" "$ticks" "$sleep_ms" "$repeats" "$baseline_json_path" "$baseline_artifact_path" "$min_speedup_multiplier" "$max_p95_ms" "$has_baseline" "$candidate_sha" "$binary_sha256" "$exit_code" <<'PY'
import json
import sys
from datetime import datetime, timezone

(
    raw_file,
    report_path,
    host,
    platform,
    architecture,
    machine_class,
    workload_profile,
    warmup_ticks,
    measured_ticks,
    sleep_ms,
    repeat_count,
    baseline_json_path,
    baseline_artifact_path,
    min_speedup_multiplier,
    max_p95_ms,
    has_baseline,
    candidate_sha,
    binary_sha256,
    exit_code,
) = sys.argv[1:]

with open(raw_file, "r", encoding="utf-8") as handle:
    raw = handle.read()
summary = None
start = raw.find("{")
end = raw.rfind("}")
if start >= 0 and end >= start:
    try:
        summary = json.loads(raw[start : end + 1])
    except json.JSONDecodeError:
        if int(exit_code) == 0:
            raise
elif int(exit_code) == 0:
    raise SystemExit("Unable to locate benchmark JSON payload in output.")

report = {
    "format_version": 4,
    "captured_at_utc": datetime.now(timezone.utc).isoformat(),
    "candidate_sha": candidate_sha,
    "binary_sha256": binary_sha256,
    "host": host,
    "measurement_origin": "owned_sampling_engine_refresh_and_protocol_serialization",
    "evidence_scope": "core_runtime_host_only",
    "whole_app_measured": False,
    "live_command": "refresh_now",
    "command_transport": "in_process_bounded_channel",
    "serialization_scope": "runtime_protocol_v3_encode_and_json",
    "latency_gate_metric": "median_live_command_p95_ms",
    "platform": platform,
    "architecture": architecture,
    "machine_class": machine_class,
    "workload_profile": workload_profile,
    "warmup_ticks": int(warmup_ticks),
    "measured_ticks": int(measured_ticks),
    "inter_command_delay_ms": int(sleep_ms),
    "repeat_count": int(repeat_count),
    "strict": True,
    "baseline_json_path": baseline_json_path,
    "baseline_artifact_path": baseline_artifact_path,
    "min_speedup_multiplier": min_speedup_multiplier if has_baseline == "1" else "",
    "max_p95_ms": max_p95_ms,
    "exit_code": int(exit_code),
    "strict_passed": bool(summary and summary.get("strict_passed")),
    "speed_ratio": summary.get("speed_ratio") if summary else None,
    "median_collection_p95_ms": summary.get("median_collection_p95_ms") if summary else None,
    "median_publication_p95_ms": summary.get("median_publication_p95_ms") if summary else None,
    "median_serialization_p95_ms": summary.get("median_serialization_p95_ms") if summary else None,
    "median_live_command_p95_ms": summary.get("median_live_command_p95_ms") if summary else None,
    "benchmark_summary": summary,
}
with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
PY

echo "Benchmark gate report written:"
echo "  $report_path"
exit "$exit_code"
