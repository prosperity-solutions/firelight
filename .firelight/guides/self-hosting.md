---
format: 1
title: Run Firelight yourself
summary: "The full self-hosting guide: Docker, Compose with TLS, Helm, plain Kubernetes, connecting repos, webhooks, auth proxies, models, operations and troubleshooting."
aliases: [self-hosting, self host, deploy, docker compose, helm, kubernetes, troubleshooting]
tags: [operations]
related: [run-firelight]
---
<!-- Generated from docs/self-hosting.md; edit that file. -->

Firelight is one process with one data folder. Git is the truth: the data
folder is a cache you can delete at any time, so there is nothing to back up.

Every block marked `ci-run` below is run by CI on every pull request (the
`docs` job, with the image and chart just built), so these steps can't rot.

## 1. Quick try

Serve one repo's `.firelight/` on your machine, with no login (only you can
reach `127.0.0.1`) and word search only (no model download):

```sh ci-run
docker run -d --name firelight -p 127.0.0.1:8080:8080 \
  -v "$PWD/fixtures/acme-infra:/repos/acme-infra:ro" \
  -e FIRELIGHT_SOURCES='[{"id":"acme-infra","kind":"dir","path":"/repos/acme-infra"}]' \
  -e FIRELIGHT_SERVER_PUBLIC_URL=http://127.0.0.1:8080 \
  -e FIRELIGHT_AUTH_UI=none -e FIRELIGHT_AUTH_MCP=none \
  -e FIRELIGHT_EMBEDDER=off \
  ghcr.io/prosperity-solutions/firelight:latest
curl -fsS --retry 60 --retry-all-errors --retry-delay 1 http://127.0.0.1:8080/readyz
curl -fsS http://127.0.0.1:8080/api/v1/repos | grep -q acme-infra
docker rm -f firelight
```

Point the mount and `path` at your own repo, open <http://127.0.0.1:8080>,
and drop `FIRELIGHT_EMBEDDER=off` to get meaning search (the model, 1.2 GB,
downloads once into the data folder).

## 2. Docker Compose with TLS and a login

Firelight serves plain HTTP; HTTPS is the reverse proxy's job. Caddy gets a
certificate on its own. Put the password in a file next to the compose file
(`printf '%s' 'a long password' > firelight-password`).

```yaml
# compose.yaml
services:
  firelight:
    image: ghcr.io/prosperity-solutions/firelight:latest
    restart: unless-stopped
    environment:
      FIRELIGHT_SERVER_PUBLIC_URL: https://firelight.acme.example
      FIRELIGHT_SOURCES: '[{"id":"infra-repo","kind":"git","url":"https://git.acme.example/acme/infra-repo.git"}]'
      FIRELIGHT_AUTH_UI: basic
      FIRELIGHT_AUTH_USER: acme
      FIRELIGHT_AUTH_PASSWORD_FILE: /run/secrets/firelight-password
      FIRELIGHT_AUTH_MCP: token
      FIRELIGHT_AUTH_MCP_TOKENS_FILE: /run/secrets/firelight-tokens
    secrets: [firelight-password, firelight-tokens]
    volumes: [firelight-data:/var/lib/firelight]
  caddy:
    image: caddy:2
    restart: unless-stopped
    ports: ["80:80", "443:443"]
    command: caddy reverse-proxy --from firelight.acme.example --to firelight:8080
    volumes: [caddy-data:/data]
secrets:
  firelight-password: { file: ./firelight-password }
  firelight-tokens: { file: ./firelight-tokens }
volumes:
  firelight-data: {}
  caddy-data: {}
```

`caddy reverse-proxy` passes the client's `Host` header, which Firelight
checks (section 7). Check the file before starting it:

```sh ci-run
cd docs/examples        # where compose.yaml and the two secret files live
trap 'rm -f firelight-password firelight-tokens' EXIT
touch firelight-password firelight-tokens
docker compose -f compose.yaml config --quiet
```

## 3. Kubernetes with Helm

```sh ci-run
helm template firelight oci://ghcr.io/prosperity-solutions/charts/firelight \
  --set publicUrl=https://firelight.acme.example \
  --set config.auth.ui=none --set config.auth.mcp=token \
  --set existingSecret=firelight \
  --set-string secretFiles.FIRELIGHT_AUTH_MCP_TOKENS_FILE=mcp-tokens \
  > /dev/null
```

