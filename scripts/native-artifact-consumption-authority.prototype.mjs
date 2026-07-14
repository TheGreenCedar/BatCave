import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { constants } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";

export const NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION = 1;

export const NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_PROFILES = Object.freeze([
  "windows:nsis",
  "linux:deb",
  "linux:appimage",
  "macos:dmg",
  "macos:macos_updater",
]);

export class NativeArtifactConsumptionPrototypeError extends Error {
  constructor(boundary, field, message) {
    super(`${field}: ${message}`);
    this.name = "NativeArtifactConsumptionPrototypeError";
    this.boundary = boundary;
  }
}

const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const BINDING_ID = /^[a-z0-9][a-z0-9-]{2,79}$/u;
const MAX_PROBE_OUTPUT_BYTES = 2 * 1024;
const FIXED_PROBE_DELAY_MS = 40;
const POLL_INTERVAL_MS = 5;
const capabilities = new WeakMap();
const results = new WeakSet();

const FIXED_CONSUMER_PROGRAM = [
  'const crypto=require("node:crypto");',
  'const fs=require("node:fs");',
  "const selectedPath=process.argv[1];",
  "const delayMs=Number(process.argv[2]);",
  "setTimeout(()=>{",
  "try{",
  "const bytes=fs.readFileSync(selectedPath);",
  'const sha256=`sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;',
  "process.stdout.write(JSON.stringify({size_bytes:bytes.length,sha256}));",
  "}catch{process.stderr.write('fixed consumption probe failed');process.exitCode=4;}",
  "},delayMs);",
].join("");

function fail(boundary, field, message) {
  throw new NativeArtifactConsumptionPrototypeError(boundary, field, message);
}

function exactKeys(value, field, expected) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("acquisition", field, "must be an object");
  }
  const actual = Object.keys(value);
  const missing = expected.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !expected.includes(key));
  if (missing.length) fail("acquisition", `${field}.${missing[0]}`, "is required");
  if (extra.length) fail("acquisition", `${field}.${extra[0]}`, "is not allowed");
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

function sameFileIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function requireRegularFile(metadata, field, boundary = "acquisition") {
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(boundary, field, "must be a regular non-link file");
  }
}

function positiveInteger(value, field, maximum) {
  if (!Number.isSafeInteger(value) || value < 1 || value > maximum) {
    fail("acquisition", field, `must be an integer from 1 through ${maximum}`);
  }
  return value;
}

function validateBinding(binding) {
  exactKeys(binding, "prototype.binding", [
    "schema_version",
    "binding_id",
    "profile_id",
    "asset",
    "timeouts",
  ]);
  if (binding.schema_version !== NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION) {
    fail(
      "acquisition",
      "prototype.binding.schema_version",
      `must equal ${NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION}`,
    );
  }
  if (typeof binding.binding_id !== "string" || !BINDING_ID.test(binding.binding_id)) {
    fail("acquisition", "prototype.binding.binding_id", "must be a stable lowercase identifier");
  }
  if (!NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_PROFILES.includes(binding.profile_id)) {
    fail("acquisition", "prototype.binding.profile_id", "is not a closed platform profile");
  }
  exactKeys(binding.asset, "prototype.binding.asset", ["name", "size_bytes", "sha256"]);
  if (
    typeof binding.asset.name !== "string" ||
    binding.asset.name.length === 0 ||
    binding.asset.name === "." ||
    binding.asset.name === ".." ||
    binding.asset.name !== path.posix.basename(binding.asset.name) ||
    binding.asset.name !== path.win32.basename(binding.asset.name)
  ) {
    fail("acquisition", "prototype.binding.asset.name", "must be one direct filename");
  }
  positiveInteger(binding.asset.size_bytes, "prototype.binding.asset.size_bytes", 2 ** 31 - 1);
  if (typeof binding.asset.sha256 !== "string" || !SHA256_DIGEST.test(binding.asset.sha256)) {
    fail("acquisition", "prototype.binding.asset.sha256", "must be a lowercase SHA-256 digest");
  }
  exactKeys(binding.timeouts, "prototype.binding.timeouts", [
    "step_timeout_ms",
    "termination_timeout_ms",
  ]);
  positiveInteger(
    binding.timeouts.step_timeout_ms,
    "prototype.binding.timeouts.step_timeout_ms",
    60_000,
  );
  positiveInteger(
    binding.timeouts.termination_timeout_ms,
    "prototype.binding.timeouts.termination_timeout_ms",
    30_000,
  );
  return deepFreeze(structuredClone(binding));
}

