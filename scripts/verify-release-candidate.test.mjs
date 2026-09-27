import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readCargoVersion } from "./verify-release-version.mjs";
import {
  BUILD_PROVENANCE_ROLE,
  RELEASE_ASSET_PHASE,
  expectedReleaseAssetRoles,
  verifyReleaseAssetInventory,
} from "./release-asset-contract.mjs";
import {
  buildReleaseInventory,
  stageReleaseAssets,
  verifyReleaseCandidateIdentity,
  verifyReleaseDirectory,
  verifyLatestRelease,
  verifyReleaseReadback,
} from "./verify-release-candidate.mjs";

const sourceSha = "0123456789abcdef0123456789abcdef01234567";
const stableTag = "v0.3.0";

function digest(contents) {
  return `sha256:${crypto.createHash("sha256").update(contents).digest("hex")}`;
}

function assetContents(name) {
  return `fixture bytes for ${name}\n`;
}

function contractAssets(tag = stableTag) {
  return expectedReleaseAssetRoles(tag)
    .roles.map(({ name }) => ({
      name,
      size: Buffer.byteLength(assetContents(name)),
      digest: digest(assetContents(name)),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

function candidateFixture(tag = stableTag) {
  const { prerelease } = expectedReleaseAssetRoles(tag);
  return { tag, source_sha: sourceSha, prerelease, assets: contractAssets(tag) };
}

function windowsSigningFixture(tag = stableTag) {
  const contract = expectedReleaseAssetRoles(tag);
  const assetByName = new Map(contractAssets(tag).map((asset) => [asset.name, asset]));
  const installer = contract.roles.find(
    ({ role }) => role === "Windows NSIS installer and updater payload",
  ).name;
  const certificate = `sha256:${"b".repeat(64)}`;
  const upstream = new Set([
    "MicrosoftEdgeWebView2RuntimeInstaller.exe",
    "onnxruntime-genai.dll",
    "onnxruntime.dll",
  ]);
  const files = [
    "Microsoft.AI.Foundry.Local.Core.dll",
    "MicrosoftEdgeWebView2RuntimeInstaller.exe",
    "batcave-collector-service.exe",
    "batcave-monitor-cli.exe",
    "batcave-monitor.exe",
    "onnxruntime-genai.dll",
    "onnxruntime.dll",
    "uninstall.exe",
    installer,
  ]
    .sort((left, right) => left.localeCompare(right))
    .map((name) => ({
      name,
      sha256: assetByName.get(name)?.digest ?? digest(`signed ${name}`),
      disposition:
        name === "Microsoft.AI.Foundry.Local.Core.dll"
          ? "third_party_resigned"
          : upstream.has(name)
            ? "upstream_preserved"
            : name === installer || name === "uninstall.exe"
              ? "generated_signed"
              : "batcave_signed",
      original_sha256:
        name === "Microsoft.AI.Foundry.Local.Core.dll"
          ? "sha256:316a50a492180b192c2cae06f791bbe8c6e66c096a7415c642a599d1735666ea"
          : null,
      publisher_subject: upstream.has(name) ? "CN=Microsoft Corporation" : "CN=Albert Najjar",
      certificate_sha256: upstream.has(name) ? `sha256:${"c".repeat(64)}` : certificate,
      rfc3161_timestamp_utc: "2026-07-19T18:00:00Z",
      timestamp_certificate_sha256: `sha256:${"d".repeat(64)}`,
      authenticode_status: "valid",
      signtool_policy: "pa_all",
    }));
  return {
    schema_version: 1,
    profile: "production",
    phase: "final",
    source_sha: sourceSha,
    publisher: { display_name: "Albert Najjar", required_subject: "CN=Albert Najjar" },
    timestamp: {
      protocol: "rfc3161",
      url: "http://timestamp.acs.microsoft.com/",
      digest: "SHA256",
    },
    files,
  };
}

function windowsStoreFixture(tag = stableTag) {
  const signing = windowsSigningFixture(tag);
  const installer = signing.files.find(({ name }) => name.endsWith("_x64-setup.exe"));
  return {
    schema_version: 1,
    qualification: "source_preflight",
    source_sha: sourceSha,
    tag,
    package: {
      name: installer.name,
      sha256: installer.sha256,
      url: `https://github.com/TheGreenCedar/BatCave/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(installer.name)}`,
      type: "exe",
      architecture: "x64",
      silent_parameters: "/S",
      offline_installer: true,
      authenticode: {
        publisher_subject: installer.publisher_subject,
        certificate_sha256: installer.certificate_sha256,
        rfc3161_timestamp_utc: installer.rfc3161_timestamp_utc,
      },
    },
    limitation: "native_clean_machine_silent_install_pending",
  };
}

test("accepts an explicitly approved main-tip candidate", () => {
  assert.deepEqual(
    verifyReleaseCandidateIdentity({
      tag: "v0.3.0-rc.1",
      channel: "prerelease",
      sourceSha,
      mainSha: sourceSha,
      approvedSourceSha: sourceSha,
    }),
    { tag: "v0.3.0-rc.1", sourceSha, prerelease: true },
  );
});

test("rejects candidates that are only ancestors of main or were not explicitly approved", () => {
  const otherSha = "abcdef0123456789abcdef0123456789abcdef01";
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: stableTag,
        channel: "stable",
        sourceSha,
        mainSha: otherSha,
        approvedSourceSha: sourceSha,
      }),
    /must equal origin\/main/,
  );
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: stableTag,
        channel: "stable",
        sourceSha,
        mainSha: sourceSha,
        approvedSourceSha: otherSha,
      }),
    /must equal approved source/,
  );
});

