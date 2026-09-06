import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { parseReleaseTag } from "./verify-release-version.mjs";

const REPOSITORY = "TheGreenCedar/BatCave";

function requireOrigin(condition, message) {
  if (!condition) throw new Error(message);
}

export function verifyPublishedReleaseOrigin({ tag, sourceSha, runId, run, jobs, artifacts, release }) {
  parseReleaseTag(tag);
  requireOrigin(/^[a-f0-9]{40}$/.test(sourceSha) && /^[1-9][0-9]*$/.test(runId), "invalid release source or run ID");
  requireOrigin(
    String(run?.id) === runId && run.path === ".github/workflows/release.yml" &&
      run.event === "workflow_dispatch" && run.head_branch === "main" && run.head_sha === sourceSha &&
      run.repository?.full_name === REPOSITORY && run.head_repository?.full_name === REPOSITORY,
    "original release run must belong to this repository, workflow, and source commit",
  );
  requireOrigin(jobs?.total_count === jobs?.jobs?.length, "publication job inventory must be complete");
  const finalizers = jobs.jobs.filter(job => job.name === "Checksums, provenance, and release");
  requireOrigin(finalizers.length === 1 && finalizers[0].run_id === run.id &&
    finalizers[0].run_attempt === run.run_attempt && finalizers[0].head_sha === sourceSha &&
    finalizers[0].status === "completed" && finalizers[0].conclusion === "success",
  "original publication job must have succeeded for this release source and attempt");
  requireOrigin(release?.tag_name === tag && release.target_commitish === sourceSha &&
    release.draft === false && release.immutable === true && typeof release.published_at === "string",
  "release must already be published and immutable at the original source commit");
  requireOrigin(artifacts?.total_count === artifacts?.artifacts?.length, "candidate artifact inventory must be complete");
  const candidates = artifacts.artifacts.filter(artifact => artifact.name === `batcave-release-candidate-${tag}`);
  requireOrigin(candidates.length === 1 && candidates[0].expired === false &&
    candidates[0].workflow_run?.id === run.id && candidates[0].workflow_run?.head_sha === sourceSha,
  "original prepublication candidate must remain available from the release run");
  return true;
}

function request(endpoint) {
  const result = spawnSync("gh", ["api", endpoint], { encoding: "utf8", timeout: 30_000, maxBuffer: 2 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error("could not read published release origin");
  return JSON.parse(result.stdout);
}

export function verifyLivePublishedReleaseOrigin(tag, sourceSha, runId, read = request) {
  parseReleaseTag(tag);
  requireOrigin(/^[a-f0-9]{40}$/.test(sourceSha) && /^[1-9][0-9]*$/.test(runId), "invalid release source or run ID");
  const prefix = `repos/${REPOSITORY}`;
  return verifyPublishedReleaseOrigin({
    tag, sourceSha, runId,
    run: read(`${prefix}/actions/runs/${runId}`),
    jobs: read(`${prefix}/actions/runs/${runId}/jobs?filter=latest&per_page=100`),
    artifacts: read(`${prefix}/actions/runs/${runId}/artifacts?per_page=100`),
    release: read(`${prefix}/releases/tags/${tag}`),
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    requireOrigin(process.argv.length === 5, "usage: verify-published-release-origin.mjs <tag> <source-sha> <run-id>");
    verifyLivePublishedReleaseOrigin(...process.argv.slice(2));
    console.log("Published release source, publication job, and original candidate verified");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
