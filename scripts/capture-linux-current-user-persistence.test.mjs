import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { validateCurrentUserPersistencePacket } from "./validate-current-user-persistence-evidence.mjs";
import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE_SHA = "a".repeat(40);
const {
  buildPacket,
  copyOwnedArtifact,
  createOutputDirectory,
  debPackageName,
  inspectRoot,
  parseArgs,
  privateRootPermissionsVerified,
  proofEnvironment,
  readStableRegularFile,
  runBoundedProcess,
  writeCorruptSettings,
} = linuxPersistenceCaptureInternals;

function temporaryDirectory(label) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), `batcave-linux-capture-${label}-`));
  fs.chmodSync(directory, 0o700);
  return directory;
}

function receipt(phase, { degraded = false, installKind = "deb" } = {}) {
  return {
    format_version: 1,
    evidence_scope: "packaged_current_user_persistence_observation",
    phase,
    release_identity: {
      app_version: "0.2.0-rc.2",
      source_commit_sha: SOURCE_SHA,
    },
    platform: "linux",
    architecture: "x86_64",
    install_kind: installKind,
    settings: degraded ? null : { theme: "ember", history_point_limit: 180 },
    health_degraded: degraded,
    persistence_warning_present: degraded,
    persistence: {
      state: degraded ? "degraded" : "healthy",
      current_user_root: {
        directory_reported: true,
        permission_state: "verified",
      },
      components: [
        {
          kind: "diagnostics",
          state: "healthy",
          durability: "durable",
          active_failure: null,
        },
        {
          kind: "settings",
          state: degraded ? "degraded" : "healthy",
          durability: degraded ? "session_only" : "durable",
          active_failure: degraded
            ? { code: "corrupt_data", operation: "parse", retryable: false }
            : null,
        },
        {
          kind: "warm_cache",
          state: "healthy",
          durability: "durable",
          active_failure: null,
        },
      ],
      suppressed_diagnostic_events: 0,
    },
  };
}

function packet(kind = "deb") {
  const installKind = kind;
  const receipts = {
    initialize: receipt("initialize", { installKind }),
    restart: receipt("restart", { installKind }),
    degraded: receipt("degraded", { degraded: true, installKind }),
  };
  return buildPacket({
    artifactDigest: `sha256:${"b".repeat(64)}`,
    artifactKind: kind,
    checks: {
      application_removed: true,
      corrupt_source_preserved: true,
      degraded_launch_succeeded: true,
      outside_sentinel_preserved: true,
      persistence_failure_visible: true,
      restart_settings_preserved: true,
      state_root_preserved: true,
    },
    host: { architecture: "x86_64", osVersion: "Ubuntu 22.04.5 LTS" },
    installKind,
    limitations:
      kind === "appimage"
        ? [
            "appimage_extract_and_run",
            "candidate_not_release_evidence",
            "local_bundle_without_public_provenance",
          ]
        : ["candidate_not_release_evidence", "local_bundle_without_public_provenance"],
    receipts,
    rootEvidence: {
      canonical_location: "home_local_share",
      owner_verified: true,
      permission_model: "unix_mode",
      private_permissions_verified: true,
      directory_mode: "0700",
      files: [
        {
          component: "settings",
          private_permissions_verified: true,
          mode: "0600",
        },
      ],
    },
    sourceSha: SOURCE_SHA,
  });
}

test("requires the exact closed Linux capture arguments", () => {
  assert.equal(debPackageName, "bat-cave-monitor");
  const values = parseArgs([
    "--deb",
    "candidate.deb",
    "--appimage",
    "candidate.AppImage",
    "--source-sha",
    SOURCE_SHA,
    "--output-dir",
    "packets",
  ]);
  assert.equal(values.sourceSha, SOURCE_SHA);
  assert.equal(values.deb, path.resolve("candidate.deb"));
  assert.equal(values.appimage, path.resolve("candidate.AppImage"));
  assert.equal(values.outputDir, path.resolve("packets"));

  assert.throws(() => parseArgs(["--deb", "candidate.deb"]), /--appimage is required/u);
  assert.throws(
    () =>
      parseArgs([
        "--deb",
        "candidate.deb",
        "--deb",
        "other.deb",
        "--appimage",
        "candidate.AppImage",
        "--source-sha",
        SOURCE_SHA,
        "--output-dir",
        "packets",
      ]),
    /duplicate argument/u,
  );
  assert.throws(
    () =>
      parseArgs([
        "--deb",
        "candidate.deb",
        "--appimage",
        "candidate.AppImage",
        "--source-sha",
        "not-a-sha",
        "--output-dir",
        "packets",
      ]),
    /exact lowercase 40-character/u,
  );
  assert.throws(
    () =>
      parseArgs([
        "--deb",
        "candidate.deb",
        "--appimage",
        "candidate.AppImage",
        "--source-sha",
        SOURCE_SHA,
        "--output-dir",
        "packets",
        "--command",
        "sh",
      ]),
    /unknown argument/u,
  );
});