async function validateRoot(verifiedRoot) {
  if (typeof verifiedRoot !== "string" || !path.isAbsolute(verifiedRoot)) {
    fail("acquisition", "prototype.verified_root", "must be an absolute directory path");
  }
  const inspected = await fs.lstat(verifiedRoot, { bigint: true });
  if (!inspected.isDirectory() || inspected.isSymbolicLink()) {
    fail("acquisition", "prototype.verified_root", "must be a non-link directory");
  }
  const real = await fs.realpath(verifiedRoot);
  const resolved = await fs.lstat(real, { bigint: true });
  if (
    !resolved.isDirectory() ||
    resolved.isSymbolicLink() ||
    !sameFileIdentity(inspected, resolved)
  ) {
    fail("acquisition", "prototype.verified_root", "changed identity while being resolved");
  }
  return { input: verifiedRoot, real, identity: resolved };
}

async function assertRootStable(root) {
  const inspected = await fs.lstat(root.input, { bigint: true });
  const real = await fs.realpath(root.input);
  const resolved = await fs.lstat(root.real, { bigint: true });
  if (
    !inspected.isDirectory() ||
    inspected.isSymbolicLink() ||
    real !== root.real ||
    !resolved.isDirectory() ||
    resolved.isSymbolicLink() ||
    !sameFileIdentity(inspected, root.identity) ||
    !sameFileIdentity(resolved, root.identity)
  ) {
    fail("acquisition", "prototype.verified_root", "changed during capability acquisition");
  }
}

async function hashHandle(handle, expectedSize, boundary, field) {
  const hash = crypto.createHash("sha256");
  const buffer = Buffer.allocUnsafe(64 * 1024);
  let position = 0;
  while (position < expectedSize) {
    const requested = Math.min(buffer.length, expectedSize - position);
    const { bytesRead } = await handle.read(buffer, 0, requested, position);
    if (bytesRead === 0) break;
    hash.update(buffer.subarray(0, bytesRead));
    position += bytesRead;
  }
  const extra = Buffer.allocUnsafe(1);
  const { bytesRead: extraBytes } = await handle.read(extra, 0, 1, position);
  if (position !== expectedSize || extraBytes !== 0) {
    fail(boundary, field, "size changed while the authority held the file");
  }
  return `sha256:${hash.digest("hex")}`;
}

async function copyVerifiedBytes(source, destination, asset) {
  const hash = crypto.createHash("sha256");
  const buffer = Buffer.allocUnsafe(64 * 1024);
  let position = 0;
  while (position < asset.size_bytes) {
    const requested = Math.min(buffer.length, asset.size_bytes - position);
    const { bytesRead } = await source.read(buffer, 0, requested, position);
    if (bytesRead === 0) break;
    hash.update(buffer.subarray(0, bytesRead));
    let written = 0;
    while (written < bytesRead) {
      const { bytesWritten } = await destination.write(
        buffer,
        written,
        bytesRead - written,
        position + written,
      );
      if (bytesWritten === 0) {
        fail("acquisition", "prototype.private_copy", "write made no progress");
      }
      written += bytesWritten;
    }
    position += bytesRead;
  }
  const extra = Buffer.allocUnsafe(1);
  const { bytesRead: extraBytes } = await source.read(extra, 0, 1, position);
  const digest = `sha256:${hash.digest("hex")}`;
  if (position !== asset.size_bytes || extraBytes !== 0 || digest !== asset.sha256) {
    fail("acquisition", "prototype.source", "does not match the bound asset bytes");
  }
  await destination.sync();
  const ownedDigest = await hashHandle(
    destination,
    asset.size_bytes,
    "acquisition",
    "prototype.private_copy",
  );
  if (ownedDigest !== asset.sha256) {
    fail("acquisition", "prototype.private_copy", "does not match the bound asset digest");
  }
}

async function attempt(operation, errors) {
  try {
    await operation();
  } catch (error) {
    errors.push(error);
  }
}

async function cleanupAcquisition(source, owned, ownedRoot) {
  const errors = [];
  if (source) await attempt(() => source.close(), errors);
  if (owned) await attempt(() => owned.close(), errors);
  if (ownedRoot) {
    await attempt(() => fs.rm(ownedRoot, { recursive: true, force: true }), errors);
  }
  return errors;
}

function stateFor(capability) {
  const state = capabilities.get(capability);
  if (!state) {
    fail(
      "replay",
      "prototype.capability",
      "must be the live process-local capability returned by this prototype module",
    );
  }
  return state;
}

