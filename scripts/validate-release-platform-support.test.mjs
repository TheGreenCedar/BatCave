import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  RELEASE_PLATFORM_SUPPORT_CONTRACT,
  RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE,
  validateReleasePlatformSupport,
  validateReleasePlatformSupportContract,
} from "./validate-release-platform-support.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const LINUX_CLI_DESTINATION = "/usr/bin/batcave-monitor-cli";
const LINUX_CLI_SOURCE = "target/release/batcave-monitor-cli";
const PROFILES = new Map(
  RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.map((profile) => [profile.id, profile]),
);

function platform(profileId, host, packageKind, architecture) {
  const profile = PROFILES.get(profileId);
  const package_ = profile.packages.find(({ kind }) => kind === packageKind);
  return {
    support_contract_version: 1,
    profile_id: profileId,
    proof: {
      declaration: "declared",
      source: "source_enforced",
      native: "observed",
    },
    os: profile.os,
    os_version: host,
    architecture: architecture ?? profile.host_architectures[0],
    runtime: structuredClone(profile.runtime),
    package: {
      kind: packageKind,
      architecture: package_?.architecture ?? "x86_64",
      asset_name: "contract-test-asset",
    },
  };
}

function fixturePlatform(profileId, packageKind, architecture) {
  const value = platform(profileId, "unused", packageKind, architecture);
  value.os_version = RELEASE_PLATFORM_SUPPORT_CONTRACT.fixture_hosts[value.os];
  value.proof.source = "pending";
  value.proof.native = "pending";
  return value;
}

function cloneContract() {
  return structuredClone(RELEASE_PLATFORM_SUPPORT_CONTRACT);
}

function withMutatedTauri(sources, mutate) {
  const tauri = structuredClone(sources.tauri);
  mutate(tauri);
  return { ...sources, tauri };
}

function workflowJob(source, id) {
  const lines = source.split("\n");
  const start = lines.findIndex((line) => line === `  ${id}:`);
  assert.notEqual(start, -1, `workflow must define the ${id} job`);
  let end = lines.findIndex((line, index) => index > start && /^  [a-z0-9_-]+:$/u.test(line));
  if (end === -1) end = lines.length;
  return lines.slice(start, end).join("\n");
}

function shellReadonly(source, name) {
  const match = new RegExp(`^readonly ${RegExp.escape(name)}=(?:"([^"]+)"|([^\\s]+))$`, "mu").exec(
    source,
  );
  assert.ok(match, `verifier must define readonly ${name}`);
  return match[1] ?? match[2];
}

function shellFunction(source, name) {
  const lines = source.split("\n");
  const start = lines.findIndex((line) => line === `${name}() {`);
  assert.notEqual(start, -1, `verifier must define ${name}`);
  const end = lines.findIndex((line, index) => index > start && line === "}");
  assert.notEqual(end, -1, `verifier must close ${name}`);
  return lines.slice(start, end + 1).join("\n");
}

function assertActiveLine(source, command, message) {
  assert.match(
    source,
    new RegExp(`^\\s*${RegExp.escape(command)}\\s*$`, "mu"),
    message ?? `source must actively run: ${command}`,
  );
}

function writeTestExecutable(file, source) {
  fs.writeFileSync(file, source, { mode: 0o755 });
}

