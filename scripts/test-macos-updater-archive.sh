#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
expected_app_name="BatCave Monitor.app"
workspace="$(mktemp -d)"
trap 'rm -rf -- "$workspace"' EXIT

make_archive_fixture() {
  local scenario="$1"
  local archive="$2"
  python3 - "$scenario" "$archive" "$expected_app_name" <<'PY'
import io
import sys
import tarfile

scenario, archive_path, app_name = sys.argv[1:]

def entry(archive, name, kind, data=b"fixture", linkname=""):
    info = tarfile.TarInfo(name)
    info.mode = 0o755 if kind == "directory" else 0o644
    if kind == "directory":
        info.type = tarfile.DIRTYPE
        info.size = 0
    elif kind == "file":
        info.type = tarfile.REGTYPE
        info.size = len(data)
    elif kind == "symlink":
        info.type = tarfile.SYMTYPE
        info.linkname = linkname
        info.size = 0
    elif kind == "hardlink":
        info.type = tarfile.LNKTYPE
        info.linkname = linkname
        info.size = 0
    elif kind == "device":
        info.type = tarfile.CHRTYPE
        info.devmajor = 1
        info.devminor = 3
        info.size = 0
    archive.addfile(info, io.BytesIO(data) if kind == "file" else None)

with tarfile.open(archive_path, "w:gz", format=tarfile.PAX_FORMAT) as archive:
    if scenario not in {"absolute", "missing"}:
        entry(archive, app_name, "directory")
    if scenario == "valid":
        entry(archive, f"{app_name}/Contents", "directory")
        entry(archive, f"{app_name}/Contents/fixture", "file")
    elif scenario == "case-prefix-collision":
        entry(archive, f"{app_name}/Contents/A", "file")
        entry(archive, f"{app_name}/contents/B", "file")
    elif scenario == "unicode-prefix-collision":
        entry(archive, f"{app_name}/Re\N{COMBINING ACUTE ACCENT}sources/A", "file")
        entry(archive, f"{app_name}/R\N{LATIN SMALL LETTER E WITH ACUTE}sources/B", "file")
    elif scenario == "file-directory-conflict":
        entry(archive, f"{app_name}/Contents/Node", "file")
        entry(archive, f"{app_name}/Contents/node/child", "file")
    elif scenario == "traversal":
        entry(archive, f"{app_name}/../outside", "file")
    elif scenario == "absolute":
        entry(archive, f"/{app_name}/Contents/fixture", "file")
    elif scenario == "symlink":
        entry(archive, f"{app_name}/Contents/link", "symlink", linkname="/tmp")
    elif scenario == "hardlink":
        entry(archive, f"{app_name}/Contents/link", "hardlink", linkname=f"{app_name}/Contents/fixture")
    elif scenario == "device":
        entry(archive, f"{app_name}/Contents/device", "device")
    elif scenario == "extra-root":
        entry(archive, "README.txt", "file")
    elif scenario == "missing":
        entry(archive, "payload", "directory")
        entry(archive, "payload/fixture", "file")
    elif scenario == "multiple":
        entry(archive, "Other.app", "directory")
        entry(archive, "Other.app/Contents/fixture", "file")
PY
}

expect_extraction_rejected() {
  local scenario="$1"
  local expected_message="$2"
  local archive="$workspace/$scenario.app.tar.gz"
  local destination="$workspace/$scenario-extracted"
  local output

  make_archive_fixture "$scenario" "$archive"
  if output="$(python3 "$repo_root/scripts/extract-macos-updater-archive.py" \
    "$archive" "$destination" --expected-app-name "$expected_app_name" 2>&1)"; then
    echo "Expected $scenario archive fixture to fail." >&2
    exit 1
  fi
  grep -Fq "$expected_message" <<<"$output" || {
    echo "Unexpected $scenario rejection: $output" >&2
    exit 1
  }
  [[ ! -e "$destination" ]] || {
    echo "$scenario fixture materialized content before rejection." >&2
    exit 1
  }
}

