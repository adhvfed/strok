//! Measurement, alignment, and snapping helpers (E2.7).
//!
//! `measure` answers "where are these two elements relative to each other?" —
//! distances, the gaps between their bounding boxes, and per-anchor alignment
//! deltas — using the *same* `element_bboxes` machinery the anchor / RelativeTo
//! resolver uses, so the numbers match what the canvas shows.
//!
//! The `--json` output here is the first stable machine-readable surface; its
//! schema (`MeasureReport::to_json`) is designed so C6's general `--json` helper
//! (E3.2) can reuse it without a breaking change — a single flat object of named
//! scalar fields. See the chunk note: don't duplicate, leave a seam.

use crate::json::Json;
use crate::resolve::element_bboxes;
use crate::scene::Scene;

type Bbox = (f64, f64, f64, f64);

/// The result of `measure <a> <b>`.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureReport {
    pub a: String,
    pub b: String,
    /// Center-to-center distance.
    pub center_distance: f64,
    /// Signed delta between centers (`b.center - a.center`).
    pub dx: f64,
    pub dy: f64,
    /// Gap between boxes on each axis. Positive ⇒ a clear space separates them;
    /// negative ⇒ they overlap by that much along the axis; 0 ⇒ touching.
    pub gap_x: f64,
    pub gap_y: f64,
    /// Whether the two boxes overlap on BOTH axes (i.e. their areas intersect).
    pub overlaps: bool,
    /// Alignment deltas: `b.<edge> - a.<edge>` (0 ⇒ that edge is aligned).
    pub align_left: f64,
    pub align_right: f64,
    pub align_top: f64,
    pub align_bottom: f64,
    pub align_center_x: f64,
    pub align_center_y: f64,
}

/// Compute the measurement between two placed elements by name. Returns the
/// missing name in `Err` so the CLI can give a clean, non-panicking message.
pub fn measure(scene: &Scene, a: &str, b: &str) -> Result<MeasureReport, String> {
    let boxes = element_bboxes(scene);
    let ba = boxes
        .get(a)
        .copied()
        .ok_or_else(|| format!("no placed element named '{}'", a))?;
    let bb = boxes
        .get(b)
        .copied()
        .ok_or_else(|| format!("no placed element named '{}'", b))?;
    Ok(measure_boxes(a, b, ba, bb))
}

fn center(bx: Bbox) -> (f64, f64) {
    ((bx.0 + bx.2) / 2.0, (bx.1 + bx.3) / 2.0)
}

/// Gap between two intervals `[a0,a1]` and `[b0,b1]` on one axis. Positive ⇒
/// clear separation; negative ⇒ overlap depth; 0 ⇒ touching.
fn axis_gap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    if b0 > a1 {
        b0 - a1 // b is to the right/below a
    } else if a0 > b1 {
        a0 - b1 // a is to the right/below b
    } else {
        // Overlapping: report negative overlap depth.
        -(a1.min(b1) - a0.max(b0))
    }
}

pub fn measure_boxes(a: &str, b: &str, ba: Bbox, bb: Bbox) -> MeasureReport {
    let (acx, acy) = center(ba);
    let (bcx, bcy) = center(bb);
    let dx = bcx - acx;
    let dy = bcy - acy;
    let gap_x = axis_gap(ba.0, ba.2, bb.0, bb.2);
    let gap_y = axis_gap(ba.1, ba.3, bb.1, bb.3);
    let overlaps = gap_x < 0.0 && gap_y < 0.0;
    MeasureReport {
        a: a.to_string(),
        b: b.to_string(),
        center_distance: (dx * dx + dy * dy).sqrt(),
        dx,
        dy,
        gap_x,
        gap_y,
        overlaps,
        align_left: bb.0 - ba.0,
        align_right: bb.2 - ba.2,
        align_top: bb.1 - ba.1,
        align_bottom: bb.3 - ba.3,
        align_center_x: bcx - acx,
        align_center_y: bcy - acy,
    }
}

/// Render a number for JSON: trims to 6 significant digits and drops a trailing
/// `.0` so integers stay integers (deterministic, snapshot-stable).
fn jnum(v: f64) -> String {
    crate::types::fmt_num(v)
}

impl MeasureReport {
    /// The measurement as a [`Json`] value (C6 / E3.2). The flat-object shape is
    /// unchanged from the C5 seam — C6 generalized the *formatting* (via the
    /// shared [`crate::json`] helper) without breaking the schema.
    pub fn to_json_value(&self) -> Json {
        Json::obj([
            ("a", Json::str(&self.a)),
            ("b", Json::str(&self.b)),
            ("center_distance", Json::num(self.center_distance)),
            ("dx", Json::num(self.dx)),
            ("dy", Json::num(self.dy)),
            ("gap_x", Json::num(self.gap_x)),
            ("gap_y", Json::num(self.gap_y)),
            ("overlaps", Json::Bool(self.overlaps)),
            ("align_left", Json::num(self.align_left)),
            ("align_right", Json::num(self.align_right)),
            ("align_top", Json::num(self.align_top)),
            ("align_bottom", Json::num(self.align_bottom)),
            ("align_center_x", Json::num(self.align_center_x)),
            ("align_center_y", Json::num(self.align_center_y)),
        ])
    }

