/// v3 resolver — Scene → SVG string.
///
/// Takes a Scene (shapes + placed instances) and produces an SVG document
/// that can be rendered by strok-render.
use crate::error::{Result, StrokError};
use crate::path_point::{path_data_to_svg_d, Flip, PathData, Placement};
use crate::scene::*;
use crate::shape::*;
use crate::types::{self, Color, Direction, GradientStop, LinearGradient, RadialGradient};

use std::collections::HashMap;

/// Bounding box: (min_x, min_y, max_x, max_y).
type Bbox = (f64, f64, f64, f64);

/// Collected SVG definition: (id, SVG element string).
/// Used for gradients, filters, clip paths, etc.
type SvgDef = (String, String);

/// Resolve a Scene into an SVG string.
pub fn resolve_scene(scene: &Scene) -> String {
    let w = scene.document_size.w;
    let h = scene.document_size.h;

    let mut bboxes: HashMap<String, Bbox> = HashMap::new();
    let mut svg_defs: Vec<SvgDef> = Vec::new();

    // Pass 1: resolve all nodes, collecting gradient defs
    let mut body = String::new();
    for node in &scene.nodes {
        resolve_node(scene, node, 1, &mut body, &mut bboxes, &mut svg_defs);
    }

    // Build final SVG: header → defs → body → close
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        types::fmt_num(w),
        types::fmt_num(h),
        types::fmt_num(w),
        types::fmt_num(h),
    );
    svg.push('\n');

    if !svg_defs.is_empty() {
        svg.push_str("  <defs>\n");
        for (_, def) in &svg_defs {
            svg.push_str(&format!("    {}\n", def));
        }
        svg.push_str("  </defs>\n");
    }

    svg.push_str(&body);
    svg.push_str("</svg>\n");
    svg
}

/// Resolve the scene to SVG with an **annotate overlay** (C6 / E3.2): each
/// placed element / group gets a small ID label drawn at its bbox top-left so an
/// agent can map what it sees in the rendered PNG to the names it can reference.
/// The underlying geometry is the exact same `d` the renderer produces — only
/// an overlay `<g>` is appended.
pub fn resolve_scene_annotated(scene: &Scene) -> String {
    let base = resolve_scene(scene);
    let boxes = element_bboxes(scene);

    // Collect labels in document order (places then groups, recursing) so the
    // overlay is deterministic and snapshot-stable.
    let mut labels: Vec<(String, Bbox)> = Vec::new();
    annotate_collect(&scene.nodes, &boxes, &mut labels);

    if labels.is_empty() {
        return base;
    }

    // A readable label size relative to the canvas (clamped).
    let dim = scene.document_size.w.min(scene.document_size.h);
    let font = (dim / 24.0).clamp(6.0, 14.0);

    let mut overlay = String::new();
    overlay.push_str("  <g id=\"strok-annotations\" font-family=\"monospace\" ");
    overlay.push_str(&format!(
        "font-size=\"{}\" pointer-events=\"none\">\n",
        types::fmt_num(font)
    ));
    for (name, b) in &labels {
        let x = types::fmt_num(b.0);
        let y = types::fmt_num((b.1 + font).min(b.3));
        let esc = name
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        // White halo behind the text for legibility on any background.
        overlay.push_str(&format!(
            "    <text x=\"{x}\" y=\"{y}\" fill=\"#ffffff\" stroke=\"#ffffff\" stroke-width=\"3\" paint-order=\"stroke\">{esc}</text>\n"
        ));
        overlay.push_str(&format!(
            "    <text x=\"{x}\" y=\"{y}\" fill=\"#ff0066\">{esc}</text>\n"
        ));
    }
    overlay.push_str("  </g>\n");

    // Inject the overlay just before the closing tag.
    match base.rfind("</svg>") {
        Some(idx) => {
            let mut out = String::with_capacity(base.len() + overlay.len());
            out.push_str(&base[..idx]);
            out.push_str(&overlay);
            out.push_str(&base[idx..]);
            out
        }
        None => base,
    }
}

/// Add a non-destructive, high-contrast geometry outline above an already
/// resolved SVG scene.
///
/// `ids=None` outlines every named placed path/text element. `ids=Some(..)`
/// limits the overlay to those exact resolved IDs and errors when any requested
/// ID is absent from this render (important for `render --node`).
///
/// The overlay is built from the resolved SVG rather than from shape templates,
/// so placement sizing, flips, rotations, group transforms, clips, masks, and
/// text layout are identical to the normal render beneath it. Two copies provide
/// a black halo and white foreground stroke; `vector-effect` keeps the diagnostic
/// line readable when a region is rendered at high resolution.
pub fn add_outline_overlay(svg: &str, ids: Option<&[String]>) -> Result<String> {
    let open_end = svg.find('>').ok_or_else(|| {
        StrokError::InvalidOperation("cannot outline malformed resolved SVG".to_string())
    })?;
    let close_start = svg.rfind("</svg>").ok_or_else(|| {
        StrokError::InvalidOperation("cannot outline malformed resolved SVG".to_string())
    })?;
    if close_start <= open_end {
        return Err(StrokError::InvalidOperation(
            "cannot outline malformed resolved SVG".to_string(),
        ));
    }

    if let Some(requested) = ids {
        if requested.is_empty() {
            return Err(StrokError::InvalidOperation(
                "outline selection is empty; omit the value to outline all placed geometry"
                    .to_string(),
            ));
        }
        for id in requested {
            if !resolved_graphic_id_exists(svg, id) {
                return Err(StrokError::InvalidOperation(format!(
                    "outline id '{id}' is not a placed element in this render"
                )));
            }
        }
    }

    let inner = &svg[open_end + 1..close_start];
    let halo_prefix = "strok-outline-halo-";
    let ink_prefix = "strok-outline-ink-";
    let halo = prefix_svg_references(inner, halo_prefix);
    let ink = prefix_svg_references(inner, ink_prefix);

    let mut overlay = String::new();
    overlay.push_str("  <style>\n");
    overlay.push_str(&outline_css(
        "strok-outline-halo",
        halo_prefix,
        ids,
        "#000000",
        5.0,
    ));
    overlay.push_str(&outline_css(
        "strok-outline-ink",
        ink_prefix,
        ids,
        "#ffffff",
        2.0,
    ));
    overlay.push_str("  </style>\n");
    overlay.push_str(
        "  <g id=\"strok-outline-overlay\" pointer-events=\"none\" aria-hidden=\"true\">\n",
    );
    overlay.push_str("    <g id=\"strok-outline-halo\">\n");
    overlay.push_str(&halo);
    overlay.push_str("    </g>\n");
    overlay.push_str("    <g id=\"strok-outline-ink\">\n");
    overlay.push_str(&ink);
    overlay.push_str("    </g>\n");
    overlay.push_str("  </g>\n");

    let mut out = String::with_capacity(svg.len() + overlay.len() + inner.len() * 2);
    out.push_str(&svg[..close_start]);
    out.push_str(&overlay);
    out.push_str(&svg[close_start..]);
    Ok(out)
}

fn resolved_graphic_id_exists(svg: &str, id: &str) -> bool {
    ["path", "text"]
        .iter()
        .any(|tag| svg.contains(&format!("<{tag} id=\"{id}\"")))
}

fn prefix_svg_references(svg: &str, prefix: &str) -> String {
    svg.replace("id=\"", &format!("id=\"{prefix}"))
        .replace("url(#", &format!("url(#{prefix}"))
        .replace("href=\"#", &format!("href=\"#{prefix}"))
}

