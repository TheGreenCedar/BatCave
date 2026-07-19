import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  RELEASE_ASSET_PHASE,
  canonicalReleaseAssetName,
  expectedReleaseAssetRoles,
  requireSafeReleaseAssetName,
  verifyReleaseAssetInventory,
} from "./release-asset-contract.mjs";
import { parseReleaseTag, verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

const COMMIT_SHA = /^[0-9a-f]{40}$/;
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const UTC_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/u;
const FOUNDRY_CORE_NAME = "Microsoft.AI.Foundry.Local.Core.dll";
const FOUNDRY_CORE_SOURCE_SHA256 =
  "sha256:316a50a492180b192c2cae06f791bbe8c6e66c096a7415c642a599d1735666ea";

function exactObjectKeys(value, expected, owner) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${owner} must be an object`);
  }
  const actual = Object.keys(value);
  const missing = expected.find((key) => !actual.includes(key));
  const extra = actual.find((key) => !expected.includes(key));
  if (missing) throw new Error(`${owner}.${missing} is required`);
  if (extra) throw new Error(`${owner}.${extra} is not allowed`);
}

function requireCommitSha(name, value) {
  if (!COMMIT_SHA.test(value)) {
    throw new Error(
      `${name} must be an exact lowercase 40-character commit SHA; received ${value}`,
    );
  }
}

export function verifyReleaseCandidateIdentity({
  tag,
  channel,
  sourceSha,
  mainSha,
  approvedSourceSha,
}) {
  const { prerelease } = parseReleaseTag(tag);
  requireCommitSha("source SHA", sourceSha);
  requireCommitSha("origin/main SHA", mainSha);
  requireCommitSha("approved source SHA", approvedSourceSha);

  if (sourceSha !== mainSha) {
    throw new Error(`release source ${sourceSha} must equal origin/main ${mainSha}`);
  }
  if (sourceSha !== approvedSourceSha) {
    throw new Error(`release source ${sourceSha} must equal approved source ${approvedSourceSha}`);
  }

  const expectedChannel = prerelease ? "prerelease" : "stable";
  if (channel !== expectedChannel) {
    throw new Error(`channel ${channel} does not match tag ${tag}; expected ${expectedChannel}`);
  }

  return { tag, sourceSha, prerelease };
}

function releaseFiles(root) {
  const files = [];
  const visit = (directory) => {
    const entries = fs
      .readdirSync(directory, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const entryPath = path.join(directory, entry.name);
      if (entry.isSymbolicLink())
        throw new Error(`release input cannot contain symlinks: ${entryPath}`);
      if (entry.isDirectory()) visit(entryPath);
      else if (entry.isFile()) files.push(entryPath);
      else throw new Error(`release input must contain regular files only: ${entryPath}`);
    }
  };
  visit(root);
  return files;
}

export function stageReleaseAssets(inputRoot, outputRoot) {
  if (!fs.statSync(inputRoot).isDirectory()) {
    throw new Error(`release input is not a directory: ${inputRoot}`);
  }
  if (fs.existsSync(outputRoot) && fs.readdirSync(outputRoot).length > 0) {
    throw new Error(`release output must be empty: ${outputRoot}`);
  }
  const names = new Map();
  for (const source of releaseFiles(inputRoot)) {
    const name = requireSafeReleaseAssetName(path.basename(source).replaceAll(" ", "."));
    const canonicalName = canonicalReleaseAssetName(name);
    const prior = names.get(canonicalName);
    if (prior) {
      throw new Error(`release assets ${prior.source} and ${source} both normalize to ${name}`);
    }
    names.set(canonicalName, { name, source });
  }

  const staged = [...names.values()].map(({ name }) => name).sort((a, b) => a.localeCompare(b));
  if (staged.length === 0) throw new Error("release input contains no files");

  fs.mkdirSync(outputRoot, { recursive: true });
  for (const name of staged) {
    const source = names.get(canonicalReleaseAssetName(name)).source;
    fs.copyFileSync(source, path.join(outputRoot, name), fs.constants.COPYFILE_EXCL);
  }
  return staged;
}

function releaseDirectoryAssets(directory) {
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) {
    throw new Error("staged release directory must contain files only");
  }
  return entries
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((entry) => {
      const file = path.join(directory, entry.name);
      return {
        name: entry.name,
        size: fs.statSync(file).size,
        digest: `sha256:${crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex")}`,
      };
    });
}

