import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  RELEASE_EVIDENCE_ROLE_TRUST,
  validateReleaseEvidencePacket,
} from "./validate-release-evidence-packet.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE_DIR = path.join(ROOT, "docs", "evidence", "releases", "fixtures", "v1");
const FIXTURES = [
  ["windows-nsis.json", "windows", "nsis"],
  ["linux-deb.json", "linux", "deb"],
  ["linux-appimage.json", "linux", "appimage"],
  ["macos-dmg.json", "macos", "dmg"],
  ["macos-updater.json", "macos", "macos_updater"],
];
const PACKAGE_ROLES = {
  appimage: "Linux AppImage package and updater payload",
  deb: "Linux deb package",
  dmg: "macOS Apple Silicon DMG",
  macos_updater: "macOS Apple Silicon updater payload",
  nsis: "Windows NSIS installer and updater payload",
};
const REAL_HOSTS = {
  "debian-12-x86_64-glibc": "debian-12",
  "macos-12-arm64": "macos-12.0",
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
const SYNTHETIC_SIGNATURE_IDENTITIES = {
  apple_notarization: "synthetic Apple notarization fixture",
  apple_staple: "synthetic stapled ticket fixture",
  authenticode: "synthetic Authenticode signer fixture",
  contained_app_developer_id: "synthetic Developer ID signer fixture",
  contained_app_notarization: "synthetic contained-app notarization fixture",
  contained_app_staple: "synthetic contained-app stapled ticket fixture",
  developer_id: "synthetic Developer ID signer fixture",
  tauri_updater: "synthetic updater key fingerprint fixture",
};

function authenticodeSignature(identity, packetKind, asset) {
  const fixture = packetKind === "schema_fixture";
  const subject = fixture ? "synthetic Authenticode subject fixture" : "CN=Albert Najjar";
  const certificate = fixture ? `sha256:${"1".repeat(64)}` : identity;
  return {
    identity,
    verified: true,
    subject,
    rfc3161_timestamp_utc: "2000-01-01T00:00:00Z",
    files: [
      {
        name: asset.name,
        sha256: asset.sha256,
        disposition: asset.name.endsWith("-setup.exe") ? "generated_signed" : "batcave_signed",
        subject,
        certificate_sha256: certificate,
        rfc3161_timestamp_utc: "2000-01-01T00:00:00Z",
        verified: true,
      },
    ],
  };
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function fixture(name = FIXTURES[0][0]) {
  return readJson(path.join(FIXTURE_DIR, name));
}

function releaseEvidence(name = FIXTURES[0][0], tag = "v9.9.9-rc.1") {
  const packet = fixture(name);
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

  const expectedRole = PACKAGE_ROLES[packet.platform.package.kind];
  const assetName = expectedReleaseAssetRoles(tag).roles.find(
    ({ role }) => role === expectedRole,
  ).name;
  packet.platform.package.asset_name = assetName;
  packet.assets[0].name = assetName;
  packet.assets[0].public_url = `https://github.com/TheGreenCedar/BatCave/releases/download/${tag}/${encodeURIComponent(assetName)}`;
  for (const [kind, signature] of Object.entries(packet.assets[0].signatures)) {
    signature.identity = REAL_SIGNATURE_IDENTITIES[kind];
  }
  if (packet.assets[0].signatures.authenticode) {
    packet.assets[0].signatures.authenticode = authenticodeSignature(
      REAL_SIGNATURE_IDENTITIES.authenticode,
      packet.packet_kind,
      packet.assets[0],
    );
  }
  return packet;
}

function addRoleAsset(packet, role) {
  const declaration = expectedReleaseAssetRoles(packet.release.tag).roles.find(
    (candidate) => candidate.role === role,
  );
  const identities =
    packet.packet_kind === "schema_fixture"
      ? SYNTHETIC_SIGNATURE_IDENTITIES
      : REAL_SIGNATURE_IDENTITIES;
  const digit = String((packet.assets.length % 8) + 2);
  const asset = {
    name: declaration.name,
    size_bytes: 2_000_000 + packet.assets.length,
    sha256: `sha256:${digit.repeat(64)}`,
    api_digest: `sha256:${digit.repeat(64)}`,
    public_url: `https://github.com/TheGreenCedar/BatCave/releases/download/${packet.release.tag}/${encodeURIComponent(declaration.name)}`,
    attestation: structuredClone(packet.assets[0].attestation),
    signatures: {},
  };
  asset.signatures = Object.fromEntries(
    RELEASE_EVIDENCE_ROLE_TRUST[role].map((kind) => [
      kind,
      kind === "authenticode"
        ? authenticodeSignature(identities[kind], packet.packet_kind, asset)
        : { identity: identities[kind], verified: true },
    ]),
  );
  packet.assets.push(asset);
  packet.assets.sort((left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
  );
  return asset;
}

function reverseKeys(value) {
  return Object.fromEntries(Object.entries(value).reverse());
}

test("publishes a closed version 1 structural schema", () => {
  const schema = readJson(
    path.join(ROOT, "docs", "evidence", "releases", "release-evidence-packet.schema.json"),
  );
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(schema.properties.schema_version.const, 1);
  assert.equal(
    schema.$defs.release.properties.tag.pattern,
    "^v?[0-9]+\\.[0-9]+\\.[0-9]+(?:-[0-9A-Za-z.-]+)?$",
  );
  assert.equal(schema.$defs.platform.properties.support_contract_version.const, 1);
  assert.deepEqual(schema.$defs.platform.properties.profile_id.enum, Object.keys(REAL_HOSTS));
  assert.deepEqual(schema.$defs.platform.properties.proof.required, [
    "declaration",
    "source",
    "native",
  ]);
  assert.deepEqual(schema.$defs.platform.properties.proof.properties.source.enum, [
    "pending",
    "source_enforced",
  ]);
  assert.deepEqual(schema.$defs.platform.properties.runtime.required, ["libc_family"]);
  for (const definition of [
    schema,
    schema.$defs.release,
    schema.$defs.platform,
    schema.$defs.platform.properties.proof,
    schema.$defs.platform.properties.runtime,
    schema.$defs.asset,
    schema.$defs.check,
  ]) {
    assert.equal(definition.additionalProperties, false);
  }
});

for (const [file, os, packageKind] of FIXTURES) {
  test(`validates the synthetic ${os} ${packageKind} packet`, () => {
    const packet = fixture(file);
    assert.equal(validateReleaseEvidencePacket(packet), packet);
    assert.equal(packet.platform.os, os);
    assert.equal(packet.platform.package.kind, packageKind);
    assert.equal(packet.packet_kind, "schema_fixture");
  });
}

for (const [file, os, packageKind] of FIXTURES) {
  test(`validates a real-shaped ${os} ${packageKind} evidence path`, () => {
    const packet = releaseEvidence(file);
    assert.equal(validateReleaseEvidencePacket(packet), packet);
  });
}

test("binds every exact release role in a valid complete multi-asset packet", () => {
  const packet = fixture();
  const contract = expectedReleaseAssetRoles(packet.release.tag);
  const roles = contract.roles.map(({ role }) => role);
  assert.deepEqual(Object.keys(RELEASE_EVIDENCE_ROLE_TRUST).sort(), [...roles].sort());
  const existingNames = new Set(packet.assets.map(({ name }) => name));
  for (const declaration of contract.roles) {
    if (!existingNames.has(declaration.name)) addRoleAsset(packet, declaration.role);
  }
  assert.equal(packet.assets.length, roles.length);
  assert.equal(validateReleaseEvidencePacket(packet), packet);
});

test("accepts a real multi-asset packet with role-specific trust", () => {
  const packet = releaseEvidence();
  addRoleAsset(packet, "Windows GUI executable");
  addRoleAsset(packet, "checksum manifest");
  assert.equal(validateReleaseEvidencePacket(packet), packet);
});

test("rejects Apple notarization proof on the Windows GUI role", () => {
  const packet = fixture();
  const gui = addRoleAsset(packet, "Windows GUI executable");
  gui.signatures = {
    apple_notarization: {
      identity: SYNTHETIC_SIGNATURE_IDENTITIES.apple_notarization,
      verified: true,
    },
    authenticode: {
      ...authenticodeSignature(
        SYNTHETIC_SIGNATURE_IDENTITIES.authenticode,
        packet.packet_kind,
        gui,
      ),
    },
  };
  assert.throws(
    () => validateReleaseEvidencePacket(packet),
    /Windows GUI executable does not accept apple_notarization/u,
  );
});

test("requires explicit contained-app trust for the macOS DMG", () => {
  const packet = fixture("macos-dmg.json");
  delete packet.assets[0].signatures.contained_app_staple;
  assert.throws(() => validateReleaseEvidencePacket(packet), /requires contained_app_staple/u);
});

test("requires contained-app notarization and staple proof for the macOS updater", () => {
  const packet = fixture("macos-updater.json");
  delete packet.assets[0].signatures.contained_app_notarization;
  assert.throws(
    () => validateReleaseEvidencePacket(packet),
    /requires contained_app_notarization/u,
  );
});

test("requires one Developer ID authority for the DMG and its contained app", () => {
  const packet = releaseEvidence("macos-dmg.json");
  packet.assets[0].signatures.contained_app_developer_id.identity =
    "Developer ID Application: Other Publisher (KLMNOPQRST)";
  assert.throws(() => validateReleaseEvidencePacket(packet), /same Developer ID identity/u);
});

test("rejects case-colliding packet assets", () => {
  const packet = fixture();
  const collision = structuredClone(packet.assets[0]);
  collision.name = collision.name.toLowerCase();
  packet.assets.push(collision);
  assert.throws(() => validateReleaseEvidencePacket(packet), /case-collides/u);
});

test("rejects an arbitrary real signer identity and updater key", () => {
  const signerPacket = releaseEvidence();
  signerPacket.assets[0].signatures.authenticode.identity = "any signer";
  assert.throws(() => validateReleaseEvidencePacket(signerPacket), /trust subject/u);

  const updaterPacket = releaseEvidence("linux-appimage.json");
  updaterPacket.assets[0].signatures.tauri_updater.identity = `sha256:${"f".repeat(64)}`;
  assert.throws(() => validateReleaseEvidencePacket(updaterPacket), /embedded updater key/u);
});

test("rejects a BatCave publisher relabeled as preserved upstream code", () => {
  const packet = releaseEvidence();
  packet.assets[0].signatures.authenticode.files[0].disposition = "upstream_preserved";
  assert.throws(() => validateReleaseEvidencePacket(packet), /trusted upstream publisher/u);
});

test("records failed and blocked outcomes without declaring a release pass", () => {
  const packet = releaseEvidence();
  packet.checks.install.package_install = {
    status: "failed",
    outcome: "nonzero status",
  };
  packet.checks.runtime.launch = {
    status: "blocked",
    outcome: "native runner unavailable",
  };
  assert.equal(validateReleaseEvidencePacket(packet), packet);
});

const SEMANTIC_PLATFORM_FAILURES = [
  [
    "a reserved synthetic host on real evidence",
    (packet) => (packet.platform.os_version = "synthetic-windows-fixture"),
    /schema_fixture only/u,
  ],
  [
    "Windows Server evidence",
    (packet) => (packet.platform.os_version = "windows-server-10.0.20348"),
    /windows_client_build host/u,
  ],
  [
    "a below-floor Windows build",
    (packet) => (packet.platform.os_version = "windows-client-10.0.15063"),
    /below supported floor/u,
  ],
  [
    "source-only proof relabeled as native evidence",
    (packet) => (packet.platform.proof.native = "pending"),
    /proof.native: must equal observed/u,
  ],
  [
    "a musl Linux runtime",
    (packet) => (packet.platform.runtime.libc_family = "musl"),
    /libc_family: must equal glibc/u,
    "linux-appimage.json",
  ],
  [
    "an unknown Linux distribution",
    (packet) => (packet.platform.os_version = "fedora-41"),
    /ubuntu_release host/u,
    "linux-appimage.json",
  ],
];

for (const [name, mutate, expected, file] of SEMANTIC_PLATFORM_FAILURES) {
  test(`rejects ${name}`, () => {
    const packet = releaseEvidence(file);
    mutate(packet);
    assert.throws(() => validateReleaseEvidencePacket(packet), expected);
  });
}

test("pure packet validation accepts a parser-aligned tag without reading Cargo version", () => {
  const packet = releaseEvidence("windows-nsis.json", "9.9.9-rc.1");
  assert.equal(validateReleaseEvidencePacket(packet), packet);
});

const FIELD_FAILURES = [
  [
    "missing schema version",
    (packet) => delete packet.schema_version,
    /schema_version: is required/u,
  ],
  ["wrong schema version", (packet) => (packet.schema_version = 2), /must equal 1/u],
  ["unknown field", (packet) => (packet.extra = "unexpected"), /packet\.extra: is not allowed/u],
  ["malformed packet ID", (packet) => (packet.packet_id = "Windows Packet"), /packet\.packet_id/u],
  [
    "non-UTC time",
    (packet) => (packet.observed_at_utc = "2000-01-01 00:00:00Z"),
    /observed_at_utc/u,
  ],
  [
    "impossible time",
    (packet) => (packet.observed_at_utc = "2000-02-31T00:00:00Z"),
    /must be a real UTC time/u,
  ],
  ["missing release", (packet) => delete packet.release, /packet\.release: is required/u],
  [
    "wrong repository",
    (packet) => (packet.release.repository = "example/BatCave"),
    /release\.repository/u,
  ],
  ["malformed tag", (packet) => (packet.release.tag = "release-1"), /release\.tag/u],
  ["unsupported build metadata", (packet) => (packet.release.tag += "+build.1"), /release\.tag/u],
  ["wrong channel", (packet) => (packet.release.channel = "stable"), /release\.channel/u],
  [
    "missing source SHA",
    (packet) => delete packet.release.source_sha,
    /release\.source_sha: is required/u,
  ],
  [
    "malformed source SHA",
    (packet) => (packet.release.source_sha = "abc123"),
    /release\.source_sha/u,
  ],
  [
    "source mismatch",
    (packet) => (packet.release.main_sha = "4".repeat(40)),
    /must identify one commit/u,
  ],
  [
    "wrong release URL",
    (packet) => (packet.release.release_url = "https://github.com/example/release"),
    /release\.release_url/u,
  ],
  [
    "wrong workflow URL",
    (packet) => (packet.release.workflow_run.url += "-other"),
    /workflow_run\.url/u,
  ],
  [
    "fixture relabeled as evidence",
    (packet) => (packet.packet_kind = "release_evidence"),
    /reserved schema-fixture tag/u,
  ],
  ["missing asset", (packet) => (packet.assets = []), /packet\.assets: must be a non-empty array/u],
  [
    "digest mismatch",
    (packet) => (packet.assets[0].api_digest = `sha256:${"4".repeat(64)}`),
    /same public bytes/u,
  ],
  [
    "wrong asset URL",
    (packet) => (packet.assets[0].public_url += "-other"),
    /assets\[0\]\.public_url/u,
  ],
  [
    "missing attestation identity",
    (packet) => delete packet.assets[0].attestation.signer_workflow,
    /signer_workflow: is required/u,
  ],
  [
    "unverified attestation",
    (packet) => (packet.assets[0].attestation.verified = false),
    /attestation\.verified/u,
  ],
  [
    "missing signature identity",
    (packet) => delete packet.assets[0].signatures.authenticode.identity,
    /identity: is required/u,
  ],
  [
    "missing package signature",
    (packet) => delete packet.assets[0].signatures.authenticode,
    /requires authenticode/u,
  ],
  [
    "asset outside the exact release role inventory",
    (packet) => {
      packet.assets[0].name = "other-setup.exe";
      packet.assets[0].public_url =
        "https://github.com/TheGreenCedar/BatCave/releases/download/v0.0.0-evidence.1/other-setup.exe";
      packet.platform.package.asset_name = "other-setup.exe";
    },
    /exact release asset role/u,
  ],
  [
    "package kind bound to the wrong exact asset role",
    (packet) => {
      packet.assets[0].name = "batcave-monitor.exe";
      packet.assets[0].public_url =
        "https://github.com/TheGreenCedar/BatCave/releases/download/v0.0.0-evidence.1/batcave-monitor.exe";
      packet.assets[0].signatures.authenticode.files[0].name = "batcave-monitor.exe";
      delete packet.assets[0].signatures.tauri_updater;
      packet.platform.package.asset_name = "batcave-monitor.exe";
    },
    /must reference the Windows NSIS installer and updater payload asset/u,
  ],
  [
    "arbitrary signer identity",
    (packet) => (packet.assets[0].signatures.authenticode.identity = "any signer"),
    /schema fixture/u,
  ],
  [
    "trust proof from another asset role",
    (packet) => {
      packet.assets[0].signatures = {
        authenticode: {
          ...authenticodeSignature(
            "synthetic Authenticode signer fixture",
            packet.packet_kind,
            packet.assets[0],
          ),
        },
        ...packet.assets[0].signatures,
      };
    },
    /Linux AppImage package and updater payload does not accept authenticode/u,
    "linux-appimage.json",
  ],
  [
    "unsorted signatures",
    (packet) => (packet.assets[0].signatures = reverseKeys(packet.assets[0].signatures)),
    /signatures: must be strictly sorted/u,
  ],
  ["wrong package kind", (packet) => (packet.platform.package.kind = "dmg"), /package\.kind/u],
  [
    "missing install checks",
    (packet) => delete packet.checks.install,
    /checks\.install: is required/u,
  ],
  [
    "missing cleanup outcome",
    (packet) => delete packet.checks.cleanup.user_state_policy,
    /user_state_policy: is required/u,
  ],
  [
    "invalid runtime status",
    (packet) => (packet.checks.runtime.launch.status = "unknown"),
    /launch\.status/u,
  ],
  [
    "unsorted checks",
    (packet) => (packet.checks.runtime = reverseKeys(packet.checks.runtime)),
    /checks\.runtime: must be strictly sorted/u,
  ],
  [
    "missing limitations",
    (packet) => delete packet.limitations,
    /packet\.limitations: is required/u,
  ],
  [
    "missing limitation disposition",
    (packet) => delete packet.limitations.synthetic_fixture_no_release_claim.disposition,
    /disposition: is required/u,
  ],
  ["fixture missing non-claim", (packet) => (packet.limitations = {}), /not release evidence/u],
  [
    "fixture accepting its non-claim",
    (packet) => (packet.limitations.synthetic_fixture_no_release_claim.disposition = "accepted"),
    /cannot accept release claims/u,
  ],
  [
    "fixture claiming a passing proof",
    (packet) => (packet.checks.install.checksum.status = "passed"),
    /cannot claim proof/u,
  ],
  [
    "fixture accepting a package limitation",
    (packet) => (packet.limitations.deb_checksum_attestation_only.disposition = "accepted"),
    /cannot accept release claims/u,
    "linux-deb.json",
  ],
  [
    "Debian trust limitation",
    (packet) => delete packet.limitations.deb_checksum_attestation_only,
    /checksum-and-attestation-only/u,
    "linux-deb.json",
  ],
  [
    "unsorted limitations",
    (packet) => (packet.limitations = reverseKeys(packet.limitations)),
    /limitations: must be strictly sorted/u,
    "linux-deb.json",
  ],
];

for (const [name, mutate, expected, file] of FIELD_FAILURES) {
  test(`rejects ${name}`, () => {
    const packet = fixture(file);
    mutate(packet);
    assert.throws(() => validateReleaseEvidencePacket(packet), expected);
  });
}

const SENSITIVE_VALUES = [
  ["Unix path", ["", "Users", "example", "release.log"].join("/")],
  ["labeled Unix path", "path:/Users/example/release.log"],
  ["source-prefixed Unix path", "source:/Users/example/release.log"],
  ["cwd-prefixed Unix path", "cwd:/private/tmp/release.log"],
  ["Unicode Unix path", "/用户/发布/证据.log"],
  ["Windows path", ["C:", "Users", "example", "release.log"].join("\\")],
  ["home path", ["~", "release", "evidence.log"].join("/")],
  ["named home path", "~builder/release/evidence.log"],
  ["expanded home path", "$HOME/release/evidence.log"],
  ["PowerShell user profile", "$env:USERPROFILE\\release\\evidence.log"],
  ["PowerShell home", "$env:HOME/release/evidence.log"],
  ["PowerShell temp", "$env:TEMP\\release\\evidence.log"],
  ["PowerShell runner temp", "$env:RUNNER_TEMP/release/evidence.log"],
  ["cmd home path", "%HOMEDRIVE%%HOMEPATH%\\release\\evidence.log"],
  ["cmd temp path", "%TEMP%\\release\\evidence.log"],
  ["cmd runner temp path", "%RUNNER_TEMP%/release/evidence.log"],
  ["file URL", ["file:", "", "", "private", "evidence.log"].join("/")],
  ["GitHub token", ["ghp", "a".repeat(36)].join("_")],
  ["private key", ["-----BEGIN", "PRIVATE KEY-----"].join(" ")],
  ["environment assignment", `${["GH", "TOKEN"].join("_")}=${"x".repeat(24)}`],
  ["CI environment dump", "CI=true RUNNER_OS=macOS"],
  ["colon environment dump", "HOME:/Users/example"],
  ["equals environment dump", "HOME=/Users/example"],
  ["JSON environment dump", '{"HOME":"/Users/example"}'],
  ["generic environment cluster", "FOO=bar BAZ=qux"],
  ["generic JSON environment cluster", '{"FOO":"bar","BAZ":"qux"}'],
  ["authorization header", `Authorization: Bearer ${"a".repeat(32)}`],
  ["bearer token", `Bearer ${"b".repeat(32)}`],
  ["password assignment", "password=hunter2"],
  ["secret-key assignment", "secret-key:deadbeef"],
  ["raw multiline output", ["first line", "second line"].join("\n")],
];

const PUNCTUATED_SENSITIVE_VALUES = [
  ["parenthesized AWS secret", `(AWS_SECRET=${"a".repeat(24)})`],
  ["bracketed password", "[password:hunter2]"],
  ["braced AWS secret", `{AWS_SECRET=${"b".repeat(24)}}`],
  ["double-quoted AWS secret", `"AWS_SECRET=${"c".repeat(24)}"`],
  ["single-quoted password", "'password=hunter2'"],
  ["comma and semicolon secret", `,AWS_SECRET=${"d".repeat(24)};`],
  ["semicolon and comma password", ";password=hunter2,"],
  ["parenthesized Unicode path", "(/用户/发布/证据.log)"],
  ["bracketed POSIX path", "[/Users/example/release.log]"],
  ["braced Windows path", "{C:\\Users\\example\\release.log}"],
  ["parenthesized UNC path", "(\\\\server\\share\\release.log)"],
  ["punctuated named-home path", ",~builder/release/evidence.log;"],
  ["quoted field-prefixed path", "'cwd:/private/tmp/release.log'"],
];

const NESTED_SENSITIVE_ASSIGNMENTS = [
  ["nested direct AWS secret", `note=AWS_SECRET=${"e".repeat(24)}`],
  ["nested parenthesized AWS secret", `note=(AWS_SECRET=${"f".repeat(24)})`],
  ["nested bracketed password", "note=[password=hunter2]"],
  ["nested environment cluster", "note=(FOO=bar BAZ=qux)"],
  ["nested JSON environment cluster", 'note=({"FOO":"bar","BAZ":"qux"})'],
];

for (const [name, value] of [
  ...SENSITIVE_VALUES,
  ...PUNCTUATED_SENSITIVE_VALUES,
  ...NESTED_SENSITIVE_ASSIGNMENTS,
]) {
  test(`rejects ${name}`, () => {
    const packet = fixture();
    packet.checks.runtime.launch.outcome = value;
    assert.throws(() => validateReleaseEvidencePacket(packet), /forbidden/u);
  });
}

const VALID_OUTCOME_PROSE = [
  "theme=dark",
  "interval=1s",
  "mode=release",
  "status=failed",
  "theme=dark interval=1s mode=release status=failed",
  "(theme=dark)",
  "[interval=1s]",
  "{mode=release}",
  '"status=failed"',
  "'theme=dark'",
  ",theme=dark; interval=1s,",
  "note=(theme=dark), next=[status=failed]; mode={release}",
  "source=(https://github.com/TheGreenCedar/BatCave)",
  "note=(theme=dark)",
  "note=[interval=1s]",
  "note=({mode=release status=failed})",
  "note=(theme=dark interval=1s mode=release status=failed)",
  "password reset failed without exposing an assignment",
];

for (const value of VALID_OUTCOME_PROSE) {
  test(`accepts ordinary outcome prose: ${value}`, () => {
    const packet = fixture();
    packet.checks.runtime.launch.outcome = value;
    assert.equal(validateReleaseEvidencePacket(packet), packet);
  });
}

test("rejects sensitive and uncontrolled output fields", () => {
  for (const key of ["environment", "raw_logs", "stdout", "private_key_material", "secret-key"]) {
    const packet = fixture();
    packet[key] = "redacted";
    assert.throws(() => validateReleaseEvidencePacket(packet), /fields are forbidden/u);
  }
});

test("the CLI accepts every platform fixture", () => {
  const script = path.join(ROOT, "scripts", "validate-release-evidence-packet.mjs");
  const files = FIXTURES.map(([file]) => path.join(FIXTURE_DIR, file));
  const result = spawnSync(process.execPath, [script, ...files], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  for (const [, os] of FIXTURES)
    assert.match(result.stdout, new RegExp(`${os}.*schema-fixture`, "u"));
});

test("release and validation workflows run the evidence contract tests", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/validate-release-evidence-packet\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/validate-release-evidence-packet\.test\.mjs/gu)?.length,
    2,
  );
});
