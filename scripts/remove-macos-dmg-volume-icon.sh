#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: remove-macos-dmg-volume-icon.sh <dmg>" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "remove-macos-dmg-volume-icon.sh must run on macOS." >&2
  exit 2
fi

dmg="$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
[[ -f "$dmg" ]] || { echo "Missing DMG: $dmg" >&2; exit 1; }

workspace="$(mktemp -d)"
mount_point="$workspace/mount"
read_write_dmg="$workspace/read-write.dmg"
clean_dmg="$workspace/clean.dmg"
mounted=0

cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$mount_point" -quiet || true
  fi
  rm -rf -- "$workspace"
}
trap cleanup EXIT

mkdir "$mount_point"
hdiutil convert -quiet "$dmg" -format UDRW -o "$read_write_dmg"
hdiutil attach -quiet -nobrowse -readwrite -mountpoint "$mount_point" "$read_write_dmg"
mounted=1

rm -f -- "$mount_point/.VolumeIcon.icns"
SetFile -a c "$mount_point"

hdiutil detach "$mount_point" -quiet
mounted=0
hdiutil convert -quiet "$read_write_dmg" -format UDZO -imagekey zlib-level=9 -o "$clean_dmg"
mv -f -- "$clean_dmg" "$dmg"

echo "Removed the custom volume icon payload from: $dmg"