test("rejects a release channel that disagrees with the tag", () => {
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: "v0.3.0-rc.1",
        channel: "stable",
        sourceSha,
        mainSha: sourceSha,
        approvedSourceSha: sourceSha,
      }),
    /expected prerelease/,
  );
});

test("executable candidate commands bind their tag to Cargo before artifact work", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-candidate-version-"));
  try {
    const script = fileURLToPath(new URL("./verify-release-candidate.mjs", import.meta.url));
    const expectedFile = path.join(root, "candidate.json");
    fs.writeFileSync(expectedFile, `${JSON.stringify({ tag: "v9.9.9" })}\n`);
    const commands = [
      ["identity", "v9.9.9", "stable", sourceSha, sourceSha, sourceSha],
      ["verify-inventory", "v9.9.9", "false", "pre-attestation", path.join(root, "missing")],
      ["inventory", "v9.9.9", sourceSha, "false", path.join(root, "missing"), "output.json"],
      ["verify-readback", expectedFile, path.join(root, "missing-readback.json"), "true"],
      ["verify-latest", expectedFile, path.join(root, "missing-latest.json")],
    ];
    for (const command of commands) {
      const result = spawnSync(process.execPath, [script, ...command], { encoding: "utf8" });
      assert.equal(result.status, 1, command[0]);
      assert.match(result.stderr, /release tag v9\.9\.9 expects version 9\.9\.9/u);
      assert.match(result.stderr, /Cargo\.toml:/u);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("stages isolated platform packets without silent basename collisions", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-release-stage-"));
  try {
    const input = path.join(root, "input");
    const output = path.join(root, "output");
    fs.mkdirSync(path.join(input, "windows"), { recursive: true });
    fs.mkdirSync(path.join(input, "macos"), { recursive: true });
    fs.writeFileSync(path.join(input, "windows", "BatCave Setup.exe"), "windows");
    fs.writeFileSync(path.join(input, "macos", "BatCave.dmg"), "macos");

    assert.deepEqual(stageReleaseAssets(input, output), ["BatCave.dmg", "BatCave.Setup.exe"]);
    assert.equal(fs.readFileSync(path.join(output, "BatCave.Setup.exe"), "utf8"), "windows");

    const collisionInput = path.join(root, "collision");
    fs.mkdirSync(path.join(collisionInput, "one"), { recursive: true });
    fs.mkdirSync(path.join(collisionInput, "two"), { recursive: true });
    fs.writeFileSync(path.join(collisionInput, "one", "same name"), "one");
    fs.writeFileSync(path.join(collisionInput, "two", "same.name"), "two");
    assert.throws(
      () => stageReleaseAssets(collisionInput, path.join(root, "collision-output")),
      /both normalize to same\.name/,
    );

    const caseCollisionInput = path.join(root, "case-collision");
    fs.mkdirSync(path.join(caseCollisionInput, "one"), { recursive: true });
    fs.mkdirSync(path.join(caseCollisionInput, "two"), { recursive: true });
    fs.writeFileSync(path.join(caseCollisionInput, "one", "Asset.bin"), "one");
    fs.writeFileSync(path.join(caseCollisionInput, "two", "asset.bin"), "two");
    assert.throws(
      () => stageReleaseAssets(caseCollisionInput, path.join(root, "case-collision-output")),
      /both normalize to asset\.bin/,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("declares the exact stable and prerelease release matrices", () => {
  const stable = expectedReleaseAssetRoles(stableTag);
  const prerelease = expectedReleaseAssetRoles("v0.3.0-rc.1");

  assert.equal(stable.prerelease, false);
  assert.equal(prerelease.prerelease, true);
  assert.deepEqual(
    stable.roles.map(({ name }) => name),
    [
      "batcave-monitor.exe",
      "batcave-monitor-cli.exe",
      "BatCave.Monitor_0.3.0_x64-setup.exe",
      "BatCave.Monitor_0.3.0_x64-setup.exe.sig",
      "BatCave.Monitor_0.3.0_amd64.deb",
      "BatCave.Monitor_0.3.0_amd64.AppImage",
      "BatCave.Monitor_0.3.0_amd64.AppImage.sig",
      "BatCave.Monitor_0.3.0_aarch64.dmg",
      "BatCave.Monitor.app.tar.gz",
      "BatCave.Monitor.app.tar.gz.sig",
      "latest.json",
      "SHA256SUMS.txt",
      "BatCave-v0.3.0-provenance.json",
    ],
  );
  assert.ok(prerelease.roles.some(({ name }) => name === "BatCave.Monitor_0.3.0-rc.1_amd64.deb"));
  assert.ok(prerelease.roles.some(({ name }) => name === "BatCave-v0.3.0-rc.1-provenance.json"));
});

test("rejects inventory channel state that disagrees with stable and prerelease tags", () => {
  assert.throws(
    () => verifyReleaseAssetInventory(stableTag, true, contractAssets(stableTag), "candidate"),
    /expected stable/,
  );
  assert.throws(
    () =>
      verifyReleaseAssetInventory("v0.3.0-rc.1", false, contractAssets("v0.3.0-rc.1"), "candidate"),
    /expected prerelease/,
  );
});

test("validates the exact pre-attestation phase without weakening the 13-role contract", () => {
  const contract = expectedReleaseAssetRoles(stableTag);
  const provenance = contract.roles.find(({ role }) => role === BUILD_PROVENANCE_ROLE);
  const preAttestationAssets = contractAssets().filter(({ name }) => name !== provenance.name);

  assert.equal(contract.roles.length, 13);
  assert.equal(
    verifyReleaseAssetInventory(
      stableTag,
      false,
      preAttestationAssets,
      "pre-attestation candidate",
      RELEASE_ASSET_PHASE.PreAttestation,
    ).roles.length,
    13,
  );
  assert.throws(
    () => verifyReleaseAssetInventory(stableTag, false, preAttestationAssets, "complete candidate"),
    /missing required build provenance bundle/,
  );
  assert.throws(
    () =>
      verifyReleaseAssetInventory(
        stableTag,
        false,
        contractAssets(),
        "pre-attestation candidate",
        RELEASE_ASSET_PHASE.PreAttestation,
      ),
    /unexpected asset BatCave-v0\.3\.0-provenance\.json/,
  );
  assert.throws(
    () =>
      verifyReleaseAssetInventory(
        stableTag,
        false,
        preAttestationAssets.filter(({ name }) => name !== "SHA256SUMS.txt"),
        "pre-attestation candidate",
        RELEASE_ASSET_PHASE.PreAttestation,
      ),
    /missing required checksum manifest/,
  );
});

for (const missingRole of expectedReleaseAssetRoles(stableTag).roles) {
  test(`rejects a missing ${missingRole.role}`, () => {
    const assets = contractAssets().filter(({ name }) => name !== missingRole.name);
    assert.throws(() => verifyReleaseAssetInventory(stableTag, false, assets, "candidate"));
  });
}

test("rejects duplicate roles, duplicate basenames, and unexpected assets", () => {
  const assets = contractAssets();
  assert.throws(
    () =>
      verifyReleaseAssetInventory(
        stableTag,
        false,
        [
          ...assets,
          {
            name: "BatCave.Monitor_9.9.9_amd64.deb",
            size: 1,
            digest: digest("x"),
          },
        ],
        "candidate",
      ),
    /duplicate Linux deb package/,
  );
  assert.throws(
    () =>
      verifyReleaseAssetInventory(
        stableTag,
        false,
        [...assets, { ...assets[0], name: assets[0].name.toUpperCase() }],
        "candidate",
      ),
    /duplicate basename/,
  );
  assert.throws(
    () =>
      verifyReleaseAssetInventory(
        stableTag,
        false,
        [...assets, { name: "notes.txt", size: 1, digest: digest("x") }],
        "candidate",
      ),
    /unexpected asset notes\.txt/,
  );
});

const wrongVersionCases = [
  ["Windows NSIS installer and updater payload", "BatCave.Monitor_9.9.9_x64-setup.exe"],
  ["Linux deb package", "BatCave.Monitor_9.9.9_amd64.deb"],
  ["Linux AppImage package and updater payload", "BatCave.Monitor_9.9.9_amd64.AppImage"],
  ["macOS Apple Silicon DMG", "BatCave.Monitor_9.9.9_aarch64.dmg"],
  ["build provenance bundle", "BatCave-v9.9.9-provenance.json"],
];

for (const [roleName, wrongName] of wrongVersionCases) {
  test(`rejects a wrong-version ${roleName}`, () => {
    const contract = expectedReleaseAssetRoles(stableTag);
    const role = contract.roles.find(({ role }) => role === roleName);
    const signature = contract.roles.find(({ signatureFor }) => signatureFor === roleName);
    const removed = new Set([role.name, signature?.name].filter(Boolean));
    const assets = contractAssets()
      .filter(({ name }) => !removed.has(name))
      .concat({ name: wrongName, size: 1, digest: digest("x") });
    if (signature) {
      assets.push({ name: `${wrongName}.sig`, size: 1, digest: digest("signature") });
    }
    assert.throws(
      () => verifyReleaseAssetInventory(stableTag, false, assets, "candidate"),
      /wrong filename/,
    );
  });
}

for (const signature of expectedReleaseAssetRoles(stableTag).roles.filter(
  ({ signatureFor }) => signatureFor,
)) {
  test(`rejects an orphan ${signature.role}`, () => {
    const payload = expectedReleaseAssetRoles(stableTag).roles.find(
      ({ role }) => role === signature.signatureFor,
    );
    const assets = contractAssets().filter(({ name }) => name !== payload.name);
    assert.throws(
      () => verifyReleaseAssetInventory(stableTag, false, assets, "candidate"),
      /orphan signature/,
    );
  });
}

test("builds a deterministic exact name, size, and digest inventory", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-release-inventory-"));
  try {
    const contract = expectedReleaseAssetRoles(stableTag);
    const provenance = contract.roles.find(({ role }) => role === BUILD_PROVENANCE_ROLE);
    const preAttestation = contractAssets().filter(({ name }) => name !== provenance.name);
    for (const { name } of preAttestation) {
      fs.writeFileSync(path.join(root, name), assetContents(name));
    }
    assert.equal(
      verifyReleaseDirectory(stableTag, false, root, RELEASE_ASSET_PHASE.PreAttestation).length,
      12,
    );

    fs.writeFileSync(path.join(root, provenance.name), assetContents(provenance.name));
    assert.deepEqual(buildReleaseInventory(stableTag, sourceSha, false, root), candidateFixture());
    const signing = windowsSigningFixture();
    const store = windowsStoreFixture();
    const signedInventory = buildReleaseInventory(
      stableTag,
      sourceSha,
      false,
      root,
      "azure",
      signing,
      store,
    );
    assert.deepEqual(signedInventory.windows_signing, signing);
    assert.deepEqual(signedInventory.windows_store_preflight, store);

    const unsignedPe = structuredClone(signing);
    unsignedPe.files.find(({ name }) => name === "batcave-monitor.exe").authenticode_status =
      "not_signed";
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", unsignedPe, store),
      /not fully verified/u,
    );
    const changedFoundrySource = structuredClone(signing);
    changedFoundrySource.files.find(
      ({ name }) => name === "Microsoft.AI.Foundry.Local.Core.dll",
    ).original_sha256 = `sha256:${"f".repeat(64)}`;
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", changedFoundrySource, store),
      /exact unsigned SDK source/u,
    );
    const falselyPreservedFoundry = structuredClone(signing);
    falselyPreservedFoundry.files.find(
      ({ name }) => name === "Microsoft.AI.Foundry.Local.Core.dll",
    ).disposition = "upstream_preserved";
    assert.throws(
      () =>
        buildReleaseInventory(stableTag, sourceSha, false, root, "azure", falselyPreservedFoundry, store),
      /exact unsigned SDK source/u,
    );
    const unrelatedException = structuredClone(signing);
    const unrelated = unrelatedException.files.find(({ name }) => name === "batcave-monitor.exe");
    unrelated.disposition = "third_party_resigned";
    unrelated.original_sha256 =
      "sha256:316a50a492180b192c2cae06f791bbe8c6e66c096a7415c642a599d1735666ea";
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", unrelatedException, store),
      /invalid signing disposition/u,
    );
    const mutableUrl = structuredClone(store);
    mutableUrl.package.url =
      "https://github.com/TheGreenCedar/BatCave/releases/latest/download/setup.exe";
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", signing, mutableUrl),
      /immutable versioned release URL/u,
    );

    fs.writeFileSync(path.join(root, "unexpected.bin"), "unexpected");
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root),
      /unexpected asset unexpected\.bin/,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Azure mode requires final publisher evidence without making Store submission a prerequisite", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-azure-inventory-"));
  try {
    for (const { name } of contractAssets()) fs.writeFileSync(path.join(root, name), assetContents(name));
    const signing = windowsSigningFixture();
    const store = windowsStoreFixture();
    const signed = buildReleaseInventory(stableTag, sourceSha, false, root, "azure", signing);
    assert.deepEqual(signed.windows_signing, signing);
    assert.equal(Object.hasOwn(signed, "windows_store_preflight"), false);
    assert.deepEqual(buildReleaseInventory(stableTag, sourceSha, false, root, "unsigned"), candidateFixture());
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure"),
      /requires a final production signing inventory/u,
    );
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", undefined, store),
      /requires a final production signing inventory/u,
    );
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "unsigned", signing),
      /unsigned Windows mode cannot include/u,
    );
    assert.throws(
      () => buildReleaseInventory(stableTag, sourceSha, false, root, "unsigned", undefined, store),
      /unsigned Windows mode cannot include/u,
    );
    for (const mode of ["", "Azure", "signed", "test", null]) {
      assert.throws(() => buildReleaseInventory(stableTag, sourceSha, false, root, mode, signing), /unknown Windows signing mode/u);
    }
    for (const [mutate, error] of [
      [s => s.profile = "test", /production profile/u],
      [s => s.phase = "inner", /must be final/u],
      [s => s.source_sha = "f".repeat(40), /match the release source SHA/u],
      [s => s.publisher.required_subject = "CN=Someone Else", /publisher contract/u],
      [s => s.files.pop(), /cover every shipped PE/u],
      [s => s.files.find(f => f.name === "batcave-monitor.exe").publisher_subject = "CN=Someone Else", /wrong publisher/u],
      [s => s.files.find(f => f.name === "batcave-monitor.exe").rfc3161_timestamp_utc = null, /invalid timestamp/u],
      [s => s.files.find(f => f.name === "batcave-monitor.exe").certificate_sha256 = `sha256:${"e".repeat(64)}`, /one Artifact Signing leaf/u],
      [s => s.files.find(f => f.name === "batcave-monitor.exe").sha256 = `sha256:${"e".repeat(64)}`, /does not match its Windows signing inventory/u],
    ]) {
      const invalid = structuredClone(signing);
      mutate(invalid);
      assert.throws(() => buildReleaseInventory(stableTag, sourceSha, false, root, "azure", invalid), error);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("inventory CLI retains signing-only evidence and rejects a missing selected Azure inventory", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-azure-cli-"));
  try {
    const repo = fileURLToPath(new URL("../", import.meta.url));
    const tag = `v${readCargoVersion(repo)}`;
    const script = fileURLToPath(new URL("./verify-release-candidate.mjs", import.meta.url));
    const assets = path.join(root, "assets");
    fs.mkdirSync(assets);
    for (const { name } of contractAssets(tag)) fs.writeFileSync(path.join(assets, name), assetContents(name));
    const signing = windowsSigningFixture(tag);
    const signingFile = path.join(root, "signing.json");
    fs.writeFileSync(signingFile, JSON.stringify(signing));
    const output = path.join(root, "candidate.json");
    const args = [script, "inventory", tag, sourceSha, "false", assets, output];
    for (const mode of [[], ["unsigned", ""], ["azure", signingFile]]) {
      const result = spawnSync(process.execPath, [...args, ...mode], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr);
      const candidate = JSON.parse(fs.readFileSync(output, "utf8"));
      assert.deepEqual(candidate.assets, contractAssets(tag));
      assert.deepEqual(candidate.windows_signing, mode[0] === "azure" ? signing : undefined);
      assert.equal(Object.hasOwn(candidate, "windows_store_preflight"), false);
    }
    fs.unlinkSync(output);
    for (const mode of [["azure"], ["azure", ""], ["unsigned", signingFile], ["Azure", signingFile]]) {
      const result = spawnSync(process.execPath, [...args, ...mode], { encoding: "utf8" });
      assert.equal(result.status, 1, result.stderr);
      assert.match(result.stderr, /requires a final production signing inventory|unsigned Windows mode cannot include|unknown Windows signing mode/u);
      assert.equal(fs.existsSync(output), false);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("verifies draft and published release readback against the same exact candidate", () => {
  const expected = candidateFixture();
  const actual = {
    tag_name: stableTag,
    target_commitish: sourceSha,
    draft: true,
    prerelease: false,
    immutable: false,
    assets: expected.assets.map((asset, id) => ({ ...asset, id })),
  };
  assert.equal(verifyReleaseReadback(expected, actual, true), true);
  assert.equal(
    verifyReleaseReadback(expected, { ...actual, draft: false, immutable: true }, false),
    true,
  );
});

test("rejects source, state, and asset drift in the prepublication readback", () => {
  const expected = candidateFixture();
  const actual = {
    tag_name: stableTag,
    target_commitish: sourceSha,
    draft: true,
    prerelease: false,
    immutable: false,
    assets: expected.assets,
  };
  assert.throws(
    () => verifyReleaseReadback(expected, { ...actual, target_commitish: "main" }, true),
    /source readback mismatch/,
  );
  assert.throws(
    () => verifyReleaseReadback(expected, { ...actual, draft: false, immutable: true }, true),
    /draft readback mismatch/,
  );
  assert.throws(() => verifyReleaseReadback(expected, actual, false), /draft readback mismatch/);
  assert.throws(
    () => verifyReleaseReadback(expected, { ...actual, draft: false }, false),
    /immutable-state readback mismatch/,
  );
  assert.throws(
    () =>
      verifyReleaseReadback(
        expected,
        {
          ...actual,
          assets: actual.assets.map((asset, index) =>
            index === 0 ? { ...asset, size: asset.size + 1, digest: digest("drift") } : asset,
          ),
        },
        true,
      ),
    /asset readback mismatch/,
  );
});

test("requires stable releases to become latest and prereleases to remain non-latest", () => {
  const stable = { tag: stableTag, source_sha: sourceSha, prerelease: false };
  const latest = {
    tag_name: stable.tag,
    target_commitish: sourceSha,
    draft: false,
    prerelease: false,
    immutable: true,
  };
  assert.equal(verifyLatestRelease(stable, latest), true);
  assert.throws(() => verifyLatestRelease(stable, null), /missing from \/releases\/latest/);
  assert.throws(
    () => verifyLatestRelease(stable, { ...latest, tag_name: "v0.2.0" }),
    /latest release mismatch/,
  );

  const prerelease = { ...stable, tag: "v0.4.0-rc.1", prerelease: true };
  assert.equal(verifyLatestRelease(prerelease, latest), true);
  assert.equal(verifyLatestRelease(prerelease, null), true);
  assert.throws(
    () => verifyLatestRelease(prerelease, { ...latest, tag_name: prerelease.tag }),
    /must not become \/releases\/latest/,
  );
});
