import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readCargoVersion } from "./verify-release-version.mjs";

test("Linux bundle filename mismatch reports the Cargo version", () => {
  const repoRoot = fileURLToPath(new URL("../", import.meta.url));
  const cargoVersion = readCargoVersion(repoRoot);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-bundle-"));
  try {
    const bin = path.join(root, "bin");
    const bundle = path.join(root, "bundle");
    fs.mkdirSync(bin);
    fs.mkdirSync(path.join(bundle, "deb"), { recursive: true });
    fs.mkdirSync(path.join(bundle, "appimage"), { recursive: true });

    const uname = path.join(bin, "uname");
    const dpkgDeb = path.join(bin, "dpkg-deb");
    fs.writeFileSync(uname, "#!/usr/bin/env bash\nprintf 'Linux\\n'\n", { mode: 0o755 });
    fs.writeFileSync(dpkgDeb, `#!/usr/bin/env bash\nprintf '${cargoVersion}\\n'\n`, {
      mode: 0o755,
    });
    fs.writeFileSync(path.join(bundle, "deb", "BatCave_wrong_amd64.deb"), "fixture");
    fs.writeFileSync(path.join(bundle, "appimage", "BatCave_wrong_amd64.AppImage"), "fixture");

    const result = spawnSync(
      "bash",
      [
        fileURLToPath(new URL("./verify-linux-bundle.sh", import.meta.url)),
        "--bundle-root",
        bundle,
      ],
      {
        encoding: "utf8",
        env: { ...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}` },
      },
    );
    assert.equal(result.status, 1);
    assert.ok(
      result.stderr.includes(
        `Expected BatCave_wrong_amd64.deb to contain version _${cargoVersion}_.`,
      ),
    );
    assert.doesNotMatch(result.stderr, /unbound variable/u);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
