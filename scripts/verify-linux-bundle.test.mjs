import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readCargoVersion } from "./verify-release-version.mjs";

const repoRoot = fileURLToPath(new URL("../", import.meta.url));
const verifier = fileURLToPath(new URL("./verify-linux-bundle.sh", import.meta.url));
const cargoVersion = readCargoVersion(repoRoot);

function writeExecutable(file, source) {
  fs.writeFileSync(file, source, { mode: 0o755 });
}

function writeAppImageFixture(file, validSquashfs = true) {
  const image = Buffer.alloc(256);
  image.set(Buffer.from([0x7f, 0x45, 0x4c, 0x46]), 0);
  if (validSquashfs) {
    const offset = 64;
    image.write("hsqs", offset, "ascii");
    image.writeUInt32LE(2, offset + 4);
    image.writeUInt32LE(131_072, offset + 12);
    image.writeUInt16LE(1, offset + 20);
    image.writeUInt16LE(17, offset + 22);
    image.writeUInt16LE(4, offset + 28);
    image.writeUInt16LE(0, offset + 30);
    image.writeBigUInt64LE(192n, offset + 40);
  }
  fs.writeFileSync(file, image, { mode: 0o755 });
}

function createFixture(scenario = "success", { wrongFilenames = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-bundle-test-"));
  const bin = path.join(root, "bin");
  const bundle = path.join(root, "bundle");
  const temp = path.join(root, "tmp");
  fs.mkdirSync(bin);
  fs.mkdirSync(path.join(bundle, "deb"), { recursive: true });
  fs.mkdirSync(path.join(bundle, "appimage"), { recursive: true });
  fs.mkdirSync(temp);

  writeExecutable(path.join(bin, "uname"), "#!/usr/bin/env bash\nprintf 'Linux\\n'\n");
  writeExecutable(
    path.join(bin, "timeout"),
    "#!/usr/bin/env bash\nset -euo pipefail\nshift 2\nexec \"$@\"\n",
  );
  writeExecutable(
    path.join(bin, "dpkg-deb"),
    `#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "--field" ]]; then
  case "$3" in
    Version)
      if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "malformed_metadata" ]]; then
        printf '${cargoVersion}\\nInjected: true\\n'
      else
        printf '${cargoVersion}\\n'
      fi
      ;;
    Architecture)
      if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "wrong_deb_arch" ]]; then
        printf 'arm64\\n'
      else
        printf 'amd64\\n'
      fi
      ;;
    Depends)
      case "\${BATCAVE_TEST_SCENARIO:-}" in
        missing_gtk) printf 'libwebkit2gtk-4.1-0\\n' ;;
        missing_webkit) printf 'libgtk-3-0\\n' ;;
        *) printf 'libwebkit2gtk-4.1-0 (>= 2.38), libgtk-3-0 | libgtk-3-0t64\\n' ;;
      esac
      ;;
    *) exit 4 ;;
  esac
elif [[ "$1" == "--extract" ]]; then
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "deb_extraction_failure" ]]; then
    exit 7
  fi
  destination="$3"
  mkdir -p "$destination/usr/bin"
  printf '\\177ELFfixture' > "$destination/usr/bin/batcave-monitor"
  chmod 755 "$destination/usr/bin/batcave-monitor"
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" != "missing_deb_binary" ]]; then
    printf '\\177ELFfixture' > "$destination/usr/bin/batcave-monitor-cli"
    chmod 755 "$destination/usr/bin/batcave-monitor-cli"
  fi
else
  exit 4
fi
`,
  );
  writeExecutable(
    path.join(bin, "unsquashfs"),
    `#!/usr/bin/env bash
set -euo pipefail
if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "appimage_extraction_failure" ]]; then
  exit 9
fi
destination=""
processors=""
data_queue=""
fragment_queue=""
filesystem=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -no-progress|-no-xattrs|-strict-errors) shift ;;
    -processors) processors="$2"; shift 2 ;;
    -data-queue) data_queue="$2"; shift 2 ;;
    -frag-queue) fragment_queue="$2"; shift 2 ;;
    -dest) destination="$2"; shift 2 ;;
    -offset) shift 2 ;;
    *.AppImage) filesystem="$1"; shift ;;
    *) exit 4 ;;
  esac
done
[[ -n "$destination" ]] || exit 4
[[ -n "$filesystem" ]] || exit 4
[[ "$processors" == "2" ]] || exit 4
[[ "$data_queue" == "64" ]] || exit 4
[[ "$fragment_queue" == "64" ]] || exit 4
[[ ! -e "$destination" ]] || exit 4
mkdir -p "$destination/usr/bin"
printf '\\177ELFfixture' > "$destination/usr/bin/batcave-monitor"
chmod 755 "$destination/usr/bin/batcave-monitor"
if [[ "\${BATCAVE_TEST_SCENARIO:-}" != "missing_appimage_binary" ]]; then
  printf '\\177ELFfixture' > "$destination/usr/bin/batcave-monitor-cli"
  chmod 755 "$destination/usr/bin/batcave-monitor-cli"
fi
if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "extra_elf_glibc_236" ]]; then
  mkdir -p "$destination/usr/lib"
  printf '\\177ELFfixture' > "$destination/usr/lib/batcave-extra.so"
  chmod 644 "$destination/usr/lib/batcave-extra.so"
fi
`,
  );
  writeExecutable(
    path.join(bin, "readelf"),
    `#!/usr/bin/env bash
set -euo pipefail
mode=""
file=""
for argument in "$@"; do
  case "$argument" in
    --file-header) mode="header" ;;
    --version-info) mode="version" ;;
    --) ;;
    *) file="$argument" ;;
  esac
done
if [[ "$mode" == "header" ]]; then
  machine='Advanced Micro Devices X86-64'
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "wrong_elf_machine" && "$file" == */deb/usr/bin/batcave-monitor-cli ]]; then
    machine='AArch64'
  fi
  printf 'ELF Header:\\n  Class:                             ELF64\\n  Type:                              DYN (Position-Independent Executable file)\\n  Machine:                           %s\\n' "$machine"
elif [[ "$mode" == "version" ]]; then
  version='2.35'
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "glibc_236" && "$file" == */deb/usr/bin/batcave-monitor-cli ]]; then
    version='2.36'
  fi
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "extra_elf_glibc_236" && "$file" == */usr/lib/batcave-extra.so ]]; then
    version='2.36'
  fi
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "glibc_version_overflow" && "$file" == */deb/usr/bin/batcave-monitor-cli ]]; then
    version='18446744073709551618.35'
  fi
  if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "missing_glibc_metadata" && "$file" == */deb/usr/bin/batcave-monitor-cli ]]; then
    printf 'No version information found in this file.\\n'
  else
    printf "  0x0010:   Name: GLIBC_%s  Flags: none  Version: 2\\n" "$version"
  fi
else
  exit 4
fi
`,
  );
  writeExecutable(
    path.join(bin, "rm"),
    `#!/usr/bin/env bash
set -euo pipefail
if [[ "\${BATCAVE_TEST_SCENARIO:-}" == "cleanup_failure" && "$*" == *batcave-linux-bundle.* ]]; then
  exit 11
fi
exec /bin/rm "$@"
`,
  );

  const debName = wrongFilenames
    ? "BatCave_wrong_amd64.deb"
    : `BatCave.Monitor_${cargoVersion}_amd64.deb`;
  const appImageName = wrongFilenames
    ? "BatCave_wrong_amd64.AppImage"
    : `BatCave.Monitor_${cargoVersion}_amd64.AppImage`;
  fs.writeFileSync(path.join(bundle, "deb", debName), "fixture");
  writeAppImageFixture(
    path.join(bundle, "appimage", appImageName),
    scenario !== "malformed_appimage",
  );

  return { root, bin, bundle, temp };
}

function runVerifier(fixture, scenario) {
  return spawnSync("bash", [verifier, "--bundle-root", fixture.bundle], {
    encoding: "utf8",
    env: {
      ...process.env,
      BATCAVE_TEST_SCENARIO: scenario,
      PATH: `${fixture.bin}${path.delimiter}${process.env.PATH}`,
      TMPDIR: fixture.temp,
    },
  });
}

function verificationWorkspaces(fixture) {
  return fs
    .readdirSync(fixture.temp)
    .filter((entry) => entry.startsWith("batcave-linux-bundle."));
}

function withFixture(scenario, assertion, options) {
  const fixture = createFixture(scenario, options);
  try {
    assertion(runVerifier(fixture, scenario), fixture);
  } finally {
    fs.rmSync(fixture.root, { recursive: true, force: true });
  }
}

test("accepts amd64 GTK3/WebKitGTK 4.1 packages through GLIBC_2.35 and cleans up", () => {
  withFixture("success", (result, fixture) => {
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /through GLIBC_2\.35/u);
    assert.deepEqual(verificationWorkspaces(fixture), []);
  });
});