    /// Stable, machine-readable JSON. A single flat object of named scalar
    /// fields — the seam C6's general `--json` helper (E3.2) now backs.
    pub fn to_json(&self) -> String {
        self.to_json_value().to_string_pretty()
    }

    /// Human-readable one-block summary for the default (non-JSON) CLI output.
    pub fn to_text(&self) -> String {
        format!(
            "measure {} → {}\n  \
             center distance: {}\n  \
             delta (b - a):   dx={} dy={}\n  \
             gap:             x={} y={}\n  \
             overlaps:        {}\n  \
             align Δ (b - a): left={} right={} top={} bottom={} cx={} cy={}\n",
            self.a,
            self.b,
            jnum(self.center_distance),
            jnum(self.dx),
            jnum(self.dy),
            jnum(self.gap_x),
            jnum(self.gap_y),
            self.overlaps,
            jnum(self.align_left),
            jnum(self.align_right),
            jnum(self.align_top),
            jnum(self.align_bottom),
            jnum(self.align_center_x),
            jnum(self.align_center_y),
        )
    }
}

/// Snap target for `snap` placement helpers (E2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    /// Round to the nearest multiple of the grid step.
    Grid,
    /// Snap to the nearest edge of the document (0 or w/h) per axis.
    Edge,
    /// Snap to the document center per axis.
    Center,
}

impl SnapMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "grid" => Some(Self::Grid),
            "edge" => Some(Self::Edge),
            "center" => Some(Self::Center),
            _ => None,
        }
    }
}

/// Snap a point to a grid/edge/center reference within a `w`×`h` canvas.
///
/// - `Grid`: round each axis to the nearest multiple of `step` (step ≤ 0 ⇒ no-op).
/// - `Edge`: snap each axis to whichever of `0` / `w` (resp. `h`) is nearer.
/// - `Center`: snap each axis to `w/2` (resp. `h/2`).
pub fn snap_point(p: (f64, f64), mode: SnapMode, step: f64, w: f64, h: f64) -> (f64, f64) {
    match mode {
        SnapMode::Grid => {
            if step <= 0.0 {
                p
            } else {
                ((p.0 / step).round() * step, (p.1 / step).round() * step)
            }
        }
        SnapMode::Edge => {
            let sx = if (p.0 - 0.0).abs() <= (p.0 - w).abs() {
                0.0
            } else {
                w
            };
            let sy = if (p.1 - 0.0).abs() <= (p.1 - h).abs() {
                0.0
            } else {
                h
            };
            (sx, sy)
        }
        SnapMode::Center => (w / 2.0, h / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_separated_touching_overlapping() {
        // separated: a=[0,10], b=[20,30] → gap 10
        assert_eq!(axis_gap(0.0, 10.0, 20.0, 30.0), 10.0);
        // touching: a=[0,10], b=[10,20] → gap 0
        assert_eq!(axis_gap(0.0, 10.0, 10.0, 20.0), 0.0);
        // overlapping by 4: a=[0,10], b=[6,20] → -4
        assert_eq!(axis_gap(0.0, 10.0, 6.0, 20.0), -4.0);
    }

    #[test]
    fn measure_matches_hand_computed() {
        // a: 0,0..10,10 (center 5,5). b: 20,0..30,10 (center 25,5).
        let r = measure_boxes("a", "b", (0.0, 0.0, 10.0, 10.0), (20.0, 0.0, 30.0, 10.0));
        assert_eq!(r.dx, 20.0);
        assert_eq!(r.dy, 0.0);
        assert_eq!(r.center_distance, 20.0);
        assert_eq!(r.gap_x, 10.0);
        // boxes overlap vertically (same y span) → gap_y is -10 (full overlap depth)
        assert_eq!(r.gap_y, -10.0);
        assert!(!r.overlaps); // separated on x
        assert_eq!(r.align_top, 0.0); // same top
        assert_eq!(r.align_left, 20.0);
    }

    #[test]
    fn overlap_both_axes_flags_true() {
        let r = measure_boxes("a", "b", (0.0, 0.0, 10.0, 10.0), (5.0, 5.0, 15.0, 15.0));
        assert!(r.overlaps);
    }

    #[test]
    fn snap_grid_edge_center() {
        assert_eq!(
            snap_point((13.0, 27.0), SnapMode::Grid, 8.0, 100.0, 100.0),
            (16.0, 24.0)
        );
        assert_eq!(
            snap_point((10.0, 90.0), SnapMode::Edge, 0.0, 100.0, 100.0),
            (0.0, 100.0)
        );
        assert_eq!(
            snap_point((10.0, 90.0), SnapMode::Center, 0.0, 100.0, 100.0),
            (50.0, 50.0)
        );
        // grid step 0 ⇒ no-op
        assert_eq!(
            snap_point((13.0, 27.0), SnapMode::Grid, 0.0, 100.0, 100.0),
            (13.0, 27.0)
        );
    }
}