make_archive_fixture valid "$workspace/valid.app.tar.gz"
extracted="$(python3 "$repo_root/scripts/extract-macos-updater-archive.py" \
  "$workspace/valid.app.tar.gz" "$workspace/valid-extracted" \
  --expected-app-name "$expected_app_name")"
[[ -f "$extracted/Contents/fixture" ]]

expect_extraction_rejected traversal "not canonical"
expect_extraction_rejected absolute "absolute or empty path"
expect_extraction_rejected symlink "symbolic link"
expect_extraction_rejected hardlink "hard link"
expect_extraction_rejected device "device entry"
expect_extraction_rejected extra-root "must contain only the expected"
expect_extraction_rejected missing "must contain only the expected"
expect_extraction_rejected multiple "must contain only the expected"
expect_extraction_rejected case-prefix-collision "filesystem-colliding path prefix"
expect_extraction_rejected unicode-prefix-collision "filesystem-colliding path prefix"
expect_extraction_rejected file-directory-conflict "both a file and directory"

python3 - "$workspace" "$repo_root/scripts/extract-macos-updater-archive.py" <<'PY'
import importlib.util
import io
import os
from pathlib import Path
import sys
import tarfile

workspace = Path(sys.argv[1])
extractor_path = Path(sys.argv[2])
spec = importlib.util.spec_from_file_location("batcave_archive_extractor", extractor_path)
extractor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extractor)

# Keep these contract values aligned with the release extractor and Rust verifier.
assert extractor.MAX_COMPRESSED_ARCHIVE_BYTES == 256 * 1024 * 1024
assert extractor.MAX_DECOMPRESSED_TAR_BYTES == 1152 * 1024 * 1024
assert extractor.MAX_MEMBER_COUNT == 50_000
assert extractor.MAX_PATH_DEPTH == 64
assert extractor.MAX_PATH_BYTES == 4_096
assert extractor.MAX_PATH_BOOKKEEPING_BYTES == 32 * 1024 * 1024
assert extractor.MAX_CANONICAL_PREFIXES == 100_000
assert extractor.MAX_FILE_BYTES == 256 * 1024 * 1024
assert extractor.MAX_EXPANDED_BYTES == 1024 * 1024 * 1024

app_name = "BatCave Monitor.app"
original_limits = {
    name: getattr(extractor, name)
    for name in (
        "MAX_COMPRESSED_ARCHIVE_BYTES",
        "MAX_DECOMPRESSED_TAR_BYTES",
        "MAX_MEMBER_COUNT",
        "MAX_PATH_DEPTH",
        "MAX_PATH_BYTES",
        "MAX_PATH_BOOKKEEPING_BYTES",
        "MAX_CANONICAL_PREFIXES",
        "MAX_FILE_BYTES",
        "MAX_EXPANDED_BYTES",
    )
}


def write_archive(path, files):
    with tarfile.open(path, "w:gz", format=tarfile.PAX_FORMAT) as archive:
        root = tarfile.TarInfo(app_name)
        root.type = tarfile.DIRTYPE
        root.mode = 0o755
        archive.addfile(root)
        for name, data in files:
            info = tarfile.TarInfo(f"{app_name}/{name}")
            info.mode = 0o644
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))


def expect_archive_rejected(label, archive, overrides, expected_message):
    destination = workspace / f"limit-{label}-out"
    for name, value in original_limits.items():
        setattr(extractor, name, value)
    for name, value in overrides.items():
        setattr(extractor, name, value)
    try:
        extractor.extract(archive, destination, app_name)
    except extractor.UnsafeArchive as error:
        if expected_message not in str(error):
            raise AssertionError(f"unexpected {label} rejection: {error}") from error
    else:
        raise AssertionError(f"expected {label} limit to reject the archive")
    if destination.exists():
        raise AssertionError(f"{label} limit materialized its destination")