test("filename mismatch reports the Cargo version", () => {
  withFixture(
    "success",
    (result) => {
      assert.equal(result.status, 1);
      assert.match(
        result.stderr,
        new RegExp(`Expected BatCave_wrong_amd64\\.deb to contain version _${cargoVersion}_`, "u"),
      );
      assert.doesNotMatch(result.stderr, /unbound variable/u);
    },
    { wrongFilenames: true },
  );
});

const hostileCases = [
  ["rejects malformed deb metadata", "malformed_metadata", /field Version is missing or malformed/u],
  ["rejects the wrong deb architecture", "wrong_deb_arch", /Expected deb architecture amd64, found arm64/u],
  ["rejects a missing GTK3 dependency", "missing_gtk", /must include GTK3 package libgtk-3-0/u],
  [
    "rejects a missing WebKitGTK 4.1 dependency",
    "missing_webkit",
    /must include WebKitGTK 4\.1 package libwebkit2gtk-4\.1-0/u,
  ],
  ["rejects deb extraction failure", "deb_extraction_failure", /Could not extract the deb payload/u],
  ["rejects a missing BatCave executable", "missing_deb_binary", /deb payload batcave-monitor-cli is missing/u],
  ["rejects the wrong ELF machine", "wrong_elf_machine", /must target x86-64/u],
  ["rejects GLIBC_2.36", "glibc_236", /requires GLIBC_2\.36, newer than the GLIBC_2\.35/u],
  [
    "rejects an extra shipped ELF requiring GLIBC_2.36",
    "extra_elf_glibc_236",
    /requires GLIBC_2\.36, newer than the GLIBC_2\.35/u,
  ],
  [
    "rejects oversized GLIBC version components before arithmetic",
    "glibc_version_overflow",
    /malformed GLIBC symbol version GLIBC_18446744073709551618\.35/u,
  ],
  [
    "rejects incomplete glibc metadata",
    "missing_glibc_metadata",
    /has no readable GLIBC symbol requirements/u,
  ],
  ["rejects a malformed AppImage payload", "malformed_appimage", /valid AppImage SquashFS payload/u],
  [
    "rejects AppImage extraction failure",
    "appimage_extraction_failure",
    /Could not extract the AppImage payload/u,
  ],
  [
    "rejects a missing AppImage executable",
    "missing_appimage_binary",
    /AppImage payload batcave-monitor-cli is missing/u,
  ],
];

for (const [name, scenario, expected] of hostileCases) {
  test(name, () => {
    withFixture(scenario, (result, fixture) => {
      assert.equal(result.status, 1, `${result.stdout}\n${result.stderr}`);
      assert.match(result.stderr, expected);
      assert.deepEqual(verificationWorkspaces(fixture), []);
    });
  });
}

test("fails closed when private-workspace cleanup fails", () => {
  withFixture("cleanup_failure", (result, fixture) => {
    assert.equal(result.status, 1, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stderr, /Failed to remove Linux bundle verification workspace/u);
    assert.equal(verificationWorkspaces(fixture).length, 1);
  });
});
