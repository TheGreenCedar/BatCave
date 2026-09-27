import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { runInNewContext } from "node:vm";
import {
  REQUIRED_STATUS_CHECK_CONTEXTS,
  verifyLiveReleaseControls,
  verifyReleaseControls,
} from "./verify-release-controls.mjs";

const releaseWorkflow = fs.readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url), "utf8",
);
const publishedWorkflow = fs.readFileSync(new URL("../.github/workflows/verify-published-release.yml", import.meta.url), "utf8");
function workflowJob(name) {
  const workflow = name.endsWith("_post_public_smoke") ? publishedWorkflow : releaseWorkflow;
  const match = workflow.match(
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
  assert.doesNotMatch(releaseWorkflow, /RELEASE_ADMIN_READ_TOKEN/);
  const prepare = workflowJob("prepare");
  const finalize = workflowJob("finalize");
  assert.doesNotMatch(prepare, /environment:/u);
  assert.doesNotMatch(workflowJob("windows"), /environment:/u);
  for (const job of [prepare, finalize]) {
    assert.match(job, /actions: read/);
    assert.match(job, /GH_TOKEN: \$\{\{ github.token \}\}/);
    assert.match(job, /node scripts\/verify-release-controls\.mjs/);
  }
  assert.ok(finalize.indexOf("verify-release-controls.mjs") < finalize.indexOf("gh \"${args[@]}\""));
  for (const name of ["windows", "windows_signed", "linux", "macos"]) {
    assert.match(workflowJob(name), /needs: prepare/);
    assert.match(workflowJob(name), /ref: \$\{\{ needs.prepare.outputs.source_sha \}\}/);
  }
});

test("runs the deb smoke on a fresh pinned Ubuntu host after public release publication", () => {
  const job = workflowJob("linux_deb_post_public_smoke");
  assert.match(job, /^    needs: verify_origin$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ github\.sha \}\}/u);
  assert.match(
    job,
    /node scripts\/linux-deb-post-public-smoke\.mjs "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: batcave-release-candidate-\$\{\{ inputs\.tag \}\}[\s\S]*path: post-public-input/u,
  );
  assert.match(
    job,
    /name: Retain sanitized Linux deb post-public observation[\s\S]*name: batcave-linux-deb-post-public-\$\{\{ inputs\.tag \}\}[\s\S]*path: post-public-output\/linux-deb-observation\.json/u,
  );
  assert.doesNotMatch(job, /(?:--deb|--output-dir|RUNNER_TEMP|github\.event|workflow_dispatch)/u);
});

test("runs the AppImage smoke from the same independent public candidate inventory", () => {
  const job = workflowJob("linux_appimage_post_public_smoke");
  assert.match(job, /^    needs: verify_origin$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ github\.sha \}\}/u);
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
    /name: batcave-release-candidate-\$\{\{ inputs\.tag \}\}[\s\S]*path: post-public-input/u,
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
  assert.match(job, /^    needs: verify_origin$/m);
  assert.match(job, /^    runs-on: macos-15$/m);
  assert.match(job, /ref: \$\{\{ github\.sha \}\}/u);
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


