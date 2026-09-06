import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { verifyPublishedReleaseOrigin } from "./verify-published-release-origin.mjs";

function fixture() {
  const sourceSha = "a".repeat(40);
  return {
    tag: "v0.2.0-rc.5", sourceSha, runId: "123",
    run: { id: 123, run_attempt: 1, path: ".github/workflows/release.yml", event: "workflow_dispatch",
      head_branch: "main", head_sha: sourceSha, conclusion: "failure",
      repository: { full_name: "TheGreenCedar/BatCave" }, head_repository: { full_name: "TheGreenCedar/BatCave" } },
    jobs: { total_count: 1, jobs: [{ name: "Checksums, provenance, and release", run_id: 123, run_attempt: 1,
      head_sha: sourceSha, status: "completed", conclusion: "success" }] },
    release: { tag_name: "v0.2.0-rc.5", target_commitish: sourceSha, draft: false, immutable: true, published_at: "2026-09-06T12:40:17Z" },
    artifacts: { total_count: 1, artifacts: [{ name: "batcave-release-candidate-v0.2.0-rc.5", expired: false,
      workflow_run: { id: 123, head_sha: sourceSha } }] },
  };
}

test("rechecks a published release even when later observers failed", () => {
  assert.equal(verifyPublishedReleaseOrigin(fixture()), true);
});

test("rejects mismatched source, publication, or retained candidate", () => {
  for (const mutate of [
    value => value.run.head_sha = "b".repeat(40),
    value => value.run.path = ".github/workflows/other.yml",
    value => value.run.repository.full_name = "other/repo",
    value => value.run.event = "pull_request",
    value => value.run.head_branch = "other",
    value => value.jobs.jobs[0].conclusion = "failure",
    value => value.jobs.jobs[0].run_attempt++,
    value => value.jobs.jobs[0].head_sha = "b".repeat(40),
    value => value.jobs.total_count++,
    value => value.release.immutable = false,
    value => value.release.draft = true,
    value => value.release.target_commitish = "main",
    value => value.release.tag_name = "v0.2.0-rc.4",
    value => value.artifacts.artifacts[0].expired = true,
    value => value.artifacts.artifacts[0].workflow_run.id++,
    value => value.artifacts.artifacts[0].workflow_run.head_sha = "b".repeat(40),
    value => value.artifacts.artifacts[0].name = "replacement",
    value => value.artifacts.artifacts.push(value.artifacts.artifacts[0]),
  ]) {
    const value = fixture();
    mutate(value);
    assert.throws(() => verifyPublishedReleaseOrigin(value));
  }
});

test("publication and manual rechecks share read-only observers with separate verifier and release revisions", () => {
  const release = fs.readFileSync(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");
  const verifier = fs.readFileSync(new URL("../.github/workflows/verify-published-release.yml", import.meta.url), "utf8");
  assert.match(release, /needs: \[prepare, finalize\][\s\S]*uses: \.\/\.github\/workflows\/verify-published-release.yml/);
  assert.match(release, /release_run_id: \$\{\{ format\('\{0\}', github.run_id\) \}\}/);
  assert.match(verifier, /workflow_dispatch:[\s\S]*workflow_call:/);
  assert.match(verifier, /if: github.ref == 'refs\/heads\/main'/);
  assert.doesNotMatch(verifier, /: write|secrets\.|gh release (?:create|edit|upload)|needs.prepare/);
  assert.equal((verifier.match(/ref: \$\{\{ github.sha \}\}/g) ?? []).length, 4);
  assert.equal((verifier.match(/run-id: \$\{\{ inputs.release_run_id \}\}/g) ?? []).length, 2);
  assert.equal((verifier.match(/needs: verify_origin/g) ?? []).length, 3);
});
