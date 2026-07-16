import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import {
  GITHUB_ACTIONS_APP_ID,
  GITHUB_API_VERSION,
  REQUIRED_STATUS_CHECK_CONTEXTS,
  githubApiArguments,
  verifyLiveReleaseControls,
  verifyReleaseControls,
} from "./verify-release-controls.mjs";

const releaseWorkflow = fs.readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url),
  "utf8",
);

function workflowJob(name) {
  const match = releaseWorkflow.match(
    new RegExp(`^  ${name}:\\n[\\s\\S]*?(?=^  [a-z][a-z0-9_-]*:\\n|(?![\\s\\S]))`, "m"),
  );
  assert.ok(match, `release workflow job ${name} must exist`);
  return match[0];
}

function workflowSteps(job) {
  const steps = job.split("\n    steps:\n")[1];
  assert.ok(steps, "workflow job must define steps");
  return steps.split(/^      - /m).slice(1);
}

function validControls() {
  return {
    immutableReleases: { enabled: true, enforced_by_owner: false },
    branchProtection: {
      required_pull_request_reviews: {
        required_approving_review_count: 1,
        dismiss_stale_reviews: true,
        require_last_push_approval: true,
        bypass_pull_request_allowances: { users: [], teams: [], apps: [] },
      },
      required_status_checks: {
        strict: true,
        checks: REQUIRED_STATUS_CHECK_CONTEXTS.map((context) => ({
          context,
          app_id: GITHUB_ACTIONS_APP_ID,
        })),
      },
      enforce_admins: { enabled: true },
      allow_force_pushes: { enabled: false },
      allow_deletions: { enabled: false },
      required_conversation_resolution: { enabled: true },
    },
    environment: {
      name: "release",
      protection_rules: [
        {
          type: "required_reviewers",
          prevent_self_review: true,
          reviewers: [{ type: "User", reviewer: { login: "release-reviewer" } }],
        },
      ],
      can_admins_bypass: false,
      deployment_branch_policy: {
        protected_branches: false,
        custom_branch_policies: true,
      },
    },
    deploymentBranchPolicies: {
      total_count: 1,
      branch_policies: [{ name: "main", type: "branch" }],
    },
  };
}

test("accepts immutable releases, reviewed main, and a protected release environment", () => {
  assert.equal(GITHUB_ACTIONS_APP_ID, 15_368);
  assert.equal(GITHUB_API_VERSION, "2022-11-28");
  assert.deepEqual(githubApiArguments("repos/owner/repository"), [
    "api",
    "-H",
    "X-GitHub-Api-Version: 2022-11-28",
    "repos/owner/repository",
  ]);
  assert.deepEqual(REQUIRED_STATUS_CHECK_CONTEXTS, [
    "Repository policy",
    "Dependency review",
    "Windows validation",
    "Linux validation",
    "Linux package transport",
    "macOS universal validation",
  ]);
  assert.equal(verifyReleaseControls(validControls()), true);
});

test("rejects mutable repository releases", () => {
  const controls = validControls();
  controls.immutableReleases.enabled = false;
  assert.throws(() => verifyReleaseControls(controls), /immutable releases must be enabled/);
});

test("rejects incomplete main branch protection", () => {
  const noReviews = validControls();
  noReviews.branchProtection.required_pull_request_reviews = null;
  assert.throws(() => verifyReleaseControls(noReviews), /approving review/);

  const forcePushes = validControls();
  forcePushes.branchProtection.allow_force_pushes.enabled = true;
  assert.throws(() => verifyReleaseControls(forcePushes), /reject force pushes/);

  const deletions = validControls();
  deletions.branchProtection.allow_deletions.enabled = true;
  assert.throws(() => verifyReleaseControls(deletions), /reject deletion/);

  const noStatusChecks = validControls();
  noStatusChecks.branchProtection.required_status_checks = null;
  assert.throws(() => verifyReleaseControls(noStatusChecks), /strict status checks/);

  const nonStrictStatusChecks = validControls();
  nonStrictStatusChecks.branchProtection.required_status_checks.strict = false;
  assert.throws(() => verifyReleaseControls(nonStrictStatusChecks), /strict status checks/);

  const adminBypass = validControls();
  adminBypass.branchProtection.enforce_admins.enabled = false;
  assert.throws(() => verifyReleaseControls(adminBypass), /include administrators/);
});

