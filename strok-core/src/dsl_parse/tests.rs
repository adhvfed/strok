
use super::*;

// ── C13: expressions, let, repeat ─────────────────────────────────

#[test]
fn expr_in_place_coords() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

place d shape=dot at=40+2*10,100-5 size=10x10
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert_eq!(p.position, PlacePosition::At(60.0, 95.0));
    } else {
        panic!("expected place");
    }
}

#[test]
fn let_binding_and_reference() {
    let input = "\
documentsize 400x400

let col 310
let inner $col+20

shape dot template=ellipse

place d shape=dot at=$col,$inner size=10x10
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.lets.len(), 2);
    assert_eq!(scene.lets[0], ("col".to_string(), "310".to_string(), 310.0));
    assert_eq!(
        scene.lets[1],
        ("inner".to_string(), "$col+20".to_string(), 330.0)
    );
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert_eq!(p.position, PlacePosition::At(310.0, 330.0));
    } else {
        panic!("expected place");
    }
}

#[test]
fn let_round_trips_via_emit() {
    let input = "\
documentsize 400x400

let col 310
let inner $col+20

shape dot template=ellipse

place d shape=dot at=$col,$inner size=10x10
";
    let scene = parse_file(input).unwrap();
    let out = crate::dsl_emit::emit_scene(&scene);
    assert!(out.contains("let col 310"), "{out}");
    assert!(out.contains("let inner $col+20"), "{out}");
    let scene2 = parse_file(&out).unwrap();
    assert_eq!(scene, scene2);
}

#[test]
fn let_undefined_forward_ref_errors() {
    let input = "\
documentsize 400x400

let a $b+1
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("unknown name '$b'"), "{err}");
}

#[test]
fn let_shadows_token_errors() {
    let input = "\
documentsize 400x400

palette
  hero #e8a840

let hero 10
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("shadows a design token"), "{err}");
}

#[test]
fn repeat_suffixes_names_and_rewrites_refs() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

repeat i 3
  place dot shape=dot center=40+$i*60,40 radius=6
  place lbl shape=dot at=dot.center size=4x4
";
    let scene = parse_file(input).unwrap();
    // 3 iterations × 2 places = 6 nodes, no repeat node survives.
    assert_eq!(scene.nodes.len(), 6);
    let names: Vec<String> = scene
        .nodes
        .iter()
        .filter_map(|n| match n {
            SceneNode::Place(p) => Some(p.name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        vec!["dot-0", "lbl-0", "dot-1", "lbl-1", "dot-2", "lbl-2"]
    );
    // The sibling reference `at=dot.center` was rewritten per iteration.
    if let SceneNode::Place(p) = &scene.nodes[1] {
        match &p.position {
            PlacePosition::RelativeTo { target, .. } => assert_eq!(target, "dot-0"),
            _ => panic!("expected RelativeTo"),
        }
    }
    // Center coord used $i: iteration 2's dot centered at x=160.
    if let SceneNode::Place(p) = &scene.nodes[4] {
        // center=160,40 radius=6 desugars to at=(154,34)
        assert_eq!(p.position, PlacePosition::At(154.0, 34.0));
    }
}

#[test]
fn repeat_count_is_an_expression() {
    let input = "\
documentsize 400x400

let n 2

shape dot template=ellipse

repeat i $n+1
  place dot shape=dot at=$i,0 size=1x1
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.nodes.len(), 3);
}

#[test]
fn repeat_nested_appends_inner_suffix() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

repeat i 2
  repeat j 2
    place dot shape=dot at=$i,$j size=1x1
";
    let scene = parse_file(input).unwrap();
    let names: Vec<String> = scene
        .nodes
        .iter()
        .filter_map(|n| match n {
            SceneNode::Place(p) => Some(p.name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["dot-0-0", "dot-0-1", "dot-1-0", "dot-1-1"]);
}

#[test]
fn repeat_count_too_big_errors() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

repeat i 10001
  place dot shape=dot at=0,0 size=1x1
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("exceeds the maximum"), "{err}");
}

#[test]
fn repeat_var_shadowing_errors() {
    let input = "\
documentsize 400x400

let i 5

shape dot template=ellipse

repeat i 2
  place dot shape=dot at=0,0 size=1x1
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("shadows"), "{err}");
}

