# Strøk task runner. `just gate` is the local mirror of CI — no chunk is done
# until it is green (see CLAUDE.md). It delegates to the xtask crate so the
# logic is one place and works without `just` installed too
# (`cargo xtask gate`).

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

# Golden suite only (textual-SVG insta snapshots + perceptual PNG comparison).
golden:
    cargo test -p strok-render --test golden

# Re-bless golden artifacts deliberately. STROK_BLESS=1 regenerates the
# expected PNGs; `cargo insta accept` accepts the textual-SVG snapshots.
# A PR that re-blesses MUST show the before/after diff PNG (see CLAUDE.md).
bless:
    STROK_BLESS=1 cargo test -p strok-render --test golden || true
    cargo insta accept || true
