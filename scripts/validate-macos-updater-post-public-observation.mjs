import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { validateSanitizedReleaseEvidenceValue } from "./validate-release-evidence-packet.mjs";
import { verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const SHA256 = /^sha256:[0-9a-f]{64}$/u;
const MAX_OBSERVATION_BYTES = 64 * 1024;
const RELEASE_REPOSITORY = "TheGreenCedar/BatCave";
const EXPECTED_CHECKS = [
  "anonymous_public_bytes",
  "archive_preflight",
  "checksum_manifest",
  "exact_owned_stream",
  "private_root_cleanup",
  "source_bound_attestations",
  "staged_tree_reverification",
  "updater_signature",
];
const EXPECTED_LIMITATIONS = [
  "github_hosted_macos_15",
  "universal_updater_archive_staging_only",
  "application_not_installed_or_launched",
  "developer_id_notarization_and_staple_not_rechecked",
  "runtime_settings_telemetry_and_degradation_not_exercised",
  "updater_a_to_b_not_exercised",
  "not_release_evidence",
];

function fail(message) {
  throw new Error(message);
}

function exactKeys(value, expected, field) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(`${field} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${field} must contain exactly: ${wanted.join(", ")}`);
  }
}

export function validateMacosUpdaterPostPublicObservation(value, tag, sourceSha) {
  const { version } = verifyWorkspaceReleaseVersion(tag);
  if (!COMMIT_SHA.test(sourceSha)) {
    fail("source SHA must be an exact lowercase 40-character commit SHA");
  }
  exactKeys(value, ["disposition", "observation", "reason"], "result");
  if (
    value.disposition !== "observation_complete" ||
    value.reason !== "exact_updater_archive_staged_and_cleaned"
  ) {
    fail("result must be the closed successful updater-staging observation");
  }
  const observation = value.observation;
  exactKeys(
    observation,
    [
      "artifact",
      "limitations",
      "observed_checks",
      "proof_scope",
      "release",
      "release_evidence_eligible",
      "repository",
      "result_kind",
      "schema_version",
    ],
    "observation",
  );
  if (
    observation.schema_version !== 1 ||
    observation.result_kind !== "macos_updater_post_public_observation" ||
    observation.proof_scope !== "post_public_macos_updater_staging_observation_only" ||
    observation.release_evidence_eligible !== false ||
    observation.repository !== RELEASE_REPOSITORY
  ) {
    fail("observation identity or non-evidence scope is invalid");
  }
  exactKeys(observation.release, ["app_version", "source_sha", "tag"], "observation.release");
  if (
    observation.release.tag !== tag ||
    observation.release.source_sha !== sourceSha ||
    observation.release.app_version !== version
  ) {
    fail("observation release identity does not match the workflow selectors");
  }
  exactKeys(
    observation.artifact,
    [
      "name",
      "sha256",
      "size_bytes",
      "staged_member_count",
      "updater_signature_name",
      "updater_signature_sha256",
    ],
    "observation.artifact",
  );
  if (
    observation.artifact.name !== "BatCave.Monitor.app.tar.gz" ||
    observation.artifact.updater_signature_name !== "BatCave.Monitor.app.tar.gz.sig" ||
    !SHA256.test(observation.artifact.sha256) ||
    !SHA256.test(observation.artifact.updater_signature_sha256) ||
    !Number.isSafeInteger(observation.artifact.size_bytes) ||
    observation.artifact.size_bytes <= 0 ||
    !Number.isSafeInteger(observation.artifact.staged_member_count) ||
    observation.artifact.staged_member_count <= 0
  ) {
    fail("observation artifact identity is invalid");
  }
  exactKeys(observation.observed_checks, EXPECTED_CHECKS, "observation.observed_checks");
  if (!EXPECTED_CHECKS.every((check) => observation.observed_checks[check] === "passed")) {
    fail("every bounded updater-staging observation must pass");
  }
  if (JSON.stringify(observation.limitations) !== JSON.stringify(EXPECTED_LIMITATIONS)) {
    fail("observation limitations must preserve the exact staging-only non-claims");
  }
  validateSanitizedReleaseEvidenceValue(value, "macos_updater_observation");
  return value;
}

export function readMacosUpdaterPostPublicObservation(file, tag, sourceSha) {
  const absolute = path.resolve(file);
  const metadata = fs.lstatSync(absolute);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size <= 0 ||
    metadata.size > MAX_OBSERVATION_BYTES ||
    fs.realpathSync(absolute) !== absolute
  ) {
    fail("observation input must be a bounded regular non-link file");
  }
  let value;
  try {
    value = JSON.parse(fs.readFileSync(absolute, "utf8"));
  } catch {
    fail("observation input must contain valid JSON");
  }
  return validateMacosUpdaterPostPublicObservation(value, tag, sourceSha);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.length !== 5) {
    console.error(
      "usage: node scripts/validate-macos-updater-post-public-observation.mjs <file> <tag> <source-sha>",
    );
    process.exit(2);
  }
  try {
    readMacosUpdaterPostPublicObservation(process.argv[2], process.argv[3], process.argv[4]);
    console.log("macOS updater post-public staging observation validated");
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

export const macosUpdaterPostPublicObservationContract = {
  expectedChecks: EXPECTED_CHECKS,
  expectedLimitations: EXPECTED_LIMITATIONS,
};