#[test]
fn repeat_group_nested_names_suffixed() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

repeat i 2
  group cluster
    place dot shape=dot at=$i,0 size=1x1
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.nodes.len(), 2);
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.name, "cluster-0");
        if let SceneNode::Place(p) = &g.children[0] {
            assert_eq!(p.name, "dot-0");
        } else {
            panic!("expected place child");
        }
    } else {
        panic!("expected group");
    }
}

#[test]
fn repeat_resolves_to_expected_svg_positions() {
    // 4 bars, each 60 apart on Y. resolve_scene must place them accordingly.
    let input = "\
documentsize 640x480

shape bar template=rectangle
  fill #3b82f6

repeat i 4
  place bar shape=bar at=310,190+$i*60 size=20x20
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.nodes.len(), 4);
    let svg = crate::resolve::resolve_scene(&scene);
    // y positions: 190, 250, 310, 370.
    for y in ["190", "250", "310", "370"] {
        assert!(svg.contains(y), "expected y={y} in SVG:\n{svg}");
    }
}

#[test]
fn recover_skips_malformed_let() {
    let input = "\
documentsize 400x400

let a $undefined+1

shape dot template=ellipse

place d shape=dot at=0,0 size=1x1
";
    let (scene, diags) = parse_file_recover(input);
    assert!(!diags.is_empty());
    // The shape and place after the bad let still parsed.
    assert_eq!(scene.shapes.len(), 1);
    assert_eq!(scene.nodes.len(), 1);
}

#[test]
fn parse_minimal_scene() {
    let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

place bg shape=bg at=0,0 size=400x400
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.document_size.w, 400.0);
    assert_eq!(scene.document_size.h, 400.0);
    assert_eq!(scene.shapes.len(), 1);
    assert_eq!(scene.shapes[0].name, "bg");
    assert_eq!(scene.shapes[0].template, Template::Rectangle);
    assert_eq!(scene.nodes.len(), 1);
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert_eq!(p.name, "bg");
        assert_eq!(p.shape_ref, "bg");
    } else {
        panic!("expected Place node");
    }
}

#[test]
fn parse_accepts_agent_natural_identifier_styles() {
    let input = "\
documentsize 24x24

shape brassDeep template=rectangle
shape line_A template=line

place BrandMark shape=brassDeep at=0,0 size=24x24
place detail_1 shape=line_A at=4,12 size=16x0
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.shapes[0].name, "brassDeep");
    assert_eq!(scene.shapes[1].name, "line_A");
    assert!(matches!(&scene.nodes[0], SceneNode::Place(p) if p.name == "BrandMark"));
}

#[test]
fn parse_shape_with_operations() {
    let input = "\
documentsize 400x400

shape stem template=path
  addpoint base at=200,385
  addpoint mid at=192,300 after=base mode=catmull-rom tension=0.3
  addpoint neck at=198,225 after=mid mode=catmull-rom tension=0.3
  stroke #3a7d44
  stroke-width 5
  stroke-linecap round
";
    let scene = parse_file(input).unwrap();
    let stem = &scene.shapes[0];
    assert_eq!(stem.name, "stem");
    assert_eq!(stem.template, Template::Path);
    // 3 addpoints + stroke + stroke-width + stroke-linecap = 6 ops
    assert_eq!(stem.operations.len(), 6);
}

#[test]
fn parse_ellipse_with_pull() {
    let input = "\
documentsize 400x400

shape petal template=ellipse
  pullpoint top dir=up 15%
  pullpoint bottom dir=down 5%
  applyeffect droop 0.15
";
    let scene = parse_file(input).unwrap();
    let petal = &scene.shapes[0];
    assert_eq!(petal.template, Template::Ellipse);
    assert_eq!(petal.operations.len(), 2); // two pullpoints
    assert_eq!(petal.effects.len(), 1); // one effect
}

