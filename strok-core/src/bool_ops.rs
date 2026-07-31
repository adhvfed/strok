//! Vector boolean operations: union, subtract, intersect, and exclude.
//!
//! `i_overlay` provides the robust f64 polygon-boolean core; `kurbo` flattens
//! Bézier paths for boolean processing.
//!
//! Pipeline: each input shape is resolved + placed into **document space**, its
//! `d`-geometry flattened to closed polygon contours, the boolean applied with
//! the author's fill-rule, and the multi-contour result rebuilt as a `path`
//! `Shape` (sharp `addpoint`s + `subpath` breaks) authored in document
//! coordinates and placed `at=0,0` (identity). Because the result is an ordinary
//! path shape it round-trips, renders, and re-edits like hand-authored geometry.

use crate::error::{Result, StrokError};
use crate::path_point::{CurveMode, NamedPoint, PathData};
use crate::shape::{Operation, Shape, Template};

use i_overlay::core::fill_rule::FillRule as IFillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

/// The four boolean ops. `Subtract`/`Intersect` fold left-to-right over the
/// operand list; `Union`/`Exclude` are commutative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
    /// Symmetric difference (XOR).
    Exclude,
}

impl BoolOp {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "union" => Ok(BoolOp::Union),
            "subtract" => Ok(BoolOp::Subtract),
            "intersect" => Ok(BoolOp::Intersect),
            "exclude" => Ok(BoolOp::Exclude),
            _ => Err(StrokError::InvalidOperation(format!(
                "unknown boolean op '{}' — valid: union, subtract, intersect, exclude",
                s
            ))),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            BoolOp::Union => "union",
            BoolOp::Subtract => "subtract",
            BoolOp::Intersect => "intersect",
            BoolOp::Exclude => "exclude",
        }
    }

    fn overlay_rule(&self) -> OverlayRule {
        match self {
            BoolOp::Union => OverlayRule::Union,
            BoolOp::Subtract => OverlayRule::Difference,
            BoolOp::Intersect => OverlayRule::Intersect,
            BoolOp::Exclude => OverlayRule::Xor,
        }
    }
}

/// A flattened 2D point in document space.
type Pt = [f64; 2];
/// A polygon contour (no implicit closing point — i_overlay closes it).
type Contour = Vec<Pt>;
/// i_overlay shapes: a list of `[outer, hole, hole, …]` groups.
type Shapes = Vec<Vec<Contour>>;

/// Flattening tolerance in document units. Boolean results are emitted as
/// polygons, so this must also hold up when a small icon is enlarged for visual
/// review. At 0.01, a 45-unit asset shown at 1200 px stays below roughly a third
/// of a display pixel while keeping compound paths reasonably compact.
const FLATTEN_TOL: f64 = 0.01;

/// Apply `op` over `operands` (already flattened to contour-sets, with each
/// operand interpreted under `fill_rule`). The first operand is the subject;
/// the rest are folded in. Returns the result contour-set (outer-then-holes per
/// shape). Never panics — i_overlay handles coincident edges / self-intersection
/// / holes; an empty result is valid (e.g. fully-subtracted geometry).
pub fn apply(op: BoolOp, operands: &[Shapes], fill_rule: IFillRule) -> Shapes {
    if operands.is_empty() {
        return Vec::new();
    }
    let mut acc: Shapes = operands[0].clone();
    let rule = op.overlay_rule();
    for clip in &operands[1..] {
        let subj: Vec<Contour> = flatten_shapes(&acc);
        let clp: Vec<Contour> = flatten_shapes(clip);
        acc = subj.overlay(&clp, rule, fill_rule);
    }
    if operands.len() == 1 {
        // A single operand: normalize it under its own fill-rule (resolves
        // self-intersections / holes into clean outer+hole contours).
        let subj: Vec<Contour> = flatten_shapes(&acc);
        let empty: Vec<Contour> = Vec::new();
        acc = subj.overlay(&empty, OverlayRule::Subject, fill_rule);
    }
    acc
}

/// High-level entry for the CLI: combine `operands` (each an SVG `d` string in
/// document space + its own fill-rule) with `op`, returning the result as a
/// `path` `Shape` named `out_name`. Keeps the `i_overlay` types entirely inside
/// `strok-core` so the CLI needs no geometry dependency. Returns a shape with no
/// operations when the result is empty (caller decides how to report that).
pub fn combine(
    op: BoolOp,
    operands: &[(String, Option<crate::types::FillRule>)],
    out_name: &str,
) -> Shape {
    let shapes: Vec<Shapes> = operands
        .iter()
        .map(|(d, rule)| {
            // Normalize each operand under its own fill-rule first, so a holey /
            // self-intersecting input is interpreted as the author intended
            // before being combined with the others (which use non-zero).
            let raw = svg_d_to_shapes(d);
            apply(BoolOp::Union, &[raw], ifill_rule(*rule))
        })
        .collect();
    let result = apply(op, &shapes, IFillRule::NonZero);
    shapes_to_shape(out_name, &result, IFillRule::NonZero)
}

