import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const workflowDirectory = new URL("../.github/workflows/", import.meta.url);

function readWorkflow(name) {
  return fs.readFileSync(new URL(name, workflowDirectory), "utf8");
}

const validationWorkflow = readWorkflow("validation.yml");
const bundlesWorkflow = readWorkflow("bundles.yml");
const releaseWorkflow = readWorkflow("release.yml");
const workflowSources = fs
  .readdirSync(workflowDirectory)
  .filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"))
  .map((name) => [name, readWorkflow(name)]);

function workflowJob(workflow, name) {
  const match = workflow.match(
    new RegExp(`^  ${name}:\\n[\\s\\S]*?(?=^  [a-z][a-z0-9-]*:\\n|(?![\\s\\S]))`, "m"),
  );
  assert.ok(match, `workflow job ${name} must exist`);
  return match[0];
}

const trustedMainSave =
  "save-if: ${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}";

function actionStep(job, action) {
  const match = job.match(
    new RegExp(
      `^      - uses: (${action.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}@[0-9a-f]{40})[^\\n]*\\n[\\s\\S]*?(?=^      - |(?![\\s\\S]))`,
      "m",
    ),
  );
  assert.ok(match, `${action} step must exist`);
  return { reference: match[1], source: match[0] };
}

function actionInput(step, name, fallback = undefined) {
  const match = step.source.match(new RegExp(`^          ${name}: (.+)$`, "m"));
  if (fallback === undefined) {
    assert.ok(match, `${name} input must exist on ${step.reference}`);
  }
  return match?.[1] ?? fallback;
}

function jobRunner(job) {
  const match = job.match(/^    runs-on: (.+)$/m);
  assert.ok(match, "job runner must exist");
  return match[1];
}

function toolchainSignature(job) {
  const step = actionStep(job, "dtolnay/rust-toolchain");
  return {
    action: step.reference,
    toolchain: actionInput(step, "toolchain", ""),
    targets: actionInput(step, "targets", ""),
  };
}

function cacheSignature(job) {
  const step = actionStep(job, "Swatinem/rust-cache");
  return {
    action: step.reference,
    sharedKey: actionInput(step, "shared-key"),
    workspaces: actionInput(step, "workspaces"),
    workspaceCrates: actionInput(step, "cache-workspace-crates", "false"),
    saveIf: actionInput(step, "save-if"),
  };
}

test("keeps required pull-request validation restore-only", () => {
  assert.match(
    validationWorkflow,
    /^on:\n  pull_request:\n  push:\n    branches:\n      - main$/m,
  );
  assert.match(validationWorkflow, /^permissions:\n  contents: read$/m);

  for (const name of ["windows", "linux", "macos"]) {
    const job = workflowJob(validationWorkflow, name);
    const cache = cacheSignature(job);
    assert.doesNotMatch(job, /^    if:/m, `${name} must remain an unconditional PR job`);
    assert.equal(cache.workspaceCrates, "false");
    assert.equal(cache.saveIf, trustedMainSave.slice("save-if: ".length));
  }

  const windows = workflowJob(validationWorkflow, "windows");
  assert.match(
    windows,
    /^    env:\n      CARGO_PROFILE_DEV_DEBUG: "0"\n      CARGO_PROFILE_TEST_DEBUG: "0"$/m,
  );
  for (const name of ["linux", "macos"]) {
    assert.doesNotMatch(
      workflowJob(validationWorkflow, name),
      /CARGO_PROFILE_(?:DEV|TEST)_DEBUG/,
    );
  }

  assert.equal(validationWorkflow.split(trustedMainSave).length - 1, 3);
  assert.doesNotMatch(validationWorkflow, /linux-package-cache-seed/);
  const transport = workflowJob(validationWorkflow, "linux-package-transport");
  const trustedPackageProducer = workflowJob(bundlesWorkflow, "linux");
  const transportCache = cacheSignature(transport);
  const producerCache = cacheSignature(trustedPackageProducer);
  assert.equal(transport.match(/validate-tauri\.sh --bundle-only/g)?.length, 1);
  assert.deepEqual(toolchainSignature(transport), toolchainSignature(trustedPackageProducer));
  assert.equal(transportCache.action, producerCache.action);
  assert.equal(transportCache.sharedKey, producerCache.sharedKey);
  assert.equal(transportCache.workspaces, producerCache.workspaces);
  assert.equal(transportCache.workspaceCrates, producerCache.workspaceCrates);
  assert.equal(transportCache.saveIf, "false");
});