function runBundleOnlyFromCleanState(host) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-tauri-bundle-order-"));
  const scripts = path.join(root, "scripts");
  const app = path.join(root, "src", "BatCave.App");
  const tauri = path.join(app, "src-tauri");
  const bin = path.join(root, "bin");
  const log = path.join(root, "commands.log");
  const cli = path.join(tauri, "target", "release", "batcave-monitor-cli");
  fs.mkdirSync(scripts, { recursive: true });
  fs.mkdirSync(tauri, { recursive: true });
  fs.mkdirSync(bin);
  fs.copyFileSync(path.join(ROOT, "scripts", "validate-tauri.sh"), path.join(scripts, "validate-tauri.sh"));
  fs.copyFileSync(path.join(ROOT, "scripts", "build-linux-cli.sh"), path.join(scripts, "build-linux-cli.sh"));
  fs.writeFileSync(path.join(tauri, "Cargo.toml"), "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n");

  writeTestExecutable(
    path.join(bin, "uname"),
    `#!/bin/bash
if [[ "\${1:-}" == "-m" ]]; then
  printf 'arm64\\n'
else
  printf '%s\\n' "$BATCAVE_TEST_HOST"
fi
`,
  );
  writeTestExecutable(
    path.join(bin, "cargo"),
    `#!/bin/bash
set -euo pipefail
printf 'cargo|%s|%s\\n' "$BATCAVE_SOURCE_COMMIT_SHA" "$*" >> "$BATCAVE_TEST_LOG"
mkdir -p "$(dirname -- "$BATCAVE_TEST_CLI")"
: > "$BATCAVE_TEST_CLI"
`,
  );
  writeTestExecutable(
    path.join(bin, "npm"),
    `#!/bin/bash
set -euo pipefail
printf 'npm|%s\\n' "$*" >> "$BATCAVE_TEST_LOG"
if [[ "$BATCAVE_TEST_HOST" == "Linux" && "$*" == "run tauri -- build" ]]; then
  [[ -f "$BATCAVE_TEST_CLI" ]] || exit 71
fi
`,
  );
  writeTestExecutable(
    path.join(bin, "rustup"),
    `#!/bin/bash
set -euo pipefail
[[ "$*" == "target list --installed" ]] || exit 72
printf 'aarch64-apple-darwin\\n'
`,
  );
  writeTestExecutable(
    path.join(scripts, "verify-linux-bundle.sh"),
    `#!/bin/bash
set -euo pipefail
[[ -f "$BATCAVE_TEST_CLI" ]] || exit 73
printf 'verify-linux|%s\\n' "$*" >> "$BATCAVE_TEST_LOG"
`,
  );
  for (const [name, label] of [
    ["build-macos-arm64-cli.sh", "build-macos"],
    ["remove-macos-dmg-volume-icon.sh", "remove-dmg-volume-icon"],
    ["verify-macos-bundle.sh", "verify-macos"],
  ]) {
    writeTestExecutable(
      path.join(scripts, name),
      `#!/bin/bash\nset -euo pipefail\nprintf '${label}|%s\\n' "$*" >> "$BATCAVE_TEST_LOG"\n`,
    );
  }

  try {
    assert.equal(fs.existsSync(cli), false, "fixture must start without a cached release CLI");
    const result = spawnSync("/bin/bash", [path.join(scripts, "validate-tauri.sh"), "--bundle-only"], {
      encoding: "utf8",
      env: {
        ...process.env,
        BATCAVE_SOURCE_COMMIT_SHA: "0123456789abcdef0123456789abcdef01234567",
        BATCAVE_TEST_CLI: cli,
        BATCAVE_TEST_HOST: host,
        BATCAVE_TEST_LOG: log,
        PATH: `${bin}${path.delimiter}/usr/bin:/bin`,
      },
    });
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    return {
      cargoManifest: path.join(tauri, "Cargo.toml"),
      commands: fs.readFileSync(log, "utf8").trim().split("\n"),
    };
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

function linuxSourceFiles() {
  return {
    validationWorkflow: fs.readFileSync(
      path.join(ROOT, ".github", "workflows", "validation.yml"),
      "utf8",
    ),
    bundlesWorkflow: fs.readFileSync(
      path.join(ROOT, ".github", "workflows", "bundles.yml"),
      "utf8",
    ),
    releaseWorkflow: fs.readFileSync(
      path.join(ROOT, ".github", "workflows", "release.yml"),
      "utf8",
    ),
    buildLinuxCli: fs.readFileSync(path.join(ROOT, "scripts", "build-linux-cli.sh"), "utf8"),
    tauriValidator: fs.readFileSync(path.join(ROOT, "scripts", "validate-tauri.sh"), "utf8"),
    verifier: fs.readFileSync(path.join(ROOT, "scripts", "verify-linux-bundle.sh"), "utf8"),
    tauri: JSON.parse(
      fs.readFileSync(
        path.join(ROOT, "src", "BatCave.App", "src-tauri", "tauri.linux.conf.json"),
        "utf8",
      ),
    ),
  };
}

function assertLinuxSourceEnforcementMatchesContract(sources) {
  const source = RELEASE_PLATFORM_SUPPORT_CONTRACT.linux_source_enforcement;
  const validationJob = workflowJob(sources.validationWorkflow, "linux");
  const bundleJob = workflowJob(sources.bundlesWorkflow, "linux");
  const releaseJob = workflowJob(sources.releaseWorkflow, "linux");
  const runner = new RegExp(`^    runs-on: ${RegExp.escape(source.hosted_runner)}$`, "mu");
  assert.match(validationJob, runner, "validation Linux runner must match the machine contract");
  assert.match(bundleJob, runner, "bundle Linux runner must match the machine contract");
  assert.match(releaseJob, runner, "release Linux runner must match the machine contract");
  assertActiveLine(
    bundleJob,
    "run: bash scripts/validate-tauri.sh --bundle-only",
    "bundle Linux job must actively run the Tauri bundle validator",
  );
  assertActiveLine(
    sources.tauriValidator,
    'bash "$repo_root/scripts/build-linux-cli.sh"',
    "Tauri bundle validation must build the Linux release CLI before packaging",
  );
  assertActiveLine(
    sources.tauriValidator,
    'bash "$repo_root/scripts/verify-linux-bundle.sh"',
    "Tauri bundle validation must actively run Linux package verification",
  );
  assertActiveLine(
    releaseJob,
    "run: bash scripts/build-linux-cli.sh",
    "release Linux job must build the release CLI before signed packaging",
  );
  assertActiveLine(
    releaseJob,
    "run: bash scripts/verify-linux-bundle.sh",
    "release Linux job must actively run Linux package verification",
  );
  assert.match(sources.releaseWorkflow, /^  BATCAVE_SOURCE_COMMIT_SHA: \$\{\{ github\.sha \}\}$/mu);

  const validatorCli = sources.tauriValidator.indexOf(
    'bash "$repo_root/scripts/build-linux-cli.sh"',
  );
  const validatorBundle = sources.tauriValidator.indexOf("npm run tauri -- build", validatorCli);
  const validatorVerify = sources.tauriValidator.indexOf(
    'bash "$repo_root/scripts/verify-linux-bundle.sh"',
    validatorBundle,
  );
  assert.ok(validatorCli < validatorBundle && validatorBundle < validatorVerify);

  const releaseCli = releaseJob.indexOf("run: bash scripts/build-linux-cli.sh");
  const releaseBundle = releaseJob.indexOf(
    "run: npm run tauri -- build --config src-tauri/tauri.updater.conf.json",
  );
  const releaseVerify = releaseJob.indexOf("run: bash scripts/verify-linux-bundle.sh");
  assert.ok(releaseCli < releaseBundle && releaseBundle < releaseVerify);

  assert.match(sources.buildLinuxCli, /if \[\[ "\$\(uname -s\)" != "Linux" \]\]; then/u);
  assert.match(
    sources.buildLinuxCli,
    /cargo build \\\n+  --manifest-path "\$cargo_manifest" \\\n+  --release \\\n+  --bin batcave-monitor-cli/u,
  );

  const packages = new Map();
  for (const profile of RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.filter(
    ({ os }) => os === "linux",
  )) {
    assert.equal(profile.runtime.libc_family, source.libc_family);
    assert.deepEqual(profile.host_architectures, [source.architecture]);
    for (const package_ of profile.packages) packages.set(package_.kind, package_);
  }
  assert.deepEqual([...packages.keys()].sort(), [...sources.tauri.bundle.targets].sort());
  for (const packageKind of ["deb", "appimage"]) {
    assert.deepEqual(
      sources.tauri.bundle.linux?.[packageKind]?.files,
      { [LINUX_CLI_DESTINATION]: LINUX_CLI_SOURCE },
      `Tauri ${packageKind} config must package the release CLI at its installed path`,
    );
  }

  const assetRoles = new Map(
    expectedReleaseAssetRoles("v9.9.9").roles.map((role) => [role.role, role]),
  );
  for (const package_ of packages.values()) {
    const assetName = assetRoles.get(package_.asset_role)?.name;
    assert.ok(assetName, `release assets must define ${package_.asset_role}`);
    const extension = path.extname(assetName);
    const artifactGlob = `src/BatCave.App/src-tauri/target/release/bundle/${package_.kind}/*${extension}`;
    assert.match(bundleJob, new RegExp(RegExp.escape(artifactGlob), "u"));
    assert.match(releaseJob, new RegExp(RegExp.escape(artifactGlob), "u"));
    assert.match(
      sources.verifier,
      new RegExp(
        `^\\w+=\\("\\$bundle_root"/${RegExp.escape(package_.kind)}/\\*${RegExp.escape(extension)}\\)$`,
        "mu",
      ),
    );
  }
  const [glibcMajor, glibcMinor] = source.maximum_glibc_version.split(".");
  assert.equal(shellReadonly(sources.verifier, "max_glibc_major"), glibcMajor);
  assert.equal(shellReadonly(sources.verifier, "max_glibc_minor"), glibcMinor);
  const glibcVerifier = `verify_${source.libc_family}_floor`;
  const allPayloadElfs = shellFunction(sources.verifier, "verify_all_payload_elfs");
  assertActiveLine(allPayloadElfs, 'verify_elf "$executable" "$package_label ELF"');
  assertActiveLine(allPayloadElfs, `${glibcVerifier} "$executable" "$package_label ELF" 1`);
  assertActiveLine(sources.verifier, 'verify_batcave_payload "$deb_root" "deb payload"');
  assertActiveLine(sources.verifier, 'verify_batcave_payload "$appimage_root" "AppImage payload"');
  assertActiveLine(sources.verifier, 'verify_elf "$appimage" "AppImage runtime" 1');
  assertActiveLine(sources.verifier, `${glibcVerifier} "$appimage" "AppImage runtime" 1`);

  const debAssetName = assetRoles.get(packages.get("deb").asset_role).name;
  const debArchitecture = /_([^_.]+)\.deb$/u.exec(debAssetName)?.[1];
  assert.ok(debArchitecture, "deb release asset must expose its architecture token");
  assert.equal(shellReadonly(sources.verifier, "expected_deb_architecture"), debArchitecture);
  assert.match(
    shellReadonly(sources.verifier, "expected_elf_machine").toLowerCase(),
    new RegExp(RegExp.escape(source.architecture.replace("_", "-")), "u"),
  );

  const dependencyPackages = [
    ...sources.verifier.matchAll(
      /^has_deb_dependency "\$deb_dependencies" "([a-z0-9.+-]+)" \|\|$/gmu,
    ),
  ].map((match) => match[1]);
  assert.deepEqual(dependencyPackages, source.required_deb_runtime_packages);
}

function releaseSupportDocumentation() {
  return {
    readme: fs.readFileSync(path.join(ROOT, "README.md"), "utf8"),
    capabilities: fs.readFileSync(path.join(ROOT, "docs", "platform-capabilities.md"), "utf8"),
    releases: fs.readFileSync(path.join(ROOT, "docs", "releases.md"), "utf8"),
  };
}

function markdownSection(source, heading) {
  const marker = `## ${heading}`;
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `documentation must define ${marker}`);
  const next = source.indexOf("\n## ", start + marker.length);
  return source.slice(start, next === -1 ? source.length : next);
}

