//! C8 (E4.1/E4.2): the UX-primitive lowering produces real component structure
//! — frames → flex/grid containers, instances → component nodes, generalized
//! tokens → @theme across categories — and the cross-backend parity still holds
//! on a UX scene (the proof the IR seam survived the richer lowering).

const UX_SRC: &str = "\
documentsize 320x200

palette
  surface #faf6f0
  accent #c8863a

tokens
  space.md 16
  radius.md 12
  radius.sm 6
  font.body \"IBM Plex Sans\"

shape title template=text
  fill $accent

component button variants=[primary, ghost] props=[label:text]
  frame root layout=flex(row, gap=8, padding=10,16)
    fill $accent
    round-corners $radius.sm
    place icon shape=title at=0,0 size=10x10

frame card layout=flex(col, gap=12, padding=16) size=320x200 at=0,0
  fill $surface
  round-corners $radius.md
  place title shape=title at=0,0 size=200x20

instance cta from=button variant=primary label=\"Get started\" at=20,160
";

fn scene() -> strok_core::scene::Scene {
    strok_core::dsl_parse::parse_file(UX_SRC).expect("UX .strok parses")
}

/// A `frame layout=flex(...)` with two children emits React+Solid whose
/// *structure* reflects the layout (not one SVG blob), and the instance becomes
/// a real component element — the doc-02 acceptance.
#[test]
fn frame_layout_emits_real_structure() {
    let scene = scene();
    let opts = strok_targets::EmitOptions::default();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let src = &react.files[0].contents;

    // Flex container from the `card` frame.
    assert!(
        src.contains("flex flex-col gap-[12px]"),
        "frame layout did not lower to flex classes:\n{src}"
    );
    // Symbolic token fill (NOT a baked hex) on the container.
    assert!(
        src.contains("bg-surface"),
        "frame fill token was not kept symbolic:\n{src}"
    );
    // Radius token resolved to its numeric value.
    assert!(
        src.contains("rounded-[12px]"),
        "radius token did not resolve:\n{src}"
    );
    // The instance lowered to a component element with props.
    assert!(
        src.contains("<Button")
            && src.contains("variant=\"primary\"")
            && src.contains("label=\"Get started\""),
        "instance did not lower to a component with props:\n{src}"
    );
}

/// Each `component` lowers to its own emitted file (one UiDoc per component).
#[test]
fn components_emit_their_own_files() {
    let scene = scene();
    let opts = strok_targets::EmitOptions::default();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let names: Vec<&str> = react.files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        names.contains(&"StrokComponent.tsx"),
        "main doc missing: {names:?}"
    );
    assert!(
        names.contains(&"Button.tsx"),
        "component file missing: {names:?}"
    );
}

/// The cross-backend parity invariant must STILL hold on a UX scene: React and
/// Solid differ only by the two known dialect knobs. This is the proof the IR
/// seam survived the C8 lowering enrichment.
#[test]
fn react_solid_parity_holds_on_ux_scene() {
    let scene = scene();
    let opts = strok_targets::EmitOptions::default();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let solid = strok_targets::target_by_id("solid")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();

    assert_eq!(
        react.files.len(),
        solid.files.len(),
        "backends emitted a different number of files"
    );
    for (r, s) in react.files.iter().zip(solid.files.iter()) {
        assert_eq!(r.path, s.path, "file paths diverge");
        let canon = r
            .contents
            .replace("className=", "class=")
            .replace("dangerouslySetInnerHTML={{ __html: ", "innerHTML={")
            .replace(" }}", "}");
        assert_eq!(
            canon, s.contents,
            "React and Solid diverge by more than the two dialect knobs in {}",
            r.path
        );
        assert_ne!(
            r.contents, s.contents,
            "dialect knobs not exercised in {} (test proves nothing)",
            r.path
        );
    }
}

