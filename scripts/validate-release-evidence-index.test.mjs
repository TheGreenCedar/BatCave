import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  RELEASE_EVIDENCE_ROLE_TRUST,
  validateReleaseEvidencePacket,
} from "./validate-release-evidence-packet.mjs";
import {
  validateReleaseEvidenceIndex,
  validateReleaseEvidenceIndexFile,
} from "./validate-release-evidence-index.mjs";
import { RELEASE_PLATFORM_SUPPORT_CONTRACT } from "./validate-release-platform-support.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const INDEX_FIXTURE = path.join(
  ROOT,
  "docs",
  "evidence",
  "releases",
  "fixtures",
  "release-evidence-index.v1.json",
);
const REAL_HOSTS = {
  "debian-12-x86_64-glibc": "debian-12",
  "macos-12-universal": "macos-12.0",
  "ubuntu-22.04-x86_64-glibc": "ubuntu-22.04",
  "windows-client-10-x86_64": "windows-client-10.0.16299",
};
const REAL_SIGNATURE_IDENTITIES = {
  apple_notarization: "submission-id:123e4567-e89b-42d3-a456-426614174000",
  apple_staple: `ticket-sha256:${"a".repeat(64)}`,
  authenticode: `sha256:${"b".repeat(64)}`,
  contained_app_developer_id: "Developer ID Application: BatCave Monitor (ABCDEFGHIJ)",
  contained_app_notarization: "submission-id:123e4567-e89b-42d3-a456-426614174001",
  contained_app_staple: `ticket-sha256:${"c".repeat(64)}`,
  developer_id: "Developer ID Application: BatCave Monitor (ABCDEFGHIJ)",
  tauri_updater: "sha256:0dad0009cf5cc87a778f2e951cefaa0faaba637b95a22f6f3064f12cd4136545",
};

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function fixture() {
  return readJson(INDEX_FIXTURE);
}

function fileDigest(file) {
  return `sha256:${crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex")}`;
}

function absolute(root, relative) {
  return path.join(root, ...relative.split("/"));
}

function copyFixtureRepository() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-release-index-"));
  const index = fixture();
  for (const reference of index.packet_references) {
    const source = absolute(ROOT, reference.path);
    const destination = absolute(root, reference.path);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.copyFileSync(source, destination);
  }
  return { root, index };
}

function withFixtureRepository(callback) {
  const repository = copyFixtureRepository();
  try {
    return callback(repository);
  } finally {
    fs.rmSync(repository.root, { recursive: true, force: true });
  }
}

function mutatePacket(root, reference, mutate) {
  const file = absolute(root, reference.path);
  const packet = readJson(file);
  mutate(packet);
  fs.writeFileSync(file, `${JSON.stringify(packet, null, 2)}\n`);
  reference.packet_sha256 = fileDigest(file);
  return packet;
}

function packetAsset(packet) {
  const asset = packet.assets.find(({ name }) => name === packet.platform.package.asset_name);
  return {
    name: asset.name,
    size_bytes: asset.size_bytes,
    sha256: asset.sha256,
    api_digest: asset.api_digest,
    public_url: asset.public_url,
  };
}

function toRealPacket(packet, tag) {
  packet.packet_kind = "release_evidence";
  packet.packet_id = packet.packet_id.replace("schema-fixture", "release-evidence");
  packet.release.tag = tag;
  packet.release.channel = tag.includes("-") ? "prerelease" : "stable";
  packet.release.release_url = `https://github.com/TheGreenCedar/BatCave/releases/tag/${tag}`;
  packet.platform.os_version = REAL_HOSTS[packet.platform.profile_id];
  packet.platform.proof.source = "source_enforced";
  packet.platform.proof.native = "observed";
  delete packet.limitations.synthetic_fixture_no_release_claim;
  if (packet.platform.package.kind === "deb") {
    packet.limitations.deb_checksum_attestation_only.disposition = "accepted";
  }

  const profile = RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.find(
    ({ id }) => id === packet.platform.profile_id,
  );
  const package_ = profile.packages.find(({ kind }) => kind === packet.platform.package.kind);
  const assetName = expectedReleaseAssetRoles(tag).roles.find(
    ({ role }) => role === package_.asset_role,
  ).name;
  packet.platform.package.asset_name = assetName;
  packet.assets[0].name = assetName;
  packet.assets[0].public_url =
    `https://github.com/TheGreenCedar/BatCave/releases/download/${tag}/` +
    encodeURIComponent(assetName);
  for (const [kind, signature] of Object.entries(packet.assets[0].signatures)) {
    signature.identity = REAL_SIGNATURE_IDENTITIES[kind];
  }
  for (const checks of Object.values(packet.checks)) {
    for (const check of Object.values(checks)) {
      check.status = "passed";
      check.outcome = "Temporary real-shape contract test only.";
    }
  }
  assert.equal(validateReleaseEvidencePacket(packet), packet);
  return { packet, packageRole: package_.asset_role };
}

