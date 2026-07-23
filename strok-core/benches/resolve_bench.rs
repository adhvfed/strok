//! Perf bench (E1.6): time `resolve_scene` on a representative doc and on a
//! large (≥1000-element) synthetic doc. Pairs with `tests/perf_budget.rs`, which
//! asserts loose ceilings so an O(n²) regression in (e.g.) the `resolve` anchor
//! lookup can't land silently. The numbers are documented in CLAUDE.md.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use strok_core::dsl_parse::parse_file;
use strok_core::resolve::resolve_scene;

/// Build a synthetic `.strok` document with `n` placed shapes.
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

fn bench_resolve(c: &mut Criterion) {
    // Representative single doc (one of the examples, parsed once).
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/rose-v3.strok"
    ))
    .expect("example exists");
    let example_scene = parse_file(&example).expect("example parses");

    c.bench_function("resolve_example_rose", |b| {
        b.iter(|| resolve_scene(black_box(&example_scene)))
    });

    for n in [100usize, 1000, 2000] {
        let doc = synthetic_doc(n);
        let scene = parse_file(&doc).expect("synthetic parses");
        c.bench_function(&format!("resolve_synthetic_{n}"), |b| {
            b.iter(|| resolve_scene(black_box(&scene)))
        });
    }
}

criterion_group!(benches, bench_resolve);
criterion_main!(benches);