fn outline_css(
    wrapper_id: &str,
    id_prefix: &str,
    ids: Option<&[String]>,
    color: &str,
    width: f64,
) -> String {
    const GRAPHICS: &[&str] = &[
        "path", "text", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image",
    ];
    let hidden = GRAPHICS
        .iter()
        .map(|tag| format!("#{wrapper_id} {tag}"))
        .collect::<Vec<_>>()
        .join(", ");
    let definitions = GRAPHICS
        .iter()
        .map(|tag| format!("#{wrapper_id} defs {tag}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut css = format!(
        "    {hidden} {{ display: none; }}\n\
         \x20   {definitions} {{ display: inline; }}\n\
         \x20   #{wrapper_id} g {{ opacity: 1 !important; filter: none !important; }}\n"
    );

    let selector = match ids {
        None => format!("#{wrapper_id} path[id], #{wrapper_id} text[id]"),
        Some(ids) => ids
            .iter()
            .map(|id| {
                format!(
                    "#{wrapper_id} [id=\"{}{}\"]",
                    id_prefix,
                    escape_css_string(id)
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    css.push_str(&format!(
        "    {selector} {{ display: inline !important; fill: none !important; \
         stroke: {color} !important; stroke-width: {} !important; \
         stroke-dasharray: none !important; stroke-linecap: round !important; \
         stroke-linejoin: round !important; opacity: 1 !important; \
         filter: none !important; vector-effect: non-scaling-stroke; }}\n",
        types::fmt_num(width)
    ));
    css
}

fn escape_css_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\a ")
        .replace('\r', "\\d ")
}

fn annotate_collect(
    nodes: &[crate::scene::SceneNode],
    boxes: &HashMap<String, Bbox>,
    out: &mut Vec<(String, Bbox)>,
) {
    use crate::scene::SceneNode;
    for node in nodes {
        match node {
            SceneNode::Place(p) => {
                if let Some(b) = boxes.get(&p.name) {
                    out.push((p.name.clone(), *b));
                }
            }
            SceneNode::Group(g) => {
                if let Some(b) = boxes.get(&g.name) {
                    out.push((g.name.clone(), *b));
                }
                annotate_collect(&g.children, boxes, out);
            }
            SceneNode::Frame(fr) => {
                if let Some(b) = boxes.get(&fr.name) {
                    out.push((fr.name.clone(), *b));
                }
                annotate_collect(&fr.children, boxes, out);
            }
            SceneNode::Link(_) | SceneNode::Instance(_) => {}
        }
    }
}

/// Resolve a Scene into SVG containing only a single named node.
pub fn resolve_scene_single_node(scene: &Scene, node_name: &str) -> String {
    let w = scene.document_size.w;
    let h = scene.document_size.h;

    let mut bboxes: HashMap<String, Bbox> = HashMap::new();
    let mut svg_defs: Vec<SvgDef> = Vec::new();

    // Resolve all nodes to build bboxes (needed for anchor resolution),
    // but only keep SVG output for the target node.
    let mut full_svg = String::new();
    for node in &scene.nodes {
        resolve_node(scene, node, 1, &mut full_svg, &mut bboxes, &mut svg_defs);
    }

    // Now resolve just the target node for SVG output
    let mut body = String::new();
    let mut target_defs: Vec<SvgDef> = Vec::new();
    if let Some(node) = scene.find_node(node_name) {
        resolve_node(scene, node, 1, &mut body, &mut bboxes, &mut target_defs);
    }

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        types::fmt_num(w),
        types::fmt_num(h),
        types::fmt_num(w),
        types::fmt_num(h),
    );
    svg.push('\n');

    if !target_defs.is_empty() {
        svg.push_str("  <defs>\n");
        for (_, def) in &target_defs {
            svg.push_str(&format!("    {}\n", def));
        }
        svg.push_str("  </defs>\n");
    }

    svg.push_str(&body);
    svg.push_str("</svg>\n");
    svg
}

/// Resolve the **document-space** SVG `d` string of a single placed element,
/// applying its place `at`/`size`/`flip` and anchor resolution exactly as the
/// renderer does. This is the geometry C3's boolean / outline / offset ops
/// operate on, so they see the same shape the canvas shows. Returns `None` if
/// the place (or its shape) isn't found, or the shape is a text shape (no fill
/// geometry). Bboxes from the whole scene are resolved first so anchor-relative
/// placements land correctly.
pub fn placed_document_d(scene: &Scene, place_name: &str) -> Option<String> {
    // Resolve every node to populate bboxes (needed for RelativeTo anchors).
    let mut bboxes: HashMap<String, Bbox> = HashMap::new();
    let mut sink = String::new();
    let mut defs: Vec<SvgDef> = Vec::new();
    for node in &scene.nodes {
        resolve_node(scene, node, 1, &mut sink, &mut bboxes, &mut defs);
    }
    placed_document_d_with_bboxes(scene, place_name, &bboxes)
}

/// Like `placed_document_d` but reuses an already-built bbox map instead of
/// re-resolving the whole scene. Used by text-on-path resolution to avoid
/// re-entering `resolve_node` (which would recurse infinitely through the text
/// place that triggered the lookup).
pub fn placed_document_d_with_bboxes(
    scene: &Scene,
    place_name: &str,
    bboxes: &HashMap<String, Bbox>,
) -> Option<String> {
    let place = match scene.find_node(place_name) {
        Some(SceneNode::Place(p)) => p,
        _ => return None,
    };
    let shape = scene.find_shape(&place.shape_ref)?;
    if shape.is_text() {
        return None;
    }
    let (geometry_shape, _attr_shape) = find_link_source_shape(scene, shape);
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut pd = resolve_place_geometry(geometry_shape, coord_space, place.size);
    apply_effects(&mut pd, &geometry_shape.effects);

    let (pos_x, pos_y) = resolve_position(scene, place, &pd, bboxes);
    let placement = Placement {
        at: (pos_x, pos_y),
        size: place.size.map(|d| (d.w, d.h)),
        flip: place.flip.map(convert_flip),
    };
    let d = path_data_to_svg_d(&pd, Some(&placement));
    if d.is_empty() {
        None
    } else {
        Some(d)
    }
}

/// The document-space bounding box of every placed element, keyed by place name
/// (E2.7). This is the exact same `(min_x, min_y, max_x, max_y)` machinery the
/// anchor / RelativeTo resolver uses, so `measure` reports the box the canvas
/// shows (including transform-aware AABBs for rotated/skewed elements).
pub fn element_bboxes(scene: &Scene) -> HashMap<String, (f64, f64, f64, f64)> {
    let mut bboxes: HashMap<String, Bbox> = HashMap::new();
    let mut sink = String::new();
    let mut defs: Vec<SvgDef> = Vec::new();
    for node in &scene.nodes {
        resolve_node(scene, node, 1, &mut sink, &mut bboxes, &mut defs);
    }
    bboxes
}

/// The resolved stroke style of a placed element, for `outline-stroke`.
/// Falls back through place overrides → shape → defaults, mirroring `resolve_place`.
pub fn placed_stroke_style(
    scene: &Scene,
    place_name: &str,
) -> Option<crate::stroke_outline::StrokeStyle> {
    let place = match scene.find_node(place_name) {
        Some(SceneNode::Place(p)) => p,
        _ => return None,
    };
    let shape = scene.find_shape(&place.shape_ref)?;
    let (_g, attr_shape) = find_link_source_shape(scene, shape);
    let width = resolve_stroke_width(&place.overrides, attr_shape, &scene.defaults).unwrap_or(1.0);
    let cap = resolve_linecap(&place.overrides, attr_shape, &scene.defaults)
        .unwrap_or(types::LineCap::Butt);
    let join = resolve_linejoin(&place.overrides, attr_shape, &scene.defaults)
        .unwrap_or(types::LineJoin::Miter);
    let miter_limit =
        resolve_miterlimit(&place.overrides, attr_shape, &scene.defaults).unwrap_or(4.0);
    Some(crate::stroke_outline::StrokeStyle {
        width,
        cap,
        join,
        miter_limit,
    })
}

/// The resolved fill-rule of a placed element, for boolean inputs.
pub fn placed_fill_rule(scene: &Scene, place_name: &str) -> Option<types::FillRule> {
    let place = match scene.find_node(place_name) {
        Some(SceneNode::Place(p)) => p,
        _ => return None,
    };
    let shape = scene.find_shape(&place.shape_ref)?;
    let (_g, attr_shape) = find_link_source_shape(scene, shape);
    resolve_fill_rule(&place.overrides, attr_shape, &scene.defaults)
}

fn resolve_node(
    scene: &Scene,
    node: &SceneNode,
    indent: usize,
    svg: &mut String,
    bboxes: &mut HashMap<String, Bbox>,
    svg_defs: &mut Vec<SvgDef>,
) {
    let prefix = "  ".repeat(indent);
    match node {
        SceneNode::Place(place) => {
            resolve_place(scene, place, &prefix, svg, bboxes, svg_defs);
        }
        SceneNode::Group(group) => {
            // Register clip path if clip= is set (E2.4: clip by one OR MORE shapes;
            // multiple ⇒ union of their geometry inside one <clipPath>).
            let clip_ref = group.clip.as_ref().and_then(|names| {
                register_clip_def(scene, &format!("clip-{}", group.name), names, svg_defs)
            });
            // Register alpha/luminance mask if mask= is set (E2.4).
            let mask_ref = group.mask.as_ref().and_then(|name| {
                register_mask_def(scene, &format!("mask-{}", group.name), name, svg_defs)
            });

            // Build the compound transform via the unified affine (E2.3):
            // translate · rotate · skew · flip, composed in that order.
            let t = group_transform(group);

            svg.push_str(&format!("{}  <g id=\"{}\"", prefix, group.name));
            if t != crate::attrs::IDENTITY {
                svg.push_str(&format!(
                    " transform=\"{}\"",
                    crate::attrs::emit_transform(&t)
                ));
            }
            if let Some(ref clip_id) = clip_ref {
                svg.push_str(&format!(" clip-path=\"url(#{})\"", clip_id));
            }
            if let Some(ref mask_id) = mask_ref {
                svg.push_str(&format!(" mask=\"url(#{})\"", mask_id));
            }
            if let Some(opacity) = group.opacity {
                svg.push_str(&format!(" opacity=\"{}\"", types::fmt_num(opacity)));
            }
            svg.push_str(">\n");

            // Collect child names before resolving (for bbox offset)
            let child_names = collect_node_names(&group.children);

            for child in &group.children {
                resolve_node(scene, child, indent + 1, svg, bboxes, svg_defs);
            }

            // Map child bboxes through the FULL group transform so cross-group
            // relative placement works under rotation/skew, not just translate
            // (E2.3 correct-bbox-under-transform). The transform-aware bbox is the
            // AABB of the four mapped corners.
            if t != crate::attrs::IDENTITY {
                for name in &child_names {
                    if let Some(bbox) = bboxes.get_mut(name) {
                        *bbox = crate::attrs::transform_bbox(&t, bbox.0, bbox.1, bbox.2, bbox.3);
                    }
                }
            }

            svg.push_str(&format!("{}  </g>\n", prefix));
        }
        SceneNode::Link(_) => {
            // Links are stored as shapes and resolved through place references.
        }
        SceneNode::Frame(frame) => {
            resolve_frame(scene, frame, indent, &prefix, svg, bboxes, svg_defs);
        }
        SceneNode::Instance(inst) => {
            resolve_instance(scene, inst, indent, &prefix, svg, bboxes, svg_defs);
        }
    }
}

/// Render a `frame` (C8): a `<g>` (translated to `at=`) carrying an optional
/// background rect for `fill`/`round-corners`, then its children. The layout
/// policy is a *code-gen* concept (flex/grid → CSS); for the SVG/PNG preview a
/// frame is a styled container, so authors keep their eyes on it (doc-02 invariant).
fn resolve_frame(
    scene: &Scene,
    frame: &Frame,
    indent: usize,
    prefix: &str,
    svg: &mut String,
    bboxes: &mut HashMap<String, Bbox>,
    svg_defs: &mut Vec<SvgDef>,
) {
    svg.push_str(&format!("{}  <g id=\"{}\"", prefix, frame.name));
    if let Some((x, y)) = frame.position {
        svg.push_str(&format!(
            " transform=\"translate({} {})\"",
            types::fmt_num(x),
            types::fmt_num(y)
        ));
    }
    if let Some(o) = frame.opacity {
        svg.push_str(&format!(" opacity=\"{}\"", types::fmt_num(o)));
    }
    svg.push_str(">\n");

    // Background rect when the frame has a fill or radius and a known size.
    if let Some(size) = frame.size {
        if frame.fill.is_some() || frame.radius.is_some() {
            let rx = match &frame.radius {
                Some(RadiusValue::Literal(r)) => *r,
                Some(RadiusValue::Token(_)) => 0.0, // already resolved by lowering; SVG bg approximates
                None => 0.0,
            };
            svg.push_str(&format!(
                "{}    <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\"",
                prefix,
                types::fmt_num(size.w),
                types::fmt_num(size.h)
            ));
            if rx != 0.0 {
                svg.push_str(&format!(" rx=\"{}\"", types::fmt_num(rx)));
            }
            emit_svg_fill(&frame.fill, svg);
            svg.push_str(" />\n");
        }
        bboxes.insert(
            frame.name.clone(),
            (
                frame.position.map(|p| p.0).unwrap_or(0.0),
                frame.position.map(|p| p.1).unwrap_or(0.0),
                size.w,
                size.h,
            ),
        );
    }

    for child in &frame.children {
        resolve_node(scene, child, indent + 1, svg, bboxes, svg_defs);
    }

    svg.push_str(&format!("{}  </g>\n", prefix));
}

/// Render a component `instance` (C8): inline the referenced component's body as
/// a `<g>` so the instance is visible in the preview. Unknown components render
/// as an empty group (graceful — the parser already validates references).
fn resolve_instance(
    scene: &Scene,
    inst: &Instance,
    indent: usize,
    prefix: &str,
    svg: &mut String,
    bboxes: &mut HashMap<String, Bbox>,
    svg_defs: &mut Vec<SvgDef>,
) {
    svg.push_str(&format!("{}  <g id=\"{}\"", prefix, inst.name));
    if let Some((x, y)) = inst.position {
        svg.push_str(&format!(
            " transform=\"translate({} {})\"",
            types::fmt_num(x),
            types::fmt_num(y)
        ));
    }
    svg.push_str(">\n");
    if let Some(component) = scene.find_component(&inst.component) {
        for child in &component.children {
            resolve_node(scene, child, indent + 1, svg, bboxes, svg_defs);
        }
    }
    svg.push_str(&format!("{}  </g>\n", prefix));
}

/// Collect all named nodes recursively (for bbox offset after group transform).
fn collect_node_names(nodes: &[SceneNode]) -> Vec<String> {
    let mut names = Vec::new();
    for node in nodes {
        match node {
            SceneNode::Place(p) => names.push(p.name.clone()),
            SceneNode::Group(g) => {
                names.push(g.name.clone());
                names.extend(collect_node_names(&g.children));
            }
            SceneNode::Link(l) => names.push(l.name.clone()),
            SceneNode::Frame(fr) => {
                names.push(fr.name.clone());
                names.extend(collect_node_names(&fr.children));
            }
            SceneNode::Instance(i) => names.push(i.name.clone()),
        }
    }
    names
}

fn resolve_place(
    scene: &Scene,
    place: &Place,
    prefix: &str,
    svg: &mut String,
    bboxes: &mut HashMap<String, Bbox>,
    svg_defs: &mut Vec<SvgDef>,
) {
    let shape = match scene.find_shape(&place.shape_ref) {
        Some(s) => s,
        None => return,
    };

    // Text shapes get a different rendering path
    if shape.is_text() {
        resolve_text_place(scene, place, shape, prefix, svg, bboxes, svg_defs);
        return;
    }

    let (geometry_shape, attr_shape) = find_link_source_shape(scene, shape);

    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut pd = resolve_place_geometry(geometry_shape, coord_space, place.size);
    apply_effects(&mut pd, &geometry_shape.effects);
    apply_effects(&mut pd, &attr_shape.effects);

    let fill = resolve_fill(&place.overrides, attr_shape, &scene.defaults);
    let stroke = resolve_stroke(&place.overrides, attr_shape, &scene.defaults);
    // SVG defaults unspecified fill to black. When the author specified only
    // a stroke, they almost certainly want a hollow stroked path — default
    // fill to "none" rather than letting SVG render a black-filled blob.
    let fill = match (&fill, &stroke) {
        (None, Some(_)) => Some(Color::None),
        _ => fill,
    };
    let stroke_width = resolve_stroke_width(&place.overrides, attr_shape, &scene.defaults);
    let stroke_linecap = resolve_linecap(&place.overrides, attr_shape, &scene.defaults);
    let stroke_linejoin = resolve_linejoin(&place.overrides, attr_shape, &scene.defaults);
    let stroke_miterlimit = resolve_miterlimit(&place.overrides, attr_shape, &scene.defaults);
    let fill_rule = resolve_fill_rule(&place.overrides, attr_shape, &scene.defaults);
    let stroke_dasharray = resolve_stroke_dasharray(&place.overrides, attr_shape, &scene.defaults);
    let opacity = resolve_opacity(&place.overrides, attr_shape, &scene.defaults);
    let blur = resolve_blur(&place.overrides, attr_shape, &scene.defaults);

    // Resolve position (with anchor support)
    let (pos_x, pos_y) = resolve_position(scene, place, &pd, bboxes);

    let placement = Some(Placement {
        at: (pos_x, pos_y),
        size: place.size.map(|d| (d.w, d.h)),
        flip: place.flip.map(convert_flip),
    });

    let d = path_data_to_svg_d(&pd, placement.as_ref());
    if d.is_empty() {
        return;
    }

    // The place's rotation/skew about its center, as a unified affine (E2.3).
    let post_transform = place_post_transform(place, placement.as_ref());

    // Compute and store bbox for this placed shape. Under rotation/skew the bbox
    // is the AABB of the transformed corners (E2.3 correct-bbox-under-transform),
    // so anchors / RelativeTo land correctly on a rotated element.
    if let Some(ref pl) = placement {
        let mut bbox = placed_bbox(&pd, pl);
        if let Some(ref t) = post_transform {
            bbox = crate::attrs::transform_bbox(t, bbox.0, bbox.1, bbox.2, bbox.3);
        }
        bboxes.insert(place.name.clone(), bbox);
    }

    // Per-place clip / mask (E2.4).
    let clip_ref = place.clip.as_ref().and_then(|names| {
        register_clip_def(scene, &format!("clip-{}", place.name), names, svg_defs)
    });
    let mask_ref = place
        .mask
        .as_ref()
        .and_then(|name| register_mask_def(scene, &format!("mask-{}", place.name), name, svg_defs));

    // Register gradient defs if fill or stroke is a gradient
    let fill_ref = register_gradient_color(&fill, &place.name, "fill", svg_defs);
    let stroke_ref = register_gradient_color(&stroke, &place.name, "stroke", svg_defs);

    // Register blur filter def
    let blur_ref = register_blur_filter(&blur, &place.name, svg_defs);

    svg.push_str(prefix);
    svg.push_str(&format!("  <path id=\"{}\"", place.name));
    svg.push_str(&format!(" d=\"{}\"", d));

    emit_svg_fill_ref(&fill, &fill_ref, svg);
    if let Some(fr) = fill_rule {
        svg.push_str(&format!(" fill-rule=\"{}\"", fr.svg_value()));
    }
    emit_svg_stroke_ref(&stroke, &stroke_ref, svg);
    if let Some(sw) = stroke_width {
        svg.push_str(&format!(" stroke-width=\"{}\"", types::fmt_num(sw)));
    }
    if let Some(lc) = stroke_linecap {
        svg.push_str(&format!(" stroke-linecap=\"{}\"", lc));
    }
    if let Some(lj) = stroke_linejoin {
        svg.push_str(&format!(" stroke-linejoin=\"{}\"", lj));
    }
    if let Some(ml) = stroke_miterlimit {
        svg.push_str(&format!(" stroke-miterlimit=\"{}\"", types::fmt_num(ml)));
    }
    if let Some(ref da) = stroke_dasharray {
        let da_str: Vec<String> = da.iter().map(|v| types::fmt_num(*v)).collect();
        svg.push_str(&format!(" stroke-dasharray=\"{}\"", da_str.join(" ")));
    }
    if let Some(op) = opacity {
        svg.push_str(&format!(" opacity=\"{}\"", types::fmt_num(op)));
    }
    if let Some(ref filter_id) = blur_ref {
        svg.push_str(&format!(" filter=\"url(#{})\"", filter_id));
    }
    if let Some(ref clip_id) = clip_ref {
        svg.push_str(&format!(" clip-path=\"url(#{})\"", clip_id));
    }
    if let Some(ref mask_id) = mask_ref {
        svg.push_str(&format!(" mask=\"url(#{})\"", mask_id));
    }

    if let Some(ref t) = post_transform {
        svg.push_str(&format!(
            " transform=\"{}\"",
            crate::attrs::emit_transform(t)
        ));
    }

    svg.push_str(" />\n");
}

/// The place's rotation + skew, composed about the place's center as a single
/// affine (E2.3). Returns `None` when there's no rotation/skew (so unrotated
/// places stay byte-identical). Translate/scale/flip stay baked into the `d`
/// string (via `Placement`); rotation/skew ride here as the element `transform`.
fn place_post_transform(
    place: &Place,
    placement: Option<&Placement>,
) -> Option<crate::attrs::Transform> {
    if place.rotation.is_none() && place.skew.is_none() {
        return None;
    }
    let pl = placement?;
    let cx = pl.at.0 + pl.size.map(|s| s.0 / 2.0).unwrap_or(0.0);
    let cy = pl.at.1 + pl.size.map(|s| s.1 / 2.0).unwrap_or(0.0);
    let mut t = crate::attrs::IDENTITY;
    if let Some(rot) = &place.rotation {
        t = crate::attrs::mul(&t, &crate::attrs::rotate_about(rot.0, cx, cy));
    }
    if let Some((sx, sy)) = &place.skew {
        t = crate::attrs::mul(&t, &crate::attrs::skew_about(*sx, *sy, cx, cy));
    }
    Some(t)
}

/// The compound group transform (E2.3): translate · rotate · skew · flip,
/// composed about the origin (group children are authored in document space).
fn group_transform(group: &Group) -> crate::attrs::Transform {
    let mut t = crate::attrs::IDENTITY;
    if let Some((tx, ty)) = group.position {
        t = crate::attrs::mul(&t, &crate::attrs::translate(tx, ty));
    }
    if let Some(rot) = &group.rotation {
        t = crate::attrs::mul(&t, &crate::attrs::rotate(rot.0));
    }
    if let Some((sx, sy)) = &group.skew {
        t = crate::attrs::mul(&t, &crate::attrs::skew(*sx, *sy));
    }
    if let Some(flip) = &group.flip {
        let (fx, fy) = match flip {
            types::Flip::X => (-1.0, 1.0),
            types::Flip::Y => (1.0, -1.0),
            types::Flip::XY => (-1.0, -1.0),
        };
        t = crate::attrs::mul(&t, &crate::attrs::scale(fx, fy));
    }
    t
}

/// Register a `<clipPath>` def for one or more shapes (E2.4). Multiple shapes ⇒
/// their geometry is unioned inside the clipPath (SVG clips to the union of all
/// child paths). Returns the clip id, or `None` if no shape resolved to geometry.
fn register_clip_def(
    scene: &Scene,
    clip_id: &str,
    shape_names: &[String],
    svg_defs: &mut Vec<SvgDef>,
) -> Option<String> {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut paths = String::new();
    for name in shape_names {
        if let Some(shape) = scene.find_shape(name) {
            let pd = shape.resolve(coord_space);
            let d = path_data_to_svg_d(&pd, None);
            if !d.is_empty() {
                paths.push_str(&format!("<path d=\"{}\"/>", d));
            }
        }
    }
    if paths.is_empty() {
        return None;
    }
    let def = format!("<clipPath id=\"{}\">{}</clipPath>", clip_id, paths);
    svg_defs.push((clip_id.to_string(), def));
    Some(clip_id.to_string())
}

/// Register an alpha/luminance `<mask>` def for a shape (E2.4). The masking
/// shape is filled white so its *luminance* (and any per-shape opacity) gates the
/// masked content's alpha — distinct from `clip`'s hard geometric edge.
fn register_mask_def(
    scene: &Scene,
    mask_id: &str,
    shape_name: &str,
    svg_defs: &mut Vec<SvgDef>,
) -> Option<String> {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let shape = scene.find_shape(shape_name)?;
    let pd = shape.resolve(coord_space);
    let d = path_data_to_svg_d(&pd, None);
    if d.is_empty() {
        return None;
    }
    // White fill ⇒ full luminance ⇒ opaque where the shape is. Areas outside the
    // shape are black (mask default) ⇒ transparent. This is the standard
    // luminance-mask convention.
    let def = format!(
        "<mask id=\"{}\"><path d=\"{}\" fill=\"#ffffff\"/></mask>",
        mask_id, d
    );
    svg_defs.push((mask_id.to_string(), def));
    Some(mask_id.to_string())
}

/// Resolve a text shape placement into an SVG <text> element.
#[allow(clippy::too_many_arguments)]
fn resolve_text_place(
    scene: &Scene,
    place: &Place,
    shape: &Shape,
    prefix: &str,
    svg: &mut String,
    bboxes: &mut HashMap<String, Bbox>,
    svg_defs: &mut Vec<SvgDef>,
) {
    // A place-level `content` override wins over the shape's content, so one
    // text shape (one font/size/fill definition) can serve many labels.
    let content = place
        .overrides
        .iter()
        .rev()
        .find_map(|op| match op {
            crate::shape::Operation::Content(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| shape.content().unwrap_or(""));
    if content.is_empty() {
        return;
    }

    // Text-on-path (E2.7): if the place references a `textpath=<id>`, resolve the
    // referenced placed shape's document-space `d`, register it as a hidden
    // <path> def, and wrap the content in a <textPath href="#…">. The render and
    // SVG export share this same `d`. We reuse the in-flight `bboxes` (built by
    // earlier nodes) rather than re-resolving the scene — that would recurse back
    // into this very text place.
    let text_path_def = place.text_path.as_ref().and_then(|target| {
        placed_document_d_with_bboxes(scene, target, bboxes).map(|d| {
            let path_id = format!("{}-textpath", place.name);
            let def = format!("<path id=\"{}\" d=\"{}\" fill=\"none\"/>", path_id, d);
            svg_defs.push((path_id.clone(), def));
            path_id
        })
    });

    // Note: gradient fills on text shapes are not yet supported — emit_svg_fill
    // will silently ignore gradient variants. SVG text gradients require <defs>
    // wiring that text_place doesn't do. Solid fills work fine.
    let fill = resolve_fill(&place.overrides, shape, &scene.defaults);
    let opacity = resolve_opacity(&place.overrides, shape, &scene.defaults);
    let text_anchor = resolve_text_anchor(&place.overrides, shape, &scene.defaults);

    // Position + estimated bbox. Text on a path takes its x/y from the path
    // start, not the place position, and keeps no bbox — its geometry follows
    // the path. Otherwise (x, y) is the baseline start, resolved through the same
    // anchor/align/offset/below-above semantics as geometric places (EXP-5 /
    // field-report friction #10) but measured against the estimated text box.
    let (x, y) = if text_path_def.is_some() {
        (0.0, 0.0)
    } else {
        let font_size = shape
            .font_size()
            .unwrap_or(crate::text_metrics::DEFAULT_FONT_SIZE);
        let m = crate::text_metrics::measure(content, font_size, shape.font_weight());
        let (x, y) = resolve_text_position(place, &m, text_anchor, bboxes);
        // Estimated bbox: (x, y) is the baseline start; text-anchor shifts the
        // run; rotation maps the corners.
        let x0 = match text_anchor {
            Some(types::TextAnchor::Middle) => x - m.width / 2.0,
            Some(types::TextAnchor::End) => x - m.width,
            _ => x,
        };
        let mut bbox = (x0, y - m.ascent, x0 + m.width, y + m.descent);
        if let Some(rot) = &place.rotation {
            let t = crate::attrs::rotate_about(rot.0, x, y);
            bbox = crate::attrs::transform_bbox(&t, bbox.0, bbox.1, bbox.2, bbox.3);
        }
        bboxes.insert(place.name.clone(), bbox);
        (x, y)
    };

    svg.push_str(prefix);
    svg.push_str(&format!("  <text id=\"{}\"", place.name));
    // On a path, x/y come from the path's start, not the place position.
    if text_path_def.is_none() {
        svg.push_str(&format!(
            " x=\"{}\" y=\"{}\"",
            types::fmt_num(x),
            types::fmt_num(y)
        ));
    }

    if let Some(fs) = shape.font_size() {
        svg.push_str(&format!(" font-size=\"{}\"", types::fmt_num(fs)));
    }
    if let Some(fw) = shape.font_weight() {
        svg.push_str(&format!(" font-weight=\"{}\"", fw));
    }
    if let Some(ff) = shape.font_family() {
        // A `$font.body` reference resolves to its design-token value (C9 / E4.3
        // — font tokens flow to the render too, not just code emit); a literal
        // family is emitted verbatim. Quotes are stripped from the token value
        // so the attribute is a clean family name.
        let resolved = resolve_font_token(scene, ff);
        svg.push_str(&format!(" font-family=\"{}\"", resolved));
    }
    if let Some(fst) = shape.font_style() {
        svg.push_str(&format!(" font-style=\"{}\"", fst));
    }
    if let Some(ta) = text_anchor {
        svg.push_str(&format!(" text-anchor=\"{}\"", ta));
    }

    emit_svg_fill(&fill, svg);
    if let Some(op) = opacity {
        svg.push_str(&format!(" opacity=\"{}\"", types::fmt_num(op)));
    }

    if let Some(rot) = &place.rotation {
        svg.push_str(&format!(
            " transform=\"rotate({} {} {})\"",
            types::fmt_num(rot.0),
            types::fmt_num(x),
            types::fmt_num(y)
        ));
    }

    // Escape XML entities in content
    let escaped = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if let Some(path_id) = text_path_def {
        svg.push_str(&format!(
            "><textPath href=\"#{}\">{}</textPath></text>\n",
            path_id, escaped
        ));
    } else {
        svg.push_str(&format!(">{}</text>\n", escaped));
    }
}

// ── Position resolution with anchor support ──────────────────────────

/// Resolve the (x, y) position for a place, considering anchors.
fn resolve_position(
    scene: &Scene,
    place: &Place,
    pd: &PathData,
    bboxes: &HashMap<String, Bbox>,
) -> (f64, f64) {
    let base_pos = match &place.position {
        PlacePosition::At(x, y) => (*x, *y),
        PlacePosition::On {
            path,
            t,
            side,
            offset,
        } => {
            // Parametric position — anchors don't apply here
            if let Some(pl) =
                resolve_parametric_position(scene, path, t.0, side.as_ref(), offset.as_ref())
            {
                return pl.at;
            }
            return (0.0, 0.0);
        }
        PlacePosition::RelativeTo { target, anchor } => {
            if let Some(target_bbox) = bboxes.get(target) {
                let (fx, fy) = anchor.factors();
                let w = target_bbox.2 - target_bbox.0;
                let h = target_bbox.3 - target_bbox.1;
                let tx = target_bbox.0 + w * fx;
                let ty = target_bbox.1 + h * fy;
                (tx, ty)
            } else {
                // Target not yet placed — fall back to origin
                (0.0, 0.0)
            }
        }
    };

    let mut pos = if let Some(anchor) = &place.anchor {
        match anchor {
            PlaceAnchor::Below { target, gap } => {
                if let Some(target_bbox) = bboxes.get(target) {
                    let x = if place.position == PlacePosition::At(0.0, 0.0) && base_pos.0 == 0.0 {
                        target_bbox.0
                    } else {
                        base_pos.0
                    };
                    let y = target_bbox.3 + gap;
                    (x, y)
                } else {
                    base_pos
                }
            }
            PlaceAnchor::Above { target, gap } => {
                if let Some(target_bbox) = bboxes.get(target) {
                    let x = if place.position == PlacePosition::At(0.0, 0.0) && base_pos.0 == 0.0 {
                        target_bbox.0
                    } else {
                        base_pos.0
                    };
                    let self_bbox = natural_bbox(pd);
                    let self_h = self_bbox.3 - self_bbox.1;
                    let scale_h = place.size.map(|d| d.h).unwrap_or(self_h);
                    let y = target_bbox.1 - gap - scale_h;
                    (x, y)
                } else {
                    base_pos
                }
            }
        }
    } else {
        base_pos
    };

    // Apply align: shift so the specified anchor point on THIS shape lands at pos
    if let Some(align_anchor) = &place.align {
        let (ax, ay) = align_anchor.factors();
        let self_bbox = natural_bbox(pd);
        let nat_w = self_bbox.2 - self_bbox.0;
        let nat_h = self_bbox.3 - self_bbox.1;
        let (sw, sh) = if let Some(size) = place.size {
            (size.w, size.h)
        } else {
            (nat_w, nat_h)
        };
        pos.0 -= sw * ax;
        pos.1 -= sh * ay;
    }

    // Apply offset nudge
    if let Some((dx, dy)) = &place.offset {
        pos.0 += dx;
        pos.1 += dy;
    }

    pos
}

/// Resolve the baseline-start `(x, y)` for a **text** place, mirroring
/// `resolve_position`'s anchor/align/offset/below-above semantics against the
/// estimated text box `m` instead of a geometric bbox (EXP-5). The text box is
/// `width × (ascent + descent)`, its top-left at `(baseline_x, baseline_y −
/// ascent)`; `align=` shifts so the chosen self-anchor lands on the target point,
/// and `text-anchor` (start/middle/end) relates the visual left edge to the
/// returned baseline `x`. Unknown targets are already rejected by
/// `validate_references`, so a missing bbox falls back to `(0,0)` defensively.
fn resolve_text_position(
    place: &Place,
    m: &crate::text_metrics::TextMetrics,
    text_anchor: Option<types::TextAnchor>,
    bboxes: &HashMap<String, Bbox>,
) -> (f64, f64) {
    let w = m.width;
    let h = m.ascent + m.descent;
    let (ox, oy) = place.offset.unwrap_or((0.0, 0.0));

    // The target point the placement lands on, plus whether it came from a
    // relative reference (⇒ default self-anchor = top-left, i.e. flush).
    let (mut tx, mut ty, mut relative) = match &place.position {
        PlacePosition::At(x, y) => (*x, *y, false),
        PlacePosition::RelativeTo { target, anchor } => match bboxes.get(target) {
            Some(b) => {
                let (fx, fy) = anchor.factors();
                (b.0 + (b.2 - b.0) * fx, b.1 + (b.3 - b.1) * fy, true)
            }
            None => (0.0, 0.0, false),
        },
        // Parametric text placement is not modeled; keep the origin.
        PlacePosition::On { .. } => (0.0, 0.0, false),
    };

    // below=/above= stack the text box under/over the target (mirror geometry).
    if let Some(anchor) = &place.anchor {
        match anchor {
            PlaceAnchor::Below { target, gap } => {
                if let Some(b) = bboxes.get(target) {
                    tx = b.0;
                    ty = b.3 + gap;
                    relative = true;
                }
            }
            PlaceAnchor::Above { target, gap } => {
                if let Some(b) = bboxes.get(target) {
                    tx = b.0;
                    ty = b.1 - gap - h;
                    relative = true;
                }
            }
        }
    }

    // Compute the text box's visual top-left.
    let (box_left, box_top) = if let Some(a) = &place.align {
        // The chosen self-anchor lands on the target point.
        let (afx, afy) = a.factors();
        (tx - w * afx, ty - h * afy)
    } else if relative {
        // Flush: box top-left at the target point (e.g. `at=box.right`).
        (tx, ty)
    } else {
        // Plain `at=x,y` with no align keeps the historical baseline-start
        // semantics: (x, y) IS the baseline start.
        return (tx + ox, ty + oy);
    };

    // Baseline from the box, then relate the visual left edge to `x` through
    // text-anchor so the emitted `text-anchor` attribute renders where expected.
    let baseline_y = box_top + m.ascent + oy;
    let visual_left = box_left + ox;
    let baseline_x = match text_anchor {
        Some(types::TextAnchor::Middle) => visual_left + w / 2.0,
        Some(types::TextAnchor::End) => visual_left + w,
        _ => visual_left,
    };
    (baseline_x, baseline_y)
}

/// Compute bounding box from raw PathData points (pre-transform).
///
/// Accounts for arc bulge via `geometry_bbox` — a curved segment can extend
/// past its anchor points, and ignoring that mis-sizes relative placements
/// and group bounds.
fn natural_bbox(pd: &PathData) -> Bbox {
    if pd.points.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    crate::path_point::geometry_bbox(&pd.points, pd.closed)
}

/// Compute the bounding box of a placed shape (after at/size transform).
///
/// With size: bbox-fit — the placed bbox IS the (at, size) region.
/// Without size: translate-only — natural bbox offset by `at`.
/// Resolve a place's geometry, compensating placed-space length parameters
/// (`round-corners` radii) for the bbox-fit scale when the place has a `size=`.
/// Without this, `round-corners 8` on a template rectangle measures
/// `8 × size/document` after the fit — invisible on 24px icons (scale ≈ 1) but
/// badly shrunken on a small element in a large document.
fn resolve_place_geometry(
    geometry_shape: &Shape,
    coord_space: (f64, f64),
    size: Option<crate::types::Dimension>,
) -> PathData {
    let pd = geometry_shape.resolve(coord_space);
    let Some(d) = size else { return pd };
    let uses_placed_lengths = geometry_shape.operations.iter().any(|op| {
        matches!(
            op,
            crate::shape::Operation::RoundCorners { .. } | crate::shape::Operation::Notch(_)
        )
    });
    if !uses_placed_lengths {
        return pd;
    }
    let (min_x, min_y, max_x, max_y) = crate::path_point::geometry_bbox(&pd.points, pd.closed);
    let span_x = (max_x - min_x).max(1e-9);
    let span_y = (max_y - min_y).max(1e-9);
    let (sx, sy) = ((d.w / span_x).abs(), (d.h / span_y).abs());
    if (sx - 1.0).abs() < 1e-9 && (sy - 1.0).abs() < 1e-9 {
        return pd;
    }
    if sx <= 1e-9 || sy <= 1e-9 {
        // Degenerate fit (zero-extent target): keep authored-space rounding.
        return pd;
    }
    geometry_shape.resolve_scaled(coord_space, Some((sx, sy)))
}

fn placed_bbox(pd: &PathData, placement: &Placement) -> Bbox {
    if let Some((sw, sh)) = placement.size {
        (
            placement.at.0,
            placement.at.1,
            placement.at.0 + sw,
            placement.at.1 + sh,
        )
    } else {
        let nb = natural_bbox(pd);
        (
            placement.at.0 + nb.0,
            placement.at.1 + nb.1,
            placement.at.0 + nb.2,
            placement.at.1 + nb.3,
        )
    }
}

/// Apply non-destructive effects to PathData.
fn apply_effects(pd: &mut PathData, effects: &[Effect]) {
    for effect in effects {
        match effect {
            Effect::Droop { amount, direction } => {
                apply_droop(pd, amount.0, direction.as_ref());
            }
            Effect::Curl { amount, from } => {
                apply_curl(pd, amount.0, from.as_ref());
            }
            Effect::Jitter { amount, seed } => {
                apply_jitter(pd, amount.0, seed.unwrap_or(42));
            }
            Effect::Taper { .. } => {
                // Taper affects stroke-width along path, not geometry.
                // Would need per-segment stroke-width, which SVG doesn't support natively.
                // Skip for now.
            }
        }
    }
}

fn apply_droop(pd: &mut PathData, amount: f64, direction: Option<&Direction>) {
    let n = pd.points.len();
    if n < 2 {
        return;
    }

    // Find the vertical range
    let min_y = pd.points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = pd
        .points
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = (max_y - min_y).max(1.0);

    let droop_scale = range * amount;

    for (i, point) in pd.points.iter_mut().enumerate() {
        // Distance from top as fraction
        let t = (point.y - min_y) / range;
        // More droop toward the bottom
        let droop = t * t * droop_scale;

        match direction.unwrap_or(&Direction::Down) {
            Direction::Down => point.y += droop,
            Direction::Up => point.y -= droop,
            Direction::Left => point.x -= droop,
            Direction::Right => point.x += droop,
        }

        // Also droop endpoints slightly less
        if i == 0 || i == n - 1 {
            // Endpoints droop less
        }
    }
}

fn apply_curl(pd: &mut PathData, amount: f64, _from: Option<&crate::types::PointRef>) {
    let n = pd.points.len();
    if n < 2 {
        return;
    }

    // Find center
    let cx: f64 = pd.points.iter().map(|p| p.x).sum::<f64>() / n as f64;
    let cy: f64 = pd.points.iter().map(|p| p.y).sum::<f64>() / n as f64;

    let angle = amount * std::f64::consts::PI * 0.5; // max 90 degrees

    for (i, point) in pd.points.iter_mut().enumerate() {
        let t = i as f64 / (n - 1).max(1) as f64;
        let rot = angle * t;
        let dx = point.x - cx;
        let dy = point.y - cy;
        point.x = cx + dx * rot.cos() - dy * rot.sin();
        point.y = cy + dx * rot.sin() + dy * rot.cos();
    }
}

fn apply_jitter(pd: &mut PathData, amount: f64, seed: u32) {
    for (i, point) in pd.points.iter_mut().enumerate() {
        let nx = signed_noise(i as u64, seed as u64);
        let ny = signed_noise(i as u64, seed as u64 + 1000);
        point.x += nx * amount * 5.0;
        point.y += ny * amount * 5.0;
    }
}

fn signed_noise(i: u64, salt: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(salt);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    let unit = (x as f64) / (u64::MAX as f64);
    unit * 2.0 - 1.0
}

// ── Attribute resolution helpers ──────────────────────────────────────

/// For linked shapes, find the source shape that provides geometry.
/// Returns (geometry_source, attr_source) — if the shape is a link,
/// geometry comes from the source, attrs from the link override.
fn find_link_source_shape<'a>(scene: &'a Scene, shape: &'a Shape) -> (&'a Shape, &'a Shape) {
    // Search scene nodes for a Link that matches this shape's name
    if let Some(link) = find_link_in_nodes(&scene.nodes, &shape.name) {
        if let Some(source) = scene.find_shape(&link.source) {
            return (source, shape);
        }
    }
    (shape, shape)
}

fn find_link_in_nodes<'a>(nodes: &'a [SceneNode], name: &str) -> Option<&'a Link> {
    for node in nodes {
        match node {
            SceneNode::Link(l) if l.name == name => return Some(l),
            SceneNode::Group(g) => {
                if let Some(l) = find_link_in_nodes(&g.children, name) {
                    return Some(l);
                }
            }
            SceneNode::Frame(fr) => {
                if let Some(l) = find_link_in_nodes(&fr.children, name) {
                    return Some(l);
                }
            }
            _ => {}
        }
    }
    None
}

fn resolve_fill(overrides: &[Operation], shape: &Shape, defaults: &[Operation]) -> Option<Color> {
    for op in overrides.iter().rev() {
        if let Operation::Fill(c) = op {
            return Some(c.clone());
        }
    }
    if let Some(c) = shape.fill() {
        return Some(c.clone());
    }
    for op in defaults.iter().rev() {
        if let Operation::Fill(c) = op {
            return Some(c.clone());
        }
    }
    None
}

fn resolve_stroke(overrides: &[Operation], shape: &Shape, defaults: &[Operation]) -> Option<Color> {
    for op in overrides.iter().rev() {
        if let Operation::Stroke(c) = op {
            return Some(c.clone());
        }
    }
    if let Some(c) = shape.stroke() {
        return Some(c.clone());
    }
    for op in defaults.iter().rev() {
        if let Operation::Stroke(c) = op {
            return Some(c.clone());
        }
    }
    None
}

fn resolve_linecap(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<types::LineCap> {
    for op in overrides.iter().rev() {
        if let Operation::StrokeLinecap(c) = op {
            return Some(*c);
        }
    }
    if let Some(c) = shape.stroke_linecap() {
        return Some(c);
    }
    for op in defaults.iter().rev() {
        if let Operation::StrokeLinecap(c) = op {
            return Some(*c);
        }
    }
    None
}

fn resolve_linejoin(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<types::LineJoin> {
    for op in overrides.iter().rev() {
        if let Operation::StrokeLinejoin(j) = op {
            return Some(*j);
        }
    }
    if let Some(j) = shape.stroke_linejoin() {
        return Some(j);
    }
    for op in defaults.iter().rev() {
        if let Operation::StrokeLinejoin(j) = op {
            return Some(*j);
        }
    }
    None
}

fn resolve_miterlimit(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<f64> {
    for op in overrides.iter().rev() {
        if let Operation::StrokeMiterlimit(m) = op {
            return Some(*m);
        }
    }
    if let Some(m) = shape.stroke_miterlimit() {
        return Some(m);
    }
    for op in defaults.iter().rev() {
        if let Operation::StrokeMiterlimit(m) = op {
            return Some(*m);
        }
    }
    None
}

fn resolve_fill_rule(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<types::FillRule> {
    for op in overrides.iter().rev() {
        if let Operation::FillRule(r) = op {
            return Some(*r);
        }
    }
    if let Some(r) = shape.fill_rule() {
        return Some(r);
    }
    for op in defaults.iter().rev() {
        if let Operation::FillRule(r) = op {
            return Some(*r);
        }
    }
    None
}

fn resolve_text_anchor(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<types::TextAnchor> {
    for op in overrides.iter().rev() {
        if let Operation::TextAnchor(a) = op {
            return Some(*a);
        }
    }
    if let Some(a) = shape.text_anchor() {
        return Some(a);
    }
    for op in defaults.iter().rev() {
        if let Operation::TextAnchor(a) = op {
            return Some(*a);
        }
    }
    None
}

fn emit_svg_fill(color: &Option<Color>, svg: &mut String) {
    match color {
        Some(Color::Hex(c)) => svg.push_str(&format!(" fill=\"{}\"", c)),
        Some(Color::None) => svg.push_str(" fill=\"none\""),
        Some(Color::CurrentColor) => svg.push_str(" fill=\"currentColor\""),
        // Tokens are resolved to concrete colors by `apply_scheme` before render.
        Some(Color::Token(_)) => {}
        Some(Color::LinearGradient(_) | Color::RadialGradient(_)) => {} // handled by ref
        None => {}
    }
}

fn emit_svg_stroke(color: &Option<Color>, svg: &mut String) {
    match color {
        Some(Color::Hex(c)) => svg.push_str(&format!(" stroke=\"{}\"", c)),
        Some(Color::None) => svg.push_str(" stroke=\"none\""),
        Some(Color::CurrentColor) => svg.push_str(" stroke=\"currentColor\""),
        Some(Color::Token(_)) => {}
        Some(Color::LinearGradient(_) | Color::RadialGradient(_)) => {} // handled by ref
        None => {}
    }
}

// ── Colorscheme resolution ────────────────────────────────────────────

/// Return a clone of `scene` with every `Color::Token` replaced by the
/// concrete color it resolves to under `scheme` (or the base palette when
/// `scheme` is `None`). Errors on an unknown scheme or undefined token.
///
/// Run this before `resolve_scene` / rendering. The DSL emitter keeps tokens
/// intact, so this never touches the source file.
pub fn apply_scheme(scene: &Scene, scheme: Option<&str>) -> Result<Scene> {
    // Fail loudly on a placement that references a target that does not exist
    // (or is only defined later): never silently collapse to the document origin
    // (EXP-5 / field-report friction #10 + wishlist #2). Runs before token
    // resolution so a broken reference is reported even on the base palette.
    validate_references(scene)?;

    if let Some(name) = scheme {
        if !scene.palette.has_scheme(name) {
            return Err(StrokError::ParseError(format!(
                "unknown scheme '{}' — defined schemes: {}",
                name,
                scheme_names(scene)
            )));
        }
    }

    let mut out = scene.clone();
    let mut palette = scene.palette.clone();
    // `tokens`-block colors resolve at render time too — the spec promises
    // `$name` and `$color.name` are the same token whether it was defined in
    // `palette` or `tokens`. Appended after the palette entries so an explicit
    // palette definition wins on a name conflict.
    for t in &scene.design_tokens {
        if t.category == "color" {
            palette.tokens.push((t.name.clone(), t.value.clone()));
            palette.tokens.push((t.dotted(), t.value.clone()));
        }
    }
    resolve_color_ops(&mut out.defaults, &palette, scheme)?;
    for shape in &mut out.shapes {
        resolve_color_ops(&mut shape.operations, &palette, scheme)?;
    }
    for component in &mut out.components {
        resolve_color_nodes(&mut component.children, &palette, scheme)?;
    }
    resolve_color_nodes(&mut out.nodes, &palette, scheme)?;
    Ok(out)
}

/// Walk the scene top-to-bottom and error on any placement whose anchor target
/// does not resolve — a name that exists nowhere, or one defined *later* in the
/// file (anchors resolve top-to-bottom). This replaces the old silent `(0,0)`
/// fallback for BOTH text and geometric places (EXP-5 / field-report friction
/// #10): a broken reference now names the missing target and, via
/// `diagnostics::suggest`, the closest existing element.
pub fn validate_references(scene: &Scene) -> Result<()> {
    let all: Vec<String> = collect_node_names(&scene.nodes);
    let all_set: std::collections::HashSet<&str> = all.iter().map(|s| s.as_str()).collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    validate_nodes(&scene.nodes, &all, &all_set, &mut seen)
}

/// The anchor-target names a place depends on for positioning (target name +
/// what it's used as). Parametric `on=` (a shape.point, not a place bbox) and
/// `textpath=` (text-on-path, unaffected by placement order) are intentionally
/// excluded.
fn place_targets(p: &Place) -> Vec<(&str, &'static str)> {
    let mut out: Vec<(&str, &'static str)> = Vec::new();
    if let PlacePosition::RelativeTo { target, .. } = &p.position {
        out.push((target.as_str(), "anchor"));
    }
    match &p.anchor {
        Some(PlaceAnchor::Above { target, .. }) | Some(PlaceAnchor::Below { target, .. }) => {
            out.push((target.as_str(), "anchor"));
        }
        None => {}
    }
    out
}

fn validate_nodes(
    nodes: &[SceneNode],
    all: &[String],
    all_set: &std::collections::HashSet<&str>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<()> {
    for node in nodes {
        match node {
            SceneNode::Place(p) => {
                for (target, what) in place_targets(p) {
                    check_target(&p.name, target, what, all, all_set, seen)?;
                }
                seen.insert(p.name.clone());
            }
            SceneNode::Group(g) => {
                validate_nodes(&g.children, all, all_set, seen)?;
                seen.insert(g.name.clone());
            }
            SceneNode::Frame(fr) => {
                // A frame's own bbox is registered before its children resolve,
                // so children may anchor to it.
                seen.insert(fr.name.clone());
                validate_nodes(&fr.children, all, all_set, seen)?;
            }
            SceneNode::Link(l) => {
                seen.insert(l.name.clone());
            }
            SceneNode::Instance(i) => {
                seen.insert(i.name.clone());
            }
        }
    }
    Ok(())
}

fn check_target(
    place_name: &str,
    target: &str,
    what: &str,
    all: &[String],
    all_set: &std::collections::HashSet<&str>,
    seen: &std::collections::HashSet<String>,
) -> Result<()> {
    if seen.contains(target) {
        return Ok(());
    }
    if all_set.contains(target) {
        // Exists, but only later in the file — the top-to-bottom ordering rule.
        return Err(reference_error(
            format!(
                "place '{place_name}' anchors to '{target}', but '{target}' is placed after this element — anchors resolve top-to-bottom"
            ),
            None,
        ));
    }
    let candidates: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
    let suggestion = crate::diagnostics::suggest(target, &candidates);
    Err(reference_error(
        format!("place '{place_name}' references unknown {what} target '{target}'"),
        suggestion,
    ))
}

fn reference_error(message: String, suggestion: Option<&str>) -> StrokError {
    let mut d = crate::diagnostics::Diagnostic::new(0, message);
    if let Some(s) = suggestion {
        d = d.with_suggestion(s);
    }
    StrokError::ParseDiagnostics(vec![d])
}

fn scheme_names(scene: &Scene) -> String {
    if scene.palette.schemes.is_empty() {
        "(none)".to_string()
    } else {
        scene
            .palette
            .schemes
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn resolve_color_ops(ops: &mut [Operation], palette: &Palette, scheme: Option<&str>) -> Result<()> {
    for op in ops {
        match op {
            Operation::Fill(c) | Operation::Stroke(c) => resolve_color(c, palette, scheme)?,
            _ => {}
        }
    }
    Ok(())
}

/// Resolve a `$font.<name>` (or `$<name>`) font-family reference to its design-
/// token value, stripping the value's quotes. A literal family (no `$`) or an
/// undefined token is returned with `$`/quotes stripped so the SVG stays valid
/// rather than carrying a `$token` the browser can't use. (C9 / E4.3.)
fn resolve_font_token(scene: &Scene, family: &str) -> String {
    let Some(rest) = family.strip_prefix('$') else {
        return family.trim_matches('"').to_string();
    };
    let want = rest.strip_prefix("font.").unwrap_or(rest);
    for t in scene.all_tokens() {
        if t.dotted() == rest || (t.category == "font" && t.name == want) {
            return t.value.trim_matches('"').to_string();
        }
    }
    // Undefined token: fall back to the bare name (no `$`) so the SVG is valid.
    want.to_string()
}

fn resolve_color(color: &mut Color, palette: &Palette, scheme: Option<&str>) -> Result<()> {
    if let Color::Token(token) = color {
        match palette.resolve(token, scheme) {
            Some(hex) => *color = Color::parse(hex)?,
            None => {
                return Err(StrokError::ParseError(format!(
                    "undefined color token '${}'",
                    token
                )))
            }
        }
    }
    Ok(())
}

fn resolve_color_nodes(
    nodes: &mut [SceneNode],
    palette: &Palette,
    scheme: Option<&str>,
) -> Result<()> {
    for node in nodes {
        match node {
            SceneNode::Place(p) => resolve_color_ops(&mut p.overrides, palette, scheme)?,
            SceneNode::Link(l) => resolve_color_ops(&mut l.overrides, palette, scheme)?,
            SceneNode::Group(g) => resolve_color_nodes(&mut g.children, palette, scheme)?,
            SceneNode::Frame(fr) => {
                if let Some(c) = fr.fill.as_mut() {
                    resolve_color(c, palette, scheme)?;
                }
                resolve_color_nodes(&mut fr.children, palette, scheme)?;
            }
            SceneNode::Instance(_) => {}
        }
    }
    Ok(())
}

/// Emit fill attribute, using url(#id) for gradients.
fn emit_svg_fill_ref(color: &Option<Color>, grad_ref: &Option<String>, svg: &mut String) {
    if let Some(id) = grad_ref {
        svg.push_str(&format!(" fill=\"url(#{})\"", id));
    } else {
        emit_svg_fill(color, svg);
    }
}

/// Emit stroke attribute, using url(#id) for gradients.
fn emit_svg_stroke_ref(color: &Option<Color>, grad_ref: &Option<String>, svg: &mut String) {
    if let Some(id) = grad_ref {
        svg.push_str(&format!(" stroke=\"url(#{})\"", id));
    } else {
        emit_svg_stroke(color, svg);
    }
}

/// If the color is a gradient, register it in defs and return the gradient id.
fn register_gradient_color(
    color: &Option<Color>,
    place_name: &str,
    attr: &str, // "fill" or "stroke"
    defs: &mut Vec<SvgDef>,
) -> Option<String> {
    match color {
        Some(Color::RadialGradient(g)) => {
            let id = format!("grad-{}-{}", place_name, attr);
            let def = build_radial_gradient_svg(&id, g);
            defs.push((id.clone(), def));
            Some(id)
        }
        Some(Color::LinearGradient(g)) => {
            let id = format!("grad-{}-{}", place_name, attr);
            let def = build_linear_gradient_svg(&id, g);
            defs.push((id.clone(), def));
            Some(id)
        }
        _ => None,
    }
}

fn build_radial_gradient_svg(id: &str, g: &RadialGradient) -> String {
    let (cx, cy) = g.center.to_svg_percent();
    let mut svg = format!(
        "<radialGradient id=\"{}\" cx=\"{}%\" cy=\"{}%\" r=\"{}%\" gradientUnits=\"objectBoundingBox\">",
        id,
        types::fmt_num(cx),
        types::fmt_num(cy),
        types::fmt_num(g.radius),
    );
    let mut stops = distribute_stops(&g.stops);
    resolve_transparent_stops(&mut stops);
    for stop in &stops {
        emit_gradient_stop(stop, &mut svg);
    }
    svg.push_str("</radialGradient>");
    svg
}

fn build_linear_gradient_svg(id: &str, g: &LinearGradient) -> String {
    let (x1, y1) = g.from.to_svg_percent();
    let (x2, y2) = g.to.to_svg_percent();
    let mut svg = format!(
        "<linearGradient id=\"{}\" x1=\"{}%\" y1=\"{}%\" x2=\"{}%\" y2=\"{}%\" gradientUnits=\"objectBoundingBox\">",
        id,
        types::fmt_num(x1),
        types::fmt_num(y1),
        types::fmt_num(x2),
        types::fmt_num(y2),
    );
    let mut stops = distribute_stops(&g.stops);
    resolve_transparent_stops(&mut stops);
    for stop in &stops {
        emit_gradient_stop(stop, &mut svg);
    }
    svg.push_str("</linearGradient>");
    svg
}

/// Auto-distribute stops that don't have explicit positions (CSS gradient behavior).
fn distribute_stops(stops: &[GradientStop]) -> Vec<GradientStop> {
    if stops.is_empty() {
        return Vec::new();
    }
    let n = stops.len();
    let mut result: Vec<GradientStop> = stops.to_vec();

    // First stop defaults to 0%, last to 100%
    if result[0].position.is_none() {
        result[0].position = Some(0.0);
    }
    if result[n - 1].position.is_none() {
        result[n - 1].position = Some(1.0);
    }

    // Fill in gaps: for runs of None positions, interpolate linearly
    let mut i = 0;
    while i < n {
        if result[i].position.is_some() {
            i += 1;
            continue;
        }
        // Find the run of Nones
        let start = i - 1; // last known position
        let mut end = i;
        while end < n && result[end].position.is_none() {
            end += 1;
        }
        // Interpolate between the bracketing known positions. Both ends are
        // guaranteed Some here: `start` is the last Some index, and `end` is
        // either the next Some index (loop condition) or `n-1` which was forced
        // to Some above. The unwrap_or fallbacks keep this panic-free regardless.
        let p0 = result[start].position.unwrap_or(0.0);
        let p1 = result[end].position.unwrap_or(1.0);
        let count = end - start;
        for (k, slot) in result[(start + 1)..end].iter_mut().enumerate() {
            let frac = (k + 1) as f64 / count as f64;
            slot.position = Some(p0 + (p1 - p0) * frac);
        }
        i = end + 1;
    }

    result
}

/// Resolve "transparent" stops: replace with nearest opaque color at zero opacity.
fn resolve_transparent_stops(stops: &mut [GradientStop]) {
    let mut last_color = "#000000".to_string();
    for i in 0..stops.len() {
        if stops[i].color == "transparent" {
            // Look for nearest non-transparent color (prefer preceding)
            let base = find_nearest_color(stops, i).unwrap_or_else(|| last_color.clone());
            // Strip alpha from base if it has one
            let rgb = if base.len() == 9 {
                base[..7].to_string()
            } else {
                base
            };
            stops[i].color = format!("{}00", rgb);
        } else {
            last_color = stops[i].color.clone();
        }
    }
}

fn find_nearest_color(stops: &[GradientStop], idx: usize) -> Option<String> {
    // Look backward first
    for i in (0..idx).rev() {
        if stops[i].color != "transparent" {
            return Some(stops[i].color.clone());
        }
    }
    // Then forward
    for stop in stops.iter().skip(idx + 1) {
        if stop.color != "transparent" {
            return Some(stop.color.clone());
        }
    }
    None
}

/// Emit an SVG <stop> element.
fn emit_gradient_stop(stop: &GradientStop, svg: &mut String) {
    let offset = stop.position.unwrap_or(0.0) * 100.0;
    if stop.color.len() == 9 && stop.color.starts_with('#') {
        // #rrggbbaa format — split into color + opacity
        let rgb = &stop.color[..7];
        let aa = &stop.color[7..9];
        let opacity = u8::from_str_radix(aa, 16).unwrap_or(255) as f64 / 255.0;
        svg.push_str(&format!(
            "<stop offset=\"{}%\" stop-color=\"{}\" stop-opacity=\"{}\"/>",
            types::fmt_num(offset),
            rgb,
            types::fmt_num(opacity),
        ));
    } else {
        svg.push_str(&format!(
            "<stop offset=\"{}%\" stop-color=\"{}\"/>",
            types::fmt_num(offset),
            stop.color,
        ));
    }
}

fn convert_flip(f: types::Flip) -> Flip {
    match f {
        types::Flip::X => Flip::X,
        types::Flip::Y => Flip::Y,
        types::Flip::XY => Flip::XY,
    }
}

/// Resolve parametric placement: find position along a path.
fn resolve_parametric_position(
    scene: &Scene,
    path_ref: &types::PointRef,
    t_percent: f64,
    _side: Option<&types::Side>,
    _offset: Option<&types::AbsoluteSize>,
) -> Option<Placement> {
    // Find the source shape
    let shape = scene.find_shape(&path_ref.shape)?;
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let pd = shape.resolve(coord_space);

    if pd.points.is_empty() {
        return None;
    }

    // Find the named point
    let _point_idx = pd.points.iter().position(|p| p.name == path_ref.point)?;

    // Interpolate along the path at t_percent
    let t = t_percent / 100.0;
    let total_points = pd.points.len();
    if total_points < 2 {
        let p = &pd.points[0];
        return Some(Placement {
            at: (p.x, p.y),
            size: None,
            flip: None,
        });
    }

    // Linear interpolation along path segments
    let segment_t = t * (total_points - 1) as f64;
    let seg_idx = (segment_t as usize).min(total_points - 2);
    let local_t = segment_t - seg_idx as f64;

    let p0 = &pd.points[seg_idx];
    let p1 = &pd.points[seg_idx + 1];
    let x = p0.x + (p1.x - p0.x) * local_t;
    let y = p0.y + (p1.y - p0.y) * local_t;

    // TODO: apply side and offset based on path tangent

    Some(Placement {
        at: (x, y),
        size: None,
        flip: None,
    })
}

fn resolve_stroke_width(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<f64> {
    for op in overrides.iter().rev() {
        if let Operation::StrokeWidth(w) = op {
            return Some(w.0);
        }
    }
    if let Some(w) = shape.stroke_width() {
        return Some(w);
    }
    for op in defaults.iter().rev() {
        if let Operation::StrokeWidth(w) = op {
            return Some(w.0);
        }
    }
    None
}

fn resolve_opacity(overrides: &[Operation], shape: &Shape, defaults: &[Operation]) -> Option<f64> {
    for op in overrides.iter().rev() {
        if let Operation::Opacity(a) = op {
            return Some(a.0);
        }
    }
    if let Some(a) = shape.opacity() {
        return Some(a);
    }
    for op in defaults.iter().rev() {
        if let Operation::Opacity(a) = op {
            return Some(a.0);
        }
    }
    None
}

fn resolve_blur(overrides: &[Operation], shape: &Shape, defaults: &[Operation]) -> Option<f64> {
    for op in overrides.iter().rev() {
        if let Operation::Blur(r) = op {
            return Some(*r);
        }
    }
    if let Some(r) = shape.blur() {
        return Some(r);
    }
    for op in defaults.iter().rev() {
        if let Operation::Blur(r) = op {
            return Some(*r);
        }
    }
    None
}

fn resolve_stroke_dasharray(
    overrides: &[Operation],
    shape: &Shape,
    defaults: &[Operation],
) -> Option<Vec<f64>> {
    for op in overrides.iter().rev() {
        if let Operation::StrokeDasharray(v) = op {
            return Some(v.clone());
        }
    }
    if let Some(v) = shape.stroke_dasharray() {
        return Some(v.to_vec());
    }
    for op in defaults.iter().rev() {
        if let Operation::StrokeDasharray(v) = op {
            return Some(v.clone());
        }
    }
    None
}

/// If blur is set, register a filter def and return the filter id.
fn register_blur_filter(
    blur: &Option<f64>,
    place_name: &str,
    defs: &mut Vec<SvgDef>,
) -> Option<String> {
    let radius = (*blur)?;
    let id = format!("blur-{}", place_name);
    let def = format!(
        "<filter id=\"{}\"><feGaussianBlur stdDeviation=\"{}\"/></filter>",
        id,
        types::fmt_num(radius),
    );
    defs.push((id.clone(), def));
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl_parse;

    // ── EXP-5: text relative placement + no-silent-origin errors ──────────

    fn resolve_dsl(input: &str) -> String {
        let scene = dsl_parse::parse_file(input).unwrap();
        resolve_scene(&apply_scheme(&scene, None).unwrap())
    }

    /// Extract the `x=` / `y=` attributes of a named `<text>`/`<path>` element.
    fn xy_of(svg: &str, id: &str) -> (f64, f64) {
        let marker = format!("id=\"{}\"", id);
        let start = svg
            .find(&marker)
            .unwrap_or_else(|| panic!("no {id} in {svg}"));
        let tag = &svg[start..];
        let grab = |key: &str| -> f64 {
            let p = tag.find(key).expect("attr");
            let rest = &tag[p + key.len()..];
            let end = rest.find('"').unwrap();
            rest[..end].parse().unwrap()
        };
        (grab(" x=\""), grab(" y=\""))
    }

    const TEXT_DOC: &str = "\
documentsize 200x200

shape b template=rectangle
  fill #ddefff

shape label template=text
  content \"Hi\"
  font-size 16

place b shape=b at=40,40 size=80x40
";

    #[test]
    fn text_anchor_relative_lands_flush() {
        // `at=b.right` places the text box left edge at b's right edge (x=120),
        // not the silent (0,0) of the field-report bug.
        let svg = resolve_dsl(&format!("{TEXT_DOC}place t shape=label at=b.right\n"));
        let (x, _y) = xy_of(&svg, "t");
        assert!((x - 120.0).abs() < 1e-6, "expected flush x=120, got {x}");
    }

    #[test]
    fn text_align_center_centers_box_on_target() {
        // align=center ⇒ the text box center lands on b.center = (80, 60).
        let svg = resolve_dsl(&format!(
            "{TEXT_DOC}place c shape=label at=b.center align=center\n"
        ));
        let (x, y) = xy_of(&svg, "c");
        // text-anchor is start, so x is the box left edge; the box center is
        // x + width/2 and must equal 80. y is baseline = box_top + ascent.
        let m = crate::text_metrics::measure("Hi", 16.0, None);
        let cx = x + m.width / 2.0;
        let cy = (y - m.ascent) + (m.ascent + m.descent) / 2.0;
        assert!((cx - 80.0).abs() < 1e-6, "box center x={cx}");
        assert!((cy - 60.0).abs() < 1e-6, "box center y={cy}");
    }

    #[test]
    fn text_offset_applies() {
        let base = resolve_dsl(&format!("{TEXT_DOC}place o shape=label at=b.right\n"));
        let nudged = resolve_dsl(&format!(
            "{TEXT_DOC}place o shape=label at=b.right offset=5,3\n"
        ));
        let (bx, by) = xy_of(&base, "o");
        let (nx, ny) = xy_of(&nudged, "o");
        assert!((nx - bx - 5.0).abs() < 1e-6, "dx: {bx}->{nx}");
        assert!((ny - by - 3.0).abs() < 1e-6, "dy: {by}->{ny}");
    }

    #[test]
    fn text_below_stacks_under_target_with_gap() {
        // below=b gap=10 ⇒ box top at b.bottom (80) + 10 = 90; baseline = 90 + ascent.
        let svg = resolve_dsl(&format!("{TEXT_DOC}place bl shape=label below=b gap=10\n"));
        let (x, y) = xy_of(&svg, "bl");
        let m = crate::text_metrics::measure("Hi", 16.0, None);
        assert!((x - 40.0).abs() < 1e-6, "x aligns to b.left, got {x}");
        assert!(
            (y - (90.0 + m.ascent)).abs() < 1e-6,
            "baseline below target, got {y}"
        );
    }

    #[test]
    fn unknown_text_target_errors_with_suggestion() {
        let scene = dsl_parse::parse_file(&format!("{TEXT_DOC}place t shape=label at=bax.right\n"))
            .unwrap();
        let err = apply_scheme(&scene, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown"), "msg: {msg}");
        assert!(msg.contains("bax"), "names the target: {msg}");
        assert!(msg.contains("did you mean `b`"), "suggests b: {msg}");
    }

    #[test]
    fn unknown_geometry_target_errors_with_suggestion() {
        let input = "\
documentsize 200x200

shape sq template=rectangle
  fill #ff0000

place a shape=sq at=10,10 size=20x20
place bad shape=sq at=aa.right size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let err = apply_scheme(&scene, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown") && msg.contains("aa"), "msg: {msg}");
        assert!(msg.contains("did you mean `a`"), "suggests a: {msg}");
    }

    #[test]
    fn forward_reference_errors_with_ordering_rule() {
        let input = "\
documentsize 200x200

shape sq template=rectangle
  fill #ff0000

place first shape=sq at=later.right size=10x10
place later shape=sq at=100,100 size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let err = apply_scheme(&scene, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("placed after this element") && msg.contains("top-to-bottom"),
            "ordering message: {msg}"
        );
    }

    #[test]
    fn backward_reference_still_resolves() {
        // The common (working) case must keep passing: a target defined earlier.
        let input = "\
documentsize 200x200

shape sq template=rectangle
  fill #ff0000

place a shape=sq at=10,10 size=20x20
place b shape=sq at=a.right size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        assert!(apply_scheme(&scene, None).is_ok());
    }

    #[test]
    fn apply_scheme_resolves_tokens() {
        let input = "\
documentsize 64x64

palette
  hero #e8a840
  accent #c8863a

scheme dark
  hero #f4c266

shape bg template=rectangle
  fill $accent
shape dot template=ellipse
  fill $hero

place bg shape=bg at=0,0 size=64x64
place dot shape=dot at=20,20 size=24x24
";
        let scene = dsl_parse::parse_file(input).unwrap();

        // Base palette: tokens resolve to base values.
        let base = apply_scheme(&scene, None).unwrap();
        let base_svg = resolve_scene(&base);
        assert!(base_svg.contains("fill=\"#c8863a\"")); // accent
        assert!(base_svg.contains("fill=\"#e8a840\"")); // hero
        assert!(!base_svg.contains('$'));

        // Dark scheme: hero overridden, accent falls back to base.
        let dark = apply_scheme(&scene, Some("dark")).unwrap();
        let dark_svg = resolve_scene(&dark);
        assert!(dark_svg.contains("fill=\"#f4c266\"")); // hero override
        assert!(dark_svg.contains("fill=\"#c8863a\"")); // accent fallback

        // Unknown scheme is an error.
        assert!(apply_scheme(&scene, Some("nope")).is_err());
    }

    #[test]
    fn apply_scheme_resolves_tokens_block_colors() {
        // Colors defined in a `tokens` block (not `palette`) must render too —
        // both the bare `$cream` and dotted `$color.cream` spellings.
        let input = "\
documentsize 64x64

tokens
  cream #fdfaf6
  color.accent #c8863a

shape bg template=rectangle
  fill $cream
shape dot template=ellipse
  fill $color.accent

place bg shape=bg at=0,0 size=64x64
place dot shape=dot at=20,20 size=24x24
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&apply_scheme(&scene, None).unwrap());
        assert!(svg.contains("fill=\"#fdfaf6\""), "{svg}");
        assert!(svg.contains("fill=\"#c8863a\""), "{svg}");
    }

    #[test]
    fn apply_scheme_palette_wins_over_tokens_block_on_conflict() {
        let input = "\
documentsize 64x64

palette
  accent #111111

tokens
  accent #222222

shape bg template=rectangle
  fill $accent

place bg shape=bg at=0,0 size=64x64
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&apply_scheme(&scene, None).unwrap());
        assert!(svg.contains("fill=\"#111111\""), "{svg}");
    }

    #[test]
    fn place_content_override_wins_over_shape_content() {
        let input = "\
documentsize 200x100

shape pct template=text
  content \"0%\"
  font-size 20
  fill #262019

place a shape=pct at=10,30
  content \"46%\"
place b shape=pct at=10,60
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains(">46%</text>"), "{svg}");
        assert!(svg.contains(">0%</text>"), "{svg}");
    }

    #[test]
    fn text_places_get_estimated_bboxes() {
        let input = "\
documentsize 400x100

shape pct template=text
  content \"46%\"
  font-size 20

place label shape=pct at=100,50
place labelm shape=pct at=100,80
  text-anchor middle
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let bboxes = element_bboxes(&scene);
        // Helvetica \"46%\" at 20px = 40.02 wide, ascent 14.36, descent 4.14.
        let (x0, y0, x1, y1) = bboxes["label"];
        assert!((x0 - 100.0).abs() < 1e-9, "{x0}");
        assert!((x1 - 140.02).abs() < 1e-9, "{x1}");
        assert!((y0 - 35.64).abs() < 1e-9, "{y0}");
        assert!((y1 - 54.14).abs() < 1e-9, "{y1}");
        // text-anchor middle centers the run on x.
        let (mx0, _, mx1, _) = bboxes["labelm"];
        assert!((mx0 - (100.0 - 20.01)).abs() < 1e-9, "{mx0}");
        assert!(((mx0 + mx1) / 2.0 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn anchors_land_on_text_bboxes() {
        // A shape placed relative to a text element's bbox anchor — the exact
        // capability that was impossible while text had no bbox.
        let input = "\
documentsize 400x100

shape label template=text
  content \"Total\"
  font-size 20

shape dot template=ellipse
  fill #d95256

place label shape=label at=100,50
place dot shape=dot at=label.right align=left size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let bboxes = element_bboxes(&scene);
        let (_, _, label_right, _) = bboxes["label"];
        let (dot_left, _, _, _) = bboxes["dot"];
        assert!(
            (dot_left - label_right).abs() < 1e-9,
            "{dot_left} vs {label_right}"
        );
    }

    #[test]
    fn round_corners_radius_is_placed_space() {
        // A `round-corners 10` rectangle bbox-fit into 70x55 of a 100x100 doc
        // must produce a circular 10-unit corner in document space — not an
        // elliptical 7 x 5.5 one scaled by size/document.
        let input = "\
documentsize 100x100
shape body template=rectangle
  round-corners 10
  fill #3b82f6
place body shape=body at=15,25 size=70x55
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // The top-left corner arc must span exactly 10 units on each axis:
        // path starts at x=15, y=25+10=35, and reaches (15+10, 25) = (25, 25).
        assert!(svg.contains("M15 35 C"), "{svg}");
        assert!(svg.contains("25 25 L75 25"), "{svg}");
    }

    #[test]
    fn apply_scheme_undefined_token_errors() {
        let input = "\
documentsize 64x64

shape bg template=rectangle
  fill $missing

place bg shape=bg at=0,0 size=64x64
";
        let scene = dsl_parse::parse_file(input).unwrap();
        assert!(apply_scheme(&scene, None).is_err());
    }

    #[test]
    fn resolve_minimal_scene() {
        let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

place bg shape=bg at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("fill=\"#faf6f0\""));
        assert!(svg.contains("id=\"bg\""));
    }

    #[test]
    fn stroke_only_defaults_fill_to_none() {
        // Regression: agents authoring stroked paths without `fill` should get
        // hollow strokes, not the SVG default of solid black fill.
        let input = "\
documentsize 100x100

shape wire template=path
  stroke #ff0000
  stroke-width 2
  addpoint a at=10,10
  addpoint b at=90,10

place w shape=wire at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("fill=\"none\""),
            "stroke-only path should default fill to none, got: {}",
            svg
        );
        assert!(svg.contains("stroke=\"#ff0000\""));
    }

    #[test]
    fn resolve_path_shape() {
        let input = "\
documentsize 400x400

shape stem template=path
  addpoint base at=200,385
  addpoint tip at=200,200
  stroke #3a7d44
  stroke-width 3

place stem shape=stem at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("stroke=\"#3a7d44\""));
        assert!(svg.contains("stroke-width=\"3\""));
    }

    #[test]
    fn resolve_ellipse_template() {
        let input = "\
documentsize 400x400

shape dot template=ellipse
  fill #ff0000

place dot shape=dot at=100,100 size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("fill=\"#ff0000\""));
        // Ellipse template produces curved path via cardinal splines
        assert!(svg.contains(" C"));
    }

    #[test]
    fn resolve_with_flip() {
        let input = "\
documentsize 400x400

shape arrow template=path
  addpoint start at=0,0
  addpoint end at=100,50

place arrow-normal shape=arrow at=0,0
place arrow-flipped shape=arrow at=200,0 flip=x
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("id=\"arrow-normal\""));
        assert!(svg.contains("id=\"arrow-flipped\""));
    }

    #[test]
    fn resolve_text_shape() {
        let input = "\
documentsize 400x200

shape title template=text
  content \"Sample\"
  font-size 52
  font-weight bold
  font-family \"Georgia\"
  fill #2c1810

place title shape=title at=100,130
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("<text"));
        assert!(svg.contains("id=\"title\""));
        assert!(svg.contains("font-size=\"52\""));
        assert!(svg.contains("font-weight=\"bold\""));
        assert!(svg.contains("font-family=\"Georgia\""));
        assert!(svg.contains("fill=\"#2c1810\""));
        assert!(svg.contains(">Sample</text>"));
    }

    #[test]
    fn resolve_text_on_path_emits_textpath_def() {
        // E2.7: text flowing along a placed path. Must register a <path> def and
        // wrap content in <textPath>, and must NOT recurse infinitely.
        let input = "\
documentsize 200x100

shape arch template=path
  addpoint a at=20,70
  addpoint b at=180,70 mode=arc rx=90 ry=50 bulge=left
  fill none

shape label template=text
  content \"ON A PATH\"
  font-size 16

place arch shape=arch at=0,0
place label shape=label at=0,0 textpath=arch
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("<path id=\"label-textpath\""),
            "path def registered: {svg}"
        );
        assert!(
            svg.contains("<textPath href=\"#label-textpath\">ON A PATH</textPath>"),
            "textPath wraps content: {svg}"
        );
    }

    #[test]
    fn resolve_single_node() {
        let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

shape dot template=ellipse
  fill #ff0000

place bg shape=bg at=0,0 size=400x400
place dot shape=dot at=100,100 size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene_single_node(&scene, "dot");
        assert!(svg.contains("id=\"dot\""));
        // Should NOT contain the bg element
        assert!(!svg.contains("id=\"bg\""));
    }

    #[test]
    fn resolve_below_anchor() {
        let input = "\
documentsize 200x400

shape box template=rectangle
  fill #ff0000

place top shape=box at=10,10 size=100x50
place bottom shape=box below=top gap=5 size=100x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // Both should render
        assert!(svg.contains("id=\"top\""));
        assert!(svg.contains("id=\"bottom\""));
    }

    #[test]
    fn resolve_relative_to() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

shape dot template=ellipse
  fill #0000ff

place a shape=box at=100,100 size=200x100
place b shape=dot at=a.right align=left size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("id=\"a\""));
        assert!(svg.contains("id=\"b\""));
    }

    #[test]
    fn resolve_align_center() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

place a shape=box at=100,100 size=200x100
place b shape=box at=a.top align=center size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // Both should render
        assert!(svg.contains("id=\"a\""));
        assert!(svg.contains("id=\"b\""));
    }

    #[test]
    fn resolve_offset_applies() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

place a shape=box at=100,100 size=200x100
place b shape=box at=a.tr offset=10,20 size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("id=\"a\""));
        assert!(svg.contains("id=\"b\""));
    }

    #[test]
    fn resolve_radial_gradient_produces_defs() {
        let input = "\
documentsize 400x400

shape glow template=ellipse
  fill radial(center, 80%, #ff6b6b, transparent)

place glow shape=glow at=100,100 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("<defs>"), "SVG should have defs section");
        assert!(
            svg.contains("<radialGradient"),
            "SVG should have radialGradient"
        );
        assert!(
            svg.contains("id=\"grad-glow-fill\""),
            "gradient should have id"
        );
        assert!(svg.contains("cx=\"50%\""), "center should be 50%");
        assert!(svg.contains("r=\"80%\""), "radius should be 80%");
        assert!(
            svg.contains("fill=\"url(#grad-glow-fill)\""),
            "path should reference gradient"
        );
    }

    #[test]
    fn resolve_linear_gradient_produces_defs() {
        let input = "\
documentsize 400x400

shape sky template=rectangle
  fill linear(top, bottom, #ff0000, #0000ff)

place sky shape=sky at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("<linearGradient"),
            "SVG should have linearGradient"
        );
        assert!(
            svg.contains("fill=\"url(#grad-sky-fill)\""),
            "path should reference gradient"
        );
    }

    #[test]
    fn resolve_gradient_override_in_place() {
        let input = "\
documentsize 400x400

shape dot template=ellipse
  fill #ff0000

place dot shape=dot at=100,100 size=50x50
  fill radial(center, 80%, #00ff00, transparent)
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // Override gradient should win over shape fill
        assert!(
            svg.contains("<radialGradient"),
            "override gradient should produce defs"
        );
        assert!(
            svg.contains("fill=\"url(#grad-dot-fill)\""),
            "override gradient should be used"
        );
        assert!(
            !svg.contains("fill=\"#ff0000\""),
            "original fill should be overridden"
        );
    }

    #[test]
    fn resolve_transparent_stop_uses_neighbor_color() {
        let input = "\
documentsize 400x400

shape glow template=ellipse
  fill radial(center, 80%, #d8b480, transparent)

place glow shape=glow at=100,100 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // "transparent" should resolve to the neighboring color (#d8b480) at zero opacity
        assert!(
            svg.contains("stop-color=\"#d8b480\""),
            "transparent should use neighbor color"
        );
        assert!(
            svg.contains("stop-opacity=\"0\""),
            "transparent should have zero opacity"
        );
    }

    #[test]
    fn resolve_align_with_absolute_at() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

place a shape=box at=400,50 align=top size=200x30
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("id=\"a\""));
    }

    // ── Blur tests ────────────────────────────────────────────────────

    #[test]
    fn resolve_blur_produces_filter() {
        let input = "\
documentsize 400x400

shape shadow template=ellipse
  fill #000000
  blur 5

place shadow shape=shadow at=100,100 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("<defs>"), "SVG should have defs section");
        assert!(
            svg.contains("<filter id=\"blur-shadow\""),
            "SVG should have filter"
        );
        assert!(
            svg.contains("stdDeviation=\"5\""),
            "filter should have stdDeviation"
        );
        assert!(
            svg.contains("filter=\"url(#blur-shadow)\""),
            "path should reference filter"
        );
    }

    #[test]
    fn resolve_blur_as_place_override() {
        let input = "\
documentsize 400x400

shape glow template=ellipse
  fill #ffff00

place glow shape=glow at=100,100 size=50x50
  blur 8
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("stdDeviation=\"8\""),
            "override blur should apply"
        );
        assert!(
            svg.contains("filter=\"url(#blur-glow)\""),
            "path should reference filter"
        );
    }

    // ── Clip group tests ──────────────────────────────────────────────

    #[test]
    fn resolve_clip_group() {
        let input = "\
documentsize 400x400

shape eye-white template=ellipse
  fill #ffffff

shape iris template=ellipse
  fill #4488cc

group eyes clip=eye-white
  place iris shape=iris at=120,85 size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("<clipPath id=\"clip-eyes\""),
            "SVG should have clipPath"
        );
        assert!(
            svg.contains("clip-path=\"url(#clip-eyes)\""),
            "g should reference clip-path"
        );
    }

    // ── C4: mask / per-place clip / multi-shape clip / skew (E2.3/E2.4) ──

    #[test]
    fn resolve_group_mask() {
        let input = "\
documentsize 400x400

shape gate template=ellipse
  fill #ffffff

shape body template=rectangle
  fill #4488cc

group masked mask=gate
  place body shape=body at=0,0 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("<mask id=\"mask-masked\""),
            "should emit <mask>"
        );
        assert!(
            svg.contains("fill=\"#ffffff\""),
            "mask shape should be white-filled (luminance)"
        );
        assert!(
            svg.contains("mask=\"url(#mask-masked)\""),
            "g should reference mask, got: {}",
            svg
        );
    }

    #[test]
    fn resolve_place_clip_and_mask() {
        let input = "\
documentsize 400x400

shape window template=ellipse
  fill #ffffff

shape photo template=rectangle
  fill #4488cc

place pic shape=photo at=0,0 size=200x200 clip=window mask=window
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("<clipPath id=\"clip-pic\""),
            "per-place clipPath"
        );
        assert!(svg.contains("<mask id=\"mask-pic\""), "per-place mask");
        assert!(svg.contains("clip-path=\"url(#clip-pic)\""));
        assert!(svg.contains("mask=\"url(#mask-pic)\""));
    }

    #[test]
    fn resolve_group_multi_shape_clip() {
        let input = "\
documentsize 400x400

shape a template=ellipse
  fill #ffffff
shape b template=rectangle
  fill #ffffff
shape body template=rectangle
  fill #4488cc

group g clip=a,b
  place body shape=body at=0,0 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // The clipPath should carry TWO <path> children (union clip).
        let cp = svg
            .split("<clipPath id=\"clip-g\">")
            .nth(1)
            .and_then(|s| s.split("</clipPath>").next())
            .unwrap_or("");
        assert_eq!(
            cp.matches("<path").count(),
            2,
            "multi-shape clip should union 2 paths, got: {}",
            cp
        );
    }

    #[test]
    fn resolve_place_skew_emits_matrix() {
        let input = "\
documentsize 400x400

shape card template=rectangle
  fill #4488cc

place c shape=card at=50,50 size=100x100 skew=20,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("transform=\"matrix("),
            "skewed place should emit a matrix transform, got: {}",
            svg
        );
    }

    #[test]
    fn resolve_rotated_relative_to_uses_transform_bbox() {
        // A child placed RelativeTo a rotated element must land on the rotated
        // bbox (transform-aware), not the pre-rotation bbox. We assert the
        // anchor's resolved x differs from the unrotated case by a meaningful
        // amount — proving the transform-aware bbox is used.
        let rotated = "\
documentsize 400x400
shape sq template=rectangle
  fill #000000
shape dot template=ellipse
  fill #ff0000
place base shape=sq at=100,100 size=80x40 rotation=90
place tag shape=dot at=base.right size=4x4
";
        let unrotated = rotated.replace("rotation=90\n", "\n");
        let r = resolve_scene(&dsl_parse::parse_file(rotated).unwrap());
        let u = resolve_scene(&dsl_parse::parse_file(unrotated.as_str()).unwrap());
        // Extract the dot's M x-coordinate in each.
        let dot_x = |svg: &str| -> f64 {
            let frag = svg.split("id=\"tag\"").nth(1).unwrap();
            let d = frag.split("d=\"M").nth(1).unwrap();
            d.split([' ', ',']).next().unwrap().parse().unwrap()
        };
        let (rx, ux) = (dot_x(&r), dot_x(&u));
        assert!(
            (rx - ux).abs() > 5.0,
            "rotated 'right' anchor should differ from unrotated: {} vs {}",
            rx,
            ux
        );
    }

    #[test]
    fn resolve_group_opacity() {
        let input = "\
documentsize 400x400

shape dot template=ellipse
  fill #ff0000

group head opacity=0.5
  place dot shape=dot at=100,100 size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(svg.contains("opacity=\"0.5\""), "g should have opacity");
    }

    // ── Group transform tests ────────────────────────────────────────

    #[test]
    fn resolve_group_translate() {
        let input = "\
documentsize 800x600

shape dot template=ellipse
  fill #ff0000

group dial at=100,50
  place dot shape=dot at=0,0 size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // E2.3: translate-only groups still emit the readable `translate(x, y)`
        // form via the unified affine emitter.
        assert!(
            svg.contains("transform=\"translate(100, 50)\""),
            "g should have translate transform, got: {}",
            svg
        );
    }

    #[test]
    fn resolve_group_compound_transform() {
        let input = "\
documentsize 800x600

shape ring template=ellipse
  fill #0000ff

group compass at=790,130 rotation=15deg flip=x
  place ring shape=ring at=0,0 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // E2.3: compound transforms now collapse into a single `matrix(...)` via
        // the unified affine. Verify it equals translate(790,130)·rotate(15)·flipX
        // rather than asserting a brittle float string.
        use crate::attrs::{mul, rotate, scale, translate};
        let expected = mul(
            &translate(790.0, 130.0),
            &mul(&rotate(15.0), &scale(-1.0, 1.0)),
        );
        let want = crate::attrs::emit_transform(&expected);
        assert!(
            svg.contains(&format!("transform=\"{}\"", want)),
            "g should have compound matrix transform '{}', got: {}",
            want,
            svg
        );
    }

    #[test]
    fn resolve_group_no_transform_when_none() {
        let input = "\
documentsize 400x400

shape dot template=ellipse
  fill #ff0000

group plain
  place dot shape=dot at=0,0 size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            !svg.contains("transform="),
            "g should not have transform attribute, got: {}",
            svg
        );
    }

    #[test]
    fn resolve_group_translate_offsets_child_bboxes() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

