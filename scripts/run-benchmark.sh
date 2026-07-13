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
min_speedup_multiplier=""
max_p95_ms=""
strict=0
dev_build=0
temp_baseline_path=""

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
    --strict)
      strict=1
      shift
      ;;
    --dev-build)
      dev_build=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$benchmark_host" != "core" ]]; then
  echo "unsupported benchmark host: $benchmark_host" >&2
  exit 2
fi
if [[ -n "$baseline_json_path" && -n "$baseline_artifact_path" ]]; then
  echo "Specify either --baseline-json or --baseline-artifact, not both." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
build_profile="release"
if [[ "$dev_build" -eq 1 ]]; then
  build_profile="debug"
fi
benchmark_exe="$repo_root/src/BatCave.App/src-tauri/target/$build_profile/batcave-monitor-cli"
runtime_platform="linux"

case "$(uname -s)" in
  Darwin)
    runtime_platform="macos"
    [[ "$architecture" == "arm64" ]] && architecture="aarch64"
    ;;
  Linux)
    runtime_platform="linux"
    ;;
  *)
    echo "run-benchmark.sh supports Linux and macOS. Use scripts/run-benchmark.ps1 on Windows." >&2
    exit 2
    ;;
esac

cleanup() {
  if [[ -n "$temp_baseline_path" ]]; then
    rm -f -- "$temp_baseline_path"
  fi
}
trap cleanup EXIT

if [[ -n "$baseline_artifact_path" ]]; then
  temp_baseline_path="$(mktemp)"
  python3 - "$baseline_artifact_path" "$temp_baseline_path" "$benchmark_host" "$runtime_platform" "$architecture" "$machine_class" "$workload_profile" "$warmup_ticks" "$ticks" "$sleep_ms" "$repeats" "$repo_root" <<'PY'
import json
import os
import sys

(
    artifact_path,
    output_path,
    host,
    platform,
    architecture,
    machine_class,
    workload_profile,
    warmup_ticks,
    measured_ticks,
    sleep_ms,
    repeat_count,
    repo_root,
) = sys.argv[1:]

with open(artifact_path, "r", encoding="utf-8") as handle:
    artifact = json.load(handle)

expected = {
    "format_version": 3,
    "host": host,
    "platform": platform,
    "architecture": architecture,
    "machine_class": machine_class,
    "workload_profile": workload_profile,
    "warmup_ticks": int(warmup_ticks),
    "measured_ticks": int(measured_ticks),
    "sleep_ms": int(sleep_ms),
    "repeat_count": int(repeat_count),
}
for key, expected_value in expected.items():
    actual = artifact.get(key)
    if actual != expected_value:
        raise SystemExit(
            f"Baseline artifact {key} mismatch. Expected {expected_value!r}, found {actual!r}."
        )

summary = artifact.get("baseline_summary")
if summary is None:
    summary_path = artifact.get("baseline_summary_path")
    if summary_path and not os.path.isabs(summary_path):
        summary_path = os.path.join(repo_root, summary_path)
    if summary_path and os.path.exists(summary_path):
        with open(summary_path, "r", encoding="utf-8") as handle:
            summary = json.load(handle)
if summary is None:
    raise SystemExit("Baseline artifact missing baseline_summary and a readable baseline_summary_path.")

with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2)
PY
  baseline_json_path="$temp_baseline_path"
fi

build_args=(build --manifest-path "$cargo_manifest" --bin batcave-monitor-cli)
if [[ "$dev_build" -eq 0 ]]; then
  build_args+=(--release)
fi
cargo "${build_args[@]}"
if [[ ! -x "$benchmark_exe" ]]; then
  echo "Benchmark executable not found after $build_profile build: $benchmark_exe" >&2
  exit 2
fi

benchmark_args=(
  --benchmark
  --platform "$runtime_platform"
  --architecture "$architecture"
  --machine-class "$machine_class"
  --workload-profile "$workload_profile"
  --warmup-ticks "$warmup_ticks"
  --ticks "$ticks"
  --sleep-ms "$sleep_ms"
  --repeats "$repeats"
)
if [[ "$strict" -eq 1 ]]; then
  benchmark_args+=(--strict)
fi
if [[ -n "$baseline_json_path" ]]; then
  benchmark_args+=(--baseline-json "$baseline_json_path")
fi
if [[ -n "$min_speedup_multiplier" ]]; then
  benchmark_args+=(--min-speedup-multiplier "$min_speedup_multiplier")
elif [[ "$strict" -eq 1 && -n "$baseline_json_path" ]]; then
  benchmark_args+=(--min-speedup-multiplier "0.90")
fi
if [[ -n "$max_p95_ms" ]]; then
  benchmark_args+=(--max-p95-ms "$max_p95_ms")
fi

"$benchmark_exe" "${benchmark_args[@]}"
