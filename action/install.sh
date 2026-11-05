#!/bin/sh
# The Action's install step (docs/HANDOVER.md §12.3): download a release's
# archive for this runner, check it against the release's SHA256SUMS and its
# GitHub attestation, and put `firelight` into $DEST. Or copy $BINARY (for
# Firelight's own CI). Needs gh with GH_TOKEN.
#
#   VERSION=v1.2.3 DEST=/tmp/bin sh action/install.sh
set -eu
dest=${DEST:?set DEST}
mkdir -p "$dest"
if [ -n "${BINARY:-}" ]; then
  cp "$BINARY" "$dest/firelight"
else
  version=${VERSION:?set VERSION}
  if ! printf '%s' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "::error::version must look like v1.2.3, not '$version'"; exit 1
  fi
  case "$(uname -s)" in Linux) os=linux ;; Darwin) os=macos ;; *) echo "::error::unsupported OS $(uname -s)"; exit 1 ;; esac
  case "$(uname -m)" in x86_64|amd64) arch=amd64 ;; arm64|aarch64) arch=arm64 ;; *) echo "::error::unsupported CPU $(uname -m)"; exit 1 ;; esac
  name="firelight-$version-$os-$arch"
  dl=$(mktemp -d)
  trap 'rm -rf "$dl"' EXIT
  gh release download "$version" --repo prosperity-solutions/firelight \
    --pattern "$name.tar.gz" --pattern SHA256SUMS --dir "$dl"
  want=$(awk -v f="$name.tar.gz" '$2 == f || $2 == "*" f {print $1}' "$dl/SHA256SUMS")
  if command -v sha256sum >/dev/null 2>&1; then
    got=$(sha256sum "$dl/$name.tar.gz" | awk '{print $1}')
  else
    got=$(shasum -a 256 "$dl/$name.tar.gz" | awk '{print $1}')
  fi
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    echo "::error::$name.tar.gz doesn't match SHA256SUMS (expected '$want', got '$got')"; exit 1
  fi
  gh attestation verify "$dl/$name.tar.gz" --repo prosperity-solutions/firelight \
    --signer-workflow prosperity-solutions/firelight/.github/workflows/ci.yml \
    --source-ref refs/heads/main --deny-self-hosted-runners >/dev/null
  # Release archives hold firelight-<version>-<os>-<arch>/firelight.
  tar -xzf "$dl/$name.tar.gz" -C "$dl" "$name/firelight"
  mv "$dl/$name/firelight" "$dest/firelight"
fi
chmod +x "$dest/firelight"
"$dest/firelight" version