function realShapedRepository() {
  const repository = copyFixtureRepository();
  const tag = "v9.9.9-rc.1";
  const index = repository.index;
  index.index_kind = "release_evidence_index";
  index.index_id = "v9.9.9-rc.1-release-evidence-index";
  index.release.tag = tag;
  index.release.channel = "prerelease";
  index.release.release_url = `https://github.com/TheGreenCedar/BatCave/releases/tag/${tag}`;
  index.non_claims = ["independent_review_and_live_publication_required"];

  for (const reference of index.packet_references) {
    const fixtureFile = absolute(repository.root, reference.path);
    const packet = readJson(fixtureFile);
    const converted = toRealPacket(packet, tag);
    const realPath = `docs/evidence/releases/${tag}/${path.posix.basename(reference.path)}`;
    const realFile = absolute(repository.root, realPath);
    fs.mkdirSync(path.dirname(realFile), { recursive: true });
    fs.writeFileSync(realFile, `${JSON.stringify(converted.packet, null, 2)}\n`);
    reference.packet_id = converted.packet.packet_id;
    reference.path = realPath;
    reference.packet_sha256 = fileDigest(realFile);
    reference.package_role = converted.packageRole;
    reference.public_asset = packetAsset(converted.packet);
  }
  index.packet_references.sort((left, right) => left.path.localeCompare(right.path, "en"));
  return repository;
}

test("publishes a closed version 1 structural schema", () => {
  const schema = readJson(
    path.join(ROOT, "docs", "evidence", "releases", "release-evidence-index.schema.json"),
  );
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.properties.schema_version.const, 1);
  assert.equal(schema.properties.support_contract_version.const, 1);
  assert.deepEqual(schema.properties.index_kind.enum, ["release_evidence_index", "schema_fixture"]);
  assert.equal(schema.$defs.release.additionalProperties, false);
  assert.equal(schema.$defs.packetReference.additionalProperties, false);
  assert.equal(schema.$defs.publicAsset.additionalProperties, false);
  assert.deepEqual(
    schema.$defs.packetReference.properties.profile_id.enum,
    RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.map(({ id }) => id).sort(),
  );
  assert.deepEqual(
    schema.$defs.packetReference.properties.package_role.enum,
    [
      ...new Set(
        RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.flatMap((profile) =>
          profile.packages.map((package_) => package_.asset_role),
        ),
      ),
    ].sort(),
  );
  assert.ok(!Object.hasOwn(schema.properties, "status"));
  assert.ok(!Object.hasOwn(schema.properties, "disposition"));
  assert.ok(!Object.hasOwn(schema.properties, "accepted"));
});

test("validates the explicitly synthetic five-packet index", () => {
  const index = fixture();
  assert.equal(validateReleaseEvidenceIndex(index), index);
  assert.equal(index.index_kind, "schema_fixture");
  assert.deepEqual(index.non_claims, [
    "independent_review_and_live_publication_required",
    "synthetic_fixture_no_release_claim",
  ]);
  assert.equal(index.packet_references.length, 5);
});

test("covers every support profile and distinct package role", () => {
  const index = fixture();
  const profiles = [...new Set(index.packet_references.map(({ profile_id: id }) => id))].sort();
  const roles = index.packet_references.map(({ package_role: role }) => role).sort();
  const expectedProfiles = RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.map(({ id }) => id).sort();
  const expectedRoles = [
    ...new Set(
      RELEASE_PLATFORM_SUPPORT_CONTRACT.profiles.flatMap((profile) =>
        profile.packages.map((package_) => package_.asset_role),
      ),
    ),
  ].sort();
  assert.deepEqual(profiles, expectedProfiles);
  assert.deepEqual(roles, expectedRoles);
});

test("validates a real-shaped index without committing release evidence", () => {
  const repository = realShapedRepository();
  try {
    assert.equal(
      validateReleaseEvidenceIndex(repository.index, { root: repository.root }),
      repository.index,
    );
    assert.equal(repository.index.index_kind, "release_evidence_index");
  } finally {
    fs.rmSync(repository.root, { recursive: true, force: true });
  }
});

