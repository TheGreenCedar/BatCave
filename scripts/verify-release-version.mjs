import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const VERSION_PATTERN = /^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;
const REPOSITORY_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// Pure tag parsing is for table-driven contracts and tests with synthetic versions.
// Executable release boundaries must call verifyWorkspaceReleaseVersion instead.
export function parseReleaseTag(tag) {
  const match = VERSION_PATTERN.exec(tag);
  if (!match) throw new Error(`release tag must be v<semver>; received ${tag}`);
  const expected = match[1];
  return { version: expected, prerelease: expected.includes("-") };
}

export function verifyReleaseVersion(tag, cargoVersion) {
  const result = parseReleaseTag(tag);
  if (typeof cargoVersion !== "string" || cargoVersion.length === 0) {
    throw new Error("Cargo package version is required for release verification");
  }
  if (cargoVersion !== result.version) {
    throw new Error(
      `release tag ${tag} expects version ${result.version}\nCargo.toml: ${cargoVersion}`,
    );
  }
  return result;
}

function tomlPackageVersion(contents, file) {
  const match = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(contents);
  if (!match) throw new Error(`could not read package version from ${file}`);
  return match[1];
}

export function readCargoVersion(repoRoot) {
  const appRoot = path.join(repoRoot, "src", "BatCave.App");
  const cargoFile = path.join(appRoot, "src-tauri", "Cargo.toml");
  return tomlPackageVersion(fs.readFileSync(cargoFile, "utf8"), cargoFile);
}

export function verifyWorkspaceReleaseVersion(tag, repoRoot = REPOSITORY_ROOT) {
  return verifyReleaseVersion(tag, readCargoVersion(repoRoot));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const tag = process.argv[2];
  if (!tag) {
    console.error("usage: node scripts/verify-release-version.mjs <vX.Y.Z[-prerelease]|--print>");
    process.exit(2);
  }
  try {
    if (tag === "--print") {
      console.log(readCargoVersion(REPOSITORY_ROOT));
      process.exit(0);
    }
    const result = verifyWorkspaceReleaseVersion(tag);
    console.log(
      `release version aligned: ${result.version} (${result.prerelease ? "prerelease" : "stable"})`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
