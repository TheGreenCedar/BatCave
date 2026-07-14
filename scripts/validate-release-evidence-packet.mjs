import fs from "node:fs";
import { pathToFileURL } from "node:url";

import {
  RELEASE_REPOSITORY,
  RELEASE_SIGNER_WORKFLOW,
  RELEASE_SOURCE_REF,
} from "./verify-public-release.mjs";
import {
  canonicalReleaseAssetName,
  expectedReleaseAssetRoles,
  requireSafeReleaseAssetName,
} from "./release-asset-contract.mjs";
import { parseReleaseTag } from "./verify-release-version.mjs";

export const RELEASE_EVIDENCE_SCHEMA_VERSION = 1;

const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const SLUG = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/u;
const UTC_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u;
const CHECK_STATUSES = new Set(["passed", "failed", "blocked", "not_applicable"]);
const LIMITATION_DISPOSITIONS = new Set(["accepted", "blocked", "not_applicable"]);
const SIGNATURE_KINDS = new Set([
  "apple_notarization",
  "apple_staple",
  "authenticode",
  "contained_app_developer_id",
  "contained_app_notarization",
  "contained_app_staple",
  "developer_id",
  "tauri_updater",
]);
const TAURI_UPDATER_KEY_IDENTITY =
  "sha256:0dad0009cf5cc87a778f2e951cefaa0faaba637b95a22f6f3064f12cd4136545";