const ROOT_FAILURES = [
  [
    "missing schema version",
    (index) => delete index.schema_version,
    /schema_version: is required/u,
  ],
  ["wrong schema version", (index) => (index.schema_version = 2), /must equal 1/u],
  ["unknown root field", (index) => (index.status = "passed"), /index\.status: is not allowed/u],
  ["overall disposition", (index) => (index.disposition = "accepted"), /is not allowed/u],
  ["malformed index ID", (index) => (index.index_id = "Release Index"), /index\.index_id/u],
  ["unknown index kind", (index) => (index.index_kind = "accepted"), /index\.index_kind/u],
  [
    "support-contract drift",
    (index) => (index.support_contract_version = 2),
    /support_contract_version: must equal 1/u,
  ],
  [
    "missing synthetic non-claim",
    (index) => index.non_claims.pop(),
    /synthetic_fixture_no_release_claim/u,
  ],
  [
    "invented acceptance non-claim",
    (index) => index.non_claims.push("release_accepted"),
    /index\.non_claims/u,
  ],
];

for (const [name, mutate, expected] of ROOT_FAILURES) {
  test(`rejects ${name}`, () => {
    const index = fixture();
    mutate(index);
    assert.throws(() => validateReleaseEvidenceIndex(index), expected);
  });
}

const PATH_FAILURES = [
  ["absolute POSIX path", "/tmp/windows-nsis.json"],
  ["absolute Windows path", "C:\\temp\\windows-nsis.json"],
  ["parent traversal", "docs/evidence/releases/fixtures/v1/../windows-nsis.json"],
  ["current-directory segment", "docs/evidence/releases/fixtures/v1/./windows-nsis.json"],
  ["duplicate separator", "docs/evidence/releases/fixtures//v1/windows-nsis.json"],
  ["backslash path", "docs\\evidence\\releases\\fixtures\\v1\\windows-nsis.json"],
  ["path outside evidence root", "scripts/windows-nsis.json"],
];

for (const [name, value] of PATH_FAILURES) {
  test(`rejects ${name}`, () => {
    const index = fixture();
    index.packet_references.at(-1).path = value;
    index.packet_references.sort((left, right) => left.path.localeCompare(right.path, "en"));
    assert.throws(() => validateReleaseEvidenceIndex(index), /canonical repository-relative/u);
  });
}

test("rejects a missing packet file", () => {
  const index = fixture();
  index.packet_references[0].path =
    "docs/evidence/releases/fixtures/v1/linux-appimage-missing.json";
  assert.throws(() => validateReleaseEvidenceIndex(index), /cannot read referenced packet/u);
});

test(
  "rejects a packet reached through a linked repository directory",
  { skip: process.platform === "win32" },
  () => {
    withFixtureRepository(({ root, index }) => {
      const fixtureDirectory = absolute(root, "docs/evidence/releases/fixtures/v1");
      const relocatedDirectory = absolute(root, "docs/evidence/releases/fixtures/relocated-v1");
      fs.renameSync(fixtureDirectory, relocatedDirectory);
      fs.symlinkSync(relocatedDirectory, fixtureDirectory, "dir");
      assert.throws(() => validateReleaseEvidenceIndex(index, { root }), /linked repository path/u);
    });
  },
);

test("rejects packet digest drift", () => {
  const index = fixture();
  index.packet_references[0].packet_sha256 = `sha256:${"f".repeat(64)}`;
  assert.throws(() => validateReleaseEvidenceIndex(index), /does not match/u);
});

test("rejects duplicate references and packet IDs", () => {
  const duplicateReference = fixture();
  duplicateReference.packet_references.push(
    structuredClone(duplicateReference.packet_references[0]),
  );
  duplicateReference.packet_references.sort((left, right) =>
    left.path.localeCompare(right.path, "en"),
  );
  assert.throws(
    () => validateReleaseEvidenceIndex(duplicateReference),
    /duplicate packet ID|duplicate path/u,
  );

  withFixtureRepository(({ root, index }) => {
    const firstId = index.packet_references[0].packet_id;
    mutatePacket(root, index.packet_references[1], (packet) => {
      packet.packet_id = firstId;
    });
    index.packet_references[1].packet_id = firstId;
    assert.throws(() => validateReleaseEvidenceIndex(index, { root }), /duplicate packet ID/u);
  });
});

test("rejects unsorted packet references", () => {
  const index = fixture();
  index.packet_references.reverse();
  assert.throws(() => validateReleaseEvidenceIndex(index), /strictly sorted/u);
});

test("rejects missing profile and package-role coverage", () => {
  const missingProfile = fixture();
  missingProfile.packet_references = missingProfile.packet_references.filter(
    ({ profile_id: profileId }) => profileId !== "debian-12-x86_64-glibc",
  );
  assert.throws(() => validateReleaseEvidenceIndex(missingProfile), /profile coverage/u);

  const missingRole = fixture();
  missingRole.packet_references = missingRole.packet_references.filter(
    ({ package_role: role }) => role !== "macOS universal updater payload",
  );
  assert.throws(() => validateReleaseEvidenceIndex(missingRole), /package-role coverage/u);
});

