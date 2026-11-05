#!/bin/sh
# Claude Code SessionStart hook (startup, resume, compact, fork).
# docs/HANDOVER.md §0.2: print the build issue's State and Blocked sections, so
# every restart and compaction lands back on track. Falls back to the offline
# cache notes/progress.md when gh can't reach GitHub. Never fails: a hook that
# errors would only hide the run state.
root=${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "$0")/../.." && pwd)}
cache=$root/notes/progress.md

section() { # print "## <name>" … up to the next "## " heading
  awk -v want="$1" '
    /^## / { on = (index($0, want) > 0) }
    on { print }
  '
}

body=""
if command -v gh >/dev/null 2>&1; then
  n=$(cd "$root" && gh issue list --label build-tracker --state open --json number --jq '.[0].number' 2>/dev/null)
  if [ -n "$n" ]; then
    body=$(cd "$root" && gh issue view "$n" --json body --jq .body 2>/dev/null)
    [ -n "$body" ] && echo "## 🔥 Firelight build: issue #$n (the run's single source of truth, docs/HANDOVER.md §0.2)"
  fi
fi
if [ -z "$body" ] && [ -f "$cache" ]; then
  echo "## notes/progress.md (offline cache of the build issue)"
  body=$(cat "$cache")
fi
if [ -z "$body" ]; then
  echo "No build issue and no notes/progress.md yet. Run plan: docs/HANDOVER.md §0 and §22."
  exit 0
fi
printf '%s\n' "$body" | section State | head -c 4000
echo
printf '%s\n' "$body" | section Blocked | head -c 4000
echo
echo "Resume: docs/HANDOVER.md §0.2 (read the full issue body, reconcile with gh pr list --author @me --state open, continue at Next action)."
exit 0
