#!/bin/sh
# Update Cargo.lock, taking only versions published at least 7 days ago
# (docs/HANDOVER.md §17.3). Arguments go to `cargo update` (e.g. -p serde).
# Uses cargo's unstable min-publish-age filter through RUSTC_BOOTSTRAP, for this
# one resolution only; tools/depage enforces the same rule in CI either way.
# Then re-applies the dated exceptions in .github/fresh-deps.txt (a fix for an
# advisory that is younger than 7 days), which the filter would undo.
set -eu
cd "$(dirname "$0")/../.."
RUSTC_BOOTSTRAP=1 cargo -Zmin-publish-age \
  --config 'registry.global-min-publish-age="7 days"' update "$@"
grep -Ev '^[[:space:]]*(#|$)' .github/fresh-deps.txt 2>/dev/null |
  while read -r name version _; do
    case $name in toolchain) continue ;; esac
    if grep -q "^name = \"$name\"$" Cargo.lock; then
      cargo update -q -p "$name" --precise "$version" ||
        echo "cargo-update: could not apply the exception $name $version" >&2
    fi
  done
