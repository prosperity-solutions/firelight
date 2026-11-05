#!/bin/sh
# Update Cargo.lock, taking only versions published at least 7 days ago
# (docs/HANDOVER.md §17.3). Arguments go to `cargo update` (e.g. -p serde).
# Uses cargo's unstable min-publish-age filter through RUSTC_BOOTSTRAP, for this
# one resolution only; tools/depage enforces the same rule in CI either way.
set -eu
cd "$(dirname "$0")/../.."
RUSTC_BOOTSTRAP=1 exec cargo -Zmin-publish-age \
  --config 'registry.global-min-publish-age="7 days"' update "$@"
