/// Analyze a scene and suggest structural and visual-safety improvements.
///
/// Findings teach reusable composition primitives, catch known curve traps, and
/// surface high-confidence label-placement mistakes with concrete rewrites.
use crate::json::Json;
use crate::path_point::CurveMode;
use crate::scene::*;
use crate::types::fmt_num;

use std::collections::HashMap;

/// A single audit finding.
#[derive(Debug, Clone)]
pub struct Finding {
    pub kind: FindingKind,
    pub message: String,
    pub detail: String,
    /// A concrete, copy-pasteable fix when one exists (C10 / E5.3 — makes the
    /// `RoughCatmull` finding *actionable* by naming the right primitive now that
    /// P2 shipped per-corner radius / `mode=arc` / notch+tail). Empty when the
    /// finding has no single mechanical fix.
    pub suggestion: String,
    pub line_savings: usize,
}

/// What type of finding.
#[derive(Debug, Clone, PartialEq)]
pub enum FindingKind {
    NearMirror,
    UnusedComposition,
    RoughCatmull,
    /// Multiple isolated catmull-rom segments in a closed contour. Because a
    /// point mode describes the segment arriving at that point, this is often a
    /// mistaken attempt to smooth symmetric corners on both sides.
    IsolatedCatmull,
    /// N≥3 places of the same shape whose positions form an arithmetic
    /// progression — a `repeat` block waiting to happen (EXP-2).
    RepeatedPlaceRhythm,
    /// ≥2 groups with structurally identical children differing only in
    /// position/fill — define-once-and-reuse (EXP-2).
    NearDuplicateGroups,
    /// The same non-trivial number repeated ≥4× across place coords/sizes in a
    /// document with no `let` bindings — a missing named constant (EXP-2).
    MagicNumberRhythm,
    /// Two absolutely-placed elements that are edge-aligned or gap-adjacent but
    /// use raw `at=` instead of a relative anchor (EXP-2).
    UnanchoredAdjacency,
    /// A text run partially intersects a closed geometric element. Text fully
    /// contained by a shape is treated as an intentional label; paths and lines
    /// are ignored because their bounding boxes are too coarse for this check.
    TextCollision,
    /// A single text run is visually centered inside a closed shape but uses a
    /// guessed absolute SVG baseline instead of attaching to the host's center.
    UnanchoredLabel,
}

impl FindingKind {
    /// Stable machine-readable id for `--json` (snapshot-stable).
    pub fn id(&self) -> &'static str {
        match self {
            FindingKind::NearMirror => "near_mirror",
            FindingKind::UnusedComposition => "unused_composition",
            FindingKind::RoughCatmull => "rough_catmull",
            FindingKind::IsolatedCatmull => "isolated_catmull",
            FindingKind::RepeatedPlaceRhythm => "repeated_place_rhythm",
            FindingKind::NearDuplicateGroups => "near_duplicate_groups",
            FindingKind::MagicNumberRhythm => "magic_number_rhythm",
            FindingKind::UnanchoredAdjacency => "unanchored_adjacency",
            FindingKind::TextCollision => "text_collision",
            FindingKind::UnanchoredLabel => "unanchored_label",
        }
    }
}

impl Finding {
    /// One finding as a JSON object (C10 / E5.3 — `audit --json`, deferred here
    /// from C6). Built on the shared `json` value builder so the schema is
    /// snapshot-stable, like every other `--json` surface.
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("kind", Json::str(self.kind.id())),
            ("message", Json::str(&self.message)),
            ("detail", Json::str(&self.detail)),
            ("suggestion", Json::str(&self.suggestion)),
            ("line_savings", Json::num(self.line_savings as f64)),
        ])
    }
}

/// Build the `audit --json` document: a stable wrapper around the findings list
/// plus a small summary. One helper so the schema is snapshot-tested in one place.
pub fn findings_to_json(findings: &[Finding]) -> Json {
    let total_savings: usize = findings.iter().map(|f| f.line_savings).sum();
    Json::obj([
        ("count", Json::num(findings.len() as f64)),
        ("total_line_savings", Json::num(total_savings as f64)),
        (
            "findings",
            Json::array(findings.iter().map(Finding::to_json)),
        ),
    ])
}

/// Run all audit checks on a scene.
pub fn audit(scene: &Scene) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mirrors = detect_mirrors(scene);
    let mirror_count = mirrors.len();
    findings.extend(mirrors);
    findings.extend(composition_summary(scene, mirror_count));
    findings.extend(detect_rough_catmull(scene));
    findings.extend(detect_isolated_catmull(scene));
    // EXP-2 — compositional findings: the tool teaching its own idioms
    // (repeat / define-once / let / relative anchors).
    findings.extend(detect_repeated_rhythm(scene));
    findings.extend(detect_duplicate_groups(scene));
    findings.extend(detect_magic_numbers(scene));
    findings.extend(detect_unanchored_adjacency(scene));
    findings.extend(detect_text_collisions(scene));
    findings.extend(detect_unanchored_labels(scene));
    findings
}

// ── EXP-2 shared helpers ──────────────────────────────────────────────

/// Absolute `at=x,y` of a place, or `None` for relative/parametric placements.
fn place_at(p: &Place) -> Option<(f64, f64)> {
    match p.position {
        PlacePosition::At(x, y) => Some((x, y)),
        _ => None,
    }
}

/// The authored `size=WxH` of a place, if any.
fn place_size(p: &Place) -> Option<(f64, f64)> {
    p.size.as_ref().map(|d| (d.w, d.h))
}

/// Places directly at this node level (does NOT descend into groups/frames).
fn direct_places(nodes: &[SceneNode]) -> Vec<&Place> {
    nodes
        .iter()
        .filter_map(|n| match n {
            SceneNode::Place(p) => Some(p),
            _ => None,
        })
        .collect()
}

/// Whether the scene reads as a structured-UI / layout document (uses design
/// tokens, components, or frames) rather than a freehand illustration. The two
/// "extract a named constant / use a relative anchor" findings only produce
/// *sensible* advice on layout documents — an illustration reuses coordinates
/// and abuts elements by coincidence constantly — so they are gated on this.
fn looks_like_layout(scene: &Scene) -> bool {
    !scene.design_tokens.is_empty() || !scene.components.is_empty() || has_frame(&scene.nodes)
}

fn has_frame(nodes: &[SceneNode]) -> bool {
    nodes.iter().any(|n| match n {
        SceneNode::Frame(_) => true,
        SceneNode::Group(g) => has_frame(&g.children),
        _ => false,
    })
}

/// `true` when every value equals the first within `eps`.
fn all_equal(vals: &[f64], eps: f64) -> bool {
    match vals.first() {
        Some(&first) => vals.iter().all(|v| (v - first).abs() <= eps),
        None => true,
    }
}

/// The common step of an arithmetic sequence, if the consecutive deltas are all
/// equal within `eps` (and there are ≥2 values). `None` otherwise.
fn arithmetic_gap(vals: &[f64], eps: f64) -> Option<f64> {
    if vals.len() < 2 {
        return None;
    }
    let g = vals[1] - vals[0];
    for w in vals.windows(2) {
        if (w[1] - w[0] - g).abs() > eps {
            return None;
        }
    }
    Some(g)
}

// ── Rough catmull-rom detection ───────────────────────────────────────

