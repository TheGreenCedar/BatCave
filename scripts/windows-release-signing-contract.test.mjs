import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildWindowsStorePreflightReceipt,
  validateWindowsStoreListing,
} from "./validate-windows-store-preflight.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const json = (file) => JSON.parse(read(file));
const sourceSha = "0123456789abcdef0123456789abcdef01234567";

test("pins the OIDC identity and every new signing tool input", () => {
  const contract = json("scripts/windows-signing/artifact-signing-contract.v1.json");
  assert.equal(contract.publisher.subject, "CN=Albert Najjar");
  assert.equal(contract.oidc.environment, "release");
  assert.equal(contract.oidc.federated_subject, "repo:TheGreenCedar/BatCave:environment:release");
  assert.equal(contract.oidc.required_role, "Artifact Signing Certificate Profile Signer");
  assert.match(contract.oidc.azure_login.commit, /^[0-9a-f]{40}$/u);
  assert.deepEqual(
    contract.client_packages.map(({ id, version, sha256 }) => [id, version, sha256.length]),
    [
      ["Microsoft.ArtifactSigning.Client", "1.0.128", 64],
      ["Microsoft.Windows.SDK.BuildTools", "10.0.26100.8249", 64],
    ],
  );
  assert.equal(contract.timestamp.protocol, "rfc3161");
  assert.equal(contract.timestamp.url, "http://timestamp.acs.microsoft.com/");
  assert.deepEqual(contract.third_party_resigned_files, [
    {
      name: "Microsoft.AI.Foundry.Local.Core.dll",
      source: "foundry-local-sdk 1.2.0",
      source_sha256: "316a50a492180b192c2cae06f791bbe8c6e66c096a7415c642a599d1735666ea",
    },
  ]);
  assert.ok(!contract.upstream_files.includes("Microsoft.AI.Foundry.Local.Core.dll"));
});

