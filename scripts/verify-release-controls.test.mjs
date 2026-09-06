import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { spawnSync } from "node:child_process";
import {
  REQUIRED_STATUS_CHECK_CONTEXTS,
  verifyLiveReleaseControls,
  verifyReleaseControls,
} from "./verify-release-controls.mjs";

const releaseWorkflow = fs.readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url), "utf8",
);
function workflowJob(name) {
  const match = releaseWorkflow.match(
    new RegExp(`^  ${name}:\\n[\\s\\S]*?(?=^  [a-z][a-z0-9_-]*:\\n|(?![\\s\\S]))`, "m"),
  );
  assert.ok(match, `release workflow job ${name} must exist`);
  return match[0];
}
function workflowSteps(job) {
  const steps = job.split("\n    steps:\n")[1];
  assert.ok(steps, "workflow job must define steps");
  return steps.split(/^      - /m).slice(1);
}
const repository = "TheGreenCedar/BatCave";
const sourceSha = "a".repeat(40);
function validControls() {
  const run = {
    id: 123, run_attempt: 2, path: ".github/workflows/validation.yml",
    head_sha: sourceSha, head_branch: "main", event: "push",
    repository: { full_name: repository }, head_repository: { full_name: repository },
    status: "completed", conclusion: "success",
  };
  return {
    repository, sourceSha, mainSha: sourceSha, run,
    jobs: {
      total_count: REQUIRED_STATUS_CHECK_CONTEXTS.length,
      jobs: REQUIRED_STATUS_CHECK_CONTEXTS.map(name => ({
        name, run_id: run.id, run_attempt: run.run_attempt, head_sha: sourceSha,
        status: "completed", conclusion: name === "Dependency review" ? "skipped" : "success",
      })),
    },
  };
}

test("accepts completed validation for exact main without another account or admin credential", () => {
  assert.equal(verifyReleaseControls(validControls()), true);
});

test("rejects wrong source, workflow, repository, event, or incomplete validation", () => {
  for (const mutate of [
    c => c.mainSha = "b".repeat(40),
    c => c.run.head_sha = "b".repeat(40),
    c => c.run.head_branch = "other",
    c => c.run.event = "pull_request",
    c => c.run.path = ".github/workflows/bundles.yml",
    c => c.run.repository.full_name = "other/repo",
    c => c.run.head_repository.full_name = "other/repo",
    c => c.run.status = "in_progress",
    c => c.run.conclusion = "failure",
    c => c.run.conclusion = "cancelled",
    c => c.run = null,
  ]) {
    const c = validControls();
    mutate(c);
    assert.throws(() => verifyReleaseControls(c));
  }
});

test("requires all platform checks from the same completed run attempt", () => {
  for (const mutate of [
    c => c.jobs.jobs.pop(),
    c => c.jobs.total_count++,
    c => c.jobs.jobs[0].name = c.jobs.jobs[1].name,
    c => c.jobs.jobs[0].conclusion = "skipped",
    c => c.jobs.jobs[0].conclusion = "failure",
    c => c.jobs.jobs[0].status = "in_progress",
    c => c.jobs.jobs[0].run_id++,
    c => c.jobs.jobs[0].run_attempt--,
    c => c.jobs.jobs[0].head_sha = "b".repeat(40),
    c => c.jobs.jobs.find(j => j.name === "Dependency review").conclusion = "failure",
  ]) {
    const c = validControls();
    mutate(c);
    assert.throws(() => verifyReleaseControls(c));
  }
});