/// Flag `catmull-rom` runs that thread through near-collinear points — the exact
/// trap that produces wavy / faceted edges on geometry that should be straight or
/// a clean arc. For each interior point that is catmull-rom (and whose neighbors
/// form part of the same smoothed run) we measure how close the triple
/// prev→curr→next is to a straight line; a small perpendicular deviation relative
/// to the span means the smoothing is buying nothing but roughness.
fn detect_rough_catmull(scene: &Scene) -> Vec<Finding> {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut findings = Vec::new();

    for shape in &scene.shapes {
        let pd = shape.resolve(coord_space);
        let n = pd.points.len();
        if n < 3 {
            continue;
        }
        let mut collinear_hits = 0usize;
        // Interior points only (collinearity needs a prev and a next).
        for i in 1..n - 1 {
            let curr = &pd.points[i];
            if !matches!(curr.mode, CurveMode::CatmullRom(_)) {
                continue;
            }
            let prev = &pd.points[i - 1];
            let next = &pd.points[i + 1];
            let ax = next.x - prev.x;
            let ay = next.y - prev.y;
            let span = (ax * ax + ay * ay).sqrt();
            if span < 1e-6 {
                continue;
            }
            // Perpendicular distance of curr from the prev→next chord.
            let cross = ((curr.x - prev.x) * ay - (curr.y - prev.y) * ax).abs();
            let perp = cross / span;
            // Near-collinear: the point barely leaves the chord (< ~2% of the
            // chord length). That is where catmull-rom wobble is visible.
            if perp / span < 0.02 {
                collinear_hits += 1;
            }
        }

        if collinear_hits >= 2 {
            findings.push(Finding {
                kind: FindingKind::RoughCatmull,
                message: format!(
                    "'{}' uses catmull-rom through {} near-collinear points",
                    shape.name, collinear_hits,
                ),
                detail: "catmull-rom through near-straight points causes wavy/faceted edges — \
                     the smoothing buys nothing but roughness on a run that's effectively straight"
                    .to_string(),
                // Actionable now that P2 (C3–C5) shipped real primitives: name the
                // exact replacement instead of a vague "use a curve". (E5.3)
                suggestion: format!(
                    "rewrite '{}': set the collinear points to mode=sharp (straight run); \
                     for a true curve use mode=arc; for a rounded box corner use \
                     round-corners (per-corner tl=/tr=/br=/bl= supported); for a tab/tail \
                     use the notch primitive",
                    shape.name,
                ),
                line_savings: 0,
            });
        }
    }

    findings
}

/// Flag catmull-rom segments that have no catmull-rom neighbour. Modes on
/// `NamedPoint` are segment modes: the mode at point B controls A→B. A common
/// authoring mistake is therefore `smooth right; smooth left` on a closed,
/// symmetric outline, expecting both corners to become smooth. It actually
/// curves two alternating incoming segments and leaves their outgoing segments
/// sharp, which creates the asymmetric shoulder seen in the shield regression.
fn detect_isolated_catmull(scene: &Scene) -> Vec<Finding> {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut findings = Vec::new();

    for shape in &scene.shapes {
        let pd = shape.resolve(coord_space);
        let n = pd.points.len();
        // A single curved segment among straight ones is useful and common in
        // open paths. This check targets the much stronger closed-contour smell:
        // multiple disconnected smooth segments (often an alternating pattern).
        if !pd.closed || n < 4 {
            continue;
        }

        let is_smooth = |i: usize| matches!(pd.points[i].mode, CurveMode::CatmullRom(_));
        let mut isolated = Vec::new();
        for i in 0..n {
            if !is_smooth(i) {
                continue;
            }
            let prev_smooth = if i > 0 {
                is_smooth(i - 1)
            } else {
                pd.closed && is_smooth(n - 1)
            };
            let next_smooth = if i + 1 < n {
                is_smooth(i + 1)
            } else {
                pd.closed && is_smooth(0)
            };
            if !prev_smooth && !next_smooth {
                isolated.push(pd.points[i].name.clone());
            }
        }

        if isolated.len() >= 2 {
            findings.push(Finding {
                kind: FindingKind::IsolatedCatmull,
                message: format!(
                    "'{}' has isolated catmull-rom segments at {}",
                    shape.name,
                    isolated.join(", "),
                ),
                detail: "a point's curve mode controls only the segment arriving at that point; an isolated `smooth p` does not smooth both sides of the anchor and can introduce a kink or asymmetric outline".to_string(),
                suggestion: format!(
                    "rewrite '{}': use smooth-corner <point> when the intent is to smooth both sides of an anchor; otherwise smooth every consecutive segment endpoint in the intended curve run, or use mode=arc / round-corners for a geometric corner",
                    shape.name,
                ),
                line_savings: 0,
            });
        }
    }

    findings
}

// ── Mirror detection ──────────────────────────────────────────────────

/// Suffix pairs to check for mirror naming conventions.
const MIRROR_SUFFIXES: &[(&str, &str)] = &[("-l", "-r"), ("-left", "-right")];

fn detect_mirrors(scene: &Scene) -> Vec<Finding> {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut findings = Vec::new();

    // Group placed nodes by their base name (stripping mirror suffixes)
    let mut pairs: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();

    for node in collect_all_places(&scene.nodes) {
        let name = &node.name;
        for &(left_suffix, right_suffix) in MIRROR_SUFFIXES {
            if let Some(base) = name.strip_suffix(left_suffix) {
                let entry = pairs.entry(base.to_string()).or_insert((None, None));
                entry.0 = Some(name.clone());
            } else if let Some(base) = name.strip_suffix(right_suffix) {
                let entry = pairs.entry(base.to_string()).or_insert((None, None));
                entry.1 = Some(name.clone());
            }
        }
    }

    // For each pair, compare geometry
    for (left_name, right_name) in pairs.values() {
        let (left_name, right_name) = match (left_name, right_name) {
            (Some(l), Some(r)) => (l, r),
            _ => continue,
        };

        // Find the placed nodes
        let left_place = find_place_in(&scene.nodes, left_name);
        let right_place = find_place_in(&scene.nodes, right_name);
        let (left_place, right_place) = match (left_place, right_place) {
            (Some(l), Some(r)) => (l, r),
            _ => continue,
        };

        // Get the shapes
        let left_shape = scene.find_shape(&left_place.shape_ref);
        let right_shape = scene.find_shape(&right_place.shape_ref);
        let (left_shape, right_shape) = match (left_shape, right_shape) {
            (Some(l), Some(r)) => (l, r),
            _ => continue,
        };

        // Resolve both to PathData
        let left_pd = left_shape.resolve(coord_space);
        let right_pd = right_shape.resolve(coord_space);

        if left_pd.points.is_empty() || right_pd.points.is_empty() {
            continue;
        }
        if left_pd.points.len() != right_pd.points.len() {
            continue;
        }

        // Check if points are x-mirrored.
        // For each left point, find the closest right point at the mirrored position.
        // This handles the case where "left" and "right" named points swap in a mirror.
        let mirror_axis = coord_space.0; // document width
        let total = left_pd.points.len();
        let tolerance = 5.0;
        let mut match_count = 0;

        for lp in &left_pd.points {
            let mirrored_x = mirror_axis - lp.x;
            let mirrored_y = lp.y;
            // Find best match in right shape
            let matched = right_pd.points.iter().any(|rp| {
                (rp.x - mirrored_x).abs() < tolerance && (rp.y - mirrored_y).abs() < tolerance
            });
            if matched {
                match_count += 1;
            }
        }

        let match_pct = (match_count as f64 / total as f64 * 100.0) as usize;
        if match_pct >= 80 {
            // Estimate line savings: shape definition + place line
            let op_count = left_shape.operations.len();
            let line_savings = op_count + 2; // shape header + ops + place line

            findings.push(Finding {
                kind: FindingKind::NearMirror,
                message: format!(
                    "'{}' and '{}' look like x-mirrors ({} points, {}% match)",
                    left_name, right_name, total, match_pct,
                ),
                detail: format!(
                    "define once, place with flip=x (saves ~{} lines)",
                    line_savings,
                ),
                suggestion: format!(
                    "delete shape '{}', then place it mirrored: \
                     place {} shape={} flip=x",
                    right_name, right_name, left_name,
                ),
                line_savings,
            });
        }
    }

    findings
}

// ── Composition summary ───────────────────────────────────────────────

