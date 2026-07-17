import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import { validateSanitizedReleaseEvidenceValue } from "./validate-release-evidence-packet.mjs";
import { RELEASE_REPOSITORY, verifyPublicRelease } from "./verify-public-release.mjs";
import { parseReleaseTag, verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const MAX_RELEASE_READBACK_BYTES = 1024 * 1024;
const RELEASE_API_ROOT = `https://api.github.com/repos/${RELEASE_REPOSITORY}/releases/tags/`;
const CANDIDATE_FILE = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../post-public-input/release-candidate.json",
);
const OUTPUT_DIRECTORY = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../post-public-output",
);

function fail(message) {
  throw new Error(message);
}

function buildDebEvidence(receipt, captureResult) {
  const state = linuxPersistenceCaptureInternals.requireVerifiedPublicDebCaptureResult(
    captureResult,
    receipt,
  );
  const { asset, packet, rootSettlements, telemetry } = state;
  if (packet.result !== "passed") fail("public deb lifecycle observation did not pass");
  const observedChecks = {
    anonymous_public_bytes: "passed",
    checksum_manifest: "passed",
    source_bound_attestations: "passed",
    package_identity: packet.source.app_version === receipt.app_version ? "passed" : "failed",
    standard_user_runtime: packet.receipts.initialize.install_kind === "deb" ? "passed" : "failed",
    settings_restart: packet.checks.restart_settings_preserved ? "passed" : "failed",
    persistence_degradation:
      packet.checks.persistence_failure_visible && packet.receipts.degraded.health_degraded
        ? "passed"
        : "failed",
    advancing_telemetry: telemetry.samples_advanced ? "passed" : "failed",
    package_owned_files_removed: packet.checks.application_removed ? "passed" : "failed",
    root_process_settlement:
      rootSettlements.length === 5 &&
      rootSettlements.every(({ process_tree_settled: settled }) => settled === true)
        ? "passed"
        : "failed",
    user_state_policy:
      packet.checks.state_root_preserved && packet.checks.outside_sentinel_preserved
        ? "passed"
        : "failed",
  };
  if (!Object.values(observedChecks).every((status) => status === "passed")) {
    fail("one or more public deb post-public observations did not pass");
  }
  return {
    schema_version: 1,
    result_kind: "linux_deb_post_public_observation",
    proof_scope: "post_public_deb_smoke_observation_only",
    disposition: "observation_complete",
    release_evidence_eligible: false,
    repository: RELEASE_REPOSITORY,
    release: {
      tag: receipt.tag,
      source_sha: receipt.source_sha,
      app_version: receipt.app_version,
    },
    artifact: {
      name: asset.name,
      size_bytes: asset.size_bytes,
      sha256: asset.sha256,
    },
    observed_checks: observedChecks,
    limitations: [
      "github_hosted_ubuntu_22_04",
      "linux_deb_amd64_only",
      "native_candidate_packet_not_promoted",
    ],
  };
}

function buildAppImageEvidence(receipt, captureResult) {
  const state = linuxPersistenceCaptureInternals.requireVerifiedPublicAppImageCaptureResult(
    captureResult,
    receipt,
  );
  const { asset, packet, signatureAsset, telemetry, updaterKeyFingerprint } = state;
  if (packet.result !== "passed") fail("public AppImage lifecycle observation did not pass");
  const observedChecks = {
    anonymous_public_bytes: "passed",
    checksum_manifest: "passed",
    source_bound_attestations: "passed",
    updater_signature: "passed",
    package_identity: packet.source.app_version === receipt.app_version ? "passed" : "failed",
    standard_user_runtime:
      packet.receipts.initialize.install_kind === "appimage" ? "passed" : "failed",
    settings_restart: packet.checks.restart_settings_preserved ? "passed" : "failed",
    persistence_degradation:
      packet.checks.persistence_failure_visible && packet.receipts.degraded.health_degraded
        ? "passed"
        : "failed",
    advancing_telemetry: telemetry.samples_advanced ? "passed" : "failed",
    appimage_removed: packet.checks.application_removed ? "passed" : "failed",
    invocation_process_groups_settled: "passed",
    user_state_policy:
      packet.checks.state_root_preserved && packet.checks.outside_sentinel_preserved
        ? "passed"
        : "failed",
  };
  if (!Object.values(observedChecks).every((status) => status === "passed")) {
    fail("one or more public AppImage post-public observations did not pass");
  }
  return {
    schema_version: 1,
    result_kind: "linux_appimage_post_public_observation",
    proof_scope: "post_public_appimage_smoke_observation_only",
    disposition: "observation_complete",
    release_evidence_eligible: false,
    repository: RELEASE_REPOSITORY,
    release: {
      tag: receipt.tag,
      source_sha: receipt.source_sha,
      app_version: receipt.app_version,
    },
    artifact: {
      name: asset.name,
      size_bytes: asset.size_bytes,
      sha256: asset.sha256,
      updater_signature_name: signatureAsset.name,
      updater_signature_sha256: signatureAsset.sha256,
      updater_key_fingerprint: updaterKeyFingerprint,
    },
    observed_checks: observedChecks,
    limitations: [
      "github_hosted_ubuntu_22_04",
      "linux_appimage_amd64_only",
      "appimage_extract_and_run",
      "desktop_window_not_observed",
      "network_isolation_not_enforced",
      "updater_a_to_b_not_exercised",
      "native_candidate_packet_not_promoted",
    ],
  };
}

