//! Icon-set manifest + `<symbol>` sprite sheet (C10 / E5.1, E5.2).
//!
//! A design-system icon pipeline needs more than rendered PNG/SVG files: it
//! needs a **registry** mapping each icon's stable *name* to its *meaning*, its
//! *tags* (for search), and the *sizes* it ships at — the contract the consuming
//! app imports against. This module builds that registry
//! (`manifest.json`) and the companion `<symbol>` sprite sheet, both from the
//! same parsed icon set so they can never drift.
//!
//! ## Metadata convention (additive, backwards-compatible)
//!
//! Meaning and tags are authored as **leading comment annotations** at the top of
//! an icon `.strok` file — comments the parser already ignores, so no DSL change
//! and every existing icon stays valid:
//!
//! ```text
//! # @meaning Close or dismiss the current view
//! # @tags close, dismiss, x, cancel
//! documentsize 24x24
//! …
//! ```
//!
//! A file with no annotations still gets a manifest entry (meaning empty, tags
//! empty) — the registry is complete by construction; annotations only enrich it.

use crate::json::Json;

/// One icon's manifest entry: its name, authored meaning/tags, the document size
/// it was authored at, and the export sizes it ships at.
#[derive(Debug, Clone, PartialEq)]
pub struct IconEntry {
    /// Stable name = the `.strok` file stem (e.g. `arrow-left`).
    pub name: String,
    /// Human meaning (`# @meaning …`), empty if unannotated.
    pub meaning: String,
    /// Search tags (`# @tags a, b, c`), order-preserving, deduped.
    pub tags: Vec<String>,
    /// Authored canvas size `(w, h)` from `documentsize`.
    pub canvas: (f64, f64),
    /// Pixel sizes this icon is exported at (from `batch --sizes`).
    pub sizes: Vec<u32>,
}

/// A whole icon set's manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Format version, so consumers can detect schema changes.
    pub version: u32,
    /// Icons, in stable (sorted-by-name) order.
    pub icons: Vec<IconEntry>,
}

/// Current manifest schema version. Bump on any breaking shape change.
pub const MANIFEST_VERSION: u32 = 1;

/// Extracted leading metadata from an icon file's comment header.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct IconMeta {
    pub meaning: String,
    pub tags: Vec<String>,
}

/// Parse the `# @meaning` / `# @tags` annotations from the top of a `.strok`
/// source. Scanning stops at the first non-comment, non-blank line (annotations
/// are a *header* — they describe the file, not arbitrary mid-file comments).
pub fn parse_meta(source: &str) -> IconMeta {
    let mut meta = IconMeta::default();
    for line in source.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim();
            if let Some(m) = rest.strip_prefix("@meaning") {
                meta.meaning = m.trim().to_string();
            } else if let Some(tg) = rest.strip_prefix("@tags") {
                for tag in tg.split(',') {
                    let tag = tag.trim();
                    if !tag.is_empty() && !meta.tags.iter().any(|e| e == tag) {
                        meta.tags.push(tag.to_string());
                    }
                }
            }
            // Other comments in the header are ignored, scanning continues.
            continue;
        }
        // First real content line — header is over.
        break;
    }
    meta
}

impl Manifest {
    /// Render the manifest to its canonical JSON (the snapshot-stable schema):
    ///
    /// ```json
    /// {
    ///   "version": 1,
    ///   "count": 2,
    ///   "icons": [
    ///     { "name": "...", "meaning": "...", "tags": [...],
    ///       "canvas": { "w": 24, "h": 24 }, "sizes": [16, 24] }
    ///   ]
    /// }
    /// ```
    pub fn to_json(&self) -> Json {
        let icons = self.icons.iter().map(|e| {
            Json::obj([
                ("name", Json::str(&e.name)),
                ("meaning", Json::str(&e.meaning)),
                ("tags", Json::array(e.tags.iter().map(Json::str))),
                (
                    "canvas",
                    Json::obj([("w", Json::num(e.canvas.0)), ("h", Json::num(e.canvas.1))]),
                ),
                (
                    "sizes",
                    Json::array(e.sizes.iter().map(|&s| Json::num(s as f64))),
                ),
            ])
        });
        Json::obj([
            ("version", Json::num(self.version as f64)),
            ("count", Json::num(self.icons.len() as f64)),
            ("icons", Json::array(icons)),
        ])
    }

    pub fn to_json_string(&self) -> String {
        self.to_json().to_string_pretty()
    }
}

/// One entry for the sprite sheet: a symbol id (= icon name), its viewBox, and
/// the *inner* SVG markup (everything between the icon's `<svg>` and `</svg>`).
pub struct SpriteSymbol {
    pub id: String,
    pub viewbox: String,
    pub inner: String,
}