#[test]
fn parse_palette_and_scheme() {
    let input = "\
documentsize 64x64

palette
  hero #e8a840
  accent #c8863a

scheme dark
  hero #f4c266

shape bg template=rectangle
  fill $accent

place bg shape=bg at=0,0 size=64x64
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.palette.tokens.len(), 2);
    assert_eq!(
        scene.palette.tokens[0],
        ("hero".to_string(), "#e8a840".to_string())
    );
    assert_eq!(scene.palette.schemes.len(), 1);
    assert_eq!(scene.palette.schemes[0].name, "dark");
    assert_eq!(scene.palette.resolve("hero", Some("dark")), Some("#f4c266"));
    // accent falls back to base palette under the dark scheme.
    assert_eq!(
        scene.palette.resolve("accent", Some("dark")),
        Some("#c8863a")
    );
    // token fill parsed as Color::Token.
    assert!(matches!(scene.shapes[0].fill(), Some(Color::Token(t)) if t == "accent"));
}

#[test]
fn parse_invalid_palette_token_errors() {
    let input = "\
documentsize 64x64

palette
  hero notacolor
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("not a valid color"));
}

#[test]
fn parse_createlink() {
    let input = "\
documentsize 400x400

shape petal template=ellipse
  fill #cc4064

createlink petal-back from=petal
  fill #a82848
  stroke #901e3a
  stroke-width 0.5

place petal-back shape=petal-back at=200,150 size=60x100 rotation=-5deg
";
    let scene = parse_file(input).unwrap();
    // Should have petal + petal-back shapes
    assert_eq!(scene.shapes.len(), 2);
    assert_eq!(scene.shapes[1].name, "petal-back");
}

#[test]
fn parse_place_with_on() {
    let input = "\
documentsize 400x400

shape stem template=path
  addpoint base at=200,385

shape leaf template=path
  addpoint tip at=0,0
  close
  fill #4a9950

place leaf-l shape=leaf on=stem.base at=70% side=left offset=15
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        match &p.position {
            PlacePosition::On {
                path,
                t,
                side,
                offset,
            } => {
                assert_eq!(path.shape, "stem");
                assert_eq!(path.point, "base");
                assert_eq!(t.0, 70.0);
                assert_eq!(*side, Some(Side::Left));
                assert_eq!(offset.map(|o| o.0), Some(15.0));
            }
            _ => panic!("expected On position"),
        }
    }
}

#[test]
fn parse_comments_and_blank_lines() {
    let input = "\
# This is the background
documentsize 400x400

# Define bg shape
shape bg template=rectangle
  fill #faf6f0

# Place it
place bg shape=bg at=0,0 size=400x400
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.shapes.len(), 1);
    assert_eq!(scene.nodes.len(), 1);
}

#[test]
fn parse_place_with_flip() {
    let input = "\
documentsize 400x400

shape leaf template=path
  addpoint tip at=0,0
  close

place leaf-r shape=leaf at=100,200 flip=x
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert_eq!(p.flip, Some(Flip::X));
    }
}

#[test]
fn parse_place_with_inline_overrides() {
    let input = "\
documentsize 400x400

shape center template=ellipse

place center-bud shape=center at=200,168 size=14x25
  fill #b83050
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert_eq!(p.name, "center-bud");
        assert_eq!(p.overrides.len(), 1);
    }
}

#[test]
fn parse_place_from_to_desugars_to_at_and_signed_size() {
    let input = "\
documentsize 24x24
shape l template=line
place slit shape=l from=16,8 to=8,16
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Place(p) = &scene.nodes[0] else {
        panic!("expected Place node");
    };
    assert_eq!(p.position, PlacePosition::At(16.0, 8.0));
    let s = p.size.unwrap();
    assert_eq!((s.w, s.h), (-8.0, 8.0));
}

#[test]
fn parse_place_from_to_axis_aligned_gives_zero_extent() {
    let input = "\
documentsize 24x24
shape l template=line
place v shape=l from=4,4 to=4,12
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Place(p) = &scene.nodes[0] else {
        panic!("expected Place node");
    };
    let s = p.size.unwrap();
    assert_eq!((s.w, s.h), (0.0, 8.0));
}

#[test]
fn parse_place_from_without_to_errors() {
    let input = "\
documentsize 24x24
shape l template=line
place v shape=l from=4,4
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("from= and to= must be used together"), "{err}");
}

#[test]
fn parse_place_from_to_conflicts_with_size() {
    let input = "\
documentsize 24x24
shape l template=line
place v shape=l from=4,4 to=8,8 size=4x4
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("cannot be combined with size="), "{err}");
}

