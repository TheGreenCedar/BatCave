import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const validationWorkflow = fs.readFileSync(
  new URL("../.github/workflows/validation.yml", import.meta.url),
  "utf8",
);

function validationJob(name) {
  const match = validationWorkflow.match(
    new RegExp(`^  ${name}:\\n[\\s\\S]*?(?=^  [a-z][a-z0-9-]*:\\n|(?![\\s\\S]))`, "m"),
  );
  assert.ok(match, `validation workflow job ${name} must exist`);
  return match[0];
}

const trustedMainSave =
  "save-if: ${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}";

test("keeps required pull-request validation restore-only", () => {
  assert.match(
    validationWorkflow,
    /^on:\n  pull_request:\n  push:\n    branches:\n      - main$/m,
  );
  assert.match(validationWorkflow, /^permissions:\n  contents: read$/m);

  for (const name of ["windows", "linux", "macos"]) {
    const job = validationJob(name);
    assert.doesNotMatch(job, /^    if:/m, `${name} must remain a required pull-request job`);
    assert.match(job, new RegExp(trustedMainSave.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }

  assert.equal(validationWorkflow.split(trustedMainSave).length - 1, 4);
  assert.doesNotMatch(validationWorkflow, /cache-on-failure|cache-workspace-crates:\s*true/);
});

test("uses the cache family and toolchains consumed by current validation", () => {
  assert.equal(
    validationWorkflow.match(
      /shared-key: batcave-validation-\$\{\{ runner\.os \}\}-\$\{\{ runner\.arch \}\}/g,
    )?.length,
    3,
  );
  assert.equal(
    validationWorkflow.match(
      /Swatinem\/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4/g,
    )?.length,
    4,
  );
  assert.equal(validationWorkflow.match(/toolchain: 1\.97\.1/g)?.length, 2);
  assert.match(validationJob("linux"), /^    runs-on: ubuntu-22\.04$/m);

  const actionReferences = [...validationWorkflow.matchAll(/^\s*- uses: (\S+)/gm)].map(
    ([, reference]) => reference,
  );
  assert.ok(actionReferences.length > 0);
  for (const reference of actionReferences) {
    assert.match(reference, /@[0-9a-f]{40}$/u, `${reference} must use an immutable commit`);
  }
});

test("seeds the package dependency graph only on trusted main pushes", () => {
  const job = validationJob("linux-package-cache-seed");
  assert.match(job, /^    name: Linux package cache seed$/m);
  assert.match(
    job,
    /^    if: github\.event_name == 'push' && github\.ref == 'refs\/heads\/main'$/m,
  );
  assert.match(
    job,
    /^          shared-key: batcave-package-release-\$\{\{ runner\.os \}\}-\$\{\{ runner\.arch \}\}$/m,
  );
  assert.match(job, /^          cache-workspace-crates: false$/m);
  assert.match(job, /^          save-if: .*refs\/heads\/main.*$/m);
  assert.match(job, /bash scripts\/validate-tauri\.sh --bundle-only/);
  assert.doesNotMatch(job, /cargo test|pull_request/);
});
