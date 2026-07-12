#!/usr/bin/env bash
set -euo pipefail

mode="adhoc"
bundle_root=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      mode="${2:?missing value for --mode}"
      shift 2
      ;;
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

if [[ "$mode" != "adhoc" && "$mode" != "release" ]]; then
  echo "--mode must be adhoc or release" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "verify-macos-bundle.sh must run on macOS." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
if [[ -z "$bundle_root" ]]; then
  bundle_root="$repo_root/src/BatCave.App/src-tauri/target/universal-apple-darwin/release/bundle"
fi

shopt -s nullglob
apps=("$bundle_root"/macos/*.app)
dmgs=("$bundle_root"/dmg/*.dmg)
if [[ "${#apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one .app under $bundle_root/macos; found ${#apps[@]}." >&2
  exit 1
fi
if [[ "${#dmgs[@]}" -ne 1 ]]; then
  echo "Expected exactly one .dmg under $bundle_root/dmg; found ${#dmgs[@]}." >&2
  exit 1
fi
app="${apps[0]}"
dmg="${dmgs[0]}"

verify_app() {
  local candidate="$1"
  local plist="$candidate/Contents/Info.plist"
  local executable_name
  local executable
  local minimum_version
  local signature_details

  [[ -f "$plist" ]] || { echo "Missing app Info.plist: $plist" >&2; return 1; }
  executable_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")"
  executable="$candidate/Contents/MacOS/$executable_name"
  [[ -f "$executable" ]] || { echo "Missing app executable: $executable" >&2; return 1; }

  lipo "$executable" -verify_arch arm64 x86_64
  minimum_version="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$plist")"
  [[ "$minimum_version" == "12.0" ]] || {
    echo "Expected LSMinimumSystemVersion 12.0, found $minimum_version." >&2
    return 1
  }

  codesign --verify --deep --strict --verbose=2 "$candidate"
  signature_details="$(codesign -dv --verbose=4 "$candidate" 2>&1)"
  grep -q 'flags=.*runtime' <<<"$signature_details" || {
    echo "Hardened runtime is not enabled for $candidate." >&2
    return 1
  }
  if [[ "$mode" == "adhoc" ]]; then
    grep -q 'Signature=adhoc' <<<"$signature_details" || {
      echo "Expected an ad-hoc signature for $candidate." >&2
      return 1
    }
  else
    grep -q '^Authority=Developer ID Application:' <<<"$signature_details" || {
      echo "Expected a Developer ID Application signature for $candidate." >&2
      return 1
    }
  fi
}

verify_app "$app"
hdiutil verify "$dmg"

mount_point="$(mktemp -d)"
mounted=0
cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$mount_point" -quiet || true
  fi
  rm -rf -- "$mount_point"
}
trap cleanup EXIT

hdiutil attach -nobrowse -readonly -mountpoint "$mount_point" "$dmg" >/dev/null
mounted=1
mounted_apps=("$mount_point"/*.app)
if [[ "${#mounted_apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one app inside $dmg; found ${#mounted_apps[@]}." >&2
  exit 1
fi
verify_app "${mounted_apps[0]}"

if [[ "$mode" == "release" ]]; then
  updater_archives=("$bundle_root"/macos/*.app.tar.gz)
  if [[ "${#updater_archives[@]}" -ne 1 ]]; then
    echo "Expected exactly one universal .app.tar.gz updater archive; found ${#updater_archives[@]}." >&2
    exit 1
  fi
  [[ -s "${updater_archives[0]}.sig" ]] || {
    echo "Missing updater signature: ${updater_archives[0]}.sig" >&2
    exit 1
  }

  codesign --verify --strict --verbose=2 "$dmg"
  spctl --assess --type execute --verbose=4 "$app"
  spctl --assess --type execute --verbose=4 "${mounted_apps[0]}"
  spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"
  xcrun stapler validate "$app"
  xcrun stapler validate "$dmg"
fi

echo "Verified universal macOS $mode app and DMG:"
echo "  $app"
echo "  $dmg"
