import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { validateSanitizedReleaseEvidenceValue } from "./validate-release-evidence-packet.mjs";

export const CURRENT_USER_PERSISTENCE_SCHEMA_VERSION = 1;

const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const SHA256 = /^sha256:[0-9a-f]{64}$/u;
const UTC_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u;
const SLUG = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/u;
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u;
const PLATFORM_ARCHITECTURES = {
  linux: new Set(["aarch64", "x86_64"]),
  macos: new Set(["aarch64", "x86_64"]),
  windows: new Set(["aarch64", "x86", "x86_64"]),
};
const ARTIFACT_INSTALL_KINDS = {
  app_bundle: new Set(["app_bundle"]),
  appimage: new Set(["appimage"]),
  deb: new Set(["deb"]),
  dmg: new Set(["app_bundle"]),
  nsis: new Set(["nsis"]),
};
const ARTIFACT_DIGEST_SCOPES = {
  app_bundle: "canonical_app_bundle_tree_v1",
  appimage: "artifact_bytes",
  deb: "artifact_bytes",
  dmg: "artifact_bytes",
  nsis: "artifact_bytes",
};
const CANONICAL_ROOTS = {
  linux: new Set(["xdg_data_home", "home_local_share"]),
  macos: new Set(["application_support"]),
  windows: new Set(["local_app_data"]),
};
const COMPONENT_KINDS = new Set(["diagnostics", "settings", "warm_cache"]);
const COMPONENT_STATES = new Set(["degraded", "healthy", "unavailable"]);
const COMPONENT_DURABILITY = new Set(["durable", "not_applicable", "not_written", "session_only"]);
const FAILURE_OPERATIONS = new Set([
  "create",
  "load",
  "migrate",
  "parse",
  "permissions",
  "remove",
  "replace",
  "resolve_root",
  "rotate",
  "serialize",
  "sync",
  "write",
]);
const PROFILE_SHAPE = [
  ["linux-appimage", "linux", "appimage"],
  ["linux-deb", "linux", "deb"],
  ["macos-dmg", "macos", "dmg"],
  ["windows-nsis", "windows", "nsis"],
];
const PACKET_LIMITATION = "candidate_not_release_evidence";

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

function string(value, field, { max = 160, pattern } = {}) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > max ||
    value !== value.normalize("NFC") ||
    [...value].some((character) => character.codePointAt(0) <= 0x1f)
  ) {
    fail(field, `must be a normalized single-line string of at most ${max} characters`);
  }
  if (pattern && !pattern.test(value)) fail(field, "has an invalid format");
  return value;
}

function utcTimestamp(value, field) {
  string(value, field, { pattern: UTC_TIMESTAMP });
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime()) || parsed.toISOString().replace(/\.\d{3}Z$/u, "Z") !== value) {
    fail(field, "must be a real UTC timestamp at whole-second precision");
  }
}

function member(value, field, values) {
  if (!values.has(value)) fail(field, `must be one of ${[...values].join(", ")}`);
  return value;
}

function boolean(value, field) {
  if (typeof value !== "boolean") fail(field, "must be a boolean");
  return value;
}

function sortedUnique(values, field) {
  if (!Array.isArray(values) || values.length === 0) fail(field, "must be a non-empty array");
  for (let index = 1; index < values.length; index += 1) {
    if (values[index - 1] >= values[index]) {
      fail(field, "must be strictly sorted with no duplicates");
    }
  }
}

function validateReleaseIdentity(identity, field, expectedSource) {
  exactKeys(identity, field, ["app_version", "source_commit_sha"]);
  string(identity.app_version, `${field}.app_version`, { pattern: VERSION });
  string(identity.source_commit_sha, `${field}.source_commit_sha`, { pattern: COMMIT_SHA });
  if (expectedSource && identity.source_commit_sha !== expectedSource.source_sha) {
    fail(`${field}.source_commit_sha`, "must match packet.source.source_sha");
  }
  if (expectedSource && identity.app_version !== expectedSource.app_version) {
    fail(`${field}.app_version`, "must match packet.source.app_version");
  }
}