const PROFILE_DOCUMENTATION_ORDER = new Map([
  ["windows-client-10-x86_64", 0],
  ["ubuntu-22.04-x86_64-glibc", 1],
  ["debian-12-x86_64-glibc", 2],
  ["macos-12-arm64", 3],
]);

const PACKAGE_DOCUMENTATION_ORDER = new Map([
  ["nsis", 0],
  ["deb", 0],
  ["appimage", 1],
  ["dmg", 0],
  ["macos_updater", 1],
]);

function documentedPackageKinds(profile) {
  return [...profile.packages]
    .sort((left, right) => {
      assert.ok(
        PACKAGE_DOCUMENTATION_ORDER.has(left.kind),
        `documentation package order is missing ${left.kind}`,
      );
      assert.ok(
        PACKAGE_DOCUMENTATION_ORDER.has(right.kind),
        `documentation package order is missing ${right.kind}`,
      );
      return (
        PACKAGE_DOCUMENTATION_ORDER.get(left.kind) - PACKAGE_DOCUMENTATION_ORDER.get(right.kind)
      );
    })
    .map(({ kind }) => {
      switch (kind) {
        case "nsis":
          return "NSIS";
        case "deb":
          return "deb";
        case "appimage":
          return "AppImage";
        case "dmg":
          return "DMG";
        case "macos_updater":
          return "updater archive";
        default:
          assert.fail(`documentation package mapping is missing ${kind}`);
      }
    })
    .join(", ");
}

function supportProfileTableRow(profileId) {
  const profile = PROFILES.get(profileId);
  assert.ok(profile, `machine contract must define ${profileId}`);
  const sourceProof = `\`${profile.proof.source}\``;
  const nativeProof = `\`${profile.proof.native_oldest_supported}\``;
  const packages = documentedPackageKinds(profile);

  switch (profileId) {
    case "windows-client-10-x86_64":
      return `| \`${profile.id}\` | Windows 10 client \`${profile.host.minimum}\`+ | \`${profile.host_architectures[0]}\` | ${packages} | ${sourceProof} | ${nativeProof} |`;
    case "ubuntu-22.04-x86_64-glibc":
      return `| \`${profile.id}\` | Ubuntu \`${profile.host.minimum}\`+ | \`${profile.host_architectures[0]}\`, ${profile.runtime.libc_family} | ${packages} | ${sourceProof} | ${nativeProof} |`;
    case "debian-12-x86_64-glibc":
      return `| \`${profile.id}\` | Debian \`${profile.host.minimum}\`+ | \`${profile.host_architectures[0]}\`, ${profile.runtime.libc_family} | ${packages} | ${sourceProof} | ${nativeProof} |`;
    case "macos-12-arm64":
      return `| \`${profile.id}\` | macOS \`${profile.host.minimum}\`+ | Apple Silicon \`${profile.host_architectures[0]}\` | ${packages} | ${sourceProof} | ${nativeProof} |`;
    default:
      assert.fail(`documentation mapping is missing ${profileId}`);
  }
}

