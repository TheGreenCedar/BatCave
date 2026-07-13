import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { verifyReleaseVersion } from "./verify-release-version.mjs";

const COMMIT_SHA = /^[0-9a-f]{40}$/;

function requireCommitSha(name, value) {
  if (!COMMIT_SHA.test(value)) {
    throw new Error(`${name} must be an exact lowercase 40-character commit SHA; received ${value}`);
  }
}

export function verifyReleaseCandidateIdentity({
  tag,
  channel,
  sourceSha,
  mainSha,
  approvedSourceSha,
}) {
  const { prerelease } = verifyReleaseVersion(tag, {});
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
      if (entry.isSymbolicLink()) throw new Error(`release input cannot contain symlinks: ${entryPath}`);
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
    const name = path.basename(source).replaceAll(" ", ".");
    const prior = names.get(name);
    if (prior) {
      throw new Error(`release assets ${prior} and ${source} both normalize to ${name}`);
    }
    names.set(name, source);
  }

  const staged = [...names.keys()].sort((a, b) => a.localeCompare(b));
  if (staged.length === 0) throw new Error("release input contains no files");

  fs.mkdirSync(outputRoot, { recursive: true });
  for (const name of staged) {
    fs.copyFileSync(names.get(name), path.join(outputRoot, name), fs.constants.COPYFILE_EXCL);
  }
  return staged;
}

export function buildReleaseInventory(tag, sourceSha, prerelease, directory) {
  verifyReleaseVersion(tag, {});
  requireCommitSha("source SHA", sourceSha);
  if (typeof prerelease !== "boolean") throw new Error("prerelease must be a boolean");

  const entries = fs.readdirSync(directory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) {
    throw new Error("staged release directory must contain files only");
  }
  const assets = entries
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((entry) => {
      const file = path.join(directory, entry.name);
      return {
        name: entry.name,
        size: fs.statSync(file).size,
        digest: `sha256:${crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex")}`,
      };
    });
  if (assets.length === 0) throw new Error("release candidate contains no assets");
  return { tag, source_sha: sourceSha, prerelease, assets };
}

export function verifyReleaseReadback(expected, actual, expectedDraft) {
  if (actual.tag_name !== expected.tag) {
    throw new Error(`release tag readback mismatch: expected ${expected.tag}, received ${actual.tag_name}`);
  }
  if (actual.target_commitish !== expected.source_sha) {
    throw new Error(
      `release source readback mismatch: expected ${expected.source_sha}, received ${actual.target_commitish}`,
    );
  }
  if (actual.draft !== expectedDraft) {
    throw new Error(`release draft readback mismatch: expected ${expectedDraft}, received ${actual.draft}`);
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
    "  node scripts/verify-release-candidate.mjs inventory <tag> <source-sha> <prerelease> <directory> <output-json>",
    "  node scripts/verify-release-candidate.mjs verify-readback <expected-json> <actual-json> <draft>",
    "  node scripts/verify-release-candidate.mjs verify-latest <expected-json> <latest-json>",
  ].join("\n");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [command, ...args] = process.argv.slice(2);
  try {
    if (command === "identity" && args.length === 5) {
      const [tag, channel, sourceSha, mainSha, approvedSourceSha] = args;
      const candidate = verifyReleaseCandidateIdentity({
        tag,
        channel,
        sourceSha,
        mainSha,
        approvedSourceSha,
      });
      console.log(`release candidate identity verified: ${candidate.tag} at ${candidate.sourceSha}`);
    } else if (command === "stage" && args.length === 2) {
      const assets = stageReleaseAssets(...args);
      console.log(`staged ${assets.length} release assets`);
    } else if (command === "inventory" && args.length === 5) {
      const [tag, sourceSha, prerelease, directory, output] = args;
      const inventory = buildReleaseInventory(
        tag,
        sourceSha,
        booleanArgument(prerelease, "prerelease"),
        directory,
      );
      fs.writeFileSync(output, `${JSON.stringify(inventory, null, 2)}\n`);
      console.log(`wrote release inventory for ${inventory.assets.length} assets`);
    } else if (command === "verify-readback" && args.length === 3) {
      const [expectedFile, actualFile, draft] = args;
      verifyReleaseReadback(
        JSON.parse(fs.readFileSync(expectedFile, "utf8")),
        JSON.parse(fs.readFileSync(actualFile, "utf8")),
        booleanArgument(draft, "draft"),
      );
      console.log("GitHub Release readback matches the local candidate");
    } else if (command === "verify-latest" && args.length === 2) {
      const [expectedFile, latestFile] = args;
      verifyLatestRelease(
        JSON.parse(fs.readFileSync(expectedFile, "utf8")),
        JSON.parse(fs.readFileSync(latestFile, "utf8")),
      );
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
