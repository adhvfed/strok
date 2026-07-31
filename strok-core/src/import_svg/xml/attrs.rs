//! Attribute and entity parsing.

pub(super) struct AttrParser<'a> {
    rest: &'a str,
}

impl<'a> AttrParser<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self { rest: source }
    }

    fn parse_value(&mut self) -> String {
        self.rest = self.rest.trim_start();
        let Some(first) = self.rest.chars().next() else {
            return String::new();
        };

        if first == '"' || first == '\'' {
            let after_quote = &self.rest[first.len_utf8()..];
            if let Some(end) = after_quote.find(first) {
                let value = decode_entities(&after_quote[..end]);
                self.rest = &after_quote[end + first.len_utf8()..];
                return value;
            }
            self.rest = "";
            return decode_entities(after_quote);
        }

        let end = self
            .rest
            .find(char::is_whitespace)
            .unwrap_or(self.rest.len());
        let value = decode_entities(&self.rest[..end]);
        self.rest = &self.rest[end..];
        value
    }
}

impl Iterator for AttrParser<'_> {
    type Item = (String, String);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.rest = self.rest.trim_start();
            if self.rest.is_empty() {
                return None;
            }

            let name_end = self
                .rest
                .find(|character: char| character == '=' || character.is_whitespace())
                .unwrap_or(self.rest.len());
            let name = self.rest[..name_end].to_string();
            self.rest = self.rest[name_end..].trim_start();

            let value = if let Some(rest) = self.rest.strip_prefix('=') {
                self.rest = rest;
                self.parse_value()
            } else {
                String::new()
            };

            if !name.is_empty() {
                return Some((name, value));
            }
        }
    }
}

/// Decode the standard XML entities plus numeric character references.
pub(super) fn decode_entities(s: &str) -> String {
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
#[path = "attrs/tests.rs"]
mod tests;