Install it the same way with `helm install`. The minimum `values.yaml`
(also in [docs/examples/values.yaml](https://github.com/prosperity-solutions/firelight/blob/main/docs/examples/values.yaml)):

```yaml
# values.yaml
publicUrl: https://firelight.acme.example
config:
  auth: { ui: none, mcp: token }   # behind your VPN or auth proxy
  sources:
    - { id: infra-repo, kind: github, url: https://github.com/acme/infra-repo }
existingSecret: firelight          # kubectl create secret generic firelight --from-file=...
secretFiles:
  FIRELIGHT_AUTH_MCP_TOKENS_FILE: mcp-tokens
  FIRELIGHT_GITHUB_APP_PRIVATE_KEY_FILE: github-app.pem
env:
  FIRELIGHT_GITHUB_APP_ID: "123456"
```

```sh ci-run
helm template firelight oci://ghcr.io/prosperity-solutions/charts/firelight \
  -f docs/examples/values.yaml > /dev/null
```

- **Resources:** the default model needs about 3 GiB, up to 3.5 GiB while it
  loads (3.9 GiB seen on a laptop); the chart asks for 3 GiB and limits at
  5 GiB. One CPU answers a query in about 0.3 s. With
  `FIRELIGHT_EMBEDDER=off`, 256 MiB is plenty.
- **Storage:** a 20 GiB PVC at `/var/lib/firelight` holds the model, the git
  mirrors and the cache. An `emptyDir` works too; a restart then downloads
  the model again.
- **One replica, `Recreate`:** the data folder is a SQLite database and git
  mirrors on one volume; two pods would fight over it. Firelight is a cache in
  front of git, so a short restart loses nothing.

## 4. Kubernetes without Helm

Render the chart once and keep the manifests (Deployment, Service, PVC,
ConfigMap, ServiceAccount) in your own repo:

```sh ci-run
helm template firelight oci://ghcr.io/prosperity-solutions/charts/firelight \
  --namespace firelight \
  --set publicUrl=https://firelight.acme.example \
  --set config.auth.ui=none --set config.auth.mcp=none \
  > firelight.yaml
grep -q 'kind: Deployment' firelight.yaml
```

Create the Secret yourself (`kubectl create secret generic firelight
--from-file=…`) and refer to it as in section 3.

## 5. Connecting repos

- **GitHub, with a GitHub App (recommended):** create an App with the
  repository permissions *Contents: read* and *Metadata: read*, and install
  it on the org or on selected repos. Leave its own webhook off unless you
  want push notifications (section 6). Set
  `FIRELIGHT_GITHUB_APP_ID` and mount its private key as
  `FIRELIGHT_GITHUB_APP_PRIVATE_KEY_FILE`. Sources are
  `{"id":"infra-repo","kind":"github","url":"https://github.com/acme/infra-repo"}`.
- **A token instead:** `{"kind":"git","url":"https://git.acme.example/acme/infra-repo.git","token_file":"/etc/firelight/secrets/git-token"}`.
  Firelight sends it only over HTTPS (or to loopback).
- **Public repos and any other git host over HTTPS:** `kind: git` with no
  credentials.

Firelight reads only `.firelight/` from each repo, from a blobless mirror,
and never follows symlinks. Check a config without starting anything:

```sh ci-run
mkdir -p /tmp/firelight-secrets && printf 'a-token-of-at-least-16' > /tmp/firelight-secrets/git-token
FIRELIGHT_SERVER_PUBLIC_URL=https://firelight.acme.example \
FIRELIGHT_AUTH_UI=none FIRELIGHT_AUTH_MCP=none \
FIRELIGHT_SOURCES='[{"id":"infra-repo","kind":"github","url":"https://github.com/acme/infra-repo"},
  {"id":"app-repo","kind":"git","url":"https://git.acme.example/acme/app-repo.git","token_file":"/tmp/firelight-secrets/git-token"},
  {"id":"docs-repo","kind":"git","url":"https://git.acme.example/acme/docs-repo.git"}]' \
FIRELIGHT_GITHUB_APP_ID=123456 FIRELIGHT_GITHUB_APP_PRIVATE_KEY_FILE=/tmp/firelight-secrets/git-token \
firelight config check
```

## 6. Webhooks (optional)

Firelight polls every source every two minutes. A webhook makes a change show
up in seconds: in the repo's (or the App's) settings, add
`https://firelight.acme.example/webhook/github`, content type
`application/json`, a secret of at least 16 characters (also set as
`FIRELIGHT_WEBHOOK_SECRET_FILE`), and the `push` event; or turn on the GitHub
App's webhook with the same URL and secret. If GitHub can't reach your
server, skip this: polling is enough.

## 7. Behind real auth

Firelight has basic auth for the page and bearer tokens for `/mcp`. For SSO,
put an auth proxy in front:

- **oauth2-proxy (or any SSO proxy) in front:** set `FIRELIGHT_AUTH_UI=none`
  and keep `FIRELIGHT_AUTH_MCP=token`. Agents can't do SSO, so the proxy must
  let `/mcp` through to Firelight's token check (oauth2-proxy:
  `--skip-auth-route=^/mcp$`); also let `/healthz` and `/readyz` through for
  probes. `FIRELIGHT_SERVER_PUBLIC_URL` is the proxy's URL, and the proxy must
  pass the client's `Host`.
- **VPN only:** `FIRELIGHT_AUTH_UI=none` and `FIRELIGHT_AUTH_MCP=none` are
  allowed, but you must write them explicitly. `FIRELIGHT_SERVER_PUBLIC_URL`
  is the address people use on the VPN; other names they use (a short host
  name, an IP) go into `FIRELIGHT_SERVER_ALLOWED_HOSTS`.
