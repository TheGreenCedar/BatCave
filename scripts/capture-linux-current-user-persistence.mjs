import { spawn, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import {
  validateCurrentUserPersistencePacket,
  validateCurrentUserPersistenceReceipt,
} from "./validate-current-user-persistence-evidence.mjs";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  requireVerifiedPublicReleaseDownloads,
  requireVerifiedPublicReleaseReceipt,
} from "./verify-public-release.mjs";

const PROOF_ENV = "BATCAVE_CURRENT_USER_PERSISTENCE_PROOF";
const DEB_PACKAGE_NAME = "bat-cave-monitor";
const SOURCE_SHA = /^[0-9a-f]{40}$/u;
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u;
const MAX_ARTIFACT_BYTES = 512 * 1024 * 1024;
const MAX_PROCESS_OUTPUT_BYTES = 64 * 1024;
const PROCESS_TIMEOUT_MS = 30_000;
const COMMAND_TIMEOUT_MS = 120_000;
const ROOT_UNIT_CLIENT_TIMEOUT_MS = 135_000;
const ROOT_UNIT_CONTROL_TIMEOUT_MS = 15_000;
const ROOT_UNIT_SETTLEMENT_TIMEOUT_MS = 5_000;
const GROUP_GRACE_MS = 100;
const TERMINATION_TIMEOUT_MS = 2_000;
const POLL_INTERVAL_MS = 20;
const COMPONENT_FILES = Object.freeze({
  "diagnostics.jsonl": "diagnostics",
  "settings.json": "settings",
  "warm-cache.json": "warm_cache",
});
const CORRUPT_SETTINGS = Buffer.from(
  '{"schema_version":1,"corrupt":"preserve-linux-candidate"',
  "utf8",
);
const FIXED_COMMAND_ENV = Object.freeze({
  LANG: "C",
  LC_ALL: "C",
  NO_COLOR: "1",
  PATH: "/usr/bin:/bin",
});
const verifiedPublicDebCaptureResults = new WeakSet();
const verifiedPublicDebCaptureStates = new WeakMap();
const verifiedRootUnitSettlementReceipts = new WeakSet();
const HOSTILE_ROOT_SETTLEMENT_PROGRAM =
  '/usr/bin/sleep 300 & normal=$!; /usr/bin/setsid /usr/bin/sleep 300 & escaped=$!; /usr/bin/printf \'{"schema_version":1,"pids":[%s,%s]}\\n\' "$normal" "$escaped"';

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  const values = new Map();
  const allowed = new Set(["--appimage", "--deb", "--output-dir", "--source-sha"]);
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || !value || value.startsWith("--")) {
      fail(`invalid or missing value for ${key ?? "argument"}`);
    }
    if (!allowed.has(key)) fail(`unknown argument: ${key}`);
    if (values.has(key)) fail(`duplicate argument: ${key}`);
    values.set(key, value);
  }
  for (const key of allowed) {
    if (!values.has(key)) fail(`${key} is required`);
  }
  const sourceSha = values.get("--source-sha");
  if (!SOURCE_SHA.test(sourceSha)) {
    fail("--source-sha must be an exact lowercase 40-character Git SHA-1");
  }
  return {
    appimage: path.resolve(values.get("--appimage")),
    deb: path.resolve(values.get("--deb")),
    outputDir: path.resolve(values.get("--output-dir")),
    sourceSha,
  };
}

function mode(metadata) {
  return (metadata.mode & 0o777).toString(8).padStart(4, "0");
}

