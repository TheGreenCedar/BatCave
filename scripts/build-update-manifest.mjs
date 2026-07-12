import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { verifyReleaseVersion } from "./verify-release-version.mjs";

const TARGETS = [
  ["windows-x86_64", "_x64-setup.exe"],
  ["linux-x86_64", "_amd64.AppImage"],
  ["darwin-aarch64", ".app.tar.gz"],
  ["darwin-x86_64", ".app.tar.gz"],
];

export function buildUpdateManifest(tag, repository, artifacts) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository: ${repository}`);
  }
  const { version } = verifyReleaseVersion(tag, {});
  const names = Object.keys(artifacts);
  const platforms = Object.fromEntries(
    TARGETS.map(([target, suffix]) => {
      const matches = names.filter((name) => name.endsWith(suffix));
      if (matches.length !== 1) {
        throw new Error(`${target} requires exactly one ${suffix} asset`);
      }
      const name = matches[0];
      const signature = artifacts[`${name}.sig`]?.trim();
      if (!signature) throw new Error(`${name}.sig is missing or empty`);
      return [
        target,
        {
          signature,
          url: `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`,
        },
      ];
    }),
  );
  return { version, notes: `BatCave ${tag}`, platforms };
}

export function writeUpdateManifest(tag, repository, distDirectory) {
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
