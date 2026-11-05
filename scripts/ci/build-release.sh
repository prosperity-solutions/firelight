#!/bin/sh
# Build and pack one release binary (CI's build-<os>-<arch> jobs).
#   TARGET             linux-amd64 | linux-arm64 | macos-arm64 | macos-amd64
#   FIRELIGHT_VERSION  X.Y.Z (baked into the binary)
# Writes dist/firelight-vX.Y.Z-<target>.tar.gz and, to $GITHUB_OUTPUT (or
# stdout), sha256=<hex> of that archive. Native builds only: each job runs on
# its own platform, so nothing is cross-compiled.
set -eu

: "${TARGET:?TARGET is required}"
: "${FIRELIGHT_VERSION:?FIRELIGHT_VERSION is required}"
printf '%s' "$FIRELIGHT_VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' ||
  { echo "FIRELIGHT_VERSION is not X.Y.Z: $FIRELIGHT_VERSION" >&2; exit 1; }

case $TARGET in
  linux-amd64) want_os=Linux want_arch=x86_64 ;;
  linux-arm64) want_os=Linux want_arch=aarch64 ;;
  macos-arm64) want_os=Darwin want_arch=arm64 ;;
  macos-amd64) want_os=Darwin want_arch=x86_64 ;;
  *) echo "unknown TARGET $TARGET" >&2; exit 1 ;;
esac
[ "$(uname -s)" = "$want_os" ] && [ "$(uname -m)" = "$want_arch" ] ||
  { echo "TARGET $TARGET needs a $want_os $want_arch runner, this is $(uname -s) $(uname -m)" >&2; exit 1; }

FIRELIGHT_COMMIT=$(git rev-parse --short=12 HEAD)
export FIRELIGHT_VERSION FIRELIGHT_COMMIT
cargo build --locked --release --bin firelight

# The binary says what it is.
got=$(target/release/firelight version | head -n 1)
[ "$got" = "firelight $FIRELIGHT_VERSION (commit $FIRELIGHT_COMMIT)" ] ||
  { echo "unexpected version line: $got" >&2; exit 1; }

name=firelight-v$FIRELIGHT_VERSION-$TARGET
rm -rf "dist/$name"
mkdir -p "dist/$name"
cp target/release/firelight LICENSE NOTICE README.md "dist/$name/"
(cd dist && tar -czf "$name.tar.gz" "$name")
rm -rf "dist/$name"

if command -v sha256sum >/dev/null 2>&1; then
  sha=$(sha256sum "dist/$name.tar.gz" | cut -d' ' -f1)
else
  sha=$(shasum -a 256 "dist/$name.tar.gz" | cut -d' ' -f1)
fi
echo "$name.tar.gz: sha256 $sha"
echo "sha256=$sha" >> "${GITHUB_OUTPUT:-/dev/stdout}"
