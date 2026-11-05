#!/bin/sh
# Build the website into <out> (docs/HANDOVER.md §11.1, §15.1): copy site/,
# fetch every published release's SHA256SUMS, verify each one's GitHub
# attestation, and bake install.sh from them. Needs gh with GH_TOKEN.
#
#   sh scripts/site/build-site.sh <out>
set -eu
out=${1:?usage: build-site.sh <out>}
root=$(cd "$(dirname "$0")/../.." && pwd)
repo=prosperity-solutions/firelight
sums=$(mktemp -d)
trap 'rm -rf "$sums"' EXIT

rm -rf "$out"
mkdir -p "$out"
cp -R "$root/site/." "$out/"
rm -f "$out/install.sh.in"

tags=$(gh release list --repo "$repo" --limit 1000 --exclude-drafts --exclude-pre-releases --json tagName --jq '.[].tagName')
for tag in $tags; do
  case "$tag" in v[0-9]*.[0-9]*.[0-9]*) ;; *) continue ;; esac
  mkdir -p "$sums/$tag"
  gh release download "$tag" --repo "$repo" --pattern SHA256SUMS --dir "$sums/$tag"
  gh attestation verify "$sums/$tag/SHA256SUMS" --repo "$repo" \
    --signer-workflow "$repo/.github/workflows/ci.yml" \
    --source-ref refs/heads/main --deny-self-hosted-runners >/dev/null
  echo "site: $tag SHA256SUMS attested"
done
sh "$root/scripts/site/gen-install.sh" "$sums" "$out/install.sh"
echo "site: built $out"
