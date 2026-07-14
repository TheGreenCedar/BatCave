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

const PROOF_ENV = "BATCAVE_CURRENT_USER_PERSISTENCE_PROOF";
const SOURCE_SHA = /^[0-9a-f]{40}$/u;
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u;
const MAX_ARTIFACT_BYTES = 512 * 1024 * 1024;
const MAX_PROCESS_OUTPUT_BYTES = 64 * 1024;
const PROCESS_TIMEOUT_MS = 30_000;
const COMMAND_TIMEOUT_MS = 120_000;
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

function copyOwnedArtifact(source, destination, label) {
  const sourceRead = readStableRegularFile(source, label);
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

async function runBoundedProcess(executable, args, { cwd, env, timeoutMs = PROCESS_TIMEOUT_MS }) {
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
  const winner = await Promise.race([
    closed.then((result) => ({ trigger: "close", result })),
    exceeded,
    delay(timeoutMs).then(() => ({ trigger: "timeout" })),
  ]);

  if (winner.trigger !== "close") {
    const settled = await terminateAndSettle(child, closed);
    if (!settled) fail(`fixed process ${winner.trigger} with unconfirmed process settlement`);
    fail(`fixed process exceeded its ${winner.trigger === "timeout" ? "timeout" : "output limit"}`);
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

function fixedCommand(executable, args, { allowFailure = false } = {}) {
  const result = spawnSync(executable, args, {
    encoding: "utf8",
    env: FIXED_COMMAND_ENV,
    maxBuffer: MAX_PROCESS_OUTPUT_BYTES,
    shell: false,
    timeout: COMMAND_TIMEOUT_MS,
  });
  if (result.error) fail(`fixed command failed to settle: ${path.basename(executable)}`);
  if (!allowFailure && result.status !== 0) {
    fail(`fixed command exited unsuccessfully: ${path.basename(executable)}`);
  }
  return result;
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
  return result.status === 0 ? result.stdout.trim() : null;
}

function debMetadata(deb) {
  const field = (name) => fixedCommand("/usr/bin/dpkg-deb", ["--field", deb, name]).stdout.trim();
  const packageName = field("Package");
  const version = field("Version");
  const architecture = field("Architecture");
  if (packageName !== "batcave-monitor") fail("deb package name must equal batcave-monitor");
  if (!VERSION.test(version)) fail("deb package version is malformed");
  if (architecture !== currentArchitecture().deb) {
    fail(`deb architecture must equal ${currentArchitecture().deb}`);
  }
  return { architecture, packageName, version };
}

function installDeb(deb, packageName) {
  if (packageInstalled(packageName)) fail("deb package is already installed on the capture host");
  fixedCommand("/usr/bin/sudo", ["-n", "/usr/bin/dpkg", "--install", deb]);
  if (!packageInstalled(packageName)) fail("deb package did not reach installed state");
  const executable = "/usr/bin/batcave-monitor-cli";
  const metadata = fs.lstatSync(executable);
  if (!metadata.isFile() || metadata.isSymbolicLink() || (metadata.mode & 0o111) === 0) {
    fail("installed deb CLI must be an executable regular non-link file");
  }
  const ownership = fixedCommand("/usr/bin/dpkg-query", ["--search", executable]);
  if (!ownership.stdout.startsWith(`${packageName}: `)) {
    fail("installed deb CLI is not owned by the expected package");
  }
  return executable;
}

function purgeDeb(packageName) {
  if (packageStatus(packageName) !== null) {
    fixedCommand("/usr/bin/sudo", ["-n", "/usr/bin/dpkg", "--purge", packageName]);
  }
  if (packageStatus(packageName) !== null) fail("deb package remained registered after purge");
  if (fs.existsSync("/usr/bin/batcave-monitor-cli")) {
    fail("deb CLI remained after package purge");
  }
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
    state_root_preserved: fs.existsSync(root),
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

async function captureDeb(source, sourceSha) {
  const workspace = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-deb-persistence-")),
  );
  fs.chmodSync(workspace, 0o700);
  const artifact = path.join(workspace, "candidate.deb");
  let packageName = null;
  let installAttempted = false;
  try {
    const artifactDigest = copyOwnedArtifact(source, artifact, "deb artifact");
    const metadata = debMetadata(artifact);
    packageName = metadata.packageName;
    installAttempted = true;
    const executable = installDeb(artifact, packageName);
    const packet = await captureLifecycle({
      expectedAppVersion: metadata.version,
      artifactDigest,
      artifactKind: "deb",
      executable,
      installKind: "deb",
      limitations: ["candidate_not_release_evidence", "local_bundle_without_public_provenance"],
      removeApplication: async () => {
        purgeDeb(packageName);
        installAttempted = false;
        return !packageInstalled(packageName) && !fs.existsSync(executable);
      },
      sourceSha,
      workspace,
    });
    if (readStableRegularFile(artifact, "private deb artifact").digest !== artifactDigest) {
      fail("deb artifact changed during native capture");
    }
    return packet;
  } finally {
    let cleanupError = null;
    try {
      if (installAttempted && packageName) purgeDeb(packageName);
    } catch (error) {
      cleanupError = error;
    } finally {
      fs.rmSync(workspace, { force: true, recursive: true });
    }
    if (cleanupError) throw cleanupError;
  }
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
        return !fs.existsSync(artifact);
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
  if (fs.existsSync(directory)) fail("--output-dir must not already exist");
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
    const deb = await captureDeb(options.deb, options.sourceSha);
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
  copyOwnedArtifact,
  createOutputDirectory,
  inspectRoot,
  parseArgs,
  privateRootPermissionsVerified,
  proofEnvironment,
  readStableRegularFile,
  runBoundedProcess,
  writeCorruptSettings,
};
