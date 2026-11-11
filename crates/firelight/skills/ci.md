# Firelight in CI

Keep `.firelight/` true in the same PR as the change. The Firelight GitHub
Action runs three checks:

1. `firelight check`: the format, refs and islands (no network, no model).
2. `firelight preview --check`: builds everything the server would, runs the
   repo's golden questions, and fails when one doesn't find its entry.
3. `firelight covers` (pull requests only): entries whose `covers` paths
   changed while the entry didn't. **Strict by default:** the PR fails until
   each flagged entry is updated or acknowledged (below).

```yaml
name: firelight
on:
  pull_request:
    # labeled/unlabeled: re-run when someone adds the firelight-covers-checked label
    types: [opened, synchronize, reopened, labeled, unlabeled]
permissions:
  contents: read
jobs:
  docs:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@<sha> # vX
        with:
          fetch-depth: 0          # covers needs the base commit
          persist-credentials: false
      - uses: prosperity-solutions/firelight/action@<sha> # vX.Y.Z
        with:
          version: vX.Y.Z
          # embedder: candle      # run golden questions with meaning search
          # strict-covers: "false"   # only warn about maybe-stale entries
```

## When `covers` fires

`covers` is how a repo's docs stay true: a change to code an entry
describes can't merge until someone has looked at the entry. For each
flagged entry the check prints its ref, its file, the changed paths that
triggered it, and the next step (`--json` has the same fields):

```text
error: app-repo/deploy-an-app may be stale
  entry:   .firelight/guides/deploy-an-app.md
  changed: deploy/values.yaml
  next:    update .firelight/guides/deploy-an-app.md so it matches the change; ...
```

Then do one of two things:

- **The entry is now wrong or incomplete:** update it in this PR.
- **It is still correct** (a version bump, a refactor): say so in a commit
  message in this PR, as a trailer (a line in the last paragraph):

  ```text
  Bump the chart

  Firelight-Covers: checked app-repo/deploy-an-app
  ```

  Name several refs separated by spaces, or bare entry ids of this repo.
  `Firelight-Covers: checked` alone acknowledges every flagged entry. A
  maintainer can instead add the PR label `firelight-covers-checked`, which
  acknowledges every flagged entry. The job summary lists what was
  acknowledged and by which commit (or the label). A ref that names no
  entry of this repo is ignored with a warning.

Two warnings never fail the job: a changed file that no entry covers, inside
a folder that other entries do cover (a gap), and an ignored trailer.
`firelight covers --report` shows which top-level paths no entry covers yet.

Pin the action by commit (Dependabot updates the commit) and set `version`
to a release; bump `version` yourself when you upgrade.
The action checks the downloaded binary against the release's `SHA256SUMS`
and its GitHub attestation before running it. Every step writes its output
into the job summary.

## Golden questions that expect another repo's entries

A golden question may expect an entry in another repo
(`expect: [infra-repo/request-access]`). In this repo's CI only this repo is
loaded, so such expects are **skipped, not failed**, and listed as
`repo infra-repo not loaded`. To run them, check the other repos out next to
this one and list their folders in `also` (one per line):

```yaml
      - uses: actions/checkout@<sha> # vX
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: actions/checkout@<sha> # vX
        with:
          repository: acme/infra-repo
          path: .siblings/infra-repo
          persist-credentials: false
          # token: ${{ secrets.INFRA_REPO_READ_TOKEN }}   # for a private repo: read-only, that repo only
      - uses: prosperity-solutions/firelight/action@<sha> # vX.Y.Z
        with:
          version: vX.Y.Z
          also: |
            .siblings/infra-repo
```

`also` feeds `firelight preview --check` only; `firelight check` and `covers`
still look at this repo alone. An error in a sibling's `.firelight/` fails the
check too, so pin the sibling with `ref:` if its main branch moves under you.

Locally, the same three commands:

```sh
firelight check .
firelight preview --check .            # --embedder off for speed
firelight preview --check . ../infra-repo   # with another repo, for cross-repo golden questions
firelight covers --base origin/main .   # add --strict to fail like CI
firelight covers --report .             # paths no entry covers yet
```
