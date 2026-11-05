#!/bin/sh
# Hardens prosperity-solutions/firelight and makes it public
# (docs/HANDOVER.md §17.3, §21.3).
#
# The OWNER runs this, in their own terminal, as a repository admin:
#   sh scripts/dev/harden-public-repo.sh
# Agents never run it; the agent guard blocks it. It is pure `gh api`,
# idempotent and safe to re-run, and ends by printing every setting as read back.
set -eu

REPO=prosperity-solutions/firelight
R=repos/$REPO
DOMAIN=firelight.oss.life.li

step() { printf '\n▶ %s\n' "$1"; }

# Create or update a ruleset by name.
ruleset() { # $1 name, $2 JSON body
  # This repository's own rulesets only, not ones inherited from the org.
  id=$(gh api "$R/rulesets?includes_parents=false" --jq ".[] | select(.name == \"$1\") | .id")
  if [ -n "$id" ]; then
    printf '%s' "$2" | gh api -X PUT "$R/rulesets/$id" --input - --silent
    echo "updated ruleset $1 ($id)"
  else
    printf '%s' "$2" | gh api -X POST "$R/rulesets" --input - --silent
    echo "created ruleset $1"
  fi
}

# --- 1. While still private -------------------------------------------------

# Every step reads the setting first and only writes when it differs: a
# re-run must not fail on a setting that is already right (GitHub refuses
# some writes once later settings, such as the workflow-execution policy,
# exist).
is_set() { # is_set <path> <jq expression that prints true when the setting is right>
  [ "$(gh api "$1" --jq "$2" 2>/dev/null)" = true ]
}
skip() { echo "already set"; }

step "Workflow token: read-only by default; Actions may not create or approve PRs"
if is_set "$R/actions/permissions/workflow" '.default_workflow_permissions == "read" and .can_approve_pull_request_reviews == false'; then
  skip
else
  gh api -X PUT "$R/actions/permissions/workflow" \
    -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false --silent
fi

step "Actions: SHA-pinned only; GitHub-owned plus an explicit list (order matters)"
if is_set "$R/actions/permissions" '.enabled == true and .allowed_actions == "selected" and .sha_pinning_required == true'; then
  skip
else
  printf '%s' '{"enabled": true, "allowed_actions": "selected", "sha_pinning_required": true}' |
    gh api -X PUT "$R/actions/permissions" --input - --silent
fi
if is_set "$R/actions/permissions/selected-actions" '.github_owned_allowed == true and .verified_allowed == false and (.patterns_allowed | sort) == ["step-security/harden-runner@*", "zizmorcore/zizmor-action@*"]'; then
  skip
else
  printf '%s' '{"github_owned_allowed": true, "verified_allowed": false, "patterns_allowed": ["step-security/harden-runner@*", "zizmorcore/zizmor-action@*"]}' |
    gh api -X PUT "$R/actions/permissions/selected-actions" --input - --silent
fi

step "Merges: squash only, commit title = PR title, delete branch on merge, no auto-merge"
if is_set "$R" '.allow_squash_merge == true and .allow_merge_commit == false and .allow_rebase_merge == false and .squash_merge_commit_title == "PR_TITLE" and .delete_branch_on_merge == true and .allow_auto_merge == false'; then
  skip
else
  gh api -X PATCH "$R" -F allow_squash_merge=true -F allow_merge_commit=false -F allow_rebase_merge=false \
    -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=PR_BODY \
    -F delete_branch_on_merge=true -F allow_auto_merge=false --silent
fi

step "Dependabot alerts, then security updates"
if gh api "$R/vulnerability-alerts" --silent 2>/dev/null; then
  skip
else
  gh api -X PUT "$R/vulnerability-alerts" --silent
fi
if is_set "$R/automated-security-fixes" '.enabled == true'; then
  skip
else
  gh api -X PUT "$R/automated-security-fixes" --silent
fi

step "Immutable releases"
if is_set "$R/immutable-releases" '.enabled == true'; then
  skip
else
  gh api -X PUT "$R/immutable-releases" --silent
fi

