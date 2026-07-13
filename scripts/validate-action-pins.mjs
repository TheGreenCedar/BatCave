import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const IMMUTABLE_REF = /^[0-9a-f]{40}$/;
const USES_LINE = /^\s*(?:-\s*)?uses:\s*(?:"([^"]+)"|'([^']+)'|([^\s#]+))\s*(?:#\s*(.*))?$/;

function workflowFiles(workflowRoot) {
  if (!fs.existsSync(workflowRoot)) return [];

  const files = [];
  const visit = (directory) => {
    const entries = fs
      .readdirSync(directory, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) visit(entryPath);
      if (entry.isFile() && /\.ya?ml$/i.test(entry.name)) files.push(entryPath);
    }
  };
  visit(workflowRoot);
  return files;
}

function checkUsesValue(value, comment) {
  if (value.startsWith("./") || value.startsWith("docker://")) return null;

  const at = value.lastIndexOf("@");
  if (at < 1) return "external action is missing an immutable commit reference";

  const action = value.slice(0, at);
  const ref = value.slice(at + 1);
  if (!/^[^/@\s]+\/[^/@\s]+(?:\/[^@\s]+)*$/.test(action)) {
    return "external action must use owner/repository[/path] syntax";
  }
  if (!IMMUTABLE_REF.test(ref)) {
    return `external action ref must be an exact lowercase 40-character commit SHA; received ${ref}`;
  }
  if (!comment?.trim()) {
    return "immutable action pin must retain its readable version as an inline comment";
  }
  return null;
}

export function collectActionPinViolations(repoRoot) {
  const workflowRoot = path.join(repoRoot, ".github", "workflows");
  const violations = [];

  for (const file of workflowFiles(workflowRoot)) {
    const relativeFile = path.relative(repoRoot, file).split(path.sep).join("/");
    const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
    lines.forEach((line, index) => {
      if (!/^\s*(?:-\s*)?uses:/.test(line)) return;

      const parsed = USES_LINE.exec(line);
      if (!parsed) {
        violations.push({
          file: relativeFile,
          line: index + 1,
          message: "uses entries must be single-line scalar values",
        });
        return;
      }

      const value = parsed[1] ?? parsed[2] ?? parsed[3];
      const message = checkUsesValue(value, parsed[4]);
      if (message) violations.push({ file: relativeFile, line: index + 1, message });
    });
  }

  return violations;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const repoRoot = path.resolve(process.argv[2] ?? process.cwd());
  const violations = collectActionPinViolations(repoRoot);
  if (violations.length > 0) {
    for (const violation of violations) {
      console.error(`${violation.file}:${violation.line}: ${violation.message}`);
    }
    process.exit(1);
  }
  console.log("GitHub Actions references are pinned to immutable commits");
}
