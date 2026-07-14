import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  CHECKSUM_MANIFEST,
  RELEASE_REPOSITORY,
  RELEASE_SIGNER_WORKFLOW,
  RELEASE_SOURCE_REF,
  attestationVerificationArguments,
  buildPublicDownloadPlan,
  downloadPublicAssets,
  parseChecksumManifest,
  releaseVerificationArguments,
  runGitHubVerifications,
  verifyChecksumManifest,
  verifyDownloadedAssets,
  verifyPublicRelease,
} from "./verify-public-release.mjs";

const sourceSha = "0123456789abcdef0123456789abcdef01234567";
const tag = "v0.3.0";
const contract = expectedReleaseAssetRoles(tag);
const provenanceName = contract.roles.find(({ role }) => role === "build provenance bundle").name;
const subjectNames = contract.roles
  .map(({ name }) => name)
  .filter((name) => name !== CHECKSUM_MANIFEST && name !== provenanceName)
  .sort((left, right) => left.localeCompare(right));
const testSubject = "batcave-monitor.exe";

function digest(contents) {
  return `sha256:${crypto.createHash("sha256").update(contents).digest("hex")}`;
}

function releaseFixture() {
  const subjects = new Map(
    subjectNames.map((name) => [
      name,
      name === "latest.json" ? '{"version":"0.3.0"}\n' : `fixture bytes for ${name}\n`,
    ]),
  );
  const checksumManifest = [...subjects]
    .map(([name, contents]) => `${digest(contents).slice("sha256:".length)}  ./${name}\n`)
    .join("");
  const payloads = new Map([
    ...subjects,
    [CHECKSUM_MANIFEST, checksumManifest],
    [provenanceName, '{"bundle":"fixture"}\n'],
  ]);
  const assets = [...payloads]
    .map(([name, contents]) => ({
      name,
      size: Buffer.byteLength(contents),
      digest: digest(contents),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
  const candidate = { tag, source_sha: sourceSha, prerelease: false, assets };
  const release = {
    tag_name: tag,
    target_commitish: sourceSha,
    draft: false,
    prerelease: false,
    immutable: true,
    assets: assets.map((asset) => ({
      ...asset,
      browser_download_url: `https://github.com/${RELEASE_REPOSITORY}/releases/download/${tag}/${asset.name}`,
    })),
  };
  return { candidate, release, payloads, checksumManifest };
}

function withTempDirectory(run) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-public-release-test-"));
  return Promise.resolve()
    .then(() => run(root))
    .finally(() => fs.rmSync(root, { recursive: true, force: true }));
}

function publicFetch(payloads, requests = []) {
  return async (url, options) => {
    requests.push({ url, options });
    const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
    if (!payloads.has(name)) return new Response("not found", { status: 404 });
    return new Response(payloads.get(name), { status: 200 });
  };
}

test("verifies anonymous public bytes, checksums, release state, and source-bound attestations", async () => {
  await withTempDirectory(async (root) => {
    const { candidate, release, payloads } = releaseFixture();
    const downloadDirectory = path.join(root, "public");
    const requests = [];
    const commands = [];

    const result = await verifyPublicRelease(candidate, release, downloadDirectory, {
      fetchImpl: publicFetch(payloads, requests),
      ghRunner: (arguments_) => commands.push(arguments_),
    });

    assert.deepEqual(result, { assetCount: 13, subjectCount: 11 });
    assert.equal(requests.length, candidate.assets.length);
    assert.ok(
      requests.every(
        ({ options }) =>
          options.redirect === "follow" &&
          options.credentials === "omit" &&
          Object.keys(options.headers).length === 0 &&
          !("authorization" in options.headers),
      ),
      "public downloads must carry no authentication header",
    );
    assert.deepEqual(commands[0], releaseVerificationArguments(tag));
    assert.deepEqual(
      commands.slice(1),
      subjectNames.map((name) =>
        attestationVerificationArguments(
          path.join(downloadDirectory, name),
          path.join(downloadDirectory, provenanceName),
          sourceSha,
        ),
      ),
    );
  });
});

test("rejects missing, unexpected, duplicate, unsafe, and mismatched release assets", () => {
  const { candidate, release } = releaseFixture();
  assert.throws(
    () => buildPublicDownloadPlan({ ...candidate, assets: candidate.assets.slice(1) }, release),
    /release candidate is missing required/,
  );
  assert.throws(
    () => buildPublicDownloadPlan(candidate, { ...release, assets: release.assets.slice(1) }),
    /published release is missing required/,
  );

  const unexpected = {
    name: "unexpected.bin",
    size: 1,
    digest: `sha256:${"0".repeat(64)}`,
    browser_download_url: `https://github.com/${RELEASE_REPOSITORY}/releases/download/${tag}/unexpected.bin`,
  };
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, { ...release, assets: [...release.assets, unexpected] }),
    /unexpected asset/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, {
        ...release,
        assets: [...release.assets, { ...release.assets[0] }],
      }),
    /duplicate release asset/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(
        {
          ...candidate,
          assets: [{ ...candidate.assets[0], name: "../escape" }, ...candidate.assets.slice(1)],
        },
        release,
      ),
    /unsafe release asset name/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, {
        ...release,
        assets: [{ ...release.assets[0], name: "../escape" }, ...release.assets.slice(1)],
      }),
    /unsafe release asset name/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, {
        ...release,
        assets: release.assets.map((asset, index) =>
          index === 0 ? { ...asset, browser_download_url: "https://example.com/asset" } : asset,
        ),
      }),
    /unexpected download URL/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, {
        ...release,
        assets: release.assets.map((asset, index) =>
          index === 0 ? { ...asset, size: asset.size + 1 } : asset,
        ),
      }),
    /size does not match/,
  );
  assert.throws(
    () =>
      buildPublicDownloadPlan(candidate, {
        ...release,
        assets: release.assets.map((asset, index) =>
          index === 0 ? { ...asset, digest: `sha256:${"f".repeat(64)}` } : asset,
        ),
      }),
    /digest does not match/,
  );
  assert.throws(
    () => buildPublicDownloadPlan(candidate, { ...release, immutable: false }),
    /non-draft and immutable/,
  );
  assert.throws(
    () => buildPublicDownloadPlan({ ...candidate, tag: "--help" }, release),
    /release tag must be/,
  );
});

