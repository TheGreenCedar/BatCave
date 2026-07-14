import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import * as prototypeModule from "./native-artifact-consumption-authority.prototype.mjs";
import {
  NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_PROFILES,
  acquireNativeArtifactConsumptionPrototype,
  closeNativeArtifactConsumptionPrototype,
  consumeNativeArtifactConsumptionPrototype,
  validateNativeArtifactConsumptionPrototypeResult,
} from "./native-artifact-consumption-authority.prototype.mjs";

const PRIVATE_ROOT_PREFIX = "batcave-consumption-prototype-";
const PAYLOAD = Buffer.from("BatCave private consumption authority prototype bytes\n");
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
let scratchRoot;

function digest(bytes) {
  return `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
}

function binding(profileId, overrides = {}) {
  return {
    schema_version: 1,
    binding_id: `prototype-${profileId.replaceAll(/[^a-z0-9]+/gu, "-")}`,
    profile_id: profileId,
    asset: {
      name: "selected-artifact.bin",
      size_bytes: PAYLOAD.length,
      sha256: digest(PAYLOAD),
    },
    timeouts: {
      step_timeout_ms: 1_000,
      termination_timeout_ms: 1_000,
    },
    ...overrides,
  };
}

async function verifiedRootFor(value = PAYLOAD) {
  const root = await fs.mkdtemp(path.join(scratchRoot, "verified-"));
  await fs.writeFile(path.join(root, "selected-artifact.bin"), value, { mode: 0o600 });
  return root;
}

async function newPrivateRoots(before) {
  return (await fs.readdir(os.tmpdir()))
    .filter((name) => name.startsWith(PRIVATE_ROOT_PREFIX) && !before.has(name))
    .map((name) => path.join(os.tmpdir(), name));
}

before(async () => {
  scratchRoot = await fs.mkdtemp(path.join(os.tmpdir(), "batcave-authority-tests-"));
});

after(async () => {
  await fs.rm(scratchRoot, { recursive: true, force: true });
});

test("exports operations but no authority, path, handle, command, or callback seam", () => {
  assert.deepEqual(Object.keys(prototypeModule).sort(), [
    "NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_PROFILES",
    "NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION",
    "NativeArtifactConsumptionPrototypeError",
    "acquireNativeArtifactConsumptionPrototype",
    "closeNativeArtifactConsumptionPrototype",
    "consumeNativeArtifactConsumptionPrototype",
    "validateNativeArtifactConsumptionPrototypeResult",
  ]);
  assert.equal(
    Object.keys(prototypeModule).some((name) =>
      /(?:authority|callback|command|descriptor|handle|path|receipt)/iu.test(
        name.replace("ConsumptionAuthority", ""),
      ),
    ),
    false,
  );
});

test("all closed profiles consume only through the fixed non-installing probe", async () => {
  for (const profileId of NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_PROFILES) {
    const root = await verifiedRootFor();
    const capability = await acquireNativeArtifactConsumptionPrototype(binding(profileId), {
      verified_root: root,
    });
    assert.deepEqual(Object.keys(capability).sort(), [
      "asset",
      "binding_id",
      "profile_id",
      "proof_scope",
      "schema_version",
    ]);
    const result = await consumeNativeArtifactConsumptionPrototype(capability);
    assert.equal(result.disposition, "prototype_consumed");
    assert.deepEqual(result.failure_boundaries, []);
    assert.deepEqual(result.boundaries, {
      acquisition: "passed",
      consumption: "passed",
      timeout: "not_triggered",
      settlement: "passed",
      cleanup: "passed",
      residue: "passed",
    });
    assert.deepEqual(result.observed_bytes, {
      size_bytes: PAYLOAD.length,
      sha256: digest(PAYLOAD),
    });
    assert.equal(result.claims.fixed_probe_completed, true);
    assert.equal(result.claims.package_bytes_executed, false);
    assert.equal(result.claims.package_installed_or_staged, false);
    assert.equal(result.claims.native_proven, false);
    assert.equal(result.native_execution_receipt, null);
    assert.equal(result.evidence_packet, null);
    assert.equal(validateNativeArtifactConsumptionPrototypeResult(result), result);
    assert.doesNotMatch(
      JSON.stringify({ capability, result }),
      /(?:batcave-consumption-prototype|\/private\/|[A-Za-z]:\\)/u,
    );
  }
});

test("forged, reconstructed, callback, descriptor, and evidence inputs cannot reach authority", async () => {
  const root = await verifiedRootFor();
  const capability = await acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
    verified_root: root,
  });
  let callbackCalled = false;
  const forbidden = [
    () => {
      callbackCalled = true;
    },
    { command: "dpkg", arguments: ["--install"] },
    { path: path.join(root, "selected-artifact.bin") },
    { handle: 3 },
    { descriptor: { native_proven: true } },
    { evidence_packet: { packet_kind: "release_evidence" } },
  ];
  for (const value of forbidden) {
    await assert.rejects(
      () => consumeNativeArtifactConsumptionPrototype(capability, value),
      /does not accept a command, callback, path, handle, descriptor, status, or evidence/u,
    );
  }
  assert.equal(callbackCalled, false);
  await assert.rejects(
    () => consumeNativeArtifactConsumptionPrototype({ ...capability }),
    /live process-local capability/u,
  );
  assert.throws(
    () =>
      validateNativeArtifactConsumptionPrototypeResult({
        disposition: "prototype_consumed",
        claims: { native_proven: false },
      }),
    /process-local result/u,
  );
  const result = await consumeNativeArtifactConsumptionPrototype(capability);
  assert.equal(result.disposition, "prototype_consumed");
});

test("the private copy remains bound after the original public path is replaced", async () => {
  const root = await verifiedRootFor();
  const source = path.join(root, "selected-artifact.bin");
  const capability = await acquireNativeArtifactConsumptionPrototype(binding("linux:appimage"), {
    verified_root: root,
  });
  await fs.rename(source, `${source}.verified`);
  await fs.writeFile(source, "hostile replacement bytes", { mode: 0o600 });

  const result = await consumeNativeArtifactConsumptionPrototype(capability);
  assert.equal(result.disposition, "prototype_consumed");
  assert.equal(result.observed_bytes.sha256, digest(PAYLOAD));
});

test("private-path substitution is detected before the fixed consumer starts", async () => {
  const root = await verifiedRootFor();
  const capability = await acquireNativeArtifactConsumptionPrototype(binding("macos:dmg"), {
    verified_root: root,
  });
  const originalLstat = fs.lstat;
  let swapped = false;
  fs.lstat = async (target, ...arguments_) => {
    const targetPath = String(target);
    if (!swapped && targetPath.includes(PRIVATE_ROOT_PREFIX)) {
      swapped = true;
      await fs.rename(targetPath, `${targetPath}.owned`);
      await fs.writeFile(targetPath, "private path substitution", { mode: 0o400 });
    }
    return originalLstat(target, ...arguments_);
  };
  let result;
  try {
    result = await consumeNativeArtifactConsumptionPrototype(capability);
  } finally {
    fs.lstat = originalLstat;
  }
  assert.equal(swapped, true);
  assert.equal(result.disposition, "failed");
  assert.deepEqual(result.failure_boundaries, ["consumption"]);
  assert.equal(result.boundaries.consumption, "failed");
  assert.equal(result.boundaries.cleanup, "passed");
  assert.equal(result.claims.fixed_probe_completed, false);
});

test("replay and close during active consumption fail without interrupting settlement", async () => {
  const root = await verifiedRootFor();
  const capability = await acquireNativeArtifactConsumptionPrototype(binding("windows:nsis"), {
    verified_root: root,
  });
  const consuming = consumeNativeArtifactConsumptionPrototype(capability);
  await assert.rejects(
    () => consumeNativeArtifactConsumptionPrototype(capability),
    /cannot consume from phase consuming/u,
  );
  await assert.rejects(
    () => closeNativeArtifactConsumptionPrototype(capability),
    /cannot close while consumption is active/u,
  );
  const result = await consuming;
  assert.equal(result.disposition, "prototype_consumed");
  await assert.rejects(
    () => consumeNativeArtifactConsumptionPrototype(capability),
    /live process-local capability/u,
  );
});

test("closing before consumption invalidates the capability and removes the private copy", async () => {
  const root = await verifiedRootFor();
  const before = new Set(
    (await fs.readdir(os.tmpdir())).filter((name) => name.startsWith(PRIVATE_ROOT_PREFIX)),
  );
  const capability = await acquireNativeArtifactConsumptionPrototype(
    binding("macos:macos_updater"),
    {
      verified_root: root,
    },
  );
  await closeNativeArtifactConsumptionPrototype(capability);
  await assert.rejects(
    () => consumeNativeArtifactConsumptionPrototype(capability),
    /live process-local capability/u,
  );
  assert.deepEqual(await newPrivateRoots(before), []);
});

test("timeout remains distinct after the fixed process settles and cleanup succeeds", async () => {
  const root = await verifiedRootFor();
  const timeoutBinding = binding("linux:deb", {
    timeouts: { step_timeout_ms: 1, termination_timeout_ms: 1_000 },
  });
  const capability = await acquireNativeArtifactConsumptionPrototype(timeoutBinding, {
    verified_root: root,
  });
  const result = await consumeNativeArtifactConsumptionPrototype(capability);
  assert.equal(result.disposition, "failed");
  assert.deepEqual(result.failure_boundaries, ["timeout"]);
  assert.equal(result.boundaries.consumption, "timeout");
  assert.equal(result.boundaries.timeout, "triggered");
  assert.equal(result.boundaries.settlement, "passed");
  assert.equal(result.boundaries.cleanup, "passed");
  assert.equal(result.boundaries.residue, "passed");
  assert.equal(result.claims.package_bytes_executed, false);
});

test("cleanup failure is preserved separately and still emits no proof", async () => {
  const root = await verifiedRootFor();
  const before = new Set(
    (await fs.readdir(os.tmpdir())).filter((name) => name.startsWith(PRIVATE_ROOT_PREFIX)),
  );
  const capability = await acquireNativeArtifactConsumptionPrototype(binding("macos:dmg"), {
    verified_root: root,
  });
  const originalRm = fs.rm;
  fs.rm = async (target, options) => {
    if (path.basename(String(target)).startsWith(PRIVATE_ROOT_PREFIX)) {
      throw new Error("simulated prototype cleanup failure");
    }
    return originalRm(target, options);
  };
  let result;
  try {
    result = await consumeNativeArtifactConsumptionPrototype(capability);
  } finally {
    fs.rm = originalRm;
  }
  assert.equal(result.disposition, "failed");
  assert.deepEqual(result.failure_boundaries, ["cleanup"]);
  assert.equal(result.boundaries.consumption, "passed");
  assert.equal(result.boundaries.settlement, "passed");
  assert.equal(result.boundaries.cleanup, "failed");
  assert.equal(result.boundaries.residue, "passed");
  assert.equal(result.native_execution_receipt, null);
  assert.equal(result.evidence_packet, null);
  assert.equal((await newPrivateRoots(before)).length, 1);
  await closeNativeArtifactConsumptionPrototype(capability);
  assert.deepEqual(await newPrivateRoots(before), []);
});

test("acquisition preserves its byte failure together with cleanup failure", async () => {
  const root = await verifiedRootFor(Buffer.from("wrong bytes"));
  const before = new Set(
    (await fs.readdir(os.tmpdir())).filter((name) => name.startsWith(PRIVATE_ROOT_PREFIX)),
  );
  const originalRm = fs.rm;
  fs.rm = async (target, options) => {
    if (path.basename(String(target)).startsWith(PRIVATE_ROOT_PREFIX)) {
      throw new Error("simulated acquisition cleanup failure");
    }
    return originalRm(target, options);
  };
  let observed;
  try {
    await acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
      verified_root: root,
    });
  } catch (error) {
    observed = error;
  } finally {
    fs.rm = originalRm;
  }
  assert.ok(observed instanceof AggregateError);
  assert.equal(observed.name, "NativeArtifactConsumptionPrototypeCleanupError");
  assert.equal(observed.boundary, "cleanup");
  assert.equal(observed.errors.length, 2);
  assert.match(observed.errors[0].message, /does not match the bound asset bytes/u);
  assert.match(observed.errors[1].message, /simulated acquisition cleanup failure/u);
  const leftovers = await newPrivateRoots(before);
  assert.equal(leftovers.length, 1);
  await Promise.all(
    leftovers.map((leftover) => originalRm(leftover, { recursive: true, force: true })),
  );
});

test("linked roots, linked assets, mismatched bytes, and open-ended binding fields fail acquisition", async (t) => {
  const outside = await verifiedRootFor();
  const linkedRoot = path.join(scratchRoot, "linked-root");
  try {
    await fs.symlink(outside, linkedRoot, process.platform === "win32" ? "junction" : "dir");
    await assert.rejects(
      () =>
        acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
          verified_root: linkedRoot,
        }),
      /must be a non-link directory/u,
    );
  } catch (error) {
    if (error.code === "EPERM") t.diagnostic("directory links are unavailable on this host");
    else throw error;
  }

  const linkedAssetRoot = await fs.mkdtemp(path.join(scratchRoot, "linked-asset-"));
  try {
    await fs.symlink(
      path.join(outside, "selected-artifact.bin"),
      path.join(linkedAssetRoot, "selected-artifact.bin"),
      "file",
    );
    await assert.rejects(
      () =>
        acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
          verified_root: linkedAssetRoot,
        }),
      /regular non-link file|symbolic link|too many levels/iu,
    );
  } catch (error) {
    if (error.code === "EPERM") t.diagnostic("file links are unavailable on this host");
    else throw error;
  }

  const wrongBytesRoot = await verifiedRootFor(Buffer.from("wrong bytes"));
  await assert.rejects(
    () =>
      acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
        verified_root: wrongBytesRoot,
      }),
    /does not match the bound asset bytes/u,
  );

  const root = await verifiedRootFor();
  await assert.rejects(
    () =>
      acquireNativeArtifactConsumptionPrototype(
        { ...binding("linux:deb"), command: "dpkg --install" },
        { verified_root: root },
      ),
    /prototype.binding.command: is not allowed/u,
  );
  await assert.rejects(
    () =>
      acquireNativeArtifactConsumptionPrototype(binding("linux:deb"), {
        verified_root: root,
        callback: () => {},
      }),
    /prototype.options.callback: is not allowed/u,
  );
  await assert.rejects(
    () =>
      acquireNativeArtifactConsumptionPrototype(
        {
          ...binding("linux:deb"),
          asset: { ...binding("linux:deb").asset, name: "..\\selected-artifact.bin" },
        },
        { verified_root: root },
      ),
    /must be one direct filename/u,
  );
});

test("hosted release-contract jobs run the authority prototype on every validation host", async () => {
  const validationWorkflow = await fs.readFile(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  const releaseWorkflow = await fs.readFile(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  assert.equal(
    validationWorkflow.match(
      /scripts\/native-artifact-consumption-authority\.prototype\.test\.mjs/gu,
    )?.length,
    3,
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/native-artifact-consumption-authority\.prototype\.test\.mjs/gu)
      ?.length,
    1,
  );
});
