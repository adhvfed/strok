//! Inspection / query layer (C6 / E3.2).
//!
//! Builds on the 9-anchor + bbox machinery (`resolve::element_bboxes`,
//! `measure`) to support agent-friendly spatial inspection:
//!
//! - **structural snapshot** at three detail levels
//!   ([`Detail::Full`]/[`Detail::Structural`]/[`Detail::Summary`]) — what's in
//!   the document, with or without path geometry;
//! - **region query** ([`query_box`]) — "what's in this rectangle?";
//! - **overlap query** ([`query_overlaps`]) — "what overlaps element X?";
//! - **spatial relations** ([`relate`]) — the qualitative relationship between
//!   two elements (left-of / contains / aligned-top / overlaps …).
//!
//! Every result carries a `to_json()` returning a [`Json`] value built with the
//! shared [`crate::json`] helper, so `--json` is one stable, snapshot-tested
//! schema everywhere — generalizing the C5 `measure --json` seam.

use crate::json::Json;
use crate::measure::measure_boxes;
use crate::resolve::{element_bboxes, resolve_scene};
use crate::scene::{Scene, SceneNode};
use std::collections::HashMap;

type Bbox = (f64, f64, f64, f64);

/// One placed element in document space.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    /// Element kind: the shape template (`rectangle`/`path`/`text`/…) or
    /// `"group"` for a group node.
    pub kind: String,
    /// The shape this place references (empty for groups).
    pub shape_ref: String,
    /// Document-space bbox `(x0, y0, x1, y1)`, if known.
    pub bbox: Option<Bbox>,
}

impl Element {
    fn bbox_json(&self) -> Json {
        match self.bbox {
            Some((x0, y0, x1, y1)) => Json::obj([
                ("x", Json::num(x0)),
                ("y", Json::num(y0)),
                ("w", Json::num(x1 - x0)),
                ("h", Json::num(y1 - y0)),
            ]),
            None => Json::Null,
        }
    }

    fn to_json(&self, detail: Detail) -> Json {
        let mut fields = vec![
            ("name", Json::str(&self.name)),
            ("kind", Json::str(&self.kind)),
        ];
        if detail != Detail::Summary {
            if !self.shape_ref.is_empty() {
                fields.push(("shape", Json::str(&self.shape_ref)));
            }
            fields.push(("bbox", self.bbox_json()));
        }
        Json::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
}

/// Detail level for the structural snapshot (E3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Full resolved SVG (every attribute + path geometry).
    Full,
    /// Structure only: element names, kinds and bboxes — no path `d` data.
    Structural,
    /// Just IDs + types (the lightest snapshot).
    Summary,
}

impl Detail {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(Detail::Full),
            "structural" => Some(Detail::Structural),
            "summary" => Some(Detail::Summary),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Detail::Full => "full",
            Detail::Structural => "structural",
            Detail::Summary => "summary",
        }
    }
}

/// Walk the scene in document order, returning every placed element / group with
/// its bbox (from `element_bboxes`, the same machinery anchors use).
pub fn elements(scene: &Scene) -> Vec<Element> {
    let boxes = element_bboxes(scene);
    let mut out = Vec::new();
    collect_elements(scene, &scene.nodes, &boxes, &mut out);
    out
}

fn collect_elements(
    scene: &Scene,
    nodes: &[SceneNode],
    boxes: &HashMap<String, Bbox>,
    out: &mut Vec<Element>,
) {
    for node in nodes {
        match node {
            SceneNode::Place(p) => {
                let kind = scene
                    .find_shape(&p.shape_ref)
                    .map(|s| s.template.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                out.push(Element {
                    name: p.name.clone(),
                    kind,
                    shape_ref: p.shape_ref.clone(),
                    bbox: boxes.get(&p.name).copied(),
                });
            }
            SceneNode::Group(g) => {
                out.push(Element {
                    name: g.name.clone(),
                    kind: "group".to_string(),
                    shape_ref: String::new(),
                    bbox: boxes.get(&g.name).copied(),
                });
                collect_elements(scene, &g.children, boxes, out);
            }
            SceneNode::Boolean(b) => {
                out.push(Element {
                    name: b.name.clone(),
                    kind: format!("boolean-{}", b.op.name()),
                    shape_ref: String::new(),
                    bbox: boxes.get(&b.name).copied(),
                });
                collect_elements(scene, &b.children, boxes, out);
            }
            SceneNode::Frame(fr) => {
                out.push(Element {
                    name: fr.name.clone(),
                    kind: "frame".to_string(),
                    shape_ref: String::new(),
                    bbox: boxes.get(&fr.name).copied(),
                });
                collect_elements(scene, &fr.children, boxes, out);
            }
            SceneNode::Instance(i) => {
                out.push(Element {
                    name: i.name.clone(),
                    kind: "instance".to_string(),
                    shape_ref: i.component.clone(),
                    bbox: boxes.get(&i.name).copied(),
                });
            }
            SceneNode::Link(_) => {}
        }
    }
}

/// The structural snapshot of a document at a given detail level.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub detail: Detail,
    pub width: f64,
    pub height: f64,
    pub elements: Vec<Element>,
    /// Full resolved SVG — only populated for [`Detail::Full`].
    pub svg: Option<String>,
}