function validateSettings(settings, field, phase) {
  if (settings === null) {
    if (phase !== "degraded") fail(field, "may be null only for the degraded phase");
    return;
  }
  exactKeys(settings, field, ["theme", "history_point_limit"]);
  if (settings.theme !== "ember") fail(`${field}.theme`, "must equal the fixed probe value ember");
  if (settings.history_point_limit !== 180) {
    fail(`${field}.history_point_limit`, "must equal the fixed probe value 180");
  }
}

function validateFailure(failure, field) {
  if (failure === null) return;
  exactKeys(failure, field, ["code", "operation", "retryable"]);
  string(failure.code, `${field}.code`, { max: 80, pattern: SLUG });
  member(failure.operation, `${field}.operation`, FAILURE_OPERATIONS);
  boolean(failure.retryable, `${field}.retryable`);
}

function validatePersistence(persistence, field, phase) {
  exactKeys(persistence, field, [
    "state",
    "current_user_root",
    "components",
    "suppressed_diagnostic_events",
  ]);
  member(persistence.state, `${field}.state`, COMPONENT_STATES);
  exactKeys(persistence.current_user_root, `${field}.current_user_root`, [
    "directory_reported",
    "permission_state",
  ]);
  boolean(
    persistence.current_user_root.directory_reported,
    `${field}.current_user_root.directory_reported`,
  );
  member(
    persistence.current_user_root.permission_state,
    `${field}.current_user_root.permission_state`,
    new Set(["invalid", "unavailable", "verified"]),
  );
  if (
    !Number.isSafeInteger(persistence.suppressed_diagnostic_events) ||
    persistence.suppressed_diagnostic_events < 0
  ) {
    fail(`${field}.suppressed_diagnostic_events`, "must be a non-negative safe integer");
  }
  if (!Array.isArray(persistence.components) || persistence.components.length !== 3) {
    fail(`${field}.components`, "must contain exactly the three current-user components");
  }
  const kinds = persistence.components.map((component) => component.kind);
  sortedUnique(kinds, `${field}.components.kind`);
  if (kinds.join(",") !== "diagnostics,settings,warm_cache") {
    fail(`${field}.components.kind`, "must contain diagnostics, settings, and warm_cache");
  }
  for (const [index, component] of persistence.components.entries()) {
    const componentField = `${field}.components.${index}`;
    exactKeys(component, componentField, ["kind", "state", "durability", "active_failure"]);
    member(component.kind, `${componentField}.kind`, COMPONENT_KINDS);
    member(component.state, `${componentField}.state`, COMPONENT_STATES);
    member(component.durability, `${componentField}.durability`, COMPONENT_DURABILITY);
    validateFailure(component.active_failure, `${componentField}.active_failure`);
  }
  if (phase === "degraded" && persistence.state === "healthy") {
    fail(`${field}.state`, "must expose degraded or unavailable persistence in the degraded phase");
  }
}

export function validateCurrentUserPersistenceReceipt(receipt, field, phase, source) {
  exactKeys(receipt, field, [
    "format_version",
    "evidence_scope",
    "phase",
    "release_identity",
    "platform",
    "architecture",
    "install_kind",
    "settings",
    "health_degraded",
    "persistence_warning_present",
    "persistence",
  ]);
  if (receipt.format_version !== 1) fail(`${field}.format_version`, "must equal 1");
  if (receipt.evidence_scope !== "packaged_current_user_persistence_observation") {
    fail(`${field}.evidence_scope`, "has an unsupported value");
  }
  if (receipt.phase !== phase) fail(`${field}.phase`, `must equal ${phase}`);
  member(receipt.platform, `${field}.platform`, new Set(Object.keys(PLATFORM_ARCHITECTURES)));
  member(receipt.architecture, `${field}.architecture`, PLATFORM_ARCHITECTURES[receipt.platform]);
  member(
    receipt.install_kind,
    `${field}.install_kind`,
    new Set(["app_bundle", "appimage", "deb", "nsis"]),
  );
  validateReleaseIdentity(receipt.release_identity, `${field}.release_identity`, source);
  validateSettings(receipt.settings, `${field}.settings`, phase);
  boolean(receipt.health_degraded, `${field}.health_degraded`);
  boolean(receipt.persistence_warning_present, `${field}.persistence_warning_present`);
  if (receipt.persistence === null) fail(`${field}.persistence`, "must be reported");
  validatePersistence(receipt.persistence, `${field}.persistence`, phase);
  if (phase === "degraded" && !receipt.persistence_warning_present) {
    fail(`${field}.persistence_warning_present`, "must be true for the degraded phase");
  }
}