test("rejects review settings that allow stale or unreviewed changes", () => {
  const staleReviews = validControls();
  staleReviews.branchProtection.required_pull_request_reviews.dismiss_stale_reviews = false;
  assert.throws(() => verifyReleaseControls(staleReviews), /dismiss stale approving reviews/);

  const unreviewedLastPush = validControls();
  unreviewedLastPush.branchProtection.required_pull_request_reviews.require_last_push_approval = false;
  assert.throws(() => verifyReleaseControls(unreviewedLastPush), /approval of the last push/);

  const unresolvedConversations = validControls();
  unresolvedConversations.branchProtection.required_conversation_resolution.enabled = false;
  assert.throws(() => verifyReleaseControls(unresolvedConversations), /conversation resolution/);
});

test("rejects every pull request review bypass allowance", () => {
  for (const kind of ["users", "teams", "apps"]) {
    const controls = validControls();
    controls.branchProtection.required_pull_request_reviews.bypass_pull_request_allowances[
      kind
    ].push({ id: 1 });
    assert.throws(
      () => verifyReleaseControls(controls),
      /prohibit all user, team, and app review bypass allowances/,
      `${kind} bypasses must fail closed`,
    );
  }

  const missingAllowances = validControls();
  delete missingAllowances.branchProtection.required_pull_request_reviews
    .bypass_pull_request_allowances;
  assert.throws(
    () => verifyReleaseControls(missingAllowances),
    /prohibit all user, team, and app review bypass allowances/,
  );
});

test("requires the exact GitHub Actions validation status check bindings", () => {
  const missing = validControls();
  missing.branchProtection.required_status_checks.checks.pop();
  assert.throws(() => verifyReleaseControls(missing), /bind exactly these status checks/);

  const extra = validControls();
  extra.branchProtection.required_status_checks.checks.push({
    context: "Unapproved check",
    app_id: GITHUB_ACTIONS_APP_ID,
  });
  assert.throws(() => verifyReleaseControls(extra), /bind exactly these status checks/);

  const arbitrary = validControls();
  arbitrary.branchProtection.required_status_checks.checks = [
    { context: "Validation", app_id: GITHUB_ACTIONS_APP_ID },
  ];
  assert.throws(() => verifyReleaseControls(arbitrary), /bind exactly these status checks/);

  const duplicate = validControls();
  duplicate.branchProtection.required_status_checks.checks[1].context =
    REQUIRED_STATUS_CHECK_CONTEXTS[0];
  assert.throws(() => verifyReleaseControls(duplicate), /bind exactly these status checks/);

  const ambiguousLegacyShape = validControls();
  ambiguousLegacyShape.branchProtection.required_status_checks = {
    strict: true,
    contexts: [...REQUIRED_STATUS_CHECK_CONTEXTS],
  };
  assert.throws(
    () => verifyReleaseControls(ambiguousLegacyShape),
    /bind exactly these status checks/,
  );

  const missingApp = validControls();
  delete missingApp.branchProtection.required_status_checks.checks[0].app_id;
  assert.throws(
    () => verifyReleaseControls(missingApp),
    /bind exactly these status checks to GitHub Actions app 15368/,
  );

  for (const appId of [null, -1, 99_999, "15368"]) {
    const wrongApp = validControls();
    wrongApp.branchProtection.required_status_checks.checks[0].app_id = appId;
    assert.throws(
      () => verifyReleaseControls(wrongApp),
      /bind exactly these status checks to GitHub Actions app 15368/,
      `app_id ${String(appId)} must fail closed`,
    );
  }

  const duplicateBinding = validControls();
  duplicateBinding.branchProtection.required_status_checks.checks.push({
    ...duplicateBinding.branchProtection.required_status_checks.checks[0],
  });
  assert.throws(
    () => verifyReleaseControls(duplicateBinding),
    /bind exactly these status checks to GitHub Actions app 15368/,
  );
});