#[test]
fn parse_place_center_radius_desugars() {
    let input = "\
documentsize 24x24
shape c template=ellipse
place hub shape=c center=12,12 radius=5
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Place(p) = &scene.nodes[0] else {
        panic!("expected Place node");
    };
    assert_eq!(p.position, PlacePosition::At(7.0, 7.0));
    let s = p.size.unwrap();
    assert_eq!((s.w, s.h), (10.0, 10.0));
}

#[test]
fn parse_place_center_with_size() {
    let input = "\
documentsize 24x24
shape c template=ellipse
place hub shape=c center=12,12 size=8x4
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Place(p) = &scene.nodes[0] else {
        panic!("expected Place node");
    };
    assert_eq!(p.position, PlacePosition::At(8.0, 10.0));
    let s = p.size.unwrap();
    assert_eq!((s.w, s.h), (8.0, 4.0));
}

#[test]
fn parse_place_radius_elliptical_with_at() {
    let input = "\
documentsize 24x24
shape c template=ellipse
place e shape=c at=2,2 radius=6,4
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Place(p) = &scene.nodes[0] else {
        panic!("expected Place node");
    };
    assert_eq!(p.position, PlacePosition::At(2.0, 2.0));
    let s = p.size.unwrap();
    assert_eq!((s.w, s.h), (12.0, 8.0));
}

#[test]
fn parse_place_center_without_extent_errors() {
    let input = "\
documentsize 24x24
shape c template=ellipse
place hub shape=c center=12,12
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("center= needs size= or radius="), "{err}");
}

#[test]
fn parse_place_unknown_key_errors_with_suggestion() {
    let input = "\
documentsize 24x24
shape l template=line
place v shape=l att=4,4 size=8x8
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("unknown place attribute 'att='"), "{err}");
    assert!(err.contains("at"), "{err}");
}

#[test]
fn parse_place_from_to_round_trips_via_at_size() {
    // Sugar desugars at parse time: emit writes plain at/size, and
    // re-parsing that yields the same scene (the round-trip invariant).
    let input = "\
documentsize 24x24
shape l template=line
place slit shape=l from=16,8 to=8,16
";
    let scene = parse_file(input).unwrap();
    let emitted = crate::dsl_emit::emit_scene(&scene);
    assert!(emitted.contains("at=16,8"), "{emitted}");
    assert!(emitted.contains("size=-8x8"), "{emitted}");
    let scene2 = parse_file(&emitted).unwrap();
    assert_eq!(scene, scene2);
}

#[test]
fn parse_group() {
    let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

group bloom
  place petal-1 shape=bg at=100,100
  place petal-2 shape=bg at=200,200
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.name, "bloom");
        assert_eq!(g.children.len(), 2);
    }
}

#[test]
fn parse_place_with_relative_to() {
    let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

shape dot template=ellipse
  fill #0000ff

place box shape=box at=100,100 size=200x100
place dot shape=dot at=box.tr size=20x20
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[1] {
        assert_eq!(p.name, "dot");
        match &p.position {
            PlacePosition::RelativeTo { target, anchor } => {
                assert_eq!(target, "box");
                assert_eq!(*anchor, BboxAnchor::TopRight);
            }
            _ => panic!("expected RelativeTo position"),
        }
    } else {
        panic!("expected Place node");
    }
}

#[test]
fn parse_place_with_align() {
    let input = "\
documentsize 800x600

shape box template=rectangle

place a shape=box at=100,100 size=200x100
place b shape=box at=a.top align=center size=50x50
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[1] {
        assert_eq!(p.align, Some(BboxAnchor::Center));
    }
}

#[test]
fn parse_place_with_offset() {
    let input = "\
documentsize 800x600

shape box template=rectangle

place a shape=box at=100,100 size=200x100
place b shape=box at=a.right align=left offset=5,-3 size=50x50
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[1] {
        assert_eq!(p.offset, Some((5.0, -3.0)));
        assert_eq!(p.align, Some(BboxAnchor::Left));
    }
}

#[test]
fn parse_align_with_absolute_at() {
    let input = "\
documentsize 800x600

shape box template=rectangle

place a shape=box at=400,50 align=top size=200x30
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert!(matches!(p.position, PlacePosition::At(400.0, 50.0)));
        assert_eq!(p.align, Some(BboxAnchor::Top));
    }
}