/// Apply an affine transform to an SVG path string. Boolean operands use this
/// to bake place rotation/skew into their document-space geometry so live and
/// destructive booleans see exactly what the renderer shows.
pub fn transform_svg_d(d: &str, transform: &crate::attrs::Transform) -> Option<String> {
    use kurbo::{BezPath, PathEl, Point};
    let source = BezPath::from_svg(d).ok()?;
    let map = |p: Point| {
        let (x, y) = crate::attrs::apply(transform, p.x, p.y);
        Point::new(x, y)
    };
    let mut out = BezPath::new();
    for element in source.iter() {
        match element {
            PathEl::MoveTo(p) => out.move_to(map(p)),
            PathEl::LineTo(p) => out.line_to(map(p)),
            PathEl::QuadTo(p1, p2) => out.quad_to(map(p1), map(p2)),
            PathEl::CurveTo(p1, p2, p3) => out.curve_to(map(p1), map(p2), map(p3)),
            PathEl::ClosePath => out.close_path(),
        }
    }
    Some(out.to_svg())
}

/// Flatten an i_overlay shape-set (`[[outer, hole…], …]`) into a flat contour
/// list for feeding back into a subsequent overlay step.
fn flatten_shapes(shapes: &Shapes) -> Vec<Contour> {
    shapes.iter().flatten().cloned().collect()
}

/// Map our `FillRule` to i_overlay's.
pub fn ifill_rule(rule: Option<crate::types::FillRule>) -> IFillRule {
    match rule {
        Some(crate::types::FillRule::EvenOdd) => IFillRule::EvenOdd,
        _ => IFillRule::NonZero,
    }
}

/// Flatten a placed `PathData` (already transformed into document space; pass the
/// path data whose points are document coordinates) into closed polygon contours
/// suitable for i_overlay. Honors subpaths and all curve modes by delegating to
/// the shared `path_data_to_svg_d` → kurbo parse path, so the boolean sees the
/// SAME geometry the renderer does (no divergence).
pub fn pathdata_to_shapes(pd: &PathData) -> Shapes {
    let d = crate::path_point::path_data_to_svg_d(pd, None);
    svg_d_to_shapes(&d)
}

/// Parse an SVG `d` string into flattened closed contours via kurbo. Each `M`
/// starts a new contour; curves flatten at `FLATTEN_TOL`. This is the bridge
/// that keeps boolean geometry identical to the rendered geometry.
pub fn svg_d_to_shapes(d: &str) -> Shapes {
    use kurbo::{BezPath, PathEl};
    let bez = match BezPath::from_svg(d) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let mut contours: Vec<Contour> = Vec::new();
    let mut cur: Contour = Vec::new();
    kurbo::flatten(bez.iter(), FLATTEN_TOL, |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() >= 3 {
                contours.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur.push([p.x, p.y]);
        }
        PathEl::LineTo(p) => cur.push([p.x, p.y]),
        PathEl::ClosePath => {
            if cur.len() >= 3 {
                contours.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
        _ => {}
    });
    if cur.len() >= 3 {
        contours.push(cur);
    }
    // i_overlay treats every input contour as a closed polygon; group them all as
    // one subject with nesting resolved by the fill-rule at overlay time.
    if contours.is_empty() {
        Vec::new()
    } else {
        vec![contours]
    }
}

/// Build a `path` `Shape` (document-coordinate, sharp points + `subpath` breaks)
/// from a boolean/offset/outline result contour-set. `name` is the new shape id.
/// Drops degenerate (<3-point) contours. The result is authored in document
/// space, so the caller places it `at=0,0` with no size (identity transform).
pub fn shapes_to_shape(name: &str, shapes: &Shapes, fill_rule: IFillRule) -> Shape {
    let mut shape = Shape::new(name, Template::Path);
    let mut first = true;
    let mut idx = 0usize;
    let mut emitted_any = false;
    let mut has_holes = false;
    for grp in shapes {
        for (ci, contour) in grp.iter().enumerate() {
            if contour.len() < 3 {
                continue;
            }
            if ci > 0 {
                has_holes = true;
            }
            if !first {
                shape.operations.push(Operation::Subpath);
            }
            first = false;
            for p in contour {
                shape.operations.push(Operation::AddPoint {
                    name: format!("p{}", idx),
                    at: (p[0], p[1]),
                    after: None,
                    mode: None,
                    tension: None,
                    arc_rx: None,
                    arc_ry: None,
                    arc_sweep: None,
                    arc_large: None,
                    arc_bulge: None,
                    control_c1: None,
                    control_c2: None,
                });
                idx += 1;
            }
            emitted_any = true;
        }
    }
    if emitted_any {
        shape.operations.push(Operation::Close);
        // A holey result must render with even-odd so the holes punch through;
        // also honor an explicit even-odd request.
        if has_holes || fill_rule == IFillRule::EvenOdd {
            shape
                .operations
                .push(Operation::FillRule(crate::types::FillRule::EvenOdd));
        }
    }
    shape
}

/// Convenience: resolve `pd` (document space) → shapes. Used by stroke_outline.
pub fn area_of(shapes: &Shapes) -> f64 {
    let mut a = 0.0;
    for grp in shapes {
        for (i, contour) in grp.iter().enumerate() {
            let area = polygon_area(contour);
            if i == 0 {
                a += area;
            } else {
                a -= area;
            }
        }
    }
    a
}

fn polygon_area(pts: &[Pt]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        a += pts[i][0] * pts[j][1] - pts[j][0] * pts[i][1];
    }
    (a / 2.0).abs()
}

