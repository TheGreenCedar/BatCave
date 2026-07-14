#!/usr/bin/env bash
set -euo pipefail

bundle_root=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-root)
      bundle_root="${2:?missing value for --bundle-root}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "verify-linux-bundle.sh must run on Linux." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_version="$(node "$repo_root/scripts/verify-release-version.mjs" --print)"
if [[ -z "$bundle_root" ]]; then
  bundle_root="$repo_root/src/BatCave.App/src-tauri/target/release/bundle"
fi

shopt -s nullglob
debs=("$bundle_root"/deb/*.deb)
appimages=("$bundle_root"/appimage/*.AppImage)
if [[ "${#debs[@]}" -ne 1 ]]; then
  echo "Expected exactly one deb under $bundle_root/deb; found ${#debs[@]}." >&2
  exit 1
fi
if [[ "${#appimages[@]}" -ne 1 ]]; then
  echo "Expected exactly one AppImage under $bundle_root/appimage; found ${#appimages[@]}." >&2
  exit 1
fi

deb="${debs[0]}"
appimage="${appimages[0]}"
deb_version="$(dpkg-deb --field "$deb" Version)"
[[ "$deb_version" == "$cargo_version" ]] || {
  echo "Expected deb version $cargo_version, found $deb_version." >&2
  exit 1
}
for artifact in "$deb" "$appimage"; do
  [[ "$(basename -- "$artifact")" == *"_${cargo_version}_"* ]] || {
    echo "Expected $(basename -- "$artifact") to contain version _${cargo_version}_." >&2
    exit 1
  }
done

echo "Verified Linux package version $cargo_version:"
echo "  $deb"
echo "  $appimage"