/// Produce a structural snapshot of the scene at `detail`.
pub fn snapshot(scene: &Scene, detail: Detail) -> Snapshot {
    Snapshot {
        detail,
        width: scene.document_size.w,
        height: scene.document_size.h,
        elements: elements(scene),
        svg: if detail == Detail::Full {
            Some(resolve_scene(scene))
        } else {
            None
        },
    }
}

impl Snapshot {
    pub fn to_json(&self) -> Json {
        let mut fields = vec![
            ("detail", Json::str(self.detail.as_str())),
            (
                "document",
                Json::obj([
                    ("width", Json::num(self.width)),
                    ("height", Json::num(self.height)),
                ]),
            ),
            ("count", Json::num(self.elements.len() as f64)),
            (
                "elements",
                Json::array(self.elements.iter().map(|e| e.to_json(self.detail))),
            ),
        ];
        if let Some(svg) = &self.svg {
            fields.push(("svg", Json::str(svg)));
        }
        Json::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
}

/// Do two bboxes overlap on both axes (their areas intersect)?
fn boxes_overlap(a: Bbox, b: Bbox) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Is `inner` fully contained within `outer`?
fn box_contains(outer: Bbox, inner: Bbox) -> bool {
    inner.0 >= outer.0 && inner.1 >= outer.1 && inner.2 <= outer.2 && inner.3 <= outer.3
}

/// The result of a region / overlaps query: matched elements + the query input.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    /// A description of the query (`box x,y,w,h` or `overlaps <id>`).
    pub query: String,
    pub matches: Vec<Element>,
}

impl QueryResult {
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("query", Json::str(&self.query)),
            ("count", Json::num(self.matches.len() as f64)),
            (
                "matches",
                Json::array(self.matches.iter().map(|e| e.to_json(Detail::Structural))),
            ),
        ])
    }
}

/// "What's in this region?" — every element whose bbox intersects the rectangle
/// `(x, y, w, h)` (E3.2). Groups are included (they are elements too).
pub fn query_box(scene: &Scene, x: f64, y: f64, w: f64, h: f64) -> QueryResult {
    let region: Bbox = (x, y, x + w, y + h);
    let matches = elements(scene)
        .into_iter()
        .filter(|e| e.bbox.map(|b| boxes_overlap(region, b)).unwrap_or(false))
        .collect();
    QueryResult {
        query: format!(
            "box {},{},{},{}",
            crate::types::fmt_num(x),
            crate::types::fmt_num(y),
            crate::types::fmt_num(w),
            crate::types::fmt_num(h)
        ),
        matches,
    }
}

/// "What overlaps element X?" — every other element whose bbox intersects X's
/// (E3.2). Returns `Err` with the missing name if `id` isn't a placed element.
pub fn query_overlaps(scene: &Scene, id: &str) -> Result<QueryResult, String> {
    let all = elements(scene);
    let target = all
        .iter()
        .find(|e| e.name == id)
        .ok_or_else(|| format!("no placed element named '{}'", id))?;
    let tbox = target
        .bbox
        .ok_or_else(|| format!("element '{}' has no bounding box", id))?;
    let matches = all
        .iter()
        .filter(|e| e.name != id)
        .filter(|e| e.bbox.map(|b| boxes_overlap(tbox, b)).unwrap_or(false))
        .cloned()
        .collect();
    Ok(QueryResult {
        query: format!("overlaps {}", id),
        matches,
    })
}

/// A qualitative spatial relationship between two elements (E3.2). Built on the
/// `measure` deltas so it stays consistent with the canvas math.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub a: String,
    pub b: String,
    /// Horizontal: `left-of` / `right-of` / `x-overlap`.
    pub horizontal: String,
    /// Vertical: `above` / `below` / `y-overlap`.
    pub vertical: String,
    pub overlaps: bool,
    /// `a-contains-b` / `b-contains-a` / `none`.
    pub containment: String,
    /// Edges that line up (within ε): any of left/right/top/bottom/cx/cy.
    pub aligned_edges: Vec<String>,
    pub gap_x: f64,
    pub gap_y: f64,
}

const ALIGN_EPS: f64 = 0.5;

/// Compute the spatial relation between two placed elements by name. `Err` names
/// the missing element so the CLI can report it without panicking.
pub fn relate(scene: &Scene, a: &str, b: &str) -> Result<Relation, String> {
    let boxes = element_bboxes(scene);
    let ba = boxes
        .get(a)
        .copied()
        .ok_or_else(|| format!("no placed element named '{}'", a))?;
    let bb = boxes
        .get(b)
        .copied()
        .ok_or_else(|| format!("no placed element named '{}'", b))?;
    Ok(relate_boxes(a, b, ba, bb))
}

