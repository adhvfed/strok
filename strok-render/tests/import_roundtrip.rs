//! Integration test for the SVG importer (EXP-3): import a real SVG, render the
//! *original* SVG and the *imported → resolved* SVG at the same size, and assert
//! the two are perceptually close (within the golden tolerance). This is the
//! measure-of-success loop for structure recovery: the imported .strok must look
//! like the source.

use strok_core::{dsl_emit, dsl_parse, import_svg, resolve};
use strok_render::{diff_png_bytes, render_svg_string, RenderOptions};

/// Import an SVG string, round-trip it through emit/parse, and return the
/// resolved SVG of the imported scene plus its document size.
fn import_resolved(svg: &str) -> (String, f64, f64) {
    let result = import_svg::import_svg(svg);
    let dsl = dsl_emit::emit_scene(&result.scene);
    // The emitted DSL must round-trip (parse(emit) == emit(parse(emit))).
    let reparsed = dsl_parse::parse_file(&dsl).expect("imported DSL re-parses");
    let dsl2 = dsl_emit::emit_scene(&reparsed);
    assert_eq!(dsl, dsl2, "import emit must round-trip");
    let (w, h) = (result.scene.document_size.w, result.scene.document_size.h);
    // Resolve palette tokens ($blue-1, …) to concrete colors before rendering.
    let themed = resolve::apply_scheme(&reparsed, None).expect("apply scheme");
    (resolve::resolve_scene(&themed), w, h)
}

/// Render both the source SVG and the imported/resolved SVG and compare.
fn assert_close(svg: &str, w: f64, h: f64) {
    let opts = RenderOptions {
        width: Some(w as u32),
        height: Some(h as u32),
        background: Some("#ffffff".into()),
        color: Some("#000000".into()),
    };
    let (imported_svg, iw, ih) = import_resolved(svg);
    assert_eq!((iw, ih), (w, h), "document size recovered");

    let orig =
        render_svg_string(svg, w as u32, h as u32, w, h, &opts).expect("render original svg");
    let imp = render_svg_string(&imported_svg, w as u32, h as u32, w, h, &opts)
        .expect("render imported svg");

    let (stats, _diff) = diff_png_bytes(&orig, &imp).expect("diff");
    assert!(
        stats.within_golden_tolerance(),
        "imported render drifted from source: mean Δ {:.2}/255, {:.2}% px changed",
        stats.mean_abs,
        stats.changed_fraction * 100.0,
    );
}

#[test]
fn import_primitives_close() {
    // rects (rounded), circle, ellipse, line, translated group, reused colors.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">
        <rect x="0" y="0" width="200" height="200" fill="#f5f0e8"/>
        <g transform="translate(10,10)">
          <rect x="0" y="0" width="70" height="50" rx="8" fill="#3366cc"/>
          <rect x="0" y="70" width="70" height="50" rx="8" fill="#3366cc"/>
          <circle cx="140" cy="35" r="28" fill="#cc4444"/>
          <ellipse cx="140" cy="110" rx="30" ry="20" fill="#cc4444"/>
        </g>
        <line x1="20" y1="180" x2="180" y2="180" stroke="#222222" stroke-width="4"/>
    </svg>"##;
    assert_close(svg, 200.0, 200.0);
}

#[test]
fn import_paths_close() {
    // cubic + quadratic + arc segments, a polygon, and a viewBox offset.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-10 -10 120 120">
        <path d="M0 0 C20 0 20 40 40 40 Q60 40 60 0 A20 20 0 0 1 100 0 L100 60 Z"
              fill="#2288aa" stroke="#114455" stroke-width="2"/>
        <polygon points="10,80 40,110 10,110" fill="#aa5522"/>
        <polyline points="60,80 80,100 100,80" fill="none" stroke="#333333" stroke-width="3"/>
    </svg>"##;
    assert_close(svg, 120.0, 120.0);
}