test("rejects an environment that can bypass review or deploy unprotected refs", () => {
  const noReviewers = validControls();
  noReviewers.environment.protection_rules[0].reviewers = [];
  assert.throws(() => verifyReleaseControls(noReviewers), /at least one reviewer/);

  const selfReview = validControls();
  selfReview.environment.protection_rules[0].prevent_self_review = false;
  assert.throws(() => verifyReleaseControls(selfReview), /prevent self-review/);

  const adminBypass = validControls();
  adminBypass.environment.can_admins_bypass = true;
  assert.throws(() => verifyReleaseControls(adminBypass), /administrator bypass/);

  const unprotectedRefs = validControls();
  unprotectedRefs.environment.deployment_branch_policy = {
    protected_branches: true,
    custom_branch_policies: false,
  };
  assert.throws(() => verifyReleaseControls(unprotectedRefs), /custom deployment branch policy/);

  const broadPolicy = validControls();
  broadPolicy.deploymentBranchPolicies = {
    total_count: 2,
    branch_policies: [
      { name: "main", type: "branch" },
      { name: "release/*", type: "branch" },
    ],
  };
  assert.throws(() => verifyReleaseControls(broadPolicy), /allow only the main branch/);

  const missingType = validControls();
  delete missingType.deploymentBranchPolicies.branch_policies[0].type;
  assert.throws(() => verifyReleaseControls(missingType), /allow only the main branch/);

  const tagPolicy = validControls();
  tagPolicy.deploymentBranchPolicies.branch_policies[0].type = "tag";
  assert.throws(() => verifyReleaseControls(tagPolicy), /allow only the main branch/);

  const wrongBranch = validControls();
  wrongBranch.deploymentBranchPolicies.branch_policies[0].name = "release";
  assert.throws(() => verifyReleaseControls(wrongBranch), /allow only the main branch/);
});

test("fails before invoking GitHub when the admin-read credential is missing", () => {
  const originalToken = process.env.GH_TOKEN;
  delete process.env.GH_TOKEN;
  try {
    assert.throws(
      () => verifyLiveReleaseControls("TheGreenCedar/BatCave"),
      /admin-read credential is missing/,
    );
  } finally {
    if (originalToken === undefined) delete process.env.GH_TOKEN;
    else process.env.GH_TOKEN = originalToken;
  }
});

