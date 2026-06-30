#!/usr/bin/env bash
set -euo pipefail

benchmark_host="core"
platform="$(uname -m)"
ticks=120
sleep_ms=1000
baseline_json_path=""
baseline_artifact_path=""
min_speedup_multiplier="0.90"
max_p95_ms=""
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
    --ticks)
      ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --sleep-ms)
      sleep_ms="${2:?missing value for $1}"
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

if [[ -z "$output_directory" ]]; then
  output_directory="$repo_root/artifacts/benchmarks"
fi
mkdir -p "$output_directory"

timestamp="$(date -u +%Y%m%d-%H%M%S)"
report_path="$output_directory/gate-${benchmark_host}-${timestamp}.json"
raw_file="$(mktemp)"
trap 'rm -f "$raw_file"' EXIT

benchmark_args=(--benchmark-host "$benchmark_host" --platform "$platform" --ticks "$ticks" --sleep-ms "$sleep_ms" --strict)
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
if [[ "$no_build" -eq 1 ]]; then
  benchmark_args+=(--no-build)
fi

set +e
bash "$run_benchmark" "${benchmark_args[@]}" >"$raw_file" 2>&1
exit_code=$?
set -e

cat "$raw_file"

python3 - "$raw_file" "$report_path" "$benchmark_host" "$platform" "$ticks" "$sleep_ms" "$baseline_json_path" "$baseline_artifact_path" "$min_speedup_multiplier" "$max_p95_ms" "$has_baseline" "$exit_code" <<'PY'
import json
import sys
from datetime import datetime, timezone

(
    raw_file,
    report_path,
    host,
    platform,
    ticks,
    sleep_ms,
    baseline_json_path,
    baseline_artifact_path,
    min_speedup_multiplier,
    max_p95_ms,
    has_baseline,
    exit_code,
) = sys.argv[1:]

with open(raw_file, "r", encoding="utf-8") as handle:
    raw = handle.read()

summary = None
start = raw.find("{")
end = raw.rfind("}")
if start >= 0 and end >= start:
    summary = json.loads(raw[start : end + 1])
elif int(exit_code) == 0:
    raise SystemExit("Unable to locate benchmark JSON payload in output.")

report = {
    "format_version": 1,
    "captured_at_utc": datetime.now(timezone.utc).isoformat(),
    "host": host,
    "platform": platform,
    "ticks": int(ticks),
    "sleep_ms": int(sleep_ms),
    "strict": True,
    "baseline_json_path": baseline_json_path,
    "baseline_artifact_path": baseline_artifact_path,
    "min_speedup_multiplier": min_speedup_multiplier if has_baseline == "1" else "",
    "max_p95_ms": max_p95_ms,
    "exit_code": int(exit_code),
    "strict_passed": bool(summary and summary.get("strict_passed")),
    "benchmark_summary": summary,
}

with open(report_path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
PY

echo "Benchmark gate report written:"
echo "  $report_path"

exit "$exit_code"