test("keeps the local signing test profile isolated from production evidence", () => {
  const testProfile = read("scripts/test-windows-signing-profile.ps1");
  const candidateVerifier = read("scripts/verify-release-candidate.mjs");
  const validationWorkflow = read(".github/workflows/validation.yml");
  assert.match(testProfile, /CN=BatCave Artifact Signing Test/u);
  assert.match(testProfile, /New-SelfSignedCertificate/u);
  assert.match(testProfile, /byte-tampered signing test fixture/u);
  assert.match(testProfile, /Cert:\\CurrentUser\\My/u);
  assert.match(testProfile, /Remove-Item -LiteralPath "Cert:\\CurrentUser\\My/u);
  assert.match(testProfile, /certutil\.exe -user -f -addstore Root/u);
  assert.match(testProfile, /certutil\.exe -user -delstore Root/u);
  assert.doesNotMatch(testProfile, /X509Store/u);
  assert.match(testProfile, /exact unsigned Foundry SDK payload/u);
  assert.doesNotMatch(testProfile, /timestamp\.acs\.microsoft\.com/u);
  assert.match(
    validationWorkflow,
    /test-windows-signing-profile\.ps1[\s\S]*-SignToolPath \$env:BATCAVE_SIGNTOOL_PATH[\s\S]*-ThirdPartyInputPath/u,
  );
  assert.match(candidateVerifier, /inventory\.profile !== "production"/u);
});

test("keeps Artifact Signing release-only and preserves the required byte order", () => {
  const workflow = read(".github/workflows/release.yml");
  const releaseConfig = json("src/BatCave.App/src-tauri/tauri.windows.release.conf.json");
  const normalConfig = json("src/BatCave.App/src-tauri/tauri.windows.conf.json");
  const signer = read("src/BatCave.App/src-tauri/windows/sign-artifact.ps1");
  const builder = read("scripts/build-signed-windows-release.ps1");
  const inventoryWriter = read("scripts/write-windows-signature-inventory.ps1");
  const metadata = read("scripts/prepare-artifact-signing-metadata.ps1");

  assert.equal(normalConfig.bundle.windows.nsis.compression, "none");
  assert.equal(normalConfig.bundle.windows.webviewInstallMode.type, "offlineInstaller");
  assert.equal(normalConfig.bundle.windows.signCommand, undefined);
  assert.equal(releaseConfig.bundle.createUpdaterArtifacts, false);
  assert.match(releaseConfig.bundle.windows.signCommand, /windows\/sign-artifact\.ps1/u);
  assert.match(workflow, /permissions:\n\s+contents: read\n\s+id-token: write/u);
  assert.match(workflow, /uses: Azure\/login@532459ea530d8321f2fb9bb10d1e0bcf23869a43 # v3\.0\.0/u);
  assert.match(workflow, /environment: release/u);
  assert.doesNotMatch(workflow, /AZURE_CLIENT_SECRET/u);
  assert.match(metadata, /AzureCliCredential-only authentication/u);
  assert.doesNotMatch(metadata, /^\s*"AzureCliCredential"\s*$/mu);
  assert.doesNotMatch(metadata, /ClientSecretCredential|AZURE_CLIENT_SECRET/u);

  const build = builder.indexOf('"build", "--no-bundle"');
  const sourceHash = builder.indexOf("Foundry Core source digest");
  const sourceSignature = builder.indexOf("Foundry Core re-signing is allowed");
  const sign = builder.indexOf("foreach ($file in @($innerOwned + $thirdPartyResigned))");
  const bundle = builder.indexOf('"bundle", "--config"');
  const updater = builder.indexOf('"signer", "sign", $installer');
  const inventory = builder.indexOf("-Phase final");
  assert.ok(
    build >= 0 &&
      build < sourceHash &&
      sourceHash < sourceSignature &&
      sourceSignature < sign &&
      sign < bundle &&
      bundle < updater &&
      updater < inventory,
  );
  assert.match(signer, /\/tr "http:\/\/timestamp\.acs\.microsoft\.com\/" \/td SHA256/u);
  assert.match(signer, /Refusing to replace an existing unexpected signature/u);
  assert.doesNotMatch(signer, /\bexit\s+0\b/u);
  assert.match(inventoryWriter, /return "third_party_resigned"/u);
  assert.match(inventoryWriter, /original_sha256 = \$originalSha256/u);
  assert.match(inventoryWriter, /has invalid Authenticode status/u);
  assert.match(builder, /Byte-tampered installer unexpectedly passed/u);
});

test("builds an immutable Store receipt from the exact signed installer", () => {
  const listing = validateWindowsStoreListing(json("docs/store/windows-listing.v1.json"));
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-store-preflight-"));
  try {
    const tag = "v9.9.9-rc.1";
    const name = "BatCave.Monitor_9.9.9-rc.1_x64-setup.exe";
    const installer = path.join(root, name);
    const bytes = Buffer.from("signed installer fixture");
    fs.writeFileSync(installer, bytes);
    const digest = `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
    const inventory = {
      source_sha: sourceSha,
      profile: "production",
      phase: "final",
      files: [
        {
          name,
          sha256: digest,
          disposition: "generated_signed",
          publisher_subject: "CN=Albert Najjar",
          certificate_sha256: `sha256:${"a".repeat(64)}`,
          rfc3161_timestamp_utc: "2026-07-19T18:00:00Z",
        },
      ],
    };
    const receipt = buildWindowsStorePreflightReceipt(listing, tag, installer, inventory);
    assert.equal(
      receipt.package.url,
      `https://github.com/TheGreenCedar/BatCave/releases/download/v9.9.9-rc.1/${name}`,
    );
    assert.equal(receipt.package.silent_parameters, "/S");
    assert.equal(receipt.limitation, "native_clean_machine_silent_install_pending");

    fs.appendFileSync(installer, "tamper");
    assert.throws(
      () => buildWindowsStorePreflightReceipt(listing, tag, installer, inventory),
      /does not match its final Authenticode inventory/u,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("rejects mutable Store URLs and silent-install drift", () => {
  const listing = json("docs/store/windows-listing.v1.json");
  listing.distribution.versioned_url_template =
    "https://github.com/TheGreenCedar/BatCave/releases/latest/download/setup.exe";
  assert.throws(() => validateWindowsStoreListing(listing), /versioned URL template/u);

  const silent = json("docs/store/windows-listing.v1.json");
  silent.distribution.silent_parameters = "/quiet";
  assert.throws(() => validateWindowsStoreListing(silent), /must be \/S/u);
});
