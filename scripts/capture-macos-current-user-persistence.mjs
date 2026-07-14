import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

import { validateCurrentUserPersistencePacket } from "./validate-current-user-persistence-evidence.mjs";

const PROOF_ENV = "BATCAVE_CURRENT_USER_PERSISTENCE_PROOF";
const SOURCE_SHA = /^[0-9a-f]{40}$/u;
const COMPONENT_FILES = {
  "diagnostics.jsonl": "diagnostics",
  "settings.json": "settings",
  "warm-cache.json": "warm_cache",
};

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
    if (!new Set(["--app", "--source-sha", "--output"]).has(key)) {
      fail(`unknown argument: ${key}`);
    }
    if (values.has(key)) fail(`duplicate argument: ${key}`);
    values.set(key, value);
  }
  const app = values.get("--app");
  const sourceSha = values.get("--source-sha");
  if (!app) fail("--app is required");
  if (!sourceSha || !SOURCE_SHA.test(sourceSha)) {
    fail("--source-sha must be an exact lowercase 40-character Git SHA-1");
  }
  return { app: path.resolve(app), sourceSha, output: values.get("--output") };
}

function canonicalArchitecture(value) {
  if (value === "arm64") return "aarch64";
  if (value === "x64") return "x86_64";
  fail(`unsupported macOS architecture: ${value}`);
}

function mode(stat) {
  return (stat.mode & 0o777).toString(8).padStart(4, "0");
}

function hashBundleTree(root) {
  const digest = crypto.createHash("sha256");
  function visit(directory, prefix = "") {
    for (const entry of fs
      .readdirSync(directory, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name))) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        digest.update(`directory\0${relative}\0`);
        visit(absolute, relative);
      } else if (entry.isSymbolicLink()) {
        digest.update(`symlink\0${relative}\0${fs.readlinkSync(absolute)}\0`);
      } else if (entry.isFile()) {
        digest.update(`file\0${relative}\0`);
        digest.update(fs.readFileSync(absolute));
        digest.update("\0");
      } else {
        fail(`unsupported app bundle entry type at ${relative}`);
      }
    }
  }
  visit(root);
  return `sha256:${digest.digest("hex")}`;
}

function packagedExecutable(app) {
  const directory = path.join(app, "Contents", "MacOS");
  if (!fs.statSync(directory).isDirectory()) fail("app bundle has no Contents/MacOS directory");
  const executableName = execFileSync(
    "/usr/bin/plutil",
    ["-extract", "CFBundleExecutable", "raw", path.join(app, "Contents", "Info.plist")],
    { encoding: "utf8" },
  ).trim();
  if (!executableName || path.basename(executableName) !== executableName) {
    fail("app bundle has an invalid CFBundleExecutable");
  }
  const executable = path.join(directory, executableName);
  if (!fs.statSync(executable).isFile()) fail("app bundle GUI executable is missing");
  return executable;
}

function proofEnvironment(home) {
  const environment = { ...process.env, HOME: home, [PROOF_ENV]: "1" };
  delete environment.XDG_DATA_HOME;
  delete environment.LOCALAPPDATA;
  return environment;
}

function runProof(executable, phase, environment) {
  const result = spawnSync(executable, ["--current-user-persistence-proof", "--phase", phase], {
    encoding: "utf8",
    env: environment,
    timeout: 30_000,
  });
  if (result.error)
    fail(`packaged persistence proof ${phase} failed to start: ${result.error.message}`);
  if (result.status !== 0) {
    fail(`packaged persistence proof ${phase} exited ${result.status}: ${result.stderr.trim()}`);
  }
  const output = result.stdout.trim();
  if (!output.startsWith("{") || !output.endsWith("}")) {
    fail(`packaged persistence proof ${phase} did not emit one JSON receipt`);
  }
  try {
    return JSON.parse(output);
  } catch (error) {
    fail(`packaged persistence proof ${phase} emitted invalid JSON: ${error.message}`);
  }
}

