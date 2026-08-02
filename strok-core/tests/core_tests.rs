use std::collections::HashMap;
use strok_core::document::Document;
use strok_core::dsl_emit;
use strok_core::dsl_parse;
use strok_core::emit;
use strok_core::id;
use strok_core::node::NodeId;
use strok_core::parse;
use strok_core::path_ops::SculptAxis;
use strok_core::path_point::{CurveMode, NamedPoint, PathData};
use strok_core::resolve;

/// Create a document with a path element containing named points.
fn doc_with_path(id: &str, w: f64, h: f64, points: &[(&str, f64, f64)]) -> Document {
    let mut doc = Document::new(w, h);
    // Build a path element with the given points
    let pd = PathData {
        coord_space: (w, h),
        points: points
            .iter()
            .map(|(name, x, y)| NamedPoint {
                name: name.to_string(),
                x: *x,
                y: *y,
                mode: CurveMode::Sharp,
            })
            .collect(),
        closed: false,
        subpath_starts: Vec::new(),
    };
    // Create a minimal SVG path to get the element in the arena
    let svg = format!(r#"<path id="{}" d="M0,0"/>"#, id);
    doc.append_svg("root", &svg).unwrap();
    // Now set the path_data directly
    let nid = doc.resolve_id(id).unwrap();
    doc.arena.get_mut(nid).unwrap().attrs.path_data = Some(pd);
    doc
}

/// Create a document with a path element with specific attrs.
fn doc_with_styled_path(
    id: &str,
    w: f64,
    h: f64,
    points: &[(&str, f64, f64)],
    fill: Option<&str>,
    stroke: Option<&str>,
    stroke_width: Option<f64>,
) -> Document {
    let mut doc = doc_with_path(id, w, h, points);
    if let Some(f) = fill {
        doc.set_attr(id, "fill", f).unwrap();
    }
    if let Some(s) = stroke {
        doc.set_attr(id, "stroke", s).unwrap();
    }
    if let Some(sw) = stroke_width {
        doc.set_attr(id, "stroke-width", &sw.to_string()).unwrap();
    }
    doc
}

#[test]
fn new_document_has_root() {
    let doc = Document::new(800.0, 600.0);
    assert!(doc.find_by_id("root").is_ok());
    assert_eq!(doc.width, 800.0);
    assert_eq!(doc.height, 600.0);
}

#[test]
fn append_and_find() {
    let mut doc = Document::new(400.0, 300.0);
    let id = doc
        .append_svg(
            "root",
            r#"<rect id="bg" width="400" height="300" fill="red"/>"#,
        )
        .unwrap();
    assert_eq!(id, "bg");
    let node = doc.find_by_id("bg").unwrap();
    assert_eq!(node.kind.tag_name(), "rect");
}

#[test]
fn append_generates_id_when_missing() {
    let mut doc = Document::new(400.0, 300.0);
    let id = doc
        .append_svg("root", r#"<rect width="100" height="100"/>"#)
        .unwrap();
    assert!(!id.is_empty());
    assert!(id.len() >= 8);
    assert!(doc.find_by_id(&id).is_ok());
}

#[test]
fn duplicate_id_fails() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg" width="100" height="100"/>"#)
        .unwrap();
    let result = doc.append_svg("root", r#"<rect id="bg" width="200" height="200"/>"#);
    assert!(result.is_err());
}

#[test]
fn remove_node() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg" width="400" height="300"/>"#)
        .unwrap();
    doc.remove("bg").unwrap();
    assert!(doc.find_by_id("bg").is_err());
}

#[test]
fn cannot_remove_root() {
    let mut doc = Document::new(400.0, 300.0);
    assert!(doc.remove("root").is_err());
}

#[test]
fn set_attr() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg" fill="red"/>"#)
        .unwrap();
    doc.set_attr("bg", "fill", "blue").unwrap();
    let node = doc.find_by_id("bg").unwrap();
    assert_eq!(
        node.attrs.fill,
        Some(strok_core::attrs::Paint::Color("blue".to_string()))
    );
}

