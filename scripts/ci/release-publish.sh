#!/bin/sh
# Create, fill and publish the GitHub release vX.Y.Z (CI's release job;
# maintenance-manager §16.2). The release is created as a draft, every asset is
# uploaded and checked by name and size, and only then published, which
# creates the tag and, with immutable releases on, locks tag and assets.
#   GH_TOKEN, GITHUB_REPOSITORY, GITHUB_SHA, VERSION, IMAGE_DIGEST, CHART_DIGEST
set -eu

: "${VERSION:?}" "${IMAGE_DIGEST:?}" "${CHART_DIGEST:?}" "${GITHUB_SHA:?}" "${GITHUB_REPOSITORY:?}"
tag=v$VERSION
for d in "$IMAGE_DIGEST" "$CHART_DIGEST"; do
  printf '%s' "$d" | grep -Eq '^sha256:[0-9a-f]{64}$' || { echo "Unexpected digest: $d" >&2; exit 1; }
done

# gh api prints the error body on stdout for a 404, so the exit code decides,
# and anything but a full SHA counts as "no tag".
at=$(gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$tag" --jq .object.sha 2>/dev/null) || at=""
printf '%s' "$at" | grep -Eq '^[0-9a-f]{40}$' || at=""
if [ -n "$at" ] && [ "$at" != "$GITHUB_SHA" ]; then
  echo "Tag $tag already points at $at, not $GITHUB_SHA. Releases roll forward: merge a new feat/fix PR." >&2
  exit 1
fi

notes=${RUNNER_TEMP:-/tmp}/notes.md
last=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
git log --format='- %s' "${last:+$last..}HEAD" > "$notes"
repo=$GITHUB_REPOSITORY
{
  printf '\n## Install\n\n```sh\n'
  printf 'docker pull ghcr.io/%s@%s\n' "$repo" "$IMAGE_DIGEST"
  printf 'helm install firelight oci://ghcr.io/%s/charts/firelight --version %s --set publicUrl=https://firelight.acme.example\n```\n\n' "${repo%%/*}" "$VERSION"
  printf '## Verify\n\n```sh\n'
  printf 'gh attestation verify firelight-%s-linux-amd64.tar.gz \\\n' "$tag"
  printf '  --repo %s --signer-workflow %s/.github/workflows/ci.yml \\\n' "$repo" "$repo"
  printf '  --source-ref refs/heads/main --deny-self-hosted-runners\n'
  for ref in "ghcr.io/$repo@$IMAGE_DIGEST" "ghcr.io/${repo%%/*}/charts/firelight@$CHART_DIGEST"; do
    printf 'gh attestation verify oci://%s \\\n' "$ref"
    printf '  --repo %s --signer-workflow %s/.github/workflows/ci.yml \\\n' "$repo" "$repo"
    printf '  --source-ref refs/heads/main --deny-self-hosted-runners\n'
  done
  # shellcheck disable=SC2016 # literal backticks: markdown code spans
  printf '```\n\nImage `%s`, chart `%s`.\n' "$IMAGE_DIGEST" "$CHART_DIGEST"
} >> "$notes"

assets="dist/SHA256SUMS"
for t in linux-amd64 linux-arm64 macos-amd64 macos-arm64; do
  assets="$assets dist/firelight-$tag-$t.tar.gz"
done

draft=$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null || echo none)
case $draft in
  none) gh release create "$tag" --draft --target "$GITHUB_SHA" --title "$tag" --notes-file "$notes" ;;
  true) gh release edit "$tag" --target "$GITHUB_SHA" --notes-file "$notes" ;;
  *) echo "$tag is already published." >&2; exit 1 ;;
esac
# shellcheck disable=SC2086 # a fixed list of paths without spaces
gh release upload "$tag" --clobber $assets
for f in $assets; do
  want=$(wc -c < "$f" | tr -d ' ')
  got=$(gh release view "$tag" --json assets --jq ".assets[] | select(.name == \"$(basename "$f")\") | .size")
  if [ "$got" != "$want" ]; then
    echo "Asset $(basename "$f"): uploaded size '$got', local size $want." >&2
    exit 1
  fi
done
gh release edit "$tag" --draft=false --latest
echo "published $tag"
