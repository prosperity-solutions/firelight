# Installing the Firelight CLI

```sh
curl -fsSL https://firelight.oss.life.li/install.sh | sh
curl -fsSL https://firelight.oss.life.li/install.sh | sh -s -- --version v1.2.3
```

The script installs `firelight` for Linux or macOS (amd64 or arm64) into
`~/.local/bin`. It never uses `sudo` and never edits your shell profile; it
prints a hint when the folder isn't on your `PATH`.

## How it checks what it installs

- The SHA-256 of every published archive is baked into `install.sh` when the
  website is built, from each release's `SHA256SUMS` after that file's GitHub
  attestation was verified. A download that doesn't match is deleted, both
  hashes are printed, and nothing is installed.
- When `gh` is installed and logged in, the script also runs
  `gh attestation verify` on the archive (signed by this repo's `ci.yml` on
  `main`, on GitHub-hosted runners only).
- Releases are immutable, so a published version can't be swapped later.

## Settings

| Variable | Meaning |
|---|---|
| `FIRELIGHT_INSTALL_DIR` | Where the binary goes (default `~/.local/bin`). |
| `FIRELIGHT_INSTALL_REQUIRE_ATTESTATION` | `1`: fail unless the attestation check ran and passed (needs `gh auth login`). |
| `FIRELIGHT_INSTALL_BASE_URL` | Download base instead of the GitHub releases (for tests and mirrors). |

Needs `curl` or `wget`, `tar`, and `sha256sum` or `shasum`.

## By hand

Download `firelight-<version>-<os>-<arch>.tar.gz` and `SHA256SUMS` from the
[releases](https://github.com/prosperity-solutions/firelight/releases), then:

```sh
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify firelight-<version>-<os>-<arch>.tar.gz \
  --repo prosperity-solutions/firelight \
  --signer-workflow prosperity-solutions/firelight/.github/workflows/ci.yml \
  --source-ref refs/heads/main --deny-self-hosted-runners
tar -xzf firelight-<version>-<os>-<arch>.tar.gz firelight-<version>-<os>-<arch>/firelight
```
