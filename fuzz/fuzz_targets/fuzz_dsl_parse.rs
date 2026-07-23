//! Fuzz target: `parse_file` must never panic on arbitrary bytes — every
//! failure path returns a `Result::Err` via `error.rs`. (E1.3)
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Only UTF-8 inputs are meaningful for a text DSL; non-UTF-8 is rejected
    // upstream by the CLI before reaching the parser.
    if let Ok(s) = std::str::from_utf8(data) {
        // Must return cleanly (Ok or Err) — never panic.
        let _ = strok_core::dsl_parse::parse_file(s);
    }
});
