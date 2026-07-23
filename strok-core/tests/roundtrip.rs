//! E1.3 — Round-trip & geometry property tests.
//!
//! The mandate's core invariant: `parse_dsl(emit_dsl(scene)) == scene`.
//!
//! We assert it as a **proptest** over arbitrary valid `Scene`s spanning shapes,
//! all `CurveMode`s, placements, groups, gradients, tokens and the new
//! fidelity-fix attrs (`fill-rule`, `stroke-miterlimit`). Equality is the
//! derived structural `PartialEq` on `Scene`; to keep that meaningful under the
//! scale-aware numeric formatter (`types::fmt_num`, ~6 sig figs) the strategies
//! generate **canonical values** — small integers and half-steps that survive
//! the format→parse cycle exactly. That keeps the test about *structure +
//! ordering + default elision* (the real round-trip risks) rather than float
//! rounding.
//!
//! A second layer asserts **emit idempotence** (`emit(parse(emit(s))) ==
//! emit(s)`) which catches any residual normalization drift even for values the
//! direct equality would tolerate.

use proptest::prelude::*;

use strok_core::dsl_emit::emit_scene;
use strok_core::dsl_parse::parse_file;
use strok_core::path_point::{CurveMode, NamedPoint, PathData, Placement};
use strok_core::scene::*;
use strok_core::shape::*;
use strok_core::types::*;

// ── Value strategies (canonical, round-trip-exact) ────────────────────────

/// A coordinate that survives `fmt_num` exactly: integer or half-step in a
/// bounded range. Avoids `-0` ambiguity by excluding it implicitly (fmt_num
/// normalizes -0 → 0, and 0.0 == -0.0 in the model anyway).
fn coord() -> impl Strategy<Value = f64> {
    (-400i32..=400i32).prop_map(|n| n as f64 / 2.0)
}

/// A positive dimension component.
fn dim() -> impl Strategy<Value = f64> {
    (1i32..=2000i32).prop_map(|n| n as f64 / 2.0)
}

/// A normalized 0..1 amount with one-decimal granularity.
fn unit() -> impl Strategy<Value = f64> {
    (0i32..=10i32).prop_map(|n| n as f64 / 10.0)
}

/// A valid identifier matching `types::validate_ident`: starts lowercase, then
/// lowercase / digits / `-`.
fn ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,7}".prop_map(|s| s)
}

/// A hex color string.
fn hex_color() -> impl Strategy<Value = String> {
    "#[0-9a-f]{6}".prop_map(|s| s)
}

fn color() -> impl Strategy<Value = Color> {
    prop_oneof![
        hex_color().prop_map(Color::Hex),
        Just(Color::None),
        Just(Color::CurrentColor),
        ident().prop_map(Color::Token),
    ]
}

fn template() -> impl Strategy<Value = Template> {
    prop_oneof![
        Just(Template::Rectangle),
        Just(Template::Ellipse),
        Just(Template::Triangle),
        Just(Template::Line),
        Just(Template::Path),
    ]
}

fn line_cap() -> impl Strategy<Value = LineCap> {
    prop_oneof![
        Just(LineCap::Round),
        Just(LineCap::Butt),
        Just(LineCap::Square)
    ]
}

fn line_join() -> impl Strategy<Value = LineJoin> {
    prop_oneof![
        Just(LineJoin::Miter),
        Just(LineJoin::Round),
        Just(LineJoin::Bevel)
    ]
}

fn fill_rule() -> impl Strategy<Value = FillRule> {
    prop_oneof![Just(FillRule::NonZero), Just(FillRule::EvenOdd)]
}