step "Labels"
gh label create build-tracker --repo "$REPO" --force --color D93F0B \
  --description "The pinned 🔥 Firelight build issue: the run's single source of truth"
gh label create no-ship --repo "$REPO" --force --color B60205 \
  --description "Owner waiver: skip the ship-stamp check"
gh label create needs-maintainer --repo "$REPO" --force --color FBCA04 \
  --description "Needs the owner: a decision, access, or a PR the agent must not touch"

# --- 2. Public --------------------------------------------------------------

step "Visibility: public"
if [ "$(gh api "$R" --jq .visibility)" = public ]; then
  echo "already public"
else
  gh repo edit "$REPO" --visibility public --accept-visibility-change-consequences
fi

# --- 3. Public-only settings ------------------------------------------------

step "Secret scanning and push protection"
if is_set "$R" '.security_and_analysis.secret_scanning.status == "enabled" and .security_and_analysis.secret_scanning_push_protection.status == "enabled"'; then
  skip
else
  gh api -X PATCH "$R" -f 'security_and_analysis[secret_scanning][status]=enabled' \
    -f 'security_and_analysis[secret_scanning_push_protection][status]=enabled' --silent
fi

step "Private vulnerability reporting (SECURITY.md points to it)"
if is_set "$R/private-vulnerability-reporting" '.enabled == true'; then
  skip
else
  gh api -X PUT "$R/private-vulnerability-reporting" --silent
fi

step "Fork PRs: approval required for all external contributors"
if is_set "$R/actions/permissions/fork-pr-contributor-approval" '.approval_policy == "all_external_contributors"'; then
  skip
else
  gh api -X PUT "$R/actions/permissions/fork-pr-contributor-approval" \
    -f approval_policy=all_external_contributors --silent
fi

step "Workflow execution protection: only push and pull_request may start a workflow"
# No `|| true` here: if the GET fails, stop rather than assume the policy exists.
existing=$(gh api "$R/actions/policies" --jq '.. | objects | select(.name? == "events-push-pr") | .name')
if [ "$existing" = events-push-pr ]; then
  echo "policy events-push-pr already exists"
else
  printf '%s' '{"name": "events-push-pr", "enforcement": "active", "rules": [{"type": "restrict_action_events", "parameters": {"allowed_events": ["push", "pull_request"]}}]}' |
    gh api -X POST "$R/actions/policies" --input - --silent
  echo "created policy events-push-pr"
fi

# --- 4. Rulesets (after going public) ---------------------------------------

step "Ruleset main: PR required, all-green required, branch up to date, signed commits, no bypass"
# require_extra_approval_for_unattributed_changes is stated as false: GitHub
# turns it on when a ruleset omits it, and its extra approval must come from
# someone other than the PR author, which a sole owner can never give
# (docs/HANDOVER.md §0.1: 0 approvals, nobody reviews).
sha=$(gh api "$R/commits/main" --jq .sha)
app=$(gh api "$R/commits/$sha/check-runs?check_name=all-green" --jq '.check_runs[0].app.id // empty')
if [ -z "$app" ]; then
  echo "No all-green check run on main ($sha). Wait for the bootstrap CI run to finish, then re-run this script." >&2
  exit 1
fi
ruleset main "$(cat <<EOF
{"name": "main", "target": "branch", "enforcement": "active", "bypass_actors": [],
 "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": []}},
 "rules": [
  {"type": "deletion"},
  {"type": "non_fast_forward"},
  {"type": "required_signatures"},
  {"type": "pull_request", "parameters": {
    "required_approving_review_count": 0, "require_code_owner_review": false,
    "dismiss_stale_reviews_on_push": false, "require_last_push_approval": false,
    "required_review_thread_resolution": true, "allowed_merge_methods": ["squash"],
    "require_extra_approval_for_unattributed_changes": false}},
  {"type": "required_status_checks", "parameters": {
    "strict_required_status_checks_policy": true, "do_not_enforce_on_create": false,
    "required_status_checks": [{"context": "all-green", "integration_id": $app}]}}
 ]}
EOF
)"

