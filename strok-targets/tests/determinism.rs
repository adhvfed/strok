//! Determinism + end-to-end emission from a real `.strok` source.
//!
//! Codegen must be a pure function of the scene: emitting twice yields
//! byte-identical output (so a design edit produces a minimal, reviewable git
//! diff — design doc §4.2). Also exercises the full `Scene → UiDoc → source`
//! path through the public `target_by_id` registry, and the Tailwind token
//! bridge.

const SRC: &str = "\
documentsize 100x80

palette
  copper #c8863a
  surface #faf6f0

shape bg template=rectangle
  fill $surface
shape dot template=ellipse
  fill $copper

place bg shape=bg at=0,0 size=100x80
place dot shape=dot at=40,30 size=20x20
";

fn scene() -> strok_core::scene::Scene {
    strok_core::dsl_parse::parse_file(SRC).expect("sample .strok parses")
}

#[test]
fn every_code_target_is_deterministic() {
    let scene = scene();
    let opts = strok_targets::EmitOptions::default();
    for id in ["react", "solid", "vanilla"] {
        let target = strok_targets::target_by_id(id).unwrap();
        let a = target.emit(&scene, &opts).unwrap();
        let b = target.emit(&scene, &opts).unwrap();
        assert_eq!(a.files, b.files, "{id} emission was not deterministic");
        assert!(
            !a.files[0].contents.is_empty(),
            "{id} produced empty output"
        );
    }
}

#[test]
fn code_targets_inline_the_resolved_svg() {
    let scene = scene();
    let opts = strok_targets::EmitOptions::default();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let src = &react.files[0].contents;
    // Tokens resolved to concrete colors in the inlined SVG (so a browser paints it).
    assert!(
        src.contains("#c8863a") || src.contains("#faf6f0"),
        "expected resolved palette colors in svg: {src}"
    );
    // Lowering diagnostic is surfaced, not silent.
    assert!(react
        .diagnostics
        .iter()
        .any(|d| d.contains("inline-SVG leaf")));
}

#[test]
fn tailwind_target_emits_theme_block() {
    let scene = scene();
    let art = strok_targets::target_by_id("tailwind")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    let css = &art.files[0].contents;
    assert!(css.contains("@theme {"), "expected @theme block: {css}");
    assert!(
        css.contains("--color-copper: #c8863a;"),
        "missing copper token: {css}"
    );
    assert!(
        css.contains("--color-surface: #faf6f0;"),
        "missing surface token: {css}"
    );
    // Tailwind v4 rule: plain @theme, not @theme inline.
    assert!(!css.contains("@theme inline"), "must not use @theme inline");
}
