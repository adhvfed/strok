//! Parsing for `place` declarations and their inline overrides.

use super::{
    eval_coord, eval_dimension, eval_rotation, eval_scalar_l, parse_blur, parse_content, parse_err,
    parse_fill, parse_fill_rule, parse_kv_attrs, parse_opacity, parse_skew_value, parse_stroke,
    parse_stroke_dasharray, parse_stroke_linecap, parse_stroke_linejoin, parse_stroke_miterlimit,
    parse_stroke_width, parse_text_anchor, validate_shape_ref, Line,
};
use crate::diagnostics::suggest;
use crate::error::{Result, StrokError};
use crate::expr::Env;
use crate::scene::*;
use crate::types::*;

/// Valid overrides inside a `place` block (for suggestions, E3.1).
const PLACE_OVERRIDES: &[&str] = &[
    "fill",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "opacity",
    "blur",
    "text-anchor",
    "content",
];

/// Valid `key=` attributes on a `place` line. Unknown keys are rejected (with
/// a suggestion) rather than silently ignored — a typo'd `at=`/`size=` used to
/// silently mis-place the element.
const PLACE_KEYS: &[&str] = &[
    "shape", "at", "size", "on", "side", "offset", "rotation", "flip", "skew", "clip", "mask",
    "below", "above", "gap", "align", "textpath", "from", "to", "center", "radius",
];