test("Windows signing selection rejects unknown or unapproved Azure modes before selecting an environment", () => {
  assert.match(releaseWorkflow, /windows_signing:[\s\S]*?options:\s+- unsigned\s+- azure\s+default: unsigned/u);
  assert.match(releaseWorkflow, /publish:[\s\S]*?default: false/u);
  const prepare = workflowJob("prepare");
  assert.match(prepare, /AZURE_SIGNING_READY: \$\{\{ vars.BATCAVE_ARTIFACT_SIGNING_READY \}\}/u);
  assert.doesNotMatch(prepare, /environment:/u);
  const guard = prepare.match(/          case "\$\{WINDOWS_SIGNING\}"[\s\S]*?          fi/u)?.[0];
  assert.ok(guard, "prepare must reject Azure before the environment job can start");
  for (const [mode, ready, expected] of [
    ["unsigned", "", 0], ["unsigned", "false", 0], ["unsigned", "true", 0],
    ["azure", "true", 0], ["azure", "", 1], ["azure", "false", 1], ["azure", "True", 1],
    ["", "true", 1], ["Azure", "true", 1], ["signed", "true", 1],
  ]) {
    const result = spawnSync("bash", ["-c", guard], {
      env: { ...process.env, WINDOWS_SIGNING: mode, AZURE_SIGNING_READY: ready }, encoding: "utf8",
    });
    assert.equal(result.status, expected, `${mode} ready=${ready}: ${result.stderr}`);
    if (mode === "azure" && expected === 1) assert.match(result.stderr, /existing release environment must be approved/u);
  }
  assert.match(workflowJob("windows_signed"), /if: needs.prepare.outputs.windows_signing == 'azure' && vars.BATCAVE_ARTIFACT_SIGNING_READY == 'true'/u);
  const signedCondition = workflowJob("windows_signed").match(/^    if: (.+)$/mu)[1];
  for (const mode of ["unsigned", "azure"]) {
    for (const ready of ["", "false", "True", "true"]) {
      assert.equal(runInNewContext(signedCondition, {
        needs: { prepare: { outputs: { windows_signing: mode } } },
        vars: { BATCAVE_ARTIFACT_SIGNING_READY: ready },
      }), mode === "azure" && ready === "true");
    }
  }
});

test("Azure pre-login validation requires readiness and every configured authority input", () => {
  const job = workflowJob("windows_signed");
  const preflight = workflowSteps(job).find(s => s.includes("Validate protected Azure signing inputs"));
  const code = preflight.split("        run: |\n")[1];
  assert.ok(code, "signed job must validate inputs before login");
  const required = ["AZURE_CLIENT_ID", "AZURE_TENANT_ID", "AZURE_SUBSCRIPTION_ID", "BATCAVE_ARTIFACT_SIGNING_ENDPOINT", "BATCAVE_ARTIFACT_SIGNING_ACCOUNT", "BATCAVE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE", "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"];
  for (const name of required.filter(n => !n.startsWith("TAURI_"))) {
    assert.ok(job.includes(name + ": ${{ vars." + name + " }}"), `${name} must come from configured variables`);
  }
  assert.doesNotMatch(job, /inputs\.(?:AZURE_|BATCAVE_ARTIFACT_)/u);
  const env = { ...process.env, BATCAVE_ARTIFACT_SIGNING_READY: "true", ...Object.fromEntries(required.map(name => [name, "fixture-present"])) };
  const run = (overrides) => spawnSync("pwsh", ["-NoProfile", "-NonInteractive", "-Command", code], {
    env: { ...env, ...overrides }, encoding: "utf8",
  });
  const clean = run({});
  assert.equal(clean.status, 0, clean.stderr || clean.error?.message);
  for (const name of required) {
    const result = run({ [name]: "" });
    assert.equal(result.status, 1, name);
    assert.ok(result.stderr.includes(`Required protected Azure release input ${name} is missing.`));
  }
  const unapproved = run({ BATCAVE_ARTIFACT_SIGNING_READY: "" });
  assert.equal(unapproved.status, 1);
  assert.match(unapproved.stderr, /must be explicitly confirmed before signing/u);
});

test("finalization requires success of the selected Windows job and preserves all platform gates", () => {
  const job = workflowJob("finalize");
  assert.match(job, /needs: \[prepare, windows, windows_signed, linux, macos\]/u);
  const expression = job.split("    if: >-\n")[1]?.split("\n    runs-on:")[0].trim();
  assert.ok(expression, "finalize must handle the intentionally skipped sibling job");
  const state = (mode) => ({
    prepare: { result: "success", outputs: { windows_signing: mode } },
    linux: { result: "success" }, macos: { result: "success" },
    windows: { result: mode === "unsigned" ? "success" : "skipped" },
    windows_signed: { result: mode === "azure" ? "success" : "skipped" },
  });
  const allowed = (needs, cancelled = false) => runInNewContext(expression, {
    needs, always: () => true, cancelled: () => cancelled,
  });
  for (const mode of ["unsigned", "azure"]) {
    assert.equal(allowed(state(mode)), true);
    assert.equal(allowed(state(mode), true), false);
    for (const name of ["prepare", "linux", "macos", mode === "azure" ? "windows_signed" : "windows"]) {
      for (const result of ["skipped", "failure", "cancelled", "pending"]) {
        const needs = state(mode);
        needs[name].result = result;
        assert.equal(allowed(needs), false, `${mode}: ${name} ${result}`);
      }
    }
    const both = state(mode);
    both.windows.result = both.windows_signed.result = "success";
    assert.equal(allowed(both), false);
  }
  assert.equal(allowed(state("unknown")), false);
});

