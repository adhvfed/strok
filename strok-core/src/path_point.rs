//! Named points, curve modes, and cardinal spline → SVG path conversion.

// `arc_extrema` takes the full SVG arc endpoint parameterization (8 args); that
// is the natural signature for the W3C algorithm it implements.
#![allow(clippy::too_many_arguments)]

/// How the segment arriving at a point is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum CurveMode {
    /// Straight line segment (default).
    Sharp,
    /// Catmull-Rom cardinal spline. Tension follows Kochanek-Bartels convention:
    /// 0 = Catmull-Rom (smooth), 1 = linear (straight lines).
    /// Negative values produce exaggerated curvature.
    CatmullRom(f64),
    /// Explicit cubic bezier control handles (absolute coordinates).
    Controls { c1: (f64, f64), c2: (f64, f64) },
    /// Explicit cubic bezier control handles (relative to point position).
    ControlsRelative { c1: (f64, f64), c2: (f64, f64) },
    /// Elliptical arc segment.
    Arc {
        rx: f64,
        ry: f64,
        sweep: bool,
        large: bool,
    },
}

/// A named point within a path's local coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedPoint {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub mode: CurveMode,
}

/// Ordered list of named points that defines a path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathData {
    /// Authoring coordinate space (width x height).
    pub coord_space: (f64, f64),
    pub points: Vec<NamedPoint>,
    pub closed: bool,
    /// Indices into `points` where a *new subpath* begins (each emits a fresh
    /// `M`). Index `0` is implicit (the first point always starts subpath 0) and
    /// is never listed. An empty vec ⇒ the classic single-subpath path; this is
    /// the default and keeps every pre-C3 path byte-identical. Boolean / outline /
    /// offset results use this to carry holes + disjoint pieces as multiple
    /// closed contours in one shape. Hand-authored paths may also use it for
    /// independent open routes; `closed` applies to every subpath.
    pub subpath_starts: Vec<usize>,
}

impl PathData {
    /// A single-subpath path (the common case). Equivalent to the pre-C3
    /// struct-literal with `subpath_starts: Vec::new()`.
    pub fn new(coord_space: (f64, f64), points: Vec<NamedPoint>, closed: bool) -> Self {
        PathData {
            coord_space,
            points,
            closed,
            subpath_starts: Vec::new(),
        }
    }

    /// Whether this path carries explicit subpath breaks (a multi-contour result
    /// from a boolean / outline / offset op).
    pub fn has_subpaths(&self) -> bool {
        !self.subpath_starts.is_empty()
    }
}

/// How to place a shape or path instance in the parent coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub at: (f64, f64),
    pub size: Option<(f64, f64)>,
    pub flip: Option<Flip>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flip {
    X,
    Y,
    XY,
}

/// Convert named points + curve modes → SVG path `d` attribute string.
///
/// Catmull-Rom cardinal spline formula (Kochanek-Bartels convention):
///   outgoing control point = P[i] + (P[i+1] - P[i-1]) * (1 - tension) / 6
///   incoming control point = P[i] - (P[i+1] - P[i-1]) * (1 - tension) / 6
/// tension=0 → Catmull-Rom (smooth), tension=1 → linear.
pub fn path_data_to_svg_d(data: &PathData, placement: Option<&Placement>) -> String {
    if data.points.is_empty() {
        return String::new();
    }

    // Compute scale, bbox-min subtraction, and offset from placement.
    // When size is given: bbox-fit — points are scaled from their natural bbox into
    //   the placed region. This matches rectangle/ellipse intuition.
    // When size is omitted: translate-only — points render at their authored coords,
    //   offset by `at`.
    let (sx, sy, bx, by, ox, oy, flip_x, flip_y) = placement_transform(data, placement);

    let pts = &data.points;
    let n = pts.len();
    let mut d = String::new();

    // Multi-subpath path (boolean / outline / offset result): emit each contour
    // as its own `M … Z`. These are flattened polygons (all sharp), so the
    // geometry is exact under any affine; we reuse the same `tx` closure built
    // below by short-circuiting here once it's available.

    // Compute the flip extent: when a size is given, flip around the placed size.
    // When no size is given, flip around the shape's natural bounding box (post bbox-subtract).
    let (flip_w, flip_h) = if let Some((w, h)) = placement.and_then(|p| p.size) {
        (w, h)
    } else {
        let max_x = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let max_y = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        ((max_x - bx) * sx, (max_y - by) * sy)
    };

    // Transform a point from local coords to placed coords.
    let tx = |x: f64, y: f64| -> (f64, f64) {
        let mut px = (x - bx) * sx;
        let mut py = (y - by) * sy;
        if flip_x {
            px = flip_w - px;
        }
        if flip_y {
            py = flip_h - py;
        }
        (px + ox, py + oy)
    };

    // Multi-subpath path: a set of independent contours (hand-authored compound
    // shapes, or a boolean/outline/offset result). Each contour `[begin, end)` is
    // emitted through the SAME curve-aware `emit_contour` the single-path branch
    // uses — so a per-point `mode=arc`/`catmull-rom` contour keeps its curves,
    // and catmull neighbour lookups wrap *within* the contour (the sub-slice
    // handed to `emit_contour` bounds the wrap, never reaching across a subpath
    // boundary). Boolean/outline/offset producers set `closed=true`; authored
    // compound routes can set `closed=false`. An all-`Sharp` closed contour
    // therefore emits the exact `M … L … Z` the previous flatten fast path did,
    // byte-for-byte (regression lock for the boolean / outline / offset goldens).
    if !data.subpath_starts.is_empty() {
        let mut starts = vec![0usize];
        starts.extend(data.subpath_starts.iter().copied().filter(|&i| i < n));
        starts.dedup();
        for w in 0..starts.len() {
            let begin = starts[w];
            let end = if w + 1 < starts.len() {
                starts[w + 1]
            } else {
                n
            };
            if begin >= end {
                continue;
            }
            emit_contour(pts, begin, end, data.closed, &tx, &mut d);
        }
        return d;
    }

    // Single-subpath path: one contour over all points, honouring `closed`.
    emit_contour(pts, 0, n, data.closed, &tx, &mut d);

    d
}