fn relate_boxes(a: &str, b: &str, ba: Bbox, bb: Bbox) -> Relation {
    let m = measure_boxes(a, b, ba, bb);

    let horizontal = if bb.0 >= ba.2 {
        "right-of"
    } else if bb.2 <= ba.0 {
        "left-of"
    } else {
        "x-overlap"
    }
    .to_string();

    let vertical = if bb.1 >= ba.3 {
        "below"
    } else if bb.3 <= ba.1 {
        "above"
    } else {
        "y-overlap"
    }
    .to_string();

    let containment = if box_contains(ba, bb) {
        "a-contains-b"
    } else if box_contains(bb, ba) {
        "b-contains-a"
    } else {
        "none"
    }
    .to_string();

    let mut aligned_edges = Vec::new();
    if m.align_left.abs() <= ALIGN_EPS {
        aligned_edges.push("left".to_string());
    }
    if m.align_right.abs() <= ALIGN_EPS {
        aligned_edges.push("right".to_string());
    }
    if m.align_top.abs() <= ALIGN_EPS {
        aligned_edges.push("top".to_string());
    }
    if m.align_bottom.abs() <= ALIGN_EPS {
        aligned_edges.push("bottom".to_string());
    }
    if m.align_center_x.abs() <= ALIGN_EPS {
        aligned_edges.push("center-x".to_string());
    }
    if m.align_center_y.abs() <= ALIGN_EPS {
        aligned_edges.push("center-y".to_string());
    }

    Relation {
        a: a.to_string(),
        b: b.to_string(),
        horizontal,
        vertical,
        overlaps: m.overlaps,
        containment,
        aligned_edges,
        gap_x: m.gap_x,
        gap_y: m.gap_y,
    }
}

impl Relation {
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("a", Json::str(&self.a)),
            ("b", Json::str(&self.b)),
            ("horizontal", Json::str(&self.horizontal)),
            ("vertical", Json::str(&self.vertical)),
            ("overlaps", Json::Bool(self.overlaps)),
            ("containment", Json::str(&self.containment)),
            (
                "aligned_edges",
                Json::array(self.aligned_edges.iter().map(Json::str)),
            ),
            ("gap_x", Json::num(self.gap_x)),
            ("gap_y", Json::num(self.gap_y)),
        ])
    }

    pub fn to_text(&self) -> String {
        let aligned = if self.aligned_edges.is_empty() {
            "none".to_string()
        } else {
            self.aligned_edges.join(", ")
        };
        format!(
            "relate {} → {}\n  \
             horizontal: {}\n  \
             vertical:   {}\n  \
             overlaps:   {}\n  \
             contains:   {}\n  \
             aligned:    {}\n  \
             gap:        x={} y={}\n",
            self.a,
            self.b,
            self.horizontal,
            self.vertical,
            self.overlaps,
            self.containment,
            aligned,
            crate::types::fmt_num(self.gap_x),
            crate::types::fmt_num(self.gap_y),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_and_contains_helpers() {
        assert!(boxes_overlap(
            (0.0, 0.0, 10.0, 10.0),
            (5.0, 5.0, 15.0, 15.0)
        ));
        assert!(!boxes_overlap(
            (0.0, 0.0, 10.0, 10.0),
            (20.0, 0.0, 30.0, 10.0)
        ));
        assert!(box_contains((0.0, 0.0, 10.0, 10.0), (2.0, 2.0, 8.0, 8.0)));
        assert!(!box_contains((0.0, 0.0, 10.0, 10.0), (2.0, 2.0, 12.0, 8.0)));
    }

    #[test]
    fn relate_left_of_and_aligned_top() {
        // a left of b, same top edge.
        let r = relate_boxes("a", "b", (0.0, 0.0, 10.0, 10.0), (20.0, 0.0, 30.0, 10.0));
        assert_eq!(r.horizontal, "right-of"); // b is right of a
        assert_eq!(r.vertical, "y-overlap");
        assert!(r.aligned_edges.contains(&"top".to_string()));
        assert!(!r.overlaps);
    }

    #[test]
    fn relate_containment() {
        let r = relate_boxes("a", "b", (0.0, 0.0, 100.0, 100.0), (10.0, 10.0, 20.0, 20.0));
        assert_eq!(r.containment, "a-contains-b");
        assert!(r.overlaps);
    }

    #[test]
    fn detail_round_trips() {
        assert_eq!(Detail::parse("full"), Some(Detail::Full));
        assert_eq!(Detail::parse("structural"), Some(Detail::Structural));
        assert_eq!(Detail::parse("summary"), Some(Detail::Summary));
        assert_eq!(Detail::parse("nope"), None);
    }
}
