#!/usr/bin/env bash
set -euo pipefail

skip_bundle=0
bundle_only=0
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
    --bundle-only)
      bundle_only=1
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

if [[ "$skip_bundle" -eq 1 && "$bundle_only" -eq 1 ]]; then
  echo "--skip-bundle and --bundle-only cannot be used together." >&2
  exit 2
fi
if [[ "$bundle_only" -eq 1 && "$benchmark_gate" -eq 1 ]]; then
  echo "--benchmark-gate cannot be used with --bundle-only." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
app_root="$repo_root/src/BatCave.App"
cargo_manifest="$app_root/src-tauri/Cargo.toml"

case "$(uname -s)" in
  Darwin | Linux) ;;
  *)
    echo "validate-tauri.sh supports Linux and macOS. Use scripts/validate-tauri.ps1 on Windows." >&2
    exit 2
    ;;
esac

cd "$app_root"

if [[ "$bundle_only" -eq 0 ]]; then
  npm run verify
  cargo fmt --manifest-path "$cargo_manifest" --check
  cargo test --manifest-path "$cargo_manifest"

  if [[ "$benchmark_gate" -eq 1 ]]; then
    echo "Running owned-engine in-process live-command p95 regression gate..."
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
    echo "Running owned-engine in-process live-command p95 smoke..."
    bash "$repo_root/scripts/run-benchmark.sh" --benchmark-host core --platform "$benchmark_platform" --warmup-ticks 0 --ticks 2 --sleep-ms 1000 --repeats 1 --strict --max-p95-ms 10000 --dev-build
  fi
fi

if [[ "$skip_bundle" -eq 0 ]]; then
  if [[ "$(uname -s)" == "Darwin" ]]; then
    missing_targets=()
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
      if ! rustup target list --installed | grep -qx "$target"; then
        missing_targets+=("$target")
      fi
    done
    if [[ "${#missing_targets[@]}" -gt 0 ]]; then
      echo "Universal macOS bundling requires: rustup target add ${missing_targets[*]}" >&2
      exit 2
    fi

    npm run build
    npm run tauri -- build --target universal-apple-darwin --config src-tauri/tauri.macos.ci.conf.json --no-bundle
    bash "$repo_root/scripts/build-macos-universal-cli.sh" --lipo-only
    npm run tauri -- build --target universal-apple-darwin --config src-tauri/tauri.macos.ci.conf.json
    bash "$repo_root/scripts/verify-macos-bundle.sh" --mode adhoc
  else
    npm run tauri -- build
    bash "$repo_root/scripts/verify-linux-bundle.sh"
  fi
fi
