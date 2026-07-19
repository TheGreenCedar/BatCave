#!/usr/bin/env bash
set -euo pipefail

mode="adhoc"
bundle_root=""
updater_archive=""

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
    --updater-archive)
      updater_archive="${2:?missing value for --updater-archive}"
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
if [[ "$mode" == "release" && -n "$updater_archive" ]]; then
  echo "--updater-archive is only available for local ad-hoc verification" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "verify-macos-bundle.sh must run on macOS." >&2
  exit 2
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "BatCave macOS bundle verification requires an Apple Silicon host." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
cargo_version="$(node "$repo_root/scripts/verify-release-version.mjs" --print)"
tauri_config="$repo_root/src/BatCave.App/src-tauri/tauri.conf.json"
expected_bundle_id="$(node -e 'const c=require(process.argv[1]); process.stdout.write(c.identifier)' "$tauri_config")"
expected_app_name="$(node -e 'const c=require(process.argv[1]); process.stdout.write(`${c.productName}.app`)' "$tauri_config")"
if [[ -z "$bundle_root" ]]; then
  bundle_root="$repo_root/src/BatCave.App/src-tauri/target/aarch64-apple-darwin/release/bundle"
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
reference_team_identifier=""
reference_authority=""

verify_foundation_models_sidecar() {
  local candidate="$1"
  local expected_team_identifier="$2"
  local expected_authority="$3"
  local sidecar="$candidate/Contents/MacOS/batcave-foundation-models"
  local minimum_version
  local load_commands
  local signature_details
  local team_identifier
  local authority
  local entitlements

  [[ -x "$sidecar" ]] || {
    echo "Missing executable Foundation Models sidecar: $sidecar" >&2
    return 1
  }
  lipo "$sidecar" -verify_arch arm64 || {
    echo "Expected an arm64 slice in $sidecar." >&2
    return 1
  }
  if lipo "$sidecar" -verify_arch x86_64 >/dev/null 2>&1; then
    echo "Unexpected Intel x86_64 slice in $sidecar." >&2
    return 1
  fi
  minimum_version="$(vtool -show-build "$sidecar" | awk '$1 == "minos" { print $2; exit }')"
  [[ "$minimum_version" == "12.0" ]] || {
    echo "Expected Foundation Models sidecar deployment target 12.0, found $minimum_version." >&2
    return 1
  }

  load_commands="$(otool -l "$sidecar")"
  awk '
    $1 == "cmd" { command = $2 }
    $1 == "name" && $2 ~ /FoundationModels\.framework/ && command == "LC_LOAD_WEAK_DYLIB" { weak = 1 }
    END { exit weak ? 0 : 1 }
  ' <<<"$load_commands" || {
    echo "FoundationModels.framework is not weak-linked in $sidecar." >&2
    return 1
  }
  if awk '
    $1 == "cmd" { command = $2 }
    $1 == "name" && $2 ~ /FoundationModels\.framework/ && command == "LC_LOAD_DYLIB" { strong = 1 }
    END { exit strong ? 0 : 1 }
  ' <<<"$load_commands"; then
    echo "FoundationModels.framework is strongly linked in $sidecar." >&2
    return 1
  fi

  codesign --verify --strict --verbose=2 "$sidecar"
  signature_details="$(codesign -dv --verbose=4 "$sidecar" 2>&1)"
  grep -q 'flags=.*runtime' <<<"$signature_details" || {
    echo "Hardened runtime is not enabled for $sidecar." >&2
    return 1
  }
  if [[ "$mode" == "adhoc" ]]; then
    grep -q 'Signature=adhoc' <<<"$signature_details" || {
      echo "Expected an ad-hoc signature for $sidecar." >&2
      return 1
    }
  else
    team_identifier="$(sed -n 's/^TeamIdentifier=//p' <<<"$signature_details" | head -n 1)"
    authority="$(sed -n 's/^Authority=//p' <<<"$signature_details" | head -n 1)"
    [[ "$team_identifier" == "$expected_team_identifier" ]] || {
      echo "Expected sidecar TeamIdentifier $expected_team_identifier, found $team_identifier." >&2
      return 1
    }
    [[ "$authority" == "$expected_authority" ]] || {
      echo "Expected sidecar signing authority $expected_authority, found $authority." >&2
      return 1
    }
  fi

  entitlements="$(codesign -d --entitlements :- "$sidecar" 2>/dev/null || true)"
  if grep -Eq 'com\.apple\.security\.network\.(client|server)' <<<"$entitlements"; then
    echo "Foundation Models sidecar must not carry network entitlements." >&2
    return 1
  fi
}