fn composition_summary(scene: &Scene, mirror_count: usize) -> Vec<Finding> {
    let mut findings = Vec::new();
    let all_places = collect_all_places(&scene.nodes);

    // Count usage of composition features
    let flip_count = all_places.iter().filter(|p| p.flip.is_some()).count();
    let link_count = scene
        .nodes
        .iter()
        .filter(|n| matches!(n, SceneNode::Link(_)))
        .count();

    // Count duplicate shape refs (same shape placed multiple times)
    let mut shape_usage: HashMap<&str, usize> = HashMap::new();
    for place in &all_places {
        *shape_usage.entry(&place.shape_ref).or_insert(0) += 1;
    }
    let duplicate_geometries = shape_usage.values().filter(|&&count| count > 2).count();

    if flip_count == 0 && mirror_count > 0 {
        findings.push(Finding {
            kind: FindingKind::UnusedComposition,
            message: format!(
                "flip=x available but never used ({} mirrored shape pairs detected)",
                mirror_count,
            ),
            detail: String::new(),
            suggestion: String::new(),
            line_savings: 0,
        });
    }

    if link_count == 0 && duplicate_geometries > 0 {
        findings.push(Finding {
            kind: FindingKind::UnusedComposition,
            message: format!(
                "createlink available but never used ({} duplicate geometries found)",
                duplicate_geometries,
            ),
            detail: String::new(),
            suggestion: String::new(),
            line_savings: 0,
        });
    }

    findings
}

// ── EXP-2 · RepeatedPlaceRhythm ───────────────────────────────────────

/// N≥3 places of the SAME shape at the same scope whose positions form an
/// arithmetic progression on one axis (uniform gap; the other axis equal within
/// 0.5) and whose sizes are equal (or also arithmetic). Suggests the exact
/// `repeat` block that replaces them.
fn detect_repeated_rhythm(scene: &Scene) -> Vec<Finding> {
    let mut findings = Vec::new();
    // One scope per node level: the top level, plus each group's direct children.
    let mut scopes: Vec<Vec<&Place>> = vec![direct_places(&scene.nodes)];
    collect_group_scopes(&scene.nodes, &mut scopes);

    for scope in &scopes {
        // Group by shape ref; only absolutely-placed, sized places qualify.
        let mut by_shape: HashMap<&str, Vec<&Place>> = HashMap::new();
        for p in scope {
            if place_at(p).is_some() {
                by_shape.entry(p.shape_ref.as_str()).or_default().push(p);
            }
        }
        // Deterministic order for stable output.
        let mut shapes: Vec<&&str> = by_shape.keys().collect();
        shapes.sort();
        for shape_ref in shapes {
            let places = &by_shape[*shape_ref];
            if places.len() < 3 {
                continue;
            }
            if let Some(f) = rhythm_finding(shape_ref, places) {
                findings.push(f);
            }
        }
    }
    findings
}

/// Recursively push each group's direct-place children as its own scope.
fn collect_group_scopes<'a>(nodes: &'a [SceneNode], out: &mut Vec<Vec<&'a Place>>) {
    for node in nodes {
        match node {
            SceneNode::Group(g) => {
                out.push(direct_places(&g.children));
                collect_group_scopes(&g.children, out);
            }
            SceneNode::Frame(fr) => {
                out.push(direct_places(&fr.children));
                collect_group_scopes(&fr.children, out);
            }
            _ => {}
        }
    }
}

fn rhythm_finding(shape_ref: &str, places: &[&Place]) -> Option<Finding> {
    const EPS: f64 = 0.5;
    let shape = shape_ref;
    let n = places.len();

    // Gather (x, y, w, h); every place needs an absolute position and a size.
    let mut rows: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(n);
    for p in places {
        let (x, y) = place_at(p)?;
        let (w, h) = place_size(p)?;
        rows.push((x, y, w, h));
    }

    // Try x as the varying axis, then y. Sort a copy by that axis.
    for axis_x in [true, false] {
        let mut r = rows.clone();
        if axis_x {
            r.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            r.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        let xs: Vec<f64> = r.iter().map(|t| t.0).collect();
        let ys: Vec<f64> = r.iter().map(|t| t.1).collect();
        let ws: Vec<f64> = r.iter().map(|t| t.2).collect();
        let hs: Vec<f64> = r.iter().map(|t| t.3).collect();

        let (varying, fixed_vals, fixed_axis_x) = if axis_x {
            (&xs, &ys, false)
        } else {
            (&ys, &xs, true)
        };
        // Other axis must be constant.
        if !all_equal(fixed_vals, EPS) {
            continue;
        }
        // Varying axis must be a real arithmetic progression.
        let gap = match arithmetic_gap(varying, EPS) {
            Some(g) if g.abs() > EPS => g,
            _ => continue,
        };

        // Sizes: equal, or arithmetic per-axis.
        let (w_expr, h_expr) = match size_exprs(&ws, &hs, EPS) {
            Some(e) => e,
            None => continue,
        };

        let base_v = varying[0];
        let fixed_v = fixed_vals[0];
        let step = format!("{}+$i*{}", fmt_num(base_v), fmt_num(gap));
        let at_expr = if fixed_axis_x {
            // varying is y ⇒ at=<fixedX>,<yexpr>
            format!("{},{}", fmt_num(fixed_v), step)
        } else {
            // varying is x ⇒ at=<xexpr>,<fixedY>
            format!("{},{}", step, fmt_num(fixed_v))
        };

        let axis_name = if axis_x { "x" } else { "y" };
        let suggestion = format!(
            "repeat i {n}\n  place {shape} shape={shape} at={at_expr} size={w_expr}x{h_expr}\n\
             # replaces the {n} '{shape}' places; produces names {shape}-0…{shape}-{last} \
             (these differ from the current place names)",
            last = n - 1,
        );
        return Some(Finding {
            kind: FindingKind::RepeatedPlaceRhythm,
            message: format!(
                "'{shape}' is placed {n}× in an arithmetic rhythm (gap {} on {axis_name})",
                fmt_num(gap),
            ),
            detail: "an even progression of the same shape is exactly what `repeat` expresses — \
                 one body line instead of N hand-placed copies that can drift out of step"
                .to_string(),
            suggestion,
            line_savings: n.saturating_sub(2),
        });
    }
    None
}

/// Build the `size=` W and H expressions for a rhythm: `None` unless each axis is
/// either constant or arithmetic. Neither expression contains a literal `x` (the
/// dimension separator), so `WxH` stays unambiguous.
fn size_exprs(ws: &[f64], hs: &[f64], eps: f64) -> Option<(String, String)> {
    let w_expr = axis_size_expr(ws, eps)?;
    let h_expr = axis_size_expr(hs, eps)?;
    Some((w_expr, h_expr))
}

fn axis_size_expr(vals: &[f64], eps: f64) -> Option<String> {
    if all_equal(vals, eps) {
        return Some(fmt_num(vals[0]));
    }
    let g = arithmetic_gap(vals, eps)?;
    // Parenthesized so `x`-splitting the dimension keeps the expression intact.
    Some(format!("({}+$i*{})", fmt_num(vals[0]), fmt_num(g)))
}

// ── EXP-2 · NearDuplicateGroups ───────────────────────────────────────

/// A translation-invariant structural signature of a group: sorted
/// (shape_ref, dx, dy, w, h) of its direct place children, relative to their
/// min corner. Fills are deliberately ignored.
struct GroupSig<'a> {
    name: &'a str,
    pos: Option<(f64, f64)>,
    children: Vec<(String, f64, f64, f64, f64)>,
}