#[test]
fn parse_radial_gradient_fill() {
    let input = "\
documentsize 400x400

shape glow template=ellipse
  fill radial(center, 80%, #ff6b6b, transparent)

place glow shape=glow at=100,100 size=200x200
";
    let scene = parse_file(input).unwrap();
    let fill = scene.shapes[0].fill().unwrap();
    match fill {
        Color::RadialGradient(g) => {
            assert_eq!(g.center, GradientEdge::Center);
            assert_eq!(g.radius, 80.0);
            assert_eq!(g.stops.len(), 2);
            assert_eq!(g.stops[0].color, "#ff6b6b");
            assert_eq!(g.stops[1].color, "transparent");
        }
        _ => panic!("expected RadialGradient, got {:?}", fill),
    }
}

#[test]
fn parse_linear_gradient_fill() {
    let input = "\
documentsize 400x400

shape sky template=rectangle
  fill linear(top, bottom, #ff0000, #0000ff)

place sky shape=sky at=0,0 size=400x400
";
    let scene = parse_file(input).unwrap();
    let fill = scene.shapes[0].fill().unwrap();
    match fill {
        Color::LinearGradient(g) => {
            assert_eq!(g.from, GradientEdge::Top);
            assert_eq!(g.to, GradientEdge::Bottom);
            assert_eq!(g.stops.len(), 2);
            assert_eq!(g.stops[0].color, "#ff0000");
            assert_eq!(g.stops[1].color, "#0000ff");
        }
        _ => panic!("expected LinearGradient, got {:?}", fill),
    }
}

#[test]
fn parse_multistop_gradient() {
    let input = "\
documentsize 400x400

shape bar template=rectangle
  fill radial(center, 50%, #d8b480 0%, #c4a070 60%, transparent 100%)

place bar shape=bar at=0,0 size=400x400
";
    let scene = parse_file(input).unwrap();
    let fill = scene.shapes[0].fill().unwrap();
    match fill {
        Color::RadialGradient(g) => {
            assert_eq!(g.stops.len(), 3);
            assert_eq!(g.stops[0].position, Some(0.0));
            assert_eq!(g.stops[1].position, Some(0.6));
            assert_eq!(g.stops[2].position, Some(1.0));
        }
        _ => panic!("expected RadialGradient"),
    }
}

#[test]
fn parse_gradient_stroke() {
    let input = "\
documentsize 400x400

shape ring template=ellipse
  fill none
  stroke linear(left, right, #ff0000, #0000ff)
  stroke-width 3

place ring shape=ring at=100,100 size=200x200
";
    let scene = parse_file(input).unwrap();
    let stroke = scene.shapes[0].stroke().unwrap();
    assert!(matches!(stroke, Color::LinearGradient(_)));
}

#[test]
fn parse_invalid_anchor_errors() {
    let input = "\
documentsize 800x600

shape box template=rectangle
place a shape=box at=body.bogus
";
    let result = parse_file(input);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("not a valid anchor"));
}

#[test]
fn parse_invalid_align_errors() {
    let input = "\
documentsize 800x600

shape box template=rectangle
place a shape=box at=0,0 align=bogus
";
    let result = parse_file(input);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("not a valid anchor"));
}

// ── Blur tests ────────────────────────────────────────────────────

#[test]
fn parse_blur_in_shape() {
    let input = "\
documentsize 400x400

shape shadow template=ellipse
  fill #000000
  opacity 0.3
  blur 5
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.shapes[0].blur(), Some(5.0));
}

#[test]
fn parse_blur_as_place_override() {
    let input = "\
documentsize 400x400

shape glow template=ellipse
  fill #ffff00

place glow shape=glow at=100,100 size=50x50
  blur 8
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert!(p
            .overrides
            .iter()
            .any(|op| matches!(op, Operation::Blur(r) if (*r - 8.0).abs() < 0.001)));
    } else {
        panic!("expected Place");
    }
}

// ── Group clip and opacity tests ──────────────────────────────────

#[test]
fn parse_group_with_clip() {
    let input = "\
documentsize 400x400

shape eye-white template=ellipse
  fill #ffffff

shape iris template=ellipse
  fill #4488cc

group eyes clip=eye-white
  place iris shape=iris at=120,85 size=20x20
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.clip, Some(vec!["eye-white".to_string()]));
        assert_eq!(g.children.len(), 1);
    } else {
        panic!("expected Group");
    }
}

