use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::path_point::PathData;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Paint {
    Color(String),
    None,
}

/// Affine transform matrix [a, b, c, d, e, f] mapping to:
/// | a c e |
/// | b d f |
/// | 0 0 1 |
pub type Transform = [f64; 6];

pub const IDENTITY: Transform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

// ── Affine algebra (E2.3) ───────────────────────────────────────────────
//
// The 6-tuple `[a, b, c, d, e, f]` is the single affine representation carried
// through placement / group composition (the unification E1.5a/E2.3 calls for).
// Helpers here are pure and side-effect-free so they can back both the SVG
// `transform=` emit and the transform-aware bbox (so anchors/`RelativeTo` work
// under rotation/skew — fixing the proven `placed_bbox` pre-transform gap).

/// Compose two affines: `mul(a, b)` is "apply `b` first, then `a`"
/// (standard matrix product `a * b`).
pub fn mul(a: &Transform, b: &Transform) -> Transform {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

/// Apply an affine to a point.
pub fn apply(t: &Transform, x: f64, y: f64) -> (f64, f64) {
    (t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5])
}

/// `translate(tx, ty)`.
pub fn translate(tx: f64, ty: f64) -> Transform {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

/// `scale(sx, sy)`.
pub fn scale(sx: f64, sy: f64) -> Transform {
    [sx, 0.0, 0.0, sy, 0.0, 0.0]
}

/// `rotate(deg)` about the origin.
pub fn rotate(deg: f64) -> Transform {
    let r = deg.to_radians();
    let (s, c) = (r.sin(), r.cos());
    [c, s, -s, c, 0.0, 0.0]
}

/// `rotate(deg, cx, cy)` about an arbitrary pivot.
pub fn rotate_about(deg: f64, cx: f64, cy: f64) -> Transform {
    mul(&translate(cx, cy), &mul(&rotate(deg), &translate(-cx, -cy)))
}

/// `skewX(deg) · skewY(deg)`. Either angle may be zero.
pub fn skew(deg_x: f64, deg_y: f64) -> Transform {
    let tx = deg_x.to_radians().tan();
    let ty = deg_y.to_radians().tan();
    [1.0, ty, tx, 1.0, 0.0, 0.0]
}

/// Skew about an arbitrary pivot (so a place's `skew=` keeps its anchor fixed).
pub fn skew_about(deg_x: f64, deg_y: f64, cx: f64, cy: f64) -> Transform {
    mul(
        &translate(cx, cy),
        &mul(&skew(deg_x, deg_y), &translate(-cx, -cy)),
    )
}

/// The axis-aligned bounding box of the four corners of `(x0,y0)-(x1,y1)` after
/// applying `t`. This is the transform-aware bbox: a rotated/skewed rectangle's
/// bbox is the AABB of its mapped corners, NOT the mapped AABB.
pub fn transform_bbox(t: &Transform, x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64, f64, f64) {
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (px, py) in corners {
        let (tx, ty) = apply(t, px, py);
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    (min_x, min_y, max_x, max_y)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Attrs {
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub stroke_width: Option<f64>,
    pub opacity: Option<f64>,
    pub transform: Option<Transform>,

    // Geometry
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub cx: Option<f64>,
    pub cy: Option<f64>,
    pub r: Option<f64>,
    pub rx: Option<f64>,
    pub ry: Option<f64>,
    pub x1: Option<f64>,
    pub y1: Option<f64>,
    pub x2: Option<f64>,
    pub y2: Option<f64>,
    pub d: Option<String>,
    pub points: Option<String>,

    // Text
    pub font_size: Option<f64>,
    pub font_family: Option<String>,
    pub font_weight: Option<String>,
    pub text_anchor: Option<String>,
    pub text_content: Option<String>,

    // DSL path data (authoritative for DSL-defined paths; fallback to `d` for SVG imports)
    #[serde(skip)]
    pub path_data: Option<PathData>,

    // Image
    pub href: Option<String>,

    // viewBox for root SVG
    pub view_box: Option<String>,

    /// Pass-through attributes not explicitly modeled
    pub extra: HashMap<String, String>,
}

impl Attrs {
    pub fn set_from_svg(&mut self, name: &str, value: &str) {
        match name {
            "fill" => {
                self.fill = Some(if value == "none" {
                    Paint::None
                } else {
                    Paint::Color(value.to_string())
                })
            }
            "stroke" => {
                self.stroke = Some(if value == "none" {
                    Paint::None
                } else {
                    Paint::Color(value.to_string())
                })
            }
            "stroke-width" => self.stroke_width = value.parse().ok(),
            "opacity" => self.opacity = value.parse().ok(),
            "x" => self.x = value.parse().ok(),
            "y" => self.y = value.parse().ok(),
            "width" => self.width = value.parse().ok(),
            "height" => self.height = value.parse().ok(),
            "cx" => self.cx = value.parse().ok(),
            "cy" => self.cy = value.parse().ok(),
            "r" => self.r = value.parse().ok(),
            "rx" => self.rx = value.parse().ok(),
            "ry" => self.ry = value.parse().ok(),
            "x1" => self.x1 = value.parse().ok(),
            "y1" => self.y1 = value.parse().ok(),
            "x2" => self.x2 = value.parse().ok(),
            "y2" => self.y2 = value.parse().ok(),
            "d" => self.d = Some(value.to_string()),
            "points" => self.points = Some(value.to_string()),
            "font-size" => self.font_size = value.parse().ok(),
            "font-family" => self.font_family = Some(value.to_string()),
            "font-weight" => self.font_weight = Some(value.to_string()),
            "text-anchor" => self.text_anchor = Some(value.to_string()),
            "href" | "xlink:href" => self.href = Some(value.to_string()),
            "viewBox" => self.view_box = Some(value.to_string()),
            "transform" => self.transform = parse_transform(value),
            "id" | "xmlns" | "xmlns:xlink" => {} // skip these
            _ => {
                self.extra.insert(name.to_string(), value.to_string());
            }
        }
    }
}

fn parse_transform(s: &str) -> Option<Transform> {
    // Handle translate(x, y)
    if let Some(inner) = s
        .strip_prefix("translate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<f64> = inner
            .split([',', ' '])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        return match parts.len() {
            1 => Some([1.0, 0.0, 0.0, 1.0, parts[0], 0.0]),
            2.. => Some([1.0, 0.0, 0.0, 1.0, parts[0], parts[1]]),
            _ => None,
        };
    }
    // Handle rotate(deg) or rotate(deg, cx, cy)
    if let Some(inner) = s.strip_prefix("rotate(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split([',', ' '])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if let Some(&deg) = parts.first() {
            let rad = deg.to_radians();
            let (sin, cos) = (rad.sin(), rad.cos());
            if parts.len() >= 3 {
                let (cx, cy) = (parts[1], parts[2]);
                return Some([
                    cos,
                    sin,
                    -sin,
                    cos,
                    cx - cos * cx + sin * cy,
                    cy - sin * cx - cos * cy,
                ]);
            }
            return Some([cos, sin, -sin, cos, 0.0, 0.0]);
        }
    }
    // Handle scale(s) or scale(sx, sy)
    if let Some(inner) = s.strip_prefix("scale(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split([',', ' '])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        return match parts.len() {
            1 => Some([parts[0], 0.0, 0.0, parts[0], 0.0, 0.0]),
            2.. => Some([parts[0], 0.0, 0.0, parts[1], 0.0, 0.0]),
            _ => None,
        };
    }
    // Handle matrix(a,b,c,d,e,f)
    if let Some(inner) = s.strip_prefix("matrix(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split([',', ' '])
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() == 6 {
            return Some([parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]]);
        }
    }
    None
}

