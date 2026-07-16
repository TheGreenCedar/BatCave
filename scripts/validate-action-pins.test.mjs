import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { collectActionPinViolations } from "./validate-action-pins.mjs";

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

function assertStepOrder(job, earlier, later) {
  const earlierIndex = job.indexOf(`- name: ${earlier}`);
  const laterIndex = job.indexOf(`- name: ${later}`);
  assert.notEqual(earlierIndex, -1, `validation step ${earlier} must exist`);
  assert.notEqual(laterIndex, -1, `validation step ${later} must exist`);
  assert.ok(earlierIndex < laterIndex, `${earlier} must run before ${later}`);
}

function withWorkflows(files, run) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-action-pins-"));
  try {
    for (const [name, contents] of Object.entries(files)) {
      const file = path.join(root, ".github", "workflows", name);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, contents);
    }
    return run(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("accepts immutable external pins and local actions", () => {
  withWorkflows(
    {
      "valid.yml": `steps:
  - uses: actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10 # v6.0.3
  - uses: owner/action/subdirectory@0123456789abcdef0123456789abcdef01234567 # v1.2.3
  - { "uses" : owner/flow-action@abcdef0123456789abcdef0123456789abcdef01 } # v3.0.0
  - uses: ./github/actions/local
  - { uses: ./github/actions/flow-local }
  - uses: docker://ghcr.io/owner/image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef # v2.4.0
`,
    },
    (root) => assert.deepEqual(collectActionPinViolations(root), []),
  );
});

test("finds uses nodes across whitespace, quoted keys, and flow mappings", () => {
  withWorkflows(
    {
      "syntax-bypasses.yml": `name: Adversarial action syntax
on: push
env:
  ACTION_KEY: &action-key uses
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses : actions/checkout@v6 # v6
      - "uses": actions/setup-node@v6 # v6
      - { uses: actions/upload-artifact@v6 } # v6
      - *action-key: actions/download-artifact@v8 # v8
`,
    },
    (root) => {
      assert.deepEqual(
        collectActionPinViolations(root).map(({ line, message }) => [line, message]),
        [
          [9, "external action ref must be an exact lowercase 40-character commit SHA; received v6"],
          [10, "external action ref must be an exact lowercase 40-character commit SHA; received v6"],
          [11, "external action ref must be an exact lowercase 40-character commit SHA; received v6"],
          [12, "external action ref must be an exact lowercase 40-character commit SHA; received v8"],
        ],
      );
    },
  );
});

test("rejects duplicate anchors without losing the first uses-key binding", () => {
  withWorkflows(
    {
      "duplicate-anchor.yml": `name: Duplicate anchor shadow
on: push
env:
  ACTION_KEY: &key uses
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - *key: actions/checkout@v6 # v6
    env:
      HARMLESS_KEY: &key harmless
      HARMLESS_VALUE: *key
`,
    },
    (root) => {
      assert.deepEqual(
        collectActionPinViolations(root).map(({ line, message }) => [line, message]),
        [
          [9, "external action ref must be an exact lowercase 40-character commit SHA; received v6"],
          [11, 'duplicate YAML anchor name "key" is not allowed'],
        ],
      );
    },
  );
});

test("rejects mutable Docker tags and accepts exact image digests", () => {
  withWorkflows(
    {
      "docker.yml": `steps:
  - uses: docker://alpine:3.20
  - uses: docker://ghcr.io/owner/image@sha256:abcdef # v2
  - uses: docker://ghcr.io/owner/image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef # v2
`,
    },
    (root) => {
      const violations = collectActionPinViolations(root);
      assert.deepEqual(
        violations.map(({ line }) => line),
        [2, 3],
      );
      assert.ok(
        violations.every(({ message }) => message.includes("exact lowercase sha256 image digest")),
      );
    },
  );
});

test("rejects mutable tags, branches, and short commit prefixes", () => {
  withWorkflows(
    {
      "mutable.yml": `steps:
  - uses: actions/checkout@v6 # v6
  - uses: dtolnay/rust-toolchain@stable # stable
  - uses: owner/action@0123456789ab # commit
`,
    },
    (root) => {
      const violations = collectActionPinViolations(root);
      assert.equal(violations.length, 3);
      assert.ok(violations.every(({ message }) => message.includes("40-character commit SHA")));
    },
  );
});

test("requires a readable version comment beside every external pin", () => {
  withWorkflows(
    {
      "missing-comment.yaml":
        "jobs:\n  audit:\n    uses: owner/repository/.github/workflows/audit.yml@0123456789abcdef0123456789abcdef01234567\n",
    },
    (root) => {
      assert.deepEqual(collectActionPinViolations(root), [
        {
          file: ".github/workflows/missing-comment.yaml",
          line: 3,
          message: "immutable action pin must retain its readable version as an inline comment",
        },
      ]);
    },
  );
});

test("reports violations in stable path and line order", () => {
  withWorkflows(
    {
      "z.yml": "steps:\n  - uses: owner/z@main # main\n",
      "a.yml": "steps:\n  - uses: owner/a@v1 # v1\n  - uses: owner/b@v2 # v2\n",
    },
    (root) => {
      assert.deepEqual(
        collectActionPinViolations(root).map(({ file, line }) => [file, line]),
        [
          [".github/workflows/a.yml", 2],
          [".github/workflows/a.yml", 3],
          [".github/workflows/z.yml", 2],
        ],
      );
    },
  );
});

test("keeps validation toolchains, cache writers, and Linux package transport bounded", () => {
  assert.match(validationWorkflow, /^  workflow_dispatch:$/m);
  assert.equal(validationWorkflow.match(/toolchain: 1\.97\.1/g)?.length, 2);
  assert.equal(
    validationWorkflow.match(
      /shared-key: batcave-validation-\$\{\{ runner\.os \}\}-\$\{\{ runner\.arch \}\}/g,
    )?.length,
    4,
  );
  assert.equal(
    validationWorkflow.match(/save-if: \$\{\{ github\.event_name != 'pull_request' \}\}/g)
      ?.length,
    3,
  );
  assert.equal(validationWorkflow.match(/save-if: false/g)?.length, 1);
  assert.doesNotMatch(validationWorkflow, /cache-on-failure|cache-workspace-crates/);

  const linux = validationJob("linux");
  assert.doesNotMatch(linux, /--bundle-only|linux_package_owned_transport/);
  assertStepOrder(linux, "Reject Rust warnings on the MSRV", "Validate Linux app");
  assertStepOrder(
    linux,
    "Reject private public-release verifier warnings",
    "Validate Linux app",
  );

  const packageTransport = validationJob("linux-package-transport");
  assert.match(packageTransport, /^    name: Linux package transport$/m);
  assert.match(
    packageTransport,
    /bash scripts\/validate-tauri\.sh --bundle-only[\s\S]*linux_package_owned_transport/u,
  );
  assert.match(packageTransport, /^          save-if: false$/m);

  const windows = validationJob("windows");
  assertStepOrder(windows, "Reject Rust warnings", "Validate Windows app");
  assertStepOrder(
    windows,
    "Reject private Windows lifecycle proof warnings",
    "Validate Windows app",
  );

  const macos = validationJob("macos");
  assertStepOrder(
    macos,
    "Reject Rust warnings for both Apple architectures",
    "Validate native macOS app",
  );
  assert.match(
    macos,
    /Verify private macOS updater staging observer[\s\S]*cargo clippy[\s\S]*cargo test/u,
  );
});