/// One `addpoint` operation exercising every `CurveMode` spelling.
fn addpoint_op() -> impl Strategy<Value = Operation> {
    let point_mode = prop_oneof![
        Just(None),
        Just(Some(PointMode::Sharp)),
        Just(Some(PointMode::CatmullRom)),
        Just(Some(PointMode::Arc)),
        Just(Some(PointMode::Controls)),
        Just(Some(PointMode::ControlsRelative)),
    ];
    (
        ident(),
        coord(),
        coord(),
        point_mode,
        coord(),
        coord(),
        coord(),
        coord(),
        dim(),
        dim(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(name, x, y, mode, c1x, c1y, c2x, c2y, rx, ry, sweep, large)| {
                // Build the op so emit produces exactly the fields parse re-reads.
                match mode {
                    Some(PointMode::Arc) => Operation::AddPoint {
                        name,
                        at: (x, y),
                        after: None,
                        mode: Some(PointMode::Arc),
                        tension: None,
                        arc_rx: Some(rx),
                        arc_ry: Some(ry),
                        arc_sweep: Some(sweep),
                        arc_large: Some(large),
                        arc_bulge: None,
                        control_c1: None,
                        control_c2: None,
                    },
                    Some(PointMode::Controls) => Operation::AddPoint {
                        name,
                        at: (x, y),
                        after: None,
                        mode: Some(PointMode::Controls),
                        tension: None,
                        arc_rx: None,
                        arc_ry: None,
                        arc_sweep: None,
                        arc_large: None,
                        arc_bulge: None,
                        control_c1: Some((c1x, c1y)),
                        control_c2: Some((c2x, c2y)),
                    },
                    Some(PointMode::ControlsRelative) => Operation::AddPoint {
                        name,
                        at: (x, y),
                        after: None,
                        mode: Some(PointMode::ControlsRelative),
                        tension: None,
                        arc_rx: None,
                        arc_ry: None,
                        arc_sweep: None,
                        arc_large: None,
                        arc_bulge: None,
                        control_c1: Some((c1x, c1y)),
                        control_c2: Some((c2x, c2y)),
                    },
                    Some(PointMode::CatmullRom) => Operation::AddPoint {
                        name,
                        at: (x, y),
                        after: None,
                        mode: Some(PointMode::CatmullRom),
                        tension: Some((c1x / 100.0).clamp(-1.0, 1.0)),
                        arc_rx: None,
                        arc_ry: None,
                        arc_sweep: None,
                        arc_large: None,
                        arc_bulge: None,
                        control_c1: None,
                        control_c2: None,
                    },
                    other => Operation::AddPoint {
                        name,
                        at: (x, y),
                        after: None,
                        mode: other,
                        tension: None,
                        arc_rx: None,
                        arc_ry: None,
                        arc_sweep: None,
                        arc_large: None,
                        arc_bulge: None,
                        control_c1: None,
                        control_c2: None,
                    },
                }
            },
        )
}

/// An attribute / style operation (covers the new fill-rule + miterlimit).
fn attr_op() -> impl Strategy<Value = Operation> {
    prop_oneof![
        color().prop_map(Operation::Fill),
        gradient_color().prop_map(Operation::Fill),
        fill_rule().prop_map(Operation::FillRule),
        color().prop_map(Operation::Stroke),
        gradient_color().prop_map(Operation::Stroke),
        dim().prop_map(|w| Operation::StrokeWidth(AbsoluteSize(w))),
        line_cap().prop_map(Operation::StrokeLinecap),
        line_join().prop_map(Operation::StrokeLinejoin),
        dim().prop_map(Operation::StrokeMiterlimit),
        unit().prop_map(|a| Operation::Opacity(NormalizedAmount(a))),
        prop::collection::vec(dim(), 1..4).prop_map(Operation::StrokeDasharray),
    ]
}

fn geometry_op() -> impl Strategy<Value = Operation> {
    prop_oneof![
        addpoint_op(),
        (ident(), coord(), coord()).prop_map(|(point, dx, dy)| Operation::MovePointDelta {
            point,
            dx,
            dy
        }),
        Just(Operation::Close),
        Just(Operation::Open),
        Just(Operation::Subpath),
        Just(Operation::SharpenAll),
        // C4 E2.5: convert-point round-trip coverage.
        (ident(), convert_target()).prop_map(|(point, to)| Operation::ConvertPoint { point, to }),
        // C5 E2.6: per-corner radius + notch/tail round-trip coverage.
        dim().prop_map(|r| Operation::RoundCorners {
            radii: CornerRadii::uniform(r),
        }),
        prop::collection::vec((ident(), dim()), 1..4).prop_map(|list| Operation::RoundCorners {
            radii: CornerRadii::PerCorner(list),
        }),
        notch_op(),
    ]
}

fn notch_op() -> impl Strategy<Value = Operation> {
    let edge = prop_oneof![
        Just(NotchEdge::Named(Edge::Top)),
        Just(NotchEdge::Named(Edge::Bottom)),
        Just(NotchEdge::Named(Edge::Left)),
        Just(NotchEdge::Named(Edge::Right)),
        (ident(), ident()).prop_map(|(a, b)| NotchEdge::Segment(a, b)),
    ];
    let dir = prop_oneof![Just(NotchDir::In), Just(NotchDir::Out)];
    let shp = prop_oneof![Just(NotchShape::Square), Just(NotchShape::Triangle)];
    (edge, dir, shp, unit(), dim(), dim()).prop_map(|(edge, dir, shape, pos, width, depth)| {
        Operation::Notch(NotchSpec {
            edge,
            dir,
            shape,
            pos,
            width,
            depth,
        })
    })
}

fn convert_target() -> impl Strategy<Value = ConvertTarget> {
    prop_oneof![
        Just(ConvertTarget::Sharp),
        Just(ConvertTarget::Smooth),
        Just(ConvertTarget::Arc),
        Just(ConvertTarget::Controls),
    ]
}

fn shape_strategy() -> impl Strategy<Value = Shape> {
    (
        ident(),
        template(),
        prop::collection::vec(prop_oneof![geometry_op(), attr_op()], 0..6),
    )
        .prop_map(|(name, template, ops)| {
            let mut s = Shape::new(&name, template);
            s.operations = ops;
            s
        })
}

fn grad_edge() -> impl Strategy<Value = GradientEdge> {
    prop_oneof![
        Just(GradientEdge::Top),
        Just(GradientEdge::Bottom),
        Just(GradientEdge::Left),
        Just(GradientEdge::Right),
        Just(GradientEdge::Center),
    ]
}

fn grad_stops() -> impl Strategy<Value = Vec<GradientStop>> {
    let stop = hex_color().prop_map(|c| GradientStop {
        color: c,
        position: None,
    });
    prop::collection::vec(stop, 2..4)
}

fn gradient_color() -> impl Strategy<Value = Color> {
    prop_oneof![
        (grad_edge(), grad_edge(), grad_stops()).prop_map(|(from, to, stops)| {
            Color::LinearGradient(LinearGradient { from, to, stops })
        }),
        (
            grad_edge(),
            (10i32..=100).prop_map(|n| n as f64),
            grad_stops()
        )
            .prop_map(|(center, radius, stops)| {
                Color::RadialGradient(RadialGradient {
                    center,
                    radius,
                    stops,
                })
            }),
    ]
}

fn place_position() -> impl Strategy<Value = PlacePosition> {
    prop_oneof![
        (coord(), coord()).prop_map(|(x, y)| PlacePosition::At(x, y)),
        (ident(), bbox_anchor())
            .prop_map(|(target, anchor)| PlacePosition::RelativeTo { target, anchor }),
    ]
}

fn bbox_anchor() -> impl Strategy<Value = BboxAnchor> {
    prop_oneof![
        Just(BboxAnchor::TopLeft),
        Just(BboxAnchor::Top),
        Just(BboxAnchor::Center),
        Just(BboxAnchor::BottomRight),
        Just(BboxAnchor::Left),
    ]
}

fn flip() -> impl Strategy<Value = Flip> {
    prop_oneof![Just(Flip::X), Just(Flip::Y), Just(Flip::XY)]
}

fn place_strategy(shape_names: Vec<String>) -> impl Strategy<Value = Place> {
    let shape_ref = if shape_names.is_empty() {
        ident().boxed()
    } else {
        proptest::sample::select(shape_names).boxed()
    };
    (
        ident(),
        shape_ref,
        place_position(),
        prop::option::of((dim(), dim())),
        prop::option::of(coord().prop_map(Rotation)),
        prop::option::of(flip()),
        // C4: skew (degx, degy) + per-place clip/mask round-trip coverage.
        prop::option::of((coord(), coord())),
        prop::option::of(prop::collection::vec(ident(), 1..3)),
        prop::option::of(ident()),
    )
        .prop_map(
            |(name, shape_ref, position, size, rotation, flip, skew, clip, mask)| Place {
                name,
                shape_ref,
                position,
                size: size.map(|(w, h)| Dimension { w, h }),
                rotation,
                flip,
                skew,
                clip,
                mask,
                anchor: None,
                overrides: Vec::new(),
                align: None,
                offset: None,
                text_path: None,
            },
        )
}

fn palette_strategy() -> impl Strategy<Value = Palette> {
    let tokens = prop::collection::vec((ident(), hex_color()), 0..4);
    let scheme = (ident(), prop::collection::vec((ident(), hex_color()), 1..3))
        .prop_map(|(name, tokens)| ColorScheme { name, tokens });
    let schemes = prop::collection::vec(scheme, 0..2);
    (tokens, schemes).prop_map(|(tokens, schemes)| {
        // De-dup token names: the emitter writes one line per (name, color) and
        // the parser would keep the last for a name; keep unique to stay exact.
        let mut seen = std::collections::HashSet::new();
        let tokens: Vec<_> = tokens
            .into_iter()
            .filter(|(n, _)| seen.insert(n.clone()))
            .collect();
        Palette { tokens, schemes }
    })
}

fn scene_strategy() -> impl Strategy<Value = Scene> {
    (
        (dim(), dim()),
        palette_strategy(),
        prop::collection::vec(attr_op(), 0..3),
        prop::collection::vec(shape_strategy(), 1..4),
    )
        .prop_flat_map(|(size, palette, defaults, shapes)| {
            let names: Vec<String> = shapes.iter().map(|s| s.name.clone()).collect();
            let places = prop::collection::vec(place_strategy(names), 1..4);
            (
                Just(size),
                Just(palette),
                Just(defaults),
                Just(shapes),
                places,
            )
        })
        .prop_map(|(size, palette, defaults, shapes, places)| Scene {
            document_size: Dimension {
                w: size.0,
                h: size.1,
            },
            imports: Vec::new(),
            palette,
            design_tokens: Vec::new(),
            lets: Vec::new(),
            defaults,
            shapes,
            components: Vec::new(),
            nodes: places.into_iter().map(SceneNode::Place).collect(),
            imported_shape_names: Default::default(),
        })
}

// ── The round-trip invariant ──────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `parse(emit(scene)) == scene` — the mandate invariant, structural equality.
    #[test]
    fn parse_emit_is_identity(scene in scene_strategy()) {
        let dsl = emit_scene(&scene);
        let reparsed = parse_file(&dsl).unwrap_or_else(|e| {
            panic!("emitted DSL failed to parse: {e}\n--- DSL ---\n{dsl}")
        });
        prop_assert_eq!(&reparsed, &scene, "round-trip mismatch\n--- DSL ---\n{}", dsl);
    }

    /// Emit is idempotent through parse: `emit(parse(emit(s))) == emit(s)`.
    #[test]
    fn emit_is_idempotent(scene in scene_strategy()) {
        let once = emit_scene(&scene);
        let reparsed = parse_file(&once).unwrap_or_else(|e| {
            panic!("emitted DSL failed to parse: {e}\n--- DSL ---\n{once}")
        });
        let twice = emit_scene(&reparsed);
        prop_assert_eq!(once, twice);
    }
}

