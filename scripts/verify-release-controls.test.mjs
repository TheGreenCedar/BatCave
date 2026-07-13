import assert from "node:assert/strict";
import test from "node:test";
import { verifyReleaseControls } from "./verify-release-controls.mjs";

function validControls() {
  return {
    immutableReleases: { enabled: true, enforced_by_owner: false },
    branchProtection: {
      required_pull_request_reviews: { required_approving_review_count: 1 },
      required_status_checks: { strict: true, checks: [{ context: "Validation" }] },
      enforce_admins: { enabled: true },
      allow_force_pushes: { enabled: false },
      allow_deletions: { enabled: false },
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

  const adminBypass = validControls();
  adminBypass.branchProtection.enforce_admins.enabled = false;
  assert.throws(() => verifyReleaseControls(adminBypass), /include administrators/);
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
});