test("requires a new download directory and fails closed on anonymous HTTP errors", async () => {
  await withTempDirectory(async (root) => {
    const { candidate, release, payloads } = releaseFixture();
    const plan = buildPublicDownloadPlan(candidate, release);
    const existing = path.join(root, "existing");
    fs.mkdirSync(existing);
    await assert.rejects(
      downloadPublicAssets(plan, existing, publicFetch(payloads)),
      /must not already exist/,
    );

    const missingPayload = new Map(payloads);
    missingPayload.delete(plan[0].name);
    await assert.rejects(
      downloadPublicAssets(plan, path.join(root, "missing"), publicFetch(missingPayload)),
      /anonymous download failed.*HTTP 404/,
    );
  });
});

test("executable public verification binds the candidate tag to Cargo before downloads", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-public-version-"));
  try {
    const candidateFile = path.join(root, "candidate.json");
    fs.writeFileSync(candidateFile, `${JSON.stringify({ tag: "v9.9.9" })}\n`);
    const downloadDirectory = path.join(root, "downloads");
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL("./verify-public-release.mjs", import.meta.url)),
        candidateFile,
        path.join(root, "missing-release.json"),
        downloadDirectory,
      ],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /release tag v9\.9\.9 expects version 9\.9\.9/u);
    assert.match(result.stderr, /Cargo\.toml:/u);
    assert.equal(fs.existsSync(downloadDirectory), false);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("rejects public size, digest, and asset-set drift after download", async () => {
  await withTempDirectory(async (root) => {
    const { candidate, release, payloads } = releaseFixture();
    const plan = buildPublicDownloadPlan(candidate, release);

    const wrongSize = new Map(payloads);
    wrongSize.set(testSubject, "too many app bytes that exceed the declared fixture size");
    const sizeDirectory = path.join(root, "size");
    await assert.rejects(
      downloadPublicAssets(plan, sizeDirectory, publicFetch(wrongSize)),
      /exceeded the candidate size/,
    );

    const wrongDigest = new Map(payloads);
    wrongDigest.set(testSubject, payloads.get(testSubject).toUpperCase());
    const digestDirectory = path.join(root, "digest");
    await downloadPublicAssets(plan, digestDirectory, publicFetch(wrongDigest));
    await assert.rejects(
      verifyDownloadedAssets(candidate, digestDirectory),
      /digest does not match/,
    );

    const extraDirectory = path.join(root, "extra");
    await downloadPublicAssets(plan, extraDirectory, publicFetch(payloads));
    fs.writeFileSync(path.join(extraDirectory, "unexpected.bin"), "unexpected");
    await assert.rejects(verifyDownloadedAssets(candidate, extraDirectory), /asset set mismatch/);
  });
});

