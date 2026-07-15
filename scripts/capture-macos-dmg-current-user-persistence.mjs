import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

import { macosPersistenceCaptureInternals } from "./capture-macos-current-user-persistence.mjs";
import { validateCurrentUserPersistencePacket } from "./validate-current-user-persistence-evidence.mjs";

const {
  canonicalArchitecture,
  hashBundleTree,
  inspectRoot,
  isoSeconds,
  packagedExecutable,
  proofEnvironment,
  runProof,
} = macosPersistenceCaptureInternals;

const DISK_IMAGES_LOCK = "/tmp/batcave-diskimages-proof.lock";
const SOURCE_SHA = /^[0-9a-f]{40}$/u;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || !value || value.startsWith("--")) {
      fail(`invalid or missing value for ${key ?? "argument"}`);
    }
    if (!new Set(["--dmg", "--source-sha", "--output"]).has(key)) {
      fail(`unknown argument: ${key}`);
    }
    if (values.has(key)) fail(`duplicate argument: ${key}`);
    values.set(key, value);
  }
  const dmg = values.get("--dmg");
  const sourceSha = values.get("--source-sha");
  if (!dmg) fail("--dmg is required");
  if (!sourceSha || !SOURCE_SHA.test(sourceSha)) {
    fail("--source-sha must be an exact lowercase 40-character Git SHA-1");
  }
  return { dmg: path.resolve(dmg), sourceSha, output: values.get("--output") };
}

function mode(stat) {
  return stat.mode & 0o777;
}