/// Generalized tokens flow to the tailwind @theme across categories.
#[test]
fn tailwind_emits_all_token_categories() {
    let scene = scene();
    let art = strok_targets::target_by_id("tailwind")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    let css = &art.files[0].contents;
    assert!(css.contains("--color-surface: #faf6f0;"), "{css}");
    assert!(css.contains("--spacing-md: 16;"), "{css}");
    assert!(css.contains("--radius-md: 12;"), "{css}");
    assert!(css.contains("--font-body: \"IBM Plex Sans\";"), "{css}");
    assert!(!css.contains("@theme inline"), "must not use @theme inline");
}

/// A geometry-only scene (no frames/instances) still lowers to a single
/// inline-SVG leaf — pre-C8 behavior unchanged.
#[test]
fn geometry_only_scene_unchanged() {
    let src = "\
documentsize 100x80

palette
  copper #c8863a

shape dot template=ellipse
  fill $copper

place dot shape=dot at=40,30 size=20x20
";
    let scene = strok_core::dsl_parse::parse_file(src).unwrap();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    assert_eq!(react.files.len(), 1, "geometry-only must emit one file");
    let src = &react.files[0].contents;
    assert!(
        src.contains("#c8863a"),
        "geometry-only should inline resolved colors:\n{src}"
    );
    assert!(react
        .diagnostics
        .iter()
        .any(|d| d.contains("inline-SVG leaf")));
}

// ── C9 (E4.3/E4.4) ──────────────────────────────────────────────────────────

const TEXT_SRC: &str = "\
documentsize 200x80

palette
  ink #1a1a1a

tokens
  font.body \"Inter\"

shape label template=text
  content \"Hello\"
  font-size 18
  font-family $font.body
  fill $ink
  text-anchor middle

frame card layout=flex(col, gap=8, padding=12) size=200x80 at=0,0
  place text shape=label at=0,0 size=120x20
";

/// A text `place` lowers to a real DOM `<span>` carrying selectable text — NOT
/// an inline `<svg>` with rasterized glyphs (C9 / E4.3). Typographic style is
/// symbolic where a token exists (`font-body`, `text-ink`).
#[test]
fn text_place_lowers_to_real_dom_text() {
    let scene = strok_core::dsl_parse::parse_file(TEXT_SRC).unwrap();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    let src = &react.files[0].contents;
    // Real text, in a span.
    assert!(src.contains("<span"), "text not lowered to a span:\n{src}");
    assert!(src.contains("Hello"), "text content missing:\n{src}");
    // The text place must NOT be a rasterized SVG <text>/glyph leaf.
    assert!(
        !src.contains("<text"),
        "text was rasterized into SVG, not lowered to DOM text:\n{src}"
    );
    // Symbolic font + color tokens become utility classes.
    assert!(
        src.contains("font-body"),
        "font token class missing:\n{src}"
    );
    assert!(
        src.contains("text-ink"),
        "color token class missing:\n{src}"
    );
    assert!(
        src.contains("text-[18px]"),
        "font size class missing:\n{src}"
    );
    assert!(
        src.contains("text-center"),
        "text-anchor align missing:\n{src}"
    );
}

/// React/Solid parity must still hold once text leaves are in the tree.
#[test]
fn text_lowering_keeps_parity() {
    let scene = strok_core::dsl_parse::parse_file(TEXT_SRC).unwrap();
    let opts = strok_targets::EmitOptions::default();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let solid = strok_targets::target_by_id("solid")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    for (r, s) in react.files.iter().zip(solid.files.iter()) {
        let canon = r
            .contents
            .replace("className=", "class=")
            .replace("dangerouslySetInnerHTML={{ __html: ", "innerHTML={")
            .replace(" }}", "}");
        assert_eq!(canon, s.contents, "parity broke in {}", r.path);
    }
}