step "Ruleset tags-v: release tags can be created, never moved or deleted"
ruleset tags-v '{"name": "tags-v", "target": "tag", "enforcement": "active", "bypass_actors": [],
 "conditions": {"ref_name": {"include": ["refs/tags/v*"], "exclude": []}},
 "rules": [{"type": "update", "parameters": {"update_allows_fetch_and_merge": false}}, {"type": "deletion"}]}'

# --- 5. GitHub Pages (docs/HANDOVER.md §15.1) --------------------------------

step "GitHub Pages: deployed by CI's pages job, at $DOMAIN"
# The DNS record comes first: CNAME firelight.oss.life.li → prosperity-solutions.github.io.
if ! gh api "$R/pages" --silent 2>/dev/null; then
  gh api -X POST "$R/pages" -f build_type=workflow --silent
  echo "Pages turned on"
elif is_set "$R/pages" '.build_type == "workflow"'; then
  echo "Pages already on, deployed by workflow"
else
  gh api -X PUT "$R/pages" -f build_type=workflow --silent
fi
if is_set "$R/pages" ".cname == \"$DOMAIN\""; then
  echo "custom domain already $DOMAIN"
else
  gh api -X PUT "$R/pages" -f cname="$DOMAIN" --silent
fi
# Only main may deploy the site.
if is_set "$R/environments/github-pages" '.deployment_branch_policy.custom_branch_policies == true and .deployment_branch_policy.protected_branches == false'; then
  echo "github-pages already uses a custom branch policy"
else
  printf '%s' '{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}' |
    gh api -X PUT "$R/environments/github-pages" --input - --silent
fi
if gh api "$R/environments/github-pages/deployment-branch-policies" --jq '.branch_policies[].name' | grep -qx main; then
  echo "github-pages already deploys from main only"
else
  gh api -X POST "$R/environments/github-pages/deployment-branch-policies" -f name=main -f type=branch --silent
  echo "github-pages now deploys from main only"
fi
# GitHub issues the certificate a few minutes after the domain is set.
if is_set "$R/pages" '.https_enforced == true'; then
  echo "HTTPS already enforced"
elif err=$(gh api -X PUT "$R/pages" -F https_enforced=true --silent 2>&1); then
  echo "HTTPS enforced"
else
  printf 'HTTPS not enforced yet (%s). Usually the certificate is still being issued: re-run this script in 15 minutes.\n' "$err" >&2
fi

# --- 6. Summary, as read back -----------------------------------------------

step "Summary (read back from GitHub)"
gh api "$R" --jq '{visibility, allow_squash_merge, allow_merge_commit, allow_rebase_merge, squash_merge_commit_title, delete_branch_on_merge, allow_auto_merge, security_and_analysis}'
gh api "$R/actions/permissions/workflow"
gh api "$R/actions/permissions"
gh api "$R/actions/permissions/selected-actions"
gh api "$R/actions/permissions/fork-pr-contributor-approval"
gh api "$R/actions/policies" || true
printf 'vulnerability alerts: '
if gh api "$R/vulnerability-alerts" --silent 2>/dev/null; then echo enabled; else echo DISABLED; fi
printf 'automated security fixes: '; gh api "$R/automated-security-fixes" --jq .enabled
printf 'immutable releases: '; gh api "$R/immutable-releases" --jq .enabled
printf 'private vulnerability reporting: '; gh api "$R/private-vulnerability-reporting" --jq .enabled
gh api "$R/pages" --jq '{build_type, cname, https_enforced}'
gh api "$R/environments/github-pages" --jq .deployment_branch_policy
printf 'github-pages deploys from: '
gh api "$R/environments/github-pages/deployment-branch-policies" --jq '[.branch_policies[].name] | join(" ")' || echo NONE
for id in $(gh api "$R/rulesets" --jq '.[].id'); do
  gh api "$R/rulesets/$id" --jq '{name, target, enforcement, bypass_actors, conditions, rules}'
done
printf 'repository secrets: '; gh api "$R/actions/secrets" --jq .total_count
gh label list --repo "$REPO" --search build-tracker
gh label list --repo "$REPO" --search no-ship
gh label list --repo "$REPO" --search needs-maintainer

printf '\n✅ Done. Tell Claude; it verifies every setting with read-only calls.\n'