const CERTIFICATE_FINGERPRINT = /^sha256:[0-9a-f]{64}$/u;
const DEVELOPER_ID = /^Developer ID Application: [^()]+ \([A-Z0-9]{10}\)$/u;
const NOTARIZATION_SUBMISSION =
  /^submission-id:[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const STAPLED_TICKET = /^ticket-sha256:[0-9a-f]{64}$/u;
const SIGNATURE_IDENTITIES = {
  apple_notarization: {
    fixture: "synthetic Apple notarization fixture",
    pattern: NOTARIZATION_SUBMISSION,
  },
  apple_staple: {
    fixture: "synthetic stapled ticket fixture",
    pattern: STAPLED_TICKET,
  },
  authenticode: {
    fixture: "synthetic Authenticode signer fixture",
    pattern: CERTIFICATE_FINGERPRINT,
  },
  contained_app_developer_id: {
    fixture: "synthetic Developer ID signer fixture",
    pattern: DEVELOPER_ID,
  },
  contained_app_notarization: {
    fixture: "synthetic contained-app notarization fixture",
    pattern: NOTARIZATION_SUBMISSION,
  },
  contained_app_staple: {
    fixture: "synthetic contained-app stapled ticket fixture",
    pattern: STAPLED_TICKET,
  },
  developer_id: {
    fixture: "synthetic Developer ID signer fixture",
    pattern: DEVELOPER_ID,
  },
  tauri_updater: {
    fixture: "synthetic updater key fingerprint fixture",
    value: TAURI_UPDATER_KEY_IDENTITY,
  },
};
const REQUIRED_CHECKS = {
  cleanup: ["application_removed", "owned_runtime_cleanup", "user_state_policy"],
  install: ["anonymous_download", "checksum", "package_install"],
  runtime: ["degradation", "launch", "release_identity", "settings", "telemetry"],
};
const PACKAGE_RULES = {
  windows: {
    host: ["x86_64"],
    package: {
      nsis: {
        role: "Windows NSIS installer and updater payload",
      },
    },
  },
  linux: {
    host: ["x86_64"],
    package: {
      appimage: {
        role: "Linux AppImage package and updater payload",
      },
      deb: { role: "Linux deb package" },
    },
  },
  macos: {
    host: ["arm64", "x86_64"],
    package: {
      dmg: {
        role: "macOS universal DMG",
      },
      macos_updater: {
        role: "macOS universal updater payload",
      },
    },
  },
};
export const RELEASE_EVIDENCE_ROLE_TRUST = {
  "Windows GUI executable": ["authenticode"],
  "Windows CLI executable": ["authenticode"],
  "Windows NSIS installer and updater payload": ["authenticode", "tauri_updater"],
  "Windows updater signature": [],
  "Linux deb package": [],
  "Linux AppImage package and updater payload": ["tauri_updater"],
  "Linux updater signature": [],
  "macOS universal DMG": [
    "apple_notarization",
    "apple_staple",
    "contained_app_developer_id",
    "contained_app_notarization",
    "contained_app_staple",
    "developer_id",
  ],
  "macOS universal updater payload": [
    "contained_app_developer_id",
    "contained_app_notarization",
    "contained_app_staple",
    "tauri_updater",
  ],
  "macOS updater signature": [],
  "updater manifest": [],
  "checksum manifest": [],
  "build provenance bundle": [],
};
const FORBIDDEN_KEYS = new Set([
  "access_key",
  "access_token",
  "api_key",
  "absolute_path",
  "authorization",
  "bearer",
  "client_secret",
  "credential",
  "credentials",
  "env",
  "environment",
  "environment_dump",
  "gh_token",
  "github_token",
  "local_path",
  "password",
  "private_key",
  "private_key_material",
  "raw_log",
  "raw_logs",
  "secret",
  "secret_key",
  "stderr",
  "stdout",
  "token",
]);
const SENSITIVE_ASSIGNMENT_KEYS = new Set([
  "AUTHORIZATION",
  "CI",
  "HOME",
  "HOMEDRIVE",
  "HOMEPATH",
  "LOGNAME",
  "OLDPWD",
  "PATH",
  "PWD",
  "RUNNER_OS",
  "RUNNER_TEMP",
  "RUNNER_TOOL_CACHE",
  "RUNNER_WORKSPACE",
  "SHELL",
  "TEMP",
  "TMP",
  "TMPDIR",
  "USER",
  "USERNAME",
  "USERPROFILE",
]);
const ASSIGNMENT =
  /(?=(?:^|[^\p{L}\p{N}_])(["']?)(\$env:)?([A-Za-z_][A-Za-z0-9_.-]*)\1\s*[:=]\s*(?:["'][^"']*["']|[^\s,;)}\]]+))/giu;

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

function hasControlCharacter(value) {
  return [...value].some((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
}

function string(value, field, { max = 240, pattern } = {}) {
  if (
    typeof value !== "string" ||
    !value.length ||
    value.length > max ||
    value !== value.normalize("NFC") ||
    hasControlCharacter(value)
  ) {
    fail(field, `must be a normalized single-line string of at most ${max} characters`);
  }
  if (pattern && !pattern.test(value)) fail(field, "has an invalid format");
  return value;
}

function positiveInteger(value, field) {
  if (!Number.isSafeInteger(value) || value <= 0) fail(field, "must be a positive safe integer");
}

function array(value, field) {
  if (!Array.isArray(value) || !value.length) fail(field, "must be a non-empty array");
  return value;
}

function sorted(keys, field) {
  for (let index = 1; index < keys.length; index += 1) {
    if (keys[index - 1] >= keys[index]) fail(field, "must be strictly sorted with no duplicates");
  }
}

function assignmentKey(key) {
  return key.replaceAll(/[.-]/gu, "_").toUpperCase();
}

function isSensitiveAssignmentKey(key) {
  const normalized = assignmentKey(key);
  if (
    SENSITIVE_ASSIGNMENT_KEYS.has(normalized) ||
    /^(?:ACTIONS|AWS|AZURE|GH|GITHUB|GOOGLE|NPM|SIGNPATH|TAURI)_/u.test(normalized)
  ) {
    return true;
  }
  const parts = new Set(normalized.split("_").filter(Boolean));
  return (
    parts.has("AUTHORIZATION") ||
    parts.has("CREDENTIAL") ||
    parts.has("PASSWORD") ||
    parts.has("PASSWD") ||
    parts.has("SECRET") ||
    parts.has("TOKEN") ||
    (parts.has("PRIVATE") && parts.has("KEY")) ||
    (parts.has("API") && parts.has("KEY")) ||
    (parts.has("ACCESS") && parts.has("KEY"))
  );
}

function assignments(value) {
  return [...value.matchAll(ASSIGNMENT)].map((match) => ({
    environmentSyntax: Boolean(match[2]),
    key: match[3],
  }));
}

function rejectSensitive(value, field = "packet") {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => rejectSensitive(entry, `${field}[${index}]`));
  } else if (value !== null && typeof value === "object") {
    for (const [key, entry] of Object.entries(value)) {
      if (FORBIDDEN_KEYS.has(key.toLowerCase().replaceAll("-", "_"))) {
        fail(
          `${field}.${key}`,
          "sensitive, raw-log, environment, or local-path fields are forbidden",
        );
      }
      rejectSensitive(entry, `${field}.${key}`);
    }
  } else if (typeof value === "string") {
    if (value.includes("\r") || value.includes("\n"))
      fail(field, "raw or multiline log content is forbidden");
    if (/-----BEGIN [^-]*PRIVATE KEY-----/iu.test(value))
      fail(field, "private-key material is forbidden");
    if (
      /(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|(?:sk|rk)-[A-Za-z0-9_-]{20,})/u.test(
        value,
      )
    ) {
      fail(field, "credential or token material is forbidden");
    }
    if (/\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/iu.test(value)) {
      fail(field, "authorization or bearer credential material is forbidden");
    }
    const observedAssignments = assignments(value);
    if (
      /(?:\$(?:env:)?(?:HOME|USERPROFILE|TEMP|RUNNER_TEMP)\b|\$\{(?:HOME|USERPROFILE|TEMP|RUNNER_TEMP)\}|%(?:HOME|USERPROFILE|TEMP|RUNNER_TEMP)%|%HOMEDRIVE%%HOMEPATH%)/iu.test(
        value,
      ) ||
      observedAssignments.some(
        ({ environmentSyntax, key }) => environmentSyntax || isSensitiveAssignmentKey(key),
      ) ||
      observedAssignments.filter(({ key }) => /^[A-Z][A-Z0-9_]*$/u.test(key)).length >= 2
    ) {
      fail(field, "environment or credential assignment is forbidden");
    }
    if (
      /file:\/\//iu.test(value) ||
      /(?:^|[^\p{L}\p{N}_\\/])(?:~[A-Za-z0-9._-]*[\\/]|[A-Za-z]:[\\/]|\\\\|\/(?!\/)\S)/u.test(value)
    ) {
      fail(field, "absolute or local machine paths are forbidden");
    }
  }
}

export function validateSanitizedReleaseEvidenceValue(value, field = "evidence") {
  rejectSensitive(value, field);
  return value;
}

function releaseUrl(tag) {
  return `https://github.com/${RELEASE_REPOSITORY}/releases/tag/${encodeURIComponent(tag)}`;
}

function assetUrl(tag, name) {
  return `https://github.com/${RELEASE_REPOSITORY}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

function validateRelease(release) {
  exactKeys(release, "packet.release", [
    "repository",
    "tag",
    "channel",
    "source_sha",
    "main_sha",
    "release_target_sha",
    "release_url",
    "workflow_run",
  ]);
  if (release.repository !== RELEASE_REPOSITORY)
    fail("packet.release.repository", `must equal ${RELEASE_REPOSITORY}`);
  const tag = string(release.tag, "packet.release.tag", { max: 80 });
  let version;
  try {
    version = parseReleaseTag(tag);
  } catch (error) {
    fail("packet.release.tag", error.message);
  }
  const channel = version.prerelease ? "prerelease" : "stable";
  if (release.channel !== channel)
    fail("packet.release.channel", `must equal ${channel} for ${tag}`);
  for (const key of ["source_sha", "main_sha", "release_target_sha"]) {
    string(release[key], `packet.release.${key}`, {
      max: 40,
      pattern: COMMIT_SHA,
    });
  }
  if (new Set([release.source_sha, release.main_sha, release.release_target_sha]).size !== 1) {
    fail("packet.release", "source_sha, main_sha, and release_target_sha must identify one commit");
  }
  if (release.release_url !== releaseUrl(tag))
    fail("packet.release.release_url", "must be the exact public release URL");

  const run = release.workflow_run;
  exactKeys(run, "packet.release.workflow_run", ["workflow_file", "run_id", "run_attempt", "url"]);
  if (run.workflow_file !== ".github/workflows/release.yml")
    fail("packet.release.workflow_run.workflow_file", "must identify the release workflow");
  positiveInteger(run.run_id, "packet.release.workflow_run.run_id");
  positiveInteger(run.run_attempt, "packet.release.workflow_run.run_attempt");
  const expectedRunUrl = `https://github.com/${RELEASE_REPOSITORY}/actions/runs/${run.run_id}/attempts/${run.run_attempt}`;
  if (run.url !== expectedRunUrl)
    fail("packet.release.workflow_run.url", "must match the exact run and attempt");
  const assetRoles = new Map(expectedReleaseAssetRoles(tag).roles.map((role) => [role.name, role]));
  return { tag, sourceSha: release.source_sha, assetRoles };
}

function validateSignatureIdentity(kind, identity, packetKind, field) {
  const rule = SIGNATURE_IDENTITIES[kind];
  if (packetKind === "schema_fixture") {
    if (identity !== rule.fixture) fail(field, `must equal ${rule.fixture} for a schema fixture`);
  } else if (rule.value) {
    if (identity !== rule.value) fail(field, `must equal the embedded updater key fingerprint`);
  } else if (!rule.pattern.test(identity)) {
    fail(field, `does not identify a valid ${kind} trust subject`);
  }
}

function validateAsset(asset, index, release, packetKind) {
  const field = `packet.assets[${index}]`;
  exactKeys(asset, field, [
    "name",
    "size_bytes",
    "sha256",
    "api_digest",
    "public_url",
    "attestation",
    "signatures",
  ]);
  let name;
  try {
    name = requireSafeReleaseAssetName(asset.name);
  } catch (error) {
    fail(`${field}.name`, error.message);
  }
  const role = release.assetRoles.get(name);
  if (!role) fail(`${field}.name`, `must be an exact release asset role for ${release.tag}`);
  positiveInteger(asset.size_bytes, `${field}.size_bytes`);
  string(asset.sha256, `${field}.sha256`, { max: 71, pattern: SHA256_DIGEST });
  string(asset.api_digest, `${field}.api_digest`, {
    max: 71,
    pattern: SHA256_DIGEST,
  });
  if (asset.sha256 !== asset.api_digest)
    fail(field, "sha256 and api_digest must identify the same public bytes");
  if (asset.public_url !== assetUrl(release.tag, name))
    fail(`${field}.public_url`, "must be the exact public asset URL");

  const attestation = asset.attestation;
  exactKeys(attestation, `${field}.attestation`, [
    "verified",
    "repository",
    "source_sha",
    "source_ref",
    "signer_workflow",
  ]);
  const expectedAttestation = {
    verified: true,
    repository: RELEASE_REPOSITORY,
    source_sha: release.sourceSha,
    source_ref: RELEASE_SOURCE_REF,
    signer_workflow: RELEASE_SIGNER_WORKFLOW,
  };
  for (const [key, expected] of Object.entries(expectedAttestation)) {
    if (attestation[key] !== expected)
      fail(`${field}.attestation.${key}`, `must equal ${expected}`);
  }

  const signatures = object(asset.signatures, `${field}.signatures`);
  const kinds = Object.keys(signatures);
  sorted(kinds, `${field}.signatures`);
  for (const kind of kinds) {
    if (!SIGNATURE_KINDS.has(kind))
      fail(`${field}.signatures.${kind}`, "is not a supported signature kind");
    exactKeys(signatures[kind], `${field}.signatures.${kind}`, ["identity", "verified"]);
    const identity = string(signatures[kind].identity, `${field}.signatures.${kind}.identity`, {
      max: 180,
    });
    validateSignatureIdentity(kind, identity, packetKind, `${field}.signatures.${kind}.identity`);
    if (signatures[kind].verified !== true)
      fail(`${field}.signatures.${kind}.verified`, "must be true");
  }
  const requiredSignatures = RELEASE_EVIDENCE_ROLE_TRUST[role.role];
  if (!requiredSignatures) {
    fail(`${field}.name`, `has no trust contract for release role ${role.role}`);
  }
  const missingSignatures = requiredSignatures.filter((kind) => !Object.hasOwn(signatures, kind));
  const extraSignatures = kinds.filter((kind) => !requiredSignatures.includes(kind));
  if (missingSignatures.length) {
    fail(`${field}.signatures`, `${role.role} requires ${missingSignatures[0]}`);
  }
  if (extraSignatures.length) {
    fail(`${field}.signatures`, `${role.role} does not accept ${extraSignatures[0]}`);
  }
  if (
    role.role === "macOS universal DMG" &&
    signatures.developer_id.identity !== signatures.contained_app_developer_id.identity
  ) {
    fail(`${field}.signatures`, "DMG and contained app must use the same Developer ID identity");
  }
  return { name, role: role.role, signatures };
}

function validateChecks(checks) {
  exactKeys(checks, "packet.checks", ["install", "runtime", "cleanup"]);
  for (const [group, required] of Object.entries(REQUIRED_CHECKS)) {
    const field = `packet.checks.${group}`;
    exactKeys(checks[group], field, required);
    const ids = Object.keys(checks[group]);
    sorted(ids, field);
    for (const id of ids) {
      const check = checks[group][id];
      exactKeys(check, `${field}.${id}`, ["status", "outcome"]);
      if (!CHECK_STATUSES.has(check.status)) fail(`${field}.${id}.status`, "is not supported");
      string(check.outcome, `${field}.${id}.outcome`, { max: 200 });
    }
  }
}

function validateLimitations(limitations) {
  object(limitations, "packet.limitations");
  const codes = Object.keys(limitations);
  sorted(codes, "packet.limitations");
  for (const code of codes) {
    string(code, `packet.limitations.${code}`, { max: 100, pattern: SLUG });
    const limitation = limitations[code];
    exactKeys(limitation, `packet.limitations.${code}`, ["disposition", "summary"]);
    if (!LIMITATION_DISPOSITIONS.has(limitation.disposition))
      fail(`packet.limitations.${code}.disposition`, "is not supported");
    string(limitation.summary, `packet.limitations.${code}.summary`, {
      max: 240,
    });
  }
}

function validatePlatform(platform, assets, limitations) {
  exactKeys(platform, "packet.platform", ["os", "os_version", "architecture", "package"]);
  string(platform.os_version, "packet.platform.os_version", { max: 120 });
  const rule = PACKAGE_RULES[platform.os];
  if (!rule) fail("packet.platform.os", "must be windows, linux, or macos");
  if (!rule.host.includes(platform.architecture))
    fail("packet.platform.architecture", `is not supported for ${platform.os}`);

  const package_ = platform.package;
  exactKeys(package_, "packet.platform.package", ["kind", "architecture", "asset_name"]);
  const packageRule = rule.package[package_.kind];
  if (!packageRule) fail("packet.platform.package.kind", `is not valid for ${platform.os}`);
  const expectedArchitecture = platform.os === "macos" ? "universal" : platform.architecture;
  if (package_.architecture !== expectedArchitecture)
    fail("packet.platform.package.architecture", `must equal ${expectedArchitecture}`);
  const asset = assets.find((candidate) => candidate.name === package_.asset_name);
  if (!asset) fail("packet.platform.package.asset_name", "must reference one packet asset");
  if (asset.role !== packageRule.role) {
    fail(
      "packet.platform.package.asset_name",
      `${package_.kind} must reference the ${packageRule.role} asset`,
    );
  }
  if (package_.kind === "deb" && !Object.hasOwn(limitations, "deb_checksum_attestation_only")) {
    fail("packet.limitations", "deb evidence must state checksum-and-attestation-only trust");
  }
}

export function validateReleaseEvidencePacket(packet) {
  object(packet, "packet");
  rejectSensitive(packet);
  exactKeys(packet, "packet", [
    "schema_version",
    "packet_kind",
    "packet_id",
    "observed_at_utc",
    "release",
    "platform",
    "assets",
    "checks",
    "limitations",
  ]);
  if (packet.schema_version !== RELEASE_EVIDENCE_SCHEMA_VERSION)
    fail("packet.schema_version", `must equal ${RELEASE_EVIDENCE_SCHEMA_VERSION}`);
  if (!new Set(["release_evidence", "schema_fixture"]).has(packet.packet_kind))
    fail("packet.packet_kind", "must be release_evidence or schema_fixture");
  string(packet.packet_id, "packet.packet_id", { max: 120, pattern: SLUG });
  const observed = string(packet.observed_at_utc, "packet.observed_at_utc", {
    max: 20,
    pattern: UTC_TIMESTAMP,
  });
  const date = new Date(observed);
  if (Number.isNaN(date.valueOf()) || date.toISOString() !== observed.replace(/Z$/u, ".000Z"))
    fail("packet.observed_at_utc", "must be a real UTC time");

  const release = validateRelease(packet.release);
  validateLimitations(packet.limitations);
  validateChecks(packet.checks);
  const fixtureTag = /^v?0\.0\.0-evidence\./u.test(release.tag);
  if (packet.packet_kind === "schema_fixture") {
    if (!fixtureTag)
      fail("packet.release.tag", "schema fixtures must use the reserved v0.0.0-evidence tag");
    if (!Object.hasOwn(packet.limitations, "synthetic_fixture_no_release_claim"))
      fail("packet.limitations", "schema fixtures must state that they are not release evidence");
    if (packet.limitations.synthetic_fixture_no_release_claim.disposition !== "not_applicable") {
      fail(
        "packet.limitations.synthetic_fixture_no_release_claim.disposition",
        "schema fixtures cannot accept release claims",
      );
    }
    for (const [code, limitation] of Object.entries(packet.limitations)) {
      if (limitation.disposition !== "not_applicable") {
        fail(
          `packet.limitations.${code}.disposition`,
          "schema fixtures cannot accept release claims",
        );
      }
    }
    for (const [group, checks] of Object.entries(packet.checks)) {
      for (const [id, check] of Object.entries(checks)) {
        if (check.status !== "not_applicable") {
          fail(`packet.checks.${group}.${id}.status`, "schema fixtures cannot claim proof");
        }
      }
    }
  } else if (fixtureTag) {
    fail("packet.release.tag", "release evidence cannot use the reserved schema-fixture tag");
  }

  const packetAssets = array(packet.assets, "packet.assets");
  const canonicalNames = new Map();
  for (const [index, asset] of packetAssets.entries()) {
    const name = asset?.name;
    let safeName;
    try {
      safeName = requireSafeReleaseAssetName(name);
    } catch (error) {
      fail(`packet.assets[${index}].name`, error.message);
    }
    const canonicalName = canonicalReleaseAssetName(safeName);
    if (canonicalNames.has(canonicalName)) {
      fail(
        `packet.assets[${index}].name`,
        `case-collides with ${canonicalNames.get(canonicalName)}`,
      );
    }
    canonicalNames.set(canonicalName, safeName);
  }
  const assets = packetAssets.map((asset, index) =>
    validateAsset(asset, index, release, packet.packet_kind),
  );
  sorted(
    assets.map((asset) => asset.name),
    "packet.assets",
  );
  validatePlatform(packet.platform, assets, packet.limitations);
  return packet;
}

export function validateReleaseEvidencePacketFile(file) {
  try {
    return validateReleaseEvidencePacket(JSON.parse(fs.readFileSync(file, "utf8")));
  } catch (error) {
    throw new Error(`${file}: invalid release evidence packet: ${error.message}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const files = process.argv.slice(2);
  if (!files.length) {
    console.error("usage: node scripts/validate-release-evidence-packet.mjs <packet.json> [...]");
    process.exit(2);
  }
  try {
    for (const file of files)
      console.log(
        `validated release evidence packet ${validateReleaseEvidencePacketFile(file).packet_id}`,
      );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
