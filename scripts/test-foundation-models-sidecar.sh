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
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--guard-controls-only" ) ]]; then
  echo "Usage: $0 [--guard-controls-only]" >&2
  exit 2
fi
guard_controls_only="${1:-}"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
source_root="$repo_root/src/BatCave.App/src-tauri/swift/foundation-models-sidecar"
sdk_path="$(xcrun --sdk macosx --show-sdk-path)"
sdk_version="$(xcrun --sdk macosx --show-sdk-version)"
test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT

compile_swift() {
  local output="$1"
  local protocol_source="$2"
  local optimization="$3"
  shift 3
  xcrun --sdk macosx swiftc \
    "$protocol_source" \
    "$@" \
    -parse-as-library \
    -target arm64-apple-macos12.0 \
    -sdk "$sdk_path" \
    "$optimization" \
    -framework Foundation \
    -Xlinker -weak_framework \
    -Xlinker FoundationModels \
    -o "$output"
}

expect_guard_failure() {
  local name="$1"
  local protocol_source="$2"
  local expected_failure="$3"
  local executable="$test_root/${name}-control"
  local log="$test_root/${name}-control.log"
  compile_swift "$executable" "$protocol_source" -Onone "$source_root/SidecarProtocolTests.swift"
  if "$executable" >"$log" 2>&1; then
    echo "Disabling the $name guard unexpectedly passed its control." >&2
    exit 1
  fi
  if ! grep -Fq "$expected_failure" "$log"; then
    cat "$log" >&2
    echo "The $name control failed for an unrelated reason." >&2
    exit 1
  fi
  echo "Confirmed the test rejects disabled $name guard."
}

test_guard_controls() {
  local baseline="$test_root/foundation-models-sidecar-control-baseline"
  compile_swift "$baseline" "$source_root/SidecarProtocol.swift" -Onone \
    "$source_root/SidecarProtocolTests.swift"
  "$baseline"

  sed '/!generation\.factDigest\.isEmpty,/d' "$source_root/SidecarProtocol.swift" \
    >"$test_root/no-empty-digest-guard.swift"
  expect_guard_failure \
    "empty-digest" \
    "$test_root/no-empty-digest-guard.swift" \
    "empty fact digest was accepted"

  sed 's/guard !data.isEmpty, data.count <= maximumInputBytes else {/guard !data.isEmpty else {/' \
    "$source_root/SidecarProtocol.swift" >"$test_root/no-input-size-guard.swift"
  expect_guard_failure \
    "input-size" \
    "$test_root/no-input-size-guard.swift" \
    "oversized request was accepted"

  sed '/guard data.count + 1 <= maximumOutputBytes else {/,/^    }/d' \
    "$source_root/SidecarProtocol.swift" >"$test_root/no-output-size-guard.swift"
  expect_guard_failure \
    "output-size" \
    "$test_root/no-output-size-guard.swift" \
    "oversized response was accepted"
}

if [[ "$guard_controls_only" == "--guard-controls-only" ]]; then
  test_guard_controls
  exit 0
fi

protocol_tests="$test_root/foundation-models-sidecar-tests"
sidecar="$test_root/batcave-foundation-models"
unavailable_sidecar="$test_root/batcave-foundation-models-unavailable"
compile_swift "$protocol_tests" "$source_root/SidecarProtocol.swift" -O "$source_root/SidecarProtocolTests.swift"
compile_swift "$sidecar" "$source_root/SidecarProtocol.swift" -O "$source_root/FoundationModelsSidecar.swift"
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