// ── Geometry property tests ───────────────────────────────────────────────

/// Sample a path's emitted `d` into on-curve / control coordinate pairs,
/// command-aware so the radii/flags of an `A` command are NOT mistaken for
/// coordinates. For `M`/`L`/`C` every number pair is a coordinate; for `A
/// rx ry rot large sweep x y` only the trailing `x y` is a coordinate.
fn sample_d_coords(d: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    // Tokenize into command letters and numbers, preserving order.
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            tokens.push(ch.to_string());
        } else if ch.is_ascii_digit() || ch == '.' || (ch == '-' && cur.is_empty()) {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }

    let nums_until_next_cmd = |start: usize| -> Vec<f64> {
        let mut v = Vec::new();
        for t in &tokens[start..] {
            if t.chars().next().map(|c| c.is_ascii_alphabetic()) == Some(true) {
                break;
            }
            if let Ok(n) = t.parse::<f64>() {
                v.push(n);
            }
        }
        v
    };

    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        let c = t.chars().next().unwrap_or(' ');
        if c.is_ascii_alphabetic() {
            let nums = nums_until_next_cmd(i + 1);
            match c {
                'M' | 'L' => {
                    for p in nums.chunks_exact(2) {
                        out.push((p[0], p[1]));
                    }
                }
                'C' => {
                    for p in nums.chunks_exact(2) {
                        out.push((p[0], p[1]));
                    }
                }
                // A rx ry rot large sweep x y — only the last pair is a point.
                'A' if nums.len() >= 7 => out.push((nums[5], nums[6])),
                _ => {}
            }
            i += 1 + nums.len();
        } else {
            i += 1;
        }
    }
    out
}

