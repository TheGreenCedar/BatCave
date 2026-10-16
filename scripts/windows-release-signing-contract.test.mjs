import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildWindowsStorePreflightReceipt,
  validateWindowsStoreListing,
} from "./validate-windows-store-preflight.mjs";
import { validateWindowsSigningInventory } from "./verify-release-candidate.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const json = (file) => JSON.parse(read(file));
const sourceSha = "0123456789abcdef0123456789abcdef01234567";

test("production inventory binds the raw spaced NSIS installer to its canonical release name", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-signature-inventory-"));
  try {
    const tag = "v0.2.1-rc.1";
    const rawInstallerName = "BatCave Monitor_0.2.1-rc.1_x64-setup.exe";
    const installerName = "BatCave.Monitor_0.2.1-rc.1_x64-setup.exe";
    const contract = json("scripts/windows-signing/artifact-signing-contract.v1.json");
    const names = [
      ...contract.batcave_owned_files,
      ...contract.upstream_files,
      ...contract.third_party_resigned_files.map(({ name }) => name),
      rawInstallerName,
    ];
    for (const name of names) fs.writeFileSync(path.join(root, name), `fixture bytes for ${name}`);
    const inputs = path.join(root, "inputs.json");
    fs.writeFileSync(inputs, JSON.stringify(names.map((name) => path.join(root, name))));
    const tool = path.join(root, "signtool.ps1");
    fs.writeFileSync(tool, `
[IO.File]::AppendAllLines($env:TEST_TOOL_CALLS, [string[]]@($args[-1]))
$global:LASTEXITCODE = $(if ($env:TEST_REJECT_TRUST -eq 'true') { 1 } else { 0 })
'The signature is timestamped: 10/16/2026 12:00:00 PM +00:00'
`);
    const harness = path.join(root, "inventory-test.ps1");
    fs.writeFileSync(harness, `
$ErrorActionPreference = 'Stop'
Set-Variable IsWindows $true -Force
if (-not ('System.Management.Automation.SignatureStatus' -as [type])) {
    Add-Type 'namespace System.Management.Automation { public enum SignatureStatus { Valid, NotSigned, HashMismatch } }'
}
$key = [Security.Cryptography.RSA]::Create(2048)
function New-TestCertificate([string]$Subject) {
    $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
        $Subject, $key, [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1)
    return $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddDays(-1), [DateTimeOffset]::UtcNow.AddDays(1))
}
$batcave = New-TestCertificate 'CN=Albert Najjar'
$microsoft = New-TestCertificate 'CN=Microsoft Corporation'
function Get-AuthenticodeSignature([string]$LiteralPath) {
    [IO.File]::AppendAllLines($env:TEST_SIGNATURE_CALLS, [string[]]@($LiteralPath))
    $name = [IO.Path]::GetFileName($LiteralPath)
    $upstream = $name -in @('onnxruntime.dll', 'onnxruntime-genai.dll', 'MicrosoftEdgeWebView2RuntimeInstaller.exe')
    $installer = $name -like 'BatCave*setup.exe'
    return [pscustomobject]@{
        Status = $(if ($installer -and $env:TEST_REJECT_SIGNATURE -eq 'true') {
            [System.Management.Automation.SignatureStatus]::HashMismatch
        } else { [System.Management.Automation.SignatureStatus]::Valid })
        SignerCertificate = $(if ($upstream -or ($installer -and $env:TEST_REJECT_PUBLISHER -eq 'true')) { $microsoft } else { $batcave })
        TimeStamperCertificate = $microsoft
    }
}
try {
    $paths = @(Get-Content -LiteralPath $env:TEST_INPUTS -Raw | ConvertFrom-Json)
    & $env:TEST_WRITER -Phase final -Path $paths -OutputPath $env:TEST_OUTPUT
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
} finally {
    $batcave.Dispose()
    $microsoft.Dispose()
    $key.Dispose()
}
`);
    const output = path.join(root, "inventory.json");
    const signatureCalls = path.join(root, "signature-calls.txt");
    const toolCalls = path.join(root, "tool-calls.txt");
    const run = (overrides = {}) => spawnSync(
      process.env.PWSH_PATH || "pwsh",
      ["-NoProfile", "-NonInteractive", "-File", harness],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          BATCAVE_WINDOWS_SIGNING_PROFILE: "production",
          BATCAVE_SOURCE_COMMIT_SHA: sourceSha,
          BATCAVE_SIGNTOOL_PATH: tool,
          TEST_WRITER: path.join(ROOT, "scripts/write-windows-signature-inventory.ps1"),
          TEST_INPUTS: inputs,
          TEST_OUTPUT: output,
          TEST_SIGNATURE_CALLS: signatureCalls,
          TEST_TOOL_CALLS: toolCalls,
          ...overrides,
        },
      },
    );
    const result = run();
    assert.equal(result.status, 0, result.stderr || result.error?.message);
    const inventory = JSON.parse(fs.readFileSync(output, "utf8"));
    const digest = (name) => `sha256:${crypto.createHash("sha256").update(fs.readFileSync(path.join(root, name))).digest("hex")}`;
    const installer = inventory.files.find(({ name }) => name === installerName);
    assert.ok(installer, "inventory must use the public release name");
    assert.equal(installer.sha256, digest(rawInstallerName));
    assert.equal(installer.publisher_subject, "CN=Albert Najjar");
    assert.equal(fs.existsSync(path.join(root, installerName)), false);
    for (const log of [signatureCalls, toolCalls]) {
      assert.deepEqual(fs.readFileSync(log, "utf8").trim().split(/\r?\n/u).sort(),
        names.map((name) => path.join(root, name)).sort());
    }
    const assets = [
      [installerName, rawInstallerName],
      ["batcave-monitor.exe", "batcave-monitor.exe"],
      ["batcave-monitor-cli.exe", "batcave-monitor-cli.exe"],
    ].map(([name, rawName]) => ({ name, digest: digest(rawName) }));
    assert.equal(validateWindowsSigningInventory(inventory, tag, sourceSha, assets), inventory);
    for (const [input, message] of [
      ["TEST_REJECT_SIGNATURE", /has invalid Authenticode status HashMismatch/u],
      ["TEST_REJECT_PUBLISHER", /has unexpected publisher CN=Microsoft Corporation/u],
      ["TEST_REJECT_TRUST", /SignTool trust verification failed/u],
    ]) {
      const rejected = run({ [input]: "true" });
      assert.equal(rejected.status, 1, rejected.stderr);
      assert.match(rejected.stderr, message);
    }
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

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
  assert.match(testProfile, /SignatureStatus\]::NotTrusted/u);
  assert.match(testProfile, /SignatureStatus\]::HashMismatch/u);
  assert.match(testProfile, /\$global:LASTEXITCODE = 0/u);
  assert.doesNotMatch(testProfile, /-addstore Root/u);
  assert.doesNotMatch(testProfile, /-delstore Root/u);
  assert.doesNotMatch(testProfile, /X509Store/u);
  assert.match(testProfile, /exact unsigned Foundry SDK payload/u);
  assert.doesNotMatch(testProfile, /timestamp\.acs\.microsoft\.com/u);
  assert.match(
    validationWorkflow,
    /test-windows-signing-profile\.ps1[\s\S]*-SignToolPath \$env:BATCAVE_SIGNTOOL_PATH[\s\S]*-ThirdPartyInputPath/u,
  );
  assert.match(candidateVerifier, /inventory\.profile !== "production"/u);
});

