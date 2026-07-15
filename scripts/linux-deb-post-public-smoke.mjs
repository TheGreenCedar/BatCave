import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import { validateSanitizedReleaseEvidenceValue } from "./validate-release-evidence-packet.mjs";
import {
  RELEASE_REPOSITORY,
  verifyPublicRelease,
} from "./verify-public-release.mjs";
import {
  parseReleaseTag,
  verifyWorkspaceReleaseVersion,
} from "./verify-release-version.mjs";

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
const OUTPUT_FILE = path.join(OUTPUT_DIRECTORY, "linux-deb-observation.json");

function fail(message) {
  throw new Error(message);
}

function parseSelectors(argv) {
  if (argv.length !== 2) {
    fail("usage: node scripts/linux-deb-post-public-smoke.mjs <tag> <source-sha>");
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

function buildEvidence(receipt, captureResult) {
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
  const evidence = {
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
  validateSanitizedReleaseEvidenceValue(evidence);
  return evidence;
}

async function run(selectors) {
  if (process.platform !== "linux") fail("post-public deb smoke requires Linux");
  if (process.getuid?.() === 0) fail("post-public deb smoke must start as a standard user");
  if (process.arch !== "x64") fail("post-public deb smoke requires the amd64 release host");
  verifyWorkspaceReleaseVersion(selectors.tag);

  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-deb-post-public-")),
  );
  fs.chmodSync(workspace, 0o700);
  try {
    const candidate = readCandidateInventory(selectors.tag, selectors.sourceSha);
    const release = await readAnonymousPublicRelease(selectors.tag);
    const downloads = path.join(workspace, "public-downloads");
    const verification = await verifyPublicRelease(candidate, release, downloads);
    const result = await linuxPersistenceCaptureInternals.captureVerifiedPublicDeb(
      verification.receipt,
    );
    return buildEvidence(verification.receipt, result);
  } finally {
    fs.rmSync(workspace, { force: true, recursive: true });
  }
}

async function main(argv) {
  const evidence = await run(parseSelectors(argv));
  try {
    fs.lstatSync(OUTPUT_DIRECTORY);
    fail("fixed post-public output directory must not already exist");
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  try {
    fs.mkdirSync(OUTPUT_DIRECTORY, { mode: 0o700 });
    fs.writeFileSync(OUTPUT_FILE, `${JSON.stringify(evidence, null, 2)}\n`, {
      flag: "wx",
      mode: 0o600,
    });
  } catch (error) {
    fs.rmSync(OUTPUT_DIRECTORY, { force: true, recursive: true });
    throw error;
  }
  console.log(JSON.stringify(evidence));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}

export const linuxDebPostPublicSmokeInternals = {
  parseSelectors,
  validateCandidateSelectors,
};