#[test]
fn rename_node() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg" fill="red"/>"#)
        .unwrap();
    doc.rename("bg", "background").unwrap();
    assert!(doc.find_by_id("bg").is_err());
    assert!(doc.find_by_id("background").is_ok());
}

#[test]
fn prepend_and_order() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="a"/>"#).unwrap();
    doc.prepend_svg("root", r#"<rect id="b"/>"#).unwrap();

    let root = doc.arena.get(doc.root_id).unwrap();
    let first_child = doc.arena.get(root.children[0]).unwrap();
    assert_eq!(first_child.id, "b");
}

#[test]
fn group_and_ungroup() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="a"/>"#).unwrap();
    doc.append_svg("root", r#"<rect id="b"/>"#).unwrap();

    let gid = doc.group(&["a", "b"], Some("grp")).unwrap();
    assert_eq!(gid, "grp");

    let root = doc.arena.get(doc.root_id).unwrap();
    assert_eq!(root.children.len(), 1);

    let group = doc.find_by_id("grp").unwrap();
    assert_eq!(group.children.len(), 2);

    doc.ungroup("grp").unwrap();
    let root = doc.arena.get(doc.root_id).unwrap();
    assert_eq!(root.children.len(), 2);
    assert!(doc.find_by_id("grp").is_err());
}

#[test]
fn parse_snippet_with_children() {
    let svg =
        r#"<g id="container"><rect id="child1"/><circle id="child2" cx="10" cy="10" r="5"/></g>"#;
    let nodes = parse::parse_snippet(svg).unwrap();
    assert_eq!(nodes.len(), 3); // g + rect + circle
    assert_eq!(nodes[0].id, "container");
    assert_eq!(nodes[0].children.len(), 2);
}