function inspectRoot(root) {
  const rootStat = fs.statSync(root);
  const currentUid = process.getuid?.();
  const files = Object.entries(COMPONENT_FILES)
    .filter(([file]) => fs.existsSync(path.join(root, file)))
    .map(([file, component]) => {
      const stat = fs.lstatSync(path.join(root, file));
      return {
        component,
        private_permissions_verified:
          stat.isFile() && stat.uid === currentUid && mode(stat) === "0600",
        mode: mode(stat),
      };
    })
    .sort((left, right) => left.component.localeCompare(right.component));
  return {
    canonical_location: "application_support",
    owner_verified: rootStat.uid === currentUid,
    permission_model: "unix_mode",
    private_permissions_verified: mode(rootStat) === "0700",
    directory_mode: mode(rootStat),
    files,
  };
}

function isoSeconds(date = new Date()) {
  return date.toISOString().replace(/\.\d{3}Z$/u, "Z");
}

function capture({ app, sourceSha }) {
  if (process.platform !== "darwin") fail("this capture helper requires macOS");
  if (!fs.statSync(app).isDirectory() || !app.endsWith(".app")) {
    fail("--app must name a local .app bundle");
  }
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-persistence-proof-"));
  const installedApp = path.join(workspace, "Applications", "BatCave Monitor.app");
  const home = path.join(workspace, "home");
  const root = path.join(home, "Library", "Application Support", "BatCaveMonitor");
  const outsideSentinel = path.join(workspace, "outside-sentinel");
  const corruptSettings = Buffer.from('{"schema_version":1,"corrupt":"preserve-me"', "utf8");
  const sentinelBytes = crypto.randomBytes(32);

  try {
    fs.mkdirSync(path.dirname(installedApp), { recursive: true });
    fs.mkdirSync(home, { recursive: true });
    fs.cpSync(app, installedApp, { recursive: true, preserveTimestamps: true });
    fs.writeFileSync(outsideSentinel, sentinelBytes, { mode: 0o600 });
    const executable = packagedExecutable(installedApp);
    const environment = proofEnvironment(home);

    const initialize = runProof(executable, "initialize", environment);
    const rootEvidence = inspectRoot(root);
    const restart = runProof(executable, "restart", environment);
    const restartSettingsPreserved =
      JSON.stringify(initialize.settings) === JSON.stringify(restart.settings);

    const settingsFile = path.join(root, "settings.json");
    fs.writeFileSync(settingsFile, corruptSettings, { mode: 0o600 });
    const degraded = runProof(executable, "degraded", environment);
    const corruptSourcePreserved = fs.readFileSync(settingsFile).equals(corruptSettings);

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
    const result = Object.values(checks).every(Boolean) ? "passed" : "failed";
    const osVersion = execFileSync("/usr/bin/sw_vers", ["-productVersion"], {
      encoding: "utf8",
    }).trim();
    const packet = {
      schema_version: 1,
      packet_kind: "native_candidate",
      packet_id: `macos-app-bundle-${sourceSha.slice(0, 12)}`,
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
        kind: "app_bundle",
        sha256: hashBundleTree(app),
        digest_scope: "canonical_app_bundle_tree_v1",
        install_kind: "app_bundle",
      },
      root: rootEvidence,
      receipts: { initialize, restart, degraded },
      checks,
      result,
      limitations: ["candidate_not_release_evidence", "staged_application_bundle_only"],
    };
    validateCurrentUserPersistencePacket(packet);
    return packet;
  } finally {
    fs.rmSync(workspace, { recursive: true, force: true });
  }
}

function main(argv) {
  const options = parseArgs(argv);
  const packet = capture(options);
  const payload = `${JSON.stringify(packet, null, 2)}\n`;
  if (options.output) {
    fs.writeFileSync(path.resolve(options.output), payload, { flag: "wx", mode: 0o600 });
    console.log(`wrote sanitized macOS persistence candidate: ${options.output}`);
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

export const macosPersistenceCaptureInternals = {
  canonicalArchitecture,
  hashBundleTree,
  inspectRoot,
  parseArgs,
};