test("uses a fixed minimal proof environment", () => {
  process.env.BATCAVE_TEST_SECRET = "must-not-cross";
  process.env.LD_PRELOAD = "/tmp/not-forwarded.so";
  process.env.http_proxy = "http://proxy.invalid";
  const environment = proofEnvironment("/private/home", "/private/tmp", { appimage: true });
  assert.deepEqual(Object.keys(environment).sort(), [
    "APPIMAGE_EXTRACT_AND_RUN",
    "BATCAVE_CURRENT_USER_PERSISTENCE_PROOF",
    "HOME",
    "LANG",
    "LC_ALL",
    "NO_COLOR",
    "PATH",
    "TMPDIR",
  ]);
  assert.equal(environment.APPIMAGE_EXTRACT_AND_RUN, "1");
  assert.equal(environment.BATCAVE_TEST_SECRET, undefined);
  assert.equal(environment.LD_PRELOAD, undefined);
  assert.equal(environment.http_proxy, undefined);
  delete process.env.BATCAVE_TEST_SECRET;
  delete process.env.LD_PRELOAD;
  delete process.env.http_proxy;
});

test(
  "copies stable artifact bytes into a private non-link file",
  { skip: process.platform !== "linux" },
  () => {
    const root = temporaryDirectory("artifact");
    try {
      const source = path.join(root, "source.AppImage");
      const destination = path.join(root, "owned.AppImage");
      fs.writeFileSync(source, "owned artifact bytes", { mode: 0o500 });
      const expected = readStableRegularFile(source, "source fixture").digest;
      assert.equal(copyOwnedArtifact(source, destination, "fixture artifact"), expected);
      assert.equal(readStableRegularFile(destination, "owned fixture").digest, expected);
      assert.equal(fs.lstatSync(destination).mode & 0o777, 0o500);

      const linked = path.join(root, "linked.AppImage");
      fs.symlinkSync(source, linked);
      assert.throws(
        () => readStableRegularFile(linked, "linked fixture"),
        /regular file reached without links/u,
      );
    } finally {
      fs.rmSync(root, { force: true, recursive: true });
    }
  },
);

test(
  "inspects only private regular current-user state",
  { skip: process.platform === "win32" },
  () => {
    const root = temporaryDirectory("root");
    try {
      fs.writeFileSync(path.join(root, "settings.json"), "{}", { mode: 0o600 });
      const evidence = inspectRoot(root);
      assert.equal(evidence.canonical_location, "home_local_share");
      assert.equal(evidence.owner_verified, true);
      assert.equal(evidence.private_permissions_verified, true);
      assert.deepEqual(evidence.files, [
        { component: "settings", private_permissions_verified: true, mode: "0600" },
      ]);

      fs.rmSync(path.join(root, "settings.json"));
      fs.symlinkSync(path.join(root, "missing"), path.join(root, "settings.json"));
      assert.throws(() => inspectRoot(root), /regular non-link file/u);
    } finally {
      fs.rmSync(root, { force: true, recursive: true });
    }
  },
);

test(
  "rejects linked state ancestors before mutation and preserves the package-created file mode",
  { skip: process.platform === "win32" },
  () => {
    const workspace = fs.realpathSync(temporaryDirectory("state-chain"));
    try {
      const home = path.join(workspace, "home");
      const outside = path.join(workspace, "outside");
      fs.mkdirSync(path.join(home, ".local"), { mode: 0o700, recursive: true });
      fs.mkdirSync(path.join(outside, "BatCaveMonitor"), { mode: 0o700, recursive: true });
      const outsideSettings = path.join(outside, "BatCaveMonitor", "settings.json");
      fs.writeFileSync(outsideSettings, "outside sentinel", { mode: 0o600 });
      fs.symlinkSync(outside, path.join(home, ".local", "share"));

      const linkedRoot = path.join(home, ".local", "share", "BatCaveMonitor");
      assert.throws(
        () => inspectRoot(linkedRoot, { expectedHome: home, expectedWorkspace: workspace }),
        /remain inside the real workspace/u,
      );
      assert.equal(fs.readFileSync(outsideSettings, "utf8"), "outside sentinel");

      fs.rmSync(path.join(home, ".local", "share"));
      const localRoot = path.join(home, ".local", "share", "BatCaveMonitor");
      fs.mkdirSync(localRoot, { mode: 0o700, recursive: true });
      const localSettings = path.join(localRoot, "settings.json");
      fs.writeFileSync(localSettings, "{}", { mode: 0o644 });
      const unsafeEvidence = inspectRoot(localRoot, {
        expectedHome: home,
        expectedWorkspace: workspace,
      });
      assert.equal(unsafeEvidence.files[0].private_permissions_verified, false);
      assert.equal(privateRootPermissionsVerified(unsafeEvidence), false);
      writeCorruptSettings(localSettings);
      assert.equal(fs.lstatSync(localSettings).mode & 0o777, 0o644);
    } finally {
      fs.rmSync(workspace, { force: true, recursive: true });
    }
  },
);

