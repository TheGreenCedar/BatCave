import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import {
  validateReleaseEvidencePacket,
  validateSanitizedReleaseEvidenceValue,
} from "./validate-release-evidence-packet.mjs";
import {
  RELEASE_REPOSITORY,
  RELEASE_SOURCE_REF,
  RELEASE_SIGNER_WORKFLOW,
  requireVerifiedPublicReleaseReceipt,
  verifyPublicRelease,
} from "./verify-public-release.mjs";
import { verifyLivePublishedReleaseOrigin } from "./verify-published-release-origin.mjs";
import { parseReleaseTag } from "./verify-release-version.mjs";

const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const RUN_ID = /^[1-9][0-9]*$/u;
const verifiedOrigins = new WeakSet();
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

function readOrigin(endpoint) {
  const result = spawnSync("gh", ["api", endpoint], {
    encoding: "utf8",
    timeout: 30_000,
    maxBuffer: 2 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) fail("could not read original release identity");
  return JSON.parse(result.stdout);
}

function verifyOrigin(selectors, read = readOrigin) {
  let run;
  verifyLivePublishedReleaseOrigin(
    selectors.tag,
    selectors.sourceSha,
    selectors.runId,
    (endpoint) => {
      const value = read(endpoint);
      if (endpoint === `repos/${RELEASE_REPOSITORY}/actions/runs/${selectors.runId}`) run = value;
      return value;
    },
  );
  if (!Number.isSafeInteger(run.run_attempt) || run.run_attempt <= 0) {
    fail("original release attempt must be a positive safe integer");
  }
  const origin = Object.freeze({
    tag: selectors.tag,
    sourceSha: selectors.sourceSha,
    runId: run.id,
    runAttempt: run.run_attempt,
  });
  verifiedOrigins.add(origin);
  return origin;
}

function requireOrigin(origin, receipt) {
  if (
    !verifiedOrigins.has(origin) ||
    origin.tag !== receipt.tag ||
    origin.sourceSha !== receipt.source_sha
  ) {
    fail("packet requires the matching in-process original release verification");
  }
}

function ubuntuHost(host, glibcVersion) {
  const version = /^Ubuntu (22\.04)(?:\.\d+)? LTS$/u.exec(host.os_version);
  if (
    host.platform !== "linux" ||
    host.architecture !== "x86_64" ||
    !version ||
    !/^2\.35(?:\.\d+)?$/u.test(glibcVersion ?? "")
  ) {
    fail("packet requires observed Ubuntu 22.04 x86_64 with glibc 2.35");
  }
  return `ubuntu-${version[1]}`;
}

function packetFromObservedState(kind, receipt, state, origin, glibcVersion) {
  const { asset, packet } = state;
  if (
    packet.result !== "passed" ||
    packet.source.source_sha !== receipt.source_sha ||
    packet.source.app_version !== receipt.app_version ||
    packet.artifact.sha256 !== asset.sha256 ||
    !receipt.assets.some(
      (verified) =>
        verified.name === asset.name &&
        verified.sha256 === asset.sha256 &&
        verified.size_bytes === asset.size_bytes &&
        verified.public_url === asset.public_url,
    ) ||
    !Object.values(packet.checks).every((passed) => passed === true) ||
    !state.telemetry?.samples_advanced ||
    packet.receipts.degraded.health_degraded !== true ||
    (kind === "deb" &&
      (state.rootSettlements?.length !== 5 ||
        !state.rootSettlements.every((settled) => settled.process_tree_settled === true)))
  ) {
    fail("packet requires matching passed public-package native observations");
  }
  const osVersion = ubuntuHost(packet.host, glibcVersion);
  const check = (status, outcome) => ({ status, outcome });
  const limitations = {
    desktop_window_not_observed: {
      disposition: "blocked",
      summary:
        "Packaged CLI phases ran; no mapped production desktop window or rendered UI was observed.",
    },
    github_hosted_ubuntu_22_04: {
      disposition: "not_applicable",
      summary:
        "Observed Ubuntu 22.04 x86_64 glibc host; this packet does not qualify other support profiles.",
    },
    qualification_review_pending: {
      disposition: "blocked",
      summary:
        "Blocked launch and independent native qualification review remain; support-contract status is unchanged.",
    },
  };
  if (kind === "deb") {
    limitations.deb_checksum_attestation_only = {
      disposition: "not_applicable",
      summary:
        "Debian package trust uses matching public checksums and source-bound GitHub attestations.",
    };
  } else {
    limitations.appimage_extract_and_run = {
      disposition: "not_applicable",
      summary:
        "Fixed extract-and-run staging was observed; no conventional package installation was performed.",
    };
    limitations.network_isolation_not_enforced = {
      disposition: "blocked",
      summary: "This run did not enforce network isolation.",
    };
    limitations.updater_a_to_b_not_exercised = {
      disposition: "blocked",
      summary: "The updater payload signature was verified; no A-to-B update was performed.",
    };
  }
  const evidence = {
    schema_version: 1,
    packet_kind: "release_evidence",
    packet_id: `ubuntu-22-04-${kind}-${receipt.source_sha.slice(0, 12)}-${origin.runId}-${origin.runAttempt}`,
    observed_at_utc: packet.observed_at_utc,
    release: {
      repository: RELEASE_REPOSITORY,
      tag: receipt.tag,
      channel: parseReleaseTag(receipt.tag).prerelease ? "prerelease" : "stable",
      source_sha: receipt.source_sha,
      main_sha: receipt.source_sha,
      release_target_sha: receipt.source_sha,
      release_url: `https://github.com/${RELEASE_REPOSITORY}/releases/tag/${receipt.tag}`,
      workflow_run: {
        workflow_file: ".github/workflows/release.yml",
        run_id: origin.runId,
        run_attempt: origin.runAttempt,
        url: `https://github.com/${RELEASE_REPOSITORY}/actions/runs/${origin.runId}/attempts/${origin.runAttempt}`,
      },
    },
    platform: {
      support_contract_version: 1,
      profile_id: "ubuntu-22.04-x86_64-glibc",
      proof: { declaration: "declared", source: "source_enforced", native: "observed" },
      os: "linux",
      os_version: osVersion,
      architecture: "x86_64",
      runtime: { libc_family: "glibc" },
      package: { kind, architecture: "x86_64", asset_name: asset.name },
    },
    assets: [
      {
        name: asset.name,
        size_bytes: asset.size_bytes,
        sha256: asset.sha256,
        api_digest: asset.sha256,
        public_url: asset.public_url,
        attestation: {
          verified: true,
          repository: RELEASE_REPOSITORY,
          source_sha: receipt.source_sha,
          source_ref: RELEASE_SOURCE_REF,
          signer_workflow: RELEASE_SIGNER_WORKFLOW,
        },
        signatures:
          kind === "appimage"
            ? {
                tauri_updater: { identity: state.updaterKeyFingerprint, verified: true },
              }
            : {},
      },
    ],
    checks: {
      install: {
        anonymous_download: check(
          "passed",
          "Anonymous public bytes matched the independently retained candidate inventory.",
        ),
        checksum: check(
          "passed",
          "Selected bytes matched the GitHub API digest and the complete public checksum manifest.",
        ),
        package_install:
          kind === "deb"
            ? check(
                "passed",
                "Exact public deb installed through an owned transient unit; package-owned GUI and CLI files were verified.",
              )
            : check(
                "not_applicable",
                "AppImage used fixed extract-and-run staging; no conventional package installation was observed.",
              ),
      },
      runtime: {
        degradation: check(
          "passed",
          "Packaged CLI reported degraded persistence and retained corrupt settings bytes.",
        ),
        launch: check(
          "blocked",
          "Packaged CLI phases completed; a mapped production desktop window and rendered UI remain unobserved.",
        ),
        release_identity: check(
          "passed",
          "Packaged CLI phases reported the exact public source, version and package install kind.",
        ),
        settings: check(
          "passed",
          "Packaged CLI restart preserved initialized settings in an isolated current-user root.",
        ),
        telemetry: check(
          "passed",
          "Two strict packaged CLI core-runtime samples advanced; this does not prove UI updates.",
        ),
      },
      cleanup: {
        application_removed: check(
          "passed",
          kind === "deb"
            ? "Package purge removed the GUI, CLI and observed package-owned files."
            : "The verified private AppImage and its staged workspace were removed.",
        ),
        owned_runtime_cleanup: check(
          "passed",
          kind === "deb"
            ? "Owned invocation process groups and all five root units settled before workspace cleanup."
            : "Owned invocation process groups settled before private workspace cleanup.",
        ),
        user_state_policy: check(
          "passed",
          "Package removal preserved isolated user state and the outside sentinel before workspace cleanup.",
        ),
      },
    },
    limitations: Object.fromEntries(
      Object.entries(limitations).sort(([a], [b]) => a.localeCompare(b)),
    ),
  };
  return validateReleaseEvidencePacket(evidence);
}

function buildReleasePacket(profile, receipt, captureResult, origin) {
  requireVerifiedPublicReleaseReceipt(receipt);
  requireOrigin(origin, receipt);
  // Retain the existing complete observation gates and both process-local capture brands.
  profile.buildEvidence(receipt, captureResult);
  const state = profile.requireCapture(captureResult, receipt);
  const glibcVersion = process.report.getReport().header.glibcVersionRuntime;
  return packetFromObservedState(profile.kind, receipt, state, origin, glibcVersion);
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
    kind: "appimage",
    packetOutputName: "linux-appimage-release-evidence.json",
    requireCapture: linuxPersistenceCaptureInternals.requireVerifiedPublicAppImageCaptureResult,
    buildEvidence: buildAppImageEvidence,
    displayName: "AppImage",
    outputName: "linux-appimage-observation.json",
    scriptName: "linux-appimage-post-public-smoke.mjs",
    workspacePrefix: "batcave-linux-appimage-post-public-",
    capture: (receipt) => linuxPersistenceCaptureInternals.captureVerifiedPublicAppImage(receipt),
  }),
  deb: Object.freeze({
    kind: "deb",
    packetOutputName: "linux-deb-release-evidence.json",
    requireCapture: linuxPersistenceCaptureInternals.requireVerifiedPublicDebCaptureResult,
    buildEvidence: buildDebEvidence,
    displayName: "deb",
    outputName: "linux-deb-observation.json",
    scriptName: "linux-deb-post-public-smoke.mjs",
    workspacePrefix: "batcave-linux-deb-post-public-",
    capture: (receipt) => linuxPersistenceCaptureInternals.captureVerifiedPublicDeb(receipt),
  }),
});

