//! Vanilla backend — a dependency-free HTML-string emitter.
//!
//! This is the *third* neutrality pressure-test (design doc §4.1): a non-JSX
//! output driven by the same `UiDoc`. If the IR carried anything JSX-shaped,
//! this backend would be awkward to write. It isn't — which is the evidence
//! that the IR is genuinely framework-neutral. SVG is inlined verbatim (no
//! dialect quirk needed); `class` is a plain HTML attribute.

use crate::backends::jsx::pascal_case;
use crate::ir::{UiDoc, UiNode};
use crate::target::{Capabilities, EmitArtifact, EmitFile, EmitOptions, FrameworkBackend};

pub struct VanillaBackend;

impl FrameworkBackend for VanillaBackend {
    fn id(&self) -> &'static str {
        "vanilla"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            vector: true,
            raster: true,
            components: false, // no component instancing in the vanilla string form yet
            auto_layout: true,
            interactivity: false,
        }
    }

    fn render(&self, doc: &UiDoc, _opts: &EmitOptions) -> EmitArtifact {
        let ident = pascal_case(&doc.name);
        let body = render_html(&doc.root, 4);
        let source =
            format!("export function {ident}(): string {{\n  return `\n{body}\n  `;\n}}\n");
        EmitArtifact {
            files: vec![EmitFile {
                path: format!("{ident}.ts"),
                contents: source,
            }],
            assets: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

fn render_html(node: &UiNode, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match node {
        UiNode::Element { tag, children, .. } => {
            let class = node.class_list().unwrap_or_default();
            let class_attr = if class.is_empty() {
                String::new()
            } else {
                format!(" class=\"{}\"", class)
            };
            if children.is_empty() {
                return format!("{pad}<{0}{1}></{0}>", tag.html_name(), class_attr);
            }
            let inner: Vec<String> = children
                .iter()
                .map(|c| render_html(c, indent + 2))
                .collect();
            format!(
                "{pad}<{tag}{class_attr}>\n{inner}\n{pad}</{tag}>",
                tag = tag.html_name(),
                inner = inner.join("\n"),
            )
        }
        UiNode::Text(t) => format!("{pad}{}", template_escape(t)),
        UiNode::Svg(svg) => format!("{pad}{}", svg.trim()),
        UiNode::Image { src, alt } => format!("{pad}<img src=\"{}\" alt=\"{}\" />", src, alt),
        UiNode::Instance { component, .. } => {
            // Vanilla has no instancing; emit a call placeholder and rely on the
            // backend's `components: false` capability to advertise the gap.
            format!("{pad}${{{}()}}", pascal_case(component))
        }
    }
}

/// Escape characters that would break a JS template literal.
fn template_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${")
}