test("signing evidence stays outside public assets and every candidate rebuild uses the selected mode", () => {
  const steps = workflowSteps(workflowJob("finalize"));
  const evidence = steps.find(s => s.includes("Download selected Azure signing evidence"));
  assert.match(evidence, /if: needs.prepare.outputs.windows_signing == 'azure'/u);
  assert.match(evidence, /name: windows-signing-evidence-\$\{\{ needs.prepare.outputs.tag \}\}/u);
  assert.match(evidence, /path: \$\{\{ runner.temp \}\}\/windows-signing-evidence/u);
  assert.doesNotMatch(evidence, /release-input|\b(?:dist|merge-multiple):/u);
  const publicDownload = steps.find(s => s.includes("pattern:"));
  assert.match(publicDownload, /pattern: batcave-release-\*-\$\{\{ needs.prepare.outputs.tag \}\}/u);
  const resolveTag = text => text.replaceAll("${{ needs.prepare.outputs.tag }}", "v0.2.0");
  const evidenceName = resolveTag(evidence.match(/          name: (.+)/u)[1]);
  const publicPattern = resolveTag(publicDownload.match(/          pattern: (.+)/u)[1]);
  assert.equal(path.matchesGlob(evidenceName, publicPattern), false, "signing evidence cannot match the public distributable glob");
  const binding = steps.find(s => s.includes("Bind selected Windows signing evidence"));
  assert.match(binding, /Azure signing inventory is missing/u);
  const inventories = steps.filter(s => /verify-release-candidate\.mjs inventory /u.test(s));
  assert.equal(inventories.length, 3);
  for (const step of inventories) assert.match(step, /"\$\{WINDOWS_SIGNING\}" "\$\{WINDOWS_SIGNING_INVENTORY\}"/u);
  for (const label of ["Create and verify draft GitHub Release", "Publish verified GitHub Release", "Verify anonymous public release bytes and attestations"]) {
    assert.match(steps.find(s => s.includes(`name: ${label}`)), /if: needs.prepare.outputs.publish == 'true'/u);
  }
  const create = steps.find(s => s.includes("Create and verify draft GitHub Release"));
  assert.ok(create.indexOf("verify-release-candidate.mjs inventory") < create.indexOf('if [[ "${WINDOWS_SIGNING}" == "azure" ]]'));
  assert.match(create, /Authenticode-signed by Albert Najjar/u);
  assert.match(create, /Windows downloads are not Authenticode-signed/u);
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


test("reads pending drafts by release ID and requires the public tag after publication", () => {
  const steps = workflowSteps(workflowJob("finalize"));
  const create = steps.find(s => s.includes("Create and verify draft GitHub Release"));
  const publish = steps.find(s => s.includes("Publish verified GitHub Release"));
  assert.match(create, /gh release view .* --json databaseId --jq .databaseId/);
  assert.match(create, /releases\/\$\{release_id\}/);
  assert.match(publish, /releases\/\$\{BATCAVE_RELEASE_ID\}/);
  assert.match(publish, /Published release tag does not target/);
  for (const step of [create, publish]) {
    const guard = step.match(/          \[\[ -z "\$\{(?:tag_sha|remote_tag_sha)\}"[^\n]+Draft release tag targets another commit[^\n]+/)[0];
    for (const [tagSha, status] of [["", 0], [sourceSha, 0], ["b".repeat(40), 1]]) {
      assert.equal(spawnSync("bash", ["-c", guard], { env: {
        ...process.env, tag_sha: tagSha, remote_tag_sha: tagSha, RELEASE_SOURCE_SHA: sourceSha,
      } }).status, status);
    }
  }
});
