import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  validateCurrentUserPersistenceIndex,
  validateCurrentUserPersistencePacket,
} from "./validate-current-user-persistence-evidence.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE_SHA = "a".repeat(40);

function receipt(phase, { degraded = false } = {}) {
  return {
    format_version: 1,
    evidence_scope: "packaged_current_user_persistence_observation",
    phase,
    release_identity: {
      app_version: "0.2.0-rc.2",
      source_commit_sha: SOURCE_SHA,
    },
    platform: "macos",
    architecture: "aarch64",
    install_kind: "app_bundle",
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
          durability: degraded ? "not_written" : "durable",
          active_failure: degraded
            ? { code: "json_parse_failed", operation: "parse", retryable: false }
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

function packet({ kind = "app_bundle" } = {}) {
  const installKind = kind === "dmg" ? "app_bundle" : kind;
  const value = {
    schema_version: 1,
    packet_kind: "native_candidate",
    packet_id: `macos-${kind}-candidate`,
    observed_at_utc: "2026-07-14T12:00:00Z",
    source: {
      repository: "TheGreenCedar/BatCave",
      source_sha: SOURCE_SHA,
      app_version: "0.2.0-rc.2",
    },
    host: {
      platform: "macos",
      architecture: "aarch64",
      os_version: "macOS 15.5",
    },
    artifact: {
      kind,
      sha256: `sha256:${"b".repeat(64)}`,
      digest_scope: kind === "app_bundle" ? "canonical_app_bundle_tree_v1" : "artifact_bytes",
      install_kind: installKind,
    },
    root: {
      canonical_location: "application_support",
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
    receipts: {
      initialize: receipt("initialize"),
      restart: receipt("restart"),
      degraded: receipt("degraded", { degraded: true }),
    },
    checks: {
      application_removed: true,
      corrupt_source_preserved: true,
      degraded_launch_succeeded: true,
      outside_sentinel_preserved: true,
      persistence_failure_visible: true,
      restart_settings_preserved: true,
      state_root_preserved: true,
    },
    result: "passed",
    limitations: ["candidate_not_release_evidence", "staged_application_bundle_only"],
  };
  for (const proof of Object.values(value.receipts)) {
    proof.install_kind = installKind;
  }
  return value;
}

test("accepts a sanitized native candidate without granting release status", () => {
  assert.equal(validateCurrentUserPersistencePacket(packet()).result, "passed");
});

test("rejects source drift, hidden fields, raw paths, and false passing results", () => {
  const sourceDrift = packet();
  sourceDrift.receipts.restart.release_identity.source_commit_sha = "c".repeat(40);
  assert.throws(
    () => validateCurrentUserPersistencePacket(sourceDrift),
    /must match packet\.source\.source_sha/u,
  );

  const hidden = packet();
  hidden.receipts.restart.local_path = "/Users/albert/private";
  assert.throws(() => validateCurrentUserPersistencePacket(hidden), /local_path: is not allowed/u);

  const rawPath = packet();
  rawPath.host.os_version = "/Users/albert/private";
  assert.throws(
    () => validateCurrentUserPersistencePacket(rawPath),
    /absolute or local machine paths/u,
  );

  const falsePass = packet();
  falsePass.checks.state_root_preserved = false;
  assert.throws(
    () => validateCurrentUserPersistencePacket(falsePass),
    /result: must equal failed/u,
  );

  const permissionFailure = packet();
  permissionFailure.root.owner_verified = false;
  assert.throws(
    () => validateCurrentUserPersistencePacket(permissionFailure),
    /result: must equal failed/u,
  );
  permissionFailure.result = "failed";
  assert.equal(validateCurrentUserPersistencePacket(permissionFailure), permissionFailure);

  const impossibleTime = packet();
  impossibleTime.observed_at_utc = "2026-02-30T12:00:00Z";
  assert.throws(
    () => validateCurrentUserPersistencePacket(impossibleTime),
    /must be a real UTC timestamp/u,
  );
});

test("requires visible degraded persistence and preserves the fixed restart mutation", () => {
  const hiddenFailure = packet();
  hiddenFailure.receipts.degraded.persistence_warning_present = false;
  assert.throws(
    () => validateCurrentUserPersistencePacket(hiddenFailure),
    /persistence_warning_present: must be true/u,
  );

  const changedSettings = packet();
  changedSettings.receipts.restart.settings.theme = "cave";
  assert.throws(
    () => validateCurrentUserPersistencePacket(changedSettings),
    /must equal the fixed probe value ember/u,
  );

  const hiddenHealth = packet();
  hiddenHealth.receipts.degraded.health_degraded = false;
  assert.throws(
    () => validateCurrentUserPersistencePacket(hiddenHealth),
    /result: must equal failed/u,
  );
  hiddenHealth.result = "failed";
  assert.equal(validateCurrentUserPersistencePacket(hiddenHealth), hiddenHealth);

  for (const rootFailure of [
    { field: "directory_reported", value: false },
    { field: "permission_state", value: "invalid" },
  ]) {
    const unverified = packet();
    unverified.receipts.restart.persistence.current_user_root[rootFailure.field] =
      rootFailure.value;
    assert.throws(
      () => validateCurrentUserPersistencePacket(unverified),
      /result: must equal failed/u,
    );
  }
});

test("validates checked-in native candidates without treating pending profiles as blocked", () => {
  const index = JSON.parse(
    fs.readFileSync(
      path.join(ROOT, "docs/evidence/persistence/current-user-persistence-index.v1.json"),
      "utf8",
    ),
  );
  assert.equal(validateCurrentUserPersistenceIndex(index, { repositoryRoot: ROOT }), index);
  assert.deepEqual(
    index.profiles.map(({ status }) => status),
    ["native_candidate", "native_candidate", "native_candidate", "pending"],
  );
});

test("validates every retained unindexed native candidate", () => {
  const candidatesRoot = path.join(ROOT, "docs/evidence/persistence/native-candidates");
  const candidates = fs
    .readdirSync(candidatesRoot, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
    .map((entry) => entry.name)
    .sort();
  assert.ok(candidates.length > 0);

  const index = JSON.parse(
    fs.readFileSync(
      path.join(ROOT, "docs/evidence/persistence/current-user-persistence-index.v1.json"),
      "utf8",
    ),
  );
  const indexedPaths = new Set(
    index.profiles.map(({ packet_path: packetPath }) => packetPath).filter(Boolean),
  );
  for (const candidate of candidates) {
    const relative = `docs/evidence/persistence/native-candidates/${candidate}`;
    assert.ok(!indexedPaths.has(relative));
    validateCurrentUserPersistencePacket(
      JSON.parse(fs.readFileSync(path.join(candidatesRoot, candidate), "utf8")),
      `candidate.${candidate}`,
    );
  }
});

test("rehashes and cross-checks indexed native packets", () => {
  const repositoryRoot = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-persistence-index-"));
  const packetPath = "docs/evidence/persistence/macos-dmg.json";
  const packetFile = path.join(repositoryRoot, packetPath);
  fs.mkdirSync(path.dirname(packetFile), { recursive: true });
  const nativePacket = packet({ kind: "dmg" });
  fs.writeFileSync(packetFile, `${JSON.stringify(nativePacket, null, 2)}\n`);
  const digest = `sha256:${crypto
    .createHash("sha256")
    .update(fs.readFileSync(packetFile))
    .digest("hex")}`;
  const index = {
    schema_version: 1,
    index_kind: "current_user_persistence_evidence",
    profiles: [
      {
        id: "linux-appimage",
        platform: "linux",
        package_kind: "appimage",
        status: "pending",
        packet_path: null,
        packet_sha256: null,
      },
      {
        id: "linux-deb",
        platform: "linux",
        package_kind: "deb",
        status: "pending",
        packet_path: null,
        packet_sha256: null,
      },
      {
        id: "macos-dmg",
        platform: "macos",
        package_kind: "dmg",
        status: "native_candidate",
        packet_path: packetPath,
        packet_sha256: digest,
      },
      {
        id: "windows-nsis",
        platform: "windows",
        package_kind: "nsis",
        status: "pending",
        packet_path: null,
        packet_sha256: null,
      },
    ],
  };
  assert.equal(validateCurrentUserPersistenceIndex(index, { repositoryRoot }), index);

  index.profiles[2].packet_sha256 = `sha256:${"0".repeat(64)}`;
  assert.throws(
    () => validateCurrentUserPersistenceIndex(index, { repositoryRoot }),
    /does not match packet bytes/u,
  );
  index.profiles[2].packet_sha256 = digest;

  const linkedRoot = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-persistence-linked-"));
  fs.writeFileSync(path.join(linkedRoot, "macos-dmg.json"), fs.readFileSync(packetFile));
  const evidenceDirectory = path.dirname(packetFile);
  fs.rmSync(evidenceDirectory, { recursive: true, force: true });
  fs.symlinkSync(linkedRoot, evidenceDirectory, process.platform === "win32" ? "junction" : "dir");
  assert.throws(
    () => validateCurrentUserPersistenceIndex(index, { repositoryRoot }),
    /must not traverse a linked repository path/u,
  );

  fs.rmSync(repositoryRoot, { recursive: true, force: true });
  fs.rmSync(linkedRoot, { recursive: true, force: true });
});

test("validation and release workflows execute this contract", () => {
  for (const workflow of [".github/workflows/validation.yml", ".github/workflows/release.yml"]) {
    const source = fs.readFileSync(path.join(ROOT, workflow), "utf8");
    assert.match(source, /scripts\/capture-macos-dmg-current-user-persistence\.test\.mjs/u);
    assert.match(source, /scripts\/validate-current-user-persistence-evidence\.test\.mjs/u);
  }
});