function validateRoot(root, field, platform) {
  exactKeys(root, field, [
    "canonical_location",
    "owner_verified",
    "permission_model",
    "private_permissions_verified",
    "directory_mode",
    "files",
  ]);
  member(root.canonical_location, `${field}.canonical_location`, CANONICAL_ROOTS[platform]);
  boolean(root.owner_verified, `${field}.owner_verified`);
  boolean(root.private_permissions_verified, `${field}.private_permissions_verified`);
  const expectedModel = platform === "windows" ? "windows_acl" : "unix_mode";
  if (root.permission_model !== expectedModel) {
    fail(`${field}.permission_model`, `must equal ${expectedModel}`);
  }
  if (expectedModel === "unix_mode") {
    if (root.directory_mode !== "0700") fail(`${field}.directory_mode`, "must equal 0700");
  } else if (root.directory_mode !== null) {
    fail(`${field}.directory_mode`, "must be null for Windows ACL evidence");
  }
  if (!Array.isArray(root.files) || root.files.length === 0) {
    fail(`${field}.files`, "must contain at least one observed owned file");
  }
  const components = root.files.map((file) => file.component);
  sortedUnique(components, `${field}.files.component`);
  if (!components.includes("settings")) fail(`${field}.files`, "must include settings");
  for (const [index, file] of root.files.entries()) {
    const fileField = `${field}.files.${index}`;
    exactKeys(file, fileField, ["component", "private_permissions_verified", "mode"]);
    member(file.component, `${fileField}.component`, COMPONENT_KINDS);
    boolean(file.private_permissions_verified, `${fileField}.private_permissions_verified`);
    if (expectedModel === "unix_mode") {
      if (file.mode !== "0600") fail(`${fileField}.mode`, "must equal 0600");
    } else if (file.mode !== null) {
      fail(`${fileField}.mode`, "must be null for Windows ACL evidence");
    }
  }
}