export function verifyReleaseDirectory(
  tag,
  prerelease,
  directory,
  phase = RELEASE_ASSET_PHASE.Complete,
) {
  parseReleaseTag(tag);
  if (typeof prerelease !== "boolean") throw new Error("prerelease must be a boolean");
  const assets = releaseDirectoryAssets(directory);
  if (assets.length === 0) throw new Error("release candidate contains no assets");
  verifyReleaseAssetInventory(tag, prerelease, assets, `release ${phase} inventory`, phase);
  return assets;
}

export function validateWindowsSigningInventory(inventory, tag, sourceSha, assets) {
  exactObjectKeys(
    inventory,
    ["schema_version", "profile", "phase", "source_sha", "publisher", "timestamp", "files"],
    "windows signing inventory",
  );
  if (inventory.schema_version !== 1 || inventory.profile !== "production") {
    throw new Error("windows signing inventory must use schema 1 and the production profile");
  }
  if (inventory.phase !== "final" || inventory.source_sha !== sourceSha) {
    throw new Error("windows signing inventory must be final and match the release source SHA");
  }
  exactObjectKeys(inventory.publisher, ["display_name", "required_subject"], "publisher");
  if (
    inventory.publisher.display_name !== "Albert Najjar" ||
    inventory.publisher.required_subject !== "CN=Albert Najjar"
  ) {
    throw new Error("windows signing inventory has the wrong BatCave publisher contract");
  }
  exactObjectKeys(inventory.timestamp, ["protocol", "url", "digest"], "timestamp");
  if (
    inventory.timestamp.protocol !== "rfc3161" ||
    inventory.timestamp.url !== "http://timestamp.acs.microsoft.com/" ||
    inventory.timestamp.digest !== "SHA256"
  ) {
    throw new Error("windows signing inventory has the wrong RFC3161 timestamp contract");
  }
  if (!Array.isArray(inventory.files) || inventory.files.length < 9) {
    throw new Error("windows signing inventory must cover every shipped PE");
  }

  const expectedKeys = [
    "name",
    "sha256",
    "disposition",
    "original_sha256",
    "publisher_subject",
    "certificate_sha256",
    "rfc3161_timestamp_utc",
    "timestamp_certificate_sha256",
    "authenticode_status",
    "signtool_policy",
  ];
  const names = inventory.files.map(({ name }) => name);
  if (JSON.stringify(names) !== JSON.stringify([...names].sort((a, b) => a.localeCompare(b)))) {
    throw new Error("windows signing inventory files must be sorted by name");
  }
  if (new Set(names).size !== names.length) {
    throw new Error("windows signing inventory contains duplicate file names");
  }
  const upstream = new Set([
    "MicrosoftEdgeWebView2RuntimeInstaller.exe",
    "onnxruntime-genai.dll",
    "onnxruntime.dll",
  ]);
  let batcaveCertificate;
  for (const [index, record] of inventory.files.entries()) {
    exactObjectKeys(record, expectedKeys, `windows signing inventory.files[${index}]`);
    if (!SHA256_DIGEST.test(record.sha256) || !SHA256_DIGEST.test(record.certificate_sha256)) {
      throw new Error(`windows signing inventory record ${record.name} has an invalid SHA-256`);
    }
    if (!SHA256_DIGEST.test(record.timestamp_certificate_sha256)) {
      throw new Error(`windows signing inventory record ${record.name} has an invalid timestamper`);
    }
    if (record.original_sha256 !== null && !SHA256_DIGEST.test(record.original_sha256)) {
      throw new Error(`windows signing inventory record ${record.name} has an invalid source hash`);
    }
    if (!UTC_TIMESTAMP.test(record.rfc3161_timestamp_utc)) {
      throw new Error(`windows signing inventory record ${record.name} has an invalid timestamp`);
    }
    if (record.authenticode_status !== "valid" || record.signtool_policy !== "pa_all") {
      throw new Error(`windows signing inventory record ${record.name} is not fully verified`);
    }
    if (upstream.has(record.name)) {
      if (
        record.disposition !== "upstream_preserved" ||
        record.original_sha256 !== null ||
        !/^CN=Microsoft (?:Corporation|Windows)(?:,|$)/u.test(record.publisher_subject)
      ) {
        throw new Error(`upstream PE ${record.name} does not retain a trusted Microsoft signature`);
      }
    } else {
      if (record.name === FOUNDRY_CORE_NAME) {
        if (
          record.disposition !== "third_party_resigned" ||
          record.original_sha256 !== FOUNDRY_CORE_SOURCE_SHA256
        ) {
          throw new Error("Foundry Core must bind the exact unsigned SDK source before re-signing");
        }
      } else if (
        record.original_sha256 !== null ||
        !["batcave_signed", "generated_signed"].includes(record.disposition)
      ) {
        throw new Error(`BatCave PE ${record.name} has an invalid signing disposition`);
      }
      if (!/^CN=Albert Najjar(?:,|$)/u.test(record.publisher_subject)) {
        throw new Error(`BatCave PE ${record.name} has the wrong publisher or disposition`);
      }
      batcaveCertificate ??= record.certificate_sha256;
      if (record.certificate_sha256 !== batcaveCertificate) {
        throw new Error("BatCave PE files must use one Artifact Signing leaf certificate");
      }
    }
  }

  const contract = expectedReleaseAssetRoles(tag);
  for (const role of [
    "Windows GUI executable",
    "Windows CLI executable",
    "Windows NSIS installer and updater payload",
  ]) {
    const name = contract.roles.find((candidate) => candidate.role === role).name;
    const asset = assets.find((candidate) => candidate.name === name);
    const signed = inventory.files.find((candidate) => candidate.name === name);
    if (!asset || !signed || asset.digest !== signed.sha256) {
      throw new Error(`release asset ${name} does not match its Windows signing inventory`);
    }
  }
  for (const name of [
    "Microsoft.AI.Foundry.Local.Core.dll",
    "MicrosoftEdgeWebView2RuntimeInstaller.exe",
    "batcave-collector-service.exe",
    "batcave-monitor-cli.exe",
    "batcave-monitor.exe",
    "onnxruntime-genai.dll",
    "onnxruntime.dll",
    "uninstall.exe",
  ]) {
    if (!names.includes(name)) throw new Error(`windows signing inventory is missing ${name}`);
  }
  return inventory;
}

