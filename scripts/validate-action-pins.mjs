import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const parser = fileURLToPath(new URL("./validate-action-pins.rb", import.meta.url));

export function collectActionPinViolations(repoRoot) {
  const result = spawnSync("ruby", [parser, "--json", path.resolve(repoRoot)], {
    encoding: "utf8",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || `action pin parser exited with status ${result.status}`);
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`action pin parser returned invalid JSON: ${error.message}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const repoRoot = path.resolve(process.argv[2] ?? process.cwd());
  try {
    const violations = collectActionPinViolations(repoRoot);
    if (violations.length > 0) {
      for (const violation of violations) {
        console.error(`${violation.file}:${violation.line}: ${violation.message}`);
      }
      process.exit(1);
    }
    console.log("GitHub Actions references are pinned to immutable commits or digests");
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
