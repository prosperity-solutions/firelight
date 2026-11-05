#!/bin/sh
# Generate install.sh from site/install.sh.in and every release's SHA256SUMS
# (docs/HANDOVER.md §11.1).
#
#   scripts/site/gen-install.sh <sums-dir> <out>
#
# <sums-dir> holds one folder per release, named by its tag, each with that
# release's SHA256SUMS (already attestation-verified by the caller: the site
# job). Only firelight-<tag>-<os>-<arch>.tar.gz lines are used; anything else
# in a SHA256SUMS, or a malformed line, fails the generation.
set -eu

sums=${1:?usage: gen-install.sh <sums-dir> <out>}
out=${2:?usage: gen-install.sh <sums-dir> <out>}
here=$(cd "$(dirname "$0")/../.." && pwd)
template="$here/site/install.sh.in"

table=$(mktemp)
trap 'rm -f "$table"' EXIT

# Tags in version order (v1.2.10 after v1.2.9), oldest first.
tags=$(for d in "$sums"/v*; do [ -d "$d" ] && basename "$d"; done | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sed 's/^v//' | sort -t. -k1,1n -k2,2n -k3,3n | sed 's/^/v/')
[ -n "$tags" ] || { echo "gen-install: no releases in $sums" >&2; exit 1; }
for tag in $tags; do
  f="$sums/$tag/SHA256SUMS"
  [ -f "$f" ] || { echo "gen-install: $f is missing" >&2; exit 1; }
  rows=0
  while read -r hash name rest || [ -n "${hash:-}" ]; do
    [ -n "$hash" ] || continue
    name=${name#\*}
    if [ -n "${rest:-}" ] || ! printf '%s' "$hash" | grep -Eq '^[0-9a-f]{64}$'; then
      echo "gen-install: malformed line in $f: $hash $name" >&2; exit 1
    fi
    case "$name" in
      "firelight-$tag-linux-amd64.tar.gz"|"firelight-$tag-linux-arm64.tar.gz"|"firelight-$tag-macos-amd64.tar.gz"|"firelight-$tag-macos-arm64.tar.gz")
        printf '%s %s %s\n' "$tag" "$name" "$hash" >> "$table"; rows=$((rows + 1)) ;;
      *) ;; # other assets (SBOMs, charts) aren't installed by this script
    esac
  done < "$f"
  [ "$rows" = 4 ] || { echo "gen-install: $f lists $rows of the 4 archives" >&2; exit 1; }
done

awk -v tf="$table" '
  $0 == "@@TABLE@@" { while ((getline l < tf) > 0) print l; next }
  { print }
' "$template" > "$out"
chmod 0755 "$out"
echo "gen-install: $(wc -l < "$table" | tr -d ' ') archives from $(echo "$tags" | wc -l | tr -d ' ') releases → $out"
