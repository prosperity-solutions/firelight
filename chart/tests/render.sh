#!/bin/sh
# Render test for the chart: scripts/dev/t helm lint/template/package. Hermetic.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
chart=$root/chart
helm() { "$root/scripts/dev/t" helm "$@"; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
# has <file> <text>: the fixed text is in the file.
has() { grep -qF -- "$2" "$1" || fail "expected '$2' in $1"; }

url=https://firelight.acme.example

helm lint "$chart" --set publicUrl=$url --set image.tag=v0.0.0-test
helm template fl "$chart" --set publicUrl=$url --set image.tag=v0.0.0-test >"$tmp/out.yaml"

has "$tmp/out.yaml" "replicas: 1"
has "$tmp/out.yaml" "type: Recreate"
has "$tmp/out.yaml" "enableServiceLinks: false"
has "$tmp/out.yaml" "automountServiceAccountToken: false"
has "$tmp/out.yaml" "readOnlyRootFilesystem: true"
has "$tmp/out.yaml" "runAsUser: 65532"
has "$tmp/out.yaml" "checksum/config:"
has "$tmp/out.yaml" "public_url = \"$url\""
has "$tmp/out.yaml" "listen = \"0.0.0.0:8080\""
has "$tmp/out.yaml" "dir = \"/var/lib/firelight\""
has "$tmp/out.yaml" "ghcr.io/prosperity-solutions/firelight:v0.0.0-test"
has "$tmp/out.yaml" "name: FIRELIGHT_CONFIG"
has "$tmp/out.yaml" "failureThreshold: 180"
has "$tmp/out.yaml" "path: /readyz"
if grep -q "/etc/firelight/secrets" "$tmp/out.yaml"; then
  fail "secrets mounted without existingSecret"
fi

# secretFiles wiring.
helm template fl "$chart" --set publicUrl=$url --set image.tag=v0.0.0-test \
  --set existingSecret=acme-creds \
  --set secretFiles.FIRELIGHT_AUTH_PASSWORD_FILE=password >"$tmp/secret.yaml"
has "$tmp/secret.yaml" "name: FIRELIGHT_AUTH_PASSWORD_FILE"
has "$tmp/secret.yaml" "value: \"/etc/firelight/secrets/password\""
has "$tmp/secret.yaml" "secretName: acme-creds"
has "$tmp/secret.yaml" "mountPath: /etc/firelight/secrets"

# Failures.
if helm template fl "$chart" --set image.tag=v0.0.0-test >"$tmp/err1" 2>&1; then
  fail "render without publicUrl succeeded"
fi
has "$tmp/err1" "publicUrl is required: the URL people open, e.g. https://firelight.acme.example"

if helm template fl "$chart" --set publicUrl=$url >"$tmp/err2" 2>&1; then
  fail "render without image tag or digest succeeded"
fi
has "$tmp/err2" "image.tag or image.digest is required for a source checkout"

# Rehearse the release pin: sed the digest, package, render the .tgz.
cp -R "$chart" "$tmp/chart"
digest=sha256:0000000000000000000000000000000000000000000000000000000000000000
sed -i.bak "s|^  digest: \"\"\$|  digest: \"$digest\"|" "$tmp/chart/values.yaml"
rm -f "$tmp/chart/values.yaml.bak"
mkdir "$tmp/pkg"
helm package "$tmp/chart" --version 0.0.1 --app-version 0.0.1 --destination "$tmp/pkg"
helm template fl "$tmp/pkg/firelight-0.0.1.tgz" --set publicUrl=$url >"$tmp/pkg.yaml"
has "$tmp/pkg.yaml" "ghcr.io/prosperity-solutions/firelight@$digest"

echo "chart render tests passed"