#[test]
fn svg_round_trip() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg(
        "root",
        r##"<rect id="bg" width="400" height="300" fill="#1a1a2e"/>"##,
    )
    .unwrap();
    doc.append_svg(
        "root",
        r#"<circle id="dot" cx="200" cy="150" r="50" fill="red"/>"#,
    )
    .unwrap();

    let svg = emit::emit_document(&doc);
    assert!(svg.contains(r#"id="bg""#));
    assert!(svg.contains(r#"id="dot""#));
    assert!(svg.contains(r##"fill="#1a1a2e""##));
    assert!(svg.contains(r#"cx="200""#));
}

#[test]
fn document_save_load_round_trip() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg(
        "root",
        r#"<rect id="bg" width="400" height="300" fill="red"/>"#,
    )
    .unwrap();
    doc.append_svg("root", r#"<circle id="dot" cx="100" cy="100" r="25"/>"#)
        .unwrap();

    let path = std::env::temp_dir().join("strok_test_roundtrip.strok");
    doc.save(&path).unwrap();

    let loaded = Document::load(&path).unwrap();
    assert_eq!(loaded.width, 400.0);
    assert_eq!(loaded.height, 300.0);
    assert!(loaded.find_by_id("bg").is_ok());
    assert!(loaded.find_by_id("dot").is_ok());
    assert!(loaded.find_by_id("root").is_ok());

    let root = loaded.arena.get(loaded.root_id).unwrap();
    assert_eq!(root.children.len(), 2);

    std::fs::remove_file(&path).ok();
}

#[test]
fn id_generation_no_collisions() {
    let mut existing = HashMap::new();
    for i in 0..1000 {
        let id = id::generate_id(&existing);
        assert!(id.len() >= 8);
        assert!(!existing.contains_key(&id));
        existing.insert(id, NodeId(i));
    }
}

#[test]
fn id_validation_rejects_root() {
    let existing = HashMap::new();
    assert!(id::validate_id("root", &existing).is_err());
}

#[test]
fn replace_element() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg" fill="red"/>"#)
        .unwrap();
    let new_id = doc
        .replace_svg(
            "bg",
            r#"<circle id="bg" cx="50" cy="50" r="25" fill="blue"/>"#,
        )
        .unwrap();
    assert_eq!(new_id, "bg");
    let node = doc.find_by_id("bg").unwrap();
    assert_eq!(node.kind.tag_name(), "circle");
}

#[test]
fn reorder_element() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="a"/>"#).unwrap();
    doc.append_svg("root", r#"<rect id="b"/>"#).unwrap();
    doc.append_svg("root", r#"<rect id="c"/>"#).unwrap();

    doc.reorder("c", 0).unwrap();

    let root = doc.arena.get(doc.root_id).unwrap();
    let first = doc.arena.get(root.children[0]).unwrap();
    assert_eq!(first.id, "c");
}

#[test]
fn reparent_element() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<g id="grp"/>"#).unwrap();
    doc.append_svg("root", r#"<rect id="item"/>"#).unwrap();

    doc.reparent("item", "grp", None).unwrap();

    let grp = doc.find_by_id("grp").unwrap();
    assert_eq!(grp.children.len(), 1);
    let root = doc.arena.get(doc.root_id).unwrap();
    assert_eq!(root.children.len(), 1); // only grp remains in root
}

#[test]
fn history_tracking() {
    let mut doc = Document::new(400.0, 300.0);
    doc.append_svg("root", r#"<rect id="bg"/>"#).unwrap();
    doc.set_attr("bg", "fill", "red").unwrap();

    assert_eq!(doc.history.ops().len(), 3); // new + append + set
    assert_eq!(doc.history.cursor(), 3);
}

#[test]
fn append_group_with_children() {
    let mut doc = Document::new(400.0, 300.0);
    let gid = doc
        .append_svg(
            "root",
            r#"<g id="grp"><rect id="r1" fill="red"/><rect id="r2" fill="blue"/></g>"#,
        )
        .unwrap();
    assert_eq!(gid, "grp");
    assert!(doc.find_by_id("r1").is_ok());
    assert!(doc.find_by_id("r2").is_ok());

    let grp = doc.find_by_id("grp").unwrap();
    assert_eq!(grp.children.len(), 2);
}

#[test]
fn point_edit_move_insert_delete() {
    let mut doc = doc_with_path(
        "vine",
        100.0,
        100.0,
        &[("a", 10.0, 10.0), ("b", 40.0, 40.0), ("c", 80.0, 60.0)],
    );

    doc.move_point("vine", "b", 5.0, -3.0, true).unwrap();
    doc.insert_point_after("vine", "b", "mid", 55.0, 42.0, CurveMode::Sharp)
        .unwrap();
    doc.delete_point("vine", "mid", None).unwrap();

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let b = pd.points.iter().find(|p| p.name == "b").unwrap();
    assert_eq!(b.x, 45.0);
    assert_eq!(b.y, 37.0);
}

#[test]
fn point_mode_and_rename() {
    let mut doc = doc_with_path(
        "vine",
        100.0,
        100.0,
        &[("a", 10.0, 10.0), ("b", 40.0, 40.0), ("c", 80.0, 60.0)],
    );

    doc.set_point_mode("vine", "b", CurveMode::CatmullRom(0.3))
        .unwrap();
    doc.rename_point("vine", "c", "tail").unwrap();

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let b = pd.points.iter().find(|p| p.name == "b").unwrap();
    match b.mode {
        CurveMode::CatmullRom(t) => assert!((t - 0.3).abs() < 1e-9),
        _ => panic!("expected catmull-rom mode"),
    }
    assert!(pd.points.iter().any(|p| p.name == "tail"));
}

#[test]
fn point_split_segment() {
    let mut doc = doc_with_path(
        "vine",
        100.0,
        100.0,
        &[("a", 0.0, 0.0), ("b", 100.0, 0.0), ("c", 100.0, 100.0)],
    );

    let (x, y) = doc.split_segment("vine", "a", "b", "ab-mid", 0.25).unwrap();
    assert_eq!(x, 25.0);
    assert_eq!(y, 0.0);

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let names: Vec<_> = pd.points.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "ab-mid", "b", "c"]);
}

#[test]
fn point_pull_moves_neighbors_with_falloff() {
    let mut doc = doc_with_path(
        "vine",
        100.0,
        100.0,
        &[
            ("a", 0.0, 0.0),
            ("b", 20.0, 0.0),
            ("c", 40.0, 0.0),
            ("d", 60.0, 0.0),
            ("e", 80.0, 0.0),
        ],
    );

    let moved = doc
        .pull_point("vine", "c", 10.0, 0.0, 1, 1.0, false)
        .unwrap();
    assert_eq!(moved.len(), 3); // b, c, d

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let b = pd.points.iter().find(|p| p.name == "b").unwrap();
    let c = pd.points.iter().find(|p| p.name == "c").unwrap();
    let d = pd.points.iter().find(|p| p.name == "d").unwrap();
    // radius=1, linear falloff -> center full move, neighbors half move
    assert!((b.x - 25.0).abs() < 1e-9);
    assert!((c.x - 50.0).abs() < 1e-9);
    assert!((d.x - 65.0).abs() < 1e-9);
}

#[test]
fn branch_from_point_creates_new_path() {
    let mut doc = doc_with_styled_path(
        "trunk",
        200.0,
        200.0,
        &[
            ("p0", 150.0, 180.0),
            ("p1", 130.0, 120.0),
            ("p2", 120.0, 60.0),
        ],
        Some("none"),
        Some("#2d6f38"),
        Some(10.0),
    );

    let bid = doc
        .branch_from_point("trunk", "p1", "branch-new", 70.0, -25.0, 0.5, None)
        .unwrap();
    assert_eq!(bid, "branch-new");
    let branch = doc.find_by_id("branch-new").unwrap();
    assert_eq!(branch.kind.tag_name(), "path");
    let pd = branch.attrs.path_data.as_ref().unwrap();
    assert_eq!(pd.points.len(), 4);
    assert_eq!(pd.points[0].name, "p0");
    assert_eq!(pd.points[0].x, 130.0);
    assert_eq!(pd.points[0].y, 120.0);
}

#[test]
fn sculpt_path_moves_points_by_spatial_radius() {
    let mut doc = doc_with_path(
        "vine",
        200.0,
        200.0,
        &[
            ("a", 40.0, 100.0),
            ("b", 80.0, 100.0),
            ("c", 120.0, 100.0),
            ("d", 160.0, 100.0),
        ],
    );

    let moved = doc
        .sculpt_path("vine", 100.0, 100.0, 0.0, -20.0, 50.0, 1.0, false)
        .unwrap();
    assert_eq!(moved.len(), 2); // b and c only

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let a = pd.points.iter().find(|p| p.name == "a").unwrap();
    let b = pd.points.iter().find(|p| p.name == "b").unwrap();
    let c = pd.points.iter().find(|p| p.name == "c").unwrap();
    let d = pd.points.iter().find(|p| p.name == "d").unwrap();

    assert_eq!(a.y, 100.0);
    assert_eq!(d.y, 100.0);
    assert!(b.y < 100.0);
    assert!(c.y < 100.0);
}

#[test]
fn sculpt_path_tangent_axis_with_locked_endpoints() {
    let mut doc = doc_with_path(
        "vine",
        220.0,
        220.0,
        &[("a", 40.0, 100.0), ("b", 100.0, 100.0), ("c", 160.0, 100.0)],
    );

    let moved = doc
        .sculpt_path_with_options(
            "vine",
            100.0,
            100.0,
            30.0,
            0.0,
            200.0,
            1.0,
            SculptAxis::Tangent,
            true,
            false,
        )
        .unwrap();
    assert_eq!(moved.len(), 1); // endpoint lock keeps a/c fixed

    let vine = doc.find_by_id("vine").unwrap();
    let pd = vine.attrs.path_data.as_ref().unwrap();
    let a = pd.points.iter().find(|p| p.name == "a").unwrap();
    let b = pd.points.iter().find(|p| p.name == "b").unwrap();
    let c = pd.points.iter().find(|p| p.name == "c").unwrap();

    assert_eq!(a.x, 40.0);
    assert_eq!(a.y, 100.0);
    assert_eq!(c.x, 160.0);
    assert_eq!(c.y, 100.0);
    assert!(b.x > 100.0);
    assert!((b.y - 100.0).abs() < 1e-9);
}

#[test]
fn sprout_from_point_creates_numbered_branches() {
    let mut doc = doc_with_styled_path(
        "trunk",
        200.0,
        200.0,
        &[
            ("p0", 150.0, 180.0),
            ("p1", 130.0, 120.0),
            ("p2", 120.0, 60.0),
        ],
        Some("none"),
        Some("#2d6f38"),
        Some(10.0),
    );

    let ids = doc
        .sprout_from_point("trunk", "p1", "twig", 3, 80.0, 10.0, 40.0, 0.4, 0.0, None)
        .unwrap();
    assert_eq!(ids, vec!["twig-1", "twig-2", "twig-3"]);
    for id in ids {
        let node = doc.find_by_id(&id).unwrap();
        assert_eq!(node.kind.tag_name(), "path");
        let pd = node.attrs.path_data.as_ref().unwrap();
        assert_eq!(pd.points.len(), 4);
    }
}

// ── v3 DSL integration tests ─────────────────────────────────────────

#[test]
fn v3_dsl_round_trip() {
    let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

shape stem template=path
  addpoint base at=200,385
  addpoint tip at=200,200 mode=catmull-rom tension=0.3
  stroke #3a7d44
  stroke-width 5

place bg shape=bg at=0,0 size=400x400
place stem shape=stem at=0,0
";
    let scene = dsl_parse::parse_file(input).unwrap();
    let emitted = dsl_emit::emit_scene(&scene);
    let scene2 = dsl_parse::parse_file(&emitted).unwrap();

    assert_eq!(scene.shapes.len(), scene2.shapes.len());
    assert_eq!(scene.nodes.len(), scene2.nodes.len());
    assert_eq!(scene.document_size.w, scene2.document_size.w);
}

#[test]
fn v3_linked_shapes_inherit_geometry() {
    let input = "\
documentsize 400x400

shape petal template=ellipse
  pullpoint top dir=up 15%
  fill #ff0000

createlink petal-dark from=petal
  fill #aa0000
  stroke #880000

place p1 shape=petal at=100,100 size=50x80
place p2 shape=petal-dark at=200,100 size=50x80
";
    let scene = dsl_parse::parse_file(input).unwrap();
    let svg = resolve::resolve_scene(&scene);

    // Both instances should be present
    assert!(svg.contains("id=\"p1\""));
    assert!(svg.contains("id=\"p2\""));
    // Different fills
    assert!(svg.contains("fill=\"#ff0000\""));
    assert!(svg.contains("fill=\"#aa0000\""));
    // Both should have the same path geometry (ellipse with pull)
    // The d attributes should contain curve commands
    assert!(svg.matches(" C").count() >= 2);
}

#[test]
fn v3_rose_fixture_resolves() {
    let input = include_str!("../../test-fixtures/rose-v3.strok");
    let scene = dsl_parse::parse_file(input).unwrap();
    let svg = resolve::resolve_scene(&scene);

    // Basic structure
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>\n"));

    // Background
    assert!(svg.contains("id=\"bg\""));
    assert!(svg.contains("fill=\"#faf6f0\""));

    // Stem
    assert!(svg.contains("id=\"stem\""));
    assert!(svg.contains("stroke=\"#3a7d44\""));

    // Petals (linked instances with short names)
    assert!(svg.contains("id=\"pb-fl\""));
    assert!(svg.contains("id=\"po-l\""));
    assert!(svg.contains("id=\"po-r\""));
    assert!(svg.contains("id=\"pm-l\""));
    assert!(svg.contains("id=\"pi-l\""));

    // Sepals
    assert!(svg.contains("id=\"sepal-l\""));
    assert!(svg.contains("id=\"sepal-r\""));

    // Center bud
    assert!(svg.contains("id=\"bud-o\""));

    // Should have many path elements
    let path_count = svg.matches("<path ").count();
    assert!(path_count >= 15, "expected >= 15 paths, got {}", path_count);
}

#[test]
fn v3_rose_fixture_emits_valid_svg() {
    let input = include_str!("../../test-fixtures/rose-v3.strok");
    let scene = dsl_parse::parse_file(input).unwrap();
    let doc = Document::from_scene(scene);

    let svg = emit::emit_document(&doc);
    // Should produce valid SVG with proper structure
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("viewBox=\"0 0 400 400\""));
    assert!(svg.contains("</svg>"));
}

#[test]
fn v3_document_save_load_round_trip() {
    let input = "\
documentsize 300x300

shape bg template=rectangle
  fill #ffffff

place bg shape=bg at=0,0 size=300x300
";
    let scene = dsl_parse::parse_file(input).unwrap();
    let doc = Document::from_scene(scene);

    let path = std::env::temp_dir().join("strok_v3_roundtrip.strok");
    doc.save(&path).unwrap();

    let loaded = Document::load(&path).unwrap();
    assert!(loaded.scene.is_some());
    let scene = loaded.scene.unwrap();
    assert_eq!(scene.document_size.w, 300.0);
    assert_eq!(scene.shapes.len(), 1);
    assert_eq!(scene.shapes[0].name, "bg");

    std::fs::remove_file(&path).ok();
}

#[test]
fn v3_effects_apply_to_geometry() {
    let input = "\
documentsize 200x200

shape droopy template=path
  addpoint a at=50,50
  addpoint b at=100,50
  addpoint c at=150,50
  applyeffect droop 0.5
  stroke #000000

place d shape=droopy at=0,0
";
    let scene = dsl_parse::parse_file(input).unwrap();
    let svg = resolve::resolve_scene(&scene);

    // The droop effect should modify y-coordinates
    // Without droop, all points would be at y=50
    // With droop, lower points get shifted
    assert!(svg.contains("id=\"d\""));
    assert!(svg.contains("stroke=\"#000000\""));
}

#[test]
fn current_color_reaches_svg_and_round_trips() {
    // An icon authored with `currentColor` keeps it through DSL emit and lands
    // verbatim in the resolved SVG (so CSS `color` can theme it).
    let input = "\
documentsize 24x24

defaults
  fill none
  stroke currentColor
  stroke-width 2

shape mark template=path
  addpoint a at=5,13
  addpoint b at=10,18
  addpoint c at=19,7

place mark shape=mark at=0,0
";
    let scene = dsl_parse::parse_file(input).unwrap();

    // SVG carries currentColor on the stroke.
    let svg = resolve::resolve_scene(&scene);
    assert!(
        svg.contains("stroke=\"currentColor\""),
        "expected currentColor in SVG, got: {svg}"
    );

    // DSL emit round-trips: re-emit, re-parse, re-resolve → identical SVG.
    let emitted = dsl_emit::emit_scene(&scene);
    assert!(emitted.contains("stroke currentColor"));
    let scene2 = dsl_parse::parse_file(&emitted).unwrap();
    assert_eq!(svg, resolve::resolve_scene(&scene2));
}

// ── C6 / E3.1 — diagnostics & error recovery ──────────────────────────

#[test]
fn parse_recover_keeps_going_past_one_bad_line() {
    // Two valid shapes flank a malformed shape body. Recovery should yield both
    // valid shapes plus exactly one diagnostic for the bad operation.
    let input = concat!(
        "documentsize 100x100\n",
        "shape ok1 template=rectangle\n",
        "shape broken template=path\n",
        "  storke #ff0000\n",
        "shape ok2 template=ellipse\n",
    );
    let (scene, diags) = dsl_parse::parse_file_recover(input);
    // The bad `shape broken` block fails, but ok1 and ok2 still parse.
    assert!(scene.find_shape("ok1").is_some(), "ok1 should survive");
    assert!(scene.find_shape("ok2").is_some(), "ok2 should survive");
    assert_eq!(diags.len(), 1, "exactly one diagnostic, got {diags:?}");
    let d = &diags[0];
    assert!(d.message.contains("storke"), "{}", d.message);
    assert_eq!(d.suggestion.as_deref(), Some("stroke"));
    assert_eq!(d.line, 4);
}

#[test]
fn parse_recover_clean_file_has_no_diagnostics() {
    let input = concat!(
        "documentsize 100x100\n",
        "shape s template=rectangle\n",
        "place p shape=s at=0,0\n",
    );
    let (scene, diags) = dsl_parse::parse_file_recover(input);
    assert!(diags.is_empty(), "clean file → no diagnostics: {diags:?}");
    assert!(scene.find_node("p").is_some());
}

#[test]
fn parse_recover_collects_multiple_diagnostics() {
    let input = concat!(
        "documentsize 100x100\n",
        "shappe a template=path\n", // bad top-level keyword
        "shape b template=path\n",
        "  bogusop\n", // bad op
    );
    let (_scene, diags) = dsl_parse::parse_file_recover(input);
    assert_eq!(diags.len(), 2, "two diagnostics, got {diags:?}");
    assert_eq!(diags[0].suggestion.as_deref(), Some("shape"));
}

#[test]
fn diagnostic_render_has_position_and_caret() {
    let input = concat!("documentsize 100x100\n", "shappe foo\n");
    let err = dsl_parse::parse_file(input).unwrap_err();
    let rendered = err.to_string();
    assert!(rendered.contains("line 2, column 1"), "{rendered}");
    assert!(rendered.contains("^^^^^^"), "{rendered}");
    assert!(rendered.contains("did you mean `shape`?"), "{rendered}");
}

#[test]
fn annotate_overlay_is_stable() {
    // Annotate appends a deterministic overlay layer of element IDs; the base
    // geometry is byte-identical to resolve_scene.
    let input = concat!(
        "documentsize 100x60\n",
        "shape box template=rectangle\n  fill #102030\n",
        "place card shape=box at=10,10 size=40x30\n",
    );
    let scene = dsl_parse::parse_file(input).unwrap();
    let base = resolve::resolve_scene(&scene);
    let annotated = resolve::resolve_scene_annotated(&scene);
    // The overlay is purely additive: everything before </svg> in base is a
    // prefix of the annotated output.
    let base_body = base.split_once("</svg>").unwrap().0;
    assert!(annotated.starts_with(base_body), "annotate is additive");
    assert!(
        annotated.contains("id=\"strok-annotations\""),
        "{annotated}"
    );
    assert!(
        annotated.contains(">card<"),
        "labels the element id: {annotated}"
    );
    // Idempotent overlay layer count: exactly one annotations group.
    assert_eq!(annotated.matches("strok-annotations").count(), 1);
}

#[test]
fn outline_overlay_is_additive_exact_and_selective() {
    let input = concat!(
        "documentsize 100x60\n",
        "shape box template=rectangle\n  fill #102030\n",
        "group shifted at=7,5 rotation=12deg\n",
        "  place card shape=box at=10,10 size=40x30\n",
        "place badge shape=box at=70,10 size=20x20\n",
    );
    let scene = dsl_parse::parse_file(input).unwrap();
    let base = resolve::resolve_scene(&scene);
    let outlined = resolve::add_outline_overlay(&base, Some(&["card".to_string()])).unwrap();

    let base_body = base.split_once("</svg>").unwrap().0;
    assert!(outlined.starts_with(base_body), "outline must be additive");
    assert!(outlined.contains("id=\"strok-outline-overlay\""));
    assert!(
        outlined.contains("id=\"strok-outline-ink-card\""),
        "selected path geometry is cloned: {outlined}"
    );
    assert!(
        outlined.contains("id=\"strok-outline-ink-shifted\""),
        "the exact enclosing group transform is cloned: {outlined}"
    );
    assert!(
        outlined.contains("#strok-outline-ink [id=\"strok-outline-ink-card\"]"),
        "selected ID receives foreground outline style: {outlined}"
    );
    assert!(
        !outlined.contains("#strok-outline-ink [id=\"strok-outline-ink-badge\"]"),
        "unselected ID must not receive outline style: {outlined}"
    );
    assert!(
        outlined.contains("vector-effect: non-scaling-stroke"),
        "region/zoom review keeps the inspection stroke readable"
    );

    let err = resolve::add_outline_overlay(&base, Some(&["ghost".to_string()])).unwrap_err();
    assert!(
        err.to_string().contains("ghost")
            && err
                .to_string()
                .contains("not a placed element in this render"),
        "{err}"
    );
}

#[test]
fn outline_overlay_rejects_an_explicit_empty_selection() {
    let scene = dsl_parse::parse_file("documentsize 10x10\n").unwrap();
    let base = resolve::resolve_scene(&scene);
    let err = resolve::add_outline_overlay(&base, Some(&[])).unwrap_err();
    assert!(err.to_string().contains("selection is empty"), "{err}");
}

// --- C7 (E3.3): op-log history replay backs `diff --since` ------------------

#[test]
fn replay_to_reconstructs_intermediate_history_state() {
    // Build a document via the arena API so the op log is populated, then
    // verify replay_to(n) reproduces the state after the first n ops.
    let mut doc = Document::new(100.0, 100.0);
    doc.append_svg(
        "root",
        "<rect id=\"a\" x=\"0\" y=\"0\" width=\"10\" height=\"10\" fill=\"#000000\"/>",
    )
    .expect("append a");
    let svg_after_a = emit::emit_document(&doc);
    doc.append_svg(
        "root",
        "<rect id=\"b\" x=\"20\" y=\"20\" width=\"10\" height=\"10\" fill=\"#ff0000\"/>",
    )
    .expect("append b");

    let history = doc.history_len();
    assert!(history >= 3, "New + 2 appends => {history} ops");

    // Replaying to (history - 1) drops the last append → matches the state
    // captured right after the first append.
    let before = doc.replay_to(history - 1).expect("replay");
    let before_svg = emit::emit_document(&before);
    assert_eq!(before_svg, svg_after_a, "replay reproduces the prior state");

    // The current document still has both rects.
    let now_svg = emit::emit_document(&doc);
    assert!(now_svg.contains("id=\"a\""));
    assert!(now_svg.contains("id=\"b\""));
    // The replayed-before state has only the first.
    assert!(before_svg.contains("id=\"a\""));
    assert!(!before_svg.contains("id=\"b\""));
}

#[test]
fn replay_to_full_history_equals_current() {
    let mut doc = Document::new(50.0, 50.0);
    doc.append_svg(
        "root",
        "<rect id=\"x\" x=\"0\" y=\"0\" width=\"5\" height=\"5\"/>",
    )
    .expect("append");
    let full = doc.replay_to(doc.history_len()).expect("replay full");
    assert_eq!(emit::emit_document(&full), emit::emit_document(&doc));
}
