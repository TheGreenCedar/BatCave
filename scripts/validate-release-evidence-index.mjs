import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { isDeepStrictEqual } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  validateReleaseEvidencePacket,
  validateSanitizedReleaseEvidenceValue,
} from "./validate-release-evidence-packet.mjs";
import {
  RELEASE_PLATFORM_SUPPORT_CONTRACT,
  RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION,
} from "./validate-release-platform-support.mjs";

export const RELEASE_EVIDENCE_INDEX_SCHEMA_VERSION = 1;

const REPOSITORY_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const INDEX_KINDS = new Set(["release_evidence_index", "schema_fixture"]);
const SLUG = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/u;
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const RELEASE_KEYS = [
  "repository",
  "tag",
  "channel",
  "source_sha",
  "main_sha",
  "release_target_sha",
  "release_url",
  "workflow_run",
];
const REFERENCE_KEYS = [
  "packet_id",
  "path",
  "packet_sha256",
  "profile_id",
  "package_role",
  "public_asset",
];
const PUBLIC_ASSET_KEYS = ["name", "size_bytes", "sha256", "api_digest", "public_url"];
const FIXTURE_NON_CLAIMS = [
  "independent_review_and_live_publication_required",
  "synthetic_fixture_no_release_claim",
];
const REAL_NON_CLAIMS = ["independent_review_and_live_publication_required"];
const SUPPORT_PROFILES = new Map(
  RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.map((profile) => [profile.id, profile]),
);
const REQUIRED_PACKAGE_ROLES = [
  ...new Set(
    RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.flatMap((profile) =>
      profile.packages.map((package_) => package_.asset_role),
    ),
  ),
].sort();

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

function string(value, field, { max = 240, pattern } = {}) {
  const hasControlCharacter =
    typeof value === "string" &&
    [...value].some((character) => {
      const codePoint = character.codePointAt(0);
      return codePoint <= 0x1f || codePoint === 0x7f;
    });
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > max ||
    value !== value.normalize("NFC") ||
    hasControlCharacter ||
    (pattern && !pattern.test(value))
  ) {
    fail(field, "is not a valid normalized string");
  }
  return value;
}

function positiveInteger(value, field) {
  if (!Number.isSafeInteger(value) || value <= 0) fail(field, "must be a positive integer");
  return value;
}

function array(value, field) {
  if (!Array.isArray(value) || value.length === 0) fail(field, "must be a non-empty array");
  return value;
}

function strictlySorted(values, field) {
  for (let index = 1; index < values.length; index += 1) {
    if (values[index - 1] >= values[index]) fail(field, "must be strictly sorted");
  }
}

function exactArray(actual, expected, field) {
  if (!isDeepStrictEqual(actual, expected)) fail(field, `must equal ${JSON.stringify(expected)}`);
}

function digest(bytes) {
  return `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
}

function validateReferencePath(value, field, root) {
  const relative = string(value, field, { max: 300 });
  const components = relative.split("/");
  if (
    path.posix.isAbsolute(relative) ||
    path.win32.isAbsolute(relative) ||
    relative.includes("\\") ||
    path.posix.normalize(relative) !== relative ||
    components.some((component) => component === "." || component === ".." || component === "") ||
    !relative.startsWith("docs/evidence/releases/") ||
    path.posix.extname(relative) !== ".json"
  ) {
    fail(field, "must be a canonical repository-relative release-evidence JSON path");
  }
  const absolute = path.resolve(root, ...components);
  const fromRoot = path.relative(root, absolute);
  if (fromRoot.startsWith("..") || path.isAbsolute(fromRoot)) {
    fail(field, "must remain inside the repository root");
  }
  return { relative, absolute };
}

function readPacket(reference, index, root) {
  const field = `index.packet_references[${index}]`;
  const resolved = validateReferencePath(reference.path, `${field}.path`, root);
  let metadata;
  let realRoot;
  let realFile;
  try {
    metadata = fs.lstatSync(resolved.absolute);
    realRoot = fs.realpathSync(root);
    realFile = fs.realpathSync(resolved.absolute);
  } catch (error) {
    fail(`${field}.path`, `cannot read referenced packet: ${error.message}`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(`${field}.path`, "must reference a regular non-link file");
  }
  const expectedRealFile = path.resolve(realRoot, ...resolved.relative.split("/"));
  if (realFile !== expectedRealFile) {
    fail(`${field}.path`, "must not traverse a linked repository path");
  }

  let descriptor;
  let opened;
  let bytes;
  try {
    descriptor = fs.openSync(
      resolved.absolute,
      fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0),
    );
    opened = fs.fstatSync(descriptor);
    if (!opened.isFile() || opened.dev !== metadata.dev || opened.ino !== metadata.ino) {
      fail(`${field}.path`, "changed identity while being opened");
    }
    bytes = fs.readFileSync(descriptor);
    const after = fs.lstatSync(resolved.absolute);
    if (
      !after.isFile() ||
      after.isSymbolicLink() ||
      after.dev !== opened.dev ||
      after.ino !== opened.ino
    ) {
      fail(`${field}.path`, "changed identity while being read");
    }
  } catch (error) {
    fail(`${field}.path`, `cannot read stable packet bytes: ${error.message}`);
  } finally {
    if (descriptor !== undefined) fs.closeSync(descriptor);
  }
  const actualDigest = digest(bytes);
  if (reference.packet_sha256 !== actualDigest) {
    fail(`${field}.packet_sha256`, `does not match ${reference.path}`);
  }
  let packet;
  try {
    packet = validateReleaseEvidencePacket(JSON.parse(bytes.toString("utf8")));
  } catch (error) {
    fail(`${field}.path`, `does not contain a valid #98 packet: ${error.message}`);
  }
  return { packet, relative: resolved.relative };
}