const PROFILES = Object.freeze({
  appimage: Object.freeze({
    buildEvidence: buildAppImageEvidence,
    displayName: "AppImage",
    outputName: "linux-appimage-observation.json",
    scriptName: "linux-appimage-post-public-smoke.mjs",
    workspacePrefix: "batcave-linux-appimage-post-public-",
    capture: (receipt) => linuxPersistenceCaptureInternals.captureVerifiedPublicAppImage(receipt),
  }),
  deb: Object.freeze({
    buildEvidence: buildDebEvidence,
    displayName: "deb",
    outputName: "linux-deb-observation.json",
    scriptName: "linux-deb-post-public-smoke.mjs",
    workspacePrefix: "batcave-linux-deb-post-public-",
    capture: (receipt) => linuxPersistenceCaptureInternals.captureVerifiedPublicDeb(receipt),
  }),
});

function parseSelectors(profile, argv) {
  if (argv.length !== 2) {
    fail(`usage: node scripts/${profile.scriptName} <tag> <source-sha>`);
  }
  const [tag, sourceSha] = argv;
  parseReleaseTag(tag);
  if (!COMMIT_SHA.test(sourceSha)) {
    fail("source SHA must be an exact lowercase 40-character commit SHA");
  }
  return { sourceSha, tag };
}

function validateCandidateSelectors(candidate, tag, sourceSha) {
  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate.assets) === false) {
    fail("pre-publication candidate inventory is invalid");
  }
  if (candidate.tag !== tag || candidate.source_sha !== sourceSha) {
    fail("pre-publication candidate inventory does not match the workflow selectors");
  }
  return candidate;
}

function readCandidateInventory(tag, sourceSha) {
  const directory = path.dirname(CANDIDATE_FILE);
  const directoryMetadata = fs.lstatSync(directory);
  if (
    !directoryMetadata.isDirectory() ||
    directoryMetadata.isSymbolicLink() ||
    fs.realpathSync(directory) !== directory
  ) {
    fail("pre-publication candidate directory must be a real non-link directory");
  }
  const metadata = fs.lstatSync(CANDIDATE_FILE);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size <= 0 ||
    metadata.size > MAX_RELEASE_READBACK_BYTES ||
    fs.realpathSync(CANDIDATE_FILE) !== CANDIDATE_FILE
  ) {
    fail("pre-publication candidate inventory must be a bounded regular non-link file");
  }
  let candidate;
  try {
    candidate = JSON.parse(fs.readFileSync(CANDIDATE_FILE, "utf8"));
  } catch {
    fail("pre-publication candidate inventory was not valid JSON");
  }
  return validateCandidateSelectors(candidate, tag, sourceSha);
}

async function readAnonymousPublicRelease(tag) {
  const response = await fetch(`${RELEASE_API_ROOT}${encodeURIComponent(tag)}`, {
    credentials: "omit",
    headers: {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
    redirect: "error",
  });
  if (!response.ok) fail(`anonymous release readback failed with HTTP ${response.status}`);
  const contents = await response.text();
  if (Buffer.byteLength(contents) > MAX_RELEASE_READBACK_BYTES) {
    fail("anonymous release readback exceeded its size boundary");
  }
  try {
    return JSON.parse(contents);
  } catch {
    fail("anonymous release readback was not valid JSON");
  }
}

async function run(profile, selectors) {
  if (process.platform !== "linux") fail(`post-public ${profile.displayName} smoke requires Linux`);
  if (process.getuid?.() === 0) {
    fail(`post-public ${profile.displayName} smoke must start as a standard user`);
  }
  if (process.arch !== "x64") {
    fail(`post-public ${profile.displayName} smoke requires the amd64 release host`);
  }
  verifyWorkspaceReleaseVersion(selectors.tag);

  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), profile.workspacePrefix)),
  );
  fs.chmodSync(workspace, 0o700);
  try {
    const candidate = readCandidateInventory(selectors.tag, selectors.sourceSha);
    const release = await readAnonymousPublicRelease(selectors.tag);
    const downloads = path.join(workspace, "public-downloads");
    const verification = await verifyPublicRelease(candidate, release, downloads);
    const result = await profile.capture(verification.receipt);
    const evidence = profile.buildEvidence(verification.receipt, result);
    validateSanitizedReleaseEvidenceValue(evidence);
    return evidence;
  } finally {
    fs.rmSync(workspace, { force: true, recursive: true });
  }
}

async function main(profile, argv) {
  const evidence = await run(profile, parseSelectors(profile, argv));
  try {
    fs.lstatSync(OUTPUT_DIRECTORY);
    fail("fixed post-public output directory must not already exist");
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  try {
    fs.mkdirSync(OUTPUT_DIRECTORY, { mode: 0o700 });
    fs.writeFileSync(
      path.join(OUTPUT_DIRECTORY, profile.outputName),
      `${JSON.stringify(evidence, null, 2)}\n`,
      {
        flag: "wx",
        mode: 0o600,
      },
    );
  } catch (error) {
    fs.rmSync(OUTPUT_DIRECTORY, { force: true, recursive: true });
    throw error;
  }
  console.log(JSON.stringify(evidence));
}

function runLinuxPostPublicSmoke(profile, argv) {
  main(profile, argv).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}

function profileInternals(profile) {
  return Object.freeze({
    parseSelectors: (argv) => parseSelectors(profile, argv),
    validateCandidateSelectors,
  });
}

export const linuxDebPostPublicSmokeInternals = profileInternals(PROFILES.deb);
export const linuxAppImagePostPublicSmokeInternals = profileInternals(PROFILES.appimage);

export const runLinuxDebPostPublicSmoke = (argv) => runLinuxPostPublicSmoke(PROFILES.deb, argv);
export const runLinuxAppImagePostPublicSmoke = (argv) =>
  runLinuxPostPublicSmoke(PROFILES.appimage, argv);