function assertLinuxSupportFacts(source) {
  const linux = RELEASE_PLATFORM_SUPPORT_CONTRACT.linux_source_enforcement;
  assert.ok(source.includes(`\`${linux.hosted_runner}\``));
  assert.ok(source.includes(linux.architecture.replace("_", "-")));
  assert.ok(source.includes(`\`GLIBC_${linux.maximum_glibc_version}\``));
  for (const dependency of linux.required_deb_runtime_packages) {
    assert.ok(source.includes(`\`${dependency}\``));
  }
}

function assertReleaseSupportDocumentationMatchesContract(documents) {
  const capabilities = markdownSection(documents.capabilities, "Distribution and CPU architecture");
  const profileOrder = RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles
    .map(({ id }) => {
      assert.ok(PROFILE_DOCUMENTATION_ORDER.has(id), `documentation order is missing ${id}`);
      return id;
    })
    .sort(
      (left, right) =>
        PROFILE_DOCUMENTATION_ORDER.get(left) - PROFILE_DOCUMENTATION_ORDER.get(right),
    );
  assert.deepEqual(
    capabilities.split("\n").filter((line) => line.startsWith("|")),
    [
      "| Profile | Minimum host | Host architecture/runtime | Contract release packages | Source proof | Oldest-host native proof |",
      "| --- | --- | --- | --- | --- | --- |",
      ...profileOrder.map(supportProfileTableRow),
    ],
  );

  const readme = markdownSection(documents.readme, "Platform support");
  const releases = markdownSection(documents.releases, "Platform support and proof");
  for (const source of [capabilities, releases]) {
    assert.match(source, /platform-support-contract\.v1\.json/u);
  }

  const windows = PROFILES.get("windows-client-10-x86_64");
  const ubuntu = PROFILES.get("ubuntu-22.04-x86_64-glibc");
  const debian = PROFILES.get("debian-12-x86_64-glibc");
  const macos = PROFILES.get("macos-12-arm64");
  const linux = RELEASE_PLATFORM_SUPPORT_CONTRACT.linux_source_enforcement;
  assert.deepEqual(
    readme.split("\n").filter((line) => line.startsWith("|")),
    [
      "| Platform | Release target | Machine telemetry | Per-process network | Package |",
      "| --- | --- | --- | --- | --- |",
      `| Windows | Windows 10 \`${windows.host.minimum}\`+, ${windows.host_architectures[0].replace("_", "-")} | Win32 and PDH | ETW; installed service for protected collection | ${documentedPackageKinds(windows)} |`,
      `| Linux | Ubuntu ${ubuntu.host.minimum}+ or Debian ${debian.host.minimum}+, ${linux.architecture.replace("_", "-")} ${linux.libc_family} | \`/proc\` and \`/sys\` | Optional bpftrace/eBPF | deb, AppImage |`,
      `| macOS | macOS ${macos.host.minimum}+, Apple Silicon | sysinfo, libproc, IOKit | XNU NStat | ${documentedPackageKinds(macos)} |`,
    ],
  );
  assert.ok(readme.includes("[Platform capabilities](docs/platform-capabilities.md)"));
  assert.ok(
    readme.includes(
      "Intel Macs, Windows ARM64, Linux ARM64, musl, and unlisted operating-system profiles are not supported release targets.",
    ),
  );

  for (const proofTerm of ["declared", "source_enforced", "native_oldest_supported: pending"]) {
    assert.ok(releases.includes(`\`${proofTerm}\``));
  }
  assertLinuxSupportFacts(capabilities);
  assertLinuxSupportFacts(releases);
  assert.ok(
    capabilities.includes(
      "Windows Server, Windows ARM64, Linux ARM64, macOS Intel `x86_64`, musl, unlisted Linux distributions, and unlisted package/host combinations are explicit non-claims.",
    ),
  );
}

test("publishes the four closed version 1 support profiles", () => {
  assert.equal(RELEASE_PLATFORM_SUPPORT_CONTRACT.schema_version, 1);
  assert.deepEqual(
    [...PROFILES.keys()],
    [
      "debian-12-x86_64-glibc",
      "macos-12-arm64",
      "ubuntu-22.04-x86_64-glibc",
      "windows-client-10-x86_64",
    ],
  );
  assert.deepEqual(RELEASE_PLATFORM_SUPPORT_CONTRACT.linux_source_enforcement, {
    hosted_runner: "ubuntu-22.04",
    architecture: "x86_64",
    libc_family: "glibc",
    maximum_glibc_version: "2.35",
    required_deb_runtime_packages: ["libgtk-3-0", "libwebkit2gtk-4.1-0"],
  });
  for (const profile of PROFILES.values()) {
    assert.deepEqual(profile.proof, {
      declaration: "declared",
      source: "source_enforced",
      native_oldest_supported: "pending",
    });
  }
});

test("keeps public platform support documentation aligned with the machine contract", () => {
  assertReleaseSupportDocumentationMatchesContract(releaseSupportDocumentation());
});

test("rejects platform support documentation drift", () => {
  const documents = releaseSupportDocumentation();
  const mutations = [
    ["capabilities", "`10.0.16299`+", "`10.0.19045`+"],
    ["readme", "Ubuntu 22.04+", "Ubuntu 24.04+"],
    ["capabilities", "Debian `12`+", "Debian `13`+"],
    ["readme", "macOS 12.0+", "macOS 13.0+"],
    ["capabilities", "`x86_64`, glibc", "`arm64`, glibc"],
    ["capabilities", "deb, AppImage", "rpm"],
    ["capabilities", "`source_enforced`", "`declared`"],
    ["capabilities", "`ubuntu-22.04`", "`ubuntu-latest`"],
    ["capabilities", "`GLIBC_2.35`", "`GLIBC_2.39`"],
    ["capabilities", "`libgtk-3-0`", "`libgtk-4-1`"],
    ["capabilities", "`libwebkit2gtk-4.1-0`", "`libwebkit2gtk-4.0-0`"],
    ["capabilities", "Windows Server, ", ""],
    ["readme", "DMG, updater archive", "DMG"],
    ["readme", "Intel Macs, ", ""],
  ];

  for (const [file, current, drifted] of mutations) {
    const mutated = { ...documents, [file]: documents[file].replace(current, drifted) };
    assert.notEqual(mutated[file], documents[file], `${file} mutation must change the fixture`);
    assert.throws(
      () => assertReleaseSupportDocumentationMatchesContract(mutated),
      assert.AssertionError,
    );
  }
});

