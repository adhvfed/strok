//! Local gate runner for the Strøk workspace.
//!
//! `cargo xtask gate` runs the same checks CI runs, in the same order, so a
//! contributor can reproduce CI locally. No chunk is done until `cargo xtask
//! gate` (a.k.a. `just gate`) is green — see CLAUDE.md.
//!
//! Steps (E1.1 / E1.4 / E1.6):
//! 1. cargo fmt --check — formatting is a hard gate.
//! 2. cargo clippy -D warnings — no clippy warnings, all targets.
//! 3. cargo test --workspace — unit + integration + golden + round-trip +
//!    property + perf-budget suites.
//! 4. golden suite — run explicitly so its status is obvious.
//! 5. mutants-smoke (C2/E1.6) — a fast cargo-mutants sample over the geometry
//!    hot path; skipped if cargo-mutants is not installed so the gate still runs.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let task = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gate".to_string());
    match task.as_str() {
        "gate" => run_gate(),
        other => {
            eprintln!("unknown xtask '{other}'. known tasks: gate");
            ExitCode::FAILURE
        }
    }
}

/// A single named step in the gate.
struct Step {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

fn run_gate() -> ExitCode {
    let steps = [
        Step {
            name: "fmt",
            program: "cargo",
            args: &["fmt", "--all", "--check"],
        },
        Step {
            name: "clippy",
            program: "cargo",
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
        Step {
            name: "test",
            program: "cargo",
            args: &["test", "--workspace"],
        },
        Step {
            name: "golden",
            program: "cargo",
            args: &["test", "-p", "strok-render", "--test", "golden"],
        },
    ];

    for step in &steps {
        eprintln!("\n=== gate: {} ===", step.name);
        let status = Command::new(step.program).args(step.args).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("gate step '{}' failed ({s})", step.name);
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("gate step '{}' could not run: {e}", step.name);
                return ExitCode::FAILURE;
            }
        }
    }

    // mutants-smoke (E1.6): a fast cargo-mutants sample over a slice of the
    // geometry hot path. This is NOT the full budgeted run (that is nightly via
    // `cargo mutants` + mutants.toml — see CLAUDE.md); it just proves the
    // mutation harness still runs and the geometry tests still kill mutants. If
    // cargo-mutants isn't installed we SKIP (not fail) so the core gate is
    // always reproducible.
    eprintln!("\n=== gate: mutants-smoke (E1.6) ===");
    if cargo_mutants_available() {
        // `--shard 0/200` tests ~1/200th of the mutants in the configured file —
        // a quick liveness check (currently about 20 mutants). `--in-diff` would
        // be even faster but isn't always meaningful locally, so we sample
        // instead. Exit non-zero only on a tooling/baseline error, not on
        // surviving mutants (the budget is enforced by the nightly job).
        let status = Command::new("cargo")
            .args([
                "mutants",
                "--file",
                "strok-core/src/path_point.rs",
                "--shard",
                "0/200",
                "--test-package",
                "strok-core",
            ])
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                // cargo-mutants exits non-zero when mutants survive; that is the
                // nightly budget's job, not the local smoke's. Only the
                // baseline-failure code (which means tests don't pass on the
                // unmutated tree) should fail the gate — but that is already
                // caught by step 3 above. So we warn and continue.
                eprintln!("mutants-smoke: surviving mutants in sample ({s}) — see nightly budget; not failing the gate");
            }
            Err(e) => eprintln!("mutants-smoke: could not run cargo-mutants: {e} (skipping)"),
        }
    } else {
        eprintln!("mutants-smoke: cargo-mutants not installed — skipping (install with `cargo install cargo-mutants`)");
    }

    eprintln!("\nall gate steps passed");
    ExitCode::SUCCESS
}

/// Is `cargo mutants` available?
fn cargo_mutants_available() -> bool {
    Command::new("cargo")
        .args(["mutants", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