/// Emit one contour — the points `[begin, end)` of `pts` — as a fresh `M …`
/// subpath, routing every point through the curve-aware segment emission
/// (sharp / catmull-rom / controls / arc). When `closed`, appends the closing
/// segment (last → first) and a `Z`.
///
/// Both the single-path and multi-subpath branches call this, so there is ONE
/// implementation of the per-point segment math (important: this file is
/// mutation-tested — a duplicated copy would bleed surviving mutants). Because
/// `emit_segment` / `emit_closing_segment` / the catmull control helpers index
/// into the `[begin, end)` sub-slice, their neighbour lookups wrap within *this*
/// contour only.
fn emit_contour(
    pts: &[NamedPoint],
    begin: usize,
    end: usize,
    closed: bool,
    tx: &dyn Fn(f64, f64) -> (f64, f64),
    d: &mut String,
) {
    let contour = &pts[begin..end];
    let n = contour.len();
    if n == 0 {
        return;
    }
    let (fx, fy) = tx(contour[0].x, contour[0].y);
    d.push_str(&format!("M{} {}", fmt_num(fx), fmt_num(fy)));
    for i in 1..n {
        emit_segment(contour, i, n, closed, tx, d);
    }
    if closed {
        emit_closing_segment(contour, n, tx, d);
        d.push('Z');
    }
}

