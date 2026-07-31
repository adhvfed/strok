//! Minimal, dependency-free XML parsing for the SVG importer.

mod attrs;
mod parser;

use super::ImportWarning;
use parser::{XmlEvent, XmlParser};

/// A parsed XML element node.
#[derive(Debug, Clone)]
pub(super) struct XmlNode {
    pub(super) name: String,
    pub(super) attrs: Vec<(String, String)>,
    pub(super) children: Vec<XmlNode>,
    pub(super) text: String,
    pub(super) line: Option<usize>,
}

impl XmlNode {
    pub(super) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name || local_name(k) == name)
            .map(|(_, v)| v.as_str())
    }
}

pub(super) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Parse an XML document, returning the root `<svg>` element. Best-effort:
/// malformed markup yields whatever tree was recovered.
pub(super) fn parse_xml(src: &str, warnings: &mut Vec<ImportWarning>) -> Option<XmlNode> {
    let mut parser = XmlParser::new(src);
    let mut stack = Vec::new();
    let mut root = None;

    while let Some(event) = parser.next_event(warnings) {
        match event {
            XmlEvent::Open(node) => stack.push(node),
            XmlEvent::Empty(node) => attach(&mut stack, &mut root, node),
            XmlEvent::Close => {
                if let Some(node) = stack.pop() {
                    attach(&mut stack, &mut root, node);
                }
            }
            XmlEvent::Text(text) if !text.trim().is_empty() => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&text);
                }
            }
            XmlEvent::Text(_) => {}
        }
    }

    while let Some(node) = stack.pop() {
        attach(&mut stack, &mut root, node);
    }
    root
}

fn attach(stack: &mut [XmlNode], root: &mut Option<XmlNode>, node: XmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.is_none() {
        *root = Some(node);
    } else if let Some(root) = root.as_mut() {
        root.children.push(node);
    }
}

#[cfg(test)]
#[path = "xml/tests.rs"]
mod tests;
