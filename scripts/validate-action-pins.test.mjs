import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { collectActionPinViolations } from "./validate-action-pins.mjs";

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
  - uses: ./github/actions/local
`,
    },
    (root) => assert.deepEqual(collectActionPinViolations(root), []),
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