test(
  "rejects a workspace replaced by a symlink before mutating external state",
  { skip: process.platform === "win32" },
  () => {
    const parent = fs.realpathSync(temporaryDirectory("workspace-anchor"));
    const workspace = path.join(parent, "workspace");
    const movedWorkspace = path.join(parent, "moved-workspace");
    const externalWorkspace = path.join(parent, "external-workspace");
    try {
      fs.mkdirSync(workspace, { mode: 0o700 });
      const externalRoot = path.join(
        externalWorkspace,
        "home",
        ".local",
        "share",
        "BatCaveMonitor",
      );
      fs.mkdirSync(externalRoot, { mode: 0o700, recursive: true });
      const externalSettings = path.join(externalRoot, "settings.json");
      fs.writeFileSync(externalSettings, "external sentinel", { mode: 0o600 });

      fs.renameSync(workspace, movedWorkspace);
      fs.symlinkSync(externalWorkspace, workspace);
      const redirectedHome = path.join(workspace, "home");
      const redirectedRoot = path.join(redirectedHome, ".local", "share", "BatCaveMonitor");
      assert.throws(
        () =>
          inspectRoot(redirectedRoot, {
            expectedHome: redirectedHome,
            expectedWorkspace: workspace,
          }),
        /remain inside the real workspace/u,
      );
      assert.equal(fs.readFileSync(externalSettings, "utf8"), "external sentinel");
    } finally {
      fs.rmSync(parent, { force: true, recursive: true });
    }
  },
);

test("builds validator-clean deb and AppImage native candidates", () => {
  for (const kind of ["deb", "appimage"]) {
    const candidate = packet(kind);
    assert.equal(validateCurrentUserPersistencePacket(candidate), candidate);
    assert.equal(candidate.packet_kind, "native_candidate");
    assert.equal(candidate.artifact.kind, kind);
    assert.equal(candidate.artifact.install_kind, kind);
    assert.equal(candidate.result, "passed");
    assert.ok(candidate.limitations.includes("candidate_not_release_evidence"));
  }
});

test("packet result fails closed when lifecycle or permissions fail", () => {
  const candidate = packet("deb");
  candidate.checks.outside_sentinel_preserved = false;
  assert.throws(
    () => validateCurrentUserPersistencePacket(candidate),
    /result: must equal failed/u,
  );
  candidate.result = "failed";
  assert.equal(validateCurrentUserPersistencePacket(candidate), candidate);
});

test(
  "bounded process capture rejects output overflow and surviving descendants",
  { skip: process.platform === "win32" },
  async () => {
    const root = temporaryDirectory("process");
    const environment = { HOME: root, PATH: process.env.PATH ?? "/usr/bin:/bin" };
    try {
      const normal = await runBoundedProcess(
        process.execPath,
        ["--eval", 'process.stdout.write("settled")'],
        { cwd: root, env: environment, timeoutMs: 2_000 },
      );
      assert.equal(normal.stdout.toString("utf8"), "settled");
      assert.equal(normal.stderr.length, 0);

      await assert.rejects(
        runBoundedProcess(
          process.execPath,
          ["--eval", 'process.stdout.write("x".repeat(70*1024));setInterval(()=>{},60000)'],
          { cwd: root, env: environment, timeoutMs: 2_000 },
        ),
        /output limit/u,
      );

      const descendant = [
        'const {spawn}=require("node:child_process");',
        'const child=spawn(process.execPath,["--eval","setInterval(()=>{},60000)"],{stdio:"ignore"});',
        'child.once("spawn",()=>{child.unref();process.exit(0)});',
      ].join("");
      await assert.rejects(
        runBoundedProcess(process.execPath, ["--eval", descendant], {
          cwd: root,
          env: environment,
          timeoutMs: 2_000,
        }),
        /left descendants/u,
      );
    } finally {
      fs.rmSync(root, { force: true, recursive: true });
    }
  },
);

test("output directory must be new and private", { skip: process.platform === "win32" }, () => {
  const root = temporaryDirectory("output");
  const output = path.join(root, "packets");
  try {
    createOutputDirectory(output);
    assert.equal(fs.lstatSync(output).mode & 0o777, 0o700);
    assert.throws(() => createOutputDirectory(output), /must not already exist/u);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("bundle workflow keeps native package execution on the trusted Linux-only manual path", () => {
  const workflow = fs.readFileSync(path.join(ROOT, ".github", "workflows", "bundles.yml"), "utf8");
  assert.match(workflow, /BATCAVE_SOURCE_COMMIT_SHA:\s*\$\{\{ github\.sha \}\}/u);
  assert.match(
    workflow,
    /github\.event_name == 'workflow_dispatch' && inputs\.capture_linux_persistence == true && github\.actor == github\.repository_owner/u,
  );
  assert.match(workflow, /capture_linux_persistence:\s*[\s\S]*?default: false/u);
  assert.equal(
    workflow.match(
      /if: github\.event_name != 'workflow_dispatch' \|\| inputs\.capture_linux_persistence != true/gu,
    )?.length,
    2,
  );
  assert.match(workflow, /capture-linux-current-user-persistence\.mjs/u);
  assert.doesNotMatch(workflow, /pull_request:/u);
});