export function validateWindowsStorePreflightReceipt(receipt, tag, sourceSha, assets) {
  exactObjectKeys(
    receipt,
    ["schema_version", "qualification", "source_sha", "tag", "package", "limitation"],
    "windows Store preflight",
  );
  if (
    receipt.schema_version !== 1 ||
    receipt.qualification !== "source_preflight" ||
    receipt.source_sha !== sourceSha ||
    receipt.tag !== tag
  ) {
    throw new Error("windows Store preflight does not match the exact release candidate");
  }
  exactObjectKeys(
    receipt.package,
    [
      "name",
      "sha256",
      "url",
      "type",
      "architecture",
      "silent_parameters",
      "offline_installer",
      "authenticode",
    ],
    "windows Store preflight.package",
  );
  const installer = expectedReleaseAssetRoles(tag).roles.find(
    ({ role }) => role === "Windows NSIS installer and updater payload",
  );
  const asset = assets.find(({ name }) => name === installer.name);
  if (
    !asset ||
    receipt.package.name !== installer.name ||
    receipt.package.sha256 !== asset.digest
  ) {
    throw new Error("windows Store preflight package does not match the signed installer asset");
  }
  const expectedUrl = `https://github.com/TheGreenCedar/BatCave/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(installer.name)}`;
  if (receipt.package.url !== expectedUrl || receipt.package.url.includes("latest")) {
    throw new Error("windows Store preflight must use the immutable versioned release URL");
  }
  if (
    receipt.package.type !== "exe" ||
    receipt.package.architecture !== "x64" ||
    receipt.package.silent_parameters !== "/S" ||
    receipt.package.offline_installer !== true
  ) {
    throw new Error("windows Store preflight must retain the offline x64 EXE and /S contract");
  }
  exactObjectKeys(
    receipt.package.authenticode,
    ["publisher_subject", "certificate_sha256", "rfc3161_timestamp_utc"],
    "windows Store preflight.package.authenticode",
  );
  if (
    !/^CN=Albert Najjar(?:,|$)/u.test(receipt.package.authenticode.publisher_subject) ||
    !SHA256_DIGEST.test(receipt.package.authenticode.certificate_sha256) ||
    !UTC_TIMESTAMP.test(receipt.package.authenticode.rfc3161_timestamp_utc)
  ) {
    throw new Error("windows Store preflight is missing the verified publisher evidence");
  }
  if (receipt.limitation !== "native_clean_machine_silent_install_pending") {
    throw new Error("windows Store source preflight must retain its native-install limitation");
  }
  return receipt;
}

