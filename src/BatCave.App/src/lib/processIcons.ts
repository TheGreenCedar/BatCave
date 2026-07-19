export type ProcessIconOrigin = "native" | "name_match" | "fallback";

export interface ResolvedProcessIcon {
  src?: string;
  origin: ProcessIconOrigin;
}

export type ResolvedProcessIconCatalog = Record<string, ResolvedProcessIcon>;

export interface ProcessIconCandidate {
  name: string;
  exe?: string | null;
}

const genericFamilyKeys = new Set([
  "gpu",
  "gpuprocess",
  "helper",
  "process",
  "renderer",
  "utility",
]);
const terminalRolePattern = "helper|renderer|gpu[\\s._-]+process|utility";
const parenthesizedTerminalRole = new RegExp(
  `\\s*\\(\\s*(?:${terminalRolePattern})\\s*\\)\\s*$`,
  "iu",
);
const plainTerminalRole = new RegExp(`(?:[\\s._-]+)(?:${terminalRolePattern})\\s*$`, "iu");
const numericInstanceSuffix = /(?:[\s._-]+)\d+\s*$/u;

export function processIconKey(process: ProcessIconCandidate): string {
  return process.exe || process.name;
}

export function processIconFamily(name: string): string | null {
  let normalized = name.normalize("NFKC").trim().toLocaleLowerCase();
  normalized = normalized.replace(/\.exe\s*$/iu, "").trim();

  let previous = "";
  while (normalized && normalized !== previous) {
    previous = normalized;
    normalized = normalized
      .replace(parenthesizedTerminalRole, "")
      .replace(numericInstanceSuffix, "")
      .replace(plainTerminalRole, "")
      .trim();
  }

  const family = normalized.replace(/[^\p{L}\p{N}]+/gu, "");
  if (family.length < 4 || genericFamilyKeys.has(family)) return null;
  return family;
}

export function buildResolvedProcessIconCatalog(
  processes: readonly ProcessIconCandidate[],
  nativeIcons: Readonly<Record<string, string>>,
): ResolvedProcessIconCatalog {
  const donors = new Map<string, Set<string>>();

  for (const process of processes) {
    const nativeIcon = nativeIcons[processIconKey(process)];
    const family = processIconFamily(process.name);
    if (!nativeIcon || !family) continue;

    const familyIcons = donors.get(family) ?? new Set<string>();
    familyIcons.add(nativeIcon);
    donors.set(family, familyIcons);
  }

  const catalog: ResolvedProcessIconCatalog = {};
  for (const process of processes) {
    const key = processIconKey(process);
    const nativeIcon = nativeIcons[key];
    if (nativeIcon) {
      catalog[key] = { src: nativeIcon, origin: "native" };
      continue;
    }

    const family = processIconFamily(process.name);
    const familyIcons = family ? donors.get(family) : undefined;
    if (familyIcons?.size === 1) {
      catalog[key] = { src: familyIcons.values().next().value, origin: "name_match" };
    } else {
      catalog[key] = { origin: "fallback" };
    }
  }

  return catalog;
}

export function resolvedProcessIcon(
  catalog: Readonly<ResolvedProcessIconCatalog>,
  key: string | null | undefined,
): ResolvedProcessIcon {
  return (key ? catalog[key] : undefined) ?? { origin: "fallback" };
}
