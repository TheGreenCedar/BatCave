#!/usr/bin/env bash
set -euo pipefail

build_binary=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --verify-only)
      build_binary=0
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-macos-arm64-cli.sh must run on macOS." >&2
  exit 2
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "BatCave macOS builds require an Apple Silicon host." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
target="aarch64-apple-darwin"
binary="$repo_root/src/BatCave.App/src-tauri/target/$target/release/batcave-monitor-cli"

if ! rustup target list --installed | grep -qx "$target"; then
  echo "Apple Silicon macOS CLI builds require: rustup target add $target" >&2
  exit 2
fi

if [[ "$build_binary" -eq 1 ]]; then
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
  cargo build \
    --manifest-path "$manifest" \
    --release \
    --bin batcave-monitor-cli \
    --features tauri/custom-protocol \
    --target "$target"
fi

[[ -f "$binary" ]] || {
  echo "Missing Apple Silicon macOS CLI binary: $binary" >&2
  exit 1
}
lipo "$binary" -verify_arch arm64
chmod +x "$binary"

echo "Built Apple Silicon macOS CLI:"
echo "  $binary"