test("live verification uses only contents and Actions reads and rejects newer failed runs", () => {
  const c = validControls();
  const runs = { workflow_runs: [c.run] };
  const responses = new Map([
    [`repos/${repository}/git/ref/heads/main`, { object: { sha: sourceSha } }],
    [`repos/${repository}/actions/workflows/validation.yml/runs?event=push&branch=main&head_sha=${sourceSha}&per_page=100`, runs],
    [`repos/${repository}/actions/runs/123/jobs?filter=latest&per_page=100`, c.jobs],
  ]);
  const requested = [];
  const request = endpoint => {
    requested.push(endpoint);
    assert.ok(responses.has(endpoint), `unexpected endpoint: ${endpoint}`);
    return responses.get(endpoint);
  };
  assert.equal(verifyLiveReleaseControls(repository, sourceSha, request), true);
  assert.deepEqual(requested, [...responses.keys()]);
  runs.workflow_runs.push({ ...c.run, id: 124, conclusion: "failure" });
  assert.throws(() => verifyLiveReleaseControls(repository, sourceSha, request), /completed successfully/);
  runs.workflow_runs = [];
  assert.throws(() => verifyLiveReleaseControls(repository, sourceSha, request), /Validation run/);
  assert.throws(() => verifyLiveReleaseControls(repository, "main", request), /40-character/);
  assert.throws(() => verifyLiveReleaseControls("bad repo", sourceSha, request), /repository/);
});

test("workflow checks green main before builds and again before publication with built-in token", () => {
  assert.doesNotMatch(releaseWorkflow, /RELEASE_ADMIN_READ_TOKEN|environment: release/);
  const prepare = workflowJob("prepare");
  const finalize = workflowJob("finalize");
  for (const job of [prepare, finalize]) {
    assert.match(job, /actions: read/);
    assert.match(job, /GH_TOKEN: \$\{\{ github.token \}\}/);
    assert.match(job, /node scripts\/verify-release-controls\.mjs/);
  }
  assert.ok(finalize.indexOf("verify-release-controls.mjs") < finalize.indexOf("gh \"${args[@]}\""));
  for (const name of ["windows", "linux", "macos"]) {
    assert.match(workflowJob(name), /needs: prepare/);
    assert.match(workflowJob(name), /ref: \$\{\{ needs.prepare.outputs.source_sha \}\}/);
  }
});

test("runs the deb smoke on a fresh pinned Ubuntu host after public release publication", () => {
  const job = workflowJob("linux_deb_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /node scripts\/linux-deb-post-public-smoke\.mjs "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-input/u,
  );
  assert.match(
    job,
    /name: Retain sanitized Linux deb post-public observation[\s\S]*name: batcave-linux-deb-post-public-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-output\/linux-deb-observation\.json/u,
  );
  assert.doesNotMatch(job, /(?:--deb|--output-dir|RUNNER_TEMP|github\.event|workflow_dispatch)/u);
});

test("runs the AppImage smoke from the same independent public candidate inventory", () => {
  const job = workflowJob("linux_appimage_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /dtolnay\/rust-toolchain@[0-9a-f]{40}[\s\S]*bash scripts\/install-linux-deps\.sh[\s\S]*cargo build --quiet --locked[\s\S]*--bin batcave-verify-updater-signature/u,
  );
  assert.match(
    job,
    /node scripts\/linux-appimage-post-public-smoke\.mjs "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-input/u,
  );
  assert.match(
    job,
    /name: Retain sanitized Linux AppImage post-public observation[\s\S]*path: post-public-output\/linux-appimage-observation\.json/u,
  );
  assert.doesNotMatch(
    job,
    /(?:--appimage|--output-dir|RUNNER_TEMP|github\.event|workflow_dispatch)/u,
  );
});

test("runs the macOS updater observer through the closed Rust-owned staging profile", () => {
  const job = workflowJob("macos_updater_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: macos-15$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /cargo run --quiet --locked[\s\S]*--bin batcave-install-smoke --features private-release-verifier -- "\$\{RELEASE_TAG\}" macos-updater/u,
  );
  assert.match(
    job,
    /node scripts\/validate-macos-updater-post-public-observation\.mjs "\$\{observation\}" "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: Retain sanitized macOS updater post-public observation[\s\S]*path: post-public-output\/macos-updater-observation\.json/u,
  );
  assert.doesNotMatch(job, /macos-dmg|hdiutil|(?:--archive|--signature|--output-dir|RUNNER_TEMP)/u);
});

