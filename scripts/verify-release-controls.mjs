import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const REQUIRED_STATUS_CHECK_CONTEXTS = Object.freeze([
  "Repository policy",
  "Dependency review",
  "Windows validation",
  "Linux validation",
  "Linux package transport",
  "macOS Apple Silicon validation",
]);

function requireControl(condition, message) {
  if (!condition) throw new Error(message);
}

function verifyRun(repository, sourceSha, run) {
  requireControl(run, "no Validation run exists for the release commit");
  requireControl(
    run.path === ".github/workflows/validation.yml" &&
      run.head_sha === sourceSha && run.head_branch === "main" && run.event === "push" &&
      run.repository?.full_name?.toLowerCase() === repository.toLowerCase() &&
      run.head_repository?.full_name?.toLowerCase() === repository.toLowerCase() &&
      Number.isSafeInteger(run.id) && Number.isSafeInteger(run.run_attempt),
    "Validation run must belong to this repository and exact main commit",
  );
  requireControl(
    run.status === "completed" && run.conclusion === "success",
    "latest Validation run must have completed successfully before release",
  );
}

export function verifyReleaseControls({ repository, sourceSha, mainSha, run, jobs }) {
  requireControl(mainSha === sourceSha, "release commit must still be the tip of main");
  verifyRun(repository, sourceSha, run);
  requireControl(
    Array.isArray(jobs?.jobs) && jobs.total_count === jobs.jobs.length &&
      jobs.jobs.length === REQUIRED_STATUS_CHECK_CONTEXTS.length,
    "Validation jobs must contain the complete check inventory",
  );
  for (const name of REQUIRED_STATUS_CHECK_CONTEXTS) {
    const matches = jobs.jobs.filter(job => job.name === name);
    requireControl(matches.length === 1, `Validation must contain exactly one ${name} job`);
    const job = matches[0];
    // Dependency review only executes on PRs. The other five checks run on main.
    const expected = name === "Dependency review" ? "skipped" : "success";
    requireControl(
      job.run_id === run.id && job.run_attempt === run.run_attempt &&
        job.head_sha === sourceSha && job.status === "completed" && job.conclusion === expected,
      `${name} must be ${expected} in the latest Validation attempt for the release commit`,
    );
  }
  return true;
}

function githubApi(endpoint) {
  const result = spawnSync("gh", ["api", "-H", "X-GitHub-Api-Version: 2022-11-28", endpoint], {
    encoding: "utf8",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`could not verify release CI: ${result.stderr.trim() || result.status}`);
  }
  return JSON.parse(result.stdout);
}

export function verifyLiveReleaseControls(repository, sourceSha, request = githubApi) {
  requireControl(/^[\w.-]+\/[\w.-]+$/.test(repository), "invalid GitHub repository");
  requireControl(/^[a-f0-9]{40}$/.test(sourceSha), "release source must be a full 40-character SHA");
  const mainSha = request(`repos/${repository}/git/ref/heads/main`).object?.sha;
  requireControl(mainSha === sourceSha, "release commit must still be the tip of main");
  const response = request(
    `repos/${repository}/actions/workflows/validation.yml/runs?event=push&branch=main&head_sha=${sourceSha}&per_page=100`,
  );
  requireControl(Array.isArray(response?.workflow_runs), "missing Validation run response");
  // Never fall back to an older green run while a newer run is pending or failed.
  const run = [...response.workflow_runs].sort((left, right) => right.id - left.id)[0];
  verifyRun(repository, sourceSha, run);
  const jobs = request(`repos/${repository}/actions/runs/${run.id}/jobs?filter=latest&per_page=100`);
  return verifyReleaseControls({ repository, sourceSha, mainSha, run, jobs });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [repository, sourceSha] = process.argv.slice(2);
  if (!repository || !sourceSha) {
    console.error("usage: node scripts/verify-release-controls.mjs <owner/repository> <source-sha>");
    process.exit(2);
  }
  try {
    verifyLiveReleaseControls(repository, sourceSha);
    console.log(`Completed main validation verified for ${sourceSha}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