test("keeps optional Artifact Signing isolated and preserves its byte order", () => {
  const workflow = read(".github/workflows/release.yml");
  const releaseConfig = json("src/BatCave.App/src-tauri/tauri.windows.release.conf.json");
  const normalConfig = json("src/BatCave.App/src-tauri/tauri.windows.conf.json");
  const signer = read("src/BatCave.App/src-tauri/windows/sign-artifact.ps1");
  const builder = read("scripts/build-signed-windows-release.ps1");
  const inventoryWriter = read("scripts/write-windows-signature-inventory.ps1");
  const metadata = read("scripts/prepare-artifact-signing-metadata.ps1");
  const unsignedJob = workflow.slice(workflow.indexOf("\n  windows:"), workflow.indexOf("\n  windows_signed:"));
  const signedJob = workflow.slice(workflow.indexOf("\n  windows_signed:"), workflow.indexOf("\n  linux:"));
  const signingContract = json("scripts/windows-signing/artifact-signing-contract.v1.json");

  assert.equal(normalConfig.bundle.windows.nsis.compression, "none");
  assert.equal(normalConfig.bundle.windows.webviewInstallMode.type, "offlineInstaller");
  assert.equal(normalConfig.bundle.windows.signCommand, undefined);
  assert.equal(releaseConfig.bundle.createUpdaterArtifacts, false);
  assert.match(releaseConfig.bundle.windows.signCommand, /windows\/sign-artifact\.ps1/u);
  assert.doesNotMatch(unsignedJob, /Azure\/login|AZURE_|BATCAVE_WINDOWS_SIGNING_PROFILE|windows-signing-evidence|environment:|id-token:/u);
  assert.match(unsignedJob, /if: needs.prepare.outputs.windows_signing == 'unsigned'/u);
  assert.match(unsignedJob, /Bundle Windows with updater signing/u);
  assert.match(unsignedJob, /tauri -- build --config src-tauri\/tauri.updater.conf.json --ci/u);
  assert.match(unsignedJob, /Verify finalized Windows updater signature[\s\S]*--bin batcave-verify-updater-signature/u);
  assert.match(signedJob, /if: needs.prepare.outputs.windows_signing == 'azure'/u);
  assert.match(signedJob, /environment: release/u);
  assert.match(signedJob, /permissions:\s+contents: read\s+id-token: write/u);
  assert.ok(signedJob.includes(`Azure/login@${signingContract.oidc.azure_login.commit}`));
  assert.match(signedJob, /BATCAVE_WINDOWS_SIGNING_PROFILE: production/u);
  assert.ok(signedJob.indexOf("Validate protected Azure signing inputs") < signedJob.indexOf("Azure/login@"));
  assert.ok(signedJob.indexOf("Azure/login@") < signedJob.indexOf("prepare-artifact-signing-metadata.ps1"));
  const signedBuild = signedJob.indexOf("run: ./scripts/build-signed-windows-release.ps1");
  const generatedContract = signedJob.indexOf("npm run verify:windows-installer-generated");
  const finalizedSignature = signedJob.indexOf("--bin batcave-verify-updater-signature");
  const collect = signedJob.indexOf("name: Collect Windows distributables");
  assert.ok(signedBuild > 0 && signedBuild < generatedContract && generatedContract < finalizedSignature && finalizedSignature < collect);
  assert.doesNotMatch(signedJob.slice(finalizedSignature, collect), /signtool\s+sign|tauri\s+(?:build|bundle)|sign-artifact\.ps1/u);
  assert.match(signedJob, /name: windows-signing-evidence-\$\{\{ needs.prepare.outputs.tag \}\}/u);
  assert.doesNotMatch(signedJob, /store-preflight|AZURE_CLIENT_SECRET|ADMIN_READ/u);
  assert.match(workflow, /Windows downloads are not Authenticode-signed/u);
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
