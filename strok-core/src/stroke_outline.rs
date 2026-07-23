//! Stroke→outline and path offset/inset (E2.2).
//!
//! Both reuse the C3 geometry stack: `kurbo` (in-tree) for the stroke
//! tessellation + bézier flattening, and `i_overlay` (D-1) to clean the result
//! into outer+hole contours. Output is a flattened-polygon `path` `Shape`
//! authored in document space (placed `at=0,0`, identity) so it round-trips,
//! renders, and re-edits like any hand-authored path.
//!
//! - `outline-stroke <id>`: convert a stroked path into the *filled* region the
//!   stroke paints, honoring width / caps / joins / miter.
//! - `offset <id> <delta>`: grow (δ>0) or shrink (δ<0) the filled region by δ.
//!   Implemented as: stroke the boundary into a band of width `2·|δ|`, then
//!   `Union` (grow) / `Difference` (shrink) it with the original — the same
//!   recipe verified in the D-1 spike (a circle offset by r → concentric circle
//!   within flatten ε).

use crate::bool_ops;
use crate::error::{Result, StrokError};
use crate::path_point::PathData;
use crate::shape::Shape;
use crate::types::{LineCap, LineJoin};

use i_overlay::core::fill_rule::FillRule as IFillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use kurbo::{stroke, BezPath, Cap, Join, PathEl, Stroke};

type Pt = [f64; 2];
type Contour = Vec<Pt>;
type Shapes = Vec<Vec<Contour>>;

const FLATTEN_TOL: f64 = 0.05;

/// Stroke style for `outline-stroke`, mirrored from the resolved shape attrs.
#[derive(Debug, Clone)]
pub struct StrokeStyle {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        StrokeStyle {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
        }
    }
}

fn to_kurbo_cap(c: LineCap) -> Cap {
    match c {
        LineCap::Butt => Cap::Butt,
        LineCap::Round => Cap::Round,
        LineCap::Square => Cap::Square,
    }
}

fn to_kurbo_join(j: LineJoin) -> Join {
    match j {
        LineJoin::Miter => Join::Miter,
        LineJoin::Round => Join::Round,
        LineJoin::Bevel => Join::Bevel,
    }
}

/// Convert a document-space `d` path into the filled outline of stroking it with
/// `style`. Returns a cleaned (outer+holes) contour-set.
pub fn outline_stroke_d(d: &str, style: &StrokeStyle) -> Result<Shapes> {
    let bez = BezPath::from_svg(d)
        .map_err(|e| StrokError::InvalidOperation(format!("invalid path geometry: {}", e)))?;
    let k = Stroke::new(style.width.max(0.0))
        .with_caps(to_kurbo_cap(style.cap))
        .with_join(to_kurbo_join(style.join))
        .with_miter_limit(style.miter_limit.max(1.0));
    let outline = stroke(bez.iter(), &k, &Default::default(), FLATTEN_TOL);
    let shapes = bezpath_to_shapes(&outline);
    // Clean overlaps (e.g. self-touching caps) into a well-formed region.
    Ok(normalize(&shapes))
}

/// Offset (grow/shrink) the filled region of `d` by `delta` document units.
/// Positive grows outward, negative shrinks inward. Returns a cleaned contour-set.
pub fn offset_d(d: &str, delta: f64) -> Result<Shapes> {
    let bez = BezPath::from_svg(d)
        .map_err(|e| StrokError::InvalidOperation(format!("invalid path geometry: {}", e)))?;
    let original = bool_ops::svg_d_to_shapes(d);
    if delta.abs() < 1e-9 {
        return Ok(normalize(&original));
    }
    // Band of width 2·|δ| centered on the boundary, round joins/caps so corners
    // grow/shrink smoothly (matching the canonical Minkowski offset of a disk).
    let band_style = Stroke::new(2.0 * delta.abs())
        .with_caps(Cap::Round)
        .with_join(Join::Round);
    let band = stroke(bez.iter(), &band_style, &Default::default(), FLATTEN_TOL);
    let band_shapes = bezpath_to_shapes(&band);

    let subj = flatten(&original);
    let clip = flatten(&band_shapes);
    let rule = if delta > 0.0 {
        OverlayRule::Union
    } else {
        OverlayRule::Difference
    };
    let res = subj.overlay(&clip, rule, IFillRule::NonZero);
    Ok(res)
}