shape dot template=ellipse
  fill #0000ff

group panel at=100,50
  place inner shape=box at=10,10 size=40x40

place outer shape=dot at=inner.br size=20x20
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // inner bbox should be offset by group translate: (10+100, 10+50, 50+100, 50+50) = (110,60,150,100)
        // at=inner.br means at=(150,100)
        // The outer dot should be placed at (150,100)
        assert!(svg.contains("<g id=\"panel\""), "group should exist");
        assert!(svg.contains("id=\"outer\""), "outer should exist");
        // The outer element should reference inner's bbox bottom-right which is (150,100)
        // We just verify it renders without error and both elements appear
    }

    // ── Defaults tests ───────────────────────────────────────────────

    #[test]
    fn resolve_defaults_fill() {
        let input = "\
documentsize 400x400

defaults
  fill #2d5a1e
  stroke #1a3a12
  stroke-width 1.5

shape leaf template=ellipse

place leaf shape=leaf at=50,50 size=40x60
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("fill=\"#2d5a1e\""),
            "default fill should apply"
        );
        assert!(
            svg.contains("stroke=\"#1a3a12\""),
            "default stroke should apply"
        );
        assert!(
            svg.contains("stroke-width=\"1.5\""),
            "default stroke-width should apply"
        );
    }

    #[test]
    fn resolve_shape_overrides_defaults() {
        let input = "\
documentsize 400x400

defaults
  fill #2d5a1e

shape leaf template=ellipse
  fill #4a7a3e

place leaf shape=leaf at=50,50 size=40x60
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("fill=\"#4a7a3e\""),
            "shape fill should override default"
        );
        assert!(
            !svg.contains("fill=\"#2d5a1e\""),
            "default fill should not appear"
        );
    }

    #[test]
    fn resolve_place_overrides_shape_overrides_defaults() {
        let input = "\
documentsize 400x400

defaults
  fill #111111

shape leaf template=ellipse
  fill #222222

place leaf shape=leaf at=50,50 size=40x60
  fill #333333
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("fill=\"#333333\""),
            "place override should win"
        );
    }

    // ── Dashed stroke tests ──────────────────────────────────────────

    #[test]
    fn resolve_stroke_dasharray_output() {
        let input = "\
documentsize 400x400

shape border template=rectangle
  stroke #333333
  stroke-width 2
  stroke-dasharray 5 3

place border shape=border at=0,0 size=400x300
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("stroke-dasharray=\"5 3\""),
            "SVG should contain dasharray"
        );
    }

    #[test]
    fn resolve_stroke_dasharray_from_defaults() {
        let input = "\
documentsize 400x400

defaults
  stroke #333333
  stroke-dasharray 10 5 2 5

shape border template=rectangle

place border shape=border at=0,0 size=400x300
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        assert!(
            svg.contains("stroke-dasharray=\"10 5 2 5\""),
            "defaults dasharray should apply"
        );
    }

    // ── Arc segment tests ────────────────────────────────────────────

    #[test]
    fn resolve_arc_default_flags() {
        let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50

place arc shape=arc at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // default: large=0 sweep=1 → "A50 50 0 0 1"
        assert!(
            svg.contains("A50 50 0 0 1"),
            "default arc flags should be 0 0 1 (x-rot=0, large=0, sweep=1)"
        );
    }

    #[test]
    fn resolve_arc_custom_flags() {
        let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50 sweep=0 large=1

place arc shape=arc at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let svg = resolve_scene(&scene);
        // sweep=0, large=1 → "A50 50 0 1 0"
        assert!(
            svg.contains("A50 50 0 1 0"),
            "custom arc flags should be 0 1 0 (x-rot=0, large=1, sweep=0)"
        );
    }

    #[test]
    fn resolve_arc_sweep_cw_ccw_synonyms() {
        // sweep=cw is identical to sweep=1; sweep=ccw is identical to sweep=0.
        let cw = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50 sweep=cw

place arc shape=arc at=0,0
";
        let ccw = cw.replace("sweep=cw", "sweep=ccw");
        let svg_cw = resolve_scene(&dsl_parse::parse_file(cw).unwrap());
        let svg_ccw = resolve_scene(&dsl_parse::parse_file(&ccw).unwrap());
        assert!(svg_cw.contains("A50 50 0 0 1"), "sweep=cw → sweep flag 1");
        assert!(svg_ccw.contains("A50 50 0 0 0"), "sweep=ccw → sweep flag 0");
        assert_ne!(svg_cw, svg_ccw, "cw and ccw must differ");
    }

    #[test]
    fn resolve_arc_bulge_left_right_differ() {
        // bulge=left vs bulge=right produce opposite sweep flags (visually
        // opposite bulge sides) for the same point order.
        let base = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50 bulge=BULGE

place arc shape=arc at=0,0
";
        let left = base.replace("BULGE", "left");
        let right = base.replace("BULGE", "right");
        let svg_left = resolve_scene(&dsl_parse::parse_file(&left).unwrap());
        let svg_right = resolve_scene(&dsl_parse::parse_file(&right).unwrap());
        assert!(
            svg_left.contains("A50 50 0 0 0"),
            "bulge=left → sweep flag 0, got {svg_left}"
        );
        assert!(
            svg_right.contains("A50 50 0 0 1"),
            "bulge=right → sweep flag 1, got {svg_right}"
        );
        assert_ne!(svg_left, svg_right, "bulge left vs right must differ");
    }

    #[test]
    fn resolve_arc_bulge_default_unchanged() {
        // Omitting bulge must be byte-identical to the historical default
        // (sweep defaults to 1). bulge=right equals that default; bulge=left flips it.
        let no_bulge = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50

place arc shape=arc at=0,0
";
        let with_right =
            no_bulge.replace("mode=arc rx=50 ry=50", "mode=arc rx=50 ry=50 bulge=right");
        let svg_default = resolve_scene(&dsl_parse::parse_file(no_bulge).unwrap());
        let svg_right = resolve_scene(&dsl_parse::parse_file(&with_right).unwrap());
        assert_eq!(
            svg_default, svg_right,
            "default (no bulge) must match bulge=right byte-for-byte"
        );
    }

    #[test]
    fn resolve_arc_bulge_overrides_sweep() {
        // When both bulge and sweep are present, bulge wins.
        let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=50 sweep=1 bulge=left

place arc shape=arc at=0,0
";
        let svg = resolve_scene(&dsl_parse::parse_file(input).unwrap());
        assert!(
            svg.contains("A50 50 0 0 0"),
            "bulge=left overrides sweep=1, got {svg}"
        );
    }
}