test("runs release controls first with only the protected environment credential", () => {
  const prepare = workflowJob("prepare");
  assert.doesNotMatch(prepare, /verify-release-controls\.mjs/);

  const sensitiveJobs = [
    "windows",
    "linux",
    "macos",
    "finalize",
    "linux_deb_post_public_smoke",
    "linux_appimage_post_public_smoke",
    "macos_updater_post_public_smoke",
  ];
  for (const name of sensitiveJobs) {
    const job = workflowJob(name);
    assert.match(job, /^    environment: release$/m, `${name} must use the release environment`);

    const steps = workflowSteps(job);
    const controlSteps = steps.filter((step) => step.includes("verify-release-controls.mjs"));
    assert.equal(controlSteps.length, 1, `${name} must run one release control check`);
    assert.match(
      controlSteps[0],
      /GH_TOKEN: \$\{\{ secrets\.RELEASE_ADMIN_READ_TOKEN \}\}/,
      `${name} must use the protected environment credential`,
    );
    assert.equal(
      controlSteps[0].match(/\$\{\{ secrets\./g)?.length,
      1,
      `${name} control check must receive no other secret`,
    );
    assert.doesNotMatch(controlSteps[0], /github\.token/);

    const controlIndex = steps.indexOf(controlSteps[0]);
    assert.ok(
      steps.slice(0, controlIndex).every((step) => !step.includes("${{ secrets.")),
      `${name} must not read a signing secret before control verification`,
    );
    const firstRunStep = steps.find((step) => /(?:^|\n)        run:/.test(step));
    assert.equal(
      firstRunStep,
      controlSteps[0],
      `${name} must verify controls before any other command`,
    );
  }

  assert.equal(releaseWorkflow.match(/verify-release-controls\.mjs/g)?.length, 7);
  assert.equal(
    releaseWorkflow.match(/GH_TOKEN: \$\{\{ secrets\.RELEASE_ADMIN_READ_TOKEN \}\}/g)?.length,
    7,
  );
});

test("runs the deb smoke on a fresh pinned Ubuntu host after public release publication", () => {
  const job = workflowJob("linux_deb_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /node scripts\/linux-deb-post-public-smoke\.mjs "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-input/u,
  );
  assert.match(
    job,
    /name: Retain sanitized Linux deb post-public observation[\s\S]*name: batcave-linux-deb-post-public-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-output\/linux-deb-observation\.json/u,
  );
  assert.doesNotMatch(job, /(?:--deb|--output-dir|RUNNER_TEMP|github\.event|workflow_dispatch)/u);
});

test("runs the AppImage smoke from the same independent public candidate inventory", () => {
  const job = workflowJob("linux_appimage_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: ubuntu-22\.04$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /dtolnay\/rust-toolchain@[0-9a-f]{40}[\s\S]*bash scripts\/install-linux-deps\.sh[\s\S]*cargo build --quiet --locked[\s\S]*--bin batcave-verify-updater-signature/u,
  );
  assert.match(
    job,
    /node scripts\/linux-appimage-post-public-smoke\.mjs "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}[\s\S]*path: post-public-input/u,
  );
  assert.match(
    job,
    /name: Retain sanitized Linux AppImage post-public observation[\s\S]*path: post-public-output\/linux-appimage-observation\.json/u,
  );
  assert.doesNotMatch(
    job,
    /(?:--appimage|--output-dir|RUNNER_TEMP|github\.event|workflow_dispatch)/u,
  );
});

test("runs the macOS updater observer through the closed Rust-owned staging profile", () => {
  const job = workflowJob("macos_updater_post_public_smoke");
  assert.match(job, /^    needs: \[prepare, finalize\]$/m);
  assert.match(job, /^    if: needs\.prepare\.outputs\.publish == 'true'$/m);
  assert.match(job, /^    runs-on: macos-15$/m);
  assert.match(job, /ref: \$\{\{ needs\.prepare\.outputs\.source_sha \}\}/u);
  assert.match(
    job,
    /cargo run --quiet --locked[\s\S]*--bin batcave-install-smoke --features private-release-verifier -- "\$\{RELEASE_TAG\}" macos-updater/u,
  );
  assert.match(
    job,
    /node scripts\/validate-macos-updater-post-public-observation\.mjs "\$\{observation\}" "\$\{RELEASE_TAG\}" "\$\{RELEASE_SOURCE_SHA\}"/u,
  );
  assert.match(
    job,
    /name: Retain sanitized macOS updater post-public observation[\s\S]*path: post-public-output\/macos-updater-observation\.json/u,
  );
  assert.doesNotMatch(job, /macos-dmg|hdiutil|(?:--archive|--signature|--output-dir|RUNNER_TEMP)/u);
});

test("gates pre-attestation and complete release inventories before unconditional upload", () => {
  const steps = workflowSteps(workflowJob("finalize"));
  const stepIndex = (label) => steps.findIndex((step) => step.includes(`name: ${label}`));
  const checksums = stepIndex("Generate checksums");
  const preAttestation = stepIndex("Verify pre-attestation release inventory");
  const attest = stepIndex("Generate build provenance");
  const retain = stepIndex("Retain provenance with release files");
  const complete = stepIndex("Verify complete release inventory");
  const upload = steps.findIndex(
    (step) =>
      step.includes("actions/upload-artifact@") &&
      step.includes("name: batcave-release-${{ needs.prepare.outputs.tag }}"),
  );
  const candidateUpload = stepIndex("Retain exact pre-publication candidate inventory");
  const create = stepIndex("Create and verify draft GitHub Release");

  for (const [label, index] of [
    ["checksums", checksums],
    ["pre-attestation inventory", preAttestation],
    ["attestation", attest],
    ["retained provenance", retain],
    ["complete inventory", complete],
    ["final artifact upload", upload],
    ["pre-publication candidate upload", candidateUpload],
    ["draft release", create],
  ]) {
    assert.ok(index >= 0, `finalize must contain ${label}`);
  }
  assert.ok(checksums < preAttestation && preAttestation < attest);
  assert.ok(
    attest < retain &&
      retain < complete &&
      complete < upload &&
      upload < candidateUpload &&
      candidateUpload < create,
  );

  assert.match(
    steps[preAttestation],
    /verify-release-candidate\.mjs verify-inventory .* pre-attestation dist/u,
  );
  assert.match(steps[complete], /verify-release-candidate\.mjs inventory .* dist /u);
  assert.doesNotMatch(steps[preAttestation], /^\s*if:/mu);
  assert.doesNotMatch(steps[complete], /^\s*if:/mu);
  assert.doesNotMatch(steps[upload], /^\s*if:/mu);
  assert.match(
    steps[candidateUpload],
    /name: batcave-release-candidate-\$\{\{ needs\.prepare\.outputs\.tag \}\}/u,
  );
  assert.match(steps[candidateUpload], /path: \$\{\{ runner\.temp \}\}\/release-candidate\.json/u);
  assert.doesNotMatch(steps[candidateUpload], /^\s*if:/mu);
});
