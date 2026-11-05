# Recipes (docs/HANDOVER.md §14). Run: scripts/dev/t just <recipe>
# Recipes for later milestones (e2e, preview) arrive with them.

# Build the binary.
build:
    cargo build --locked --bin firelight

# Every test in the workspace.
test:
    cargo test --locked --workspace

# fmt, clippy, cargo deny, shellcheck, actionlint, zizmor.
lint:
    cargo fmt --all --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    scripts/dev/t cargo-deny --locked check
    scripts/dev/t shellcheck -s sh scripts/dev/t scripts/dev/*.sh scripts/ci/*.sh chart/tests/*.sh
    scripts/dev/t actionlint
    scripts/dev/t zizmor --offline .github/workflows/ci.yml

# Regenerate the generated docs and schemas (CI fails when they drift).
schema:
    cargo run --locked -q --bin firelight -- config reference > docs/config.md
    rm -rf schema/v1
    cargo run --locked -q --bin firelight -- schema --out schema/v1

# The fuzz seed corpus on stable (the cargo-fuzz targets in fuzz/ need nightly).
fuzz-seeds:
    cargo test --locked -p firelight-format --test fuzz_seeds

# Check the acme fixtures with the real binary.
check-fixtures:
    cargo run --locked -q --bin firelight -- check fixtures/acme-infra fixtures/acme-app

# The chart's lint and render tests.
chart:
    sh chart/tests/render.sh

# Build the local git histories (fixtures/histories) into bare repos under target/fixtures/.
fixtures:
    sh fixtures/histories/build.sh target/fixtures