test("parses a strict checksum manifest and requires exact subject coverage", async () => {
  await withTempDirectory((root) => {
    const { candidate, checksumManifest } = releaseFixture();
    fs.writeFileSync(path.join(root, CHECKSUM_MANIFEST), checksumManifest);
    assert.deepEqual(verifyChecksumManifest(candidate, root), {
      subjects: subjectNames,
      bundleName: provenanceName,
    });

    assert.throws(() => parseChecksumManifest(""), /non-empty/);
    assert.throws(() => parseChecksumManifest(checksumManifest.trimEnd()), /end with a newline/);
    assert.throws(
      () => parseChecksumManifest(`${"0".repeat(64)}  ./../escape\n`),
      /unsafe release asset name/,
    );
    const firstLine = checksumManifest.split("\n")[0];
    assert.throws(() => parseChecksumManifest(`${firstLine}\n${firstLine}\n`), /duplicate asset/);

    const writeAndVerify = (contents) => {
      fs.writeFileSync(path.join(root, CHECKSUM_MANIFEST), contents);
      return () => verifyChecksumManifest(candidate, root);
    };
    assert.throws(
      writeAndVerify(checksumManifest.split("\n").slice(0, 1).join("\n") + "\n"),
      /found 11 exceptions/,
    );
    assert.throws(
      writeAndVerify(`${checksumManifest}${"0".repeat(64)}  ./unexpected.bin\n`),
      /unexpected subject/,
    );
    assert.throws(
      writeAndVerify(checksumManifest.replace(/^[0-9a-f]{64}/u, "0".repeat(64))),
      /digest does not match/,
    );
    assert.throws(
      writeAndVerify(`${checksumManifest}${"0".repeat(64)}  ./${CHECKSUM_MANIFEST}\n`),
      /cannot checksum itself/,
    );
  });
});

test("rejects swapping the declared provenance exception with another release role", async () => {
  await withTempDirectory((root) => {
    const { candidate, checksumManifest } = releaseFixture();
    const provenanceDigest = candidate.assets
      .find(({ name }) => name === provenanceName)
      .digest.slice("sha256:".length);
    const swappedManifest = [
      ...checksumManifest
        .trimEnd()
        .split("\n")
        .filter((line) => !line.endsWith(`./${testSubject}`)),
      `${provenanceDigest}  ./${provenanceName}`,
    ].join("\n");
    fs.writeFileSync(path.join(root, CHECKSUM_MANIFEST), `${swappedManifest}\n`);

    assert.throws(
      () => verifyChecksumManifest(candidate, root),
      /must leave only the declared build provenance bundle unchecksummed; found batcave-monitor\.exe/,
    );
  });
});

test("binds release and subject verification to the exact GitHub trust context", () => {
  assert.deepEqual(releaseVerificationArguments(tag), [
    "release",
    "verify",
    tag,
    "--repo",
    "TheGreenCedar/BatCave",
    "--format",
    "json",
  ]);
  assert.deepEqual(attestationVerificationArguments("asset", "bundle", sourceSha), [
    "attestation",
    "verify",
    path.resolve("asset"),
    "--bundle",
    path.resolve("bundle"),
    "--repo",
    "TheGreenCedar/BatCave",
    "--source-digest",
    sourceSha,
    "--source-ref",
    "refs/heads/main",
    "--signer-workflow",
    "TheGreenCedar/BatCave/.github/workflows/release.yml",
    "--deny-self-hosted-runners",
  ]);
  assert.equal(RELEASE_SOURCE_REF, "refs/heads/main");
  assert.equal(RELEASE_SIGNER_WORKFLOW, "TheGreenCedar/BatCave/.github/workflows/release.yml");

  const calls = [];
  runGitHubVerifications(
    { tag, source_sha: sourceSha },
    { subjects: ["asset"], bundleName: "bundle" },
    "/public",
    (arguments_) => calls.push(arguments_),
  );
  assert.equal(calls.length, 2);
  assert.throws(
    () =>
      runGitHubVerifications(
        { tag, source_sha: sourceSha },
        { subjects: ["asset"], bundleName: "bundle" },
        "/public",
        () => {
          throw new Error("verification failed");
        },
      ),
    /verification failed/,
  );
  let verificationCall = 0;
  assert.throws(
    () =>
      runGitHubVerifications(
        { tag, source_sha: sourceSha },
        { subjects: ["asset"], bundleName: "bundle" },
        "/public",
        () => {
          verificationCall += 1;
          if (verificationCall === 2) throw new Error("attestation failed");
        },
      ),
    /attestation failed/,
  );
});

test("runs the public verifier after publication and in every release contract suite", () => {
  const releaseWorkflow = fs.readFileSync(
    new URL("../.github/workflows/release.yml", import.meta.url),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    new URL("../.github/workflows/validation.yml", import.meta.url),
    "utf8",
  );
  const publication = releaseWorkflow.indexOf("- name: Publish verified GitHub Release");
  const publicProof = releaseWorkflow.indexOf(
    "- name: Verify anonymous public release bytes and attestations",
  );
  assert.ok(publication >= 0 && publicProof > publication);
  assert.match(
    releaseWorkflow.slice(publicProof),
    /node scripts\/verify-public-release\.mjs "\$\{candidate\}" "\$\{readback\}" "\$\{public_downloads\}"/u,
  );
  assert.equal(releaseWorkflow.match(/scripts\/verify-public-release\.test\.mjs/gu)?.length, 1);
  assert.equal(validationWorkflow.match(/scripts\/verify-public-release\.test\.mjs/gu)?.length, 2);
});
