# Contributing

Thank you for helping improve Strøk. Bug fixes, focused features,
documentation, test cases, and design-file examples are welcome.

## Development

Install a stable Rust toolchain with `rustfmt` and `clippy`, then run:

```sh
cargo xtask gate
```

The gate checks formatting, lints every target, runs the workspace tests, and
validates the golden render suite. Please add the smallest test that proves a
behavioral change.

## Golden renders

SVG snapshots are the authoritative, platform-independent rendering contract.
Geometry-only fixtures also have perceptual PNG comparisons. If an intentional
rendering change affects them, inspect the output before accepting it:

```sh
cargo install cargo-insta
cargo insta review
STROK_BLESS=1 cargo test -p strok-render --test golden
```

Do not bless an unexplained difference.

## Optional checks

Fuzz and mutation checks are slower and need separate tools:

```sh
cargo install cargo-fuzz cargo-mutants --locked
cd fuzz && cargo +nightly fuzz run fuzz_dsl_parse
cargo mutants
```

## Pull requests

Keep changes scoped, explain the user-visible result, and mention the commands
used to verify it. By contributing, you agree that your work is licensed under
the project's dual MIT/Apache-2.0 terms.