def expect_limit_rejected(label, files, overrides, expected_message):
    archive = workspace / f"limit-{label}.tar.gz"
    write_archive(archive, files)
    expect_archive_rejected(label, archive, overrides, expected_message)


expect_limit_rejected(
    "compressed",
    [("fixture", b"fixture")],
    {"MAX_COMPRESSED_ARCHIVE_BYTES": 1},
    "compressed archive exceeds",
)
expect_limit_rejected(
    "members",
    [("A", b"a"), ("B", b"b")],
    {"MAX_MEMBER_COUNT": 2},
    "member limit",
)
expect_limit_rejected(
    "depth",
    [("one/two/three", b"x")],
    {"MAX_PATH_DEPTH": 3},
    "depth limit",
)
expect_limit_rejected(
    "path-bytes",
    [("a-longer-than-fixture-name", b"x")],
    {"MAX_PATH_BYTES": len(app_name.encode()) + 5},
    "byte limit",
)
expect_limit_rejected(
    "file-bytes",
    [("fixture", b"1234")],
    {"MAX_FILE_BYTES": 3},
    "archive file exceeds",
)
expect_limit_rejected(
    "expanded-bytes",
    [("A", b"1234"), ("B", b"5678")],
    {"MAX_EXPANDED_BYTES": 7},
    "expanded-size limit",
)
expect_limit_rejected(
    "canonical-prefixes",
    [("A/one", b"1"), ("B/two", b"2")],
    {"MAX_CANONICAL_PREFIXES": 3},
    "canonical-prefix limit",
)
expect_limit_rejected(
    "path-bookkeeping",
    [("A", b"1")],
    {"MAX_PATH_BOOKKEEPING_BYTES": len(app_name.encode()) + 1},
    "path-bookkeeping limit",
)

# The decompressed budget sits below tarfile parsing, so large extension records
# are rejected before they can become retained TarInfo/PAX state.
pax_archive = workspace / "limit-pax-metadata.tar.gz"
with tarfile.open(pax_archive, "w:gz", format=tarfile.PAX_FORMAT) as archive:
    root = tarfile.TarInfo(app_name)
    root.type = tarfile.DIRTYPE
    archive.addfile(root)
    info = tarfile.TarInfo(f"{app_name}/fixture")
    info.size = 0
    info.pax_headers = {"comment": "x" * 8_192}
    archive.addfile(info, io.BytesIO())
expect_archive_rejected(
    "pax-metadata",
    pax_archive,
    {"MAX_DECOMPRESSED_TAR_BYTES": 2_048},
    "decompressed tar stream exceeds",
)

gnu_archive = workspace / "limit-gnu-metadata.tar.gz"
with tarfile.open(gnu_archive, "w:gz", format=tarfile.GNU_FORMAT) as archive:
    root = tarfile.TarInfo(app_name)
    root.type = tarfile.DIRTYPE
    archive.addfile(root)
    info = tarfile.TarInfo(f"{app_name}/{'x' * 8_192}")
    info.size = 0
    archive.addfile(info, io.BytesIO())
expect_archive_rejected(
    "gnu-metadata",
    gnu_archive,
    {"MAX_DECOMPRESSED_TAR_BYTES": 2_048},
    "decompressed tar stream exceeds",
)

if hasattr(os, "O_NOFOLLOW"):
    symlink_target = workspace / "symlink-target.tar.gz"
    write_archive(symlink_target, [("fixture", b"fixture")])
    symlink_archive = workspace / "symlink-archive.tar.gz"
    symlink_archive.symlink_to(symlink_target)
    expect_archive_rejected(
        "archive-symlink",
        symlink_archive,
        {},
        "must not be a symbolic link",
    )

