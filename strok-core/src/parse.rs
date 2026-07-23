use crate::error::{Result, StrokError};
use crate::node::{NodeId, NodeKind, SceneNode};

/// Parse an SVG snippet into a flat list of SceneNodes.
///
/// Returns nodes in depth-first order. Index 0 is the root element.
/// Each node's `children` field contains indices (as NodeId) into this vec,
/// NOT real arena NodeIds. The Document layer resolves these during allocation.
pub fn parse_snippet(svg: &str) -> Result<Vec<SceneNode>> {
    let wrapped = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">{}</svg>",
        svg
    );
    let doc =
        roxmltree::Document::parse(&wrapped).map_err(|e| StrokError::ParseError(e.to_string()))?;

    let svg_elem = doc.root_element();
    let children: Vec<_> = svg_elem.children().filter(|n| n.is_element()).collect();

    if children.is_empty() {
        return Err(StrokError::ParseError(
            "no elements found in SVG snippet".to_string(),
        ));
    }
    if children.len() > 1 {
        return Err(StrokError::ParseError(
            "SVG snippet must contain exactly one root element (use <g> to wrap multiple)"
                .to_string(),
        ));
    }

    let mut nodes = Vec::new();
    collect_nodes(&children[0], &mut nodes)?;
    Ok(nodes)
}

fn collect_nodes(elem: &roxmltree::Node, out: &mut Vec<SceneNode>) -> Result<usize> {
    let tag = elem.tag_name().name();
    let kind = NodeKind::from_tag(tag)
        .ok_or_else(|| StrokError::ParseError(format!("unsupported SVG element: <{}>", tag)))?;

    let id = elem.attribute("id").unwrap_or("").to_string();
    let mut node = SceneNode::new(id, kind);

    for attr in elem.attributes() {
        node.attrs.set_from_svg(attr.name(), attr.value());
    }

    if tag == "text" {
        let text: String = elem
            .descendants()
            .filter(|n| n.is_text())
            .map(|n| n.text().unwrap_or(""))
            .collect();
        if !text.is_empty() {
            node.attrs.text_content = Some(text);
        }
    }

    let my_index = out.len();
    out.push(node); // placeholder — children filled below

    let child_elems: Vec<_> = elem.children().filter(|n| n.is_element()).collect();
    let mut child_indices = Vec::new();
    for child_elem in child_elems {
        let child_idx = collect_nodes(&child_elem, out)?;
        child_indices.push(child_idx);
    }

    out[my_index].children = child_indices
        .into_iter()
        .map(|i| NodeId(i as u32))
        .collect();

    Ok(my_index)
}