pub(super) fn parse_place_line(line: &Line, body: &[Line], env: &Env) -> Result<Place> {
    // place <name> shape=<ref> [at=x,y] [on=path.point at=N%] [size=WxH] [rotation=Ndeg] [flip=x|y|xy]
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "place requires a name"));
    }
    let name = line.tokens[1].clone();
    // A place name must be a clean identifier (a stray quote/`=` here means the
    // tokenizer absorbed following attrs into the name, which would not
    // round-trip). Found by `fuzz_roundtrip`.
    crate::types::validate_ident(&name)
        .map_err(|e| parse_err(line, &format!("invalid place name: {e}")))?;
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    // Unknown keys on a place line were historically ignored — which silently
    // mis-places the element (a typo'd `att=` lands the shape at 0,0 fitted to
    // the whole document). Reject them with a suggestion instead.
    for key in attrs.keys() {
        if !PLACE_KEYS.contains(&key.as_str()) {
            let mut d = line.diag(0, format!("unknown place attribute '{}='", key));
            if let Some(s) = suggest(key, PLACE_KEYS) {
                d = d.with_suggestion(s);
            }
            return Err(StrokError::ParseDiagnostics(vec![d]));
        }
    }

    let shape_ref = attrs
        .get("shape")
        .ok_or_else(|| parse_err(line, "place requires shape="))?
        .clone();
    // The shape reference must be a clean identifier, optionally namespaced as
    // `ns.name` (module import). A malformed `shape=...` (embedded quote, `=`,
    // stray chars) would re-emit differently and break round-trip.
    validate_shape_ref(line, &shape_ref)?;

    // Placement sugar — desugared here so everything downstream (resolve,
    // emit, round-trip) sees plain `at` + `size`:
    //   from=x1,y1 to=x2,y2   line endpoints in document space (size may be
    //                          negative/zero — direction is preserved)
    //   center=cx,cy          center-anchored placement (needs size= or radius=)
    //   radius=r | rx,ry      size expressed as radii (size = 2r x 2r)
    let conflict = |keys: &[&str], sugar: &str| -> Result<()> {
        for k in keys {
            if attrs.contains_key(*k) {
                return Err(parse_err(
                    line,
                    &format!("{sugar} cannot be combined with {k}="),
                ));
            }
        }
        Ok(())
    };
    let from_to = match (attrs.get("from"), attrs.get("to")) {
        (Some(f), Some(t)) => {
            conflict(
                &[
                    "at", "size", "on", "center", "radius", "align", "below", "above",
                ],
                "from=/to=",
            )?;
            Some((eval_coord(line, f, env)?, eval_coord(line, t, env)?))
        }
        (None, None) => None,
        _ => {
            return Err(parse_err(
                line,
                "from= and to= must be used together (line endpoints)",
            ))
        }
    };
    let radius = attrs
        .get("radius")
        .map(|v| -> Result<(f64, f64)> {
            let parts: Vec<&str> = v.split(',').collect();
            let parse = |s: &str| -> Result<f64> { eval_scalar_l(line, s, env) };
            match parts.len() {
                1 => {
                    let r = parse(parts[0])?;
                    Ok((r, r))
                }
                2 => Ok((parse(parts[0])?, parse(parts[1])?)),
                _ => Err(parse_err(line, "radius takes r or rx,ry")),
            }
        })
        .transpose()?;
    if radius.is_some() {
        conflict(&["size"], "radius=")?;
    }
    let center = attrs
        .get("center")
        .map(|v| eval_coord(line, v, env))
        .transpose()?;
    if center.is_some() {
        conflict(&["at", "on", "align", "below", "above"], "center=")?;
    }

    let position = if let Some(((x1, y1), _)) = from_to {
        PlacePosition::At(x1, y1)
    } else if let Some((cx, cy)) = center {
        let (w, h) = match (radius, attrs.get("size")) {
            (Some((rx, ry)), _) => (2.0 * rx, 2.0 * ry),
            (None, Some(v)) => {
                let d = eval_dimension(line, v, env)?;
                (d.w, d.h)
            }
            (None, None) => {
                return Err(parse_err(
                    line,
                    "center= needs size= or radius= to determine the extent",
                ))
            }
        };
        PlacePosition::At(cx - w / 2.0, cy - h / 2.0)
    } else if let Some(on_val) = attrs.get("on") {
        let path = PointRef::parse(on_val)?;
        let t_val = attrs
            .get("at")
            .ok_or_else(|| parse_err(line, "on= requires at= for parametric position"))?;
        let t = RelativeSize::parse(t_val)?;
        let side = attrs.get("side").map(|v| Side::parse(v)).transpose()?;
        let offset = attrs
            .get("offset")
            .map(|v| AbsoluteSize::parse(v))
            .transpose()?;
        PlacePosition::On {
            path,
            t,
            side,
            offset,
        }
    } else if let Some(at_val) = attrs.get("at") {
        // A coordinate pair always contains a comma (`x,y`); a `target.anchor`
        // reference never does. Disambiguate on the comma first, otherwise the
        // `.` in a decimal coordinate like `10.5,20.5` is mistaken for the
        // target/anchor separator.
        if at_val.contains(',') {
            let (x, y) = eval_coord(line, at_val, env)?;
            PlacePosition::At(x, y)
        } else if let Some((target, anchor_str)) = at_val.split_once('.') {
            if let Some(anchor) = BboxAnchor::parse(anchor_str) {
                PlacePosition::RelativeTo {
                    target: target.to_string(),
                    anchor,
                }
            } else {
                // Not a valid anchor name — could be an error or a coordinate
                return Err(parse_err(
                    line,
                    &format!(
                        "'{}' is not a valid anchor — use: tl, top, tr, left, center, right, bl, bottom, br",
                        anchor_str
                    ),
                ));
            }
        } else {
            let (x, y) = eval_coord(line, at_val, env)?;
            PlacePosition::At(x, y)
        }
    } else {
        PlacePosition::At(0.0, 0.0)
    };

    let size = if let Some(((x1, y1), (x2, y2))) = from_to {
        Some(Dimension {
            w: x2 - x1,
            h: y2 - y1,
        })
    } else if let Some((rx, ry)) = radius {
        Some(Dimension {
            w: 2.0 * rx,
            h: 2.0 * ry,
        })
    } else {
        attrs
            .get("size")
            .map(|v| eval_dimension(line, v, env))
            .transpose()?
    };
    let rotation = attrs
        .get("rotation")
        .map(|v| eval_rotation(line, v, env))
        .transpose()?;
    let flip = attrs.get("flip").map(|v| Flip::parse(v)).transpose()?;
    let skew = attrs
        .get("skew")
        .map(|v| parse_skew_value(line, v))
        .transpose()?;
    let clip = attrs
        .get("clip")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let mask = attrs.get("mask").cloned();

    // Parse body lines as inline attribute overrides
    let mut overrides = Vec::new();
    for body_line in body {
        if body_line.tokens.is_empty() {
            continue;
        }
        match body_line.tokens[0].as_str() {
            "fill" => overrides.push(parse_fill(body_line)?),
            "fill-rule" => overrides.push(parse_fill_rule(body_line)?),
            "stroke" => overrides.push(parse_stroke(body_line)?),
            "stroke-width" => overrides.push(parse_stroke_width(body_line)?),
            "stroke-linecap" => overrides.push(parse_stroke_linecap(body_line)?),
            "stroke-linejoin" => overrides.push(parse_stroke_linejoin(body_line)?),
            "stroke-miterlimit" => overrides.push(parse_stroke_miterlimit(body_line)?),
            "stroke-dasharray" => overrides.push(parse_stroke_dasharray(body_line)?),
            "opacity" => overrides.push(parse_opacity(body_line)?),
            "blur" => overrides.push(parse_blur(body_line)?),
            "text-anchor" => overrides.push(parse_text_anchor(body_line)?),
            // Per-place text content: lets one text shape serve many labels.
            "content" => overrides.push(parse_content(body_line)?),
            _ => {
                let mut d = body_line.diag(
                    0,
                    format!(
                        "unexpected override '{}' in place block",
                        body_line.tokens[0]
                    ),
                );
                if let Some(s) = suggest(&body_line.tokens[0], PLACE_OVERRIDES) {
                    d = d.with_suggestion(s);
                }
                return Err(StrokError::ParseDiagnostics(vec![d]));
            }
        }
    }

    let anchor = if let Some(target) = attrs.get("below") {
        let gap = attrs
            .get("gap")
            .map(|v| eval_scalar_l(line, v, env))
            .transpose()?
            .unwrap_or(0.0);
        Some(PlaceAnchor::Below {
            target: target.clone(),
            gap,
        })
    } else if let Some(target) = attrs.get("above") {
        let gap = attrs
            .get("gap")
            .map(|v| eval_scalar_l(line, v, env))
            .transpose()?
            .unwrap_or(0.0);
        Some(PlaceAnchor::Above {
            target: target.clone(),
            gap,
        })
    } else {
        None
    };

    let align = if let Some(align_val) = attrs.get("align") {
        Some(BboxAnchor::parse(align_val).ok_or_else(|| {
            parse_err(
                line,
                &format!(
                    "'{}' is not a valid anchor — use: tl, top, tr, left, center, right, bl, bottom, br",
                    align_val
                ),
            )
        })?)
    } else {
        None
    };

    // Only parse offset as (dx,dy) for non-On positions.
    // On positions handle offset= as AbsoluteSize internally.
    let offset = if !matches!(position, PlacePosition::On { .. }) {
        if let Some(offset_val) = attrs.get("offset") {
            Some(eval_coord(line, offset_val, env)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(Place {
        name,
        shape_ref,
        position,
        size,
        rotation,
        flip,
        skew,
        clip,
        mask,
        anchor,
        overrides,
        align,
        offset,
        text_path: attrs.get("textpath").cloned(),
    })
}
