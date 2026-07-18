import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  macosUpdaterPostPublicObservationContract,
  validateMacosUpdaterPostPublicObservation,
} from "./validate-macos-updater-post-public-observation.mjs";
import { readCargoVersion } from "./verify-release-version.mjs";

const APP_VERSION = readCargoVersion(fileURLToPath(new URL("..", import.meta.url)));
const TAG = `v${APP_VERSION}`;
const SOURCE_SHA = "a".repeat(40);
const DIGEST = `sha256:${"b".repeat(64)}`;

function observation() {
  return {
    disposition: "observation_complete",
    reason: "exact_updater_archive_staged_and_cleaned",
    observation: {
      schema_version: 1,
      result_kind: "macos_updater_post_public_observation",
      proof_scope: "post_public_macos_updater_staging_observation_only",
      release_evidence_eligible: false,
      repository: "TheGreenCedar/BatCave",
      release: { tag: TAG, source_sha: SOURCE_SHA, app_version: APP_VERSION },
      artifact: {
        name: "BatCave.Monitor.app.tar.gz",
        size_bytes: 123,
        sha256: DIGEST,
        updater_signature_name: "BatCave.Monitor.app.tar.gz.sig",
        updater_signature_sha256: DIGEST,
        staged_member_count: 17,
      },
      observed_checks: Object.fromEntries(
        macosUpdaterPostPublicObservationContract.expectedChecks.map((check) => [check, "passed"]),
      ),
      limitations: [...macosUpdaterPostPublicObservationContract.expectedLimitations],
    },
  };
}

test("accepts the exact staging-only non-evidence observation", () => {
  assert.ok(
    macosUpdaterPostPublicObservationContract.expectedLimitations.includes(
      "arm64_updater_archive_staging_only",
    ),
  );
  assert.equal(
    validateMacosUpdaterPostPublicObservation(observation(), TAG, SOURCE_SHA).disposition,
    "observation_complete",
  );
});

test("rejects promotion, failed gates, selector drift, and extra authority fields", () => {
  const cases = [
    (value) => (value.observation.release_evidence_eligible = true),
    (value) => (value.observation.observed_checks.updater_signature = "failed"),
    (value) => (value.observation.release.source_sha = "c".repeat(40)),
    (value) => (value.observation.artifact.local_path = "/private/tmp/archive"),
    (value) => value.observation.limitations.pop(),
  ];
  for (const mutate of cases) {
    const value = observation();
    mutate(value);
    assert.throws(() => validateMacosUpdaterPostPublicObservation(value, TAG, SOURCE_SHA));
  }
});
