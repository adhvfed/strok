use crate::attrs::{emit_transform, Paint};
use crate::document::Document;
use crate::node::{NodeId, NodeKind};
use crate::path_point::path_data_to_svg_d;

/// Emit an entire document as an SVG string.
///
/// If the document has a v3 Scene, delegates to resolve_scene.
/// Otherwise falls back to the arena-based emit.
pub fn emit_document(doc: &Document) -> String {
    // v3 path: use resolve_scene
    if let Some(ref scene) = doc.scene {
        return crate::resolve::resolve_scene(scene);
    }

    // v2 fallback: arena-based
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        doc.width, doc.height, doc.width, doc.height
    ));
    out.push('\n');

    // The root is expected to exist; if the arena is somehow missing it, emit a
    // valid-but-empty document rather than panicking (no-panic policy, E1.4).
    if let Ok(root) = doc.arena.get(doc.root_id) {
        let children = root.children.clone();
        for child_id in children {
            emit_node(doc, child_id, 1, None, &mut out);
        }
    }

    out.push_str("</svg>\n");
    out
}

/// Emit a subtree starting at the given node.
pub fn emit_subtree(doc: &Document, node_id: NodeId, depth_limit: Option<usize>) -> String {
    let mut out = String::new();
    emit_node(doc, node_id, 0, depth_limit, &mut out);
    out
}

fn emit_node(
    doc: &Document,
    node_id: NodeId,
    indent: usize,
    depth_limit: Option<usize>,
    out: &mut String,
) {
    let node = match doc.arena.get(node_id) {
        Ok(n) => n,
        Err(_) => return,
    };

    let prefix = "  ".repeat(indent);
    let tag = node.kind.tag_name();

    // Root nodes are emitted by emit_document, skip here
    if node.kind == NodeKind::Root {
        for &child_id in &node.children {
            emit_node(doc, child_id, indent, depth_limit, out);
        }
        return;
    }

    out.push_str(&prefix);
    out.push('<');
    out.push_str(tag);

    // ID
    out.push_str(&format!(" id=\"{}\"", node.id));

    // Attributes
    let a = &node.attrs;

    if let Some(ref fill) = a.fill {
        match fill {
            Paint::Color(c) => out.push_str(&format!(" fill=\"{}\"", c)),
            Paint::None => out.push_str(" fill=\"none\""),
        }
    }
    if let Some(ref stroke) = a.stroke {
        match stroke {
            Paint::Color(c) => out.push_str(&format!(" stroke=\"{}\"", c)),
            Paint::None => out.push_str(" stroke=\"none\""),
        }
    }
    if let Some(sw) = a.stroke_width {
        out.push_str(&format!(" stroke-width=\"{}\"", sw));
    }
    if let Some(op) = a.opacity {
        out.push_str(&format!(" opacity=\"{}\"", op));
    }
    if let Some(ref t) = a.transform {
        let ts = emit_transform(t);
        if !ts.is_empty() {
            out.push_str(&format!(" transform=\"{}\"", ts));
        }
    }

    // Geometry
    if let Some(v) = a.x {
        out.push_str(&format!(" x=\"{}\"", v));
    }
    if let Some(v) = a.y {
        out.push_str(&format!(" y=\"{}\"", v));
    }
    if let Some(v) = a.width {
        out.push_str(&format!(" width=\"{}\"", v));
    }
    if let Some(v) = a.height {
        out.push_str(&format!(" height=\"{}\"", v));
    }
    if let Some(v) = a.cx {
        out.push_str(&format!(" cx=\"{}\"", v));
    }
    if let Some(v) = a.cy {
        out.push_str(&format!(" cy=\"{}\"", v));
    }
    if let Some(v) = a.r {
        out.push_str(&format!(" r=\"{}\"", v));
    }
    if let Some(v) = a.rx {
        out.push_str(&format!(" rx=\"{}\"", v));
    }
    if let Some(v) = a.ry {
        out.push_str(&format!(" ry=\"{}\"", v));
    }
    if let Some(v) = a.x1 {
        out.push_str(&format!(" x1=\"{}\"", v));
    }
    if let Some(v) = a.y1 {
        out.push_str(&format!(" y1=\"{}\"", v));
    }
    if let Some(v) = a.x2 {
        out.push_str(&format!(" x2=\"{}\"", v));
    }
    if let Some(v) = a.y2 {
        out.push_str(&format!(" y2=\"{}\"", v));
    }
    if let Some(ref pd) = a.path_data {
        let d_str = path_data_to_svg_d(pd, None);
        out.push_str(&format!(" d=\"{}\"", d_str));
    } else if let Some(ref v) = a.d {
        out.push_str(&format!(" d=\"{}\"", v));
    }
    if let Some(ref v) = a.points {
        out.push_str(&format!(" points=\"{}\"", v));
    }

    // Text attrs
    if let Some(v) = a.font_size {
        out.push_str(&format!(" font-size=\"{}\"", v));
    }
    if let Some(ref v) = a.font_family {
        out.push_str(&format!(" font-family=\"{}\"", v));
    }
    if let Some(ref v) = a.font_weight {
        out.push_str(&format!(" font-weight=\"{}\"", v));
    }
    if let Some(ref v) = a.text_anchor {
        out.push_str(&format!(" text-anchor=\"{}\"", v));
    }

    // Image
    if let Some(ref v) = a.href {
        out.push_str(&format!(" href=\"{}\"", v));
    }

    // Extra pass-through attributes
    for (k, v) in &a.extra {
        out.push_str(&format!(" {}=\"{}\"", k, v));
    }

    let has_children = !node.children.is_empty();
    let has_text = node.attrs.text_content.is_some();
    let at_depth_limit = depth_limit.is_some_and(|l| l == 0);

    if at_depth_limit && has_children {
        out.push_str(" />\n");
        return;
    }

    if !has_children && !has_text {
        out.push_str(" />\n");
    } else if has_text && !has_children {
        out.push('>');
        if let Some(ref text) = node.attrs.text_content {
            out.push_str(text);
        }
        out.push_str(&format!("</{}>\n", tag));
    } else {
        out.push_str(">\n");
        if let Some(ref text) = node.attrs.text_content {
            out.push_str(&format!("{}  {}\n", prefix, text));
        }
        let child_limit = depth_limit.map(|l| l.saturating_sub(1));
        for &child_id in &node.children {
            emit_node(doc, child_id, indent + 1, child_limit, out);
        }
        out.push_str(&format!("{}</{}>\n", prefix, tag));
    }
}
