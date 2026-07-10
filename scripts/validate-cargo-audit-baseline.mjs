import fs from "node:fs";
import { pathToFileURL } from "node:url";

function warningKey(warning) {
  return [warning.id, warning.kind, warning.package, warning.version].join("|");
}

export function validateAudit(audit, baseline, today = new Date().toISOString().slice(0, 10)) {
  const errors = [];
  if (audit.vulnerabilities?.count !== 0 || audit.vulnerabilities?.found) {
    errors.push(
      `cargo audit reported ${audit.vulnerabilities?.count ?? "unknown"} vulnerabilities`,
    );
  }

  const current = Object.entries(audit.warnings ?? {}).flatMap(([kind, warnings]) =>
    warnings.map((warning) => ({
      id: warning.advisory.id,
      kind,
      package: warning.package.name,
      version: warning.package.version,
    })),
  );
  const reviewed = baseline.warnings ?? [];
  const currentKeys = new Set(current.map(warningKey));
  const reviewedKeys = new Set(reviewed.map(warningKey));

  for (const warning of reviewed) {
    if (!warning.owner || !warning.runtime_reachability || !warning.upstream_blocker) {
      errors.push(`baseline metadata is incomplete for ${warningKey(warning)}`);
    }
    if (!warning.review_expires_at || warning.review_expires_at < today) {
      errors.push(
        `baseline review expired for ${warningKey(warning)} on ${warning.review_expires_at ?? "unknown"}`,
      );
    }
  }
  for (const key of currentKeys) {
    if (!reviewedKeys.has(key)) errors.push(`unreviewed cargo-audit warning: ${key}`);
  }
  for (const key of reviewedKeys) {
    if (!currentKeys.has(key)) errors.push(`stale cargo-audit baseline entry: ${key}`);
  }

  if (errors.length > 0) throw new Error(errors.join("\n"));
  return {
    vulnerabilities: 0,
    warnings: current.length,
    review_expires_at: baseline.review_expires_at,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [, , auditPath, baselinePath] = process.argv;
  if (!auditPath || !baselinePath) {
    console.error(
      "usage: node scripts/validate-cargo-audit-baseline.mjs <audit.json> <baseline.json>",
    );
    process.exit(2);
  }
  try {
    const result = validateAudit(
      JSON.parse(fs.readFileSync(auditPath, "utf8")),
      JSON.parse(fs.readFileSync(baselinePath, "utf8")),
    );
    console.log(
      `cargo-audit baseline matched: 0 vulnerabilities, ${result.warnings} reviewed warnings`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
