#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-macos-universal-cli.sh must run on macOS." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
manifest="$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
target_root="$repo_root/src/BatCave.App/src-tauri/target"
binary_name="batcave-monitor-cli"

missing_targets=()
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  if ! rustup target list --installed | grep -qx "$target"; then
    missing_targets+=("$target")
  fi
done
if [[ "${#missing_targets[@]}" -gt 0 ]]; then
  echo "Universal macOS CLI build requires: rustup target add ${missing_targets[*]}" >&2
  exit 2
fi

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --manifest-path "$manifest" --release --bin "$binary_name" --target "$target"
done

output_directory="$target_root/universal-apple-darwin/release"
output="$output_directory/$binary_name"
mkdir -p "$output_directory"
lipo -create \
  "$target_root/aarch64-apple-darwin/release/$binary_name" \
  "$target_root/x86_64-apple-darwin/release/$binary_name" \
  -output "$output"
lipo "$output" -verify_arch arm64 x86_64
chmod +x "$output"

echo "Built universal macOS CLI:"
echo "  $output"
