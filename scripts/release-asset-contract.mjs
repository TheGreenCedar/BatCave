import path from "node:path";
import { parseReleaseTag } from "./verify-release-version.mjs";

const declarations = [
  {
    role: "Windows GUI executable",
    name: () => "batcave-monitor.exe",
    family: /^batcave-monitor\.exe$/u,
  },
  {
    role: "Windows CLI executable",
    name: () => "batcave-monitor-cli.exe",
    family: /^batcave-monitor-cli\.exe$/u,
  },
  {
    role: "Windows NSIS installer and updater payload",
    name: ({ version }) => `BatCave.Monitor_${version}_x64-setup.exe`,
    family: /^BatCave\.Monitor_.+_x64-setup\.exe$/u,
    updateTargets: ["windows-x86_64"],
  },
  {
    role: "Windows updater signature",
    name: ({ roles }) => `${roles.get("Windows NSIS installer and updater payload")}.sig`,
    family: /^BatCave\.Monitor_.+_x64-setup\.exe\.sig$/u,
    signatureFor: "Windows NSIS installer and updater payload",
  },
  {
    role: "Linux deb package",
    name: ({ version }) => `BatCave.Monitor_${version}_amd64.deb`,
    family: /^BatCave\.Monitor_.+_amd64\.deb$/u,
  },
  {
    role: "Linux AppImage package and updater payload",
    name: ({ version }) => `BatCave.Monitor_${version}_amd64.AppImage`,
    family: /^BatCave\.Monitor_.+_amd64\.AppImage$/u,
    updateTargets: ["linux-x86_64"],
  },
  {
    role: "Linux updater signature",
    name: ({ roles }) => `${roles.get("Linux AppImage package and updater payload")}.sig`,
    family: /^BatCave\.Monitor_.+_amd64\.AppImage\.sig$/u,
    signatureFor: "Linux AppImage package and updater payload",
  },
  {
    role: "macOS universal DMG",
    name: ({ version }) => `BatCave.Monitor_${version}_universal.dmg`,
    family: /^BatCave\.Monitor_.+_universal\.dmg$/u,
  },
  {
    role: "macOS universal updater payload",
    name: () => "BatCave.Monitor.app.tar.gz",
    family: /\.app\.tar\.gz$/u,
    updateTargets: ["darwin-aarch64", "darwin-x86_64"],
  },
  {
    role: "macOS updater signature",
    name: ({ roles }) => `${roles.get("macOS universal updater payload")}.sig`,
    family: /\.app\.tar\.gz\.sig$/u,
    signatureFor: "macOS universal updater payload",
  },
  {
    role: "updater manifest",
    name: () => "latest.json",
    family: /^latest\.json$/u,
  },
  {
    role: "checksum manifest",
    name: () => "SHA256SUMS.txt",
    family: /^SHA256SUMS\.txt$/u,
  },
  {
    role: "build provenance bundle",
    name: ({ tag }) => `BatCave-${tag}-provenance.json`,
    family: /^BatCave-v.+-provenance\.json$/u,
  },
];

export function canonicalReleaseAssetName(name) {
  return name.normalize("NFC").toLowerCase();
}

export function requireSafeReleaseAssetName(name) {
  const containsControlCharacter =
    typeof name === "string" &&
    [...name].some((character) => {
      const codePoint = character.codePointAt(0);
      return codePoint <= 0x1f || codePoint === 0x7f;
    });
  if (
    typeof name !== "string" ||
    name.length === 0 ||
    name === "." ||
    name === ".." ||
    name !== name.normalize("NFC") ||
    path.posix.basename(name) !== name ||
    path.win32.basename(name) !== name ||
    containsControlCharacter
  ) {
    throw new Error(`unsafe release asset name: ${String(name)}`);
  }
  return name;
}

export function expectedReleaseAssetRoles(tag) {
  const { version, prerelease } = parseReleaseTag(tag);
  const namesByRole = new Map();
  const roles = declarations.map((declaration) => {
    const name = declaration.name({ tag, version, prerelease, roles: namesByRole });
    namesByRole.set(declaration.role, name);
    return Object.freeze({
      role: declaration.role,
      name,
      family: declaration.family,
      signatureFor: declaration.signatureFor,
      updateTargets: declaration.updateTargets
        ? Object.freeze([...declaration.updateTargets])
        : undefined,
    });
  });
  return Object.freeze({ tag, version, prerelease, roles: Object.freeze(roles) });
}

function assetName(asset, owner) {
  const name = typeof asset === "string" ? asset : asset?.name;
  try {
    return requireSafeReleaseAssetName(name);
  } catch (error) {
    throw new Error(`${owner} ${error.message}`);
  }
}

export function verifyReleaseAssetInventory(tag, prerelease, assets, owner = "release inventory") {
  const contract = expectedReleaseAssetRoles(tag);
  if (typeof prerelease !== "boolean") {
    throw new Error(`${owner} prerelease state must be a boolean`);
  }
  if (prerelease !== contract.prerelease) {
    const channel = contract.prerelease ? "prerelease" : "stable";
    throw new Error(`${owner} channel does not match ${tag}; expected ${channel}`);
  }
  if (!Array.isArray(assets) || assets.length === 0) {
    throw new Error(`${owner} must contain release assets`);
  }

  const names = assets.map((asset) => assetName(asset, owner));
  const canonicalNames = new Map();
  for (const name of names) {
    const canonical = canonicalReleaseAssetName(name);
    if (canonicalNames.has(canonical)) {
      throw new Error(
        `${owner} contains duplicate basename ${canonicalNames.get(canonical)} and ${name}`,
      );
    }
    canonicalNames.set(canonical, name);
  }

  const actualNames = new Set(names);
  const expectedNames = new Set(contract.roles.map((role) => role.name));

  for (const signature of contract.roles.filter((role) => role.signatureFor)) {
    const payload = contract.roles.find((role) => role.role === signature.signatureFor);
    if (actualNames.has(signature.name) && !actualNames.has(payload.name)) {
      throw new Error(
        `${owner} contains orphan signature ${signature.name}; missing ${payload.name}`,
      );
    }
  }

  for (const role of contract.roles) {
    const matches = names.filter((name) => role.family.test(name));
    if (matches.length > 1) {
      throw new Error(`${owner} contains duplicate ${role.role} assets: ${matches.join(", ")}`);
    }
    if (matches.length === 1 && matches[0] !== role.name) {
      throw new Error(
        `${owner} ${role.role} has the wrong filename for ${tag}: ${matches[0]}; expected ${role.name}`,
      );
    }
  }

  for (const role of contract.roles) {
    if (!actualNames.has(role.name)) {
      throw new Error(`${owner} is missing required ${role.role} asset ${role.name}`);
    }
  }

  for (const name of names) {
    if (!expectedNames.has(name)) {
      throw new Error(`${owner} contains unexpected asset ${name}`);
    }
  }

  return contract;
}
