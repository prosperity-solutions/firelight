# Releases

Releases happen in `.github/workflows/ci.yml`, on a push to `main`, after the
`all-green` job. There is no separate release workflow and no manual step.

- **Releasable** means the commits since the last published release include a
  Conventional Commit of type `feat` or `fix`, or a breaking change (`!`).
  Everything else (`docs`, `ci`, `chore`, …) merges without a release.
- **The GitHub release is the source of truth.** `vX.Y.Z` exists only once its
  release is published. Releases are immutable, and `v*` tags can be created
  but never moved or deleted: a bad version is superseded, never replaced.
- **What ships** (from M1): `firelight` binaries for Linux amd64/arm64 and
  macOS arm64/amd64 with a `SHA256SUMS` file, the image
  `ghcr.io/prosperity-solutions/firelight` (amd64 + arm64) and the chart
  `oci://ghcr.io/prosperity-solutions/charts/firelight`. Every asset, the image
  and the chart get GitHub build-provenance attestations.
- **No secrets.** Builds, pushes and attestations use only `GITHUB_TOKEN` and
  GitHub OIDC.

## Verify

```sh
gh attestation verify <file or oci://ref> \
  --repo prosperity-solutions/firelight \
  --signer-workflow prosperity-solutions/firelight/.github/workflows/ci.yml \
  --source-ref refs/heads/main --deny-self-hosted-runners
```

The exact commands, with digests, are in every release's notes.

## Recovery

- **The first release's packages.** The image and chart packages are created
  by the first `publish`. Packages that a workflow in a public repository
  creates are linked to it and normally start public (maintenance-manager
  found this for its own first release). If `verify` fails on an anonymous
  pull, the owner sets each package to public once (GitHub → Packages → the
  package → Package settings → Change visibility; there is no API for it),
  then re-runs `verify` with `gh run rerun --failed <run-id>`.

- `gh run rerun --failed <run-id>` re-runs failed release jobs for the same
  commit.
- A failure before the release is published leaves no tag, so the next
  releasable merge retries the same version.
