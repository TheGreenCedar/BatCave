import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const VERSION_PATTERN = /^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;

export function verifyReleaseVersion(tag, cargoVersion) {
  const match = VERSION_PATTERN.exec(tag);
  if (!match) throw new Error(`release tag must be v<semver>; received ${tag}`);
  const expected = match[1];
  if (cargoVersion !== undefined && cargoVersion !== expected) {
    throw new Error(`release tag ${tag} expects version ${expected}\nCargo.toml: ${cargoVersion}`);
  }
  return { version: expected, prerelease: expected.includes("-") };
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

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const tag = process.argv[2];
  if (!tag) {
    console.error("usage: node scripts/verify-release-version.mjs <vX.Y.Z[-prerelease]|--print>");
    process.exit(2);
  }
  try {
    const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const cargoVersion = readCargoVersion(repoRoot);
    if (tag === "--print") {
      console.log(cargoVersion);
      process.exit(0);
    }
    const result = verifyReleaseVersion(tag, cargoVersion);
    console.log(
      `release version aligned: ${result.version} (${result.prerelease ? "prerelease" : "stable"})`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
