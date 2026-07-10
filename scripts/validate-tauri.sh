#!/usr/bin/env bash
set -euo pipefail

skip_bundle=0
benchmark_gate=0
benchmark_platform="$(uname -m)"
benchmark_ticks=120
benchmark_sleep_ms=1000
benchmark_baseline_json_path=""
benchmark_baseline_artifact_path=""
benchmark_min_speedup_multiplier="0.90"
benchmark_max_p95_ms=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-bundle)
      skip_bundle=1
      shift
      ;;
    --benchmark-gate)
      benchmark_gate=1
      shift
      ;;
    --benchmark-platform)
      benchmark_platform="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-ticks)
      benchmark_ticks="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-sleep-ms)
      benchmark_sleep_ms="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-baseline-json)
      benchmark_baseline_json_path="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-baseline-artifact)
      benchmark_baseline_artifact_path="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-min-speedup-multiplier)
      benchmark_min_speedup_multiplier="${2:?missing value for $1}"
      shift 2
      ;;
    --benchmark-max-p95-ms)
      benchmark_max_p95_ms="${2:?missing value for $1}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
app_root="$repo_root/src/BatCave.App"
cargo_manifest="$app_root/src-tauri/Cargo.toml"
tauri_build_script="tauri:build:linux"

cd "$app_root"

npm run verify
cargo fmt --manifest-path "$cargo_manifest" --check
cargo check --manifest-path "$cargo_manifest"
cargo test --manifest-path "$cargo_manifest"

if [[ "$benchmark_gate" -eq 1 ]]; then
  gate_args=(--benchmark-host core --platform "$benchmark_platform" --ticks "$benchmark_ticks" --sleep-ms "$benchmark_sleep_ms")
  if [[ -n "$benchmark_baseline_json_path" ]]; then
    gate_args+=(--baseline-json "$benchmark_baseline_json_path")
  fi
  if [[ -n "$benchmark_baseline_artifact_path" ]]; then
    gate_args+=(--baseline-artifact "$benchmark_baseline_artifact_path")
  fi
  if [[ -n "$benchmark_min_speedup_multiplier" ]]; then
    gate_args+=(--min-speedup-multiplier "$benchmark_min_speedup_multiplier")
  fi
  if [[ -n "$benchmark_max_p95_ms" ]]; then
    gate_args+=(--max-p95-ms "$benchmark_max_p95_ms")
  fi

  bash "$repo_root/scripts/run-benchmark-gate.sh" "${gate_args[@]}"
else
  bash "$repo_root/scripts/run-benchmark.sh" --benchmark-host core --platform "$benchmark_platform" --warmup-ticks 0 --ticks 2 --sleep-ms 0 --repeats 1 --strict --max-p95-ms 10000
fi

if [[ "$skip_bundle" -eq 0 ]]; then
  npm run "$tauri_build_script"
fi
