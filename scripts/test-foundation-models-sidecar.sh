#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Foundation Models sidecar tests require macOS." >&2
  exit 2
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "Foundation Models sidecar tests require Apple Silicon." >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
source_root="$repo_root/src/BatCave.App/src-tauri/swift/foundation-models-sidecar"
sdk_path="$(xcrun --sdk macosx --show-sdk-path)"
sdk_version="$(xcrun --sdk macosx --show-sdk-version)"
test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT

compile_swift() {
  local output="$1"
  shift
  xcrun --sdk macosx swiftc \
    "$source_root/SidecarProtocol.swift" \
    "$@" \
    -parse-as-library \
    -target arm64-apple-macos12.0 \
    -sdk "$sdk_path" \
    -O \
    -framework Foundation \
    -Xlinker -weak_framework \
    -Xlinker FoundationModels \
    -o "$output"
}

protocol_tests="$test_root/foundation-models-sidecar-tests"
sidecar="$test_root/batcave-foundation-models"
unavailable_sidecar="$test_root/batcave-foundation-models-unavailable"
compile_swift "$protocol_tests" "$source_root/SidecarProtocolTests.swift"
compile_swift "$sidecar" "$source_root/FoundationModelsSidecar.swift"
"$protocol_tests"

xcrun --sdk macosx swiftc \
  "$source_root/SidecarProtocol.swift" \
  "$source_root/FoundationModelsSidecar.swift" \
  -parse-as-library \
  -D BATCAVE_FOUNDATION_MODELS_UNAVAILABLE \
  -target arm64-apple-macos12.0 \
  -sdk "$sdk_path" \
  -O \
  -framework Foundation \
  -o "$unavailable_sidecar"
unavailable_status_json="$(printf '%s\n' '{"version":1,"operation":"status"}' | "$unavailable_sidecar")"
node -e '
  const response = JSON.parse(process.argv[1]);
  if (response.version !== 1 || response.availability !== "unsupported" || response.result !== undefined) process.exit(1);
' "$unavailable_status_json"
if otool -L "$unavailable_sidecar" | grep -q 'FoundationModels.framework'; then
  echo "Unavailable Foundation Models sidecar unexpectedly links the framework." >&2
  exit 1
fi

lipo "$sidecar" -verify_arch arm64
if lipo "$sidecar" -verify_arch x86_64 >/dev/null 2>&1; then
  echo "Foundation Models sidecar unexpectedly contains an Intel slice." >&2
  exit 1
fi
minos="$(vtool -show-build "$sidecar" | awk '$1 == "minos" { print $2; exit }')"
[[ "$minos" == "12.0" ]] || {
  echo "Expected Foundation Models sidecar deployment target 12.0, found $minos." >&2
  exit 1
}
otool -l "$sidecar" | awk '
  $1 == "cmd" { command = $2 }
  $1 == "name" && $2 ~ /FoundationModels\.framework/ && command == "LC_LOAD_WEAK_DYLIB" { weak = 1 }
  END { exit weak ? 0 : 1 }
' || {
  echo "FoundationModels.framework is not weak-linked." >&2
  exit 1
}
otool -l "$sidecar" | awk '
  $1 == "cmd" { command = $2 }
  $1 == "name" && $2 ~ /FoundationModels\.framework/ && command == "LC_LOAD_DYLIB" { strong = 1 }
  END { exit strong ? 0 : 1 }
' && {
  echo "FoundationModels.framework is also strongly linked." >&2
  exit 1
}

status_json="$(printf '%s\n' '{"version":1,"operation":"status"}' | "$sidecar")"
availability="$(node -e '
  const response = JSON.parse(process.argv[1]);
  const allowed = new Set(["available", "unsupported", "model_not_ready", "runtime_missing", "busy"]);
  if (response.version !== 1 || !allowed.has(response.availability) || response.result !== undefined) process.exit(1);
  process.stdout.write(response.availability);
' "$status_json")"

invalid_json="$(printf '%s\n' '{"version":99,"operation":"status"}' | "$sidecar")"
node -e '
  const response = JSON.parse(process.argv[1]);
  if (response.version !== 1 || response.availability !== "unsupported") process.exit(1);
' "$invalid_json"

echo "Verified Foundation Models sidecar with macOS SDK $sdk_version."
echo "FOUNDATION_MODELS_AVAILABILITY=$availability"