function validateRelease(release) {
  exactKeys(release, "index.release", RELEASE_KEYS);
  string(release.tag, "index.release.tag", { max: 80 });
  return release;
}

function validatePublicAsset(asset, field) {
  exactKeys(asset, field, PUBLIC_ASSET_KEYS);
  string(asset.name, `${field}.name`, { max: 240 });
  positiveInteger(asset.size_bytes, `${field}.size_bytes`);
  string(asset.sha256, `${field}.sha256`, { max: 71, pattern: SHA256_DIGEST });
  string(asset.api_digest, `${field}.api_digest`, { max: 71, pattern: SHA256_DIGEST });
  string(asset.public_url, `${field}.public_url`, { max: 500 });
  return asset;
}

function packetAssetIdentity(packet) {
  const asset = packet.assets.find(({ name }) => name === packet.platform.package.asset_name);
  if (!asset) fail("index.packet", "validated packet has no selected package asset");
  return Object.fromEntries(PUBLIC_ASSET_KEYS.map((key) => [key, asset[key]]));
}

function expectedPacketDirectory(kind, tag) {
  return kind === "schema_fixture"
    ? "docs/evidence/releases/fixtures/v1"
    : `docs/evidence/releases/${tag}`;
}

function validateReference(reference, index, context) {
  const field = `index.packet_references[${index}]`;
  exactKeys(reference, field, REFERENCE_KEYS);
  string(reference.packet_id, `${field}.packet_id`, { max: 120, pattern: SLUG });
  string(reference.packet_sha256, `${field}.packet_sha256`, {
    max: 71,
    pattern: SHA256_DIGEST,
  });
  string(reference.profile_id, `${field}.profile_id`, { max: 120, pattern: SLUG });
  string(reference.package_role, `${field}.package_role`, { max: 160 });
  validateSanitizedReleaseEvidenceValue(reference.package_role, `${field}.package_role`);
  validatePublicAsset(reference.public_asset, `${field}.public_asset`);

  const { packet, relative } = readPacket(reference, index, context.root);
  const expectedPacketKind =
    context.kind === "schema_fixture" ? "schema_fixture" : "release_evidence";
  if (packet.packet_kind !== expectedPacketKind) {
    fail(`${field}.path`, `${context.kind} cannot reference a ${packet.packet_kind} packet`);
  }
  const expectedDirectory = expectedPacketDirectory(context.kind, context.release.tag);
  if (path.posix.dirname(relative) !== expectedDirectory) {
    fail(`${field}.path`, `must live directly under ${expectedDirectory}`);
  }
  if (reference.packet_id !== packet.packet_id) {
    fail(`${field}.packet_id`, `must equal referenced packet ID ${packet.packet_id}`);
  }
  if (!isDeepStrictEqual(context.release, packet.release)) {
    fail(`${field}.path`, "release identity contradicts the index");
  }
  if (packet.platform.support_contract_version !== context.supportContractVersion) {
    fail(`${field}.path`, "support-contract version contradicts the index");
  }
  if (reference.profile_id !== packet.platform.profile_id) {
    fail(`${field}.profile_id`, `must equal packet profile ${packet.platform.profile_id}`);
  }

  const profile = SUPPORT_PROFILES.get(reference.profile_id);
  if (!profile) fail(`${field}.profile_id`, "is not a declared support profile");
  const package_ = profile.packages.find(
    (candidate) => candidate.kind === packet.platform.package.kind,
  );
  if (!package_) {
    fail(`${field}.path`, "packet package kind is not allowed by the referenced profile");
  }
  if (reference.package_role !== package_.asset_role) {
    fail(`${field}.package_role`, `must equal packet package role ${package_.asset_role}`);
  }

  const packetAsset = packetAssetIdentity(packet);
  if (!isDeepStrictEqual(reference.public_asset, packetAsset)) {
    fail(`${field}.public_asset`, "contradicts the packet's selected public asset");
  }
  return { packet, relative, packageRole: package_.asset_role, publicAsset: packetAsset };
}

