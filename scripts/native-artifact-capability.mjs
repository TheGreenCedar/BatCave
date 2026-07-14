import crypto from "node:crypto";
import { constants } from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { validateInstallSmokePlan } from "./install-smoke-contract.mjs";

export const NATIVE_ARTIFACT_CAPABILITY_SCHEMA_VERSION = 1;

export class NativeArtifactCapabilityCleanupError extends AggregateError {
  constructor(errors) {
    super(errors, "native artifact capability cleanup failed");
    this.name = "NativeArtifactCapabilityCleanupError";
  }
}

const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const capabilities = new WeakMap();
const verificationReceipts = new WeakSet();

function fail(field, message) {
  throw new Error(`${field}: ${message}`);
}

function exactKeys(value, field, keys) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(field, "must be an object");
  }
  const actual = Object.keys(value);
  const missing = keys.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !keys.includes(key));
  if (missing.length) fail(`${field}.${missing[0]}`, "is required");
  if (extra.length) fail(`${field}.${extra[0]}`, "is not allowed");
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

function assetIdentity(plan) {
  return {
    name: plan.asset.name,
    size_bytes: plan.asset.size_bytes,
    sha256: plan.asset.sha256,
    public_url: plan.asset.public_url,
  };
}

function sameFileIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function requireRegularFile(stats, field) {
  if (!stats.isFile() || stats.isSymbolicLink()) {
    fail(field, "must be a regular non-link file");
  }
}

async function attemptCleanup(operation, errors) {
  try {
    await operation();
  } catch (error) {
    errors.push(error);
  }
}

async function validateRoot(root) {
  if (typeof root !== "string" || !path.isAbsolute(root)) {
    fail("native_artifact.verified_root", "must be an absolute directory path");
  }
  const metadata = await fs.lstat(root, { bigint: true });
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    fail("native_artifact.verified_root", "must be a non-link directory");
  }
  const real = await fs.realpath(root);
  const resolvedMetadata = await fs.lstat(real, { bigint: true });
  if (!resolvedMetadata.isDirectory() || resolvedMetadata.isSymbolicLink()) {
    fail("native_artifact.verified_root", "must resolve to a non-link directory");
  }
  if (!sameFileIdentity(metadata, resolvedMetadata)) {
    fail("native_artifact.verified_root", "identity changed while being resolved");
  }
  return { input: root, real, identity: resolvedMetadata };
}

async function assertRootStable(root) {
  const inputMetadata = await fs.lstat(root.input, { bigint: true });
  if (!inputMetadata.isDirectory() || inputMetadata.isSymbolicLink()) {
    fail("native_artifact.verified_root", "changed into a linked or non-directory path");
  }
  const currentReal = await fs.realpath(root.input);
  const realMetadata = await fs.lstat(root.real, { bigint: true });
  if (
    currentReal !== root.real ||
    !realMetadata.isDirectory() ||
    realMetadata.isSymbolicLink() ||
    !sameFileIdentity(inputMetadata, root.identity) ||
    !sameFileIdentity(realMetadata, root.identity)
  ) {
    fail("native_artifact.verified_root", "identity or containment changed during acquisition");
  }
}

async function validateCandidate(root, candidate) {
  await assertRootStable(root);
  const before = await fs.lstat(candidate, { bigint: true });
  requireRegularFile(before, "native_artifact.source");
  const real = await fs.realpath(candidate);
  if (path.dirname(real) !== root.real) {
    fail("native_artifact.source", "must remain directly inside the verified root");
  }
  return { before, real };
}

async function hashHandle(handle, expectedSize) {
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
    fail("native_artifact.bytes", "size changed while the capability owned the file handle");
  }
  return `sha256:${hash.digest("hex")}`;
}

async function copyVerifiedBytes(source, destination, expected) {
  const hash = crypto.createHash("sha256");
  const buffer = Buffer.allocUnsafe(64 * 1024);
  let position = 0;
  while (position < expected.size_bytes) {
    const requested = Math.min(buffer.length, expected.size_bytes - position);
    const { bytesRead } = await source.read(buffer, 0, requested, position);
    if (bytesRead === 0) break;
    hash.update(buffer.subarray(0, bytesRead));
    let written = 0;
    while (written < bytesRead) {
      const result = await destination.write(
        buffer,
        written,
        bytesRead - written,
        position + written,
      );
      if (result.bytesWritten === 0) fail("native_artifact.owned_copy", "write made no progress");
      written += result.bytesWritten;
    }
    position += bytesRead;
  }
  const extra = Buffer.allocUnsafe(1);
  const { bytesRead: extraBytes } = await source.read(extra, 0, 1, position);
  const digest = `sha256:${hash.digest("hex")}`;
  if (position !== expected.size_bytes || extraBytes !== 0) {
    fail("native_artifact.source", "size does not match the verified selected asset");
  }
  if (!SHA256_DIGEST.test(digest) || digest !== expected.sha256) {
    fail("native_artifact.source", "digest does not match the verified selected asset");
  }
  await destination.sync();
  const ownedDigest = await hashHandle(destination, expected.size_bytes);
  if (ownedDigest !== expected.sha256) {
    fail("native_artifact.owned_copy", "does not contain the verified selected bytes");
  }
}

async function assertSourceStable(candidate, root, expectedReal, openedIdentity) {
  await assertRootStable(root);
  const after = await fs.lstat(candidate, { bigint: true });
  requireRegularFile(after, "native_artifact.source");
  if (!sameFileIdentity(after, openedIdentity)) {
    fail("native_artifact.source", "was replaced while the capability acquired it");
  }
  const real = await fs.realpath(candidate);
  if (real !== expectedReal || path.dirname(real) !== root.real) {
    fail("native_artifact.source", "escaped the verified root while being acquired");
  }
}