/// Build a document-space `PathData` from a shape that places its geometry at
/// `at`/`size`. This resolves the shape and applies the placement transform so
/// the returned points are in document coordinates — the common frame booleans
/// operate in.
pub fn placed_pathdata(pd: &PathData, placement: Option<&crate::path_point::Placement>) -> Shapes {
    let d = crate::path_point::path_data_to_svg_d(pd, placement);
    svg_d_to_shapes(&d)
}

/// Reconstruct a `PathData` (document coords, sharp polygon) directly from a
/// contour-set — used by tests and the stroke/offset producers that already
/// have contours.
pub fn shapes_to_pathdata(shapes: &Shapes) -> PathData {
    let mut points = Vec::new();
    let mut subpath_starts = Vec::new();
    let mut first = true;
    let mut idx = 0usize;
    for grp in shapes {
        for contour in grp {
            if contour.len() < 3 {
                continue;
            }
            if !first {
                subpath_starts.push(points.len());
            }
            first = false;
            for p in contour {
                points.push(NamedPoint {
                    name: format!("p{}", idx),
                    x: p[0],
                    y: p[1],
                    mode: CurveMode::Sharp,
                });
                idx += 1;
            }
        }
    }
    PathData {
        coord_space: (0.0, 0.0),
        points,
        closed: true,
        subpath_starts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_d(x: f64, y: f64, s: f64) -> String {
        format!(
            "M{} {} L{} {} L{} {} L{} {}Z",
            x,
            y,
            x + s,
            y,
            x + s,
            y + s,
            x,
            y + s
        )
    }

    #[test]
    fn union_area_identity() {
        let a = svg_d_to_shapes(&square_d(0.0, 0.0, 10.0));
        let b = svg_d_to_shapes(&square_d(5.0, 5.0, 10.0));
        let inter = apply(
            BoolOp::Intersect,
            &[a.clone(), b.clone()],
            IFillRule::NonZero,
        );
        let union = apply(BoolOp::Union, &[a.clone(), b.clone()], IFillRule::NonZero);
        let ia = area_of(&inter);
        let ua = area_of(&union);
        assert!((ia - 25.0).abs() < 1e-6, "intersect area {ia}");
        // area(A∪B) ≈ area(A)+area(B)−area(A∩B)
        assert!((ua - (100.0 + 100.0 - ia)).abs() < 1e-6, "union {ua}");
    }

    #[test]
    fn subtract_makes_hole() {
        let outer = svg_d_to_shapes(&square_d(0.0, 0.0, 30.0));
        let inner = svg_d_to_shapes(&square_d(10.0, 10.0, 10.0));
        let res = apply(BoolOp::Subtract, &[outer, inner], IFillRule::NonZero);
        let a = area_of(&res);
        assert!((a - 800.0).abs() < 1e-6, "donut area {a}");
        // the result shape carries a hole → even-odd fill-rule
        let shape = shapes_to_shape("donut", &res, IFillRule::NonZero);
        assert!(shape
            .operations
            .iter()
            .any(|o| matches!(o, Operation::FillRule(crate::types::FillRule::EvenOdd))));
    }

    #[test]
    fn exclude_is_symmetric_difference() {
        let a = svg_d_to_shapes(&square_d(0.0, 0.0, 10.0));
        let b = svg_d_to_shapes(&square_d(5.0, 5.0, 10.0));
        let res = apply(BoolOp::Exclude, &[a, b], IFillRule::NonZero);
        let area = area_of(&res);
        assert!((area - 150.0).abs() < 1e-6, "xor area {area}");
    }

    #[test]
    fn intersect_disjoint_is_empty() {
        let a = svg_d_to_shapes(&square_d(0.0, 0.0, 10.0));
        let b = svg_d_to_shapes(&square_d(100.0, 100.0, 10.0));
        let res = apply(BoolOp::Intersect, &[a, b], IFillRule::NonZero);
        assert!(area_of(&res) < 1e-9, "disjoint intersect empty");
    }

    #[test]
    fn degenerate_input_no_panic() {
        // empty d, single-point, line — must not panic, yields empty.
        for d in ["", "M0 0", "M0 0 L10 10"] {
            let s = svg_d_to_shapes(d);
            let _ = apply(BoolOp::Union, &[s], IFillRule::NonZero);
        }
    }

    #[test]
    fn parse_round_trips_names() {
        for name in ["union", "subtract", "intersect", "exclude"] {
            assert_eq!(BoolOp::parse(name).unwrap().name(), name);
        }
        assert!(BoolOp::parse("nope").is_err());
    }
}