- **The Host check:** every route except `/healthz`, `/readyz` and the
  webhook answers only to the host in `FIRELIGHT_SERVER_PUBLIC_URL`, plus
  `FIRELIGHT_SERVER_ALLOWED_HOSTS`, a JSON list (for example
  `'["firelight.firelight.svc"]'`, the Service name for in-cluster clients). Your proxy must pass the
  client's `Host` header (nginx: `proxy_set_header Host $host;`), or you get
  421.
- **The Origin check:** browsers may call `/mcp` only from
  `FIRELIGHT_SERVER_ALLOWED_ORIGINS`, a JSON list (default: the public URL's
  origin).
  Command-line clients send no `Origin` and pass.

## 8. Embedding models

- **The default** (`Qwen/Qwen3-Embedding-0.6B`, in-process on the CPU):
  nothing to set. It downloads once and is checked against checksums built
  into the binary.
- **Another model:** `FIRELIGHT_EMBED_MODEL` (a Hugging Face id) and
  `FIRELIGHT_EMBED_REVISION` (its 40-hex commit; branch names are refused).
  Supported architectures: Qwen3, BERT (e.g. `intfloat/multilingual-e5-small`,
  smaller and faster on small machines), XLM-RoBERTa, ModernBERT.
- **A mirror:** `FIRELIGHT_EMBED_HF_ENDPOINT`; a non-default model from a
  mirror also needs `FIRELIGHT_EMBED_SHA256` for every file.
- **Air-gapped:** copy the model folder in and set `FIRELIGHT_EMBED_PATH`.
- **A server instead:** `FIRELIGHT_EMBEDDER=openai-compatible` with
  `FIRELIGHT_EMBED_BASE_URL` (e.g. Ollama's `http://ollama:11434/v1`) and
  `FIRELIGHT_EMBED_MODEL`. Chunk text then leaves the process.
- **Off:** `FIRELIGHT_EMBEDDER=off` keeps word search only, fully supported.

## 9. Operations

- **Health:** `/healthz` answers as soon as the process runs; `/readyz` turns
  200 once the catalog is loaded and the built-in golden check passed (a
  first start may take minutes while the model downloads).
- **Status:** `/status` (and `/api/v1/status`, MCP `status`) shows each
  source's commit, last pull, why a newest commit isn't served, warnings,
  golden results, the embedder and the effective config with secrets as
  `••••`.
- **OpenTelemetry:** set `OTEL_EXPORTER_OTLP_ENDPOINT` to export traces and
  metrics; search spans record the query only with
  `FIRELIGHT_OTEL_RECORD_QUERIES=true`.
- **Upgrades and rollbacks:** change the image tag. A database newer than the
  binary (after a rollback) is deleted and rebuilt from git.
- **Backups:** none needed; git is the truth.

## 10. Troubleshooting

Firelight checks its whole config at start and lists every problem, then
exits with code 2. The most common:

| Message (as printed) | Fix |
|---|---|
| `Pick an auth mode` | Set `FIRELIGHT_AUTH_UI` and `FIRELIGHT_AUTH_MCP` (`none` must be written explicitly). |
| `none is refused with auth.ui = basic: MCP would expose everything the login protects` | Use `FIRELIGHT_AUTH_MCP=token` with tokens. |
| `required for serve: the URL people open` | Set `FIRELIGHT_SERVER_PUBLIC_URL`. |
| `required with auth.ui = basic. Set FIRELIGHT_AUTH_PASSWORD_FILE` | Mount the password and point the variable at it. |
| `required with auth.mcp = token: at least one token` | Set `FIRELIGHT_AUTH_MCP_TOKENS_FILE` (one token per line, at least 16 characters each). |
| `a token in the list is empty` | An unset variable in a token list; remove the empty entry. |
| `needs at least 16 characters, or anyone could sign deliveries` | A longer webhook secret: `openssl rand -hex 32`. |
| `unknown key` | A typo in the TOML file or a `FIRELIGHT_*` variable Firelight doesn't know; see docs/config.md. |
| `is not a 40-hex commit` | Use the model's commit, not a branch name or tag. |
| `required for a model other than the default` | Set `FIRELIGHT_EMBED_REVISION` for a non-default model. |
| `needs embed.sha256` | A non-default model from a mirror: give every file's checksum. |
| `cannot create the data folder` | Point `FIRELIGHT_DATA_DIR` at a writable folder (the chart mounts one). |
| `the embedder can't start` | The model couldn't be downloaded or verified: check egress to huggingface.co, or use `FIRELIGHT_EMBED_PATH` or `FIRELIGHT_EMBEDDER=off`. |
| 421 in the browser | The `Host` doesn't match the public URL: fix the proxy, or add `FIRELIGHT_SERVER_ALLOWED_HOSTS`. |

```sh ci-run
out=$(FIRELIGHT_SERVER_PUBLIC_URL=https://firelight.acme.example firelight config check 2>&1 || true)
echo "$out" | grep -q 'Pick an auth mode'
```

`firelight config show` prints the effective config with secrets as `••••`.
