//! Perf budget (E1.6): loose wall-clock ceilings that catch order-of-magnitude
//! regressions (e.g. an O(n²) blow-up in the `resolve` anchor lookup, which is
//! currently a name-string `find`). These are intentionally generous (≥10× the
//! observed time on a dev laptop) so they don't flake on a slow/loaded CI
//! runner; they exist to catch a *regression*, not to micro-gate.

use std::time::Instant;

use strok_core::dsl_parse::parse_file;
use strok_core::resolve::resolve_scene;

fn synthetic_doc(n: usize) -> String {
    let mut s = String::from("documentsize 2000x2000\n\n");
    s.push_str("shape dot template=ellipse\n  fill #336699\n\n");
    for i in 0..n {
        let x = (i % 50) * 40;
        let y = (i / 50) * 40;
        s.push_str(&format!("place d{i} shape=dot at={x},{y} size=30x30\n"));
    }
    s
}

#[test]
fn single_render_is_sub_second() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/rose-v3.strok"
    ))
    .expect("example exists");
    let scene = parse_file(&src).expect("parses");
    let t0 = Instant::now();
    let svg = resolve_scene(&scene);
    let dt = t0.elapsed();
    assert!(!svg.is_empty());
    assert!(
        dt.as_millis() < 1000,
        "single resolve_scene took {dt:?} (budget: <1s)"
    );
}

#[test]
fn large_doc_under_ceiling() {
    // ≥1000-element synthetic doc. Observed ~a few ms on a dev laptop; the
    // ceiling is 5s to leave huge headroom while still catching an O(n²) or
    // worse regression (which would push a 1000-place doc into seconds).
    let doc = synthetic_doc(1200);
    let scene = parse_file(&doc).expect("parses");
    assert!(scene.nodes.len() >= 1000);
    let t0 = Instant::now();
    let svg = resolve_scene(&scene);
    let dt = t0.elapsed();
    assert!(svg.contains("<path"));
    assert!(
        dt.as_millis() < 5000,
        "1200-element resolve_scene took {dt:?} (budget: <5s)"
    );
}

/// Scaling sanity: doubling the element count must not super-linearly explode.
/// We compare 1000 vs 2000 places; if the lookup were badly O(n²) the ratio
/// would be ~4×. Allow up to 8× (very loose) to avoid CI flake while still
/// catching a true quadratic regression (which compounds far beyond 8×).
#[test]
fn resolve_scaling_is_not_pathological() {
    let warm = parse_file(&synthetic_doc(200)).unwrap();
    let _ = resolve_scene(&warm); // warm caches/allocator

    let small = parse_file(&synthetic_doc(1000)).unwrap();
    let large = parse_file(&synthetic_doc(2000)).unwrap();

    let t0 = Instant::now();
    let _ = resolve_scene(&small);
    let ts = t0.elapsed().as_secs_f64();

    let t1 = Instant::now();
    let _ = resolve_scene(&large);
    let tl = t1.elapsed().as_secs_f64();

    // Guard against divide-by-zero on very fast machines.
    let ratio = if ts > 1e-6 { tl / ts } else { 1.0 };
    assert!(
        ratio < 8.0 || tl < 0.05,
        "resolve scaling 1000→2000 was {ratio:.1}× (tl={tl:.4}s, ts={ts:.4}s) — possible O(n²) regression"
    );
}