/// Build a `<symbol>` sprite sheet from per-icon SVG strings.
///
/// Each icon's full `<svg …>…</svg>` is reduced to its `viewBox` + inner markup
/// and wrapped in a `<symbol id="name" viewBox="…">`. The result is one SVG file
/// the host references with `<use href="sprite.svg#name"/>`. `currentColor` is
/// preserved (icons stay themeable). The sheet is deterministic: symbols appear
/// in the order given (the `batch` path sorts by name).
pub fn build_sprite(symbols: &[SpriteSymbol]) -> String {
    let mut out = String::new();
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\">\n");
    for s in symbols {
        out.push_str(&format!(
            "  <symbol id=\"{}\" viewBox=\"{}\">\n",
            xml_escape(&s.id),
            xml_escape(&s.viewbox)
        ));
        for line in s.inner.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("  </symbol>\n");
    }
    out.push_str("</svg>\n");
    out
}

/// Extract `(viewBox, inner_markup)` from a full SVG string. Returns `None` if
/// the string isn't a single rooted `<svg>…</svg>`. Used to turn a rendered icon
/// SVG into a sprite `<symbol>`.
pub fn split_svg(svg: &str) -> Option<(String, String)> {
    let open_start = svg.find("<svg")?;
    let open_end = svg[open_start..].find('>')? + open_start;
    let close = svg.rfind("</svg>")?;
    if close <= open_end {
        return None;
    }
    let open_tag = &svg[open_start..=open_end];
    let viewbox = extract_attr(open_tag, "viewBox").or_else(|| {
        // Synthesize a viewBox from width/height if the SVG only carries those.
        let w = extract_attr(open_tag, "width")?;
        let h = extract_attr(open_tag, "height")?;
        Some(format!("0 0 {w} {h}"))
    })?;
    let inner = svg[open_end + 1..close].trim().to_string();
    Some((viewbox, inner))
}

/// Extract the value of `attr="…"` from an opening tag, if present.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Minimal XML attribute escaping for ids/viewBox (icon names are slugs, but be
/// safe).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_svg_into_viewbox_and_inner() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\">\n<path d=\"M0 0\"/>\n</svg>\n";
        let (vb, inner) = split_svg(svg).unwrap();
        assert_eq!(vb, "0 0 24 24");
        assert_eq!(inner, "<path d=\"M0 0\"/>");
    }

    #[test]
    fn synthesizes_viewbox_from_width_height() {
        let svg = "<svg width=\"24\" height=\"24\"><path/></svg>";
        let (vb, _) = split_svg(svg).unwrap();
        assert_eq!(vb, "0 0 24 24");
    }

    #[test]
    fn builds_sprite_sheet() {
        let syms = vec![
            SpriteSymbol {
                id: "a".into(),
                viewbox: "0 0 24 24".into(),
                inner: "<path d=\"M0 0\" stroke=\"currentColor\"/>".into(),
            },
            SpriteSymbol {
                id: "b".into(),
                viewbox: "0 0 24 24".into(),
                inner: "<circle r=\"5\"/>".into(),
            },
        ];
        let sheet = build_sprite(&syms);
        assert!(sheet.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\">"));
        assert!(sheet.contains("<symbol id=\"a\" viewBox=\"0 0 24 24\">"));
        assert!(sheet.contains("currentColor"));
        assert!(sheet.contains("<symbol id=\"b\""));
        assert!(sheet.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn parses_meaning_and_tags_header() {
        let src = "\
# @meaning Close the current view
# @tags close, dismiss, x
documentsize 24x24
# this mid-file comment is not metadata
# @tags ignored, here
shape s template=rectangle
";
        let meta = parse_meta(src);
        assert_eq!(meta.meaning, "Close the current view");
        assert_eq!(meta.tags, vec!["close", "dismiss", "x"]);
    }

    #[test]
    fn missing_meta_is_empty() {
        let meta = parse_meta("documentsize 24x24\nshape s template=rectangle\n");
        assert!(meta.meaning.is_empty());
        assert!(meta.tags.is_empty());
    }

    #[test]
    fn dedups_tags() {
        let meta = parse_meta("# @tags a, b, a, b, c\ndocumentsize 24x24\n");
        assert_eq!(meta.tags, vec!["a", "b", "c"]);
    }

    #[test]
    fn manifest_json_schema_shape() {
        let m = Manifest {
            version: MANIFEST_VERSION,
            icons: vec![IconEntry {
                name: "arrow-left".into(),
                meaning: "Go back".into(),
                tags: vec!["back".into(), "previous".into()],
                canvas: (24.0, 24.0),
                sizes: vec![16, 24],
            }],
        };
        let s = m.to_json_string();
        assert!(s.contains("\"version\": 1"));
        assert!(s.contains("\"count\": 1"));
        assert!(s.contains("\"name\": \"arrow-left\""));
        // sizes is a nested array of integers (formatted via fmt_num).
        assert!(s.contains("\"sizes\": ["));
        assert!(s.contains("16,"));
        assert!(s.contains("24"));
        assert!(s.contains("\"tags\": ["));
    }
}
