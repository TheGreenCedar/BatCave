#!/usr/bin/env bash
set -euo pipefail

benchmark_host="core"
architecture="$(uname -m)"
workload_profile="fixed-default"
machine_class="${HOSTNAME:-local}"
warmup_ticks=30
measured_ticks=120
sleep_ms=1000
repeat_count=5
output_directory=""

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
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" == "Darwin" && "$architecture" == "arm64" ]]; then
  architecture="aarch64"
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
artifact_prefix="baseline-${benchmark_host}-${timestamp}"
artifact_path="$output_directory/${artifact_prefix}.json"
baseline_summary_path="$output_directory/${artifact_prefix}.summary.json"
raw_file="$(mktemp)"
trap 'rm -f -- "$raw_file"' EXIT

echo "Capturing benchmark protocol v3 baseline ($benchmark_host)..."
bash "$run_benchmark" \
  --benchmark-host "$benchmark_host" \
  --platform "$architecture" \
  --machine-class "$machine_class" \
  --workload-profile "$workload_profile" \
  --warmup-ticks "$warmup_ticks" \
  --ticks "$measured_ticks" \
  --sleep-ms "$sleep_ms" \
  --repeats "$repeat_count" >"$raw_file"

base_sha="$(git -C "$repo_root" rev-parse HEAD)"
if [[ -n "$(git -C "$repo_root" status --porcelain)" ]]; then
  base_sha="${base_sha}-dirty"
fi
if command -v sha256sum >/dev/null 2>&1; then
  binary_sha256="$(sha256sum "$benchmark_exe" | awk '{print $1}')"
else
  binary_sha256="$(shasum -a 256 "$benchmark_exe" | awk '{print $1}')"
fi

python3 - "$raw_file" "$artifact_path" "$baseline_summary_path" "$base_sha" "$binary_sha256" <<'PY'
import json
import sys
from datetime import datetime, timezone

raw_file, artifact_path, summary_path, base_sha, binary_sha256 = sys.argv[1:]
with open(raw_file, "r", encoding="utf-8") as handle:
    raw = handle.read()
start = raw.find("{")
end = raw.rfind("}")
if start < 0 or end < start:
    raise SystemExit("Unable to locate benchmark JSON payload in output.")
summary = json.loads(raw[start : end + 1])

artifact = {
    "format_version": 3,
    "captured_at_utc": datetime.now(timezone.utc).isoformat(),
    "base_sha": base_sha,
    "binary_sha256": binary_sha256,
    "host": summary["host"],
    "measurement_origin": summary["measurement_origin"],
    "platform": summary["platform"],
    "architecture": summary["architecture"],
    "machine_class": summary["machine_class"],
    "workload_profile": summary["workload_profile"],
    "warmup_ticks": summary["warmup_ticks"],
    "measured_ticks": summary["measured_ticks"],
    "sleep_ms": summary["sleep_ms"],
    "repeat_count": summary["repeat_count"],
    "baseline_selection": "median-by-tick-p95",
    "baseline_summary": summary,
    "baseline_summary_path": summary_path,
}

with open(artifact_path, "w", encoding="utf-8") as handle:
    json.dump(artifact, handle, indent=2)
with open(summary_path, "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2)
PY

echo "Baseline artifact written:"
echo "  $artifact_path"
echo "Baseline summary written:"
echo "  $baseline_summary_path"