test("rejects profile, package-role, and selected-asset contradictions", () => {
  const profile = fixture();
  profile.packet_references[0].profile_id = "debian-12-x86_64-glibc";
  assert.throws(() => validateReleaseEvidenceIndex(profile), /must equal packet profile/u);

  const role = fixture();
  role.packet_references[0].package_role = "Linux deb package";
  assert.throws(() => validateReleaseEvidenceIndex(role), /must equal packet package role/u);

  for (const key of ["name", "size_bytes", "sha256", "api_digest", "public_url"]) {
    const asset = fixture();
    const value = asset.packet_references[0].public_asset[key];
    asset.packet_references[0].public_asset[key] =
      typeof value === "number" ? value + 1 : `${value}-other`;
    assert.throws(() => validateReleaseEvidenceIndex(asset), /public_asset/u);
  }
});

test("rejects an index release or workflow identity that contradicts packets", () => {
  const source = fixture();
  for (const key of ["source_sha", "main_sha", "release_target_sha"]) {
    source.release[key] = "9".repeat(40);
  }
  assert.throws(() => validateReleaseEvidenceIndex(source), /release identity contradicts/u);

  const workflow = fixture();
  workflow.release.workflow_run.run_id += 1;
  workflow.release.workflow_run.url =
    `https://github.com/TheGreenCedar/BatCave/actions/runs/` +
    `${workflow.release.workflow_run.run_id}/attempts/1`;
  assert.throws(() => validateReleaseEvidenceIndex(workflow), /release identity contradicts/u);
});

test("rejects valid #98 packets that disagree on source or workflow identity", () => {
  withFixtureRepository(({ root, index }) => {
    mutatePacket(root, index.packet_references[0], (packet) => {
      for (const key of ["source_sha", "main_sha", "release_target_sha"]) {
        packet.release[key] = "8".repeat(40);
      }
      packet.assets[0].attestation.source_sha = "8".repeat(40);
    });
    assert.throws(
      () => validateReleaseEvidenceIndex(index, { root }),
      /release identity contradicts/u,
    );
  });

  withFixtureRepository(({ root, index }) => {
    mutatePacket(root, index.packet_references[0], (packet) => {
      packet.release.workflow_run.run_id = 900000099;
      packet.release.workflow_run.url =
        "https://github.com/TheGreenCedar/BatCave/actions/runs/900000099/attempts/1";
    });
    assert.throws(
      () => validateReleaseEvidenceIndex(index, { root }),
      /release identity contradicts/u,
    );
  });
});

test("runs every referenced packet through the existing #98 validator", () => {
  withFixtureRepository(({ root, index }) => {
    mutatePacket(root, index.packet_references[0], (packet) => {
      packet.checks.install.checksum.status = "passed";
    });
    assert.throws(
      () => validateReleaseEvidenceIndex(index, { root }),
      /valid #98 packet.*cannot claim proof/u,
    );
  });
});

test("rejects fixture packets in a real release index", () => {
  const index = fixture();
  index.index_kind = "release_evidence_index";
  index.non_claims = ["independent_review_and_live_publication_required"];
  assert.throws(
    () => validateReleaseEvidenceIndex(index),
    /release_evidence_index cannot reference a schema_fixture packet/u,
  );
});

test("the CLI validates only the synthetic review input", () => {
  const script = path.join(ROOT, "scripts", "validate-release-evidence-index.mjs");
  const result = spawnSync(process.execPath, [script, INDEX_FIXTURE], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /schema_fixture; 5 packets; review input only/u);
  assert.doesNotMatch(result.stdout, /accepted|passed/u);
  assert.equal(validateReleaseEvidenceIndexFile(INDEX_FIXTURE).index_kind, "schema_fixture");
});

test("release and every validation lane run the index contract tests", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/validate-release-evidence-index\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/validate-release-evidence-index\.test\.mjs/gu)?.length,
    3,
  );
  const validationLanes = [
    ["windows", "linux"],
    ["linux", "macos"],
    ["macos", undefined],
  ];
  for (const [lane, nextLane] of validationLanes) {
    const start = validationWorkflow.indexOf(`\n  ${lane}:\n`);
    assert.notEqual(start, -1, `${lane} validation lane is present`);
    const end = nextLane ? validationWorkflow.indexOf(`\n  ${nextLane}:\n`, start) : undefined;
    const job = validationWorkflow.slice(start, end);
    assert.match(job, /scripts\/validate-release-evidence-index\.test\.mjs/u, `${lane} lane`);
  }
});