pub fn emit_transform(t: &Transform) -> String {
    if *t == IDENTITY {
        return String::new();
    }
    // Check for simple translate
    if t[0] == 1.0 && t[1] == 0.0 && t[2] == 0.0 && t[3] == 1.0 {
        return format!("translate({}, {})", t[4], t[5]);
    }
    format!(
        "matrix({}, {}, {}, {}, {}, {})",
        t[0], t[1], t[2], t[3], t[4], t[5]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_neutral() {
        let t = [2.0, 0.5, -0.3, 1.5, 7.0, -4.0];
        assert_eq!(mul(&IDENTITY, &t), t);
        assert_eq!(mul(&t, &IDENTITY), t);
    }

    #[test]
    fn translate_then_apply() {
        let t = translate(5.0, -3.0);
        assert_eq!(apply(&t, 1.0, 2.0), (6.0, -1.0));
    }

    #[test]
    fn rotate_90_maps_axes() {
        let t = rotate(90.0);
        let (x, y) = apply(&t, 1.0, 0.0);
        assert!(
            (x - 0.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9,
            "({x},{y})"
        );
    }

    #[test]
    fn skew_x_shears() {
        // skewX(45deg): x' = x + tan(45)*y = x + y.
        let t = skew(45.0, 0.0);
        let (x, y) = apply(&t, 0.0, 2.0);
        assert!(
            (x - 2.0).abs() < 1e-9 && (y - 2.0).abs() < 1e-9,
            "({x},{y})"
        );
    }

    #[test]
    fn rotate_about_keeps_pivot_fixed() {
        let t = rotate_about(37.0, 10.0, 20.0);
        let (x, y) = apply(&t, 10.0, 20.0);
        assert!((x - 10.0).abs() < 1e-9 && (y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn transform_bbox_of_rotated_square() {
        // A unit square rotated 45° about origin: bbox is [-√2/2, √2/2] each axis.
        let t = rotate(45.0);
        let (x0, y0, x1, y1) = transform_bbox(&t, -0.5, -0.5, 0.5, 0.5);
        let half = std::f64::consts::FRAC_1_SQRT_2; // √2/2
        assert!((x0 + half).abs() < 1e-9, "x0 {x0}");
        assert!((y0 + half).abs() < 1e-9, "y0 {y0}");
        assert!((x1 - half).abs() < 1e-9, "x1 {x1}");
        assert!((y1 - half).abs() < 1e-9, "y1 {y1}");
    }

    #[test]
    fn compose_apply_order() {
        // mul(a, b) applies b first: scale then translate.
        let t = mul(&translate(10.0, 0.0), &scale(2.0, 2.0));
        assert_eq!(apply(&t, 3.0, 0.0), (16.0, 0.0));
    }
}
