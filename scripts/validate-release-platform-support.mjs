import fs from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";

export const RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION = 1;
export const RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE = fileURLToPath(
  new URL("../docs/evidence/releases/platform-support-contract.v1.json", import.meta.url),
);

const CONTRACT_ID = "batcave-release-platform-support";
const FIXTURE_HOSTS = Object.freeze({
  linux: "synthetic-linux-fixture",
  macos: "synthetic-macos-fixture",
  windows: "synthetic-windows-fixture",
});
const PROFILE_IDS = Object.freeze([
  "debian-12-x86_64-glibc",
  "macos-12-arm64",
  "ubuntu-22.04-x86_64-glibc",
  "windows-client-10-x86_64",
]);
const PROFILE_RULES = Object.freeze({
  "debian-12-x86_64-glibc": {
    os: "linux",
    hostKind: "debian_release",
    minimum: "12",
    hostArchitectures: ["x86_64"],
    libcFamily: "glibc",
    packages: [
      {
        kind: "appimage",
        architecture: "x86_64",
        assetRole: "Linux AppImage package and updater payload",
        updaterTargets: ["linux-x86_64"],
      },
      {
        kind: "deb",
        architecture: "x86_64",
        assetRole: "Linux deb package",
        updaterTargets: [],
      },
    ],
  },
  "macos-12-arm64": {
    os: "macos",
    hostKind: "macos_release",
    minimum: "12.0",
    hostArchitectures: ["arm64"],
    libcFamily: "not_applicable",
    packages: [
      {
        kind: "dmg",
        architecture: "arm64",
        assetRole: "macOS Apple Silicon DMG",
        updaterTargets: [],
      },
      {
        kind: "macos_updater",
        architecture: "arm64",
        assetRole: "macOS Apple Silicon updater payload",
        updaterTargets: ["darwin-aarch64"],
      },
    ],
  },
  "ubuntu-22.04-x86_64-glibc": {
    os: "linux",
    hostKind: "ubuntu_release",
    minimum: "22.04",
    hostArchitectures: ["x86_64"],
    libcFamily: "glibc",
    packages: [
      {
        kind: "appimage",
        architecture: "x86_64",
        assetRole: "Linux AppImage package and updater payload",
        updaterTargets: ["linux-x86_64"],
      },
      {
        kind: "deb",
        architecture: "x86_64",
        assetRole: "Linux deb package",
        updaterTargets: [],
      },
    ],
  },
  "windows-client-10-x86_64": {
    os: "windows",
    hostKind: "windows_client_build",
    minimum: "10.0.16299",
    hostArchitectures: ["x86_64"],
    libcFamily: "not_applicable",
    packages: [
      {
        kind: "nsis",
        architecture: "x86_64",
        assetRole: "Windows NSIS installer and updater payload",
        updaterTargets: ["windows-x86_64"],
      },
    ],
  },
});
const PROOF_DECLARATION = "declared";
const PROOF_SOURCE = "source_enforced";
const PROOF_SOURCE_PENDING = "pending";
const PROOF_NATIVE_PENDING = "pending";
const PROOF_NATIVE_OBSERVED = "observed";
const VERSION_COMPONENT = /^(?:0|[1-9][0-9]*)$/u;
const UBUNTU_VERSION = /^(0|[1-9][0-9]*)\.(04|10)$/u;
const MACOS_VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\.(0|[1-9][0-9]*))?$/u;
const WINDOWS_VERSION = /^10\.0\.(0|[1-9][0-9]*)$/u;

function fail(field, message) {
  throw new Error(`${field}: ${message}`);
}

function object(value, field) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(field, "must be an object");
  }
  return value;
}

function exactKeys(value, field, keys) {
  object(value, field);
  const actual = Object.keys(value);
  const missing = keys.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !keys.includes(key));
  if (missing.length) fail(`${field}.${missing[0]}`, "is required");
  if (extra.length) fail(`${field}.${extra[0]}`, "is not allowed");
}

function nonemptyString(value, field) {
  if (typeof value !== "string" || value.length === 0 || value !== value.normalize("NFC")) {
    fail(field, "must be a non-empty normalized string");
  }
  return value;
}

