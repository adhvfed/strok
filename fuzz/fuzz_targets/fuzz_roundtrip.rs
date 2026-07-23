//! Fuzz target: for any input that parses, the round-trip invariant must hold —
//! `parse(emit(parse(input)))` reproduces the same scene, and neither emit nor
//! re-parse panics. (E1.3)
#![no_main]

use libfuzzer_sys::fuzz_target;

use strok_core::dsl_emit::emit_scene;
use strok_core::dsl_parse::parse_file;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(scene) = parse_file(s) else {
        return;
    };
    // Emit must not panic, and the result must re-parse...
    let dsl = emit_scene(&scene);
    let reparsed = match parse_file(&dsl) {
        Ok(sc) => sc,
        Err(e) => panic!("emit produced un-parseable DSL: {e}\n--- DSL ---\n{dsl}"),
    };
    // ...to the same scene (round-trip stability). A second emit must be
    // byte-identical (emit∘parse idempotence).
    let dsl2 = emit_scene(&reparsed);
    if dsl != dsl2 {
        panic!("round-trip not stable\n--- once ---\n{dsl}\n--- twice ---\n{dsl2}");
    }
});