async function assertPrivatePathStable(state, boundary = "consumption") {
  const metadata = await fs.lstat(state.ownedPath, { bigint: true });
  requireRegularFile(metadata, "prototype.private_copy", boundary);
  const real = await fs.realpath(state.ownedPath);
  if (
    real !== state.ownedPath ||
    path.dirname(real) !== state.ownedRoot ||
    !sameFileIdentity(metadata, state.ownedIdentity)
  ) {
    fail(boundary, "prototype.private_copy", "identity changed while authority was live");
  }
  const opened = await state.handle.stat({ bigint: true });
  requireRegularFile(opened, "prototype.private_handle", boundary);
  if (!sameFileIdentity(opened, state.ownedIdentity)) {
    fail(boundary, "prototype.private_handle", "no longer identifies the private copy");
  }
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function processGroupAlive(child) {
  if (!child.pid || child.pid <= 0) return false;
  try {
    if (process.platform === "win32") process.kill(child.pid, 0);
    else process.kill(-child.pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}

function signalOwnedProcess(child, signal) {
  try {
    if (process.platform === "win32") return child.kill(signal);
    process.kill(-child.pid, signal);
    return true;
  } catch (error) {
    return error?.code === "ESRCH";
  }
}

async function waitForSettlement(child, closed, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!processGroupAlive(child)) {
      await closed;
      return true;
    }
    await delay(POLL_INTERVAL_MS);
  }
  return !processGroupAlive(child);
}

async function terminateAndSettle(child, closed, timeoutMs) {
  const softTimeout = Math.max(1, Math.floor(timeoutMs / 2));
  signalOwnedProcess(child, "SIGTERM");
  if (await waitForSettlement(child, closed, softTimeout)) return true;
  signalOwnedProcess(child, "SIGKILL");
  return waitForSettlement(child, closed, Math.max(1, timeoutMs - softTimeout));
}

async function runFixedConsumer(state) {
  const child = spawn(
    process.execPath,
    ["--eval", FIXED_CONSUMER_PROGRAM, state.ownedPath, String(FIXED_PROBE_DELAY_MS)],
    {
      cwd: state.ownedRoot,
      detached: process.platform !== "win32",
      env: {
        HOME: state.ownedRoot,
        LANG: "C",
        LC_ALL: "C",
        NO_COLOR: "1",
        TMPDIR: state.ownedRoot,
      },
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  let stdout = Buffer.alloc(0);
  let stderrBytes = 0;
  let outputOverflow = false;
  const collectStdout = (chunk) => {
    if (stdout.length + chunk.length > MAX_PROBE_OUTPUT_BYTES) outputOverflow = true;
    else stdout = Buffer.concat([stdout, chunk]);
  };
  const collectStderr = (chunk) => {
    stderrBytes += chunk.length;
    if (stderrBytes > MAX_PROBE_OUTPUT_BYTES) outputOverflow = true;
  };
  child.stdout.on("data", collectStdout);
  child.stderr.on("data", collectStderr);
  const closed = new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    child.once("error", () => finish({ code: null, signal: null, spawn_failed: true }));
    child.once("close", (code, signal) => finish({ code, signal, spawn_failed: false }));
  });
  const trigger = await Promise.race([
    closed.then(() => "closed"),
    delay(state.binding.timeouts.step_timeout_ms).then(() => "timeout"),
  ]);
  if (trigger === "timeout" || outputOverflow) {
    const settled = await terminateAndSettle(
      child,
      closed,
      state.binding.timeouts.termination_timeout_ms,
    );
    return {
      trigger: outputOverflow ? "output_limit" : "timeout",
      settled,
      observation: null,
    };
  }
  const exit = await closed;
  const settled = !processGroupAlive(child);
  if (exit.spawn_failed || exit.code !== 0 || exit.signal !== null || stderrBytes !== 0) {
    return { trigger: "consumer_failed", settled, observation: null };
  }
  try {
    const observation = JSON.parse(stdout.toString("utf8"));
    exactKeys(observation, "prototype.consumer_observation", ["size_bytes", "sha256"]);
    return { trigger: "consumed", settled, observation };
  } catch {
    return { trigger: "consumer_failed", settled, observation: null };
  }
}

async function cleanupConsumedState(state, settled) {
  if (!settled) return "retained_unsettled";
  const errors = [];
  if (!state.handleClosed) {
    try {
      await state.handle.close();
      state.handleClosed = true;
    } catch (error) {
      errors.push(error);
    }
  }
  await attempt(() => fs.rm(state.ownedRoot, { recursive: true, force: true }), errors);
  return errors.length ? "failed" : "passed";
}

function finalResult(state, boundaries, failures, observation) {
  const result = deepFreeze({
    schema_version: NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION,
    proof_scope: "non_installing_native_consumption_authority_prototype",
    binding_id: state.binding.binding_id,
    profile_id: state.binding.profile_id,
    disposition: failures.length === 0 ? "prototype_consumed" : "failed",
    failure_boundaries: [...failures],
    boundaries,
    observed_bytes: observation
      ? {
          size_bytes: observation.size_bytes,
          sha256: observation.sha256,
        }
      : null,
    claims: {
      fixed_probe_completed: boundaries.consumption === "passed",
      package_bytes_executed: false,
      package_installed_or_staged: false,
      native_proven: false,
      release_evidence_emitted: false,
      private_path_returned: false,
      raw_handle_returned: false,
    },
    native_execution_receipt: null,
    evidence_packet: null,
  });
  results.add(result);
  return validateNativeArtifactConsumptionPrototypeResult(result);
}

export async function acquireNativeArtifactConsumptionPrototype(binding, options) {
  const validatedBinding = validateBinding(binding);
  exactKeys(options, "prototype.options", ["verified_root"]);
  const root = await validateRoot(options.verified_root);
  const candidate = path.join(root.real, validatedBinding.asset.name);
  if (path.dirname(candidate) !== root.real) {
    fail("acquisition", "prototype.source", "must remain directly inside verified_root");
  }
  let source;
  let owned;
  let ownedRoot;
  try {
    await assertRootStable(root);
    const before = await fs.lstat(candidate, { bigint: true });
    requireRegularFile(before, "prototype.source");
    const real = await fs.realpath(candidate);
    if (real !== candidate || path.dirname(real) !== root.real) {
      fail("acquisition", "prototype.source", "must resolve directly inside verified_root");
    }
    source = await fs.open(candidate, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    const opened = await source.stat({ bigint: true });
    requireRegularFile(opened, "prototype.source_handle");
    if (!sameFileIdentity(before, opened)) {
      fail("acquisition", "prototype.source", "changed between inspection and open");
    }
    ownedRoot = await fs.mkdtemp(path.join(os.tmpdir(), "batcave-consumption-prototype-"));
    await fs.chmod(ownedRoot, 0o700);
    ownedRoot = await fs.realpath(ownedRoot);
    const ownedPath = path.join(ownedRoot, crypto.randomBytes(16).toString("hex"));
    owned = await fs.open(
      ownedPath,
      constants.O_CREAT | constants.O_EXCL | constants.O_RDWR,
      0o600,
    );
    await copyVerifiedBytes(source, owned, validatedBinding.asset);
    await assertRootStable(root);
    const after = await fs.lstat(candidate, { bigint: true });
    if (!sameFileIdentity(after, opened)) {
      fail("acquisition", "prototype.source", "was replaced during private-copy acquisition");
    }
    await source.close();
    source = undefined;
    await fs.chmod(ownedPath, 0o400);
    const ownedIdentity = await owned.stat({ bigint: true });
    const capability = deepFreeze({
      schema_version: NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION,
      proof_scope: "opaque_consumption_capability_prototype",
      binding_id: validatedBinding.binding_id,
      profile_id: validatedBinding.profile_id,
      asset: structuredClone(validatedBinding.asset),
    });
    capabilities.set(capability, {
      binding: validatedBinding,
      phase: "acquired",
      handle: owned,
      ownedRoot,
      ownedPath,
      ownedIdentity,
      handleClosed: false,
    });
    return capability;
  } catch (error) {
    const cleanupErrors = await cleanupAcquisition(source, owned, ownedRoot);
    if (cleanupErrors.length) {
      const aggregate = new AggregateError(
        [error, ...cleanupErrors],
        "prototype acquisition and cleanup both failed",
      );
      aggregate.name = "NativeArtifactConsumptionPrototypeCleanupError";
      aggregate.boundary = "cleanup";
      throw aggregate;
    }
    throw error;
  }
}

export async function consumeNativeArtifactConsumptionPrototype(
  capability,
  ...unexpectedArguments
) {
  if (unexpectedArguments.length) {
    fail(
      "authority",
      "prototype.consumption",
      "does not accept a command, callback, path, handle, descriptor, status, or evidence",
    );
  }
  const state = stateFor(capability);
  if (state.phase !== "acquired") {
    fail("replay", "prototype.capability", `cannot consume from phase ${state.phase}`);
  }
  state.phase = "consuming";
  let trigger = "consumer_failed";
  let settled = true;
  let observation = null;
  const failures = [];
  const boundaries = {
    acquisition: "passed",
    consumption: "failed",
    timeout: "not_triggered",
    settlement: "not_started",
    cleanup: "not_started",
    residue: "not_inspected",
  };
  try {
    await assertPrivatePathStable(state);
    const digestBefore = await hashHandle(
      state.handle,
      state.binding.asset.size_bytes,
      "consumption",
      "prototype.private_copy",
    );
    if (digestBefore !== state.binding.asset.sha256) {
      fail("consumption", "prototype.private_copy", "changed before fixed consumption");
    }
    const consumed = await runFixedConsumer(state);
    trigger = consumed.trigger;
    settled = consumed.settled;
    observation = consumed.observation;
    if (trigger === "timeout") {
      boundaries.consumption = "timeout";
      boundaries.timeout = "triggered";
      failures.push("timeout");
    } else if (trigger !== "consumed") {
      boundaries.consumption = "failed";
      failures.push("consumption");
    } else if (
      !observation ||
      observation.size_bytes !== state.binding.asset.size_bytes ||
      observation.sha256 !== state.binding.asset.sha256
    ) {
      boundaries.consumption = "failed";
      failures.push("consumption");
    } else {
      boundaries.consumption = "passed";
    }
    boundaries.settlement = settled ? "passed" : "unresolved";
    boundaries.residue = settled ? "passed" : "unresolved";
    if (!settled) failures.push("settlement", "residue");
    if (settled) {
      await assertPrivatePathStable(state, "residue");
      const digestAfter = await hashHandle(
        state.handle,
        state.binding.asset.size_bytes,
        "residue",
        "prototype.private_copy",
      );
      if (digestAfter !== state.binding.asset.sha256) {
        boundaries.residue = "failed";
        failures.push("residue");
      }
    }
  } catch (error) {
    const boundary =
      error instanceof NativeArtifactConsumptionPrototypeError ? error.boundary : "consumption";
    if (!failures.includes(boundary)) failures.push(boundary);
    if (boundary === "residue") boundaries.residue = "failed";
    else boundaries.consumption = "failed";
  }
  const cleanup = await cleanupConsumedState(state, settled);
  boundaries.cleanup = cleanup;
  if (cleanup === "failed") failures.push("cleanup");
  if (cleanup === "retained_unsettled" && !failures.includes("settlement")) {
    failures.push("settlement");
  }
  state.phase =
    cleanup === "retained_unsettled"
      ? "retained_unsettled"
      : cleanup === "failed"
        ? "retained_cleanup_failed"
        : "closed";
  if (state.phase === "closed") capabilities.delete(capability);
  return finalResult(state, boundaries, [...new Set(failures)], observation);
}

export async function closeNativeArtifactConsumptionPrototype(capability) {
  const state = stateFor(capability);
  if (state.phase === "consuming") {
    fail("early_close", "prototype.capability", "cannot close while consumption is active");
  }
  if (state.phase === "retained_unsettled") {
    fail(
      "settlement",
      "prototype.capability",
      "cannot clean up while process settlement remains unresolved",
    );
  }
  if (state.phase !== "acquired" && state.phase !== "retained_cleanup_failed") {
    fail("replay", "prototype.capability", `cannot close from phase ${state.phase}`);
  }
  state.phase = "closing";
  const cleanup = await cleanupConsumedState(state, true);
  if (cleanup !== "passed") {
    state.phase = "retained_cleanup_failed";
    fail("cleanup", "prototype.capability", "failed to close the unused capability");
  }
  state.phase = "closed";
  capabilities.delete(capability);
}

export function validateNativeArtifactConsumptionPrototypeResult(result) {
  if (!result || typeof result !== "object" || !results.has(result)) {
    fail(
      "evidence",
      "prototype.result",
      "must be the process-local result returned by this non-proof prototype",
    );
  }
  if (
    result.schema_version !== NATIVE_ARTIFACT_CONSUMPTION_PROTOTYPE_SCHEMA_VERSION ||
    result.proof_scope !== "non_installing_native_consumption_authority_prototype" ||
    result.claims.package_bytes_executed !== false ||
    result.claims.package_installed_or_staged !== false ||
    result.claims.fixed_probe_completed !== (result.boundaries.consumption === "passed") ||
    result.claims.native_proven !== false ||
    result.claims.release_evidence_emitted !== false ||
    result.claims.private_path_returned !== false ||
    result.claims.raw_handle_returned !== false ||
    result.native_execution_receipt !== null ||
    result.evidence_packet !== null
  ) {
    fail("evidence", "prototype.result", "crosses the non-proof prototype boundary");
  }
  const expectedDisposition = result.failure_boundaries.length ? "failed" : "prototype_consumed";
  if (result.disposition !== expectedDisposition) {
    fail("evidence", "prototype.result.disposition", "does not match its failure boundaries");
  }
  return result;
}