function exactArray(value, field, expected) {
  if (!Array.isArray(value) || JSON.stringify(value) !== JSON.stringify(expected)) {
    fail(field, `must equal ${JSON.stringify(expected)}`);
  }
}

function compareVersions(left, right) {
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

function numericComponents(parts, field) {
  const components = parts.map(Number);
  if (components.some((component) => !Number.isSafeInteger(component))) {
    fail(field, "contains a version component outside the safe integer range");
  }
  return components;
}

function parseUbuntuVersion(value, field) {
  const match = UBUNTU_VERSION.exec(value);
  if (!match) fail(field, "must use a canonical April or October Ubuntu release");
  return numericComponents(match.slice(1), field);
}

function parseMacosVersion(value, field) {
  const match = MACOS_VERSION.exec(value);
  if (!match) fail(field, "must use canonical major.minor or major.minor.patch form");
  return numericComponents(
    match.slice(1).filter((part) => part !== undefined),
    field,
  );
}

function parseDebianVersion(value, field) {
  if (!VERSION_COMPONENT.test(value)) fail(field, "must use a canonical major release");
  return numericComponents([value], field);
}

function parseWindowsVersion(value, field) {
  const match = WINDOWS_VERSION.exec(value);
  if (!match) fail(field, "must use canonical Windows client 10.0 build form");
  return numericComponents(["10", "0", match[1]], field);
}

function parseProfileHostVersion(kind, value, field) {
  if (kind === "ubuntu_release") return parseUbuntuVersion(value, field);
  if (kind === "debian_release") return parseDebianVersion(value, field);
  if (kind === "macos_release") return parseMacosVersion(value, field);
  if (kind === "windows_client_build") return parseWindowsVersion(value, field);
  fail(field, `unsupported host kind ${kind}`);
}

function hostVersionFromIdentifier(kind, identifier, field) {
  const prefixes = {
    debian_release: "debian-",
    macos_release: "macos-",
    ubuntu_release: "ubuntu-",
    windows_client_build: "windows-client-",
  };
  const prefix = prefixes[kind];
  if (!identifier.startsWith(prefix)) fail(field, `must identify a ${kind} host`);
  return parseProfileHostVersion(kind, identifier.slice(prefix.length), field);
}

function validateContractProof(proof, field) {
  exactKeys(proof, field, ["declaration", "source", "native_oldest_supported"]);
  if (proof.declaration !== PROOF_DECLARATION)
    fail(`${field}.declaration`, `must equal ${PROOF_DECLARATION}`);
  if (proof.source !== PROOF_SOURCE) fail(`${field}.source`, `must equal ${PROOF_SOURCE}`);
  if (proof.native_oldest_supported !== PROOF_NATIVE_PENDING) {
    fail(`${field}.native_oldest_supported`, `must equal ${PROOF_NATIVE_PENDING}`);
  }
}

function validateContractPackage(package_, expected, field) {
  exactKeys(package_, field, ["kind", "architecture", "asset_role", "updater_targets"]);
  if (package_.kind !== expected.kind) fail(`${field}.kind`, `must equal ${expected.kind}`);
  if (package_.architecture !== expected.architecture) {
    fail(`${field}.architecture`, `must equal ${expected.architecture}`);
  }
  if (package_.asset_role !== expected.assetRole) {
    fail(`${field}.asset_role`, `must equal ${expected.assetRole}`);
  }
  exactArray(package_.updater_targets, `${field}.updater_targets`, expected.updaterTargets);
}

function validateContractProfile(profile, index) {
  const field = `contract.profiles[${index}]`;
  exactKeys(profile, field, [
    "id",
    "os",
    "host",
    "host_architectures",
    "runtime",
    "packages",
    "proof",
  ]);
  if (profile.id !== PROFILE_IDS[index]) fail(`${field}.id`, `must equal ${PROFILE_IDS[index]}`);
  const expected = PROFILE_RULES[profile.id];
  if (profile.os !== expected.os) fail(`${field}.os`, `must equal ${expected.os}`);
  exactKeys(profile.host, `${field}.host`, ["kind", "minimum"]);
  if (profile.host.kind !== expected.hostKind) {
    fail(`${field}.host.kind`, `must equal ${expected.hostKind}`);
  }
  if (profile.host.minimum !== expected.minimum) {
    fail(`${field}.host.minimum`, `must equal ${expected.minimum}`);
  }
  const minimum = parseProfileHostVersion(
    profile.host.kind,
    nonemptyString(profile.host.minimum, `${field}.host.minimum`),
    `${field}.host.minimum`,
  );
  exactArray(profile.host_architectures, `${field}.host_architectures`, expected.hostArchitectures);
  exactKeys(profile.runtime, `${field}.runtime`, ["libc_family"]);
  if (profile.runtime.libc_family !== expected.libcFamily) {
    fail(`${field}.runtime.libc_family`, `must equal ${expected.libcFamily}`);
  }
  if (!Array.isArray(profile.packages) || profile.packages.length !== expected.packages.length) {
    fail(`${field}.packages`, `must contain ${expected.packages.length} closed packages`);
  }
  for (const [packageIndex, package_] of profile.packages.entries()) {
    validateContractPackage(
      package_,
      expected.packages[packageIndex],
      `${field}.packages[${packageIndex}]`,
    );
  }
  validateContractProof(profile.proof, `${field}.proof`);
  return { ...profile, minimum };
}

export function validateReleasePlatformSupportContract(contract) {
  exactKeys(contract, "contract", [
    "schema_version",
    "contract_id",
    "fixture_hosts",
    "linux_source_enforcement",
    "profiles",
  ]);
  if (contract.schema_version !== RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION) {
    fail("contract.schema_version", `must equal ${RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION}`);
  }
  if (contract.contract_id !== CONTRACT_ID)
    fail("contract.contract_id", `must equal ${CONTRACT_ID}`);
  exactKeys(contract.fixture_hosts, "contract.fixture_hosts", ["linux", "macos", "windows"]);
  for (const [os, token] of Object.entries(FIXTURE_HOSTS)) {
    if (contract.fixture_hosts[os] !== token)
      fail(`contract.fixture_hosts.${os}`, `must equal ${token}`);
  }
  exactKeys(contract.linux_source_enforcement, "contract.linux_source_enforcement", [
    "hosted_runner",
    "architecture",
    "libc_family",
    "maximum_glibc_version",
    "required_deb_runtime_packages",
  ]);
  const linuxSource = contract.linux_source_enforcement;
  if (
    linuxSource.hosted_runner !== "ubuntu-22.04" ||
    linuxSource.architecture !== "x86_64" ||
    linuxSource.libc_family !== "glibc" ||
    linuxSource.maximum_glibc_version !== "2.35"
  ) {
    fail(
      "contract.linux_source_enforcement",
      "must pin Ubuntu 22.04, x86_64, glibc, and a 2.35 maximum required symbol version",
    );
  }
  exactArray(
    linuxSource.required_deb_runtime_packages,
    "contract.linux_source_enforcement.required_deb_runtime_packages",
    ["libgtk-3-0", "libwebkit2gtk-4.1-0"],
  );
  if (!Array.isArray(contract.profiles) || contract.profiles.length !== PROFILE_IDS.length) {
    fail("contract.profiles", `must contain the ${PROFILE_IDS.length} closed release profiles`);
  }
  const profiles = contract.profiles.map(validateContractProfile);
  return { contract, profiles };
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

const loadedContract = JSON.parse(fs.readFileSync(RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE, "utf8"));
const { profiles: validatedProfiles } = validateReleasePlatformSupportContract(loadedContract);
export const RELEASE_PLATFORM_SUPPORT_CONTRACT = deepFreeze(loadedContract);
const PROFILES = new Map(validatedProfiles.map((profile) => [profile.id, deepFreeze(profile)]));

function validateEvidenceProof(proof, packetKind, field) {
  exactKeys(proof, field, ["declaration", "source", "native"]);
  if (proof.declaration !== PROOF_DECLARATION)
    fail(`${field}.declaration`, `must equal ${PROOF_DECLARATION}`);
  const expectedSource = packetKind === "schema_fixture" ? PROOF_SOURCE_PENDING : PROOF_SOURCE;
  if (proof.source !== expectedSource) fail(`${field}.source`, `must equal ${expectedSource}`);
  const expectedNative =
    packetKind === "release_evidence" ? PROOF_NATIVE_OBSERVED : PROOF_NATIVE_PENDING;
  if (proof.native !== expectedNative) fail(`${field}.native`, `must equal ${expectedNative}`);
}

export function validateReleasePlatformSupport(platform, packetKind) {
  if (
    packetKind !== "release_evidence" &&
    packetKind !== "release_plan" &&
    packetKind !== "schema_fixture"
  ) {
    fail("packet.packet_kind", "must be release_evidence, release_plan, or schema_fixture");
  }
  exactKeys(platform, "packet.platform", [
    "support_contract_version",
    "profile_id",
    "proof",
    "os",
    "os_version",
    "architecture",
    "runtime",
    "package",
  ]);
  if (platform.support_contract_version !== RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION) {
    fail(
      "packet.platform.support_contract_version",
      `must equal ${RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION}`,
    );
  }
  const profile = PROFILES.get(platform.profile_id);
  if (!profile) fail("packet.platform.profile_id", "is not a supported release profile");
  if (platform.os !== profile.os) fail("packet.platform.os", `must equal ${profile.os}`);
  nonemptyString(platform.os_version, "packet.platform.os_version");
  validateEvidenceProof(platform.proof, packetKind, "packet.platform.proof");

  const fixtureToken = RELEASE_PLATFORM_SUPPORT_CONTRACT.fixture_hosts[profile.os];
  if (packetKind === "schema_fixture") {
    if (platform.os_version !== fixtureToken) {
      fail("packet.platform.os_version", `must equal reserved fixture host ${fixtureToken}`);
    }
  } else {
    if (
      Object.values(RELEASE_PLATFORM_SUPPORT_CONTRACT.fixture_hosts).includes(platform.os_version)
    ) {
      fail("packet.platform.os_version", "reserved synthetic hosts are schema_fixture only");
    }
    const observedVersion = hostVersionFromIdentifier(
      profile.host.kind,
      platform.os_version,
      "packet.platform.os_version",
    );
    if (compareVersions(observedVersion, profile.minimum) < 0) {
      fail("packet.platform.os_version", `is below supported floor ${profile.host.minimum}`);
    }
  }

  if (!profile.host_architectures.includes(platform.architecture)) {
    fail("packet.platform.architecture", `is not supported by ${profile.id}`);
  }
  exactKeys(platform.runtime, "packet.platform.runtime", ["libc_family"]);
  if (platform.runtime.libc_family !== profile.runtime.libc_family) {
    fail(
      "packet.platform.runtime.libc_family",
      `must equal ${profile.runtime.libc_family} for ${profile.id}`,
    );
  }
  exactKeys(platform.package, "packet.platform.package", ["kind", "architecture", "asset_name"]);
  const package_ = profile.packages.find(({ kind }) => kind === platform.package.kind);
  if (!package_) fail("packet.platform.package.kind", `is not supported by ${profile.id}`);
  if (platform.package.architecture !== package_.architecture) {
    fail("packet.platform.package.architecture", `must equal ${package_.architecture}`);
  }
  nonemptyString(platform.package.asset_name, "packet.platform.package.asset_name");
  return { profile, package: package_ };
}

export function validateReleasePlatformSupportContractFile(
  file = RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE,
) {
  try {
    return validateReleasePlatformSupportContract(JSON.parse(fs.readFileSync(file, "utf8")))
      .contract;
  } catch (error) {
    throw new Error(`${file}: invalid release platform support contract: ${error.message}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const files = process.argv.slice(2);
  try {
    for (const file of files.length ? files : [RELEASE_PLATFORM_SUPPORT_CONTRACT_FILE]) {
      const contract = validateReleasePlatformSupportContractFile(file);
      console.log(
        `validated release platform support contract v${contract.schema_version} (${contract.profiles.length} profiles)`,
      );
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