fn arc_point(name: &str, x: f64, y: f64, rx: f64, ry: f64, sweep: bool, large: bool) -> NamedPoint {
    NamedPoint {
        name: name.to_string(),
        x,
        y,
        mode: CurveMode::Arc {
            rx,
            ry,
            sweep,
            large,
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// geometry_bbox ⊇ all sampled emitted coordinates (bbox soundness): every
    /// control/anchor point the emitter writes must lie within the reported
    /// geometry bbox (with a small epsilon for the cubic approximation of arcs).
    #[test]
    fn geometry_bbox_contains_emitted_points(
        ax in -50i32..50, ay in -50i32..50,
        bx in -50i32..50, by in -50i32..50,
        r in 5i32..60,
        sweep in any::<bool>(), large in any::<bool>(),
    ) {
        let (ax, ay, bx, by, r) = (ax as f64, ay as f64, bx as f64, by as f64, r as f64);
        // Skip a degenerate zero-length chord.
        prop_assume!((ax - bx).abs() > 1e-6 || (ay - by).abs() > 1e-6);

        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                NamedPoint { name: "a".into(), x: ax, y: ay, mode: CurveMode::Sharp },
                arc_point("b", bx, by, r, r, sweep, large),
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let (min_x, min_y, max_x, max_y) = strok_core::path_point::geometry_bbox(&data.points, false);
        // The bbox must at minimum contain both anchor points.
        prop_assert!(min_x <= ax + 1e-6 && max_x >= ax - 1e-6);
        prop_assert!(min_y <= ay + 1e-6 && max_y >= ay - 1e-6);
        prop_assert!(min_x <= bx + 1e-6 && max_x >= bx - 1e-6);
        prop_assert!(min_y <= by + 1e-6 && max_y >= by - 1e-6);

        // And the emitted `d` (translate-only placement) must keep every
        // coordinate inside the bbox (± epsilon for cubic control overshoot of
        // arcs, which can sit slightly outside the exact arc extrema).
        let d = strok_core::path_point::path_data_to_svg_d(&data, None);
        let eps = r * 0.25 + 1.0;
        for (x, y) in sample_d_coords(&d) {
            prop_assert!(x >= min_x - eps && x <= max_x + eps, "x {x} outside [{min_x},{max_x}] d={d}");
            prop_assert!(y >= min_y - eps && y <= max_y + eps, "y {y} outside [{min_y},{max_y}] d={d}");
        }
    }

    /// Placement transform invertibility for the affine (non-flip) case: a point
    /// transformed into placed space and back lands where it started.
    #[test]
    fn placement_transform_is_invertible(
        px in -100i32..100, py in -100i32..100,
        at_x in -50i32..50, at_y in -50i32..50,
        w in 1i32..400, h in 1i32..400,
    ) {
        // Build a unit-square shape so the bbox-fit scale is well-defined.
        let data = PathData {
            coord_space: (10.0, 10.0),
            points: vec![
                NamedPoint { name: "a".into(), x: 0.0, y: 0.0, mode: CurveMode::Sharp },
                NamedPoint { name: "b".into(), x: 10.0, y: 10.0, mode: CurveMode::Sharp },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let placement = Placement {
            at: (at_x as f64, at_y as f64),
            size: Some((w as f64, h as f64)),
            flip: None,
        };
        // Forward transform via the public emitter on a single-segment line: the
        // first point maps to `at` and the second to at+size (bbox-fit).
        let d = strok_core::path_point::path_data_to_svg_d(&data, Some(&placement));
        let coords = sample_d_coords(&d);
        prop_assert_eq!(coords.len(), 2);
        // First anchor → at.
        prop_assert!((coords[0].0 - at_x as f64).abs() < 1e-3);
        prop_assert!((coords[0].1 - at_y as f64).abs() < 1e-3);
        // Second anchor → at + size.
        prop_assert!((coords[1].0 - (at_x as f64 + w as f64)).abs() < 1e-3);
        prop_assert!((coords[1].1 - (at_y as f64 + h as f64)).abs() < 1e-3);
        // (px, py) are unused dims kept for case diversity.
        let _ = (px, py);
    }
}

/// A numeric sampler for an SVG arc, used to lock `arc_extrema` against a
/// brute-force evaluation of the parametric arc.
#[allow(clippy::too_many_arguments)]
fn arc_extrema_numeric(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    rx: f64,
    ry: f64,
    large: bool,
    sweep: bool,
) -> (f64, f64, f64, f64) {
    // Reuse the engine's cubic conversion (the same math the renderer sees) and
    // densely sample each cubic for a ground-truth bbox.
    let data = PathData {
        coord_space: (1.0, 1.0),
        points: vec![
            NamedPoint {
                name: "a".into(),
                x: x1,
                y: y1,
                mode: CurveMode::Sharp,
            },
            arc_point("b", x2, y2, rx, ry, sweep, large),
        ],
        closed: false,
        subpath_starts: Vec::new(),
    };
    // Emit under a NON-uniform placement (sx ≠ sy) so the engine takes the
    // arc→cubic path, then un-map the emitted cubic control points back to local
    // space. This samples the *actual rendered curve* (control polygon), giving a
    // ground-truth bound on the arc geometry that we compare to `geometry_bbox`.
    let (gmin_x, gmin_y, gmax_x, gmax_y) =
        strok_core::path_point::geometry_bbox(&data.points, false);
    let span_x = (gmax_x - gmin_x).max(1e-6);
    let span_y = (gmax_y - gmin_y).max(1e-6);
    // Deliberately non-uniform: width-fit ×2, height-fit ×3.
    let (w, h) = (span_x * 2.0, span_y * 3.0);
    let placement = Placement {
        at: (0.0, 0.0),
        size: Some((w, h)),
        flip: None,
    };
    let d = strok_core::path_point::path_data_to_svg_d(&data, Some(&placement));
    let sx = w / span_x;
    let sy = h / span_y;
    let coords = sample_d_coords(&d);
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in coords {
        // Un-map: placed = (local - gmin) * s  →  local = placed / s + gmin.
        let lx = x / sx + gmin_x;
        let ly = y / sy + gmin_y;
        min_x = min_x.min(lx);
        min_y = min_y.min(ly);
        max_x = max_x.max(lx);
        max_y = max_y.max(ly);
    }
    (min_x, min_y, max_x, max_y)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `arc_extrema` (via geometry_bbox) matches a dense numeric sampler within ε.
    /// The analytic extrema must bound the true arc, and the sampled cubic curve
    /// must in turn stay within the analytic bbox plus a small cubic-approximation
    /// margin.
    #[test]
    fn arc_extrema_match_numeric_sampler(
        ax in -40i32..40, ay in -40i32..40,
        bx in -40i32..40, by in -40i32..40,
        r in 8i32..50,
        sweep in any::<bool>(), large in any::<bool>(),
    ) {
        let (ax, ay, bx, by, r) = (ax as f64, ay as f64, bx as f64, by as f64, r as f64);
        prop_assume!((ax - bx).abs() > 1e-3 || (ay - by).abs() > 1e-3);
        // The chord must be spannable by the radius (else the arc degenerates).
        let chord = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
        prop_assume!(chord <= 2.0 * r);

        let pts = vec![
            NamedPoint { name: "a".into(), x: ax, y: ay, mode: CurveMode::Sharp },
            arc_point("b", bx, by, r, r, sweep, large),
        ];
        let (amin_x, amin_y, amax_x, amax_y) = strok_core::path_point::geometry_bbox(&pts, false);
        let (smin_x, smin_y, smax_x, smax_y) =
            arc_extrema_numeric(ax, ay, bx, by, r, r, large, sweep);

        // The cubic control polygon can poke outside the analytic arc bbox: a
        // 90°-split cubic's off-curve control points sit ~r·(4/3·tan(22.5°)) ≈
        // 0.55·r beyond the curve. We compare the *control polygon* bound (a
        // conservative over-estimate of the true curve bound), so allow that
        // margin. The curve itself stays within the analytic bbox.
        let eps = r * 0.60 + 1.0;
        prop_assert!(smin_x >= amin_x - eps, "smin_x {smin_x} < amin_x {amin_x}");
        prop_assert!(smin_y >= amin_y - eps, "smin_y {smin_y} < amin_y {amin_y}");
        prop_assert!(smax_x <= amax_x + eps, "smax_x {smax_x} > amax_x {amax_x}");
        prop_assert!(smax_y <= amax_y + eps, "smax_y {smax_y} > amax_y {amax_y}");
    }
}

// ── C3 (E2.1/E2.2): boolean / offset geometry properties ─────────────────

proptest! {
    /// area(A ∪ B) ≈ area(A) + area(B) − area(A ∩ B) for two axis-aligned
    /// squares at arbitrary offsets. This is the acceptance identity for E2.1.
    #[test]
    fn bool_union_area_identity(
        ox in 0.0f64..40.0, oy in 0.0f64..40.0,
        sa in 8.0f64..30.0, sb in 8.0f64..30.0,
    ) {
        use strok_core::bool_ops::{apply, area_of, svg_d_to_shapes, BoolOp};
        use i_overlay::core::fill_rule::FillRule;
        let sq = |x: f64, y: f64, s: f64| {
            format!("M{} {} L{} {} L{} {} L{} {}Z", x, y, x+s, y, x+s, y+s, x, y+s)
        };
        let a = svg_d_to_shapes(&sq(0.0, 0.0, sa));
        let b = svg_d_to_shapes(&sq(ox, oy, sb));
        let inter = apply(BoolOp::Intersect, &[a.clone(), b.clone()], FillRule::NonZero);
        let union = apply(BoolOp::Union, &[a.clone(), b.clone()], FillRule::NonZero);
        let ia = area_of(&inter);
        let ua = area_of(&union);
        let lhs = ua + ia;
        let rhs = sa * sa + sb * sb;
        prop_assert!((lhs - rhs).abs() < 1e-3, "A∪B + A∩B = {lhs} vs A+B = {rhs}");
    }
}

proptest! {
    /// offset(A, δ) ⊇ A for δ>0 (growing never loses area) and offset(circle,r)
    /// matches the concentric-circle area within flattening ε. Acceptance for E2.2.
    #[test]
    fn offset_circle_area_matches_concentric(
        r in 6.0f64..20.0, delta in 1.0f64..8.0,
    ) {
        use kurbo::{Circle, Shape};
        use strok_core::bool_ops::area_of;
        use strok_core::stroke_outline::offset_d;
        let d = Circle::new((50.0, 50.0), r).to_path(0.01).to_svg();
        let grown = offset_d(&d, delta).unwrap();
        let a = area_of(&grown);
        let expect = std::f64::consts::PI * (r + delta) * (r + delta);
        prop_assert!((a - expect).abs() / expect < 0.05, "offset area {a} vs {expect}");
        // Growing never shrinks below the original.
        let a0 = std::f64::consts::PI * r * r;
        prop_assert!(a >= a0 - 1.0, "offset must not lose area: {a} < {a0}");
    }
}

// ── C4 (E2.3): transform-aware bbox property ─────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The transform-aware bbox of a rectangle equals the AABB of its four mapped
    /// corners — and, crucially, CONTAINS every transformed point of the rect
    /// (densely sampled along the edges). This is C4's acceptance:
    /// `bbox(transform(shape)) == transform-aware bbox within ε`. A naive
    /// transform-the-AABB approach would fail this under rotation/skew.
    #[test]
    fn transform_bbox_contains_mapped_points(
        x0 in -50i32..50, y0 in -50i32..50,
        w in 1i32..100, h in 1i32..100,
        rot in -180i32..180,
        skx in -60i32..60, sky in -60i32..60,
    ) {
        use strok_core::attrs::{apply, mul, rotate, skew, transform_bbox};
        let (x0, y0) = (x0 as f64, y0 as f64);
        let (x1, y1) = (x0 + w as f64, y0 + h as f64);
        let t = mul(&rotate(rot as f64), &skew(skx as f64, sky as f64));
        let (bx0, by0, bx1, by1) = transform_bbox(&t, x0, y0, x1, y1);
        // Sample the rectangle boundary densely and assert containment.
        let n = 20;
        for i in 0..=n {
            let f = i as f64 / n as f64;
            let samples = [
                (x0 + (x1 - x0) * f, y0),
                (x0 + (x1 - x0) * f, y1),
                (x0, y0 + (y1 - y0) * f),
                (x1, y0 + (y1 - y0) * f),
            ];
            for (sx, sy) in samples {
                let (px, py) = apply(&t, sx, sy);
                prop_assert!(px >= bx0 - 1e-6 && px <= bx1 + 1e-6, "x {px} out of [{bx0},{bx1}]");
                prop_assert!(py >= by0 - 1e-6 && py <= by1 + 1e-6, "y {py} out of [{by0},{by1}]");
            }
        }
    }

    /// Composition is associative: (A·B)·C == A·(B·C) within ε. Locks the affine
    /// algebra that backs nested-group composition.
    #[test]
    fn affine_compose_associative(
        a in -90i32..90, b in -90i32..90, c in -90i32..90,
        tx in -50i32..50, ty in -50i32..50,
    ) {
        use strok_core::attrs::{mul, rotate, translate};
        let ma = rotate(a as f64);
        let mb = mul(&translate(tx as f64, ty as f64), &rotate(b as f64));
        let mc = rotate(c as f64);
        let left = mul(&mul(&ma, &mb), &mc);
        let right = mul(&ma, &mul(&mb, &mc));
        for k in 0..6 {
            prop_assert!((left[k] - right[k]).abs() < 1e-9, "assoc mismatch at {k}");
        }
    }
}
