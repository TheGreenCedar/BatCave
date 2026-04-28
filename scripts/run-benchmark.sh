#!/usr/bin/env bash
set -euo pipefail

benchmark_host="core"
platform="$(uname -m)"
ticks=120
sleep_ms=1000
baseline_json_path=""
baseline_artifact_path=""
min_speedup_multiplier=""
max_p95_ms=""
strict=0
no_build=0
temp_baseline_path=""

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
    --strict)
      strict=1
      shift
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

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
benchmark_exe="$repo_root/src/BatCave.App/src-tauri/target/release/batcave-monitor"

if [[ "$no_build" -eq 0 ]]; then
  cargo build --manifest-path "$cargo_manifest" --release
fi

if [[ ! -x "$benchmark_exe" ]]; then
  echo "Benchmark executable not found: $benchmark_exe. Run without --no-build first." >&2
  exit 2
fi

if [[ -n "$baseline_artifact_path" ]]; then
  resolved_baseline="$(
    python3 - "$baseline_artifact_path" "$benchmark_host" "$platform" "$repo_root" "$ticks" "$sleep_ms" <<'PY'
import json
import os
import sys
import tempfile

artifact_path, host, platform, repo_root, ticks, sleep_ms = sys.argv[1:]
with open(artifact_path, "r", encoding="utf-8") as handle:
    artifact = json.load(handle)

def require_match(key, expected):
    actual = artifact.get(key)
    if actual not in (None, "") and str(actual) != str(expected):
        raise SystemExit(f"Baseline artifact {key} mismatch. Expected {expected!r}, found {actual!r}.")

require_match("host", host)
require_match("platform", platform)
require_match("measured_ticks", ticks)
require_match("sleep_ms", sleep_ms)

summary_path = artifact.get("baseline_summary_path")
if summary_path:
    if not os.path.isabs(summary_path):
        summary_path = os.path.join(repo_root, summary_path)
    if os.path.exists(summary_path):
        print(summary_path)
        raise SystemExit(0)

summary = artifact.get("baseline_summary")
if summary is None:
    raise SystemExit("Baseline artifact missing baseline_summary and baseline_summary_path.")

fd, path = tempfile.mkstemp(prefix="batcave-baseline-summary-", suffix=".json")
with os.fdopen(fd, "w", encoding="utf-8") as handle:
    json.dump(summary, handle, indent=2)
print(path)
PY
  )"
  baseline_json_path="$resolved_baseline"
  if [[ "$baseline_json_path" == /tmp/batcave-baseline-summary-* ]]; then
    temp_baseline_path="$baseline_json_path"
  fi
fi

benchmark_args=(--benchmark --ticks "$ticks" --sleep-ms "$sleep_ms")
if [[ "$strict" -eq 1 ]]; then
  benchmark_args+=(--strict)
fi
if [[ -n "$baseline_json_path" ]]; then
  benchmark_args+=(--baseline-json "$baseline_json_path")
fi
if [[ -n "$min_speedup_multiplier" ]]; then
  benchmark_args+=(--min-speedup-multiplier "$min_speedup_multiplier")
elif [[ "$strict" -eq 1 && -n "$baseline_json_path" ]]; then
  benchmark_args+=(--min-speedup-multiplier "10")
fi
if [[ -n "$max_p95_ms" ]]; then
  benchmark_args+=(--max-p95-ms "$max_p95_ms")
fi

cleanup() {
  if [[ -n "$temp_baseline_path" ]]; then
    rm -f "$temp_baseline_path"
  fi
}
trap cleanup EXIT

"$benchmark_exe" "${benchmark_args[@]}"