export function buildReleaseInventory(
  tag,
  sourceSha,
  prerelease,
  directory,
  windowsSigning,
  windowsStorePreflight,
) {
  parseReleaseTag(tag);
  requireCommitSha("source SHA", sourceSha);
  const assets = verifyReleaseDirectory(tag, prerelease, directory);
  const inventory = { tag, source_sha: sourceSha, prerelease, assets };
  if (windowsSigning !== undefined || windowsStorePreflight !== undefined) {
    if (windowsSigning === undefined || windowsStorePreflight === undefined) {
      throw new Error("Windows signing and Store preflight receipts must be supplied together");
    }
    inventory.windows_signing = validateWindowsSigningInventory(
      windowsSigning,
      tag,
      sourceSha,
      assets,
    );
    inventory.windows_store_preflight = validateWindowsStorePreflightReceipt(
      windowsStorePreflight,
      tag,
      sourceSha,
      assets,
    );
  }
  return inventory;
}

export function verifyReleaseReadback(expected, actual, expectedDraft) {
  verifyReleaseAssetInventory(
    expected.tag,
    expected.prerelease,
    expected.assets,
    "release candidate",
  );
  if (actual.tag_name !== expected.tag) {
    throw new Error(
      `release tag readback mismatch: expected ${expected.tag}, received ${actual.tag_name}`,
    );
  }
  if (actual.target_commitish !== expected.source_sha) {
    throw new Error(
      `release source readback mismatch: expected ${expected.source_sha}, received ${actual.target_commitish}`,
    );
  }
  if (actual.draft !== expectedDraft) {
    throw new Error(
      `release draft readback mismatch: expected ${expectedDraft}, received ${actual.draft}`,
    );
  }
  if (actual.prerelease !== expected.prerelease) {
    throw new Error(
      `release channel readback mismatch: expected prerelease=${expected.prerelease}, received ${actual.prerelease}`,
    );
  }
  const expectedImmutable = !expectedDraft;
  if (actual.immutable !== expectedImmutable) {
    throw new Error(
      `release immutable-state readback mismatch: expected ${expectedImmutable}, received ${actual.immutable}`,
    );
  }

  const actualAssets = (actual.assets ?? [])
    .map(({ name, size, digest }) => ({ name, size, digest }))
    .sort((a, b) => a.name.localeCompare(b.name));
  const duplicate = actualAssets.find(
    (asset, index) => asset.name === actualAssets[index - 1]?.name,
  );
  if (duplicate) throw new Error(`release readback contains duplicate asset ${duplicate.name}`);
  verifyReleaseAssetInventory(expected.tag, actual.prerelease, actualAssets, "release readback");
  if (JSON.stringify(actualAssets) !== JSON.stringify(expected.assets)) {
    throw new Error(
      `release asset readback mismatch\nexpected: ${JSON.stringify(
        expected.assets,
      )}\nreceived: ${JSON.stringify(actualAssets)}`,
    );
  }
  return true;
}

