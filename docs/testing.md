# Testing Firelight

Every test is hermetic unless it says otherwise: no GitHub, no Hugging Face,
no network beyond `127.0.0.1`. Local git repos, local web servers and a fake
embedder stand in for the real thing.

## What runs where

| Layer | What it covers | Where |
|---|---|---|
| Unit and crate tests | the format (a fixture per rule, diagnostics as snapshots), refs, islands, templates and their URL rules, markdown escaping, config refusals, the store's migrations, chunking, ranking and fusion with a deterministic fake embedder, golden questions, auth and the Host/Origin checks, every `/api/v1` route | `cargo test --workspace` (the `test` job) |
| Sources | git and dir sources against local bare repos built from `fixtures/histories/` (first pull, no change, broken commit keeps the last good, force-moved ref, symlinks refused, `covers` staleness); GitHub App tokens and webhooks against a local mock | `cargo test -p firelight-source` |
| The binary | `firelight serve` with token auth and an MCP client over real HTTP (tools, resources, prompt, provenance, token and Origin); OpenTelemetry export to a local OTLP receiver; startup readiness and the golden gate | `cargo test -p firelight` |
| Browser | the page without JS, ⌘K, Show me, hide/unhide, template islands, permalinks, axe in light and dark (also with search open), 375 px | `tests/browser` (Playwright), the `e2e` job |
| The model | the default model against its model card's reference vectors, the built-in golden set, query latency (p95 budget) and peak memory | the `model` job; the only test job that reaches Hugging Face |
| The Action | `action/action.yml` on a fixture repo with this commit's binary, including `covers` on a pull request | the `action` job |
| The installer | a fake release on a local web server: good install, tampered archive, tampered `SHA256SUMS`, unknown version, no sha tool, attestation required without `gh`; on Ubuntu, Alpine (busybox) and macOS | `sh tests/installer/test.sh`, the `installer` jobs |
| Docs and site | every `ci-run` snippet of `docs/self-hosting.md` with the image and chart just built; the website's rules (one h1, title, description, CSP, footer, install line, contrast); `llms.txt` drift | the `docs` job, `python3 tests/validate-site.py` |
| Repo rules | workflow pins, egress allow-lists, permissions, the dependency rules between crates, the agent guard, scripts | `cargo test -p harness` |

## Locally

```sh
cargo fmt --all --check && cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace                       # everything hermetic
(cd tests/browser && npm ci --ignore-scripts && npx --no-install playwright install chromium && npx --no-install playwright test)
sh tests/installer/test.sh
python3 tests/validate-site.py
```

The model tests download the default model once (1.2 GB) into `target/`:

```sh
FIRELIGHT_MODEL_TEST=1 cargo test --release -p firelight-embed --test reference -- --ignored
FIRELIGHT_MODEL_TEST=1 cargo test --release -p firelight model_job -- --ignored --nocapture
```

Snapshots use `insta`: `INSTA_UPDATE=always cargo test` rewrites them; review
the diff before committing. `UPDATE_SNAPSHOTS=1` regenerates the generated
files that are checked for drift (`docs/config.md`, `site/llms*.txt`).