function capabilityState(capability) {
  const state = capabilities.get(capability);
  if (!state) {
    fail(
      "native_artifact.capability",
      "must be the process-local capability returned by acquireNativeArtifactCapability",
    );
  }
  if (state.closed) fail("native_artifact.capability", "is already closed");
  return state;
}

export async function acquireNativeArtifactCapability(plan, options) {
  validateInstallSmokePlan(plan);
  if (plan.execution_kind !== "plan") {
    fail(
      "native_artifact.plan",
      "must be a contract-only plan; fixtures cannot acquire native bytes",
    );
  }
  exactKeys(options, "native_artifact.options", ["verified_root"]);
  const root = await validateRoot(options.verified_root);
  const candidate = path.join(root.real, plan.asset.name);
  if (path.dirname(candidate) !== root.real) {
    fail("native_artifact.source", "asset name escapes the verified root");
  }
  const { before, real } = await validateCandidate(root, candidate);
  const flags = constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0);
  let source;
  let owned;
  let ownedRoot;
  try {
    source = await fs.open(candidate, flags);
    const opened = await source.stat({ bigint: true });
    requireRegularFile(opened, "native_artifact.open_handle");
    if (!sameFileIdentity(before, opened)) {
      fail("native_artifact.source", "was replaced between inspection and open");
    }

    ownedRoot = await fs.mkdtemp(path.join(os.tmpdir(), "batcave-native-artifact-"));
    await fs.chmod(ownedRoot, 0o700);
    const ownedPath = path.join(ownedRoot, "selected-artifact");
    owned = await fs.open(
      ownedPath,
      constants.O_CREAT | constants.O_EXCL | constants.O_RDWR,
      0o600,
    );
    await copyVerifiedBytes(source, owned, plan.asset);
    await assertSourceStable(candidate, root, real, opened);
    await source.close();
    source = undefined;
    await fs.chmod(ownedPath, 0o400);

    const capability = deepFreeze({
      schema_version: NATIVE_ARTIFACT_CAPABILITY_SCHEMA_VERSION,
      proof_scope: "capability_only",
      plan_id: plan.plan_id,
      asset: assetIdentity(plan),
    });
    capabilities.set(capability, {
      handle: owned,
      ownedRoot,
      ownedPath,
      expected: assetIdentity(plan),
      closed: false,
      verifying: false,
      verified: false,
    });
    return capability;
  } catch (error) {
    const cleanupErrors = [];
    if (source) await attemptCleanup(() => source.close(), cleanupErrors);
    if (owned) await attemptCleanup(() => owned.close(), cleanupErrors);
    if (ownedRoot) {
      await attemptCleanup(() => fs.rm(ownedRoot, { recursive: true, force: true }), cleanupErrors);
    }
    if (cleanupErrors.length) {
      throw new NativeArtifactCapabilityCleanupError([error, ...cleanupErrors]);
    }
    throw error;
  }
}

export async function verifyOwnedNativeArtifactCapability(capability, ...unexpectedArguments) {
  if (unexpectedArguments.length) {
    fail(
      "native_artifact.verification",
      "does not accept a caller-supplied reader, callback, adapter, or options",
    );
  }
  const state = capabilityState(capability);
  if (state.verifying) fail("native_artifact.capability", "is already being verified");
  if (state.verified) fail("native_artifact.capability", "has already been verified");
  state.verifying = true;
  state.verified = true;
  try {
    const digest = await hashHandle(state.handle, state.expected.size_bytes);
    if (digest !== state.expected.sha256) {
      fail("native_artifact.owned_copy", "changed before owned-byte verification completed");
    }
    const receipt = deepFreeze({
      schema_version: NATIVE_ARTIFACT_CAPABILITY_SCHEMA_VERSION,
      proof_scope: "artifact_owned_bytes_verified",
      plan_id: capability.plan_id,
      verification_sequence: 1,
      asset: structuredClone(state.expected),
    });
    verificationReceipts.add(receipt);
    return receipt;
  } finally {
    state.verifying = false;
  }
}

export function requireOwnedNativeArtifactVerificationReceipt(receipt, plan) {
  if (!receipt || typeof receipt !== "object" || !verificationReceipts.has(receipt)) {
    fail(
      "native_artifact.verification_receipt",
      "must come from process-local owned-byte verification",
    );
  }
  if (
    receipt.proof_scope !== "artifact_owned_bytes_verified" ||
    receipt.verification_sequence !== 1
  ) {
    fail("native_artifact.verification_receipt", "has an invalid proof scope or sequence");
  }
  if (
    receipt.plan_id !== plan.plan_id ||
    JSON.stringify(receipt.asset) !== JSON.stringify(assetIdentity(plan))
  ) {
    fail("native_artifact.verification_receipt", "must bind the exact plan and selected asset");
  }
  return receipt;
}

export async function closeNativeArtifactCapability(capability) {
  const state = capabilityState(capability);
  if (state.verifying) fail("native_artifact.capability", "cannot close during verification");
  state.closed = true;
  capabilities.delete(capability);
  const errors = [];
  await attemptCleanup(() => state.handle.close(), errors);
  await attemptCleanup(() => fs.rm(state.ownedRoot, { recursive: true, force: true }), errors);
  if (errors.length) {
    throw new NativeArtifactCapabilityCleanupError(errors);
  }
}
