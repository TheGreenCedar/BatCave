import assert from "node:assert/strict";
import test from "node:test";
import { validateAudit } from "./validate-cargo-audit-baseline.mjs";

const warning = {
  id: "RUSTSEC-TEST",
  kind: "unmaintained",
  package: "example",
  version: "1.0.0",
  owner: "BatCave maintainers",
  runtime_reachability: "test",
  upstream_blocker: "test",
  review_expires_at: "2026-10-10",
};
const audit = {
  vulnerabilities: { found: false, count: 0 },
  warnings: {
    unmaintained: [
      {
        advisory: { id: warning.id },
        package: { name: warning.package, version: warning.version },
      },
    ],
  },
};

test("accepts the exact reviewed warning set", () => {
  assert.equal(validateAudit(audit, { warnings: [warning] }, "2026-07-10").warnings, 1);
});

test("rejects a new unreviewed warning", () => {
  assert.throws(
    () => validateAudit(audit, { warnings: [] }, "2026-07-10"),
    /unreviewed cargo-audit warning/,
  );
});

test("rejects changed versions and stale entries", () => {
  assert.throws(
    () => validateAudit(audit, { warnings: [{ ...warning, version: "0.9.0" }] }, "2026-07-10"),
    /unreviewed cargo-audit warning.*stale cargo-audit baseline entry/s,
  );
});

test("rejects expired reviews and vulnerabilities", () => {
  const vulnerable = { ...audit, vulnerabilities: { found: true, count: 1 } };
  assert.throws(
    () =>
      validateAudit(
        vulnerable,
        { warnings: [{ ...warning, review_expires_at: "2026-07-09" }] },
        "2026-07-10",
      ),
    /vulnerabilities.*review expired/s,
  );
});