export function validateCurrentUserPersistencePacket(packet, field = "packet") {
  exactKeys(packet, field, [
    "schema_version",
    "packet_kind",
    "packet_id",
    "observed_at_utc",
    "source",
    "host",
    "artifact",
    "root",
    "receipts",
    "checks",
    "result",
    "limitations",
  ]);
  if (packet.schema_version !== CURRENT_USER_PERSISTENCE_SCHEMA_VERSION) {
    fail(`${field}.schema_version`, `must equal ${CURRENT_USER_PERSISTENCE_SCHEMA_VERSION}`);
  }
  if (packet.packet_kind !== "native_candidate") {
    fail(`${field}.packet_kind`, "must equal native_candidate");
  }
  string(packet.packet_id, `${field}.packet_id`, { pattern: SLUG });
  utcTimestamp(packet.observed_at_utc, `${field}.observed_at_utc`);

  exactKeys(packet.source, `${field}.source`, ["repository", "source_sha", "app_version"]);
  if (packet.source.repository !== "TheGreenCedar/BatCave") {
    fail(`${field}.source.repository`, "must equal TheGreenCedar/BatCave");
  }
  string(packet.source.source_sha, `${field}.source.source_sha`, { pattern: COMMIT_SHA });
  string(packet.source.app_version, `${field}.source.app_version`, { pattern: VERSION });

  exactKeys(packet.host, `${field}.host`, ["platform", "architecture", "os_version"]);
  member(
    packet.host.platform,
    `${field}.host.platform`,
    new Set(Object.keys(PLATFORM_ARCHITECTURES)),
  );
  member(
    packet.host.architecture,
    `${field}.host.architecture`,
    PLATFORM_ARCHITECTURES[packet.host.platform],
  );
  string(packet.host.os_version, `${field}.host.os_version`, { max: 120 });

  exactKeys(packet.artifact, `${field}.artifact`, [
    "kind",
    "sha256",
    "digest_scope",
    "install_kind",
  ]);
  member(
    packet.artifact.kind,
    `${field}.artifact.kind`,
    new Set(Object.keys(ARTIFACT_INSTALL_KINDS)),
  );
  string(packet.artifact.sha256, `${field}.artifact.sha256`, { pattern: SHA256 });
  if (packet.artifact.digest_scope !== ARTIFACT_DIGEST_SCOPES[packet.artifact.kind]) {
    fail(
      `${field}.artifact.digest_scope`,
      `must equal ${ARTIFACT_DIGEST_SCOPES[packet.artifact.kind]}`,
    );
  }
  member(
    packet.artifact.install_kind,
    `${field}.artifact.install_kind`,
    ARTIFACT_INSTALL_KINDS[packet.artifact.kind],
  );

  validateRoot(packet.root, `${field}.root`, packet.host.platform);
  exactKeys(packet.receipts, `${field}.receipts`, ["initialize", "restart", "degraded"]);
  for (const phase of ["initialize", "restart", "degraded"]) {
    const receipt = packet.receipts[phase];
    validateCurrentUserPersistenceReceipt(
      receipt,
      `${field}.receipts.${phase}`,
      phase,
      packet.source,
    );
    if (receipt.platform !== packet.host.platform) {
      fail(`${field}.receipts.${phase}.platform`, "must match packet.host.platform");
    }
    if (receipt.architecture !== packet.host.architecture) {
      fail(`${field}.receipts.${phase}.architecture`, "must match packet.host.architecture");
    }
    if (receipt.install_kind !== packet.artifact.install_kind) {
      fail(`${field}.receipts.${phase}.install_kind`, "must match packet.artifact.install_kind");
    }
  }

  const checkKeys = [
    "application_removed",
    "corrupt_source_preserved",
    "degraded_launch_succeeded",
    "outside_sentinel_preserved",
    "persistence_failure_visible",
    "restart_settings_preserved",
    "state_root_preserved",
  ];
  exactKeys(packet.checks, `${field}.checks`, checkKeys);
  for (const key of checkKeys) boolean(packet.checks[key], `${field}.checks.${key}`);
  const permissionsPassed =
    packet.root.owner_verified &&
    packet.root.private_permissions_verified &&
    packet.root.files.every((file) => file.private_permissions_verified) &&
    Object.values(packet.receipts).every(
      (receipt) =>
        receipt.persistence.current_user_root.directory_reported === true &&
        receipt.persistence.current_user_root.permission_state === "verified",
    );
  const expectedResult =
    checkKeys.every((key) => packet.checks[key]) &&
    permissionsPassed &&
    packet.receipts.degraded.health_degraded === true
      ? "passed"
      : "failed";
  if (packet.result !== expectedResult) fail(`${field}.result`, `must equal ${expectedResult}`);

  sortedUnique(packet.limitations, `${field}.limitations`);
  for (const [index, limitation] of packet.limitations.entries()) {
    string(limitation, `${field}.limitations.${index}`, { pattern: SLUG });
  }
  if (!packet.limitations.includes(PACKET_LIMITATION)) {
    fail(`${field}.limitations`, `must include ${PACKET_LIMITATION}`);
  }
  validateSanitizedReleaseEvidenceValue(packet, field);
  return packet;
}

function safePacketPath(value, field) {
  string(value, field, { max: 240 });
  const components = value.split("/");
  if (
    path.posix.isAbsolute(value) ||
    path.win32.isAbsolute(value) ||
    value.includes("\\") ||
    path.posix.normalize(value) !== value ||
    components.some((component) => component === "" || component === "." || component === "..") ||
    !value.startsWith("docs/evidence/persistence/") ||
    path.posix.extname(value) !== ".json"
  ) {
    fail(field, "must be a canonical repository-relative persistence-evidence JSON path");
  }
}