test("binds Linux source enforcement to the integrated #123 workflows and verifier", () => {
  assertLinuxSourceEnforcementMatchesContract(linuxSourceFiles());
});

test("builds the Linux release CLI from a clean target before bundle and verification", () => {
  const sourceSha = "0123456789abcdef0123456789abcdef01234567";
  const { cargoManifest, commands } = runBundleOnlyFromCleanState("Linux");
  assert.deepEqual(commands, [
    `cargo|${sourceSha}|build --manifest-path ${cargoManifest} --release --bin batcave-monitor-cli`,
    "npm|run tauri -- build",
    "verify-linux|",
  ]);
});

test("keeps the Linux CLI prebuild out of the macOS arm64 bundle path", () => {
  const { commands } = runBundleOnlyFromCleanState("Darwin");
  assert.deepEqual(commands, [
    "npm|run build",
    "npm|run tauri -- build --target aarch64-apple-darwin --config src-tauri/tauri.macos.ci.conf.json --no-bundle",
    "build-macos|--verify-only",
    "npm|run tauri -- build --target aarch64-apple-darwin --config src-tauri/tauri.macos.ci.conf.json",
    "remove-dmg-volume-icon|src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/*.dmg",
    "verify-macos|--mode adhoc",
  ]);
});

test("removes the internal macOS volume icon before release signing", () => {
  const workflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const sanitizer = 'bash scripts/remove-macos-dmg-volume-icon.sh "${dmgs[0]}"';
  const signer = 'codesign --force --timestamp --sign "${APPLE_SIGNING_IDENTITY}" "${dmgs[0]}"';
  const sanitizerIndex = workflow.indexOf(sanitizer);
  const signerIndex = workflow.indexOf(signer);

  assert.notEqual(sanitizerIndex, -1);
  assert.notEqual(signerIndex, -1);
  assert.ok(sanitizerIndex < signerIndex, "DMG cleanup must happen before release signing");

  const verifier = fs.readFileSync(path.join(ROOT, "scripts", "verify-macos-bundle.sh"), "utf8");
  assert.match(verifier, /\$mount_point\/\.VolumeIcon\.icns/u);
});

