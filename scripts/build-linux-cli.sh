#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "build-linux-cli.sh must run on Linux." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"

cargo build \
  --manifest-path "$cargo_manifest" \
  --release \
  --bin batcave-monitor-cli
