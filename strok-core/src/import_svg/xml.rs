//! Minimal, dependency-free XML parsing for the SVG importer.

use super::ImportWarning;

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
/// malformed markup yields a warning and whatever tree was recovered.
pub(super) fn parse_xml(src: &str, warnings: &mut Vec<ImportWarning>) -> Option<XmlNode> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut line = 1usize;
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;

    let count_lines = |from: usize, to: usize, b: &[u8]| -> usize {
        let lo = from.min(b.len());
        let hi = to.min(b.len()).max(lo);
        b[lo..hi].iter().filter(|&&c| c == b'\n').count()
    };

    while i < b.len() {
        if b[i] == b'<' {
            if src[i..].starts_with("<!--") {
                if let Some(end) = src[i..].find("-->") {
                    line += count_lines(i, i + end, b);
                    i += end + 3;
                    continue;
                } else {
                    break;
                }
            }
            if src[i..].starts_with("<![CDATA[") {
                warnings.push(ImportWarning::new("CDATA section ignored", Some(line)));
                if let Some(end) = src[i..].find("]]>") {
                    line += count_lines(i, i + end, b);
                    i += end + 3;
                    continue;
                } else {
                    break;
                }
            }
            if src[i..].starts_with("<!") {
                if src[i..].starts_with("<!DOCTYPE") {
                    warnings.push(ImportWarning::new("DOCTYPE ignored", Some(line)));
                }
                if let Some(end) = src[i..].find('>') {
                    line += count_lines(i, i + end, b);
                    i += end + 1;
                    continue;
                } else {
                    break;
                }
            }
            if src[i..].starts_with("<?") {
                if let Some(end) = src[i..].find("?>") {
                    line += count_lines(i, i + end, b);
                    i += end + 2;
                    continue;
                } else {
                    break;
                }
            }
            let tag_line = line;
            let Some(rel_end) = find_tag_end(&src[i..]) else {
                break;
            };
            let tag = &src[i + 1..i + rel_end];
            line += count_lines(i, i + rel_end, b);
            i += rel_end + 1;

            if let Some(close) = tag.strip_prefix('/') {
                let _name = close.trim();
                if let Some(node) = stack.pop() {
                    attach(&mut stack, &mut root, node);
                }
            } else if let Some(open) = tag.strip_suffix('/') {
                let node = parse_tag(open, tag_line);
                attach(&mut stack, &mut root, node);
            } else {
                let node = parse_tag(tag, tag_line);
                stack.push(node);
            }
        } else {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            let text = decode_entities(&src[start..i]);
            line += count_lines(start, i, b);
            if !text.trim().is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text);
                }
            }
        }
    }
    while let Some(node) = stack.pop() {
        attach(&mut stack, &mut root, node);
    }
    root
}

/// Find the index of the tag-closing `>` respecting quoted attribute values.
fn find_tag_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 1; // skip '<'
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn attach(stack: &mut [XmlNode], root: &mut Option<XmlNode>, node: XmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.is_none() {
        *root = Some(node);
    } else if let Some(r) = root.as_mut() {
        r.children.push(node);
    }
}

/// Parse a start-tag body (`name attr="v" …`) into an [`XmlNode`].
fn parse_tag(body: &str, line: usize) -> XmlNode {
    let body = body.trim();
    let mut name_end = body.len();
    for (idx, c) in body.char_indices() {
        if c.is_whitespace() {
            name_end = idx;
            break;
        }
    }
    let name = body[..name_end].to_string();
    let rest = &body[name_end..];
    let attrs = parse_attrs(rest);
    XmlNode {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
        line: Some(line),
    }
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let ns = i;
        while i < b.len() && b[i] != b'=' && !(b[i] as char).is_whitespace() {
            i += 1;
        }
        let name = s[ns..i].trim().to_string();
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && (b[i] as char).is_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                let val = decode_entities(&s[vs..i.min(s.len())]);
                if !name.is_empty() {
                    attrs.push((name, val));
                }
                i += 1; // past quote
            } else {
                let vs = i;
                while i < b.len() && !(b[i] as char).is_whitespace() {
                    i += 1;
                }
                let val = decode_entities(&s[vs..i]);
                if !name.is_empty() {
                    attrs.push((name, val));
                }
            }
        } else if !name.is_empty() {
            attrs.push((name, String::new()));
        }
    }
    attrs
}

/// Decode the standard XML entities plus numeric character references.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }

    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after_amp = &rest[amp + 1..];
        let Some(semi) = after_amp.find(';') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let entity = &after_amp[..semi];
        if let Some(decoded) = decode_entity(entity) {
            out.push(decoded);
            rest = &after_amp[semi + 1..];
        } else {
            // Preserve unknown entities verbatim and continue looking for a
            // later entity that may still be valid.
            out.push('&');
            rest = after_amp;
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        hex if hex.starts_with("#x") || hex.starts_with("#X") => u32::from_str_radix(&hex[2..], 16)
            .ok()
            .and_then(char::from_u32),
        decimal if decimal.starts_with('#') => {
            decimal[1..].parse::<u32>().ok().and_then(char::from_u32)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "xml/tests.rs"]
mod tests;