function parseSelectors(profile, argv) {
  if (argv.length !== 2 && argv.length !== 3) {
    fail(
      `usage: node scripts/${profile.scriptName} <tag> <source-sha> [<original-release-run-id>]`,
    );
  }
  const [tag, sourceSha, runId] = argv;
  parseReleaseTag(tag);
  if (!COMMIT_SHA.test(sourceSha)) {
    fail("source SHA must be an exact lowercase 40-character commit SHA");
  }
  if (runId !== undefined && (!RUN_ID.test(runId) || !Number.isSafeInteger(Number(runId)))) {
    fail("original release run ID must be a positive safe integer");
  }
  return { sourceSha, tag, ...(runId === undefined ? {} : { runId }) };
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
  parseReleaseTag(selectors.tag);

  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), profile.workspacePrefix)),
  );
  fs.chmodSync(workspace, 0o700);
  try {
    const candidate = readCandidateInventory(selectors.tag, selectors.sourceSha);
    const origin = selectors.runId === undefined ? null : verifyOrigin(selectors);
    const release = await readAnonymousPublicRelease(selectors.tag);
    const downloads = path.join(workspace, "public-downloads");
    const verification = await verifyPublicRelease(candidate, release, downloads);
    const result = await profile.capture(verification.receipt);
    const evidence = profile.buildEvidence(verification.receipt, result);
    validateSanitizedReleaseEvidenceValue(evidence);
    const packet = origin
      ? buildReleasePacket(profile, verification.receipt, result, origin)
      : null;
    return { evidence, packet };
  } finally {
    fs.rmSync(workspace, { force: true, recursive: true });
  }
}

async function main(profile, argv) {
  const { evidence, packet } = await run(profile, parseSelectors(profile, argv));
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
    if (packet) {
      fs.writeFileSync(
        path.join(OUTPUT_DIRECTORY, profile.packetOutputName),
        `${JSON.stringify(packet, null, 2)}\n`,
        {
          flag: "wx",
          mode: 0o600,
        },
      );
    }
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
    verifyOrigin,
    requireOrigin,
    packetFromObservedState: (receipt, state, origin, glibcVersion) =>
      packetFromObservedState(profile.kind, receipt, state, origin, glibcVersion),
    buildReleasePacket: (receipt, captureResult, origin) =>
      buildReleasePacket(profile, receipt, captureResult, origin),
  });
}

export const linuxDebPostPublicSmokeInternals = profileInternals(PROFILES.deb);
export const linuxAppImagePostPublicSmokeInternals = profileInternals(PROFILES.appimage);

export const runLinuxDebPostPublicSmoke = (argv) => runLinuxPostPublicSmoke(PROFILES.deb, argv);
export const runLinuxAppImagePostPublicSmoke = (argv) =>
  runLinuxPostPublicSmoke(PROFILES.appimage, argv);