test("gates pre-attestation and complete release inventories before unconditional upload", () => {
  const steps = workflowSteps(workflowJob("finalize"));
  const stepIndex = (label) => steps.findIndex((step) => step.includes(`name: ${label}`));
  const checksums = stepIndex("Generate checksums");
  const preAttestation = stepIndex("Verify pre-attestation release inventory");
  const attest = stepIndex("Generate build provenance");
  const retain = stepIndex("Retain provenance with release files");
  const complete = stepIndex("Verify complete release inventory");
  const upload = steps.findIndex(
    (step) =>
      step.includes("actions/upload-artifact@") &&
      step.includes("name: batcave-release-${{ needs.prepare.outputs.tag }}"),
  );
  const candidateUpload = stepIndex("Retain exact pre-publication candidate inventory");
  const create = stepIndex("Create and verify draft GitHub Release");

  for (const [label, index] of [
    ["checksums", checksums],
    ["pre-attestation inventory", preAttestation],
    ["attestation", attest],
    ["retained provenance", retain],
    ["complete inventory", complete],
    ["final artifact upload", upload],
    ["pre-publication candidate upload", candidateUpload],
    ["draft release", create],
  ]) {
    assert.ok(index >= 0, `finalize must contain ${label}`);
  }
  assert.ok(checksums < preAttestation && preAttestation < attest);
  assert.ok(
    attest < retain &&
      retain < complete &&
      complete < upload &&
      upload < candidateUpload &&
      candidateUpload < create,
  );

  assert.match(
    steps[preAttestation],
    /verify-release-candidate\.mjs verify-inventory .* pre-attestation dist/u,
  );
  assert.match(steps[complete], /verify-release-candidate\.mjs inventory .* dist /u);
  assert.doesNotMatch(steps[preAttestation], /^\s*if:/mu);
  assert.doesNotMatch(steps[complete], /^\s*if:/mu);
  assert.doesNotMatch(steps[upload], /^\s*if:/mu);
  assert.match(
    steps[candidateUpload],
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}/u,
  );
  assert.match(steps[candidateUpload], /path: \$\{\{ runner\.temp \}\}\/release-candidate\.json/u);
  assert.doesNotMatch(steps[candidateUpload], /^\s*if:/mu);
});


test("ad-hoc macOS publication is explicit and restricted to previews", () => {
  const guard = releaseWorkflow.match(/          case "\$\{MACOS_SIGNING\}"[\s\S]*?          fi/)[0];
  for (const [mode, channel, expected] of [
    ["adhoc", "prerelease", 0], ["adhoc", "stable", 1],
    ["notarized", "prerelease", 0], ["notarized", "stable", 0],
    ["", "prerelease", 1], ["unsigned", "prerelease", 1], ["Notarized", "stable", 1],
  ]) {
    const result = spawnSync("bash", ["-c", guard], {
      env: { ...process.env, MACOS_SIGNING: mode, INPUT_CHANNEL: channel },
    });
    assert.equal(result.status, expected, `${mode} ${channel}`);
  }
  const steps = workflowSteps(workflowJob("macos"));
  const preview = steps.find(s => s.includes("Verify ad-hoc preview"));
  assert.match(preview, /--bin batcave-verify-updater-signature/);
  assert.match(preview, /--mode adhoc --updater-archive "\$\{verified\}"/);
  assert.ok(preview.indexOf("batcave-verify-updater-signature") < preview.indexOf("--mode adhoc"));
  const credentials = steps.find(s => s.includes("Prepare Apple signing credentials"));
  assert.match(credentials, /if: inputs.macos_signing == 'notarized'/);
  assert.match(credentials, /Required release secret .* is missing/);
  assert.match(releaseWorkflow, /macOS preview is ad-hoc signed and is not notarized/);
});
