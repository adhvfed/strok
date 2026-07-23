//! Shared JSX tree-walker for the React and Solid backends.
//!
//! React and Solid differ in exactly two syntactic knobs — the class attribute
//! name (`className` vs `class`) and how raw SVG is inlined
//! (`dangerouslySetInnerHTML` vs `innerHTML`). Everything else — the tree walk,
//! indentation, element/instance/text emission — is *one* function here.
//!
//! That is deliberate: it makes "React and Solid produce the same structure"
//! true by construction, not by discipline. The parity test
//! (`tests/parity.rs`) then exists to catch any future change that tries to
//! special-case one dialect and reintroduce drift.

use crate::ir::{Tag, UiNode};

/// The two knobs that distinguish JSX dialects. Nothing structural lives here.
pub struct JsxDialect {
    /// `"className"` (React) or `"class"` (Solid).
    pub class_attr: &'static str,
    /// Builds the raw-SVG inlining attribute from raw SVG markup.
    pub svg_attr: fn(&str) -> String,
    /// Module header (imports), or `""`.
    pub header: &'static str,
}

impl JsxDialect {
    /// Render a complete component module for `doc`.
    pub fn render_component(&self, name: &str, root: &UiNode) -> String {
        let ident = pascal_case(name);
        let body = self.render_node(root, 2);
        let mut out = String::new();
        if !self.header.is_empty() {
            out.push_str(self.header);
            out.push('\n');
        }
        out.push_str(&format!("export function {ident}() {{\n"));
        out.push_str("  return (\n");
        out.push_str(&body);
        out.push('\n');
        out.push_str("  );\n");
        out.push_str("}\n");
        out
    }

    fn render_node(&self, node: &UiNode, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match node {
            UiNode::Element { tag, children, .. } => {
                let class = node.class_list().unwrap_or_default();
                let class_attr = if class.is_empty() {
                    String::new()
                } else {
                    format!(" {}=\"{}\"", self.class_attr, class)
                };

                // Single SVG child → inline via the dialect's inner-HTML attr.
                if let [UiNode::Svg(svg)] = children.as_slice() {
                    return format!(
                        "{pad}<{tag}{class_attr} {svg} />",
                        tag = tag.html_name(),
                        svg = (self.svg_attr)(svg),
                    );
                }

                if children.is_empty() {
                    return format!("{pad}<{}{} />", tag.html_name(), class_attr);
                }

                let inner: Vec<String> = children
                    .iter()
                    .map(|c| self.render_node(c, indent + 2))
                    .collect();
                format!(
                    "{pad}<{tag}{class_attr}>\n{inner}\n{pad}</{tag}>",
                    tag = tag.html_name(),
                    inner = inner.join("\n"),
                )
            }
            UiNode::Text(t) => format!("{pad}{}", jsx_text(t)),
            UiNode::Svg(svg) => {
                // A standalone SVG leaf gets its own wrapper div.
                format!("{pad}<div {} />", (self.svg_attr)(svg))
            }
            UiNode::Image { src, alt } => {
                format!(
                    "{pad}<img src=\"{}\" alt=\"{}\" />",
                    attr_escape(src),
                    attr_escape(alt)
                )
            }
            UiNode::Instance {
                component,
                props,
                children,
            } => {
                let ident = pascal_case(component);
                let mut prop_str = String::new();
                for (k, v) in props {
                    prop_str.push_str(&format!(" {}=\"{}\"", k, attr_escape(v)));
                }
                if children.is_empty() {
                    return format!("{pad}<{ident}{prop_str} />");
                }
                let inner: Vec<String> = children
                    .iter()
                    .map(|c| self.render_node(c, indent + 2))
                    .collect();
                format!(
                    "{pad}<{ident}{prop_str}>\n{}\n{pad}</{ident}>",
                    inner.join("\n")
                )
            }
        }
    }
}

/// Escape a string for embedding inside a JS double-quoted string literal.
pub fn escape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

fn attr_escape(s: &str) -> String {
    s.replace('"', "&quot;")
}

/// Minimal JSX text escaping for the characters that would break a text node.
fn jsx_text(s: &str) -> String {
    s.replace('{', "&#123;")
        .replace('}', "&#125;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Turn an arbitrary name into a PascalCase identifier (`"my button"` → `"MyButton"`).
pub fn pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty()
        || out
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    {
        out.insert(0, 'C');
    }
    out
}

// Keep `Tag` import meaningful even if unused paths change.
#[allow(dead_code)]
fn _tag_names_are_stable(t: Tag) -> &'static str {
    t.html_name()
}