/// The DTCG target emits a valid W3C DTCG document: tokens grouped by category,
/// each with `$value`, groups typed via `$type`, dimensions carrying a unit.
#[test]
fn dtcg_target_emits_valid_document() {
    let scene = scene();
    let art = strok_targets::target_by_id("dtcg")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    assert_eq!(art.files[0].path, "design-tokens.json");
    let json = &art.files[0].contents;
    // Color group, typed, with a $value.
    assert!(json.contains("\"color\": {"), "no color group:\n{json}");
    assert!(
        json.contains("\"$type\": \"color\""),
        "color group untyped:\n{json}"
    );
    assert!(
        json.contains("\"$value\": \"#faf6f0\""),
        "color value missing:\n{json}"
    );
    // Dimension tokens carry a px unit.
    assert!(
        json.contains("\"$type\": \"dimension\""),
        "dimension type missing:\n{json}"
    );
    assert!(
        json.contains("\"$value\": \"16px\""),
        "space not unit-suffixed:\n{json}"
    );
    // Font family.
    assert!(
        json.contains("\"$type\": \"fontFamily\""),
        "fontFamily type missing:\n{json}"
    );
    assert!(
        json.contains("\"$value\": \"IBM Plex Sans\""),
        "font value missing:\n{json}"
    );
    // It parses as JSON.
    assert!(json.trim_start().starts_with('{') && json.trim_end().ends_with('}'));
}

/// `dtcg` is in the target registry alongside the others.
#[test]
fn dtcg_is_registered() {
    assert!(strok_targets::TARGET_IDS.contains(&"dtcg"));
    assert!(strok_targets::target_by_id("dtcg").is_some());
}

/// A `place` that lives only inside a `component` resolves its geometry to a
/// non-empty, place-sized SVG (C8 follow-up #2, resolved in C9) — not an empty
/// full-document `<svg>`.
#[test]
fn component_internal_geometry_resolves() {
    let src = include_str!("../../examples/design-system.strok");
    let scene = strok_core::dsl_parse::parse_file(src).unwrap();
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &strok_targets::EmitOptions::default())
        .unwrap();
    // The `button` component's icon place (`glyph`, an ellipse, size 8x8) resolves
    // to an inline SVG sized to the icon — not the 360x240 empty document canvas.
    let button = react
        .files
        .iter()
        .find(|f| f.path == "Button.tsx")
        .expect("Button.tsx emitted");
    assert!(
        button.contents.contains("viewBox=\\\"0 0 8 8\\\""),
        "component-internal place not sized to the icon:\n{}",
        button.contents
    );
    assert!(
        button.contents.contains("<path"),
        "component-internal geometry resolved empty:\n{}",
        button.contents
    );
}

/// The design-system example emits to every framework target
/// end-to-end, with cross-backend parity and the right artifact shapes.
#[test]
fn design_system_emits_end_to_end() {
    let src = include_str!("../../examples/design-system.strok");
    let scene = strok_core::dsl_parse::parse_file(src).expect("design-system example parses");
    let opts = strok_targets::EmitOptions::default();

    for id in ["react", "solid", "vanilla", "tailwind", "dtcg"] {
        let art = strok_targets::target_by_id(id)
            .unwrap_or_else(|| panic!("target {id} missing"))
            .emit(&scene, &opts)
            .unwrap_or_else(|e| panic!("emit {id} failed: {e}"));
        assert!(!art.files.is_empty(), "target {id} emitted no files");
    }

    // Component files exist (button + navitem) plus the main doc.
    let react = strok_targets::target_by_id("react")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    let names: Vec<&str> = react.files.iter().map(|f| f.path.as_str()).collect();
    assert!(names.contains(&"Button.tsx"), "Button missing: {names:?}");
    assert!(names.contains(&"Navitem.tsx"), "Navitem missing: {names:?}");

    // Cross-backend parity holds on the whole design system.
    let solid = strok_targets::target_by_id("solid")
        .unwrap()
        .emit(&scene, &opts)
        .unwrap();
    assert_eq!(react.files.len(), solid.files.len());
    for (r, s) in react.files.iter().zip(solid.files.iter()) {
        let canon = r
            .contents
            .replace("className=", "class=")
            .replace("dangerouslySetInnerHTML={{ __html: ", "innerHTML={")
            .replace(" }}", "}");
        assert_eq!(canon, s.contents, "parity broke in {}", r.path);
    }
}