#[test]
fn parse_group_with_opacity() {
    let input = "\
documentsize 400x400

shape dot template=ellipse

group head opacity=0.5
  place dot shape=dot at=0,0 size=10x10
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.opacity, Some(0.5));
    } else {
        panic!("expected Group");
    }
}

#[test]
fn parse_group_with_clip_and_opacity() {
    let input = "\
documentsize 400x400

shape mask template=ellipse
shape dot template=ellipse

group eye clip=mask opacity=0.7
  place dot shape=dot at=0,0
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.clip, Some(vec!["mask".to_string()]));
        assert_eq!(g.opacity, Some(0.7));
    } else {
        panic!("expected Group");
    }
}

// ── Group transform tests ────────────────────────────────────────

#[test]
fn parse_group_with_position() {
    let input = "\
documentsize 800x600

shape dot template=ellipse

group dial at=110,130
  place dot shape=dot at=0,0 size=10x10
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.position, Some((110.0, 130.0)));
        assert_eq!(g.rotation, None);
        assert_eq!(g.flip, None);
    } else {
        panic!("expected Group");
    }
}

#[test]
fn parse_group_with_all_transforms() {
    let input = "\
documentsize 800x600

shape ring template=ellipse

group compass at=790,130 rotation=15deg flip=x
  place ring shape=ring at=0,0 size=200x200
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.position, Some((790.0, 130.0)));
        assert_eq!(g.rotation, Some(Rotation(15.0)));
        assert_eq!(g.flip, Some(Flip::X));
    } else {
        panic!("expected Group");
    }
}

#[test]
fn parse_group_with_transforms_and_clip() {
    let input = "\
documentsize 800x600

shape mask template=ellipse
shape dot template=ellipse

group dial at=100,50 rotation=45deg clip=mask opacity=0.8
  place dot shape=dot at=0,0 size=10x10
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Group(g) = &scene.nodes[0] {
        assert_eq!(g.position, Some((100.0, 50.0)));
        assert_eq!(g.rotation, Some(Rotation(45.0)));
        assert_eq!(g.clip, Some(vec!["mask".to_string()]));
        assert_eq!(g.opacity, Some(0.8));
    } else {
        panic!("expected Group");
    }
}

// ── Use/import tests ──────────────────────────────────────────────

#[test]
fn parse_use_flat() {
    let input = "\
documentsize 400x400

use \"./eye.strok\"

shape bg template=rectangle
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.imports.len(), 1);
    assert_eq!(scene.imports[0].path, "./eye.strok");
    assert_eq!(scene.imports[0].namespace, None);
}

#[test]
fn parse_use_namespaced() {
    let input = "\
documentsize 400x400

use \"./face.strok\" as face

shape bg template=rectangle
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.imports.len(), 1);
    assert_eq!(scene.imports[0].path, "./face.strok");
    assert_eq!(scene.imports[0].namespace, Some("face".to_string()));
}

// ── Standard library import tests (EXP-1) ──────────────────────────

#[test]
fn std_import_resolves_shapes_into_scene() {
    let dir = std::env::temp_dir();
    let path = dir.join("strok_std_import_test.strok");
    let input = "\
documentsize 400x400

use \"std/figures\" as fig

place p shape=fig.person-standing at=0,0 size=40x100
";
    std::fs::write(&path, input).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let scene = parse_file_with_path(&text, &path).unwrap();
    std::fs::remove_file(&path).ok();

    assert!(scene.shapes.iter().any(|s| s.name == "fig.person-standing"));
    assert!(scene.shapes.iter().any(|s| s.name == "fig.person-pointing"));
}

#[test]
fn std_import_without_namespace_uses_bare_shape_names() {
    let dir = std::env::temp_dir();
    let path = dir.join("strok_std_import_bare_test.strok");
    let input = "\
documentsize 400x400

use \"std/arrows\"

place p shape=arrow-right at=0,0 size=60x24
";
    std::fs::write(&path, input).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let scene = parse_file_with_path(&text, &path).unwrap();
    std::fs::remove_file(&path).ok();

    assert!(scene.shapes.iter().any(|s| s.name == "arrow-right"));
}

