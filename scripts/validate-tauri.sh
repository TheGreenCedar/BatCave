#!/usr/bin/env bash
set -euo pipefail

skip_bundle=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-bundle)
      skip_bundle=1
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
app_root="$repo_root/src/BatCave.App"
cargo_manifest="$app_root/src-tauri/Cargo.toml"

cd "$app_root"

npm run verify
cargo fmt --manifest-path "$cargo_manifest" --check
cargo check --manifest-path "$cargo_manifest"
cargo test --manifest-path "$cargo_manifest"

if [[ "$skip_bundle" -eq 0 ]]; then
  npm run tauri:build
fi