function readStablePacket(relative, repositoryRoot, field) {
  const components = relative.split("/");
  const absolute = path.resolve(repositoryRoot, ...components);
  let metadata;
  let realRoot;
  let realFile;
  try {
    metadata = fs.lstatSync(absolute);
    realRoot = fs.realpathSync(repositoryRoot);
    realFile = fs.realpathSync(absolute);
  } catch (error) {
    fail(field, `cannot read referenced packet: ${error.message}`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(field, "must reference a regular non-link file");
  }
  if (realFile !== path.resolve(realRoot, ...components)) {
    fail(field, "must not traverse a linked repository path");
  }

  let descriptor;
  let opened;
  let bytes;
  try {
    descriptor = fs.openSync(absolute, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
    opened = fs.fstatSync(descriptor);
    if (!opened.isFile() || opened.dev !== metadata.dev || opened.ino !== metadata.ino) {
      fail(field, "changed identity while being opened");
    }
    bytes = fs.readFileSync(descriptor);
    const after = fs.lstatSync(absolute);
    if (
      !after.isFile() ||
      after.isSymbolicLink() ||
      after.dev !== opened.dev ||
      after.ino !== opened.ino
    ) {
      fail(field, "changed identity while being read");
    }
  } catch (error) {
    fail(field, `cannot read stable packet bytes: ${error.message}`);
  } finally {
    if (descriptor !== undefined) fs.closeSync(descriptor);
  }
  return bytes;
}

export function validateCurrentUserPersistenceIndex(index, { repositoryRoot } = {}) {
  exactKeys(index, "index", ["schema_version", "index_kind", "profiles"]);
  if (index.schema_version !== CURRENT_USER_PERSISTENCE_SCHEMA_VERSION) {
    fail("index.schema_version", `must equal ${CURRENT_USER_PERSISTENCE_SCHEMA_VERSION}`);
  }
  if (index.index_kind !== "current_user_persistence_evidence") {
    fail("index.index_kind", "has an unsupported value");
  }
  if (!Array.isArray(index.profiles) || index.profiles.length !== PROFILE_SHAPE.length) {
    fail("index.profiles", `must contain exactly ${PROFILE_SHAPE.length} profiles`);
  }
  for (const [offset, expected] of PROFILE_SHAPE.entries()) {
    const profile = index.profiles[offset];
    const field = `index.profiles.${offset}`;
    exactKeys(profile, field, [
      "id",
      "platform",
      "package_kind",
      "status",
      "packet_path",
      "packet_sha256",
    ]);
    const [id, platform, packageKind] = expected;
    if (profile.id !== id) fail(`${field}.id`, `must equal ${id}`);
    if (profile.platform !== platform) fail(`${field}.platform`, `must equal ${platform}`);
    if (profile.package_kind !== packageKind) {
      fail(`${field}.package_kind`, `must equal ${packageKind}`);
    }
    member(profile.status, `${field}.status`, new Set(["native_candidate", "pending"]));
    if (profile.status === "pending") {
      if (profile.packet_path !== null || profile.packet_sha256 !== null) {
        fail(field, "pending profiles must not claim a packet");
      }
      continue;
    }
    safePacketPath(profile.packet_path, `${field}.packet_path`);
    string(profile.packet_sha256, `${field}.packet_sha256`, { pattern: SHA256 });
    if (repositoryRoot) {
      const bytes = readStablePacket(profile.packet_path, repositoryRoot, `${field}.packet_path`);
      const digest = `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
      if (digest !== profile.packet_sha256) {
        fail(`${field}.packet_sha256`, "does not match packet bytes");
      }
      const packet = JSON.parse(bytes.toString("utf8"));
      validateCurrentUserPersistencePacket(packet, `packet.${profile.id}`);
      if (packet.host.platform !== platform || packet.artifact.kind !== packageKind) {
        fail(field, "packet platform and artifact kind must match the indexed profile");
      }
    }
  }
  validateSanitizedReleaseEvidenceValue(index, "index");
  return index;
}

function loadJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(file, error instanceof Error ? error.message : String(error));
  }
}

function main(argv) {
  if (argv.length === 0) {
    console.error(
      "usage: node scripts/validate-current-user-persistence-evidence.mjs <packet-or-index.json> [...]",
    );
    return 2;
  }
  const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  for (const input of argv) {
    const file = path.resolve(input);
    const value = loadJson(file);
    if (value?.index_kind === "current_user_persistence_evidence") {
      validateCurrentUserPersistenceIndex(value, { repositoryRoot });
    } else {
      validateCurrentUserPersistencePacket(value, input);
    }
    console.log(`validated current-user persistence evidence: ${input}`);
  }
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

export const currentUserPersistenceInternals = {
  PACKET_LIMITATION,
  PROFILE_SHAPE,
};