# Replace the pathname after the first descriptor pass. The already-open source
# loses a link, so fstat rejects the drift and no code ever reopens the new path.
replacement_archive = workspace / "replacement-source.tar.gz"
replacement_payload = workspace / "replacement-payload.tar.gz"
write_archive(replacement_archive, [("original", b"original")])
write_archive(replacement_payload, [("replacement", b"replacement")])
original_copy_descriptor_pass = extractor.copy_descriptor_pass
descriptor_passes = 0


def replace_after_first_pass(source, size, output):
    global descriptor_passes
    digest = original_copy_descriptor_pass(source, size, output)
    descriptor_passes += 1
    if descriptor_passes == 1:
        replacement_payload.replace(replacement_archive)
    return digest


extractor.copy_descriptor_pass = replace_after_first_pass
try:
    expect_archive_rejected(
        "archive-replacement",
        replacement_archive,
        {},
        "descriptor changed",
    )
finally:
    extractor.copy_descriptor_pass = original_copy_descriptor_pass

try:
    extractor.copy_member(io.BytesIO(b"1234"), io.BytesIO(), 4, 3)
except extractor.UnsafeArchive as error:
    assert "expanded-size budget" in str(error)
else:
    raise AssertionError("expected remaining extraction budget to be enforced")

for name, value in original_limits.items():
    setattr(extractor, name, value)
PY

python3 - "$workspace" <<'PY'
import base64
import json
from pathlib import Path
import sys

workspace = Path(sys.argv[1])
public_key = """untrusted comment: minisign public key E7620F1842B4E81F
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3"""
signature = """untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA=="""
(workspace / "signed.app.tar.gz").write_bytes(b"test")
(workspace / "signed.app.tar.gz.sig").write_text(
    base64.b64encode(signature.encode()).decode() + "\n"
)
(workspace / "tauri.conf.json").write_text(
    json.dumps({"plugins": {"updater": {"pubkey": base64.b64encode(public_key.encode()).decode()}}})
)
PY

signature_command=(cargo run --quiet --locked
  --manifest-path "$repo_root/src/BatCave.App/src-tauri/Cargo.toml"
  --bin batcave-verify-updater-signature --)
"${signature_command[@]}" \
  "$workspace/signed.app.tar.gz" "$workspace/signed.app.tar.gz.sig" \
  "$workspace/tauri.conf.json" "$workspace/verified-copy.app.tar.gz"
cmp "$workspace/signed.app.tar.gz" "$workspace/verified-copy.app.tar.gz"

printf 'drift' >> "$workspace/signed.app.tar.gz"
if "${signature_command[@]}" \
  "$workspace/signed.app.tar.gz" "$workspace/signed.app.tar.gz.sig" \
  "$workspace/tauri.conf.json" "$workspace/drift-copy.app.tar.gz" \
  >"$workspace/signature-drift.log" 2>&1; then
  echo "Expected updater archive byte drift to fail signature verification." >&2
  exit 1
fi
grep -Fq "updater signature verification failed" "$workspace/signature-drift.log"
[[ ! -e "$workspace/drift-copy.app.tar.gz" ]]

# This sparse fixture proves the Rust exact-byte buffer shares the 256 MiB ceiling.
python3 - "$workspace/oversized.app.tar.gz" <<'PY'
from pathlib import Path
import sys

with Path(sys.argv[1]).open("wb") as oversized:
    oversized.truncate(256 * 1024 * 1024 + 1)
PY
if "${signature_command[@]}" \
  "$workspace/oversized.app.tar.gz" "$workspace/signed.app.tar.gz.sig" \
  "$workspace/tauri.conf.json" "$workspace/oversized-copy.app.tar.gz" \
  >"$workspace/oversized.log" 2>&1; then
  echo "Expected oversized updater archive to fail before signature verification." >&2
  exit 1
fi
grep -Fq "compressed updater archive exceeds the 268435456-byte limit" \
  "$workspace/oversized.log"
