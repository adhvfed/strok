
use super::*;

#[test]
fn garbage_bytes_never_panic() {
    for s in [
        "",
        "<",
        "<svg",
        "<<<>>>",
        "&;&#;&#x;",
        "<svg><rect width=",
        "<svg viewBox=\"0 0 x y\"><path d=\"M z q q q\"/></svg>",
        "not xml at all { } ; 123",
        "<?xml?><!DOCTYPE><![CDATA[",
    ] {
        let _ = import_svg(s);
    }
}

#[test]
fn imports_rect_as_rectangle() {
    let svg = r##"<svg viewBox="0 0 100 100"><rect x="10" y="20" width="30" height="40" fill="#ff0000"/></svg>"##;
    let r = import_svg(svg);
    assert_eq!(r.scene.document_size.w, 100.0);
    assert_eq!(r.scene.shapes.len(), 1);
    assert_eq!(r.scene.shapes[0].template, Template::Rectangle);
    match &r.scene.nodes[0] {
        SceneNode::Place(p) => {
            assert_eq!(p.position, PlacePosition::At(10.0, 20.0));
            assert_eq!(p.size, Some(Dimension { w: 30.0, h: 40.0 }));
        }
        _ => panic!("expected place"),
    }
}

#[test]
fn imports_circle_as_ellipse() {
    let svg = r##"<svg width="50" height="50"><circle cx="25" cy="25" r="10"/></svg>"##;
    let r = import_svg(svg);
    assert_eq!(r.scene.shapes[0].template, Template::Ellipse);
    match &r.scene.nodes[0] {
        SceneNode::Place(p) => {
            assert_eq!(p.position, PlacePosition::At(15.0, 15.0));
            assert_eq!(p.size, Some(Dimension { w: 20.0, h: 20.0 }));
        }
        _ => panic!("expected place"),
    }
}

#[test]
fn reuses_identical_shapes() {
    let svg = r##"<svg viewBox="0 0 100 100">
            <rect x="0" y="0" width="10" height="10" fill="#0000ff"/>
            <rect x="20" y="20" width="10" height="10" fill="#0000ff"/>
        </svg>"##;
    let r = import_svg(svg);
    assert_eq!(r.scene.shapes.len(), 1, "identical rects share a shape def");
    assert_eq!(r.scene.nodes.len(), 2);
}

#[test]
fn extracts_palette_for_repeated_color() {
    let svg = r##"<svg viewBox="0 0 100 100">
            <rect x="0" y="0" width="10" height="10" fill="#123456"/>
            <rect x="20" y="20" width="10" height="10" fill="#123456"/>
        </svg>"##;
    let r = import_svg(svg);
    assert_eq!(r.scene.palette.tokens.len(), 1, "repeated color → 1 token");
    assert!(matches!(r.scene.shapes[0].fill(), Some(Color::Token(_))));
}

#[test]
fn round_trips_through_emit() {
    let svg = r##"<svg viewBox="0 0 200 200">
            <g id="grp">
              <rect x="10" y="10" width="50" height="30" rx="4" fill="#3366cc" stroke="#000000" stroke-width="2"/>
              <circle cx="120" cy="60" r="20" fill="#3366cc"/>
              <path d="M10 100 L60 120 C80 130 90 150 60 160 Z" fill="#cc3366"/>
            </g>
        </svg>"##;
    let r = import_svg(svg);
    let dsl = crate::dsl_emit::emit_scene(&r.scene);
    let reparsed = crate::dsl_parse::parse_file(&dsl)
        .unwrap_or_else(|e| panic!("emitted DSL failed to parse: {e}\n{dsl}"));
    let dsl2 = crate::dsl_emit::emit_scene(&reparsed);
    assert_eq!(dsl, dsl2, "import → emit must round-trip");
}

#[test]
fn path_with_cubics_and_arcs() {
    let svg = r##"<svg viewBox="0 0 100 100"><path d="M10 10 C20 20 30 20 40 10 A5 5 0 0 1 50 10 L60 60 Z"/></svg>"##;
    let r = import_svg(svg);
    assert_eq!(r.scene.shapes[0].template, Template::Path);
    let has_arc = r.scene.shapes[0].operations.iter().any(|o| {
        matches!(
            o,
            Operation::AddPoint {
                mode: Some(PointMode::Arc),
                ..
            }
        )
    });
    assert!(has_arc, "arc segment preserved as mode=arc");
}

#[test]
fn unsupported_element_warns() {
    let svg =
        r##"<svg viewBox="0 0 100 100"><image href="x.png"/><rect width="10" height="10"/></svg>"##;
    let r = import_svg(svg);
    assert!(r.warnings.iter().any(|w| w.message.contains("image")));
}