/// ≥2 groups whose child structure is identical (same shapes, same relative
/// offsets and sizes) differing only in group position / child fills.
fn detect_duplicate_groups(scene: &Scene) -> Vec<Finding> {
    let sigs = collect_group_sigs(&scene.nodes);
    let mut findings = Vec::new();
    let mut used = vec![false; sigs.len()];

    for i in 0..sigs.len() {
        if used[i] || sigs[i].children.len() < 2 {
            continue;
        }
        let mut cluster = vec![i];
        for (j, item) in sigs.iter().enumerate().skip(i + 1) {
            if used[j] {
                continue;
            }
            if sigs_match(&sigs[i], item, 0.5) {
                cluster.push(j);
            }
        }
        if cluster.len() < 2 {
            continue;
        }
        for &k in &cluster {
            used[k] = true;
        }

        let names: Vec<&str> = cluster.iter().map(|&k| sigs[k].name).collect();
        let child_count = sigs[i].children.len();

        // If the group positions themselves form a rhythm, suggest `repeat`;
        // otherwise suggest defining the structure once and reusing it.
        let positions: Vec<(f64, f64)> = cluster.iter().filter_map(|&k| sigs[k].pos).collect();
        let rhythm = positions.len() == cluster.len() && group_positions_are_rhythm(&positions);

        let sketch = sketch_group(&sigs[i]);
        let suggestion = if rhythm {
            format!(
                "these groups are structurally identical and evenly spaced — wrap the shared \
                 body in a `repeat`:\n{sketch}"
            )
        } else {
            format!(
                "these groups are structurally identical — define the structure once (a \
                 `component`, or one `group` you place/reference) instead of repeating it:\n{sketch}"
            )
        };

        findings.push(Finding {
            kind: FindingKind::NearDuplicateGroups,
            message: format!(
                "{} groups are structurally identical ({}): {}",
                cluster.len(),
                names.join(", "),
                format_args!("{child_count} children each, differing only in position/fill"),
            ),
            detail: "duplicated group structure drifts apart under edits — one definition keeps \
                 every copy in sync"
                .to_string(),
            suggestion,
            line_savings: (cluster.len() - 1) * child_count,
        });
    }
    findings
}

fn collect_group_sigs(nodes: &[SceneNode]) -> Vec<GroupSig<'_>> {
    let mut out = Vec::new();
    for node in nodes {
        if let SceneNode::Group(g) = node {
            if let Some(sig) = group_sig(g) {
                out.push(sig);
            }
            out.extend(collect_group_sigs(&g.children));
        }
    }
    out
}

fn group_sig(g: &Group) -> Option<GroupSig<'_>> {
    let places = direct_places(&g.children);
    if places.len() < 2 {
        return None;
    }
    let mut rows: Vec<(String, f64, f64, f64, f64)> = Vec::new();
    for p in &places {
        let (x, y) = place_at(p)?;
        let (w, h) = place_size(p).unwrap_or((0.0, 0.0));
        rows.push((p.shape_ref.clone(), x, y, w, h));
    }
    // Normalize to the min corner so the signature is translation-invariant.
    let min_x = rows.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let min_y = rows.iter().map(|r| r.2).fold(f64::INFINITY, f64::min);
    for r in &mut rows {
        r.1 -= min_x;
        r.2 -= min_y;
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    Some(GroupSig {
        name: &g.name,
        pos: g.position,
        children: rows,
    })
}

fn sigs_match(a: &GroupSig, b: &GroupSig, eps: f64) -> bool {
    if a.children.len() != b.children.len() {
        return false;
    }
    a.children.iter().zip(&b.children).all(|(x, y)| {
        x.0 == y.0
            && (x.1 - y.1).abs() <= eps
            && (x.2 - y.2).abs() <= eps
            && (x.3 - y.3).abs() <= eps
            && (x.4 - y.4).abs() <= eps
    })
}

fn group_positions_are_rhythm(positions: &[(f64, f64)]) -> bool {
    if positions.len() < 3 {
        // Two groups: a rhythm is ambiguous; prefer the define-once phrasing.
        return false;
    }
    let mut xs: Vec<f64> = positions.iter().map(|p| p.0).collect();
    let mut ys: Vec<f64> = positions.iter().map(|p| p.1).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let x_rhythm = all_equal(&xs, 0.5) || arithmetic_gap(&xs, 0.5).is_some_and(|g| g.abs() > 0.5);
    let y_rhythm = all_equal(&ys, 0.5) || arithmetic_gap(&ys, 0.5).is_some_and(|g| g.abs() > 0.5);
    x_rhythm && y_rhythm
}

fn sketch_group(sig: &GroupSig) -> String {
    let mut s = String::from("  group <name> at=<x>,<y>\n");
    for (shape_ref, dx, dy, w, h) in &sig.children {
        s.push_str(&format!(
            "    place {shape_ref} shape={shape_ref} at=<x>+{},<y>+{} size={}x{}\n",
            fmt_num(*dx),
            fmt_num(*dy),
            fmt_num(*w),
            fmt_num(*h),
        ));
    }
    s.trim_end().to_string()
}

// ── EXP-2 · MagicNumberRhythm ─────────────────────────────────────────

/// The same non-trivial number (|v|≥4) used ≥4× **as one shared size dimension**
/// (every one of its uses is a width, or every use is a height — never a
/// coordinate) in a document with NO `let` bindings ⇒ a missing named constant
/// (a shared column width or row height). Suggests `let <slot-name> <value>`.
///
/// Restricting to a single exclusive *size* slot is the key precision lever:
/// illustrations reuse the same coordinate/dimension by coincidence constantly,
/// but a value that is *always* a width across ≥4 places is a deliberate design
/// constant. This keeps the finding silent on the repo's illustration fixtures.
fn detect_magic_numbers(scene: &Scene) -> Vec<Finding> {
    if !scene.lets.is_empty() || !looks_like_layout(scene) {
        return Vec::new();
    }
    // slot: 0=x, 1=y, 2=w, 3=h. Count occurrences per rounded value per slot.
    let mut counts: HashMap<i64, [usize; 4]> = HashMap::new();
    for p in collect_all_places(&scene.nodes) {
        if let Some((x, y)) = place_at(p) {
            bump(&mut counts, x, 0);
            bump(&mut counts, y, 1);
        }
        if let Some((w, h)) = place_size(p) {
            bump(&mut counts, w, 2);
            bump(&mut counts, h, 3);
        }
    }

    // Deterministic order: by value.
    let mut keys: Vec<i64> = counts.keys().copied().collect();
    keys.sort();

    let mut findings = Vec::new();
    let mut used_names: HashMap<String, usize> = HashMap::new();
    for key in keys {
        let slots = counts[&key];
        let value = key as f64;
        if value.abs() < 4.0 {
            continue;
        }
        // Must be used EXCLUSIVELY as one size dimension (w or h): no coordinate
        // uses (slots 0/1 zero) and only one of the two size slots nonzero.
        if slots[0] != 0 || slots[1] != 0 {
            continue;
        }
        let (slot, dominant) = if slots[2] >= 4 && slots[3] == 0 {
            (2usize, slots[2])
        } else if slots[3] >= 4 && slots[2] == 0 {
            (3usize, slots[3])
        } else {
            continue;
        };

        let (base, dim_word) = if slot == 2 {
            ("col-w", "width")
        } else {
            ("row-h", "height")
        };
        let name = disambiguate(base, &mut used_names);
        findings.push(Finding {
            kind: FindingKind::MagicNumberRhythm,
            message: format!(
                "the value {} is the {dim_word} of {dominant} places but has no name",
                fmt_num(value),
            ),
            detail: "a bare number reused as the same dimension across many places is a design \
                 constant with no name — one edit to change it means finding every copy. This \
                 document has no `let` bindings"
                .to_string(),
            suggestion: format!(
                "let {name} {v}\n# then reference ${name} as the {dim_word} in each size= \
                 (e.g. size={size_example}) — one edit changes every use",
                v = fmt_num(value),
                size_example = if slot == 2 {
                    format!("(${name})x30")
                } else {
                    format!("40x(${name})")
                },
            ),
            line_savings: 0,
        });
    }
    findings
}

fn bump(counts: &mut HashMap<i64, [usize; 4]>, v: f64, slot: usize) {
    // Only whole-ish values are worth naming; round to the nearest integer key.
    if (v - v.round()).abs() > 0.01 {
        return;
    }
    let key = v.round() as i64;
    counts.entry(key).or_insert([0; 4])[slot] += 1;
}