function readStableRegularFile(file, label) {
  const before = fs.lstatSync(file);
  if (!before.isFile() || before.isSymbolicLink()) fail(`${label} must be a regular non-link file`);
  let descriptor;
  try {
    descriptor = fs.openSync(file, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
    const opened = fs.fstatSync(descriptor);
    if (!opened.isFile() || opened.dev !== before.dev || opened.ino !== before.ino) {
      fail(`${label} changed identity while being opened`);
    }
    const bytes = fs.readFileSync(descriptor);
    const after = fs.lstatSync(file);
    if (
      !after.isFile() ||
      after.isSymbolicLink() ||
      after.dev !== opened.dev ||
      after.ino !== opened.ino ||
      after.size !== opened.size
    ) {
      fail(`${label} changed identity while being read`);
    }
    return bytes;
  } finally {
    if (descriptor !== undefined) fs.closeSync(descriptor);
  }
}

function sha256(bytes) {
  return `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
}

function writePrivateArtifact(file, bytes) {
  const descriptor = fs.openSync(file, "wx", 0o400);
  try {
    fs.writeFileSync(descriptor, bytes);
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
  const metadata = fs.lstatSync(file);
  if (!metadata.isFile() || metadata.isSymbolicLink() || mode(metadata) !== 0o400) {
    fail("private DMG copy did not retain its fixed regular-file authority");
  }
}

function withDiskImagesLock(callback, lockPath = DISK_IMAGES_LOCK) {
  let descriptor;
  let identity;
  try {
    descriptor = fs.openSync(lockPath, "wx", 0o600);
    identity = fs.fstatSync(descriptor);
    if (!identity.isFile() || mode(identity) !== 0o600 || identity.uid !== process.getuid?.()) {
      fail("DiskImages proof lock has unsafe ownership or mode");
    }
    fs.writeFileSync(descriptor, `${process.pid}\n`);
    fs.fsyncSync(descriptor);
    return callback();
  } catch (error) {
    if (error?.code === "EEXIST") fail("DiskImages proof lock is busy");
    throw error;
  } finally {
    if (descriptor !== undefined) {
      try {
        const current = fs.lstatSync(lockPath);
        if (current.dev !== identity.dev || current.ino !== identity.ino) {
          fail("DiskImages proof lock identity changed before release");
        }
        fs.unlinkSync(lockPath);
      } finally {
        fs.closeSync(descriptor);
      }
    }
  }
}

function runHdiutil(args) {
  const result = spawnSync("/usr/bin/hdiutil", args, {
    encoding: "utf8",
    timeout: 60_000,
    maxBuffer: 64 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error) fail(`fixed hdiutil operation failed to start: ${result.error.message}`);
  if (result.status !== 0) fail(`fixed hdiutil operation exited ${result.status}`);
}

function onlyMountedApp(mountPoint) {
  const apps = fs
    .readdirSync(mountPoint, { withFileTypes: true })
    .filter(
      (entry) => entry.isDirectory() && !entry.isSymbolicLink() && entry.name.endsWith(".app"),
    );
  if (apps.length !== 1) fail("mounted DMG must contain exactly one real app bundle");
  return path.join(mountPoint, apps[0].name);
}

function copyMountedApplication({ privateDmg, expectedDmgDigest, mountPoint, installedApp }) {
  let mounted = false;
  return withDiskImagesLock(() => {
    try {
      if (sha256(readStableRegularFile(privateDmg, "private DMG")) !== expectedDmgDigest) {
        fail("private DMG bytes changed before verification");
      }
      runHdiutil(["verify", privateDmg]);
      runHdiutil(["attach", "-nobrowse", "-readonly", "-mountpoint", mountPoint, privateDmg]);
      mounted = true;
      const mountedApp = onlyMountedApp(mountPoint);
      const mountedDigest = hashBundleTree(mountedApp);
      execFileSync("/usr/bin/ditto", ["--noqtn", mountedApp, installedApp], {
        stdio: ["ignore", "ignore", "pipe"],
        timeout: 60_000,
      });
      const installedDigest = hashBundleTree(installedApp);
      if (installedDigest !== mountedDigest) fail("installed app copy differs from mounted app");
      if (sha256(readStableRegularFile(privateDmg, "private DMG")) !== expectedDmgDigest) {
        fail("private DMG bytes changed while mounted");
      }
      return { installedDigest, mountedDigest };
    } finally {
      if (mounted) runHdiutil(["detach", mountPoint, "-quiet"]);
      if (sha256(readStableRegularFile(privateDmg, "private DMG")) !== expectedDmgDigest) {
        fail("private DMG bytes changed after detach");
      }
    }
  });
}

function capture({ dmg, sourceSha }) {
  if (process.platform !== "darwin") fail("this capture helper requires macOS");
  if (!dmg.endsWith(".dmg")) fail("--dmg must name a local .dmg file");
  const sourceBytes = readStableRegularFile(dmg, "source DMG");
  const artifactDigest = sha256(sourceBytes);
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-dmg-persistence-proof-"));
  fs.chmodSync(workspace, 0o700);
  const privateDmg = path.join(workspace, "candidate.dmg");
  const mountPoint = path.join(workspace, "mount");
  const installedApp = path.join(workspace, "Applications", "BatCave Monitor.app");
  const home = path.join(workspace, "home");
  const root = path.join(home, "Library", "Application Support", "BatCaveMonitor");
  const outsideSentinel = path.join(workspace, "outside-sentinel");
  const corruptSettings = Buffer.from('{"schema_version":1,"corrupt":"preserve-me"', "utf8");
  const sentinelBytes = crypto.randomBytes(32);

  try {
    fs.mkdirSync(mountPoint, { mode: 0o700 });
    fs.mkdirSync(path.dirname(installedApp), { recursive: true, mode: 0o700 });
    fs.mkdirSync(home, { mode: 0o700 });
    writePrivateArtifact(privateDmg, sourceBytes);
    sourceBytes.fill(0);
    fs.writeFileSync(outsideSentinel, sentinelBytes, { mode: 0o600 });

    const { installedDigest, mountedDigest } = copyMountedApplication({
      privateDmg,
      expectedDmgDigest: artifactDigest,
      mountPoint,
      installedApp,
    });
    if (installedDigest !== mountedDigest) fail("mounted application binding failed");

    const executable = packagedExecutable(installedApp);
    const environment = proofEnvironment(home);
    const initialize = runProof(executable, "initialize", environment);
    const restart = runProof(executable, "restart", environment);
    const restartSettingsPreserved =
      JSON.stringify(initialize.settings) === JSON.stringify(restart.settings);

    const settingsFile = path.join(root, "settings.json");
    fs.writeFileSync(settingsFile, corruptSettings, { mode: 0o600 });
    const degraded = runProof(executable, "degraded", environment);
    const corruptSourcePreserved = fs.readFileSync(settingsFile).equals(corruptSettings);
    const rootEvidence = inspectRoot(root);
    if (hashBundleTree(installedApp) !== installedDigest) {
      fail("installed app changed while persistence proof was running");
    }
    if (sha256(readStableRegularFile(privateDmg, "private DMG")) !== artifactDigest) {
      fail("private DMG bytes changed before application removal");
    }

    fs.rmSync(installedApp, { recursive: true, force: true });
    const checks = {
      application_removed: !fs.existsSync(installedApp),
      corrupt_source_preserved: corruptSourcePreserved,
      degraded_launch_succeeded: true,
      outside_sentinel_preserved: fs.readFileSync(outsideSentinel).equals(sentinelBytes),
      persistence_failure_visible:
        degraded.persistence_warning_present === true && degraded.persistence?.state !== "healthy",
      restart_settings_preserved: restartSettingsPreserved,
      state_root_preserved: fs.existsSync(root),
    };
    const permissionsPassed =
      rootEvidence.owner_verified &&
      rootEvidence.private_permissions_verified &&
      rootEvidence.files.every((file) => file.private_permissions_verified) &&
      [initialize, restart, degraded].every(
        (receipt) =>
          receipt.persistence?.current_user_root?.directory_reported === true &&
          receipt.persistence.current_user_root.permission_state === "verified",
      );
    const result =
      Object.values(checks).every(Boolean) && permissionsPassed && degraded.health_degraded === true
        ? "passed"
        : "failed";
    const osVersion = execFileSync("/usr/bin/sw_vers", ["-productVersion"], {
      encoding: "utf8",
    }).trim();
    const packet = {
      schema_version: 1,
      packet_kind: "native_candidate",
      packet_id: `macos-dmg-${sourceSha.slice(0, 12)}`,
      observed_at_utc: isoSeconds(),
      source: {
        repository: "TheGreenCedar/BatCave",
        source_sha: sourceSha,
        app_version: initialize.release_identity.app_version,
      },
      host: {
        platform: "macos",
        architecture: canonicalArchitecture(process.arch),
        os_version: `macOS ${osVersion}`,
      },
      artifact: {
        kind: "dmg",
        sha256: artifactDigest,
        digest_scope: "artifact_bytes",
        install_kind: "app_bundle",
      },
      root: rootEvidence,
      receipts: { initialize, restart, degraded },
      checks,
      result,
      limitations: [
        "adhoc_signature_only",
        "candidate_not_release_evidence",
        "local_bundle_without_public_provenance",
        "path_based_dmg_mount_without_owned_byte_transport",
      ],
    };
    validateCurrentUserPersistencePacket(packet);
    return packet;
  } finally {
    sourceBytes.fill(0);
    fs.rmSync(workspace, { recursive: true, force: true });
  }
}

function main(argv) {
  const options = parseArgs(argv);
  const packet = capture(options);
  const payload = `${JSON.stringify(packet, null, 2)}\n`;
  if (options.output) {
    fs.writeFileSync(path.resolve(options.output), payload, { flag: "wx", mode: 0o600 });
    console.log(`wrote sanitized macOS DMG persistence candidate: ${options.output}`);
  } else {
    process.stdout.write(payload);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

export const macosDmgPersistenceCaptureInternals = {
  copyMountedApplication,
  parseArgs,
  readStableRegularFile,
  sha256,
  withDiskImagesLock,
  writePrivateArtifact,
};
