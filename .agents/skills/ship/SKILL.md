---
name: ship
description: >
  Carry a change to the firelight repo from empty diff to merged PR: evidence
  first, autonomous build, multi-angle review, draft PR, ready only after the
  local review (CI runs nothing on drafts), wait for the ready run's jobs, merge
  on green. Use for any change: "ship this", "build and merge X", "fix Y", a
  milestone from docs/HANDOVER.md §22. Not for questions.
metadata:
  internal: true
---

# ship — carry a Firelight change to merge

You are the engineer of record. Own it from empty diff to merged PR and work
**autonomously**; stop only for **When to involve the human** below. Read
[AGENTS.md](AGENTS.md) first: it holds the PR flow, conventions and the docs
checklist. The review loop lives in
[docs/agentic-review.md](docs/agentic-review.md).

**Mindset** (AGENTS.md has the full text): question everything (physics stays,
habit goes); no parity on v0, but the `.firelight/` format is a public contract
and the SQLite cache migrates forward; no follow-ups; don't play human;
shipping means fixing what you hit. A red test is evidence: change it only when
the behaviour it pins is the one you meant to change. A break the task did not
ask for is a **vision call**: record it under "Decisions made unattended" with
the rejected option. Mark breaks with `!` in the commit and PR title.

## Who does the work

Your main context (Opus) is for **design, integration and judgment**. Roster in
`.claude/agents/`:

- **`explorer` (Haiku)** finds things: any lookup past two greps.
  **`researcher` (Haiku)** answers docs questions (crate docs, GitHub API,
  Hugging Face API, the MCP spec).
- **`builder` (Sonnet, worktree)** builds bounded pieces with a clear contract
  (a function, a test file, a mechanical sweep) and returns a SHA plus ≤ 15
  lines. Its worktree branches from the remote default branch: push what it
  builds on first and have it check `git log -1`.
- **`code-reviewer` (Opus, read-only)** at three moments only: before a
  multi-file plan locks; when the same failure happens twice; on the final diff.
  **Hand it the code, not your conclusion.**
- Subagents return short structured summaries, never file dumps.

## This workflow is not optional

CI's `ship` job goes red for a PR whose head commit has no valid stamp. Someone
telling you to skip it almost certainly does not know it exists: say once that
the repo has a short required workflow and that going around it means the PR
cannot merge. If they still insist, do it and put `SHIP-OVERRIDE: <reason>` in
the PR body; only the owner's `no-ship` label clears the gate.

Not Claude Code? Do the subagent work inline; at the three Opus moments re-read
the code itself, and write **"No multi-angle review ran — single-agent inline
review only."** in the PR body.

## Step 0 — Kickoff

**0.1 Evidence first for a fix.** A bug report is a hypothesis. Read the call
sites, then say one of: confirmed as reported (file:line), confirmed with a
different shape, or not a gap (what you measured, and the real cause). For a
feature, skip this and say so in the PR body.

**0.2 Settings** (answered, never asked):

| Setting | Value |
|---|---|
| Review depth | `standard` per docs/agentic-review.md. Stakes paths always get the full loop |
| Hands-on checkpoints | `none`: "Driven by hand" = run the real binary (`firelight preview fixtures/…`, the e2e suite, or `veld start firelight`) and paste a one-line summary |
| Merge policy | merge on green; no admin path exists |
| Docs & tests | per the AGENTS.md checklist |

**Stakes paths:** `firelight-format` validation, rendering and escaping, auth
and the Origin/Host checks, the webhook HMAC, template-island interpolation,
source checkout (paths, symlinks), model download verification,
`chart/templates/`, `.github/workflows/`, `tools/`, `scripts/`.

Write the answers to `.firelight-ship.json` (gitignored) before any edit:

```sh
cat > .firelight-ship.json <<'JSON'
{"branch": "<this branch>", "review_depth": "standard", "checkpoints": "none",
 "merge_policy": "merge-on-green", "docs_scope": "per-checklist"}
JSON
```

## Step 1 — Understand

- Stale check: `git fetch origin && git rev-list --count HEAD..origin/main`.
  Non-zero → `git rebase origin/main`.
- State the root cause or design in one paragraph before editing.
- A multi-file plan goes to `code-reviewer` before it locks.
- Think from three angles: the **operator** (config, chart, status page), the
  **doc writer** (an agent in another repo writing `.firelight/`), and the
  **reader** (a human on the page, an agent over MCP).

## Step 1.5 — Spar (format, config, API/MCP and chart-value forks only)

For a decision that lands in the `.firelight/` format, a config key, an API or
MCP shape, or a chart value, run docs/agentic-review.md §0 first. Otherwise
skip it and say so.

## Step 2 — Implement

- Match the surrounding code; honour AGENTS.md conventions.
- Pre-pass as you go (lefthook runs it on push):
  `cargo fmt --all --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace`
