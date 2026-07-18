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
export const GITHUB_ACTIONS_APP_ID = 15_368;
export const GITHUB_API_VERSION = "2022-11-28";
const SORTED_REQUIRED_STATUS_CHECK_CONTEXTS = [...REQUIRED_STATUS_CHECK_CONTEXTS].sort();

function requireControl(condition, message) {
  if (!condition) throw new Error(message);
}

export function verifyReleaseControls({
  immutableReleases,
  branchProtection,
  environment,
  deploymentBranchPolicies,
}) {
  requireControl(
    immutableReleases?.enabled === true,
    "repository immutable releases must be enabled before signing or publication",
  );

  const reviews = branchProtection?.required_pull_request_reviews;
  requireControl(
    Number.isInteger(reviews?.required_approving_review_count) &&
      reviews.required_approving_review_count >= 1,
    "main branch protection must require at least one approving review",
  );
  requireControl(
    reviews?.dismiss_stale_reviews === true,
    "main branch protection must dismiss stale approving reviews",
  );
  requireControl(
    reviews?.require_last_push_approval === true,
    "main branch protection must require approval of the last push",
  );
  const reviewBypasses = reviews?.bypass_pull_request_allowances;
  requireControl(
    reviewBypasses &&
      ["users", "teams", "apps"].every(
        (kind) => Array.isArray(reviewBypasses[kind]) && reviewBypasses[kind].length === 0,
      ),
    "main branch protection must prohibit all user, team, and app review bypass allowances",
  );

  const statusChecks = branchProtection?.required_status_checks;
  requireControl(
    statusChecks?.strict === true,
    "main branch protection must require strict status checks",
  );
  const statusCheckBindings = statusChecks?.checks;
  const actualStatusCheckContexts = Array.isArray(statusCheckBindings)
    ? statusCheckBindings.map((check) => check?.context)
    : [];
  const sortedActualStatusCheckContexts = Array.isArray(statusCheckBindings)
    ? [...actualStatusCheckContexts].sort()
    : [];
  requireControl(
    Array.isArray(statusCheckBindings) &&
      statusCheckBindings.every((check) => check?.app_id === GITHUB_ACTIONS_APP_ID) &&
      actualStatusCheckContexts.every(
        (context) => typeof context === "string" && context.trim() === context && context.length > 0,
      ) &&
      new Set(actualStatusCheckContexts).size === actualStatusCheckContexts.length &&
      actualStatusCheckContexts.length === REQUIRED_STATUS_CHECK_CONTEXTS.length &&
      sortedActualStatusCheckContexts.every(
        (context, index) => context === SORTED_REQUIRED_STATUS_CHECK_CONTEXTS[index],
      ),
    `main branch protection must bind exactly these status checks to GitHub Actions app ${GITHUB_ACTIONS_APP_ID}: ${REQUIRED_STATUS_CHECK_CONTEXTS.join(
      ", ",
    )}`,
  );
  requireControl(
    branchProtection?.enforce_admins?.enabled === true,
    "main branch protection must include administrators",
  );
  requireControl(
    branchProtection?.allow_force_pushes?.enabled === false,
    "main branch protection must reject force pushes",
  );
  requireControl(
    branchProtection?.allow_deletions?.enabled === false,
    "main branch protection must reject deletion",
  );
  requireControl(
    branchProtection?.required_conversation_resolution?.enabled === true,
    "main branch protection must require conversation resolution",
  );

  requireControl(environment?.name === "release", "protected release environment is missing");
  const reviewerRule = environment.protection_rules?.find(
    (rule) => rule.type === "required_reviewers",
  );
  requireControl(
    Array.isArray(reviewerRule?.reviewers) && reviewerRule.reviewers.length > 0,
    "release environment must require at least one reviewer",
  );
  requireControl(
    reviewerRule.prevent_self_review === true,
    "release environment must prevent self-review",
  );
  requireControl(
    environment.can_admins_bypass === false,
    "release environment must prevent administrator bypass",
  );
  requireControl(
    environment.deployment_branch_policy?.protected_branches === false &&
      environment.deployment_branch_policy?.custom_branch_policies === true,
    "release environment must use a custom deployment branch policy",
  );
  const branchPolicies = deploymentBranchPolicies?.branch_policies;
  requireControl(
    deploymentBranchPolicies?.total_count === 1 &&
      Array.isArray(branchPolicies) &&
      branchPolicies.length === 1 &&
      branchPolicies[0].name === "main" &&
      branchPolicies[0].type === "branch",
    "release environment must allow only the main branch",
  );
  return true;
}

export function githubApiArguments(endpoint) {
  return ["api", "-H", `X-GitHub-Api-Version: ${GITHUB_API_VERSION}`, endpoint];
}

function githubApi(endpoint) {
  const result = spawnSync("gh", githubApiArguments(endpoint), { encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `could not verify release control ${endpoint}: ${result.stderr.trim() || `gh exited with status ${result.status}`}`,
    );
  }
  return JSON.parse(result.stdout);
}

export function verifyLiveReleaseControls(repository) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository: ${repository}`);
  }
  requireControl(
    typeof process.env.GH_TOKEN === "string" && process.env.GH_TOKEN.trim().length > 0,
    "release admin-read credential is missing",
  );
  return verifyReleaseControls({
    immutableReleases: githubApi(`repos/${repository}/immutable-releases`),
    branchProtection: githubApi(`repos/${repository}/branches/main/protection`),
    environment: githubApi(`repos/${repository}/environments/release`),
    deploymentBranchPolicies: githubApi(
      `repos/${repository}/environments/release/deployment-branch-policies`,
    ),
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const repository = process.argv[2];
  if (!repository) {
    console.error("usage: node scripts/verify-release-controls.mjs <owner/repository>");
    process.exit(2);
  }
  try {
    verifyLiveReleaseControls(repository);
    console.log("immutable releases, protected main, and release environment verified");
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
