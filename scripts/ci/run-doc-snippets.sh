#!/bin/sh
# Run every ```sh ci-run block of docs/self-hosting.md (docs/HANDOVER.md
# §11.2), from the repo root, each in its own `sh -eu`. The published image
# and chart are swapped for the ones CI just built:
#   IMAGE  the image to use instead of ghcr.io/prosperity-solutions/firelight:latest
#   CHART  the chart to use instead of oci://ghcr.io/prosperity-solutions/charts/firelight
# Also checks that docs/examples/{compose,values}.yaml are the files the guide shows.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
doc="$root/docs/self-hosting.md"
image=${IMAGE:?set IMAGE}
chart=${CHART:?set CHART}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for f in compose.yaml values.yaml; do
  awk -v name="# $f" '/^```yaml$/{getline l; if (l==name){on=1; print l; next}} on&&/^```$/{on=0} on{print}' "$doc" > "$work/$f"
  if ! diff -u "$root/docs/examples/$f" "$work/$f"; then
    echo "docs/examples/$f differs from the $f block in docs/self-hosting.md" >&2
    exit 1
  fi
done

awk -v dir="$work" '
  /^```sh ci-run$/ { n++; f = sprintf("%s/snippet-%02d.sh", dir, n); on = 1; next }
  on && /^```$/ { on = 0; close(f); next }
  on { print > f }
' "$doc"
count=$(find "$work" -name "snippet-*.sh" | wc -l | tr -d ' ')
[ "$count" -gt 0 ] || { echo "no ci-run snippets found" >&2; exit 1; }
cd "$root"
for s in "$work"/snippet-*.sh; do
  sed -e "s#ghcr.io/prosperity-solutions/firelight:latest#$image#g" \
      -e "s#oci://ghcr.io/prosperity-solutions/charts/firelight#$chart#g" "$s" > "$s.run"
  echo "::group::$(basename "$s")"
  cat "$s.run"
  echo "::endgroup::"
  sh -eu "$s.run" || { echo "FAILED: $(basename "$s")" >&2; exit 1; }
done
rm -f firelight.yaml

# The website's own one-liners (site/index.html pre#docker, pre#helm), run as
# shown: Docker detached for the check, Helm rendered instead of installed.
site_cmd() {
  python3 -c 'import html,re,sys; t=open(sys.argv[1]).read(); m=re.search(r"<pre id=\""+sys.argv[2]+r"\"><code>(.*?)</code></pre>", t, re.S); print(html.unescape(m.group(1)) if m else "")' "$root/site/index.html" "$1"
}
docker_cmd=$(site_cmd docker)
helm_cmd=$(site_cmd helm)
[ -n "$docker_cmd" ] && [ -n "$helm_cmd" ] || { echo "site/index.html lost pre#docker or pre#helm" >&2; exit 1; }
printf '%s\n' "$docker_cmd" | sed -e "s#^docker run --rm #docker run -d --rm --name firelight-site #" \
  -e "s#ghcr.io/prosperity-solutions/firelight:latest#$image#g" > "$work/site-docker.sh"
cat >> "$work/site-docker.sh" <<'SH'
trap 'docker rm -f firelight-site >/dev/null 2>&1' EXIT
curl -fsS --retry 60 --retry-all-errors --retry-delay 1 http://127.0.0.1:8080/readyz
SH
printf '%s > /dev/null\n' "$helm_cmd" | sed -e "s#^helm install #helm template #" \
  -e "s#oci://ghcr.io/prosperity-solutions/charts/firelight#$chart#g" > "$work/site-helm.sh"
# The Docker line runs in a repo whose id is my-repo, as the page says.
site_repo="$work/my-repo"
mkdir -p "$site_repo/.firelight"
printf 'format: 1\nid: my-repo\nname: My repo\nsummary: A repo for the website check.\n' > "$site_repo/.firelight/repo.yaml"
for s in "$work/site-docker.sh" "$work/site-helm.sh"; do
  echo "::group::$(basename "$s")"; cat "$s"; echo "::endgroup::"
  case "$s" in *docker*) dir=$site_repo ;; *) dir=$root ;; esac
  (cd "$dir" && sh -eu "$s") || { echo "FAILED: the website's $(basename "$s")" >&2; exit 1; }
done
echo "docs: $count ci-run snippets and the website's commands pass"
