//! Embedded standard shape library (EXP-1).
//!
//! Agents authoring illustrations kept re-deriving "a person", "an arrow", "a
//! speech bubble" from raw primitives every time. This module embeds a small
//! set of reusable, geometry-only `.strok` modules directly in the `strok`
//! binary (via `include_str!` — no files on disk needed) so any document can
//! pull them in with:
//!
//! ```text
//! use "std/figures" as fig
//! place p shape=fig.person-standing at=0,0 size=40x100
//!   fill #2d5a1e
//! ```
//!
//! `dsl_parse::resolve_imports` intercepts any import path starting with
//! `std/` (with or without a `.strok` suffix) *before* touching the
//! filesystem and parses the embedded source instead — see that function for
//! the interception point.
//!
//! Source of truth for the library content lives in the repo at
//! `std/<module>.strok` — ordinary `.strok` files, so they parse with the
//! same parser everything else uses (tested in `dsl_parse::tests`) and stay
//! readable/editable like any other document.

/// One embedded module: its bare name (no `std/` prefix, no `.strok` suffix)
/// and its raw `.strok` source.
pub struct Module {
    pub name: &'static str,
    pub source: &'static str,
}

macro_rules! std_module {
    ($name:literal, $path:literal) => {
        Module {
            name: $name,
            source: include_str!($path),
        }
    };
}

/// All embedded standard-library modules, in a stable order (also the order
/// `strok lib list` prints them in).
static MODULES: &[Module] = &[
    std_module!("figures", "../../std/figures.strok"),
    std_module!("arrows", "../../std/arrows.strok"),
    std_module!("bubbles", "../../std/bubbles.strok"),
    std_module!("devices", "../../std/devices.strok"),
    std_module!("furniture", "../../std/furniture.strok"),
];

/// Every embedded module, name + source.
pub fn modules() -> &'static [Module] {
    MODULES
}

/// Look up an embedded module's source by bare name (no `std/` prefix, no
/// `.strok` suffix). Returns `None` for an unknown module.
pub fn get(module: &str) -> Option<&'static str> {
    MODULES.iter().find(|m| m.name == module).map(|m| m.source)
}

/// Normalize a `use` import path into a bare std-module name if it refers to
/// the embedded standard library, e.g. `std/figures`, `std/figures.strok`,
/// `./std/figures` all normalize to `figures`. Returns `None` if the path
/// does not start with `std/` (a plain filesystem import).
pub fn strip_std_prefix(path: &str) -> Option<&str> {
    let trimmed = path.trim_start_matches("./");
    let rest = trimmed.strip_prefix("std/")?;
    Some(rest.strip_suffix(".strok").unwrap_or(rest))
}

/// A comma-joined list of available module names, for error messages.
pub fn available_names() -> String {
    MODULES
        .iter()
        .map(|m| m.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One shape's `# @meaning` / `# @tags` annotation, extracted from the
/// comment block directly above its `shape <name> template=…` line. Reuses
/// the same authoring convention as the icon manifest (`manifest::parse_meta`)
/// but scans per-shape rather than only the file header, since a module file
/// defines many shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeMeta {
    pub name: String,
    pub meaning: String,
    pub tags: Vec<String>,
}

/// Extract `ShapeMeta` for every `shape …` definition in a `.strok` source,
/// in file order. A shape with no leading annotation still gets an entry
/// (empty meaning/tags) — simple line scanning, no parser dependency, so it
/// works even on a source that fails to parse.
pub fn shapes_meta(source: &str) -> Vec<ShapeMeta> {
    let mut out = Vec::new();
    let mut pending_meaning = String::new();
    let mut pending_tags: Vec<String> = Vec::new();

    for line in source.lines() {
        let t = line.trim();
        if t.is_empty() {
            // A blank line breaks the association between a comment block
            // and the shape that follows it two-or-more lines down, but not
            // between comment lines themselves.
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim();
            if let Some(m) = rest.strip_prefix("@meaning") {
                pending_meaning = m.trim().to_string();
            } else if let Some(tg) = rest.strip_prefix("@tags") {
                pending_tags = tg
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("shape ") {
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            if !name.is_empty() {
                out.push(ShapeMeta {
                    name,
                    meaning: std::mem::take(&mut pending_meaning),
                    tags: std::mem::take(&mut pending_tags),
                });
            }
            continue;
        }
        // Any other non-comment top-level content (indented ops, `use`,
        // `documentsize`, …) doesn't carry annotations forward past it,
        // except indentation inside a shape block, which we don't reset on
        // since annotations only ever precede a `shape` line anyway.
        if line.starts_with(|c: char| !c.is_whitespace()) {
            pending_meaning.clear();
            pending_tags.clear();
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl_parse::parse_file;

    #[test]
    fn every_module_parses() {
        for m in modules() {
            parse_file(m.source)
                .unwrap_or_else(|e| panic!("std/{} failed to parse: {}", m.name, e));
        }
    }

    #[test]
    fn every_module_has_at_least_one_annotated_shape() {
        for m in modules() {
            assert!(
                m.source.contains("# @meaning"),
                "std/{} has no @meaning annotations",
                m.name
            );
        }
    }

    #[test]
    fn get_known_and_unknown() {
        assert!(get("figures").is_some());
        assert!(get("nope").is_none());
    }

    #[test]
    fn strip_std_prefix_variants() {
        assert_eq!(strip_std_prefix("std/figures"), Some("figures"));
        assert_eq!(strip_std_prefix("std/figures.strok"), Some("figures"));
        assert_eq!(strip_std_prefix("./std/figures"), Some("figures"));
        assert_eq!(strip_std_prefix("./face.strok"), None);
        assert_eq!(strip_std_prefix("components.strok"), None);
    }
}