/// Emit a single segment from point i-1 to point i.
fn emit_segment(
    pts: &[NamedPoint],
    i: usize,
    n: usize,
    closed: bool,
    tx: &dyn Fn(f64, f64) -> (f64, f64),
    d: &mut String,
) {
    let prev = &pts[i - 1];
    let curr = &pts[i];

    match &curr.mode {
        CurveMode::Sharp => {
            let (x, y) = tx(curr.x, curr.y);
            d.push_str(&format!(" L{} {}", fmt_num(x), fmt_num(y)));
        }
        CurveMode::CatmullRom(tension) => {
            // Cardinal spline: compute control points.
            let (c1x, c1y) = outgoing_control(pts, i - 1, n, closed, *tension);
            let (c2x, c2y) = incoming_control(pts, i, n, closed, *tension);
            let (c1x, c1y) = tx(c1x, c1y);
            let (c2x, c2y) = tx(c2x, c2y);
            let (x, y) = tx(curr.x, curr.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::Controls { c1, c2 } => {
            let (c1x, c1y) = tx(c1.0, c1.1);
            let (c2x, c2y) = tx(c2.0, c2.1);
            let (x, y) = tx(curr.x, curr.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::ControlsRelative { c1, c2 } => {
            let abs_c1 = (prev.x + c1.0, prev.y + c1.1);
            let abs_c2 = (curr.x + c2.0, curr.y + c2.1);
            let (c1x, c1y) = tx(abs_c1.0, abs_c1.1);
            let (c2x, c2y) = tx(abs_c2.0, abs_c2.1);
            let (x, y) = tx(curr.x, curr.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::Arc {
            rx,
            ry,
            sweep,
            large,
        } => {
            emit_arc_segment(
                prev.x, prev.y, curr.x, curr.y, *rx, *ry, *sweep, *large, tx, d,
            );
        }
    }
}

/// Emit the closing segment (last point → first point).
fn emit_closing_segment(
    pts: &[NamedPoint],
    n: usize,
    tx: &dyn Fn(f64, f64) -> (f64, f64),
    d: &mut String,
) {
    let first = &pts[0];

    match &first.mode {
        CurveMode::Sharp => {
            // Z handles this — no explicit line needed.
        }
        CurveMode::CatmullRom(tension) => {
            let (c1x, c1y) = outgoing_control(pts, n - 1, n, true, *tension);
            let (c2x, c2y) = incoming_control(pts, 0, n, true, *tension);
            let (c1x, c1y) = tx(c1x, c1y);
            let (c2x, c2y) = tx(c2x, c2y);
            let (x, y) = tx(first.x, first.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::Controls { c1, c2 } => {
            let (c1x, c1y) = tx(c1.0, c1.1);
            let (c2x, c2y) = tx(c2.0, c2.1);
            let (x, y) = tx(first.x, first.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::ControlsRelative { c1, c2 } => {
            let last = &pts[n - 1];
            let abs_c1 = (last.x + c1.0, last.y + c1.1);
            let abs_c2 = (first.x + c2.0, first.y + c2.1);
            let (c1x, c1y) = tx(abs_c1.0, abs_c1.1);
            let (c2x, c2y) = tx(abs_c2.0, abs_c2.1);
            let (x, y) = tx(first.x, first.y);
            d.push_str(&format!(
                " C{} {}, {} {}, {} {}",
                fmt_num(c1x),
                fmt_num(c1y),
                fmt_num(c2x),
                fmt_num(c2y),
                fmt_num(x),
                fmt_num(y),
            ));
        }
        CurveMode::Arc {
            rx,
            ry,
            sweep,
            large,
        } => {
            let last = &pts[n - 1];
            emit_arc_segment(
                last.x, last.y, first.x, first.y, *rx, *ry, *sweep, *large, tx, d,
            );
        }
    }
}

/// Emit one elliptical-arc segment from local (x1,y1) to local (x2,y2),
/// transformed by `tx`.
///
/// **Cause B fix (Decision D-2: convert to cubics under non-uniform/flipped
/// transforms).** The arc's `rx`/`ry` describe a circle (or axis-aligned
/// ellipse) in *local* coordinates. Under a uniform scale we can keep the
/// compact `A` command (scaling both radii by the single factor preserves the
/// shape). But under a *non-uniform* scale — or a flip, which mirrors one axis —
/// the old code emitted `A {rx·sx} {ry·sy} 0 …`, which silently turns a circular
/// arc into a distorted ellipse arc with the wrong proportions and no rotation
/// term. Instead we sample the arc as cubic Béziers *in local space* (where the
/// geometry is correct), then push every control point through `tx`. An affine
/// map sends Béziers to Béziers exactly, so the result is the geometrically
/// faithful image of the intended arc under any placement.
#[allow(clippy::too_many_arguments)]
fn emit_arc_segment(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    rx: f64,
    ry: f64,
    sweep: bool,
    large: bool,
    tx: &dyn Fn(f64, f64) -> (f64, f64),
    d: &mut String,
) {
    let sx = sx_from_tx(tx);
    let sy = sy_from_tx(tx);
    let uniform = (sx - sy).abs() <= 1e-9 * sx.max(sy).max(1.0);
    let flipped = transform_flips(tx);

    if uniform && !flipped {
        // Uniform scale, no mirror: the compact `A` command is exact.
        let (x, y) = tx(x2, y2);
        let arx = rx * sx;
        let ary = ry * sy;
        d.push_str(&format!(
            " A{} {} 0 {} {} {} {}",
            fmt_num(arx),
            fmt_num(ary),
            if large { 1 } else { 0 },
            if sweep { 1 } else { 0 },
            fmt_num(x),
            fmt_num(y),
        ));
        return;
    }

    // Non-uniform or flipped: convert to cubics in local space, then transform.
    let beziers = arc_to_cubics(x1, y1, x2, y2, rx, ry, large, sweep);
    if beziers.is_empty() {
        // Degenerate arc → straight line to endpoint.
        let (x, y) = tx(x2, y2);
        d.push_str(&format!(" L{} {}", fmt_num(x), fmt_num(y)));
        return;
    }
    for (c1, c2, end) in beziers {
        let (c1x, c1y) = tx(c1.0, c1.1);
        let (c2x, c2y) = tx(c2.0, c2.1);
        let (ex, ey) = tx(end.0, end.1);
        d.push_str(&format!(
            " C{} {}, {} {}, {} {}",
            fmt_num(c1x),
            fmt_num(c1y),
            fmt_num(c2x),
            fmt_num(c2y),
            fmt_num(ex),
            fmt_num(ey),
        ));
    }
}

/// Does the transform mirror an axis? A flip negates a basis-vector direction;
/// we detect it by checking whether the x or y image direction is reversed.
fn transform_flips(tx: &dyn Fn(f64, f64) -> (f64, f64)) -> bool {
    let (ox, oy) = tx(0.0, 0.0);
    let (x1, _) = tx(1.0, 0.0);
    let (_, y1) = tx(0.0, 1.0);
    (x1 - ox) < 0.0 || (y1 - oy) < 0.0
}

/// A cubic Bézier segment as `(control1, control2, end)`; the start point is the
/// previous segment's end (or the path's current point).
type CubicSeg = ((f64, f64), (f64, f64), (f64, f64));

/// Convert an SVG elliptical arc (endpoint parameterization, x-axis-rotation 0)
/// into a list of cubic Bézier segments `(c1, c2, end)` in the same coordinate
/// space. Returns empty for a degenerate arc (zero radius / zero-length chord),
/// which the caller renders as a line.
///
/// Follows the SVG implementation notes' endpoint→center conversion
/// (https://www.w3.org/TR/SVG/implnote.html#ArcConversionEndpointToCenter) and
/// the standard `4/3·tan(Δ/4)` cubic approximation, splitting the sweep into
/// ≤90° pieces so each cubic stays well within tolerance.
#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    rx: f64,
    ry: f64,
    large: bool,
    sweep: bool,
) -> Vec<CubicSeg> {
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    if rx < 1e-12 || ry < 1e-12 || ((x1 - x2).abs() < 1e-12 && (y1 - y2).abs() < 1e-12) {
        return Vec::new();
    }
    // Endpoint → center parameterization (x-axis-rotation φ = 0).
    let x1p = (x1 - x2) / 2.0;
    let y1p = (y1 - y2) / 2.0;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let num = (rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p).max(0.0);
    let den = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * (-(ry * x1p / rx));
    let cx = cxp + (x1 + x2) / 2.0;
    let cy = cyp + (y1 + y2) / 2.0;

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = vec_angle(1.0, 0.0, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * std::f64::consts::PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * std::f64::consts::PI;
    }

    // Split into ≤90° pieces. Endpoint-to-center conversion can put an exact
    // quarter turn a few ULPs above π/2 on one architecture and below it on
    // another. Subtracting a tiny relative tolerance keeps the segment count
    // deterministic without accepting a meaningfully larger arc.
    let quarter_turns = dtheta.abs() / std::f64::consts::FRAC_PI_2;
    let segments = (quarter_turns - 1e-12).ceil().max(1.0) as usize;
    let delta = dtheta / segments as f64;
    let t = (4.0 / 3.0) * (delta / 4.0).tan();

    let mut out = Vec::with_capacity(segments);
    let mut angle = theta1;
    // Point and derivative on the (axis-aligned) ellipse at param `a`.
    let pt = |a: f64| (cx + rx * a.cos(), cy + ry * a.sin());
    let der = |a: f64| (-rx * a.sin(), ry * a.cos());
    let (mut px, mut py) = pt(angle);
    for _ in 0..segments {
        let a2 = angle + delta;
        let (ex, ey) = pt(a2);
        let (d1x, d1y) = der(angle);
        let (d2x, d2y) = der(a2);
        let c1 = (px + t * d1x, py + t * d1y);
        let c2 = (ex - t * d2x, ey - t * d2y);
        out.push((c1, c2, (ex, ey)));
        px = ex;
        py = ey;
        angle = a2;
    }
    out
}

/// Outgoing control point for Catmull-Rom cardinal spline at point index `i`.
/// Formula: P[i] + (P[next] - P[prev]) * (1 - tension) / 6
fn outgoing_control(
    pts: &[NamedPoint],
    i: usize,
    n: usize,
    closed: bool,
    tension: f64,
) -> (f64, f64) {
    let p = &pts[i];
    let next = if i + 1 < n {
        &pts[i + 1]
    } else if closed {
        &pts[0]
    } else {
        &pts[i]
    };
    let prev = if i > 0 {
        &pts[i - 1]
    } else if closed {
        &pts[n - 1]
    } else {
        &pts[i]
    };
    let t = (1.0 - tension) / 6.0;
    (p.x + (next.x - prev.x) * t, p.y + (next.y - prev.y) * t)
}

/// Incoming control point for Catmull-Rom cardinal spline at point index `i`.
/// Formula: P[i] - (P[next] - P[prev]) * (1 - tension) / 6
fn incoming_control(
    pts: &[NamedPoint],
    i: usize,
    n: usize,
    closed: bool,
    tension: f64,
) -> (f64, f64) {
    let p = &pts[i];
    let next = if i + 1 < n {
        &pts[i + 1]
    } else if closed {
        &pts[0]
    } else {
        &pts[i]
    };
    let prev = if i > 0 {
        &pts[i - 1]
    } else if closed {
        &pts[n - 1]
    } else {
        &pts[i]
    };
    let t = (1.0 - tension) / 6.0;
    (p.x - (next.x - prev.x) * t, p.y - (next.y - prev.y) * t)
}

/// Compute (sx, sy, bx, by, ox, oy, flip_x, flip_y) from PathData + Placement.
///
/// - `sx`, `sy`: scale factors applied to local coords.
/// - `bx`, `by`: bbox-min subtraction applied before scaling (bbox-fit semantics
///   when `size` is given; zero otherwise).
/// - `ox`, `oy`: translation added after scaling (= `at`).
///
/// With size: scale so that the points' bbox span maps to the placed size,
/// translate so bbox_min lands at `at`. This matches rectangle/ellipse intuition.
/// Without size: scale = 1, no bbox subtraction — absolute coords translated by `at`.
fn placement_transform(
    data: &PathData,
    placement: Option<&Placement>,
) -> (f64, f64, f64, f64, f64, f64, bool, bool) {
    match placement {
        None => (1.0, 1.0, 0.0, 0.0, 0.0, 0.0, false, false),
        Some(p) => {
            let (sx, sy, bx, by) = match p.size {
                Some((w, h)) => {
                    // Use the geometry bbox (which accounts for arc bulge), not
                    // just the anchor points — otherwise a horizontal-chord arc
                    // reports zero height and the fit scale blows up.
                    let (min_x, min_y, max_x, max_y) = geometry_bbox(&data.points, data.closed);
                    let span_x = (max_x - min_x).max(1e-9);
                    let span_y = (max_y - min_y).max(1e-9);
                    (w / span_x, h / span_y, min_x, min_y)
                }
                None => (1.0, 1.0, 0.0, 0.0),
            };
            let (flip_x, flip_y) = match p.flip {
                Some(Flip::X) => (true, false),
                Some(Flip::Y) => (false, true),
                Some(Flip::XY) => (true, true),
                None => (false, false),
            };
            (sx, sy, bx, by, p.at.0, p.at.1, flip_x, flip_y)
        }
    }
}

/// Compute bbox (min_x, min_y, max_x, max_y) of the given points.
fn points_bbox(pts: &[NamedPoint]) -> (f64, f64, f64, f64) {
    if pts.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in pts {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    (min_x, min_y, max_x, max_y)
}

/// Compute the true geometry bbox of a path: the anchor points plus the
/// extrema contributed by curved segments. Today only arc segments bulge
/// beyond their endpoints (catmull/bezier handles stay within the anchor
/// span for our purposes), so those are the only extrema added.
///
/// This matters for bbox-fit placement: an arc whose endpoints share a
/// coordinate (e.g. a semicircle from (0,18) to (36,18)) has zero *anchor*
/// height but real geometric height. Scaling against the anchor-only span
/// divides by ~zero and explodes the rendered radius.
pub fn geometry_bbox(pts: &[NamedPoint], closed: bool) -> (f64, f64, f64, f64) {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = points_bbox(pts);
    if pts.is_empty() {
        return (min_x, min_y, max_x, max_y);
    }
    let n = pts.len();
    let mut consider = |x: f64, y: f64| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    // A segment arrives at point i carrying point i's mode. The closing
    // segment (last → first) carries the first point's mode.
    for i in 1..n {
        if let CurveMode::Arc {
            rx,
            ry,
            sweep,
            large,
        } = pts[i].mode
        {
            for (ex, ey) in arc_extrema(
                pts[i - 1].x,
                pts[i - 1].y,
                pts[i].x,
                pts[i].y,
                rx,
                ry,
                large,
                sweep,
            ) {
                consider(ex, ey);
            }
        }
    }
    if closed {
        if let CurveMode::Arc {
            rx,
            ry,
            sweep,
            large,
        } = pts[0].mode
        {
            for (ex, ey) in arc_extrema(
                pts[n - 1].x,
                pts[n - 1].y,
                pts[0].x,
                pts[0].y,
                rx,
                ry,
                large,
                sweep,
            ) {
                consider(ex, ey);
            }
        }
    }
    (min_x, min_y, max_x, max_y)
}

/// Axis-aligned extreme points of an SVG elliptical arc from (x1,y1) to
/// (x2,y2). strok arcs have no x-axis rotation, so the extrema sit at the
/// param angles 0, 90, 180, 270° that fall on the swept portion. Endpoints
/// are handled by the caller; this returns only the interior extrema.
///
/// Follows the SVG implementation notes' endpoint→center parameterization
/// (https://www.w3.org/TR/SVG/implnote.html#ArcImplementationNotes).
fn arc_extrema(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    rx: f64,
    ry: f64,
    large: bool,
    sweep: bool,
) -> Vec<(f64, f64)> {
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    // Degenerate radius or zero-length segment → renders as a line; endpoints
    // already cover it.
    if rx < 1e-9 || ry < 1e-9 || ((x1 - x2).abs() < 1e-12 && (y1 - y2).abs() < 1e-12) {
        return Vec::new();
    }
    let x1p = (x1 - x2) / 2.0;
    let y1p = (y1 - y2) / 2.0;
    // Scale radii up if they're too small to span the chord.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let num = (rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p).max(0.0);
    let den = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * (-(ry * x1p / rx));
    let cx = cxp + (x1 + x2) / 2.0;
    let cy = cyp + (y1 + y2) / 2.0;

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = vec_angle(1.0, 0.0, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * std::f64::consts::PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * std::f64::consts::PI;
    }

    use std::f64::consts::{FRAC_PI_2, PI};
    let candidates = [0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2];
    let mut out = Vec::new();
    for phi in candidates {
        if angle_on_sweep(phi, theta1, dtheta) {
            out.push((cx + rx * phi.cos(), cy + ry * phi.sin()));
        }
    }
    out
}

/// Signed angle (radians) from vector (ux,uy) to (vx,vy), in (-π, π].
fn vec_angle(ux: f64, uy: f64, vx: f64, vy: f64) -> f64 {
    let dot = ux * vx + uy * vy;
    let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
    if len == 0.0 {
        return 0.0;
    }
    let mut a = (dot / len).clamp(-1.0, 1.0).acos();
    if ux * vy - uy * vx < 0.0 {
        a = -a;
    }
    a
}

/// Does param angle `phi` fall on the arc swept from `theta1` by `dtheta`?
///
/// `phi` is a candidate axis angle (0/90/180/270°) that is NOT pre-reduced
/// relative to `theta1`, so `delta = phi - theta1` must be normalized into the
/// canonical range for the sweep direction *fully* (modulo 2π), not just nudged
/// across zero. The earlier version only added 2π while `delta < 0` (and only
/// subtracted while `> 0`), so a candidate more than one turn away from
/// `theta1` (e.g. phi=270° with theta1≈-155°, giving delta≈425°) was wrongly
/// rejected — dropping the extremum and under-reporting the geometry bbox of a
/// large arc. We now reduce into `[0, 2π)` (or `(-2π, 0]`) before comparing.
fn angle_on_sweep(phi: f64, theta1: f64, dtheta: f64) -> bool {
    if dtheta == 0.0 {
        return false;
    }
    let two_pi = 2.0 * std::f64::consts::PI;
    let delta = phi - theta1;
    if dtheta > 0.0 {
        // Reduce delta into [0, 2π).
        let d = delta.rem_euclid(two_pi);
        d <= dtheta + 1e-9
    } else {
        // Reduce delta into (-2π, 0].
        let mut d = delta.rem_euclid(two_pi);
        if d > 0.0 {
            d -= two_pi;
        }
        d >= dtheta - 1e-9
    }
}

/// Helper: extract sx scale factor. We use (1,0) → measure x distance.
fn sx_from_tx(tx: &dyn Fn(f64, f64) -> (f64, f64)) -> f64 {
    let (x0, _) = tx(0.0, 0.0);
    let (x1, _) = tx(1.0, 0.0);
    (x1 - x0).abs()
}

fn sy_from_tx(tx: &dyn Fn(f64, f64) -> (f64, f64)) -> f64 {
    let (_, y0) = tx(0.0, 0.0);
    let (_, y1) = tx(0.0, 1.0);
    (y1 - y0).abs()
}

/// Format a number for an SVG coordinate, delegating to the shared scale-aware
/// formatter in `types` so path data and attribute output never diverge.
///
/// Fixes Cause C: the old `{:.4}` truncation quantized coordinates to 1e-4 of a
/// *document unit*, which after an 8–64× render scale became visible sub-pixel
/// drift on long smooth curves. `types::fmt_num` now emits ~6 significant
/// figures, scale-aware, so high-precision control points survive.
fn fmt_num(n: f64) -> String {
    crate::types::fmt_num(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sharp(name: &str, x: f64, y: f64) -> NamedPoint {
        NamedPoint {
            name: name.to_string(),
            x,
            y,
            mode: CurveMode::Sharp,
        }
    }

    fn smooth(name: &str, x: f64, y: f64, tension: f64) -> NamedPoint {
        NamedPoint {
            name: name.to_string(),
            x,
            y,
            mode: CurveMode::CatmullRom(tension),
        }
    }

    #[test]
    fn exact_quarter_arc_uses_one_cubic_on_every_architecture() {
        let segments = arc_to_cubics(0.0, 10.0, 10.0, 0.0, 10.0, 10.0, false, false);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn two_sharp_points_open() {
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![sharp("a", 0.0, 0.0), sharp("b", 100.0, 50.0)],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        assert_eq!(d, "M0 0 L100 50");
    }

    #[test]
    fn triangle_closed() {
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 50.0, 0.0),
                sharp("b", 100.0, 100.0),
                sharp("c", 0.0, 100.0),
            ],
            closed: true,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        assert_eq!(d, "M50 0 L100 100 L0 100Z");
    }

    #[test]
    fn smooth_diamond() {
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                smooth("left", 0.0, 50.0, -0.2),
                smooth("top", 50.0, 0.0, -0.2),
                smooth("right", 100.0, 50.0, -0.2),
                smooth("bottom", 50.0, 100.0, -0.2),
            ],
            closed: true,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        // Should contain C commands and end with Z.
        assert!(d.starts_with("M0 50"));
        assert!(d.contains('C'));
        assert!(d.ends_with('Z'));
    }

    #[test]
    fn placement_offset_and_scale() {
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![sharp("a", 0.0, 0.0), sharp("b", 100.0, 100.0)],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let p = Placement {
            at: (10.0, 20.0),
            size: Some((50.0, 50.0)),
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        assert_eq!(d, "M10 20 L60 70");
    }

    #[test]
    fn placement_bbox_fit_respects_author_bbox() {
        // Path with points NOT starting at origin: author intent is that the
        // bbox of authored points maps into the placed size. `at` should be
        // where the bbox's top-left lands.
        let data = PathData {
            coord_space: (400.0, 400.0), // irrelevant for path bbox-fit
            points: vec![sharp("a", 10.0, 20.0), sharp("b", 30.0, 40.0)],
            closed: false,
            subpath_starts: Vec::new(),
        };
        // Placed at its own bbox size: should render at absolute coords.
        let p = Placement {
            at: (10.0, 20.0),
            size: Some((20.0, 20.0)),
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        assert_eq!(d, "M10 20 L30 40");
    }

    #[test]
    fn placement_bbox_fit_scales_reusable_shape() {
        // A "glyph" authored in 0..10 × 0..10 space placed at different sizes
        // should scale naturally to the placed region.
        let data = PathData {
            coord_space: (800.0, 600.0), // document size — should NOT leak into scale
            points: vec![
                sharp("a", 0.0, 10.0),
                sharp("b", 5.0, 0.0),
                sharp("c", 10.0, 10.0),
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let p = Placement {
            at: (100.0, 500.0),
            size: Some((20.0, 20.0)), // 2x the authored bbox
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        // Each authored unit = 2 doc units; a→b goes from (0,10) to (5,0)
        // → (100, 520) to (110, 500).
        assert_eq!(d, "M100 520 L110 500 L120 520");
    }

    #[test]
    fn placement_without_size_preserves_absolute_coords() {
        // Paths placed without size should render at authored coords, translated by at.
        let data = PathData {
            coord_space: (800.0, 600.0),
            points: vec![sharp("a", 277.0, 380.0), sharp("b", 357.0, 540.0)],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let p = Placement {
            at: (0.0, 0.0),
            size: None,
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        assert_eq!(d, "M277 380 L357 540");
    }

    #[test]
    fn arc_bbox_accounts_for_bulge_under_bbox_fit() {
        // A semicircle whose endpoints share a y (chord is horizontal) has zero
        // *anchor* height but real geometric height. bbox-fit must scale against
        // the geometry bbox, not the anchor span — otherwise sy → ∞ and the
        // emitted radius explodes. Regression for the "180° arc" bug.
        let data = PathData {
            coord_space: (56.0, 56.0),
            points: vec![
                sharp("a", 0.0, 18.0),
                NamedPoint {
                    name: "b".to_string(),
                    x: 36.0,
                    y: 18.0,
                    mode: CurveMode::Arc {
                        rx: 18.0,
                        ry: 18.0,
                        sweep: true,
                        large: false,
                    },
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        // The geometry bbox is 36 wide × 18 tall; placing at that exact size
        // must leave the radius unscaled (1:1).
        let p = Placement {
            at: (10.0, 20.0),
            size: Some((36.0, 18.0)),
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        assert_eq!(d, "M10 38 A18 18 0 0 1 46 38");

        // And the geometry bbox itself spans the full bulge.
        let (min_x, min_y, max_x, max_y) = geometry_bbox(&data.points, data.closed);
        assert!((min_x - 0.0).abs() < 1e-6, "min_x = {min_x}");
        assert!((min_y - 0.0).abs() < 1e-6, "min_y = {min_y}");
        assert!((max_x - 36.0).abs() < 1e-6, "max_x = {max_x}");
        assert!((max_y - 18.0).abs() < 1e-6, "max_y = {max_y}");
    }

    #[test]
    fn cause_b_circular_arc_stays_circular_under_nonuniform_size() {
        // The headline Cause B regression. A circular arc (rx=ry=30) placed at a
        // NON-square size used to emit `A {30·sx} {30·sy} 0 …` — silently turning
        // the circle into a distorted ellipse arc. The fix converts the arc to
        // cubics in local space and transforms them, so the rendered curve is the
        // geometrically faithful image of a circle under the placement's affine.
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 20.0, 70.0),
                NamedPoint {
                    name: "b".to_string(),
                    x: 80.0,
                    y: 70.0,
                    mode: CurveMode::Arc {
                        rx: 30.0,
                        ry: 30.0,
                        sweep: false,
                        large: false,
                    },
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        // Natural geometry bbox is 60 wide × 30 tall; placing at 80×40 is a
        // NON-uniform scale (sx = 80/60 ≈ 1.333, sy = 40/30 ≈ 1.333 — actually
        // uniform here). Use a deliberately non-uniform size to force the cubic path.
        let p = Placement {
            at: (10.0, 10.0),
            size: Some((80.0, 20.0)), // sx ≈ 1.333, sy ≈ 0.667 → non-uniform
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        // Must NOT contain an `A` command (would re-introduce the distortion);
        // the arc is now cubics.
        assert!(
            !d.contains(" A"),
            "non-uniform arc must be emitted as cubics, got: {d}"
        );
        assert!(d.contains('C'), "expected cubic segments, got: {d}");

        // The emitted curve must trace the SAME points an explicit non-uniform
        // affine would produce on a true circle. Sample the local circle arc and
        // confirm the placed midpoint matches: the circle through (20,70)→(80,70)
        // with the chosen bulge has its apex at local (50, 70 + h) for some h;
        // under the affine that apex maps faithfully. We verify by reconstructing
        // the local cubics and transforming, then comparing the SVG's last point.
        // End point check: (80,70) → ((80-20)*sx+10, (70-70)*sy+10) where bbox min
        // is (20,40) after bulge. Simpler: the endpoint must equal tx(80,70).
        let (sx, sy, bx, by, ox, oy, _, _) = placement_transform(&data, Some(&p));
        let ex = (80.0 - bx) * sx + ox;
        let ey = (70.0 - by) * sy + oy;
        let last = format!("{} {}", fmt_num(ex), fmt_num(ey));
        assert!(
            d.ends_with(&last),
            "endpoint must be the faithful image of (80,70): expected to end with '{last}', got: {d}"
        );
    }

    #[test]
    fn controls_relative_segment_d_is_pinned() {
        // Locks the ControlsRelative branch (mutation coverage): control handles
        // are relative to the *segment endpoints* (prev for c1, curr for c2).
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 10.0, 10.0),
                NamedPoint {
                    name: "b".to_string(),
                    x: 90.0,
                    y: 10.0,
                    mode: CurveMode::ControlsRelative {
                        c1: (5.0, 20.0),  // relative to prev (10,10) → (15,30)
                        c2: (-5.0, 20.0), // relative to curr (90,10) → (85,30)
                    },
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        assert_eq!(d, "M10 10 C15 30, 85 30, 90 10");
    }

    #[test]
    fn controls_relative_closing_segment_d_is_pinned() {
        // Locks the closing-segment ControlsRelative branch: c1 relative to the
        // LAST point, c2 relative to the FIRST point.
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                NamedPoint {
                    name: "a".to_string(),
                    x: 10.0,
                    y: 10.0,
                    mode: CurveMode::ControlsRelative {
                        c1: (0.0, 0.0),
                        c2: (0.0, 0.0),
                    },
                },
                sharp("b", 90.0, 10.0),
                sharp("c", 90.0, 90.0),
            ],
            closed: true,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        // Closing segment c→a: c1 = last(90,90)+(0,0); c2 = first(10,10)+(0,0).
        assert_eq!(d, "M10 10 L90 10 L90 90 C90 90, 10 10, 10 10Z");
    }

    #[test]
    fn flipped_arc_emits_cubics_not_distorted_arc() {
        // A flip mirrors one axis; the compact `A` command can't express that
        // faithfully, so the arc converts to cubics (locks transform_flips +
        // the flip branch). The endpoint must be the mirrored image.
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 0.0, 50.0),
                NamedPoint {
                    name: "b".to_string(),
                    x: 100.0,
                    y: 50.0,
                    mode: CurveMode::Arc {
                        rx: 50.0,
                        ry: 50.0,
                        sweep: true,
                        large: false,
                    },
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let p = Placement {
            at: (0.0, 0.0),
            size: None,
            flip: Some(Flip::X),
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        assert!(!d.contains(" A"), "flipped arc must be cubics, got: {d}");
        assert!(d.contains('C'), "expected cubics, got: {d}");
    }

    #[test]
    fn cause_b_uniform_scale_keeps_compact_arc() {
        // Under a UNIFORM scale (and no flip) the compact `A` command is exact,
        // so we keep it (smaller `d`, identical geometry). A circle stays circular.
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 0.0, 50.0),
                NamedPoint {
                    name: "b".to_string(),
                    x: 100.0,
                    y: 50.0,
                    mode: CurveMode::Arc {
                        rx: 50.0,
                        ry: 50.0,
                        sweep: true,
                        large: false,
                    },
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        // Geometry bbox 100 wide × 50 tall; place at 200×100 → sx=sy=2 (uniform).
        let p = Placement {
            at: (0.0, 0.0),
            size: Some((200.0, 100.0)),
            flip: None,
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        // Uniform: radii scaled equally → still a circle arc `A100 100`.
        assert!(
            d.contains("A100 100"),
            "expected uniform A100 100, got: {d}"
        );
    }

    #[test]
    fn placement_flip_x() {
        let data = PathData {
            coord_space: (100.0, 50.0),
            points: vec![sharp("a", 0.0, 0.0), sharp("b", 100.0, 50.0)],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let p = Placement {
            at: (0.0, 0.0),
            size: None,
            flip: Some(Flip::X),
        };
        let d = path_data_to_svg_d(&data, Some(&p));
        // flip_x: x → coord_space.0 - x = 100-0=100, 100-100=0
        assert_eq!(d, "M100 0 L0 50");
    }

    #[test]
    fn smooth_with_known_controls() {
        // 3 points in a line with catmull-rom middle, tension -0.2
        // (1 - (-0.2)) / 6 = 0.2, same scale factor as old formula
        // prev=(0,0), curr=(50,0), next=(100,0)
        // outgoing from prev: prev + (curr - prev_prev) * 0.2
        //   prev_prev doesn't exist (open), so prev_prev = prev
        //   = (0,0) + (50-0, 0-0) * 0.2 = (10, 0)
        // incoming to curr: curr - (next - prev) * 0.2
        //   = (50,0) - (100-0, 0-0) * 0.2 = (30, 0)
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 0.0, 0.0),
                smooth("b", 50.0, 0.0, -0.2),
                sharp("c", 100.0, 0.0),
            ],
            closed: false,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);
        // Segment a→b: b is CatmullRom(-0.2)
        // outgoing from a: a + (b - a_prev) * (1-(-0.2))/6 = a + (50,0) * 0.2 = (10, 0)
        // incoming to b: b - (c - a) * 0.2 = (50,0) - (100,0) * 0.2 = (30, 0)
        assert_eq!(d, "M0 0 C10 0, 30 0, 50 0 L100 0");
    }

    fn arc_pt(name: &str, x: f64, y: f64, rx: f64, ry: f64, sweep: bool) -> NamedPoint {
        NamedPoint {
            name: name.to_string(),
            x,
            y,
            mode: CurveMode::Arc {
                rx,
                ry,
                sweep,
                large: false,
            },
        }
    }

    #[test]
    fn multi_subpath_all_sharp_is_byte_identical_to_flatten_fastpath() {
        // Two all-`Sharp` closed contours (the boolean/outline/offset shape of a
        // result). The curve-aware `emit_contour` MUST reproduce the exact
        // `M … L … Z` the old flatten fast path emitted, byte-for-byte — this is
        // the regression lock that keeps the bool-*/outline/offset goldens
        // unchanged.
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 0.0, 0.0),
                sharp("b", 40.0, 0.0),
                sharp("c", 40.0, 40.0),
                sharp("d", 0.0, 40.0),
                // second contour (a hole)
                sharp("e", 10.0, 10.0),
                sharp("f", 30.0, 10.0),
                sharp("g", 30.0, 30.0),
                sharp("h", 10.0, 30.0),
            ],
            closed: true,
            subpath_starts: vec![4],
        };
        let d = path_data_to_svg_d(&data, None);
        assert_eq!(d, "M0 0 L40 0 L40 40 L0 40ZM10 10 L30 10 L30 30 L10 30Z");
    }

    #[test]
    fn multi_subpath_open_routes_stay_independent_and_open() {
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                sharp("a", 0.0, 0.0),
                sharp("b", 40.0, 0.0),
                sharp("c", 10.0, 20.0),
                sharp("d", 40.0, 20.0),
                sharp("e", 10.0, 40.0),
                sharp("f", 40.0, 40.0),
            ],
            closed: false,
            subpath_starts: vec![2, 4],
        };
        let d = path_data_to_svg_d(&data, None);
        assert_eq!(d, "M0 0 L40 0M10 20 L40 20M10 40 L40 40");
    }

    #[test]
    fn multi_subpath_inner_contour_honours_arc_mode() {
        // A sharp square outer contour + an arc-mode circle inner contour. The
        // old fast path flattened BOTH to polygons; now the inner contour keeps
        // its arcs. Assert the outer contour is straight lines and the inner
        // emits curve commands (`A` under the identity/uniform placement here).
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                // outer: sharp square
                sharp("a", 0.0, 0.0),
                sharp("b", 60.0, 0.0),
                sharp("c", 60.0, 60.0),
                sharp("d", 0.0, 60.0),
                // inner: circle as two semicircle arcs, entered at the seam point
                arc_pt("p", 20.0, 30.0, 10.0, 10.0, true),
                arc_pt("q", 40.0, 30.0, 10.0, 10.0, true),
            ],
            closed: true,
            subpath_starts: vec![4],
        };
        let d = path_data_to_svg_d(&data, None);
        let contours: Vec<&str> = d.split('M').filter(|s| !s.is_empty()).collect();
        assert_eq!(contours.len(), 2, "expected two contours, got: {d}");
        // Outer contour: straight lines only, no curves.
        assert!(
            contours[0].contains('L') && !contours[0].contains('A') && !contours[0].contains('C'),
            "outer contour should be polygon lines, got: {}",
            contours[0]
        );
        // Inner contour: arcs preserved (two semicircle `A` commands).
        assert_eq!(
            contours[1].matches('A').count(),
            2,
            "inner contour should keep its two arcs, got: {}",
            contours[1]
        );
    }

    #[test]
    fn multi_subpath_catmull_wraps_within_contour() {
        // A catmull-rom closed contour that lives as the SECOND subpath must
        // compute its control points from its OWN neighbours (wrapping within the
        // contour), never reaching back into the first contour. Proof: the second
        // contour's emitted `M…Z` must be byte-identical to the same points
        // emitted as a standalone single-subpath closed catmull path.
        let tri = |prefix: &str, ox: f64, oy: f64| {
            vec![
                smooth(&format!("{prefix}1"), ox + 0.0, oy + 0.0, -0.2),
                smooth(&format!("{prefix}2"), ox + 30.0, oy + 5.0, -0.2),
                smooth(&format!("{prefix}3"), ox + 15.0, oy + 30.0, -0.2),
            ]
        };

        // Standalone second contour as its own path.
        let solo = PathData {
            coord_space: (100.0, 100.0),
            points: tri("b", 50.0, 50.0),
            closed: true,
            subpath_starts: Vec::new(),
        };
        let solo_d = path_data_to_svg_d(&solo, None);

        // Same points as the second contour of a two-contour path.
        let mut points = tri("a", 0.0, 0.0);
        let first_len = points.len();
        points.extend(tri("b", 50.0, 50.0));
        let compound = PathData {
            coord_space: (100.0, 100.0),
            points,
            closed: true,
            subpath_starts: vec![first_len],
        };
        let compound_d = path_data_to_svg_d(&compound, None);

        // The compound output ends with the second contour's `M…Z`; it must match
        // the standalone emission exactly (wrap stayed inside the contour).
        assert!(
            compound_d.ends_with(&solo_d),
            "second contour must match standalone catmull emission.\n solo:     {solo_d}\n compound: {compound_d}"
        );
    }

    #[test]
    fn ellipse_circle_control_points() {
        // An ellipse template with tension = 1 - 4*(√2-1) ≈ -0.6569 should
        // produce control points matching the standard Bézier circle
        // approximation constant κ = 4*(√2-1)/3 ≈ 0.5523.
        //
        // For a 100x100 circle (radius 50, center 50,50):
        //   top→right arc should have:
        //     C1 = (50 + 50*κ, 0)   ≈ (77.61, 0)
        //     C2 = (100, 50 - 50*κ) ≈ (100, 22.39)
        //
        // Derivation: κ = (1 - tension) / 3 (for 4-point cardinal on diameter 2r)
        // → tension = 1 - 3κ = 1 - 4*(√2-1) = 5 - 4√2
        let tension = 1.0 - 4.0 * (std::f64::consts::SQRT_2 - 1.0);
        let data = PathData {
            coord_space: (100.0, 100.0),
            points: vec![
                smooth("top", 50.0, 0.0, tension),
                smooth("right", 100.0, 50.0, tension),
                smooth("bottom", 50.0, 100.0, tension),
                smooth("left", 0.0, 50.0, tension),
            ],
            closed: true,
            subpath_starts: Vec::new(),
        };
        let d = path_data_to_svg_d(&data, None);

        // Parse C1 and C2 from the first curve segment (top → right).
        // Format: "M50 0 C<c1x> <c1y>, <c2x> <c2y>, 100 50 ..."
        let kappa = 4.0 * (std::f64::consts::SQRT_2 - 1.0) / 3.0;
        let expected_c1x = 50.0 + 50.0 * kappa; // ≈ 77.61
        let expected_c1y = 0.0;
        let expected_c2x = 100.0;
        let expected_c2y = 50.0 - 50.0 * kappa; // ≈ 22.39

        // Verify outgoing_control and incoming_control directly.
        let pts = &data.points;
        let (c1x, c1y) = outgoing_control(pts, 0, 4, true, tension);
        let (c2x, c2y) = incoming_control(pts, 1, 4, true, tension);
        assert!(
            (c1x - expected_c1x).abs() < 0.01,
            "c1x: {} vs {}",
            c1x,
            expected_c1x
        );
        assert!(
            (c1y - expected_c1y).abs() < 0.01,
            "c1y: {} vs {}",
            c1y,
            expected_c1y
        );
        assert!(
            (c2x - expected_c2x).abs() < 0.01,
            "c2x: {} vs {}",
            c2x,
            expected_c2x
        );
        assert!(
            (c2y - expected_c2y).abs() < 0.01,
            "c2y: {} vs {}",
            c2y,
            expected_c2y
        );

        // Also verify the SVG path contains C commands (not just L lines).
        assert!(d.starts_with("M50 0"));
        assert!(d.contains('C'));
        assert!(d.ends_with('Z'));
    }
}
