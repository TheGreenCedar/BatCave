import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import { verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

function requireExactRoleAsset(role, names) {
  const matches = names.filter((name) => role.family.test(name));
  if (matches.length !== 1) {
    throw new Error(`${role.role} requires exactly one asset; found ${matches.length}`);
  }
  if (matches[0] !== role.name) {
    throw new Error(`${role.role} must be named ${role.name}; received ${matches[0]}`);
  }
  return matches[0];
}

export function buildUpdateManifest(tag, repository, artifacts) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository: ${repository}`);
  }
  const contract = expectedReleaseAssetRoles(tag);
  const names = Object.keys(artifacts);
  const signaturesByPayload = new Map(
    contract.roles
      .filter((role) => role.signatureFor)
      .map((signature) => [signature.signatureFor, signature]),
  );
  const platforms = {};
  for (const payload of contract.roles.filter((role) => role.updateTargets)) {
    const name = requireExactRoleAsset(payload, names);
    const signatureRole = signaturesByPayload.get(payload.role);
    const signatureName = requireExactRoleAsset(signatureRole, names);
    const signature = artifacts[signatureName]?.trim();
    if (!signature) throw new Error(`${signatureName} is missing or empty`);

    for (const target of payload.updateTargets) {
      platforms[target] = {
        signature,
        url: `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`,
      };
    }
  }
  return { version: contract.version, notes: `BatCave ${tag}`, platforms };
}

export function writeUpdateManifest(tag, repository, distDirectory) {
  verifyWorkspaceReleaseVersion(tag);
  const artifacts = Object.fromEntries(
    fs
      .readdirSync(distDirectory)
      .map((name) => [
        name,
        name.endsWith(".sig") ? fs.readFileSync(path.join(distDirectory, name), "utf8") : "",
      ]),
  );
  const manifest = buildUpdateManifest(tag, repository, artifacts);
  fs.writeFileSync(
    path.join(distDirectory, "latest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [tag, repository, distDirectory = "dist"] = process.argv.slice(2);
  if (!tag || !repository) {
    console.error("usage: node scripts/build-update-manifest.mjs <tag> <owner/repo> [dist]");
    process.exit(2);
  }
  try {
    writeUpdateManifest(tag, repository, distDirectory);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
