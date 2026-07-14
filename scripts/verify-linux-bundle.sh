#!/usr/bin/env bash
set -euo pipefail

readonly inspection_timeout_seconds=120
readonly max_glibc_major=2
readonly max_glibc_minor=35
readonly expected_deb_architecture="amd64"
readonly expected_elf_machine="Advanced Micro Devices X86-64"

fail() {
  echo "$*" >&2
  exit 1
}

run_bounded() {
  timeout --signal=KILL "${inspection_timeout_seconds}s" "$@"
}

read_deb_field() {
  local deb="$1"
  local field="$2"
  local value

  if ! value="$(run_bounded dpkg-deb --field "$deb" "$field")"; then
    fail "Could not read deb field $field."
  fi
  if [[ -z "$value" || "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    fail "Deb field $field is missing or malformed."
  fi
  printf '%s' "$value"
}

has_deb_dependency() {
  local dependencies="$1"
  local package="$2"
  local pattern="(^|,[[:space:]]*|\\|[[:space:]]*)${package}([[:space:]]*\\([^)]*\\))?([[:space:]]*([,|]|$))"
  [[ "$dependencies" =~ $pattern ]]
}

verify_elf() {
  local executable="$1"
  local label="$2"
  local require_executable="${3:-0}"
  local header

  if [[ ! -f "$executable" || -L "$executable" ]]; then
    fail "$label is missing or is not a regular file: $executable"
  fi
  if [[ "$require_executable" -eq 1 && ! -x "$executable" ]]; then
    fail "$label is not executable: $executable"
  fi
  if ! header="$(run_bounded readelf --wide --file-header -- "$executable")"; then
    fail "$label is not a readable ELF file: $executable"
  fi
  [[ "$header" =~ Class:[[:space:]]+ELF64 ]] ||
    fail "$label must be ELF64: $executable"
  [[ "$header" =~ Machine:[[:space:]]+${expected_elf_machine} ]] ||
    fail "$label must target x86-64; readelf reported a different machine: $executable"
  if [[ "$require_executable" -eq 1 ]]; then
    [[ "$header" =~ Type:[[:space:]]+(EXEC|DYN)[[:space:]] ]] ||
      fail "$label must have an executable ELF type: $executable"
  fi
}

verify_glibc_floor() {
  local executable="$1"
  local label="$2"
  local allow_no_references="${3:-0}"
  local version_info
  local remaining
  local seen=0
  local major
  local minor
  local matched
  local major_text
  local minor_text

  if ! version_info="$(run_bounded readelf --wide --version-info -- "$executable")"; then
    fail "Could not inspect glibc requirements for $label: $executable"
  fi

  while IFS= read -r remaining; do
    while [[ "$remaining" =~ GLIBC_([0-9]+)\.([0-9]+) ]]; do
      seen=1
      matched="${BASH_REMATCH[0]}"
      major_text="${BASH_REMATCH[1]}"
      minor_text="${BASH_REMATCH[2]}"
      if (( ${#major_text} > 3 || ${#minor_text} > 3 )); then
        fail "$label has a malformed GLIBC symbol version $matched: $executable"
      fi
      major=$((10#$major_text))
      minor=$((10#$minor_text))
      if (( major > max_glibc_major || (major == max_glibc_major && minor > max_glibc_minor) )); then
        fail "$label requires $matched, newer than the GLIBC_2.35 compatibility ceiling: $executable"
      fi
      remaining="${remaining#*"$matched"}"
    done
  done <<< "$version_info"

  if [[ "$seen" -eq 0 && "$allow_no_references" -eq 0 ]]; then
    fail "$label has no readable GLIBC symbol requirements: $executable"
  fi
}

list_payload_elfs() {
  local payload_root="$1"

  run_bounded python3 - "$payload_root" <<'PY'
import os
import stat
import sys

root = os.path.realpath(sys.argv[1])
files_seen = 0
output_bytes = 0
for directory, directories, files in os.walk(root, followlinks=False):
    directories.sort()
    files.sort()
    for name in files:
        files_seen += 1
        if files_seen > 20_000:
            raise SystemExit("extracted payload contains too many files")
        path = os.path.join(directory, name)
        metadata = os.lstat(path)
        if not stat.S_ISREG(metadata.st_mode):
            continue
        with open(path, "rb") as handle:
            if handle.read(4) != b"\x7fELF":
                continue
        if "\n" in path or "\r" in path:
            raise SystemExit("extracted ELF path contains a line break")
        output_bytes += len(os.fsencode(path)) + 1
        if output_bytes > 1024 * 1024:
            raise SystemExit("extracted ELF inventory is too large")
        print(path)
PY
}

verify_all_payload_elfs() {
  local payload_root="$1"
  local package_label="$2"
  local elf_list
  local executable

  if ! elf_list="$(list_payload_elfs "$payload_root")"; then
    fail "Could not enumerate $package_label ELF files."
  fi
  [[ -n "$elf_list" ]] || fail "$package_label contains no ELF files."
  while IFS= read -r executable; do
    verify_elf "$executable" "$package_label ELF"
    verify_glibc_floor "$executable" "$package_label ELF" 1
  done <<< "$elf_list"
}

verify_batcave_payload() {
  local payload_root="$1"
  local package_label="$2"
  local binary
  local executable

  verify_all_payload_elfs "$payload_root" "$package_label"
  for binary in batcave-monitor batcave-monitor-cli; do
    executable="$payload_root/usr/bin/$binary"
    verify_elf "$executable" "$package_label $binary" 1
    verify_glibc_floor "$executable" "$package_label $binary"
  done
}

find_appimage_squashfs_offset() {
  local appimage="$1"

  run_bounded python3 - "$appimage" <<'PY'
import mmap
import os
import struct
import sys

path = sys.argv[1]
size = os.path.getsize(path)
if size < 96 or size > 512 * 1024 * 1024:
    raise SystemExit("AppImage size is outside the static inspection boundary")

with open(path, "rb") as handle, mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ) as image:
    position = 0
    candidates = 0
    while True:
        offset = image.find(b"hsqs", position)
        if offset < 0:
            break
        position = offset + 1
        candidates += 1
        if candidates > 8:
            raise SystemExit("AppImage contains too many SquashFS candidates")
        if offset + 96 > size:
            continue
        fields = struct.unpack_from("<5I6H8Q", image, offset)
        block_size = fields[3]
        compression = fields[5]
        block_log = fields[6]
        major = fields[9]
        minor = fields[10]
        bytes_used = fields[12]
        if (
            fields[0] == 0x73717368
            and fields[1] > 0
            and 4096 <= block_size <= 1024 * 1024
            and block_size == 1 << block_log
            and 1 <= compression <= 6
            and major == 4
            and minor == 0
            and 96 <= bytes_used <= size - offset
        ):
            print(offset)
            raise SystemExit(0)

raise SystemExit("AppImage does not contain a valid SquashFS superblock")
PY
}

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

for tool in dpkg-deb node python3 readelf timeout unsquashfs; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "verify-linux-bundle.sh requires $tool." >&2
    exit 2
  }
done

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
deb_version="$(read_deb_field "$deb" Version)"
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


deb_architecture="$(read_deb_field "$deb" Architecture)"
[[ "$deb_architecture" == "$expected_deb_architecture" ]] ||
  fail "Expected deb architecture $expected_deb_architecture, found $deb_architecture."

deb_dependencies="$(read_deb_field "$deb" Depends)"
has_deb_dependency "$deb_dependencies" "libgtk-3-0" ||
  fail "Deb dependencies must include GTK3 package libgtk-3-0."
has_deb_dependency "$deb_dependencies" "libwebkit2gtk-4.1-0" ||
  fail "Deb dependencies must include WebKitGTK 4.1 package libwebkit2gtk-4.1-0."

umask 077
workspace=""
cleanup() {
  local status=$?
  trap - EXIT
  if [[ -n "$workspace" && -e "$workspace" ]]; then
    if ! rm -rf -- "$workspace" || [[ -e "$workspace" ]]; then
      echo "Failed to remove Linux bundle verification workspace: $workspace" >&2
      status=1
    fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

temp_base="${TMPDIR:-/tmp}"
workspace="$(mktemp -d "${temp_base%/}/batcave-linux-bundle.XXXXXX")"
chmod 700 "$workspace"
deb_root="$workspace/deb"
appimage_root="$workspace/appimage"
mkdir -m 700 "$deb_root"

if ! run_bounded dpkg-deb --extract "$deb" "$deb_root"; then
  fail "Could not extract the deb payload for static inspection."
fi
verify_batcave_payload "$deb_root" "deb payload"

verify_elf "$appimage" "AppImage runtime" 1
verify_glibc_floor "$appimage" "AppImage runtime" 1
if ! squashfs_offset="$(find_appimage_squashfs_offset "$appimage")"; then
  fail "Could not locate a valid AppImage SquashFS payload."
fi
if ! run_bounded unsquashfs \
  -no-progress \
  -no-xattrs \
  -strict-errors \
  -processors 2 \
  -data-queue 64 \
  -frag-queue 64 \
  -dest "$appimage_root" \
  -offset "$squashfs_offset" \
  "$appimage" >/dev/null; then
  fail "Could not extract the AppImage payload for static inspection."
fi
verify_batcave_payload "$appimage_root" "AppImage payload"

echo "Verified Linux package compatibility through GLIBC_2.35 for version $cargo_version:"
echo "  $deb"
echo "  $appimage"