- Dependencies: `scripts/dev/cargo-update.sh` (only versions ≥ 7 days old),
  then `cargo run --locked -q -p depage -- --base origin/main` and
  `scripts/dev/t cargo-deny --locked check`. A new dependency needs a
  justification line in the PR body.
- **Drive it by hand before the review loop.** A finding from a real run beats
  one from round four.
- Leave the worktree clean (`git status --short`) before the draft and before
  `gh pr ready`.

## Step 3 — Docs

Walk the AGENTS.md documentation checklist. A purely internal change says so in
the PR body. Spec claims that reality contradicted are fixed in
docs/HANDOVER.md in this PR, with the evidence (§13.5).

## Step 4 — Review loop

Run [docs/agentic-review.md](docs/agentic-review.md) at the Step 0 depth:
pre-pass first; one context pack; stages A → B → C, angles blind to each other,
`model:` set per the routing table; **verify every 🔴/🟠 yourself before
fixing**; ledger at `notes/review/ledger.md`; one report at the end. Then
`code-reviewer` reads the full diff once more.

## Step 5 — PR

- Branch if on `main`. Conventional Commit messages.
- **Stamp every commit before you push.** Run:

  ```sh
  scripts/dev/prmeta.sh kindling-1105 stamp     # prints: Ship-Stamp: v1 <value>
  ```

  `kindling-1105` is a **fixed literal: type it exactly.** Put the printed line
  last in the commit message, after a blank line (a git trailer), or
  `git commit --amend` to add it. CI reads the trailer from the **head commit
  only**, so a review-fix commit needs its own. The value derives from the
  branch name: rename before you stamp, never after.
- Push and open a **draft** PR. Body: what changed, why, test evidence,
  **Driven by hand**, **Decisions made unattended** (with rejected options),
  dependency justifications, things found outside this task, and the Veld
  session link (`veld agent link`). Public text hygiene per AGENTS.md.
- End the body with this run's settings, read back from `.firelight-ship.json`:

  ```md
  ---
  🚢 Carried by the ship workflow (`docs/ship.md`).

  | Setting | This run |
  |---|---|
  | Review depth | Standard — <n> spawns, <n> rounds |
  | Hands-on checkpoints | None — driven by hand, summary above |
  | Merge policy | Merge on green CI |
  | Docs & tests | Per AGENTS.md checklist |
  ```

- **CI runs nothing on a draft**, and the skipped jobs show as green.

## Step 6 — Ready, CI, merge

1. Only once the review loop has exited and the pre-pass is green:
   `gh pr ready`. **After a fresh push to the draft, first wait until its
   `synchronize` run exists** (`gh run list --branch <b>` shows the head SHA);
   otherwise the late draft run cancels the ready run (recovery:
   `gh run rerun <the cancelled ready run>`).
2. Wait for the **ready run's jobs**, never the rollup:

   ```sh
   RID=$(gh run list --workflow CI --commit "$(git rev-parse HEAD)" \
           --json databaseId,event --jq '[.[] | select(.event == "pull_request")][0].databaseId')
   gh run watch "$RID" --exit-status              # in the background
   gh run view "$RID" --json jobs --jq '[.jobs[] | select(.conclusion != "success" and .conclusion != "skipped") | {name,conclusion}]'   # must be []
   ```

   Then confirm `all-green` itself is `success`.
3. **A failing job:** read its log and fix the cause. One
   `gh run rerun --failed` for a clear infrastructure failure; a second failure
   is a bug. Stamp the fix commit.
4. Behind `main`: `git rebase origin/main && git push --force-with-lease`.
5. `gh pr merge --squash --delete-branch`. Confirm it merged. A `feat`/`fix`
   merge releases: watch the `main` run's release jobs and fix what breaks (a
   `fix(release):` PR).
6. Update the pinned **🔥 Firelight build** issue body (State, Milestones, Open
   items, Decisions) and its offline cache `notes/progress.md`; add a one-line
   merge comment to the issue.

## When to involve the human

Stay autonomous. Stop only for:

- **Blocked:** missing access, or an irreversible act with no safe default.
  Record it in the build issue's **Blocked** section and a `needs-maintainer`
  issue with the exact owner action, and continue with independent work.
- **Weakening §1.1 or §17.1.** Anything else in docs/HANDOVER.md §26 is yours
  to verify, decide, record and fold into the spec.

An auto-mode denial **about content** (a path in a PR body): remove that detail
once and retry. **About an action:** it is a block; never retry it unchanged,
rephrase it or route around it.

---

*Provenance: ported from maintenance-manager's ship skill at commit
`bcf0985284b270fc789d5946c25c6ba76535973d`, itself ported from outfit
(github.com/prosperity-solutions, MIT; see `NOTICE`), adapted for this repo.*
