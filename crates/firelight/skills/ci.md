# Firelight in CI

Keep `.firelight/` true in the same PR as the change. The Firelight GitHub
Action runs three checks:

1. `firelight check`: the format, refs and islands (no network, no model).
2. `firelight preview --check`: builds everything the server would, runs the
   repo's golden questions, and fails when one doesn't find its entry.
3. `firelight covers` (pull requests only): entries whose `covers` paths
   changed while the entry didn't. A warning unless `strict-covers: true`.

```yaml
name: firelight
on: [pull_request]
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
          # strict-covers: "true"
```

Pin the action by commit (Dependabot updates the commit) and set `version`
to a release; bump `version` yourself when you upgrade.
The action checks the downloaded binary against the release's `SHA256SUMS`
and its GitHub attestation before running it. Every step writes its output
into the job summary.

Locally, the same three commands:

```sh
firelight check .
firelight preview --check .            # --embedder off for speed
firelight covers --base origin/main .
```