/// Normalize a contour-set through a self-overlay (NonZero) so overlapping /
/// self-touching pieces become clean outer+hole contours.
fn normalize(shapes: &Shapes) -> Shapes {
    let subj = flatten(shapes);
    let empty: Vec<Contour> = Vec::new();
    subj.overlay(&empty, OverlayRule::Subject, IFillRule::NonZero)
}

fn flatten(shapes: &Shapes) -> Vec<Contour> {
    shapes.iter().flatten().cloned().collect()
}

/// Flatten a kurbo `BezPath` into closed polygon contours.
fn bezpath_to_shapes(bez: &BezPath) -> Shapes {
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
    if contours.is_empty() {
        Vec::new()
    } else {
        vec![contours]
    }
}

/// Build the resulting `path` `Shape` for an outline/offset op.
pub fn shapes_to_shape(name: &str, shapes: &Shapes) -> Shape {
    bool_ops::shapes_to_shape(name, shapes, IFillRule::NonZero)
}

/// Helper for callers/tests: document-space `PathData` from contours.
pub fn shapes_to_pathdata(shapes: &Shapes) -> PathData {
    bool_ops::shapes_to_pathdata(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bool_ops::area_of;

    #[test]
    fn outline_round_cap_line_area() {
        // A 100-long horizontal line, width 10, round caps → ≈ 100·10 + π·5² caps.
        let d = "M0 0 L100 0";
        let style = StrokeStyle {
            width: 10.0,
            cap: LineCap::Round,
            join: LineJoin::Round,
            miter_limit: 4.0,
        };
        let res = outline_stroke_d(d, &style).unwrap();
        let a = area_of(&res);
        let expect = 100.0 * 10.0 + std::f64::consts::PI * 25.0;
        assert!(
            (a - expect).abs() / expect < 0.02,
            "outline area {a} vs {expect}"
        );
    }

    #[test]
    fn offset_circle_grows_concentric() {
        use kurbo::{Circle, Shape as _};
        let c = Circle::new((50.0, 50.0), 10.0);
        let d = c.to_path(0.01).to_svg();
        let res = offset_d(&d, 5.0).unwrap();
        let a = area_of(&res);
        let expect = std::f64::consts::PI * 15.0 * 15.0;
        assert!(
            (a - expect).abs() / expect < 0.03,
            "offset+5 area {a} vs {expect}"
        );
    }

    #[test]
    fn inset_circle_shrinks_concentric() {
        use kurbo::{Circle, Shape as _};
        let c = Circle::new((50.0, 50.0), 10.0);
        let d = c.to_path(0.01).to_svg();
        let res = offset_d(&d, -4.0).unwrap();
        let a = area_of(&res);
        let expect = std::f64::consts::PI * 6.0 * 6.0;
        assert!(
            (a - expect).abs() / expect < 0.05,
            "offset-4 area {a} vs {expect}"
        );
    }

    #[test]
    fn offset_zero_is_identity_area() {
        let d = "M0 0 L20 0 L20 20 L0 20Z";
        let res = offset_d(d, 0.0).unwrap();
        assert!((area_of(&res) - 400.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate_no_panic() {
        for d in ["M0 0", "M0 0 L1 1"] {
            let _ = offset_d(d, 3.0);
            let _ = outline_stroke_d(d, &StrokeStyle::default());
        }
    }

    #[test]
    fn invalid_path_errors() {
        assert!(offset_d("not a path", 3.0).is_err());
        assert!(outline_stroke_d("xyzzy", &StrokeStyle::default()).is_err());
    }
}