export function verifyLatestRelease(expected, latest) {
  if (expected.prerelease) {
    if (latest?.tag_name === expected.tag) {
      throw new Error(`prerelease ${expected.tag} must not become /releases/latest`);
    }
    return true;
  }

  if (!latest) throw new Error(`stable release ${expected.tag} is missing from /releases/latest`);
  if (latest.tag_name !== expected.tag) {
    throw new Error(
      `latest release mismatch: expected ${expected.tag}, received ${latest.tag_name}`,
    );
  }
  if (latest.target_commitish !== expected.source_sha) {
    throw new Error(
      `latest release source mismatch: expected ${expected.source_sha}, received ${latest.target_commitish}`,
    );
  }
  if (latest.draft !== false || latest.prerelease !== false || latest.immutable !== true) {
    throw new Error("latest stable release must be published, stable, and immutable");
  }
  return true;
}

function booleanArgument(value, name) {
  if (value === "true") return true;
  if (value === "false") return false;
  throw new Error(`${name} must be true or false; received ${value}`);
}

function usage() {
  return [
    "usage:",
    "  node scripts/verify-release-candidate.mjs identity <tag> <channel> <source-sha> <main-sha> <approved-source-sha>",
    "  node scripts/verify-release-candidate.mjs stage <input-directory> <output-directory>",
    "  node scripts/verify-release-candidate.mjs verify-inventory <tag> <prerelease> <phase> <directory>",
    "  node scripts/verify-release-candidate.mjs inventory <tag> <source-sha> <prerelease> <directory> <output-json> [<windows-signing-json> <windows-store-preflight-json>]",
    "  node scripts/verify-release-candidate.mjs verify-readback <expected-json> <actual-json> <draft>",
    "  node scripts/verify-release-candidate.mjs verify-latest <expected-json> <latest-json>",
  ].join("\n");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [command, ...args] = process.argv.slice(2);
  try {
    if (command === "identity" && args.length === 5) {
      const [tag, channel, sourceSha, mainSha, approvedSourceSha] = args;
      verifyWorkspaceReleaseVersion(tag);
      const candidate = verifyReleaseCandidateIdentity({
        tag,
        channel,
        sourceSha,
        mainSha,
        approvedSourceSha,
      });
      console.log(
        `release candidate identity verified: ${candidate.tag} at ${candidate.sourceSha}`,
      );
    } else if (command === "stage" && args.length === 2) {
      const assets = stageReleaseAssets(...args);
      console.log(`staged ${assets.length} release assets`);
    } else if (command === "verify-inventory" && args.length === 4) {
      const [tag, prerelease, phase, directory] = args;
      verifyWorkspaceReleaseVersion(tag);
      const assets = verifyReleaseDirectory(
        tag,
        booleanArgument(prerelease, "prerelease"),
        directory,
        phase,
      );
      console.log(`verified ${phase} release inventory for ${assets.length} assets`);
    } else if (command === "inventory" && [5, 7].includes(args.length)) {
      const [tag, sourceSha, prerelease, directory, output, signingFile, storeFile] = args;
      verifyWorkspaceReleaseVersion(tag);
      const inventory = buildReleaseInventory(
        tag,
        sourceSha,
        booleanArgument(prerelease, "prerelease"),
        directory,
        signingFile ? JSON.parse(fs.readFileSync(signingFile, "utf8")) : undefined,
        storeFile ? JSON.parse(fs.readFileSync(storeFile, "utf8")) : undefined,
      );
      fs.writeFileSync(output, `${JSON.stringify(inventory, null, 2)}\n`);
      console.log(`wrote release inventory for ${inventory.assets.length} assets`);
    } else if (command === "verify-readback" && args.length === 3) {
      const [expectedFile, actualFile, draft] = args;
      const expected = JSON.parse(fs.readFileSync(expectedFile, "utf8"));
      verifyWorkspaceReleaseVersion(expected.tag);
      verifyReleaseReadback(
        expected,
        JSON.parse(fs.readFileSync(actualFile, "utf8")),
        booleanArgument(draft, "draft"),
      );
      console.log("GitHub Release readback matches the local candidate");
    } else if (command === "verify-latest" && args.length === 2) {
      const [expectedFile, latestFile] = args;
      const expected = JSON.parse(fs.readFileSync(expectedFile, "utf8"));
      verifyWorkspaceReleaseVersion(expected.tag);
      verifyLatestRelease(expected, JSON.parse(fs.readFileSync(latestFile, "utf8")));
      console.log("GitHub latest-release semantics match the release channel");
    } else {
      console.error(usage());
      process.exit(2);
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