test("pins every external action and rejects unsafe cache modes", () => {
  let actionCount = 0;
  for (const [name, source] of workflowSources) {
    const references = [...source.matchAll(/^\s*(?:-\s*)?uses:\s+(\S+)/gm)].map(
      ([, reference]) => reference,
    );
    actionCount += references.length;
    for (const reference of references) {
      if (!reference.startsWith("./")) {
        assert.match(
          reference,
          /@[0-9a-f]{40}$/u,
          `${name}: ${reference} must use an immutable commit`,
        );
      }
    }
    assert.doesNotMatch(source, /cache-on-failure|cache-workspace-crates:\s*true/);
  }
  assert.ok(actionCount > 0);
});

test("uses one trusted Linux package seed and restore-only releases", () => {
  const bundle = workflowJob(bundlesWorkflow, "linux");
  const release = workflowJob(releaseWorkflow, "linux");
  const bundleCache = cacheSignature(bundle);
  const releaseCache = cacheSignature(release);

  assert.match(bundlesWorkflow, /^permissions:\n  contents: read$/m);
  assert.equal(jobRunner(bundle), "ubuntu-22.04");
  assert.equal(jobRunner(release), jobRunner(bundle));
  assert.deepEqual(toolchainSignature(release), toolchainSignature(bundle));
  assert.equal(releaseCache.action, bundleCache.action);
  assert.equal(releaseCache.sharedKey, bundleCache.sharedKey);
  assert.equal(releaseCache.workspaces, bundleCache.workspaces);
  assert.equal(releaseCache.workspaceCrates, bundleCache.workspaceCrates);
  assert.match(bundle, /bash scripts\/validate-tauri\.sh --bundle-only/);
  assert.equal(bundle.match(/validate-tauri\.sh --bundle-only/g)?.length, 1);
  assert.equal(
    bundleCache.sharedKey,
    "batcave-package-release-${{ runner.os }}-${{ runner.arch }}",
  );
  assert.equal(bundleCache.workspaceCrates, "false");
  assert.equal(bundleCache.saveIf, trustedMainSave.slice("save-if: ".length));
  assert.equal(releaseCache.saveIf, "false");
});

test("shares dependency-only bundle caches without release writes", () => {
  for (const name of ["windows", "macos"]) {
    const bundle = workflowJob(bundlesWorkflow, name);
    const release = workflowJob(releaseWorkflow, name);
    const bundleCache = cacheSignature(bundle);
    const releaseCache = cacheSignature(release);

    assert.equal(jobRunner(release), jobRunner(bundle));
    assert.deepEqual(toolchainSignature(release), toolchainSignature(bundle));
    assert.equal(releaseCache.action, bundleCache.action);
    assert.equal(releaseCache.sharedKey, bundleCache.sharedKey);
    assert.equal(releaseCache.workspaces, bundleCache.workspaces);
    assert.equal(releaseCache.workspaceCrates, bundleCache.workspaceCrates);
    assert.equal(
      bundleCache.sharedKey,
      "batcave-bundle-release-${{ runner.os }}-${{ runner.arch }}",
    );
    assert.equal(bundleCache.workspaceCrates, "false");
    assert.equal(bundleCache.saveIf, trustedMainSave.slice("save-if: ".length));
    assert.equal(releaseCache.saveIf, "false");
  }
});