fn disambiguate(base: &str, used: &mut HashMap<String, usize>) -> String {
    let count = used.entry(base.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base.to_string()
    } else {
        format!("{base}-{count}")
    }
}

// ── EXP-2 · UnanchoredAdjacency ───────────────────────────────────────

/// Two absolutely-placed elements that are edge-aligned or gap-adjacent (within
/// 2px) but use raw `at=` instead of a relative anchor. Capped at the 5 most
/// adjacent pairs. Suggests the concrete `at=<target>.<anchor>` rewrite.
/// A place name paired with its resolved bbox `(x0, y0, x1, y1)`.
type NamedBbox<'a> = (&'a str, (f64, f64, f64, f64));

fn detect_unanchored_adjacency(scene: &Scene) -> Vec<Finding> {
    if !looks_like_layout(scene) {
        return Vec::new();
    }
    let bboxes = crate::resolve::element_bboxes(scene);

    // Only places positioned by a raw absolute `at=` (no relative anchor) count —
    // those already using anchors are the good case we're teaching toward.
    let mut items: Vec<NamedBbox> = Vec::new();
    for p in collect_all_places(&scene.nodes) {
        let absolute = matches!(p.position, PlacePosition::At(_, _))
            && p.anchor.is_none()
            && p.align.is_none();
        if !absolute {
            continue;
        }
        if let Some(&bb) = bboxes.get(&p.name) {
            items.push((p.name.as_str(), bb));
        }
    }

    let mut cands: Vec<(f64, Finding)> = Vec::new();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            if let Some((dev, finding)) = adjacency_finding(items[i], items[j]) {
                cands.push((dev, finding));
            }
        }
    }
    cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    cands.into_iter().take(5).map(|(_, f)| f).collect()
}

// ── Agent layout safety · text collisions ────────────────────────────

type Bbox = (f64, f64, f64, f64);

fn boxes_overlap(a: Bbox, b: Bbox) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

fn box_contains(outer: Bbox, inner: Bbox) -> bool {
    const EPS: f64 = 0.5;
    inner.0 >= outer.0 - EPS
        && inner.1 >= outer.1 - EPS
        && inner.2 <= outer.2 + EPS
        && inner.3 <= outer.3 + EPS
}

fn is_closed_geometry(scene: &Scene, place: &Place) -> bool {
    scene.find_shape(&place.shape_ref).is_some_and(|shape| {
        matches!(
            shape.template,
            crate::shape::Template::Rectangle
                | crate::shape::Template::Ellipse
                | crate::shape::Template::Triangle
        )
    })
}

