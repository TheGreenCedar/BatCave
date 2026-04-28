#!/usr/bin/env bash
set -euo pipefail

benchmark_host="core"
platform="$(uname -m)"
workload_profile="fixed-default"
machine_class="${HOSTNAME:-linux}"
warmup_ticks=30
measured_ticks=120
sleep_ms=1000
repeat_count=5
output_directory=""
no_build=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --benchmark-host|--host)
      benchmark_host="${2:?missing value for $1}"
      shift 2
      ;;
    --platform)
      platform="${2:?missing value for $1}"
      shift 2
      ;;
    --workload-profile)
      workload_profile="${2:?missing value for $1}"
      shift 2
      ;;
    --machine-class)
      machine_class="${2:?missing value for $1}"
      shift 2
      ;;
    --warmup-ticks)
      warmup_ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --measured-ticks)
      measured_ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --sleep-ms)
      sleep_ms="${2:?missing value for $1}"
      shift 2
      ;;
    --repeat-count)
      repeat_count="${2:?missing value for $1}"
      shift 2
      ;;
    --output-directory)
      output_directory="${2:?missing value for $1}"
      shift 2
      ;;
    --no-build)
      no_build=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
run_benchmark="$script_dir/run-benchmark.sh"

if [[ -z "$output_directory" ]]; then
  output_directory="$repo_root/artifacts/benchmarks"
fi
mkdir -p "$output_directory"

timestamp="$(date -u +%Y%m%d-%H%M%S)"
artifact_prefix="baseline-${benchmark_host}-${timestamp}"
artifact_path="$output_directory/${artifact_prefix}.json"
baseline_summary_path="$output_directory/${artifact_prefix}.summary.json"

echo "Capturing baseline benchmark ($benchmark_host) with fixed profile '$workload_profile'..."
echo "Warmup: $warmup_ticks ticks, measured: $measured_ticks ticks, sleep: $sleep_ms ms, repeats: $repeat_count."

run_no_build_arg=()
if [[ "$no_build" -eq 0 ]]; then
  echo "Building Rust benchmark host before baseline capture..."
  cargo build --manifest-path "$cargo_manifest" --release
  run_no_build_arg=(--no-build)
else
  run_no_build_arg=(--no-build)
fi

echo "Running warmup window..."
bash "$run_benchmark" --benchmark-host "$benchmark_host" --platform "$platform" --ticks "$warmup_ticks" --sleep-ms "$sleep_ms" "${run_no_build_arg[@]}" >/dev/null

runs_file="$(mktemp)"
trap 'rm -f "$runs_file"' EXIT

for ((index = 1; index <= repeat_count; index++)); do
  echo "Running measured repeat $index/$repeat_count..."
  raw="$(bash "$run_benchmark" --benchmark-host "$benchmark_host" --platform "$platform" --ticks "$measured_ticks" --sleep-ms "$sleep_ms" "${run_no_build_arg[@]}")"
  python3 - "$runs_file" "$raw" <<'PY'
import json
import sys

runs_file, raw = sys.argv[1:]
start = raw.find("{")
end = raw.rfind("}")
if start < 0 or end < start:
    raise SystemExit("Unable to locate benchmark JSON payload in output.")
run = json.loads(raw[start : end + 1])
with open(runs_file, "a", encoding="utf-8") as handle:
    handle.write(json.dumps(run) + "\n")
PY
done

python3 - "$runs_file" "$artifact_path" "$baseline_summary_path" "$machine_class" "$benchmark_host" "$platform" "$workload_profile" "$warmup_ticks" "$measured_ticks" "$sleep_ms" "$repeat_count" <<'PY'
import json
import sys
from datetime import datetime, timezone

(
    runs_file,
    artifact_path,
    baseline_summary_path,
    machine_class,
    host,
    platform,
    workload_profile,
    warmup_ticks,
    measured_ticks,
    sleep_ms,
    repeat_count,
) = sys.argv[1:]

with open(runs_file, "r", encoding="utf-8") as handle:
    runs = [json.loads(line) for line in handle if line.strip()]

if not runs:
    raise SystemExit("No benchmark runs were captured.")

baseline_summary = sorted(runs, key=lambda run: run.get("tick_p95_ms", 0))[max(0, (len(runs) - 1) // 2)]
artifact = {
    "format_version": 1,
    "captured_at_utc": datetime.now(timezone.utc).isoformat(),
    "machine_class": machine_class,
    "host": host,
    "platform": platform,
    "workload_profile": workload_profile,
    "warmup_ticks": int(warmup_ticks),
    "measured_ticks": int(measured_ticks),
    "sleep_ms": int(sleep_ms),
    "repeat_count": int(repeat_count),
    "baseline_selection": "median-by-tick-p95",
    "baseline_summary": baseline_summary,
    "baseline_summary_path": baseline_summary_path,
    "runs": runs,
}

with open(artifact_path, "w", encoding="utf-8") as handle:
    json.dump(artifact, handle, indent=2)
with open(baseline_summary_path, "w", encoding="utf-8") as handle:
    json.dump(baseline_summary, handle, indent=2)
PY

echo "Baseline artifact written:"
echo "  $artifact_path"
echo "Baseline summary for --baseline-json written:"
echo "  $baseline_summary_path"
