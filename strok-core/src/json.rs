//! A tiny, dependency-free JSON value builder (C6 / E3.2).
//!
//! This generalizes the `measure --json` seam left by C5: instead of every
//! command hand-formatting its own JSON `format!` string, they build a
//! [`Json`] value and call [`Json::to_string_pretty`]. One helper, one stable
//! formatting style (2-space indent, deterministic key order = insertion
//! order), so `--json` is byte-for-byte snapshot-stable across every
//! inspection / query / measure command.
//!
//! Numbers go through [`crate::types::fmt_num`] so integers stay integers and
//! floats trim to the same precision the `d`-string emitter uses — the property
//! the C5 seam relied on. There is no parser here: Strøk only *emits* JSON.

/// A JSON value. Object key order is preserved (insertion order) for stable
/// snapshots.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// A number, already formatted via `fmt_num` semantics at construction.
    Num(f64),
    Str(String),
    Array(Vec<Json>),
    /// Insertion-ordered object (not a map) so output order is deterministic.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn num(v: f64) -> Json {
        Json::Num(v)
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    /// Build an object from `(key, value)` pairs, preserving order.
    pub fn obj(pairs: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
        Json::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn array(items: impl IntoIterator<Item = Json>) -> Json {
        Json::Array(items.into_iter().collect())
    }

    /// Pretty-print with 2-space indentation and a trailing newline. This is the
    /// canonical, snapshot-stable `--json` rendering used by every command.
    pub fn to_string_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    /// Compact, single-line rendering (no indentation, no trailing newline).
    /// Used for line-delimited transports like the MCP stdio JSON-RPC server
    /// (E3.4), where one message must be exactly one line.
    pub fn to_string_compact(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    fn write_compact(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&crate::types::fmt_num(*n)),
            Json::Str(s) => {
                out.push('"');
                out.push_str(&escape(s));
                out.push('"');
            }
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_compact(out);
                }
                out.push(']');
            }
            Json::Object(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    out.push_str(&escape(k));
                    out.push_str("\":");
                    v.write_compact(out);
                }
                out.push('}');
            }
        }
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&crate::types::fmt_num(*n)),
            Json::Str(s) => {
                out.push('"');
                out.push_str(&escape(s));
                out.push('"');
            }
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                let pad = "  ".repeat(indent + 1);
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad);
                    item.write(out, indent + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(indent));
                out.push(']');
            }
            Json::Object(pairs) => {
                if pairs.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let pad = "  ".repeat(indent + 1);
                for (i, (k, v)) in pairs.iter().enumerate() {
                    out.push_str(&pad);
                    out.push('"');
                    out.push_str(&escape(k));
                    out.push_str("\": ");
                    v.write(out, indent + 1);
                    if i + 1 < pairs.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(indent));
                out.push('}');
            }
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_stay_integers() {
        assert_eq!(Json::num(20.0).to_string_pretty().trim(), "20");
        assert_eq!(Json::num(10.5).to_string_pretty().trim(), "10.5");
    }

    #[test]
    fn object_order_is_insertion_order() {
        let j = Json::obj([("b", Json::num(1.0)), ("a", Json::num(2.0))]);
        let s = j.to_string_pretty();
        let b_pos = s.find("\"b\"").unwrap();
        let a_pos = s.find("\"a\"").unwrap();
        assert!(b_pos < a_pos, "{s}");
    }

    #[test]
    fn nested_and_arrays() {
        let j = Json::obj([
            ("name", Json::str("hi")),
            ("ok", Json::Bool(true)),
            ("items", Json::array([Json::num(1.0), Json::num(2.0)])),
        ]);
        let s = j.to_string_pretty();
        assert_eq!(
            s,
            "{\n  \"name\": \"hi\",\n  \"ok\": true,\n  \"items\": [\n    1,\n    2\n  ]\n}\n"
        );
    }

    #[test]
    fn empty_collections() {
        assert_eq!(Json::array([]).to_string_pretty().trim(), "[]");
        assert_eq!(Json::obj([]).to_string_pretty().trim(), "{}");
    }

    #[test]
    fn escapes_strings() {
        let j = Json::str("a\"b\\c\n");
        assert_eq!(j.to_string_pretty().trim(), r#""a\"b\\c\n""#);
    }
}
