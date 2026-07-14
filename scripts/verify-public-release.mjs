import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import {
  BUILD_PROVENANCE_ROLE,
  canonicalReleaseAssetName,
  requireSafeReleaseAssetName,
  verifyReleaseAssetInventory,
} from "./release-asset-contract.mjs";
import { parseReleaseTag, verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

export const RELEASE_REPOSITORY = "TheGreenCedar/BatCave";
export const RELEASE_SOURCE_REF = "refs/heads/main";
export const RELEASE_SIGNER_WORKFLOW = "TheGreenCedar/BatCave/.github/workflows/release.yml";
export const CHECKSUM_MANIFEST = "SHA256SUMS.txt";

const COMMIT_SHA = /^[0-9a-f]{40}$/;
const SHA256_DIGEST = /^sha256:([0-9a-f]{64})$/;
const verifiedPublicReleaseReceipts = new WeakSet();

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

function createVerifiedPublicReleaseReceipt(candidate, plan) {
  const { version } = parseReleaseTag(candidate.tag);
  const receipt = deepFreeze({
    schema_version: 1,
    verifier: "scripts/verify-public-release.mjs",
    disposition: "passed",
    repository: RELEASE_REPOSITORY,
    tag: candidate.tag,
    source_sha: candidate.source_sha,
    app_version: version,
    assets: plan.map(({ name, size, digest, url }) => ({
      name,
      size_bytes: size,
      sha256: digest,
      public_url: url,
    })),
  });
  verifiedPublicReleaseReceipts.add(receipt);
  return receipt;
}

export function requireVerifiedPublicReleaseReceipt(receipt) {
  if (!receipt || typeof receipt !== "object" || !verifiedPublicReleaseReceipts.has(receipt)) {
    throw new Error(
      "public verification receipt must come from a successful in-process verifyPublicRelease call",
    );
  }
  return receipt;
}

export function requireSafeAssetName(name) {
  return requireSafeReleaseAssetName(name);
}

function validatedAssets(assets, owner) {
  if (!Array.isArray(assets) || assets.length === 0) {
    throw new Error(`${owner} must contain at least one release asset`);
  }

  const names = new Map();
  return assets
    .map((asset) => {
      if (!asset || typeof asset !== "object") {
        throw new Error(`${owner} contains an invalid release asset`);
      }
      const name = requireSafeAssetName(asset.name);
      const canonicalName = canonicalReleaseAssetName(name);
      if (names.has(canonicalName)) {
        throw new Error(`${owner} contains duplicate release asset ${name}`);
      }
      names.set(canonicalName, name);

      if (!Number.isSafeInteger(asset.size) || asset.size < 0) {
        throw new Error(`${owner} asset ${name} has an invalid size`);
      }
      if (typeof asset.digest !== "string" || !SHA256_DIGEST.test(asset.digest)) {
        throw new Error(`${owner} asset ${name} must have an exact lowercase SHA-256 digest`);
      }
      return { ...asset, name };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

function expectedPublicUrl(tag, name) {
  return `https://github.com/${RELEASE_REPOSITORY}/releases/download/${encodeURIComponent(
    tag,
  )}/${encodeURIComponent(name)}`;
}

export function buildPublicDownloadPlan(candidate, release) {
  if (!candidate || typeof candidate !== "object") {
    throw new Error("release candidate inventory is invalid");
  }
  if (!release || typeof release !== "object") {
    throw new Error("published release readback is invalid");
  }
  if (typeof candidate.tag !== "string" || candidate.tag.length === 0) {
    throw new Error("release candidate tag is missing");
  }
  parseReleaseTag(candidate.tag);
  if (!COMMIT_SHA.test(candidate.source_sha)) {
    throw new Error(
      "release candidate source SHA must be an exact lowercase 40-character commit SHA",
    );
  }
  if (typeof candidate.prerelease !== "boolean") {
    throw new Error("release candidate prerelease state must be a boolean");
  }

  const expectedAssets = validatedAssets(candidate.assets, "release candidate");
  const publicAssets = validatedAssets(release.assets, "published release");
  verifyReleaseAssetInventory(
    candidate.tag,
    candidate.prerelease,
    expectedAssets,
    "release candidate",
  );
  verifyReleaseAssetInventory(candidate.tag, release.prerelease, publicAssets, "published release");

  if (release.tag_name !== candidate.tag) {
    throw new Error(
      `published release tag mismatch: expected ${candidate.tag}, received ${release.tag_name}`,
    );
  }
  if (release.target_commitish !== candidate.source_sha) {
    throw new Error(
      `published release source mismatch: expected ${candidate.source_sha}, received ${release.target_commitish}`,
    );
  }
  if (release.draft !== false || release.immutable !== true) {
    throw new Error("published release must be non-draft and immutable");
  }
  if (release.prerelease !== candidate.prerelease) {
    throw new Error("published release channel does not match the candidate");
  }

  const actualByName = new Map(publicAssets.map((asset) => [asset.name, asset]));
  const expectedNames = new Set(expectedAssets.map((asset) => asset.name));
  const missing = expectedAssets.filter((asset) => !actualByName.has(asset.name));
  const unexpected = publicAssets.filter((asset) => !expectedNames.has(asset.name));
  if (missing.length > 0) {
    throw new Error(`published release is missing asset ${missing[0].name}`);
  }
  if (unexpected.length > 0) {
    throw new Error(`published release contains unexpected asset ${unexpected[0].name}`);
  }

  return expectedAssets.map((expected) => {
    const actual = actualByName.get(expected.name);
    if (actual.size !== expected.size) {
      throw new Error(`published release asset ${expected.name} size does not match the candidate`);
    }
    if (actual.digest !== expected.digest) {
      throw new Error(
        `published release asset ${expected.name} digest does not match the candidate`,
      );
    }
    const expectedUrl = expectedPublicUrl(candidate.tag, expected.name);
    if (actual.browser_download_url !== expectedUrl) {
      throw new Error(`published release asset ${expected.name} has an unexpected download URL`);
    }
    return { ...expected, url: expectedUrl };
  });
}

function createFreshDirectory(directory) {
  if (fs.existsSync(directory)) {
    throw new Error(`public release download directory must not already exist: ${directory}`);
  }
  fs.mkdirSync(directory, { mode: 0o700 });
}

async function writePublicResponse(response, asset, destination) {
  if (!response || response.ok !== true) {
    const status = response?.status ?? "unknown";
    throw new Error(`anonymous download failed for ${asset.name}: HTTP ${status}`);
  }
  if (!response.body) {
    if (asset.size !== 0) {
      throw new Error(`anonymous download returned no body for ${asset.name}`);
    }
    fs.writeFileSync(destination, "", { flag: "wx", mode: 0o600 });
    return;
  }

  let bytesWritten = 0;
  const sizeGate = new Transform({
    transform(chunk, _encoding, callback) {
      bytesWritten += chunk.length;
      if (bytesWritten > asset.size) {
        callback(new Error(`public asset ${asset.name} exceeded the candidate size`));
      } else {
        callback(null, chunk);
      }
    },
  });
  try {
    await pipeline(
      Readable.fromWeb(response.body),
      sizeGate,
      fs.createWriteStream(destination, { flags: "wx", mode: 0o600 }),
    );
    if (bytesWritten !== asset.size) {
      throw new Error(`public asset ${asset.name} size does not match the candidate`);
    }
  } catch (error) {
    fs.rmSync(destination, { force: true });
    throw error;
  }
}

export async function downloadPublicAssets(plan, directory, fetchImpl = globalThis.fetch) {
  if (typeof fetchImpl !== "function") {
    throw new Error("anonymous public release downloader is unavailable");
  }
  createFreshDirectory(directory);

  for (const asset of plan) {
    const response = await fetchImpl(asset.url, {
      redirect: "follow",
      credentials: "omit",
      headers: {},
    });
    await writePublicResponse(response, asset, path.join(directory, asset.name));
  }
}

async function sha256File(file) {
  const hash = crypto.createHash("sha256");
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

export async function verifyDownloadedAssets(candidate, directory) {
  const expected = validatedAssets(candidate.assets, "release candidate");
  verifyReleaseAssetInventory(candidate.tag, candidate.prerelease, expected, "release candidate");
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  const actual = validatedAssets(
    entries.map((entry) => ({ name: entry.name, size: 0, digest: `sha256:${"0".repeat(64)}` })),
    "public download directory",
  );
  if (entries.some((entry) => !entry.isFile())) {
    throw new Error("public download directory must contain regular files only");
  }

  const expectedNames = expected.map((asset) => asset.name);
  const actualNames = actual.map((asset) => asset.name);
  if (JSON.stringify(actualNames) !== JSON.stringify(expectedNames)) {
    throw new Error(
      `public download asset set mismatch: expected ${expectedNames.join(", ")}; received ${actualNames.join(", ")}`,
    );
  }

  for (const asset of expected) {
    const file = path.join(directory, asset.name);
    const stat = fs.statSync(file);
    if (stat.size !== asset.size) {
      throw new Error(`public asset ${asset.name} size does not match the candidate`);
    }
    const digest = `sha256:${await sha256File(file)}`;
    if (digest !== asset.digest) {
      throw new Error(`public asset ${asset.name} digest does not match the candidate`);
    }
  }
  return expected;
}

export function parseChecksumManifest(contents) {
  if (typeof contents !== "string" || contents.length === 0 || !contents.endsWith("\n")) {
    throw new Error("checksum manifest must be non-empty and end with a newline");
  }

  const lines = contents.slice(0, -1).split("\n");
  const checksums = new Map();
  const canonicalNames = new Set();
  for (const [index, line] of lines.entries()) {
    const match = /^([0-9a-f]{64}) ([ *])\.\/(.+)$/u.exec(line);
    if (!match) {
      throw new Error(`invalid checksum manifest line ${index + 1}`);
    }
    const name = requireSafeAssetName(match[3]);
    const canonicalName = canonicalReleaseAssetName(name);
    if (canonicalNames.has(canonicalName)) {
      throw new Error(`checksum manifest contains duplicate asset ${name}`);
    }
    canonicalNames.add(canonicalName);
    checksums.set(name, match[1]);
  }
  if (checksums.size === 0) throw new Error("checksum manifest contains no subjects");
  return checksums;
}

export function verifyChecksumManifest(candidate, directory) {
  const assets = validatedAssets(candidate.assets, "release candidate");
  const contract = verifyReleaseAssetInventory(
    candidate.tag,
    candidate.prerelease,
    assets,
    "release candidate",
  );
  const provenance = contract.roles.find(({ role }) => role === BUILD_PROVENANCE_ROLE);
  if (!provenance) {
    throw new Error(`release contract is missing the declared ${BUILD_PROVENANCE_ROLE}`);
  }
  const byName = new Map(assets.map((asset) => [asset.name, asset]));
  if (!byName.has(CHECKSUM_MANIFEST)) {
    throw new Error(`release candidate is missing ${CHECKSUM_MANIFEST}`);
  }

  const manifest = parseChecksumManifest(
    fs.readFileSync(path.join(directory, CHECKSUM_MANIFEST), "utf8"),
  );
  if (manifest.has(CHECKSUM_MANIFEST)) {
    throw new Error(`${CHECKSUM_MANIFEST} cannot checksum itself`);
  }

  for (const [name, digest] of manifest) {
    const asset = byName.get(name);
    if (!asset) {
      throw new Error(`checksum manifest contains unexpected subject ${name}`);
    }
    if (asset.digest !== `sha256:${digest}`) {
      throw new Error(`checksum manifest digest does not match candidate subject ${name}`);
    }
  }

  const unchecksummed = assets
    .map((asset) => asset.name)
    .filter((name) => !manifest.has(name) && name !== CHECKSUM_MANIFEST);
  if (unchecksummed.length !== 1) {
    throw new Error(
      `checksum manifest must cover every candidate subject except one public attestation bundle; found ${unchecksummed.length} exceptions`,
    );
  }
  if (unchecksummed[0] !== provenance.name) {
    throw new Error(
      `checksum manifest must leave only the declared ${BUILD_PROVENANCE_ROLE} unchecksummed; found ${unchecksummed[0]}`,
    );
  }

  return {
    subjects: [...manifest.keys()].sort((left, right) => left.localeCompare(right)),
    bundleName: provenance.name,
  };
}

export function releaseVerificationArguments(tag) {
  parseReleaseTag(tag);
  return ["release", "verify", tag, "--repo", RELEASE_REPOSITORY, "--format", "json"];
}

export function attestationVerificationArguments(file, bundle, sourceSha) {
  if (!COMMIT_SHA.test(sourceSha)) {
    throw new Error("attestation source SHA must be an exact lowercase 40-character commit SHA");
  }
  return [
    "attestation",
    "verify",
    path.resolve(file),
    "--bundle",
    path.resolve(bundle),
    "--repo",
    RELEASE_REPOSITORY,
    "--source-digest",
    sourceSha,
    "--source-ref",
    RELEASE_SOURCE_REF,
    "--signer-workflow",
    RELEASE_SIGNER_WORKFLOW,
    "--deny-self-hosted-runners",
  ];
}

function runGh(arguments_) {
  const result = spawnSync("gh", arguments_, { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`gh ${arguments_.slice(0, 2).join(" ")} failed with status ${result.status}`);
  }
}

export function runGitHubVerifications(candidate, proof, directory, runner = runGh) {
  runner(releaseVerificationArguments(candidate.tag));
  const bundle = path.join(directory, proof.bundleName);
  for (const subject of proof.subjects) {
    runner(
      attestationVerificationArguments(path.join(directory, subject), bundle, candidate.source_sha),
    );
  }
}

export async function verifyPublicRelease(
  candidate,
  release,
  directory,
  { fetchImpl = globalThis.fetch, ghRunner = runGh } = {},
) {
  const plan = buildPublicDownloadPlan(candidate, release);
  await downloadPublicAssets(plan, directory, fetchImpl);
  await verifyDownloadedAssets(candidate, directory);
  const proof = verifyChecksumManifest(candidate, directory);
  runGitHubVerifications(candidate, proof, directory, ghRunner);
  return {
    assetCount: plan.length,
    subjectCount: proof.subjects.length,
    receipt: createVerifiedPublicReleaseReceipt(candidate, plan),
  };
}

function readJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`could not read ${label} ${file}: ${error.message}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [candidateFile, releaseFile, downloadDirectory] = process.argv.slice(2);
  if (!candidateFile || !releaseFile || !downloadDirectory) {
    console.error(
      "usage: node scripts/verify-public-release.mjs <candidate-json> <published-release-json> <new-download-directory>",
    );
    process.exit(2);
  }

  try {
    const candidate = readJson(candidateFile, "release candidate");
    verifyWorkspaceReleaseVersion(candidate.tag);
    const result = await verifyPublicRelease(
      candidate,
      readJson(releaseFile, "published release readback"),
      downloadDirectory,
    );
    console.log(
      `verified ${result.assetCount} anonymous public assets and ${result.subjectCount} source-bound attestations`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