fn smallest_containing_closed_host<'a>(
    scene: &Scene,
    text: &Place,
    text_box: Bbox,
    places: &'a [&'a Place],
    bboxes: &HashMap<String, Bbox>,
) -> Option<(&'a Place, Bbox)> {
    places
        .iter()
        .copied()
        .filter(|place| place.name != text.name && is_closed_geometry(scene, place))
        .filter_map(|place| {
            let &bbox = bboxes.get(&place.name)?;
            box_contains(bbox, text_box).then_some((place, bbox))
        })
        .min_by(|(_, a), (_, b)| {
            bbox_area(*a)
                .partial_cmp(&bbox_area(*b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// A label contained by a small foreground badge can legitimately overlap the
/// larger panel behind the badge. Treat the smallest containing host as the
/// visual owner when it is smaller than, and overlaps, the candidate obstacle.
fn foreground_host_masks_obstacle(
    host: Option<(&Place, Bbox)>,
    obstacle: &Place,
    obstacle_box: Bbox,
) -> bool {
    host.is_some_and(|(host, host_box)| {
        host.name != obstacle.name
            && bbox_area(host_box) < bbox_area(obstacle_box)
            && boxes_overlap(host_box, obstacle_box)
    })
}

/// Find text that clips the edge of a closed geometric place.
///
/// This deliberately does not report every overlap. A text bbox fully contained
/// by a rectangle/ellipse/triangle is almost always a label inside its intended
/// host. Open lines and paths are ignored because bbox intersection says little
/// about their painted pixels. What remains is the high-signal failure mode an
/// agent commonly misses: a label grazing an adjacent badge, node, or panel.
fn detect_text_collisions(scene: &Scene) -> Vec<Finding> {
    let bboxes = crate::resolve::element_bboxes(scene);
    let places = collect_all_places(&scene.nodes);
    let mut findings = Vec::new();

    for text in places.iter().copied().filter(|p| {
        scene
            .find_shape(&p.shape_ref)
            .is_some_and(|s| s.template == crate::shape::Template::Text)
    }) {
        let Some(&text_box) = bboxes.get(&text.name) else {
            continue;
        };
        let host = smallest_containing_closed_host(scene, text, text_box, &places, &bboxes);

        for obstacle in places.iter().copied() {
            if obstacle.name == text.name {
                continue;
            }
            if !is_closed_geometry(scene, obstacle) {
                continue;
            }
            let Some(&obstacle_box) = bboxes.get(&obstacle.name) else {
                continue;
            };
            if !boxes_overlap(text_box, obstacle_box) {
                continue;
            }
            // A contained text run is the expected label-inside-shape case.
            // Suppress only that pair: the same label can still collide with a
            // neighboring shape inside the larger host panel.
            if box_contains(obstacle_box, text_box) {
                continue;
            }
            if foreground_host_masks_obstacle(host, obstacle, obstacle_box) {
                continue;
            }

            findings.push(Finding {
                kind: FindingKind::TextCollision,
                message: format!(
                    "text '{}' partially overlaps '{}'",
                    text.name, obstacle.name
                ),
                detail: "partial text/shape intersections are usually accidental; place labels relative to their host or keep them clear of neighboring geometry (text bounds use layout-grade estimated font metrics)".to_string(),
                suggestion: collision_anchor_suggestion(&text.name, &obstacle.name, text_box, obstacle_box),
                line_savings: 0,
            });

            // One concrete repair per text run keeps audit output scannable.
            break;
        }
    }

    findings
}

fn collision_anchor_suggestion(
    text_name: &str,
    obstacle_name: &str,
    text_box: Bbox,
    obstacle_box: Bbox,
) -> String {
    const GAP: i32 = 8;
    let text_center = (
        (text_box.0 + text_box.2) / 2.0,
        (text_box.1 + text_box.3) / 2.0,
    );
    let obstacle_center = (
        (obstacle_box.0 + obstacle_box.2) / 2.0,
        (obstacle_box.1 + obstacle_box.3) / 2.0,
    );
    let dx = text_center.0 - obstacle_center.0;
    let dy = text_center.1 - obstacle_center.1;

    let placement = if dx.abs() >= dy.abs() {
        if dx <= 0.0 {
            format!("at={obstacle_name}.left align=right offset=-{GAP},0")
        } else {
            format!("at={obstacle_name}.right align=left offset={GAP},0")
        }
    } else if dy <= 0.0 {
        format!("at={obstacle_name}.top align=bottom offset=0,-{GAP}")
    } else {
        format!("at={obstacle_name}.bottom align=top offset=0,{GAP}")
    };

    format!("replace '{text_name}' position with: {placement}; then render and inspect")
}

/// Find the high-confidence "text centered in a node by eyeballed baseline"
/// pattern and teach the relational form. We only report a host when exactly one
/// raw-positioned text run chooses it as its smallest containing closed shape;
/// multi-line labels and dense panels therefore stay silent.
fn detect_unanchored_labels(scene: &Scene) -> Vec<Finding> {
    let bboxes = crate::resolve::element_bboxes(scene);
    let places = collect_all_places(&scene.nodes);
    let mut candidates: Vec<(&Place, &Place, Bbox, Bbox)> = Vec::new();

    for text in places.iter().copied().filter(|p| {
        matches!(p.position, PlacePosition::At(_, _))
            && p.anchor.is_none()
            && p.align.is_none()
            && scene
                .find_shape(&p.shape_ref)
                .is_some_and(|s| s.template == crate::shape::Template::Text)
    }) {
        let Some(&text_box) = bboxes.get(&text.name) else {
            continue;
        };
        if has_partial_closed_overlap(scene, text, text_box, &places, &bboxes) {
            // The collision finding is the more urgent and more precise repair;
            // do not also suggest centering the same label inside a broad panel.
            continue;
        }
        let host = smallest_containing_closed_host(scene, text, text_box, &places, &bboxes);
        let Some((host, host_box)) = host else {
            continue;
        };

        let text_center = bbox_center(text_box);
        let host_center = bbox_center(host_box);
        let host_w = host_box.2 - host_box.0;
        let host_h = host_box.3 - host_box.1;
        let near_center = (text_center.0 - host_center.0).abs() <= host_w * 0.30
            && (text_center.1 - host_center.1).abs() <= host_h * 0.30;
        if near_center {
            candidates.push((text, host, text_box, host_box));
        }
    }

    let mut host_counts: HashMap<&str, usize> = HashMap::new();
    for (_, host, _, _) in &candidates {
        *host_counts.entry(host.name.as_str()).or_insert(0) += 1;
    }

    candidates
        .into_iter()
        .filter(|(_, host, _, _)| host_counts.get(host.name.as_str()) == Some(&1))
        .map(|(text, host, _, _)| Finding {
            kind: FindingKind::UnanchoredLabel,
            message: format!(
                "text '{}' looks centered in '{}' but uses a raw baseline",
                text.name, host.name
            ),
            detail: "plain text at=x,y means SVG baseline-start; attaching the text box to its host makes centering explicit and resilient to geometry edits".to_string(),
            suggestion: format!(
                "replace '{}' position with: at={}.center align=center; then render and inspect optical centering",
                text.name, host.name
            ),
            line_savings: 0,
        })
        .collect()
}

fn bbox_area(b: Bbox) -> f64 {
    (b.2 - b.0).abs() * (b.3 - b.1).abs()
}

fn bbox_center(b: Bbox) -> (f64, f64) {
    ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
}

fn has_partial_closed_overlap(
    scene: &Scene,
    text: &Place,
    text_box: Bbox,
    places: &[&Place],
    bboxes: &HashMap<String, Bbox>,
) -> bool {
    let host = smallest_containing_closed_host(scene, text, text_box, places, bboxes);
    places.iter().copied().any(|obstacle| {
        if obstacle.name == text.name {
            return false;
        }
        if !is_closed_geometry(scene, obstacle) {
            return false;
        }
        let Some(&obstacle_box) = bboxes.get(&obstacle.name) else {
            return false;
        };
        boxes_overlap(text_box, obstacle_box)
            && !box_contains(obstacle_box, text_box)
            && !foreground_host_masks_obstacle(host, obstacle, obstacle_box)
    })
}

/// bbox tuple = (x0, y0, x1, y1) — min/max corners, as `resolve::element_bboxes`
/// returns. Returns (deviation, finding) if the pair is an unanchored neighbor,
/// else `None`.
fn adjacency_finding(a: NamedBbox, b: NamedBbox) -> Option<(f64, Finding)> {
    const TOL: f64 = 2.0;
    let (an, (ax0, ay0, ax1, ay1)) = a;
    let (bn, (bx0, by0, bx1, by1)) = b;
    let (a_cx, a_cy) = ((ax0 + ax1) / 2.0, (ay0 + ay1) / 2.0);
    let (b_cx, b_cy) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0);

    // Decide the dominant relationship axis by center separation.
    let horizontal = (b_cx - a_cx).abs() >= (b_cy - a_cy).abs();

    if horizontal {
        // Order left→right. Each entry is (x0, y0, x1, y1).
        let (l, r, ln, rn) = if a_cx <= b_cx {
            ((ax0, ay0, ax1, ay1), (bx0, by0, bx1, by1), an, bn)
        } else {
            ((bx0, by0, bx1, by1), (ax0, ay0, ax1, ay1), bn, an)
        };
        let gap = r.0 - l.2;
        // Must be horizontally disjoint (neighbors, not overlapping like eyes).
        if gap < -0.5 {
            return None;
        }
        // Share a vertical band (otherwise it's a diagonal, not a row neighbor).
        let ov = l.3.min(r.3) - l.1.max(r.1);
        if ov <= 0.0 {
            return None;
        }
        // Require BOTH: the boxes touch (gap≈0) AND a flush horizontal edge
        // (tops or bottoms aligned). This "abutting, flush" relationship is a
        // deliberate layout — rare by coincidence — which keeps the finding
        // silent on illustrations where elements merely brush past each other.
        let dev_touch = gap.abs();
        let dev_edge = (l.1 - r.1).abs().min((l.3 - r.3).abs());
        if dev_touch > TOL || dev_edge > TOL {
            return None;
        }
        let dev = dev_touch.max(dev_edge);
        let dy = r.1 - l.1;
        let suggestion = format!(
            "place {rn} … at={ln}.right align=left offset={},{}",
            fmt_num(gap.round()),
            fmt_num(dy.round()),
        );
        Some((
            dev,
            Finding {
                kind: FindingKind::UnanchoredAdjacency,
                message: format!(
                    "'{ln}' and '{rn}' are horizontal neighbors placed with absolute at="
                ),
                detail: "hard-coded coordinates for adjacent elements drift when either moves — a \
                     relative anchor keeps them locked together"
                    .to_string(),
                suggestion,
                line_savings: 0,
            },
        ))
    } else {
        // Order top→bottom. Each entry is (x0, y0, x1, y1).
        let (t, bo, tn, bn2) = if a_cy <= b_cy {
            ((ax0, ay0, ax1, ay1), (bx0, by0, bx1, by1), an, bn)
        } else {
            ((bx0, by0, bx1, by1), (ax0, ay0, ax1, ay1), bn, an)
        };
        let gap = bo.1 - t.3;
        if gap < -0.5 {
            return None;
        }
        let ov = t.2.min(bo.2) - t.0.max(bo.0);
        if ov <= 0.0 {
            return None;
        }
        // Require BOTH: touching (gap≈0) AND a flush vertical edge (lefts or
        // rights aligned) — a deliberate stacked layout, rare by coincidence.
        let dev_touch = gap.abs();
        let dev_edge = (t.0 - bo.0).abs().min((t.2 - bo.2).abs());
        if dev_touch > TOL || dev_edge > TOL {
            return None;
        }
        let dev = dev_touch.max(dev_edge);
        let dx = bo.0 - t.0;
        let suggestion = format!(
            "place {bn2} … at={tn}.bl align=tl offset={},{}",
            fmt_num(dx.round()),
            fmt_num(gap.round()),
        );
        Some((
            dev,
            Finding {
                kind: FindingKind::UnanchoredAdjacency,
                message: format!(
                    "'{tn}' and '{bn2}' are vertical neighbors placed with absolute at="
                ),
                detail: "hard-coded coordinates for adjacent elements drift when either moves — a \
                     relative anchor keeps them locked together"
                    .to_string(),
                suggestion,
                line_savings: 0,
            },
        ))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Collect all Place nodes from the scene tree (flattening groups).
fn collect_all_places(nodes: &[SceneNode]) -> Vec<&Place> {
    let mut places = Vec::new();
    for node in nodes {
        match node {
            SceneNode::Place(p) => places.push(p),
            SceneNode::Group(g) => places.extend(collect_all_places(&g.children)),
            SceneNode::Frame(fr) => places.extend(collect_all_places(&fr.children)),
            SceneNode::Link(_) | SceneNode::Instance(_) => {}
        }
    }
    places
}

/// Find a Place node by name.
fn find_place_in<'a>(nodes: &'a [SceneNode], name: &str) -> Option<&'a Place> {
    for node in nodes {
        match node {
            SceneNode::Place(p) if p.name == name => return Some(p),
            SceneNode::Group(g) => {
                if let Some(found) = find_place_in(&g.children, name) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl_parse;

    #[test]
    fn detects_mirror_pair() {
        let input = "\
documentsize 400x400

shape eye-l template=ellipse
  movepoint top to=120,80
  movepoint bottom to=120,100
  movepoint left to=110,90
  movepoint right to=130,90
  fill #3a2510

shape eye-r template=ellipse
  movepoint top to=280,80
  movepoint bottom to=280,100
  movepoint left to=270,90
  movepoint right to=290,90
  fill #3a2510

place eye-l shape=eye-l at=0,0 size=40x30
place eye-r shape=eye-r at=0,0 size=40x30
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            findings.iter().any(|f| f.kind == FindingKind::NearMirror),
            "should detect mirror pair, got: {:?}",
            findings,
        );
    }

    #[test]
    fn detects_rough_catmull_through_collinear_points() {
        // A near-straight run smoothed with catmull-rom — the wavy-edge trap.
        let input = "\
documentsize 400x400

shape edge template=path
  addpoint a at=10,100
  addpoint b at=60,100 mode=catmull-rom
  addpoint c at=110,101 mode=catmull-rom
  addpoint d at=160,100 mode=catmull-rom
  addpoint e at=210,100

place edge shape=edge at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            findings.iter().any(|f| f.kind == FindingKind::RoughCatmull),
            "should flag rough catmull, got: {:?}",
            findings,
        );
    }

    #[test]
    fn no_rough_catmull_for_genuine_curve() {
        // Genuinely curved points — catmull-rom is the right tool, no warning.
        let input = "\
documentsize 400x400

shape arch template=path
  addpoint a at=10,200
  addpoint b at=60,80 mode=catmull-rom
  addpoint c at=200,40 mode=catmull-rom
  addpoint d at=340,80 mode=catmull-rom
  addpoint e at=390,200

place arch shape=arch at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            !findings.iter().any(|f| f.kind == FindingKind::RoughCatmull),
            "should NOT flag a genuine curve, got: {:?}",
            findings,
        );
    }

    #[test]
    fn detects_isolated_catmull_segments_in_closed_path() {
        let input = "\
documentsize 24x24

shape shield template=path
  addpoint tl at=6,4
  addpoint tr at=18,4
  addpoint r at=19,12
  addpoint tip at=12,21
  addpoint l at=5,12
  close
  smooth r
  smooth l
  round-corners tl=0.6 tr=0.6

place shield shape=shield at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::IsolatedCatmull),
            "should flag isolated incoming curve segments, got: {:?}",
            findings,
        );
    }

    #[test]
    fn no_isolated_catmull_for_continuous_closed_run() {
        let input = "\
documentsize 24x24

shape shield template=path
  addpoint tl at=6,4
  addpoint tr at=18,4
  addpoint r at=19,12
  addpoint tip at=12,21
  addpoint l at=5,12
  close
  smooth r
  smooth tip
  smooth l
  smooth tl

place shield shape=shield at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            !findings
                .iter()
                .any(|f| f.kind == FindingKind::IsolatedCatmull),
            "continuous curve run should not be flagged, got: {:?}",
            findings,
        );
    }

    #[test]
    fn no_false_mirror_for_different_shapes() {
        let input = "\
documentsize 400x400

shape arm-l template=path
  addpoint a at=100,200
  addpoint b at=50,300
  fill #d8b480

shape arm-r template=path
  addpoint a at=300,150
  addpoint b at=350,280
  fill #d8b480

place arm-l shape=arm-l at=0,0
place arm-r shape=arm-r at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            !findings.iter().any(|f| f.kind == FindingKind::NearMirror),
            "should not detect mirror for non-mirrored shapes",
        );
    }

    #[test]
    fn reports_unused_flip() {
        let input = "\
documentsize 400x400

shape eye-l template=ellipse
  movepoint top to=120,80
  movepoint bottom to=120,100
  movepoint left to=110,90
  movepoint right to=130,90

shape eye-r template=ellipse
  movepoint top to=280,80
  movepoint bottom to=280,100
  movepoint left to=270,90
  movepoint right to=290,90

place eye-l shape=eye-l at=0,0 size=40x30
place eye-r shape=eye-r at=0,0 size=40x30
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::UnusedComposition && f.message.contains("flip=x")),
            "should report unused flip",
        );
    }

    #[test]
    fn no_findings_for_clean_file() {
        let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

place bg shape=bg at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(findings.is_empty(), "clean file should have no findings");
    }

    // ── EXP-2 · compositional findings ────────────────────────────────

    fn has(findings: &[Finding], kind: FindingKind) -> bool {
        findings.iter().any(|f| f.kind == kind)
    }

    fn find(findings: &[Finding], kind: FindingKind) -> &Finding {
        findings
            .iter()
            .find(|f| f.kind == kind)
            .expect("expected finding of this kind")
    }

    #[test]
    fn repeated_rhythm_positive_and_suggests_repeat_block() {
        let input = "\
documentsize 800x600

shape bar template=rectangle
  fill #333333

place bar shape=bar at=50,538 size=40x4
place bar shape=bar at=150,538 size=40x4
place bar shape=bar at=250,538 size=40x4
place bar shape=bar at=350,538 size=40x4
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(has(&findings, FindingKind::RepeatedPlaceRhythm));
        let f = find(&findings, FindingKind::RepeatedPlaceRhythm);
        assert!(
            f.suggestion.contains("repeat i 4"),
            "suggestion: {}",
            f.suggestion
        );
        assert!(
            f.suggestion.contains("at=50+$i*100,538"),
            "suggestion: {}",
            f.suggestion
        );
        assert!(f.suggestion.contains("size=40x4"), "{}", f.suggestion);
    }

    #[test]
    fn repeated_rhythm_negative_for_irregular_spacing() {
        // Same shape, three places, but the gaps are not uniform ⇒ no rhythm.
        let input = "\
documentsize 800x600

shape bar template=rectangle
  fill #333333

place bar shape=bar at=50,10 size=40x4
place bar shape=bar at=150,10 size=40x4
place bar shape=bar at=400,10 size=40x4
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::RepeatedPlaceRhythm));
    }

    #[test]
    fn duplicate_groups_positive() {
        let input = "\
documentsize 400x400

shape dot template=rectangle
  fill #333333

group g1 at=300,300
  place d1 shape=dot at=0,0 size=10x10
  place d2 shape=dot at=20,0 size=10x10

group g2 at=300,340
  place d3 shape=dot at=0,0 size=10x10
  place d4 shape=dot at=20,0 size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(has(&findings, FindingKind::NearDuplicateGroups));
        let f = find(&findings, FindingKind::NearDuplicateGroups);
        assert!(f.message.contains("g1"), "{}", f.message);
        assert!(f.message.contains("g2"), "{}", f.message);
        assert!(f.suggestion.contains("place dot"), "{}", f.suggestion);
    }

    #[test]
    fn duplicate_groups_negative_for_different_structure() {
        let input = "\
documentsize 400x400

shape dot template=rectangle
  fill #333333

shape box template=rectangle
  fill #444444

group g1 at=300,300
  place d1 shape=dot at=0,0 size=10x10
  place d2 shape=dot at=20,0 size=10x10

group g2 at=300,340
  place b1 shape=box at=0,0 size=10x10
  place b2 shape=box at=40,0 size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::NearDuplicateGroups));
    }

    #[test]
    fn magic_number_positive_on_layout_doc() {
        // A layout doc (has tokens) with a width reused ≥4× exclusively.
        let input = "\
documentsize 400x400

tokens
  radius.md 8

shape bar template=rectangle
  fill #333333

place b1 shape=bar at=10,20 size=60x10
place b2 shape=bar at=10,50 size=60x10
place b3 shape=bar at=10,80 size=60x10
place b4 shape=bar at=10,110 size=60x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(has(&findings, FindingKind::MagicNumberRhythm));
        let f = find(&findings, FindingKind::MagicNumberRhythm);
        assert!(f.suggestion.contains("let col-w 60"), "{}", f.suggestion);
    }

    #[test]
    fn magic_number_negative_on_illustration() {
        // Same repeated width, but NOT a layout doc (no tokens/components/frames)
        // ⇒ silent: coincidental repetition in freehand art is not a constant.
        let input = "\
documentsize 400x400

shape bar template=rectangle
  fill #333333

place b1 shape=bar at=10,20 size=60x10
place b2 shape=bar at=10,50 size=60x10
place b3 shape=bar at=10,80 size=60x10
place b4 shape=bar at=10,110 size=60x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::MagicNumberRhythm));
    }

    #[test]
    fn unanchored_adjacency_positive() {
        let input = "\
documentsize 400x400

tokens
  radius.md 8

shape chip template=rectangle
  fill #333333

place chip-a shape=chip at=200,200 size=40x20
place chip-b shape=chip at=240,200 size=40x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(has(&findings, FindingKind::UnanchoredAdjacency));
        let f = find(&findings, FindingKind::UnanchoredAdjacency);
        assert!(f.suggestion.contains("at=chip-a.right"), "{}", f.suggestion);
        assert!(f.suggestion.contains("align=left"), "{}", f.suggestion);
    }

    #[test]
    fn unanchored_adjacency_negative_for_overlapping_pair() {
        // Two fully-overlapping boxes (like eye-l/eye-r placed at the same spot)
        // are NOT neighbors — must never be flagged as adjacency.
        let input = "\
documentsize 400x400

tokens
  radius.md 8

shape chip template=rectangle
  fill #333333

place chip-a shape=chip at=200,200 size=40x20
place chip-b shape=chip at=200,200 size=40x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::UnanchoredAdjacency));
    }

    #[test]
    fn text_collision_flags_partial_overlap_and_suggests_anchor() {
        let input = "\
documentsize 400x300

shape badge template=ellipse
  fill #1248cf

shape label template=text
  content \"RUNTIME A\"
  font-size 20
  font-weight 800

place badge shape=badge at=220,100 size=60x60
place label shape=label at=185,135
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        let f = find(&findings, FindingKind::TextCollision);
        assert!(f.message.contains("label"), "{}", f.message);
        assert!(f.message.contains("badge"), "{}", f.message);
        assert!(f.suggestion.contains("at=badge.left"), "{}", f.suggestion);
        assert!(f.suggestion.contains("align=right"), "{}", f.suggestion);
    }

    #[test]
    fn text_collision_ignores_containment_but_finds_neighbor_inside_panel() {
        let input = "\
documentsize 400x300

shape panel template=rectangle
  fill #fffaf0

shape badge template=ellipse
  fill #1248cf

shape label template=text
  content \"RUNTIME A\"
  font-size 20
  font-weight 800

place panel shape=panel at=100,80 size=220x100
place badge shape=badge at=230,100 size=60x60
place label shape=label at=190,135
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        let collisions: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.kind == FindingKind::TextCollision)
            .collect();
        assert_eq!(collisions.len(), 1, "{findings:#?}");
        assert!(collisions[0].message.contains("badge"));
        assert!(!collisions[0].message.contains("panel"));
    }

    #[test]
    fn text_collision_ignores_panel_behind_foreground_badge() {
        let input = "\
documentsize 400x300

shape panel template=rectangle
  fill #fffaf0

shape badge template=ellipse
  fill #1248cf

shape label template=text
  content \"fed\"
  font-size 20
  font-weight 800
  text-anchor middle

place panel shape=panel at=100,100 size=220x100
place badge shape=badge center=210,100 size=80x80
place label shape=label at=badge.center align=center
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::TextCollision), "{findings:#?}");
    }

    #[test]
    fn text_collision_ignores_text_fully_inside_shape_and_open_paths() {
        let input = "\
documentsize 400x300

shape host template=rectangle
  fill #1248cf

shape route template=path
  addpoint a at=10,10
  addpoint b at=300,200 after=a
  open

shape label template=text
  content \"OK\"
  font-size 20

place host shape=host at=100,80 size=100x60
place route shape=route at=0,0
place label shape=label at=130,115
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(!has(&findings, FindingKind::TextCollision), "{findings:#?}");
    }

    #[test]
    fn unanchored_label_teaches_center_alignment_for_single_hosted_text() {
        let input = "\
documentsize 300x200

shape node template=ellipse
  fill #1248cf

shape label template=text
  content \"FIT\"
  font-size 24
  text-anchor middle

place node shape=node center=150,100 size=100x100
place label shape=label at=150,92
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        let f = find(&findings, FindingKind::UnanchoredLabel);
        assert!(f.message.contains("raw baseline"), "{}", f.message);
        assert!(
            f.suggestion.contains("at=node.center align=center"),
            "{}",
            f.suggestion
        );
    }

    #[test]
    fn unanchored_label_ignores_multiline_host_and_relational_text() {
        let input = "\
documentsize 400x240

shape output template=rectangle
  fill #171712

shape label template=text
  content \"TRY\"
  font-size 24
  text-anchor middle

place output shape=output at=60,40 size=120x140
place line-a shape=label at=120,90
place line-b shape=label at=120,122
  content \"FED\"

place second-output shape=output at=240,40 size=120x140
place centered shape=label at=second-output.center align=center
  content \"FIT\"
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);
        assert!(
            !has(&findings, FindingKind::UnanchoredLabel),
            "{findings:#?}"
        );
    }

    /// Snapshot-style: a single purpose-built fixture that exercises all four
    /// EXP-2 findings at once. The exact `kind|message` dump is pinned so any
    /// drift in detection or wording is caught.
    #[test]
    fn all_four_findings_on_purpose_built_fixture() {
        let input = "\
documentsize 400x400

tokens
  radius.md 8

shape bar template=rectangle
  fill #333333

shape chip template=rectangle
  fill #444444

shape dot template=rectangle
  fill #555555

place bar shape=bar at=20,20 size=60x10
place bar shape=bar at=20,50 size=60x10
place bar shape=bar at=20,80 size=60x10
place bar shape=bar at=20,110 size=60x10

place chip-a shape=chip at=200,200 size=40x20
place chip-b shape=chip at=240,200 size=40x20

group g1 at=300,300
  place d1 shape=dot at=0,0 size=12x12
  place d2 shape=dot at=25,0 size=12x12

group g2 at=300,340
  place d3 shape=dot at=0,0 size=12x12
  place d4 shape=dot at=25,0 size=12x12
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let findings = audit(&scene);

        // Filter to the EXP-2 kinds so unrelated (existing) findings don't make
        // this brittle, then pin the exact messages.
        let new_kinds = [
            FindingKind::RepeatedPlaceRhythm,
            FindingKind::NearDuplicateGroups,
            FindingKind::MagicNumberRhythm,
            FindingKind::UnanchoredAdjacency,
        ];
        let dump: String = findings
            .iter()
            .filter(|f| new_kinds.contains(&f.kind))
            .map(|f| format!("{}|{}", f.kind.id(), f.message))
            .collect::<Vec<_>>()
            .join("\n");

        let expected = "\
repeated_place_rhythm|'bar' is placed 4× in an arithmetic rhythm (gap 30 on y)
near_duplicate_groups|2 groups are structurally identical (g1, g2): 2 children each, differing only in position/fill
magic_number_rhythm|the value 10 is the height of 4 places but has no name
magic_number_rhythm|the value 60 is the width of 4 places but has no name
unanchored_adjacency|'chip-a' and 'chip-b' are horizontal neighbors placed with absolute at=";
        assert_eq!(dump, expected, "\nfull findings: {findings:#?}");

        // And every EXP-2 finding carries an actionable suggestion.
        for f in findings.iter().filter(|f| new_kinds.contains(&f.kind)) {
            assert!(!f.suggestion.is_empty(), "{:?} lacks a suggestion", f.kind);
        }
    }
}
