# Strøk task runner. `just gate` mirrors CI through the xtask crate, so the
# same checks also work without `just` (`cargo xtask gate`).

# Run the full local gate: fmt + clippy(-D warnings) + test + golden + mutants-smoke.
gate:
    cargo xtask gate

# Full mutation-testing run (slow; nightly in CI). Uses mutants.toml scope.
mutants:
    cargo mutants

# Fuzz targets (need nightly + cargo-fuzz). Bounded run; CI runs ~60s per PR.
fuzz-parse:
    cd fuzz && cargo +nightly fuzz run fuzz_dsl_parse -- -max_total_time=60

fuzz-roundtrip:
    cd fuzz && cargo +nightly fuzz run fuzz_roundtrip -- -max_total_time=60

# Individual steps, for fast iteration.
fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

# Regenerate the checked-in example previews from their canonical sources.
examples:
    cargo build -p strok-cli
    target/debug/strok -f examples/button.strok render --out examples/button.png
    target/debug/strok -f examples/card.strok render --out examples/card.png
    target/debug/strok -f examples/design-system.strok render --out examples/design-system.png
    target/debug/strok -f examples/pelican-on-a-bicycle.strok render --out examples/pelican-on-a-bicycle.png
    target/debug/strok -f examples/rose-v3.strok render --out examples/rose-v3.png
    target/debug/strok -f examples/shape-language.strok render --out examples/shape-language.png
    target/debug/strok -f examples/std-library.strok render --out examples/std-library.png
    target/debug/strok -f examples/tea.strok render --out examples/tea.png
    target/debug/strok -f examples/field-test/launch-day.strok render --out examples/field-test/launch-day.png

# Golden suite only (textual-SVG insta snapshots + perceptual PNG comparison).
golden:
    cargo test -p strok-render --test golden

# Re-bless golden artifacts deliberately. STROK_BLESS=1 regenerates the
# expected PNGs; `cargo insta accept` accepts the textual-SVG snapshots.
# A pull request that re-blesses should show the before/after diff PNG.
bless:
    STROK_BLESS=1 cargo test -p strok-render --test golden || true
    cargo insta accept || true
