import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const VERSION_PATTERN = /^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;

export function verifyReleaseVersion(tag, versions) {
  const match = VERSION_PATTERN.exec(tag);
  if (!match) throw new Error(`release tag must be v<semver>; received ${tag}`);
  const expected = match[1];
  const mismatches = Object.entries(versions).filter(([, version]) => version !== expected);
  if (mismatches.length > 0) {
    throw new Error(
      [
        `release tag ${tag} expects version ${expected}`,
        ...mismatches.map(([file, version]) => `${file}: ${version}`),
      ].join("\n"),
    );
  }
  return { version: expected, prerelease: expected.includes("-") };
}

function tomlPackageVersion(contents, file) {
  const match = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(contents);
  if (!match) throw new Error(`could not read package version from ${file}`);
  return match[1];
}

export function readWorkspaceVersions(repoRoot) {
  const appRoot = path.join(repoRoot, "src", "BatCave.App");
  const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
  const packageJson = readJson(path.join(appRoot, "package.json"));
  const packageLock = readJson(path.join(appRoot, "package-lock.json"));
  const tauriConfig = readJson(path.join(appRoot, "src-tauri", "tauri.conf.json"));
  const cargoFile = path.join(appRoot, "src-tauri", "Cargo.toml");
  return {
    "package.json": packageJson.version,
    "package-lock.json": packageLock.version,
    "package-lock.json workspace": packageLock.packages?.[""]?.version,
    "Cargo.toml": tomlPackageVersion(fs.readFileSync(cargoFile, "utf8"), cargoFile),
    "tauri.conf.json": tauriConfig.version,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const tag = process.argv[2];
  if (!tag) {
    console.error("usage: node scripts/verify-release-version.mjs <vX.Y.Z[-prerelease]>");
    process.exit(2);
  }
  try {
    const result = verifyReleaseVersion(tag, readWorkspaceVersions(process.cwd()));
    console.log(
      `release version aligned: ${result.version} (${result.prerelease ? "prerelease" : "stable"})`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
