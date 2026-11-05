# Recipes (docs/HANDOVER.md §14). Run: scripts/dev/t just <recipe>
# Recipes for later milestones (e2e, fuzz-seeds, preview, fixtures) arrive with them.

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

# The chart's lint and render tests.
chart:
    sh chart/tests/render.sh