[[ ! -e "$workspace/oversized-copy.app.tar.gz" ]]

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Updater archive preflight and exact-byte signature fixtures passed; skipped macOS app fixtures."
  exit 0
fi

cargo_version="$(node "$repo_root/scripts/verify-release-version.mjs" --print)"
bundle_root="$workspace/bundle"
app="$bundle_root/macos/$expected_app_name"
mkdir -p "$app/Contents/MacOS" "$bundle_root/dmg"
python3 - "$app/Contents/Info.plist" "$cargo_version" <<'PY'
import plistlib
from pathlib import Path
import sys

path, version = sys.argv[1:]
Path(path).write_bytes(plistlib.dumps({
    "CFBundleExecutable": "batcave-fixture",
    "CFBundleIdentifier": "dev.batcave.monitor",
    "CFBundleName": "BatCave Monitor",
    "CFBundleShortVersionString": version,
    "CFBundleVersion": version,
    "LSMinimumSystemVersion": "12.0",
}))
PY
printf 'int main(void) { return 0; }\n' | clang -x c -arch arm64 -mmacosx-version-min=12.0 - -o "$workspace/fixture-arm64"
cp "$workspace/fixture-arm64" "$app/Contents/MacOS/batcave-fixture"
codesign --force --options runtime --sign - "$app"

mkdir -p "$workspace/dmg-source"
ditto "$app" "$workspace/dmg-source/$expected_app_name"
hdiutil create -quiet -ov -format UDZO -srcfolder "$workspace/dmg-source" \
  "$bundle_root/dmg/BatCave Monitor_${cargo_version}_aarch64.dmg"

archive_app() {
  local source_app="$1"
  local archive="$2"
  python3 - "$source_app" "$archive" "$expected_app_name" <<'PY'
import gzip
from pathlib import Path
import sys
import tarfile

source, output, app_name = sys.argv[1:]
with Path(output).open("wb") as raw:
    with gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
            archive.add(source, arcname=app_name, recursive=True)
PY
}

valid_app_archive="$workspace/real-adhoc.app.tar.gz"
archive_app "$app" "$valid_app_archive"
bash "$repo_root/scripts/verify-macos-bundle.sh" --mode adhoc \
  --bundle-root "$bundle_root" --updater-archive "$valid_app_archive"

expect_bundle_rejected() {
  local label="$1"
  local expected_message="$2"
  local archive="$3"
  local output
  if output="$(bash "$repo_root/scripts/verify-macos-bundle.sh" --mode adhoc \
    --bundle-root "$bundle_root" --updater-archive "$archive" 2>&1)"; then
    echo "Expected $label updater app fixture to fail." >&2
    exit 1
  fi
  grep -Fq "$expected_message" <<<"$output" || {
    echo "Unexpected $label rejection: $output" >&2
    exit 1
  }
}

identity_app="$workspace/Identity Drift.app"
ditto "$app" "$identity_app"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier dev.batcave.drifted' \
  "$identity_app/Contents/Info.plist"
codesign --force --options runtime --sign - "$identity_app"
archive_app "$identity_app" "$workspace/identity-drift.app.tar.gz"
expect_bundle_rejected identity "Expected CFBundleIdentifier dev.batcave.monitor" \
  "$workspace/identity-drift.app.tar.gz"

architecture_app="$workspace/Architecture Drift.app"
ditto "$app" "$architecture_app"
printf 'int main(void) { return 0; }\n' | clang -x c -arch x86_64 -mmacosx-version-min=12.0 - \
  -o "$architecture_app/Contents/MacOS/batcave-fixture"
codesign --force --options runtime --sign - "$architecture_app"
archive_app "$architecture_app" "$workspace/architecture-drift.app.tar.gz"
expect_bundle_rejected architecture "Expected an arm64 slice" \
  "$workspace/architecture-drift.app.tar.gz"

echo "Updater archive extraction, signature, identity, and architecture fixtures passed."
