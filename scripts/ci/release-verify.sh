#!/bin/sh
# Verify a published release the way a user would (CI's verify job): the image
# and chart pull anonymously, every asset and both OCI subjects carry a
# provenance attestation from main's ci.yml, the image has both platforms, the
# checksums match, and the chart renders with the image pinned by digest.
#   GH_TOKEN, VERSION, IMAGE_DIGEST, CHART_DIGEST
set -eu

: "${VERSION:?}" "${IMAGE_DIGEST:?}" "${CHART_DIGEST:?}"
REPO=prosperity-solutions/firelight
OWNER=${REPO%%/*}
tag=v$VERSION
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
verify() { # verify <subject>
  gh attestation verify "$1" --repo "$REPO" \
    --signer-workflow "$REPO/.github/workflows/ci.yml" \
    --source-ref refs/heads/main --deny-self-hosted-runners
}

echo "▶ The image pulls anonymously and has linux/amd64 and linux/arm64"
token=$(curl -fsS "https://ghcr.io/token?scope=repository:$REPO:pull" | jq -r .token)
curl -fsS -H "Authorization: Bearer $token" \
  -H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json' \
  "https://ghcr.io/v2/$REPO/manifests/$IMAGE_DIGEST" > "$work/index.json"
platforms=$(jq -r '[.manifests[].platform | select(.os == "linux") | .architecture] | sort | join(",")' "$work/index.json")
echo "platforms: $platforms"
[ "$platforms" = "amd64,arm64" ] || { echo "Expected linux/amd64 and linux/arm64." >&2; exit 1; }

echo "▶ The chart pulls anonymously, by version, with the published digest"
out=$(scripts/dev/t helm pull "oci://ghcr.io/$OWNER/charts/firelight" --version "$VERSION" -d "$work" 2>&1)
printf '%s\n' "$out"
printf '%s\n' "$out" | grep -qx "Digest: $CHART_DIGEST" || { echo "Pulled chart digest does not match $CHART_DIGEST." >&2; exit 1; }
scripts/dev/t helm template fl "$work/firelight-$VERSION.tgz" --set publicUrl=https://firelight.acme.example > "$work/render.yaml"
grep -q "image: \"ghcr.io/$REPO@$IMAGE_DIGEST\"" "$work/render.yaml" ||
  { echo "The released chart does not pin the image by digest." >&2; exit 1; }

echo "▶ Attestations: image and chart"
verify "oci://ghcr.io/$REPO@$IMAGE_DIGEST"
verify "oci://ghcr.io/$OWNER/charts/firelight@$CHART_DIGEST"

echo "▶ Release assets: checksums and attestations"
gh release download "$tag" --repo "$REPO" -D "$work/assets"
(cd "$work/assets" && sha256sum -c SHA256SUMS)
for f in "$work"/assets/*.tar.gz "$work/assets/SHA256SUMS"; do
  verify "$f"
done
echo "✅ $tag verifies."