#[test]
fn std_import_unknown_module_is_positioned_error() {
    let dir = std::env::temp_dir();
    let path = dir.join("strok_std_import_unknown_test.strok");
    let input = "\
documentsize 400x400

use \"std/nonexistent\" as x
";
    std::fs::write(&path, input).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let err = parse_file_with_path(&text, &path).unwrap_err();
    std::fs::remove_file(&path).ok();

    let msg = err.to_string();
    assert!(msg.contains("std/nonexistent"), "{}", msg);
    assert!(msg.contains("figures"), "{}", msg); // lists available modules
}

#[test]
fn std_import_with_strok_suffix_also_resolves() {
    let dir = std::env::temp_dir();
    let path = dir.join("strok_std_import_suffix_test.strok");
    let input = "\
documentsize 400x400

use \"std/arrows.strok\" as arr

place p shape=arr.chevron at=0,0 size=24x24
";
    std::fs::write(&path, input).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let scene = parse_file_with_path(&text, &path).unwrap();
    std::fs::remove_file(&path).ok();

    assert!(scene.shapes.iter().any(|s| s.name == "arr.chevron"));
}

// ── Defaults tests ───────────────────────────────────────────────

#[test]
fn parse_defaults_block() {
    let input = "\
documentsize 400x400

defaults
  fill #2d5a1e
  stroke #1a3a12
  stroke-width 1.5

shape leaf template=ellipse

place leaf shape=leaf at=50,50 size=40x60
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.defaults.len(), 3);
    assert!(matches!(&scene.defaults[0], Operation::Fill(Color::Hex(c)) if c == "#2d5a1e"));
    assert!(matches!(&scene.defaults[1], Operation::Stroke(Color::Hex(c)) if c == "#1a3a12"));
    assert!(
        matches!(&scene.defaults[2], Operation::StrokeWidth(AbsoluteSize(w)) if (*w - 1.5).abs() < 0.001)
    );
}

#[test]
fn parse_defaults_rejects_geometry_ops() {
    let input = "\
documentsize 400x400

defaults
  addpoint a at=0,0
";
    let result = parse_file(input);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("defaults block"));
}

// ── Dashed stroke tests ──────────────────────────────────────────

#[test]
fn parse_stroke_dasharray_in_shape() {
    let input = "\
documentsize 400x400

shape border template=rectangle
  stroke #333333
  stroke-width 2
  stroke-dasharray 5 3
";
    let scene = parse_file(input).unwrap();
    assert_eq!(scene.shapes[0].stroke_dasharray(), Some(&[5.0, 3.0][..]));
}

#[test]
fn parse_stroke_dasharray_complex() {
    let input = "\
documentsize 400x400

shape border template=rectangle
  stroke-dasharray 10 5 2 5
";
    let scene = parse_file(input).unwrap();
    assert_eq!(
        scene.shapes[0].stroke_dasharray(),
        Some(&[10.0, 5.0, 2.0, 5.0][..])
    );
}

#[test]
fn parse_stroke_dasharray_as_place_override() {
    let input = "\
documentsize 400x400

shape box template=rectangle

place box shape=box at=0,0 size=100x100
  stroke-dasharray 8 4
";
    let scene = parse_file(input).unwrap();
    if let SceneNode::Place(p) = &scene.nodes[0] {
        assert!(p
            .overrides
            .iter()
            .any(|op| matches!(op, Operation::StrokeDasharray(v) if v == &[8.0, 4.0])));
    } else {
        panic!("expected Place");
    }
}

// ── Arc segment tests ────────────────────────────────────────────

#[test]
fn parse_addpoint_arc_basic() {
    let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,0
  addpoint end at=100,50 mode=arc rx=30 ry=20
";
    let scene = parse_file(input).unwrap();
    let ops = &scene.shapes[0].operations;
    assert_eq!(ops.len(), 2);
    if let Operation::AddPoint {
        mode,
        arc_rx,
        arc_ry,
        arc_sweep,
        arc_large,
        ..
    } = &ops[1]
    {
        assert_eq!(*mode, Some(PointMode::Arc));
        assert_eq!(*arc_rx, Some(30.0));
        assert_eq!(*arc_ry, Some(20.0));
        assert_eq!(*arc_sweep, None); // defaults
        assert_eq!(*arc_large, None); // defaults
    } else {
        panic!("expected AddPoint");
    }
}

