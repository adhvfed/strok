//! XML event tokenization and cursor management.

use super::attrs::{decode_entities, AttrParser};
use super::{ImportWarning, XmlNode};

pub(super) enum XmlEvent {
    Open(XmlNode),
    Empty(XmlNode),
    Close,
    Text(String),
}

pub(super) struct XmlParser<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
}

impl<'a> XmlParser<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
        }
    }

    fn rest(&self) -> &'a str {
        &self.source[self.offset..]
    }

    fn advance(&mut self, bytes: usize) {
        let end = (self.offset + bytes).min(self.source.len());
        self.line += self.source[self.offset..end]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        self.offset = end;
    }

    fn skip_through(&mut self, terminator: &str) -> bool {
        let Some(end) = self.rest().find(terminator) else {
            self.offset = self.source.len();
            return false;
        };
        self.advance(end + terminator.len());
        true
    }

    pub(super) fn next_event(&mut self, warnings: &mut Vec<ImportWarning>) -> Option<XmlEvent> {
        while !self.rest().is_empty() {
            if !self.rest().starts_with('<') {
                let length = self.rest().find('<').unwrap_or(self.rest().len());
                let text = decode_entities(&self.rest()[..length]);
                self.advance(length);
                return Some(XmlEvent::Text(text));
            }

            if self.rest().starts_with("<!--") {
                if !self.skip_through("-->") {
                    return None;
                }
                continue;
            }
            if self.rest().starts_with("<![CDATA[") {
                warnings.push(ImportWarning::new("CDATA section ignored", Some(self.line)));
                if !self.skip_through("]]>") {
                    return None;
                }
                continue;
            }
            if self.rest().starts_with("<!") {
                if self.rest().starts_with("<!DOCTYPE") {
                    warnings.push(ImportWarning::new("DOCTYPE ignored", Some(self.line)));
                }
                if !self.skip_through(">") {
                    return None;
                }
                continue;
            }
            if self.rest().starts_with("<?") {
                if !self.skip_through("?>") {
                    return None;
                }
                continue;
            }

            let tag_line = self.line;
            let end = find_tag_end(self.rest())?;
            let tag = &self.rest()[1..end];
            let event = if tag.strip_prefix('/').is_some() {
                XmlEvent::Close
            } else if let Some(open) = tag.strip_suffix('/') {
                XmlEvent::Empty(parse_tag(open, tag_line))
            } else {
                XmlEvent::Open(parse_tag(tag, tag_line))
            };
            self.advance(end + 1);
            return Some(event);
        }
        None
    }
}

/// Find the index of the tag-closing `>` while respecting quoted attributes.
fn find_tag_end(source: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in source.char_indices().skip(1) {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '>' => return Some(index),
            None => {}
        }
    }
    None
}

/// Parse a start-tag body (`name attr="v" …`) into an [`XmlNode`].
fn parse_tag(body: &str, line: usize) -> XmlNode {
    let body = body.trim();
    let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
    XmlNode {
        name: body[..name_end].to_string(),
        attrs: AttrParser::new(&body[name_end..]).collect(),
        children: Vec::new(),
        text: String::new(),
        line: Some(line),
    }
}