verify_app() {
  local candidate="$1"
  local role="$2"
  local plist="$candidate/Contents/Info.plist"
  local bundle_identifier
  local executable_name
  local executable
  local minimum_version
  local bundle_short_version
  local bundle_version
  local team_identifier
  local authority
  local signature_details

  [[ -f "$plist" ]] || { echo "Missing app Info.plist: $plist" >&2; return 1; }
  executable_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")"
  executable="$candidate/Contents/MacOS/$executable_name"
  [[ -f "$executable" ]] || { echo "Missing app executable: $executable" >&2; return 1; }

  bundle_short_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist")"
  bundle_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$plist")"
  bundle_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")"
  [[ "$bundle_identifier" == "$expected_bundle_id" ]] || {
    echo "Expected CFBundleIdentifier $expected_bundle_id, found $bundle_identifier in $candidate." >&2
    return 1
  }
  [[ "$bundle_short_version" == "$cargo_version" ]] || {
    echo "Expected CFBundleShortVersionString $cargo_version, found $bundle_short_version." >&2
    return 1
  }
  [[ "$bundle_version" == "$cargo_version" ]] || {
    echo "Expected CFBundleVersion $cargo_version, found $bundle_version." >&2
    return 1
  }

  lipo "$executable" -verify_arch arm64 || {
    echo "Expected an arm64 slice in $executable." >&2
    return 1
  }
  if lipo "$executable" -verify_arch x86_64 >/dev/null 2>&1; then
    echo "Unexpected Intel x86_64 slice in $executable." >&2
    return 1
  fi
  minimum_version="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$plist")"
  [[ "$minimum_version" == "12.0" ]] || {
    echo "Expected LSMinimumSystemVersion 12.0, found $minimum_version." >&2
    return 1
  }

  codesign --verify --deep --strict --verbose=2 "$candidate"
  signature_details="$(codesign -dv --verbose=4 "$candidate" 2>&1)"
  team_identifier="$(sed -n 's/^TeamIdentifier=//p' <<<"$signature_details" | head -n 1)"
  authority="$(sed -n 's/^Authority=//p' <<<"$signature_details" | head -n 1)"
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
    [[ -n "$team_identifier" && "$team_identifier" != "not set" ]] || {
      echo "Expected a Developer ID TeamIdentifier for $candidate." >&2
      return 1
    }
    if [[ "$role" == "direct" ]]; then
      reference_team_identifier="$team_identifier"
      reference_authority="$authority"
    else
      [[ "$team_identifier" == "$reference_team_identifier" ]] || {
        echo "Expected TeamIdentifier $reference_team_identifier, found $team_identifier in $candidate." >&2
        return 1
      }
      [[ "$authority" == "$reference_authority" ]] || {
        echo "Expected signing authority $reference_authority, found $authority in $candidate." >&2
        return 1
      }
    fi
  fi

  verify_foundation_models_sidecar "$candidate" "$team_identifier" "$authority"
}

verify_app "$app" direct
hdiutil verify "$dmg"

mount_point="$(mktemp -d)"
verification_workspace="$(mktemp -d)"
mounted=0
cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$mount_point" -quiet || true
  fi
  rm -rf -- "$mount_point"
  rm -rf -- "$verification_workspace"
}
trap cleanup EXIT

hdiutil attach -nobrowse -readonly -mountpoint "$mount_point" "$dmg" >/dev/null
mounted=1
if [[ -e "$mount_point/.VolumeIcon.icns" ]]; then
  echo "DMG exposes the internal .VolumeIcon.icns file in its install window." >&2
  exit 1
fi
mounted_apps=("$mount_point"/*.app)
if [[ "${#mounted_apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one app inside $dmg; found ${#mounted_apps[@]}." >&2
  exit 1
fi
verify_app "${mounted_apps[0]}" mounted

if [[ "$mode" == "release" ]]; then
  updater_archives=("$bundle_root"/macos/*.app.tar.gz)
  if [[ "${#updater_archives[@]}" -ne 1 ]]; then
    echo "Expected exactly one Apple Silicon .app.tar.gz updater archive; found ${#updater_archives[@]}." >&2
    exit 1
  fi
  [[ -s "${updater_archives[0]}.sig" ]] || {
    echo "Missing updater signature: ${updater_archives[0]}.sig" >&2
    exit 1
  }

  signed_archive="$verification_workspace/verified.app.tar.gz"
  cargo run --quiet --locked \
    --manifest-path "$repo_root/src/BatCave.App/src-tauri/Cargo.toml" \
    --bin batcave-verify-updater-signature -- \
    "${updater_archives[0]}" "${updater_archives[0]}.sig" "$tauri_config" "$signed_archive"
  updater_archive="$signed_archive"
fi

extracted_app=""
if [[ -n "$updater_archive" ]]; then
  [[ -f "$updater_archive" ]] || {
    echo "Missing updater archive: $updater_archive" >&2
    exit 1
  }
  extracted_app="$(python3 "$repo_root/scripts/extract-macos-updater-archive.py" \
    "$updater_archive" "$verification_workspace/extracted" \
    --expected-app-name "$expected_app_name")"
  verify_app "$extracted_app" updater
fi

if [[ "$mode" == "release" ]]; then
  codesign --verify --strict --verbose=2 "$dmg"
  spctl --assess --type execute --verbose=4 "$app"
  spctl --assess --type execute --verbose=4 "${mounted_apps[0]}"
  spctl --assess --type execute --verbose=4 "$extracted_app"
  spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"
  xcrun stapler validate "$app"
  xcrun stapler validate "$extracted_app"
  xcrun stapler validate "$dmg"
fi

echo "Verified Apple Silicon macOS $mode app and DMG:"
echo "  $app"
echo "  $dmg"
if [[ -n "$updater_archive" ]]; then
  echo "  updater archive app: $extracted_app"
fi
