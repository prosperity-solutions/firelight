#!/bin/sh
# Check the four release archives in dist/ against the sha256 their build jobs
# output, then write dist/SHA256SUMS. Refuses any archive whose hash differs
# (an artifact swapped between jobs). Prints digest=sha256:<hex> of SHA256SUMS
# (also to $GITHUB_OUTPUT when set): its content follows from the four hashes,
# so publish and release both arrive at the same digest.
#   VERSION, SHA_LINUX_AMD64, SHA_LINUX_ARM64, SHA_MACOS_ARM64, SHA_MACOS_AMD64
set -eu

: "${VERSION:?}"
sum() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

lines=""
for t in linux-amd64 linux-arm64 macos-amd64 macos-arm64; do
  case $t in
    linux-amd64) want=${SHA_LINUX_AMD64:-} ;;
    linux-arm64) want=${SHA_LINUX_ARM64:-} ;;
    macos-amd64) want=${SHA_MACOS_AMD64:-} ;;
    macos-arm64) want=${SHA_MACOS_ARM64:-} ;;
  esac
  f=firelight-v$VERSION-$t.tar.gz
  printf '%s' "$want" | grep -Eq '^[0-9a-f]{64}$' || { echo "no sha256 output for $t" >&2; exit 1; }
  [ -f "dist/$f" ] || { echo "dist/$f is missing" >&2; exit 1; }
  got=$(sum "dist/$f")
  if [ "$got" != "$want" ]; then
    echo "dist/$f: sha256 $got, but its build job made $want. Refusing it." >&2
    exit 1
  fi
  lines="$lines$want  $f
"
done
printf '%s' "$lines" > dist/SHA256SUMS
cat dist/SHA256SUMS
d=sha256:$(sum dist/SHA256SUMS)
echo "digest=$d"
if [ -n "${GITHUB_OUTPUT:-}" ]; then echo "digest=$d" >> "$GITHUB_OUTPUT"; fi