function requireUnique(values, field, label) {
  const seen = new Set();
  for (const value of values) {
    if (seen.has(value)) fail(field, `contains duplicate ${label} ${value}`);
    seen.add(value);
  }
}

function validateCoverage(validated) {
  const profiles = [...new Set(validated.map(({ packet }) => packet.platform.profile_id))].sort();
  const expectedProfiles = [...SUPPORT_PROFILES.keys()].sort();
  exactArray(profiles, expectedProfiles, "index.packet_references profile coverage");

  const roles = validated.map(({ packageRole }) => packageRole).sort();
  exactArray(roles, REQUIRED_PACKAGE_ROLES, "index.packet_references package-role coverage");
}

export function validateReleaseEvidenceIndex(index, options = {}) {
  object(index, "index");
  exactKeys(index, "index", [
    "schema_version",
    "index_kind",
    "index_id",
    "release",
    "support_contract_version",
    "packet_references",
    "non_claims",
  ]);
  if (index.schema_version !== RELEASE_EVIDENCE_INDEX_SCHEMA_VERSION) {
    fail("index.schema_version", `must equal ${RELEASE_EVIDENCE_INDEX_SCHEMA_VERSION}`);
  }
  if (!INDEX_KINDS.has(index.index_kind)) fail("index.index_kind", "is not supported");
  string(index.index_id, "index.index_id", { max: 120, pattern: SLUG });
  const release = validateRelease(index.release);
  if (index.support_contract_version !== RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION) {
    fail(
      "index.support_contract_version",
      `must equal ${RELEASE_PLATFORM_SUPPORT_CONTRACT_VERSION}`,
    );
  }
  const expectedNonClaims =
    index.index_kind === "schema_fixture" ? FIXTURE_NON_CLAIMS : REAL_NON_CLAIMS;
  exactArray(index.non_claims, expectedNonClaims, "index.non_claims");

  const root = path.resolve(options.root ?? REPOSITORY_ROOT);
  const references = array(index.packet_references, "index.packet_references");
  const validated = references.map((reference, referenceIndex) =>
    validateReference(reference, referenceIndex, {
      root,
      kind: index.index_kind,
      release,
      supportContractVersion: index.support_contract_version,
    }),
  );

  requireUnique(
    references.map(({ packet_id: packetId }) => packetId),
    "index.packet_references",
    "packet ID",
  );
  requireUnique(
    references.map(({ path: packetPath }) => packetPath),
    "index.packet_references",
    "path",
  );
  requireUnique(
    references.map(({ packet_sha256: packetDigest }) => packetDigest),
    "index.packet_references",
    "packet digest",
  );
  strictlySorted(
    references.map(({ path: packetPath }) => packetPath),
    "index.packet_references",
  );
  requireUnique(
    validated.map(({ publicAsset }) => publicAsset.name.toLowerCase()),
    "index.packet_references",
    "public asset",
  );
  validateCoverage(validated);
  return index;
}

export function validateReleaseEvidenceIndexFile(file, options = {}) {
  try {
    const index = JSON.parse(fs.readFileSync(file, "utf8"));
    return validateReleaseEvidenceIndex(index, options);
  } catch (error) {
    throw new Error(`${file}: invalid release evidence index: ${error.message}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const files = process.argv.slice(2);
  if (!files.length) {
    console.error("usage: node scripts/validate-release-evidence-index.mjs <index.json> [...]");
    process.exit(2);
  }
  try {
    for (const file of files) {
      const index = validateReleaseEvidenceIndexFile(file);
      console.log(
        `validated release evidence index ${index.index_id} (${index.index_kind}; ${index.packet_references.length} packets; review input only)`,
      );
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