#[test]
fn parse_addpoint_arc_with_flags() {
    let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,0
  addpoint end at=200,100 mode=arc rx=40 ry=40 sweep=0 large=1
";
    let scene = parse_file(input).unwrap();
    if let Operation::AddPoint {
        arc_sweep,
        arc_large,
        ..
    } = &scene.shapes[0].operations[1]
    {
        assert_eq!(*arc_sweep, Some(false));
        assert_eq!(*arc_large, Some(true));
    } else {
        panic!("expected AddPoint");
    }
}

#[test]
fn parse_addpoint_arc_bulge_and_sweep_synonyms() {
    let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,0
  addpoint a at=100,0 mode=arc rx=40 bulge=left
  addpoint b at=200,0 mode=arc rx=40 sweep=cw
  addpoint c at=300,0 mode=arc rx=40 sweep=ccw
";
    let scene = parse_file(input).unwrap();
    let ops = &scene.shapes[0].operations;
    if let Operation::AddPoint {
        arc_bulge,
        arc_sweep,
        ..
    } = &ops[1]
    {
        assert_eq!(*arc_bulge, Some(ArcBulge::Left));
        assert_eq!(*arc_sweep, None);
    } else {
        panic!("expected AddPoint a");
    }
    if let Operation::AddPoint {
        arc_sweep,
        arc_bulge,
        ..
    } = &ops[2]
    {
        assert_eq!(*arc_sweep, Some(true)); // cw → 1
        assert_eq!(*arc_bulge, None);
    } else {
        panic!("expected AddPoint b");
    }
    if let Operation::AddPoint { arc_sweep, .. } = &ops[3] {
        assert_eq!(*arc_sweep, Some(false)); // ccw → 0
    } else {
        panic!("expected AddPoint c");
    }
}

#[test]
fn parse_addpoint_arc_bulge_invalid() {
    let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,0
  addpoint end at=100,0 mode=arc rx=40 bulge=sideways
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("bulge must be left or right"), "got: {err}");
}

#[test]
fn parse_addpoint_controls_absolute() {
    let input = "\
documentsize 400x400

shape curve template=path
  addpoint start at=0,0
  addpoint end at=100,100 mode=controls c1=25,0 c2=100,75
";
    let scene = parse_file(input).unwrap();
    if let Operation::AddPoint {
        mode,
        control_c1,
        control_c2,
        ..
    } = &scene.shapes[0].operations[1]
    {
        assert_eq!(*mode, Some(PointMode::Controls));
        assert_eq!(*control_c1, Some((25.0, 0.0)));
        assert_eq!(*control_c2, Some((100.0, 75.0)));
    } else {
        panic!("expected AddPoint");
    }
}

#[test]
fn parse_addpoint_controls_require_handles() {
    let input = "\
documentsize 400x400

shape curve template=path
  addpoint start at=0,0
  addpoint end at=100,100 mode=controls c1=25,0
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("controls modes require both c1=x,y and c2=x,y"));
}

#[test]
fn parse_live_boolean_with_named_operands_and_style() {
    let input = "\
documentsize 100x100

shape block template=rectangle

boolean silhouette op=union
  place head shape=block at=10,10 size=30x30
  place neck shape=block at=25,30 size=20x40 rotation=8
  fill #f7f3ea
  stroke #332f29
  stroke-width 1.5
";
    let scene = parse_file(input).unwrap();
    let SceneNode::Boolean(boolean) = &scene.nodes[0] else {
        panic!("expected live boolean");
    };
    assert_eq!(boolean.name, "silhouette");
    assert_eq!(boolean.op, crate::bool_ops::BoolOp::Union);
    assert_eq!(boolean.children.len(), 2);
    assert_eq!(boolean.operations.len(), 3);
    let SceneNode::Place(neck) = &boolean.children[1] else {
        panic!("expected named place operand");
    };
    assert_eq!(neck.name, "neck");
    assert_eq!(neck.rotation, Some(crate::types::Rotation(8.0)));
}

#[test]
fn live_boolean_requires_two_operands() {
    let input = "\
documentsize 100x100

shape block template=rectangle

boolean silhouette op=union
  place head shape=block at=10,10 size=30x30
";
    let err = parse_file(input).unwrap_err().to_string();
    assert!(err.contains("at least two placed operands"), "{err}");
}