function pathPresent(file) {
  try {
    fs.lstatSync(file);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function sha256(bytes) {
  return `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
}

function readRegularFileOnce(file, label) {
  const absolute = path.resolve(file);
  let pathMetadata;
  let realFile;
  try {
    pathMetadata = fs.lstatSync(absolute);
    realFile = fs.realpathSync(absolute);
  } catch (error) {
    fail(`${label} is unavailable: ${error.message}`);
  }
  if (!pathMetadata.isFile() || pathMetadata.isSymbolicLink() || realFile !== absolute) {
    fail(`${label} must be a regular file reached without links`);
  }
  if (pathMetadata.size <= 0 || pathMetadata.size > MAX_ARTIFACT_BYTES) {
    fail(`${label} size is outside the capture boundary`);
  }

  let descriptor;
  try {
    descriptor = fs.openSync(absolute, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
    const opened = fs.fstatSync(descriptor);
    if (
      !opened.isFile() ||
      opened.dev !== pathMetadata.dev ||
      opened.ino !== pathMetadata.ino ||
      opened.size !== pathMetadata.size
    ) {
      fail(`${label} changed identity while being opened`);
    }
    const bytes = fs.readFileSync(descriptor);
    const afterDescriptor = fs.fstatSync(descriptor);
    const afterPath = fs.lstatSync(absolute);
    if (
      afterDescriptor.dev !== opened.dev ||
      afterDescriptor.ino !== opened.ino ||
      afterDescriptor.size !== opened.size ||
      !afterPath.isFile() ||
      afterPath.isSymbolicLink() ||
      afterPath.dev !== opened.dev ||
      afterPath.ino !== opened.ino ||
      afterPath.size !== opened.size
    ) {
      fail(`${label} changed identity while being read`);
    }
    return { bytes, identity: { dev: opened.dev, ino: opened.ino, size: opened.size } };
  } finally {
    if (descriptor !== undefined) fs.closeSync(descriptor);
  }
}

function readStableRegularFile(file, label) {
  const first = readRegularFileOnce(file, label);
  const second = readRegularFileOnce(file, label);
  if (
    first.identity.dev !== second.identity.dev ||
    first.identity.ino !== second.identity.ino ||
    first.identity.size !== second.identity.size ||
    !first.bytes.equals(second.bytes)
  ) {
    fail(`${label} changed between stable reads`);
  }
  return { bytes: first.bytes, digest: sha256(first.bytes) };
}

function createPrivateDirectory(directory) {
  fs.mkdirSync(directory, { mode: 0o700 });
  fs.chmodSync(directory, 0o700);
  const metadata = fs.lstatSync(directory);
  if (!metadata.isDirectory() || metadata.isSymbolicLink() || mode(metadata) !== "0700") {
    fail("capture directory is not a private real directory");
  }
}

function copyOwnedArtifact(source, destination, label, expected = null) {
  const sourceRead = readStableRegularFile(source, label);
  if (
    expected &&
    (sourceRead.digest !== expected.sha256 || sourceRead.bytes.length !== expected.size_bytes)
  ) {
    fail(`${label} does not match the verified public size and digest`);
  }
  const descriptor = fs.openSync(
    destination,
    fs.constants.O_WRONLY |
      fs.constants.O_CREAT |
      fs.constants.O_EXCL |
      (fs.constants.O_NOFOLLOW ?? 0),
    0o500,
  );
  try {
    fs.writeFileSync(descriptor, sourceRead.bytes);
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
  fs.chmodSync(destination, 0o500);
  const copied = readStableRegularFile(destination, `private ${label}`);
  if (copied.digest !== sourceRead.digest) fail(`${label} private copy digest drifted`);
  return copied.digest;
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function groupAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    return error?.code !== "ESRCH";
  }
}

async function waitForGroupSettlement(pid, deadline) {
  while (Date.now() < deadline) {
    if (!groupAlive(pid)) return true;
    await delay(POLL_INTERVAL_MS);
  }
  return !groupAlive(pid);
}

function signalGroup(pid, signal) {
  try {
    process.kill(-pid, signal);
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
}

async function terminateAndSettle(child, closed) {
  const pid = child.pid;
  if (!Number.isSafeInteger(pid) || pid <= 0) {
    return Promise.race([closed.then(() => true), delay(TERMINATION_TIMEOUT_MS).then(() => false)]);
  }
  signalGroup(pid, "SIGTERM");
  const softDeadline = Date.now() + Math.floor(TERMINATION_TIMEOUT_MS / 2);
  const softSettled = await waitForGroupSettlement(pid, softDeadline);
  if (!softSettled) {
    signalGroup(pid, "SIGKILL");
  }
  const hardDeadline = Date.now() + Math.ceil(TERMINATION_TIMEOUT_MS / 2);
  const [closeObserved, groupSettled] = await Promise.all([
    Promise.race([closed.then(() => true), delay(TERMINATION_TIMEOUT_MS).then(() => false)]),
    waitForGroupSettlement(pid, hardDeadline),
  ]);
  return closeObserved && groupSettled;
}

async function runBoundedProcess(
  executable,
  args,
  { cwd, env, interruptPromise = null, timeoutMs = PROCESS_TIMEOUT_MS },
) {
  const child = spawn(executable, args, {
    cwd,
    detached: true,
    env,
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const stdoutChunks = [];
  const stderrChunks = [];
  let outputBytes = 0;
  let outputExceeded = false;
  let resolveOutputExceeded;
  const exceeded = new Promise((resolve) => {
    resolveOutputExceeded = resolve;
  });
  const observe = (destination, chunk) => {
    outputBytes += chunk.length;
    if (outputBytes <= MAX_PROCESS_OUTPUT_BYTES) destination.push(chunk);
    if (!outputExceeded && outputBytes > MAX_PROCESS_OUTPUT_BYTES) {
      outputExceeded = true;
      resolveOutputExceeded({ trigger: "output_limit" });
    }
  };
  child.stdout.on("data", (chunk) => observe(stdoutChunks, chunk));
  child.stderr.on("data", (chunk) => observe(stderrChunks, chunk));

  const closed = new Promise((resolve) => {
    let spawnError = null;
    child.once("error", (error) => {
      spawnError = error;
    });
    child.once("close", (code, signal) => resolve({ code, signal, spawnError }));
  });
  const races = [
    closed.then((result) => ({ trigger: "close", result })),
    exceeded,
    delay(timeoutMs).then(() => ({ trigger: "timeout" })),
  ];
  if (interruptPromise) {
    races.push(interruptPromise.then((signal) => ({ signal, trigger: "interrupt" })));
  }
  const winner = await Promise.race(races);

  if (winner.trigger !== "close") {
    const settled = await terminateAndSettle(child, closed);
    if (!settled) fail(`fixed process ${winner.trigger} with unconfirmed process settlement`);
    const reason =
      winner.trigger === "timeout"
        ? "timeout"
        : winner.trigger === "output_limit"
          ? "output limit"
          : `interrupt ${winner.signal}`;
    fail(`fixed process reached its ${reason}`);
  }
  if (winner.result.spawnError) fail("fixed process failed to spawn");
  const groupSettled = await waitForGroupSettlement(child.pid, Date.now() + GROUP_GRACE_MS);
  if (!groupSettled) {
    const settled = await terminateAndSettle(child, closed);
    if (!settled) fail("fixed process descendants did not settle");
    fail("fixed process left descendants after parent exit");
  }
  if (winner.result.code !== 0 || winner.result.signal !== null) {
    fail(`fixed process exited unsuccessfully (${winner.result.code ?? winner.result.signal})`);
  }
  return {
    stderr: Buffer.concat(stderrChunks),
    stdout: Buffer.concat(stdoutChunks),
  };
}

function proofEnvironment(home, temporaryDirectory, { appimage = false } = {}) {
  const environment = {
    ...FIXED_COMMAND_ENV,
    HOME: home,
    TMPDIR: temporaryDirectory,
    [PROOF_ENV]: "1",
  };
  if (appimage) environment.APPIMAGE_EXTRACT_AND_RUN = "1";
  return environment;
}

async function runProof(executable, phase, environment, cwd) {
  const output = await runBoundedProcess(
    executable,
    ["--current-user-persistence-proof", "--phase", phase],
    { cwd, env: environment },
  );
  const text = output.stdout.toString("utf8").trim();
  if (!text.startsWith("{") || !text.endsWith("}") || text.includes("\n")) {
    fail(`packaged persistence proof ${phase} did not emit exactly one JSON receipt`);
  }
  try {
    return JSON.parse(text);
  } catch {
    fail(`packaged persistence proof ${phase} emitted invalid JSON`);
  }
}

async function runInstalledTelemetryProof(executable, sourceSha, appVersion, workspace) {
  const home = path.join(workspace, "telemetry-home");
  const temporaryDirectory = path.join(workspace, "telemetry-tmp");
  const runtimeDirectory = path.join(workspace, "telemetry-runtime");
  createPrivateDirectory(home);
  createPrivateDirectory(temporaryDirectory);
  createPrivateDirectory(runtimeDirectory);
  const output = await runBoundedProcess(
    executable,
    [
      "--benchmark",
      "--platform",
      "linux",
      "--architecture",
      "x86_64",
      "--machine-class",
      "github-hosted-ubuntu-22.04",
      "--workload-profile",
      "fixed-default",
      "--warmup-ticks",
      "0",
      "--ticks",
      "2",
      "--sleep-ms",
      "1000",
      "--repeats",
      "1",
      "--strict",
      "--max-p95-ms",
      "10000",
    ],
    {
      cwd: runtimeDirectory,
      env: proofEnvironment(home, temporaryDirectory),
      timeoutMs: COMMAND_TIMEOUT_MS,
    },
  );
  let summary;
  try {
    summary = JSON.parse(output.stdout.toString("utf8"));
  } catch {
    fail("installed deb telemetry proof emitted invalid JSON");
  }
  if (
    summary?.format_version !== 4 ||
    summary.release_identity?.source_commit_sha !== sourceSha ||
    summary.release_identity?.app_version !== appVersion ||
    summary.platform !== "linux" ||
    summary.architecture !== "x86_64" ||
    summary.measurement_origin !== "owned_sampling_engine_refresh_and_protocol_serialization" ||
    summary.evidence_scope !== "core_runtime_host_only" ||
    summary.live_command !== "refresh_now" ||
    summary.warmup_ticks !== 0 ||
    summary.measured_ticks !== 2 ||
    summary.inter_command_delay_ms !== 1000 ||
    summary.repeat_count !== 1 ||
    summary.sample_quality_passed !== true ||
    summary.strict_passed !== true
  ) {
    fail("installed deb telemetry proof did not satisfy the fixed advancing-sample contract");
  }
  return Object.freeze({
    evidence_scope: summary.evidence_scope,
    measured_ticks: summary.measured_ticks,
    samples_advanced: true,
  });
}

function fixedCommand(
  executable,
  args,
  { allowFailure = false, timeoutMs = COMMAND_TIMEOUT_MS } = {},
) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    env: FIXED_COMMAND_ENV,
    maxBuffer: MAX_PROCESS_OUTPUT_BYTES,
    shell: false,
    timeout: timeoutMs,
  });
  if (result.error) fail(`fixed command failed to settle: ${path.basename(executable)}`);
  if (!allowFailure && result.status !== 0) {
    fail(`fixed command exited unsuccessfully: ${path.basename(executable)}`);
  }
  return result;
}

function requireFixedExecutable(file) {
  const metadata = fs.lstatSync(file);
  if (!metadata.isFile() || metadata.isSymbolicLink() || (metadata.mode & 0o111) === 0) {
    fail(`required fixed executable is unavailable: ${path.basename(file)}`);
  }
}

function rootUnitPayload(operation, value) {
  if (operation === "apt-update") return ["/usr/bin/apt-get", "--quiet=2", "update"];
  if (operation === "apt-install") {
    return [
      "/usr/bin/apt-get",
      "--quiet=2",
      "--yes",
      "--no-install-recommends",
      "install",
      "libgtk-3-0",
      "libwebkit2gtk-4.1-0",
      "libayatana-appindicator3-1",
      "librsvg2-2",
      "libxdo3",
    ];
  }
  if (operation === "install") return ["/usr/bin/dpkg", "--install", value];
  if (operation === "purge") return ["/usr/bin/dpkg", "--purge", DEB_PACKAGE_NAME];
  if (operation === "hostile-settlement") {
    return ["/usr/bin/bash", "-c", HOSTILE_ROOT_SETTLEMENT_PROGRAM];
  }
  fail("unknown fixed root unit operation");
}

function rootUnitProperty(unit, property, allowed) {
  const result = fixedCommand(
    "/usr/bin/systemctl",
    ["show", unit, `--property=${property}`, "--value", "--no-pager"],
    { allowFailure: true, timeoutMs: ROOT_UNIT_CONTROL_TIMEOUT_MS },
  );
  if (
    result.status !== 0 ||
    result.signal !== null ||
    result.stdout.includes("\n\n") ||
    !allowed.has(result.stdout.trim())
  ) {
    fail(`fixed root unit ${property} could not be verified`);
  }
  return result.stdout.trim();
}

function rootUnitState(unit) {
  return {
    loadState: rootUnitProperty(unit, "LoadState", new Set(["loaded", "not-found"])),
    activeState: rootUnitProperty(unit, "ActiveState", new Set(["inactive", "failed"])),
  };
}

async function settleRootUnit(unit) {
  const stop = fixedCommand(
    "/usr/bin/sudo",
    ["-n", "/usr/bin/systemctl", "stop", unit],
    { allowFailure: true, timeoutMs: ROOT_UNIT_CONTROL_TIMEOUT_MS },
  );
  if (![0, 5].includes(stop.status) || stop.signal !== null) {
    fail("fixed root unit stop did not return success or exact not-found status");
  }

  const deadline = Date.now() + ROOT_UNIT_SETTLEMENT_TIMEOUT_MS;
  let state;
  let stableSettledObservations = 0;
  let lastDisposition = null;
  do {
    state = rootUnitState(unit);
    if (state.activeState === "inactive") {
      if (stop.status === 5 && state.loadState !== "not-found") {
        fail("fixed root unit stop reported not-found for a loaded unit");
      }
      const disposition = state.loadState === "not-found" ? "collected" : "inactive";
      stableSettledObservations =
        disposition === lastDisposition ? stableSettledObservations + 1 : 1;
      lastDisposition = disposition;
      if (stableSettledObservations >= 3) {
        return Object.freeze({
          disposition,
          process_tree_settled: true,
        });
      }
    } else {
      stableSettledObservations = 0;
      lastDisposition = null;
    }
    await delay(POLL_INTERVAL_MS);
  } while (Date.now() < deadline);
  fail("fixed root unit did not become inactive or collected");
}

function requireRootUnitSettlementReceipt(receipt) {
  if (!receipt || !verifiedRootUnitSettlementReceipts.has(receipt)) {
    fail("root unit settlement receipt must come from the fixed in-process supervisor");
  }
  return receipt;
}

async function runFixedRootUnit(operation, value = null) {
  for (const executable of [
    "/usr/bin/sudo",
    "/usr/bin/systemd-run",
    "/usr/bin/systemctl",
  ]) {
    requireFixedExecutable(executable);
  }
  const payload = rootUnitPayload(operation, value);
  requireFixedExecutable(payload[0]);
  const unit = `batcave-deb-${operation}-${crypto.randomBytes(12).toString("hex")}.service`;
  let interrupted = null;
  let resolveSignal;
  const signalReceived = new Promise((resolve) => {
    resolveSignal = resolve;
  });
  const handlers = new Map(
    ["SIGHUP", "SIGINT", "SIGTERM"].map((signal) => [
      signal,
      () => {
        interrupted ??= signal;
        resolveSignal(signal);
      },
    ]),
  );
  for (const [signal, handler] of handlers) process.on(signal, handler);
  const command = runBoundedProcess(
    "/usr/bin/sudo",
    [
      "-n",
      "/usr/bin/systemd-run",
      "--quiet",
      "--wait",
      "--pipe",
      "--collect",
      "--service-type=exec",
      `--unit=${unit}`,
      "--property=KillMode=control-group",
      "--property=SendSIGKILL=yes",
      "--property=TimeoutStopSec=10s",
      "--property=RuntimeMaxSec=120s",
      "--property=TasksMax=256",
      "--property=ProtectControlGroups=yes",
      "--property=Delegate=no",
      "--setenv=LANG=C",
      "--setenv=LC_ALL=C",
      "--setenv=PATH=/usr/bin:/bin",
      "--setenv=DEBIAN_FRONTEND=noninteractive",
      "--",
      ...payload,
    ],
    {
      env: FIXED_COMMAND_ENV,
      interruptPromise: signalReceived,
      timeoutMs: ROOT_UNIT_CLIENT_TIMEOUT_MS,
    },
  ).then(
    (output) => ({ output }),
    (error) => ({ error }),
  );
  let outcome;
  let settlement;
  let settlementError = null;
  try {
    const winner = await Promise.race([
      command.then((result) => ({ kind: "command", result })),
      signalReceived.then((signal) => ({ kind: "signal", signal })),
    ]);
    if (winner.kind === "signal") {
      try {
        await settleRootUnit(unit);
      } catch {
        // This is only a prompt best-effort stop. The post-client settlement below is authoritative.
      }
      outcome = await command;
    } else {
      outcome = winner.result;
    }
  } finally {
    try {
      settlement = await settleRootUnit(unit);
    } catch (error) {
      settlementError = error;
    } finally {
      for (const [signal, handler] of handlers) process.removeListener(signal, handler);
    }
  }
  if (settlementError) {
    const operationError = outcome?.error?.message;
    fail(
      `fixed root unit cleanup failed${operationError ? ` after operation failure: ${operationError}` : ""}: ${settlementError.message}`,
    );
  }
  if (interrupted) fail(`fixed root unit interrupted by ${interrupted} after settlement`);
  if (outcome?.error) throw outcome.error;
  const receipt = Object.freeze({
    disposition: settlement.disposition,
    operation,
    process_tree_settled: settlement.process_tree_settled,
  });
  verifiedRootUnitSettlementReceipts.add(receipt);
  return { output: outcome.output, receipt };
}

async function runRootSettlementHostileProof() {
  const result = await runFixedRootUnit("hostile-settlement");
  let fixture;
  try {
    fixture = JSON.parse(result.output.stdout.toString("utf8"));
  } catch {
    fail("root settlement hostile fixture emitted invalid JSON");
  }
  if (
    fixture?.schema_version !== 1 ||
    !Array.isArray(fixture.pids) ||
    fixture.pids.length !== 2 ||
    fixture.pids.some((pid) => !Number.isSafeInteger(pid) || pid <= 1)
  ) {
    fail("root settlement hostile fixture receipt is malformed");
  }
  if (fixture.pids.some((pid) => pathPresent(`/proc/${pid}`))) {
    fail("root settlement hostile fixture left a process after unit settlement");
  }
  requireRootUnitSettlementReceipt(result.receipt);
  return result.receipt;
}

async function installFixedRuntimePrerequisites() {
  const update = await runFixedRootUnit("apt-update");
  const install = await runFixedRootUnit("apt-install");
  for (const receipt of [update.receipt, install.receipt]) {
    if (!requireRootUnitSettlementReceipt(receipt).process_tree_settled) {
      fail("fixed runtime prerequisite unit did not settle");
    }
  }
  return [update.receipt, install.receipt];
}

function currentArchitecture() {
  if (process.arch === "x64") return { packet: "x86_64", deb: "amd64" };
  if (process.arch === "arm64") return { packet: "aarch64", deb: "arm64" };
  fail(`unsupported Linux capture architecture: ${process.arch}`);
}

function linuxVersion() {
  const source = fs.readFileSync("/etc/os-release", "utf8");
  const value = source
    .split("\n")
    .find((line) => line.startsWith("PRETTY_NAME="))
    ?.slice("PRETTY_NAME=".length)
    .replace(/^"|"$/gu, "");
  if (!value || value.includes("/") || value.includes("\\")) {
    fail("Linux host version is unavailable or unsafe to retain");
  }
  return value;
}

function verifyOwnedDirectoryChain(workspace, home, root) {
  const absoluteWorkspace = path.resolve(workspace);
  const absoluteHome = path.resolve(home);
  const absoluteRoot = path.resolve(root);
  const relativeComponents = [".local", "share", "BatCaveMonitor"];
  if (
    absoluteHome !== path.join(absoluteWorkspace, "home") ||
    absoluteRoot !== path.join(absoluteHome, ...relativeComponents)
  ) {
    fail("current-user root must be the canonical directory under the isolated HOME");
  }
  const currentUid = process.getuid?.();
  const directories = [
    absoluteWorkspace,
    absoluteHome,
    ...relativeComponents.map((_, offset) =>
      path.join(absoluteHome, ...relativeComponents.slice(0, offset + 1)),
    ),
  ];
  for (const directory of directories) {
    const metadata = fs.lstatSync(directory);
    if (
      !metadata.isDirectory() ||
      metadata.isSymbolicLink() ||
      metadata.uid !== currentUid ||
      fs.realpathSync(directory) !== directory
    ) {
      fail(
        "current-user root path must remain inside the real workspace with only owned directories",
      );
    }
  }
}

function inspectRoot(root, { expectedHome = null, expectedWorkspace = null } = {}) {
  if (expectedHome || expectedWorkspace) {
    if (!expectedHome || !expectedWorkspace) {
      fail("both the expected workspace and HOME are required for contained root inspection");
    }
    verifyOwnedDirectoryChain(expectedWorkspace, expectedHome, root);
  }
  const rootMetadata = fs.lstatSync(root);
  if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
    fail("current-user root must be a real directory");
  }
  const currentUid = process.getuid?.();
  const files = Object.entries(COMPONENT_FILES)
    .flatMap(([file, component]) => {
      let metadata;
      try {
        metadata = fs.lstatSync(path.join(root, file));
      } catch (error) {
        if (error?.code === "ENOENT") return [];
        throw error;
      }
      if (!metadata.isFile() || metadata.isSymbolicLink()) {
        fail(`current-user ${component} must be a regular non-link file`);
      }
      return [
        {
          component,
          private_permissions_verified: metadata.uid === currentUid && mode(metadata) === "0600",
          mode: mode(metadata),
        },
      ];
    })
    .sort((left, right) => left.component.localeCompare(right.component));
  return {
    canonical_location: "home_local_share",
    owner_verified: rootMetadata.uid === currentUid,
    permission_model: "unix_mode",
    private_permissions_verified: mode(rootMetadata) === "0700",
    directory_mode: mode(rootMetadata),
    files,
  };
}

function writeCorruptSettings(settingsFile) {
  const metadata = fs.lstatSync(settingsFile);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail("settings source must be a regular non-link file before corruption");
  }
  const descriptor = fs.openSync(
    settingsFile,
    fs.constants.O_WRONLY | fs.constants.O_TRUNC | (fs.constants.O_NOFOLLOW ?? 0),
  );
  try {
    const opened = fs.fstatSync(descriptor);
    if (opened.dev !== metadata.dev || opened.ino !== metadata.ino || !opened.isFile()) {
      fail("settings source changed identity while being opened for corruption");
    }
    fs.writeFileSync(descriptor, CORRUPT_SETTINGS);
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
}

function privateRootPermissionsVerified(rootEvidence) {
  return (
    rootEvidence.owner_verified &&
    rootEvidence.private_permissions_verified &&
    rootEvidence.files.every((file) => file.private_permissions_verified)
  );
}

function validateReceipt(receipt, phase, sourceSha, appVersion, installKind) {
  const source = { source_sha: sourceSha, app_version: appVersion };
  validateCurrentUserPersistenceReceipt(receipt, `receipt.${phase}`, phase, source);
  if (receipt.platform !== "linux") fail(`receipt.${phase}.platform must equal linux`);
  if (receipt.architecture !== currentArchitecture().packet) {
    fail(`receipt.${phase}.architecture does not match the capture host`);
  }
  if (receipt.install_kind !== installKind) {
    fail(`receipt.${phase}.install_kind must equal ${installKind}`);
  }
}

function isoSeconds(date = new Date()) {
  return date.toISOString().replace(/\.\d{3}Z$/u, "Z");
}

function buildPacket({
  artifactDigest,
  artifactKind,
  checks,
  installKind,
  limitations,
  receipts,
  rootEvidence,
  sourceSha,
  host = null,
}) {
  const permissionsPassed =
    rootEvidence.owner_verified &&
    rootEvidence.private_permissions_verified &&
    rootEvidence.files.every((file) => file.private_permissions_verified) &&
    Object.values(receipts).every(
      (receipt) =>
        receipt.persistence?.current_user_root?.directory_reported === true &&
        receipt.persistence.current_user_root.permission_state === "verified",
    );
  const result =
    Object.values(checks).every(Boolean) &&
    permissionsPassed &&
    receipts.degraded.health_degraded === true
      ? "passed"
      : "failed";
  const packet = {
    schema_version: 1,
    packet_kind: "native_candidate",
    packet_id: `linux-${artifactKind}-${sourceSha.slice(0, 12)}`,
    observed_at_utc: isoSeconds(),
    source: {
      repository: "TheGreenCedar/BatCave",
      source_sha: sourceSha,
      app_version: receipts.initialize.release_identity.app_version,
    },
    host: {
      platform: "linux",
      architecture: host?.architecture ?? currentArchitecture().packet,
      os_version: host?.osVersion ?? linuxVersion(),
    },
    artifact: {
      kind: artifactKind,
      sha256: artifactDigest,
      digest_scope: "artifact_bytes",
      install_kind: installKind,
    },
    root: rootEvidence,
    receipts,
    checks,
    result,
    limitations,
  };
  validateCurrentUserPersistencePacket(packet);
  return packet;
}

function packageInstalled(packageName) {
  return packageStatus(packageName) === "install ok installed";
}

function packageStatus(packageName) {
  const result = fixedCommand(
    "/usr/bin/dpkg-query",
    ["--show", "--showformat=${Status}", packageName],
    { allowFailure: true },
  );
  if (result.status === 0 && result.signal === null) return result.stdout.trim();
  const absent = `dpkg-query: no packages found matching ${packageName}`;
  if (
    result.status === 1 &&
    result.signal === null &&
    result.stdout.length === 0 &&
    result.stderr.trim() === absent
  ) {
    return null;
  }
  fail("dpkg package-state query failed without proving the package absent");
}

function debMetadata(deb) {
  const field = (name) => fixedCommand("/usr/bin/dpkg-deb", ["--field", deb, name]).stdout.trim();
  const packageName = field("Package");
  const version = field("Version");
  const architecture = field("Architecture");
  if (packageName !== DEB_PACKAGE_NAME) {
    fail(`deb package name must equal ${DEB_PACKAGE_NAME}`);
  }
  if (!VERSION.test(version)) fail("deb package version is malformed");
  if (architecture !== currentArchitecture().deb) {
    fail(`deb architecture must equal ${currentArchitecture().deb}`);
  }
  return { architecture, packageName, version };
}

function installedPackageFiles(packageName, { requireExecutables = true } = {}) {
  const result = fixedCommand("/usr/bin/dpkg-query", ["--listfiles", packageName]);
  const candidates = result.stdout
    .split("\n")
    .filter(Boolean)
    .map((entry) => path.normalize(entry));
  if (
    candidates.length === 0 ||
    candidates.some((entry) => !path.isAbsolute(entry) || entry.includes("\u0000")) ||
    new Set(candidates).size !== candidates.length
  ) {
    fail("installed deb owned-path inventory is malformed");
  }
  const files = candidates.filter((entry) => {
    try {
      return !fs.lstatSync(entry).isDirectory();
    } catch (error) {
      if (error?.code === "ENOENT") return false;
      throw error;
    }
  });
  for (const executable of ["/usr/bin/batcave-monitor", "/usr/bin/batcave-monitor-cli"]) {
    if (!requireExecutables) continue;
    if (!files.includes(executable)) {
      fail(`installed deb owned-path inventory is missing ${path.basename(executable)}`);
    }
    const ownership = fixedCommand("/usr/bin/dpkg-query", ["--search", executable]);
    if (!ownership.stdout.startsWith(`${packageName}: `)) {
      fail(`installed ${path.basename(executable)} is not owned by the expected package`);
    }
  }
  return files;
}

async function installDeb(deb, packageName) {
  if (packageStatus(packageName) !== null) {
    fail("deb package already has registered state on the capture host");
  }
  const rootUnit = await runFixedRootUnit("install", deb);
  if (!packageInstalled(packageName)) fail("deb package did not reach installed state");
  const executable = "/usr/bin/batcave-monitor-cli";
  for (const installedExecutable of ["/usr/bin/batcave-monitor", executable]) {
    const metadata = fs.lstatSync(installedExecutable);
    if (!metadata.isFile() || metadata.isSymbolicLink() || (metadata.mode & 0o111) === 0) {
      fail(`installed ${path.basename(installedExecutable)} must be an executable regular non-link file`);
    }
  }
  return {
    executable,
    ownedFiles: installedPackageFiles(packageName),
    settlementReceipt: rootUnit.receipt,
  };
}

async function purgeDeb(packageName, ownedFiles = []) {
  const errors = [];
  let settlementReceipt = null;
  if (packageStatus(packageName) !== null) {
    if (ownedFiles.length === 0) {
      try {
        ownedFiles = installedPackageFiles(packageName, { requireExecutables: false });
      } catch (error) {
        errors.push(`owned inventory: ${error.message}`);
      }
    }
    try {
      settlementReceipt = (await runFixedRootUnit("purge")).receipt;
    } catch (error) {
      errors.push(`fixed purge: ${error.message}`);
    }
  }
  try {
    if (packageStatus(packageName) !== null) errors.push("package remained registered");
  } catch (error) {
    errors.push(`post-purge package state: ${error.message}`);
  }
  let residue = null;
  try {
    residue = [
      ...ownedFiles,
      "/usr/bin/batcave-monitor",
      "/usr/bin/batcave-monitor-cli",
    ].find((file) => pathPresent(file));
  } catch (error) {
    errors.push(`post-purge residue query: ${error.message}`);
  }
  if (residue) {
    errors.push(`package-owned path remained: ${path.basename(residue)}`);
  }
  if (errors.length > 0) fail(`deb purge cleanup failed (${errors.join("; ")})`);
  return settlementReceipt;
}

async function captureLifecycle({
  expectedAppVersion,
  artifactDigest,
  artifactKind,
  executable,
  installKind,
  limitations,
  removeApplication,
  sourceSha,
  workspace,
}) {
  const home = path.join(workspace, "home");
  const temporaryDirectory = path.join(workspace, "tmp");
  const runtimeDirectory = path.join(workspace, "runtime");
  createPrivateDirectory(home);
  createPrivateDirectory(temporaryDirectory);
  createPrivateDirectory(runtimeDirectory);
  const root = path.join(home, ".local", "share", "BatCaveMonitor");
  const outsideSentinel = path.join(workspace, "outside-sentinel");
  const sentinelBytes = crypto.randomBytes(32);
  fs.writeFileSync(outsideSentinel, sentinelBytes, { flag: "wx", mode: 0o600 });
  const environment = proofEnvironment(home, temporaryDirectory, {
    appimage: installKind === "appimage",
  });

  const initialize = await runProof(executable, "initialize", environment, runtimeDirectory);
  if (initialize.release_identity?.source_commit_sha !== sourceSha) {
    fail("packaged initialize receipt does not contain the exact source SHA");
  }
  const appVersion = initialize.release_identity?.app_version;
  if (!VERSION.test(appVersion ?? "")) {
    fail("packaged initialize receipt version is malformed");
  }
  if (expectedAppVersion && appVersion !== expectedAppVersion) {
    fail("packaged initialize receipt version does not match the package");
  }
  validateReceipt(initialize, "initialize", sourceSha, appVersion, installKind);
  const restart = await runProof(executable, "restart", environment, runtimeDirectory);
  validateReceipt(restart, "restart", sourceSha, appVersion, installKind);
  const restartSettingsPreserved =
    JSON.stringify(initialize.settings) === JSON.stringify(restart.settings);

  const settingsFile = path.join(root, "settings.json");
  const initialRootEvidence = inspectRoot(root, {
    expectedHome: home,
    expectedWorkspace: workspace,
  });
  if (!privateRootPermissionsVerified(initialRootEvidence)) {
    fail("packaged current-user state was not private before capture mutation");
  }
  writeCorruptSettings(settingsFile);
  const degraded = await runProof(executable, "degraded", environment, runtimeDirectory);
  validateReceipt(degraded, "degraded", sourceSha, appVersion, installKind);
  const corruptSourcePreserved =
    readStableRegularFile(settingsFile, "corrupt settings source").digest ===
    sha256(CORRUPT_SETTINGS);

  const applicationRemoved = await removeApplication();
  const rootEvidence = inspectRoot(root, { expectedHome: home, expectedWorkspace: workspace });
  const checks = {
    application_removed: applicationRemoved,
    corrupt_source_preserved: corruptSourcePreserved,
    degraded_launch_succeeded: true,
    outside_sentinel_preserved:
      readStableRegularFile(outsideSentinel, "outside sentinel").digest === sha256(sentinelBytes),
    persistence_failure_visible:
      degraded.persistence_warning_present === true && degraded.persistence?.state !== "healthy",
    restart_settings_preserved: restartSettingsPreserved,
    state_root_preserved: pathPresent(root),
  };
  return buildPacket({
    artifactDigest,
    artifactKind,
    checks,
    installKind,
    limitations,
    receipts: { initialize, restart, degraded },
    rootEvidence,
    sourceSha,
  });
}

async function captureDeb(
  source,
  sourceSha,
  { expectedArtifact = null, rootSettlementRequired = false, telemetryRequired = false } = {},
) {
  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-deb-persistence-")),
  );
  fs.chmodSync(workspace, 0o700);
  const artifact = path.join(workspace, "candidate.deb");
  let packageName = null;
  let ownedFiles = [];
  let installAttempted = false;
  let purgeSettlement = null;
  try {
    const artifactDigest = copyOwnedArtifact(source, artifact, "deb artifact", expectedArtifact);
    const hostileSettlement = rootSettlementRequired
      ? await runRootSettlementHostileProof()
      : null;
    const prerequisiteSettlements = rootSettlementRequired
      ? await installFixedRuntimePrerequisites()
      : [];
    const metadata = debMetadata(artifact);
    packageName = metadata.packageName;
    installAttempted = true;
    const installation = await installDeb(artifact, packageName);
    const { executable } = installation;
    ownedFiles = installation.ownedFiles;
    const telemetry = telemetryRequired
      ? await runInstalledTelemetryProof(executable, sourceSha, metadata.version, workspace)
      : null;
    const packet = await captureLifecycle({
      expectedAppVersion: metadata.version,
      artifactDigest,
      artifactKind: "deb",
      executable,
      installKind: "deb",
      limitations: ["candidate_not_release_evidence", "local_bundle_without_public_provenance"],
      removeApplication: async () => {
        purgeSettlement = await purgeDeb(packageName, ownedFiles);
        installAttempted = false;
        return !packageInstalled(packageName) && ownedFiles.every((file) => !pathPresent(file));
      },
      sourceSha,
      workspace,
    });
    if (readStableRegularFile(artifact, "private deb artifact").digest !== artifactDigest) {
      fail("deb artifact changed during native capture");
    }
    const rootSettlements = [
      hostileSettlement,
      ...prerequisiteSettlements,
      installation.settlementReceipt,
      purgeSettlement,
    ].filter(Boolean);
    if (
      rootSettlementRequired &&
      (rootSettlements.length !== 5 ||
        rootSettlements.some(
          (receipt) => !requireRootUnitSettlementReceipt(receipt).process_tree_settled,
        ))
    ) {
      fail("verified public deb root unit settlement receipts are incomplete");
    }
    return { packet, rootSettlements, telemetry };
  } finally {
    let cleanupError = null;
    try {
      if (installAttempted && packageName) await purgeDeb(packageName, ownedFiles);
    } catch (error) {
      cleanupError = error;
    } finally {
      fs.rmSync(workspace, { force: true, recursive: true });
    }
    if (cleanupError) throw cleanupError;
  }
}

async function captureVerifiedPublicDeb(receipt) {
  const verified = requireVerifiedPublicReleaseReceipt(receipt);
  const directory = requireVerifiedPublicReleaseDownloads(verified);
  const contract = expectedReleaseAssetRoles(verified.tag);
  const role = contract.roles.find(({ role: name }) => name === "Linux deb package");
  if (!role) fail("release contract has no Linux deb package role");
  const asset = verified.assets.find(({ name }) => name === role.name);
  if (!asset) fail("verified public release receipt has no Linux deb package");

  const { packet, rootSettlements, telemetry } = await captureDeb(
    path.join(directory, role.name),
    verified.source_sha,
    { expectedArtifact: asset, rootSettlementRequired: true, telemetryRequired: true },
  );
  if (packet.result !== "passed") fail("verified public deb lifecycle did not pass");
  if (packet.artifact.sha256 !== asset.sha256) {
    fail("verified public deb lifecycle digest does not match the public verification receipt");
  }
  if (packet.source.app_version !== verified.app_version) {
    fail("verified public deb lifecycle version does not match the public verification receipt");
  }
  if (!telemetry?.samples_advanced) fail("verified public deb telemetry proof did not pass");
  const result = Object.freeze({
    schema_version: 1,
    proof_scope: "post_public_deb_native_observation",
    disposition: "passed",
    release_evidence_eligible: false,
  });
  verifiedPublicDebCaptureResults.add(result);
  verifiedPublicDebCaptureStates.set(result, {
    asset: Object.freeze({ ...asset }),
    packet,
    receipt: verified,
    rootSettlements,
    telemetry,
  });
  return result;
}

function requireVerifiedPublicDebCaptureResult(result, receipt) {
  const verified = requireVerifiedPublicReleaseReceipt(receipt);
  const state = verifiedPublicDebCaptureStates.get(result);
  if (!result || !verifiedPublicDebCaptureResults.has(result) || state?.receipt !== verified) {
    fail("public deb capture result must be the matching in-process verified native result");
  }
  return state;
}

async function captureAppImage(source, sourceSha) {
  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-appimage-persistence-")),
  );
  fs.chmodSync(workspace, 0o700);
  const artifact = path.join(workspace, "candidate.AppImage");
  try {
    const artifactDigest = copyOwnedArtifact(source, artifact, "AppImage artifact");
    const packet = await captureLifecycle({
      expectedAppVersion: null,
      artifactDigest,
      artifactKind: "appimage",
      executable: artifact,
      installKind: "appimage",
      limitations: [
        "appimage_extract_and_run",
        "candidate_not_release_evidence",
        "local_bundle_without_public_provenance",
      ],
      removeApplication: async () => {
        if (
          readStableRegularFile(artifact, "private AppImage artifact").digest !== artifactDigest
        ) {
          fail("AppImage artifact changed during native capture");
        }
        fs.rmSync(artifact);
        return !pathPresent(artifact);
      },
      sourceSha,
      workspace,
    });
    return packet;
  } finally {
    fs.rmSync(workspace, { force: true, recursive: true });
  }
}

function createOutputDirectory(directory) {
  if (pathPresent(directory)) fail("--output-dir must not already exist");
  createPrivateDirectory(directory);
}

function writePacket(file, packet) {
  validateCurrentUserPersistencePacket(packet);
  fs.writeFileSync(file, `${JSON.stringify(packet, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
}

async function capture(options) {
  if (process.platform !== "linux") fail("this capture helper requires Linux");
  createOutputDirectory(options.outputDir);
  try {
    const deb = (await captureDeb(options.deb, options.sourceSha)).packet;
    const appimage = await captureAppImage(options.appimage, options.sourceSha);
    if (deb.source.app_version !== appimage.source.app_version) {
      fail("deb and AppImage receipts disagree on application version");
    }
    const shortSha = options.sourceSha.slice(0, 12);
    const debOutput = path.join(options.outputDir, `linux-deb-${shortSha}.json`);
    const appimageOutput = path.join(options.outputDir, `linux-appimage-${shortSha}.json`);
    writePacket(debOutput, deb);
    writePacket(appimageOutput, appimage);
    return { appimageOutput, debOutput };
  } catch (error) {
    fs.rmSync(options.outputDir, { force: true, recursive: true });
    throw error;
  }
}

async function main(argv) {
  const options = parseArgs(argv);
  const outputs = await capture(options);
  console.log(`wrote sanitized Linux persistence candidates: ${outputs.debOutput}`);
  console.log(`wrote sanitized Linux persistence candidates: ${outputs.appimageOutput}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}

export const linuxPersistenceCaptureInternals = {
  buildPacket,
  captureVerifiedPublicDeb,
  copyOwnedArtifact,
  createOutputDirectory,
  debPackageName: DEB_PACKAGE_NAME,
  inspectRoot,
  parseArgs,
  privateRootPermissionsVerified,
  proofEnvironment,
  readStableRegularFile,
  requireVerifiedPublicDebCaptureResult,
  runBoundedProcess,
  writeCorruptSettings,
};