test("rejects runner, ABI, package, and runtime drift from the integrated #123 sources", () => {
  const sources = linuxSourceFiles();
  const source = RELEASE_PLATFORM_SUPPORT_CONTRACT.linux_source_enforcement;
  const mutations = [
    {
      ...sources,
      validationWorkflow: sources.validationWorkflow.replace(
        `runs-on: ${source.hosted_runner}`,
        "runs-on: ubuntu-latest",
      ),
    },
    {
      ...sources,
      bundlesWorkflow: sources.bundlesWorkflow.replace(
        `runs-on: ${source.hosted_runner}`,
        "runs-on: ubuntu-latest",
      ),
    },
    {
      ...sources,
      releaseWorkflow: sources.releaseWorkflow.replace(
        `runs-on: ${source.hosted_runner}`,
        "runs-on: ubuntu-latest",
      ),
    },
    {
      ...sources,
      bundlesWorkflow: sources.bundlesWorkflow.replace(
        "        run: bash scripts/validate-tauri.sh --bundle-only",
        "        # run: bash scripts/validate-tauri.sh --bundle-only",
      ),
    },
    {
      ...sources,
      tauriValidator: sources.tauriValidator.replace(
        '    bash "$repo_root/scripts/build-linux-cli.sh"',
        '    # bash "$repo_root/scripts/build-linux-cli.sh"',
      ),
    },
    {
      ...sources,
      releaseWorkflow: sources.releaseWorkflow.replace(
        "        run: bash scripts/build-linux-cli.sh",
        "        # run: bash scripts/build-linux-cli.sh",
      ),
    },
    {
      ...sources,
      buildLinuxCli: sources.buildLinuxCli.replace(
        "  --bin batcave-monitor-cli",
        "  --bin batcave-monitor",
      ),
    },
    {
      ...sources,
      tauriValidator: sources.tauriValidator
        .replace('    bash "$repo_root/scripts/build-linux-cli.sh"\n', "")
        .replace(
          "    npm run tauri -- build\n",
          '    npm run tauri -- build\n    bash "$repo_root/scripts/build-linux-cli.sh"\n',
        ),
    },
    {
      ...sources,
      releaseWorkflow: sources.releaseWorkflow
        .replace("      - name: Build Linux release CLI\n        run: bash scripts/build-linux-cli.sh\n\n", "")
        .replace(
          "      - name: Verify Linux package version\n",
          "      - name: Build Linux release CLI\n        run: bash scripts/build-linux-cli.sh\n\n      - name: Verify Linux package version\n",
        ),
    },
    {
      ...sources,
      tauriValidator: sources.tauriValidator.replace(
        '    bash "$repo_root/scripts/verify-linux-bundle.sh"',
        '    # bash "$repo_root/scripts/verify-linux-bundle.sh"',
      ),
    },
    {
      ...sources,
      releaseWorkflow: sources.releaseWorkflow.replace(
        "        run: bash scripts/verify-linux-bundle.sh",
        "        # run: bash scripts/verify-linux-bundle.sh",
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        `readonly max_glibc_minor=${source.maximum_glibc_version.split(".")[1]}`,
        "readonly max_glibc_minor=36",
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        'readonly expected_deb_architecture="amd64"',
        'readonly expected_deb_architecture="arm64"',
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        'readonly expected_elf_machine="Advanced Micro Devices X86-64"',
        'readonly expected_elf_machine="AArch64"',
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        '    verify_glibc_floor "$executable" "$package_label ELF" 1',
        '    # verify_glibc_floor "$executable" "$package_label ELF" 1',
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        'verify_glibc_floor "$appimage" "AppImage runtime" 1',
        '# verify_glibc_floor "$appimage" "AppImage runtime" 1',
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replaceAll(source.required_deb_runtime_packages[0], "libgtk-4-1"),
    },
    {
      ...sources,
      verifier: sources.verifier.replaceAll(
        source.required_deb_runtime_packages[1],
        "libwebkit2gtk-4.0-0",
      ),
    },
    {
      ...sources,
      verifier: sources.verifier.replace(
        '"$bundle_root"/appimage/*.AppImage',
        '"$bundle_root"/appimage/*.rpm',
      ),
    },
    {
      ...sources,
      tauri: { bundle: { targets: ["deb"] } },
    },
    withMutatedTauri(sources, (tauri) => {
      tauri.bundle.linux.deb.files[LINUX_CLI_DESTINATION] =
        "target/debug/batcave-monitor-cli";
    }),
    withMutatedTauri(sources, (tauri) => {
      delete tauri.bundle.linux.appimage.files[LINUX_CLI_DESTINATION];
    }),
    withMutatedTauri(sources, (tauri) => {
      tauri.bundle.linux.appimage.files["/usr/lib/batcave-monitor-cli"] =
        tauri.bundle.linux.appimage.files[LINUX_CLI_DESTINATION];
      delete tauri.bundle.linux.appimage.files[LINUX_CLI_DESTINATION];
    }),
  ];
  for (const mutation of mutations) {
    assert.throws(() => assertLinuxSourceEnforcementMatchesContract(mutation));
  }
});

test("keeps package roles and updater targets aligned with the release asset contract", () => {
  const assetRoles = new Map(
    expectedReleaseAssetRoles("v9.9.9").roles.map((role) => [role.role, role]),
  );
  for (const profile of PROFILES.values()) {
    for (const package_ of profile.packages) {
      const role = assetRoles.get(package_.asset_role);
      assert.ok(role, package_.asset_role);
      assert.deepEqual(role.updateTargets ?? [], package_.updater_targets);
    }
  }
});

test("keeps package and macOS floor declarations aligned with Tauri configuration", () => {
  const tauriRoot = path.join(ROOT, "src", "BatCave.App", "src-tauri");
  const windows = JSON.parse(fs.readFileSync(path.join(tauriRoot, "tauri.windows.conf.json")));
  const linux = JSON.parse(fs.readFileSync(path.join(tauriRoot, "tauri.linux.conf.json")));
  const macos = JSON.parse(fs.readFileSync(path.join(tauriRoot, "tauri.macos.conf.json")));
  assert.deepEqual(windows.bundle.targets, ["nsis"]);
  assert.deepEqual(linux.bundle.targets, ["deb", "appimage"]);
  assert.deepEqual(macos.bundle.targets, ["app", "dmg"]);
  assert.equal(macos.bundle.macOS.minimumSystemVersion, "12.0");
});

function assertActiveLineCount(source, command, expected, message) {
  const matches = source.match(new RegExp(`^\\s*${RegExp.escape(command)}\\s*$`, "gmu")) ?? [];
  assert.equal(
    matches.length,
    expected,
    message ?? `source must actively run ${command} ${expected} times`,
  );
}

function withoutActiveLine(source, command) {
  return source.replace(new RegExp(`^\\s*${RegExp.escape(command)}\\s*\\n?`, "mu"), "");
}

function assertCanonicalTauriSources(sources) {
  assert.equal(sources.packageJson.scripts.tauri, "tauri");
  assert.deepEqual(
    Object.keys(sources.packageJson.scripts).filter(
      (name) => name === "tauri" || name.startsWith("tauri:"),
    ),
    ["tauri"],
  );
  for (const [name, source] of Object.entries(sources)) {
    if (name !== "packageJson") assert.doesNotMatch(source, /tauri:(?:dev|build):/u);
  }

  assertActiveLine(sources.runPowerShell, "npm run tauri -- dev @AppArgs");
  assertActiveLine(sources.runPowerShell, "npm run tauri -- dev");
  assertActiveLine(sources.runShell, 'npm run tauri -- dev "${app_args[@]}"');
  assertActiveLine(sources.runShell, "npm run tauri -- dev");
  assertActiveLine(sources.validatePowerShell, "npm run tauri -- build");
  assertActiveLine(sources.validateShell, "npm run tauri -- build");
  assertActiveLine(sources.validateShell, 'bash "$repo_root/scripts/verify-linux-bundle.sh"');

  const macBuild =
    'npm run tauri -- build --target "$target" --config src-tauri/tauri.macos.ci.conf.json';
  assertActiveLine(sources.validateShell, 'target="aarch64-apple-darwin"');
  assertActiveLine(sources.validateShell, `${macBuild} --no-bundle`);
  assertActiveLine(
    sources.validateShell,
    'bash "$repo_root/scripts/build-macos-arm64-cli.sh" --verify-only',
  );
  assertActiveLine(sources.validateShell, macBuild);
  assertActiveLine(
    sources.validateShell,
    'bash "$repo_root/scripts/remove-macos-dmg-volume-icon.sh" "${dmg_candidates[0]}"',
  );
  assertActiveLine(
    sources.validateShell,
    'bash "$repo_root/scripts/verify-macos-bundle.sh" --mode adhoc',
  );

  assertActiveLineCount(sources.appReadme, "npm run tauri -- dev", 1);
  assertActiveLineCount(sources.appReadme, "npm run tauri -- build", 1);
  assertActiveLineCount(
    sources.appReadme,
    "npm run tauri -- build --target aarch64-apple-darwin  # macOS Apple Silicon",
    1,
  );
  assertActiveLineCount(
    sources.runtimeDocs,
    "npm run tauri -- build --target aarch64-apple-darwin",
    1,
  );
}

test("uses one canonical Tauri npm entry while preserving platform config resolution", () => {
  const appRoot = path.join(ROOT, "src", "BatCave.App");
  const sources = {
    packageJson: JSON.parse(fs.readFileSync(path.join(appRoot, "package.json"), "utf8")),
    appReadme: fs.readFileSync(path.join(appRoot, "README.md"), "utf8"),
    runtimeDocs: fs.readFileSync(path.join(ROOT, "docs", "runtime-telemetry.md"), "utf8"),
    runPowerShell: fs.readFileSync(path.join(ROOT, "scripts", "run-dev.ps1"), "utf8"),
    runShell: fs.readFileSync(path.join(ROOT, "scripts", "run-dev.sh"), "utf8"),
    validatePowerShell: fs.readFileSync(path.join(ROOT, "scripts", "validate-tauri.ps1"), "utf8"),
    validateShell: fs.readFileSync(path.join(ROOT, "scripts", "validate-tauri.sh"), "utf8"),
  };
  assertCanonicalTauriSources(sources);

  const packageMutations = [
    {
      ...sources,
      packageJson: {
        ...sources.packageJson,
        scripts: { ...sources.packageJson.scripts, tauri: "tauri dev" },
      },
    },
    {
      ...sources,
      packageJson: {
        ...sources.packageJson,
        scripts: {
          ...sources.packageJson.scripts,
          [["tauri", "build", "windows"].join(":")]:
            "tauri build --config src-tauri/tauri.windows.conf.json",
        },
      },
    },
  ];
  for (const mutation of packageMutations) {
    assert.throws(() => assertCanonicalTauriSources(mutation));
  }

  const criticalLines = [
    ["runPowerShell", "npm run tauri -- dev @AppArgs"],
    ["runPowerShell", "npm run tauri -- dev"],
    ["runShell", 'npm run tauri -- dev "${app_args[@]}"'],
    ["runShell", "npm run tauri -- dev"],
    ["validatePowerShell", "npm run tauri -- build"],
    ["validateShell", "npm run tauri -- build"],
    ["validateShell", 'bash "$repo_root/scripts/verify-linux-bundle.sh"'],
    [
      "validateShell",
      'npm run tauri -- build --target "$target" --config src-tauri/tauri.macos.ci.conf.json --no-bundle',
    ],
    ["validateShell", 'bash "$repo_root/scripts/build-macos-arm64-cli.sh" --verify-only'],
    [
      "validateShell",
      'npm run tauri -- build --target "$target" --config src-tauri/tauri.macos.ci.conf.json',
    ],
    [
      "validateShell",
      'bash "$repo_root/scripts/remove-macos-dmg-volume-icon.sh" "${dmg_candidates[0]}"',
    ],
    ["validateShell", 'bash "$repo_root/scripts/verify-macos-bundle.sh" --mode adhoc'],
    ["appReadme", "npm run tauri -- dev"],
    ["appReadme", "npm run tauri -- build"],
    ["appReadme", "npm run tauri -- build --target aarch64-apple-darwin  # macOS Apple Silicon"],
    ["runtimeDocs", "npm run tauri -- build --target aarch64-apple-darwin"],
  ];
  for (const [name, command] of criticalLines) {
    const mutated = { ...sources, [name]: withoutActiveLine(sources[name], command) };
    assert.notEqual(mutated[name], sources[name], `${name} mutation must change the fixture`);
    assert.throws(() => assertCanonicalTauriSources(mutated));
  }
});

const VALID_REAL_PLATFORMS = [
  ["Windows floor", "windows-client-10-x86_64", "windows-client-10.0.16299", "nsis"],
  ["Windows newer build", "windows-client-10-x86_64", "windows-client-10.0.26100", "nsis"],
  ["Ubuntu floor deb", "ubuntu-22.04-x86_64-glibc", "ubuntu-22.04", "deb"],
  ["Ubuntu newer AppImage", "ubuntu-22.04-x86_64-glibc", "ubuntu-24.10", "appimage"],
  ["Debian floor AppImage", "debian-12-x86_64-glibc", "debian-12", "appimage"],
  ["Debian newer deb", "debian-12-x86_64-glibc", "debian-13", "deb"],
  ["macOS arm64 floor", "macos-12-arm64", "macos-12.0", "dmg", "arm64"],
  ["macOS newer arm64", "macos-12-arm64", "macos-15.5.1", "macos_updater", "arm64"],
];

for (const [name, profileId, host, packageKind, architecture] of VALID_REAL_PLATFORMS) {
  test(`accepts ${name}`, () => {
    const value = platform(profileId, host, packageKind, architecture);
    assert.equal(validateReleasePlatformSupport(value, "release_evidence").profile.id, profileId);
  });
}

test("rejects Intel macOS evidence", () => {
  const value = platform("macos-12-arm64", "macos-15.5.1", "dmg", "x86_64");
  assert.throws(
    () => validateReleasePlatformSupport(value, "release_evidence"),
    /architecture: is not supported/u,
  );
});

for (const [profileId, packageKind] of [
  ["windows-client-10-x86_64", "nsis"],
  ["ubuntu-22.04-x86_64-glibc", "appimage"],
  ["debian-12-x86_64-glibc", "deb"],
  ["macos-12-arm64", "dmg"],
  ["macos-12-arm64", "macos_updater"],
]) {
  test(`accepts the reserved ${profileId} ${packageKind} schema fixture host`, () => {
    const value = fixturePlatform(profileId, packageKind);
    assert.equal(validateReleasePlatformSupport(value, "schema_fixture").profile.id, profileId);
  });
}

const HOST_FAILURES = [
  [
    "Windows below floor",
    "windows-client-10-x86_64",
    "windows-client-10.0.15063",
    "nsis",
    /below supported floor/u,
  ],
  [
    "Windows Server",
    "windows-client-10-x86_64",
    "windows-server-10.0.20348",
    "nsis",
    /windows_client_build host/u,
  ],
  [
    "Windows noncanonical revision",
    "windows-client-10-x86_64",
    "windows-client-10.0.19045.1",
    "nsis",
    /canonical Windows client/u,
  ],
  [
    "Windows unsafe build",
    "windows-client-10-x86_64",
    `windows-client-10.0.${"9".repeat(30)}`,
    "nsis",
    /safe integer range/u,
  ],
  [
    "Ubuntu below floor",
    "ubuntu-22.04-x86_64-glibc",
    "ubuntu-20.04",
    "deb",
    /below supported floor/u,
  ],
  [
    "Ubuntu malformed minor",
    "ubuntu-22.04-x86_64-glibc",
    "ubuntu-22.4",
    "deb",
    /April or October/u,
  ],
  [
    "Ubuntu impossible month",
    "ubuntu-22.04-x86_64-glibc",
    "ubuntu-22.99",
    "deb",
    /April or October/u,
  ],
  ["Ubuntu zero month", "ubuntu-22.04-x86_64-glibc", "ubuntu-23.00", "deb", /April or October/u],
  [
    "unknown Linux distribution",
    "ubuntu-22.04-x86_64-glibc",
    "fedora-41",
    "appimage",
    /ubuntu_release host/u,
  ],
  ["Debian below floor", "debian-12-x86_64-glibc", "debian-11", "deb", /below supported floor/u],
  [
    "Debian noncanonical point release",
    "debian-12-x86_64-glibc",
    "debian-12.1",
    "deb",
    /canonical major/u,
  ],
  ["macOS below floor", "macos-12-arm64", "macos-11.7.10", "dmg", /below supported floor/u],
  ["macOS malformed version", "macos-12-arm64", "macos-12", "dmg", /major.minor/u],
];

for (const [name, profileId, host, packageKind, expected] of HOST_FAILURES) {
  test(`rejects ${name}`, () => {
    assert.throws(
      () =>
        validateReleasePlatformSupport(platform(profileId, host, packageKind), "release_evidence"),
      expected,
    );
  });
}

test("rejects reserved synthetic hosts on real evidence and real hosts on fixtures", () => {
  const real = platform("ubuntu-22.04-x86_64-glibc", "synthetic-linux-fixture", "appimage");
  assert.throws(
    () => validateReleasePlatformSupport(real, "release_evidence"),
    /schema_fixture only/u,
  );
  const fixture = fixturePlatform("ubuntu-22.04-x86_64-glibc", "appimage");
  fixture.os_version = "ubuntu-22.04";
  assert.throws(
    () => validateReleasePlatformSupport(fixture, "schema_fixture"),
    /reserved fixture host/u,
  );
});

const PROFILE_FAILURES = [
  [
    "musl runtime",
    (value) => (value.runtime.libc_family = "musl"),
    /libc_family: must equal glibc/u,
  ],
  [
    "unsupported Linux architecture",
    (value) => (value.architecture = "arm64"),
    /architecture: is not supported/u,
  ],
  [
    "Windows package on Linux",
    (value) => (value.package.kind = "nsis"),
    /package.kind: is not supported/u,
  ],
  [
    "universal Linux package architecture",
    (value) => (value.package.architecture = "universal"),
    /package.architecture: must equal x86_64/u,
  ],
  [
    "profile identity drift",
    (value) => (value.profile_id = "debian-12-x86_64-glibc"),
    /debian_release host/u,
  ],
  [
    "source proof drift",
    (value) => (value.proof.source = "declared"),
    /proof.source: must equal source_enforced/u,
  ],
  [
    "native proof missing from real evidence",
    (value) => (value.proof.native = "pending"),
    /proof.native: must equal observed/u,
  ],
  [
    "wrong contract version",
    (value) => (value.support_contract_version = 2),
    /support_contract_version: must equal 1/u,
  ],
];

for (const [name, mutate, expected] of PROFILE_FAILURES) {
  test(`rejects ${name}`, () => {
    const value = platform("ubuntu-22.04-x86_64-glibc", "ubuntu-22.04", "appimage");
    mutate(value);
    assert.throws(() => validateReleasePlatformSupport(value, "release_evidence"), expected);
  });
}

test("rejects observed native proof from a schema fixture", () => {
  const value = fixturePlatform("debian-12-x86_64-glibc", "deb");
  value.proof.native = "observed";
  assert.throws(
    () => validateReleasePlatformSupport(value, "schema_fixture"),
    /proof.native: must equal pending/u,
  );
});

test("rejects hosted source proof from a schema fixture", () => {
  const value = fixturePlatform("ubuntu-22.04-x86_64-glibc", "appimage");
  value.proof.source = "source_enforced";
  assert.throws(
    () => validateReleasePlatformSupport(value, "schema_fixture"),
    /proof.source: must equal pending/u,
  );
});

test("keeps release plans source-enforced but native-pending", () => {
  const pending = platform("ubuntu-22.04-x86_64-glibc", "ubuntu-22.04", "appimage");
  pending.proof.native = "pending";
  assert.equal(
    validateReleasePlatformSupport(pending, "release_plan").profile.id,
    pending.profile_id,
  );

  const observed = structuredClone(pending);
  observed.proof.native = "observed";
  assert.throws(
    () => validateReleasePlatformSupport(observed, "release_plan"),
    /proof.native: must equal pending/u,
  );
});

test("rejects machine-contract source, package, and native-state drift", () => {
  const source = cloneContract();
  source.linux_source_enforcement.maximum_glibc_version = "2.39";
  assert.throws(() => validateReleasePlatformSupportContract(source), /must pin Ubuntu 22.04/u);

  const package_ = cloneContract();
  package_.profiles[0].packages[0].kind = "rpm";
  assert.throws(
    () => validateReleasePlatformSupportContract(package_),
    /kind: must equal appimage/u,
  );

  const proof = cloneContract();
  proof.profiles[0].proof.native_oldest_supported = "proven";
  assert.throws(
    () => validateReleasePlatformSupportContract(proof),
    /native_oldest_supported: must equal pending/u,
  );
});

test("the focused validator CLI accepts the versioned contract", () => {
  const script = path.join(ROOT, "scripts", "validate-release-platform-support.mjs");
  const result = spawnSync(process.execPath, [script, RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /contract v1 \(4 profiles\)/u);
});

test("release and validation workflows run the focused platform contract tests", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/validate-release-platform-support\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/validate-release-platform-support\.test\.mjs/gu)?.length,
    2,
  );
});
