//! Token sync between an icon set and the P4 design-token system (C10 / E5.3).
//!
//! An icon set and the design system that consumes it must agree on tokens: an
//! icon that paints with `$accent` or `$color.ink` only renders correctly if the
//! design system actually *defines* that token. This module cross-checks the two:
//! it collects every `$`-token an icon set references and compares them against
//! the tokens a design-system scene defines ([`Scene::all_tokens`]), reporting
//!
//! - **undefined** references (icons use a token the system doesn't define — a
//!   broken icon waiting to happen), and
//! - **unused** tokens (the system defines tokens no icon references — dead
//!   weight or a naming drift).
//!
//! This is a *report*, not a mutation: the stateless CLI surfaces the drift (with
//! `--json` for tooling) so the human/agent fixes the source of truth. It is the
//! design-system half of the same `$token` resolution the renderer already does.

use crate::json::Json;
use crate::scene::Scene;
use std::collections::BTreeSet;

/// The result of cross-checking an icon set against a design-token system.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    /// Every token name the design system defines (dotted spelling for design
    /// tokens; bare name for legacy palette colors — both are how they're
    /// referenced).
    pub defined: Vec<String>,
    /// Token references that appear in icons but are NOT defined by the system.
    pub undefined: Vec<String>,
    /// Defined tokens that NO icon references.
    pub unused: Vec<String>,
    /// Token references that resolve cleanly (defined and used).
    pub matched: Vec<String>,
}

impl SyncReport {
    /// True when every icon reference resolves (no undefined tokens). `unused`
    /// tokens do not make the set out-of-sync — they're informational.
    pub fn is_in_sync(&self) -> bool {
        self.undefined.is_empty()
    }

    pub fn to_json(&self) -> Json {
        let arr = |v: &[String]| Json::array(v.iter().map(Json::str));
        Json::obj([
            ("in_sync", Json::Bool(self.is_in_sync())),
            ("defined", arr(&self.defined)),
            ("matched", arr(&self.matched)),
            ("undefined", arr(&self.undefined)),
            ("unused", arr(&self.unused)),
        ])
    }

    pub fn to_json_string(&self) -> String {
        self.to_json().to_string_pretty()
    }
}

/// Names a design system defines, in both the spellings a reference can use. A
/// `DesignToken` is referenced dotted (`$color.ink`); a legacy `palette` color is
/// *also* surfaced as a bare name (`$ink`) for backwards-compat — so we accept
/// both for a color token.
fn defined_names(system: &Scene) -> (BTreeSet<String>, Vec<String>) {
    let mut accept: BTreeSet<String> = BTreeSet::new();
    let mut canonical: Vec<String> = Vec::new();
    for t in system.all_tokens() {
        let dotted = t.dotted();
        // Canonical (reported) spelling is dotted.
        if !canonical.contains(&dotted) {
            canonical.push(dotted.clone());
        }
        accept.insert(dotted);
        // A color also matches its bare name (legacy `$copper`).
        if t.category == "color" {
            accept.insert(t.name.clone());
        }
    }
    canonical.sort();
    (accept, canonical)
}

/// Collect every `$`-token reference in a parsed icon scene, by scanning the
/// authored DSL source (references live in attribute *values* like `fill $accent`,
/// `stroke $color.ink`). Scanning the source is robust to where the reference
/// appears and matches the renderer's `$`-resolution surface.
pub fn references_in_source(source: &str) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_alphanumeric() || c == b'.' || c == b'-' || c == b'_' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > start {
                refs.insert(source[start..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    refs
}

/// Cross-check the icon-set token references against the design-system tokens.
///
/// `references` is the union of `references_in_source` over every icon file;
/// `system` is the parsed design-system scene (the token source of truth).
pub fn sync(references: &BTreeSet<String>, system: &Scene) -> SyncReport {
    let (accept, defined) = defined_names(system);

    let mut undefined: Vec<String> = Vec::new();
    let mut matched: Vec<String> = Vec::new();
    for r in references {
        if accept.contains(r) {
            matched.push(r.clone());
        } else {
            undefined.push(r.clone());
        }
    }
    undefined.sort();
    matched.sort();

    // A defined token is "used" if any reference resolves to it. A color token is
    // hit by either its dotted or bare spelling.
    let unused: Vec<String> = defined
        .iter()
        .filter(|d| {
            let bare = d.strip_prefix("color.").unwrap_or(d);
            !references.iter().any(|r| r == *d || r == bare)
        })
        .cloned()
        .collect();

    SyncReport {
        defined,
        undefined,
        unused,
        matched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl_parse;

    fn system() -> Scene {
        let src = "\
documentsize 24x24
palette
  copper #b87333
tokens
  color.ink #1a1a1a
  space.md 16
";
        dsl_parse::parse_file(src).unwrap()
    }

    #[test]
    fn collects_references() {
        let src = "shape s template=rectangle\n  fill $accent\n  stroke $color.ink\n";
        let refs = references_in_source(src);
        assert!(refs.contains("accent"));
        assert!(refs.contains("color.ink"));
    }

    #[test]
    fn flags_undefined_and_unused() {
        let mut refs = BTreeSet::new();
        refs.insert("color.ink".to_string()); // defined (dotted)
        refs.insert("copper".to_string()); // defined (legacy bare color)
        refs.insert("missing".to_string()); // undefined
        let rep = sync(&refs, &system());
        assert!(!rep.is_in_sync());
        assert_eq!(rep.undefined, vec!["missing"]);
        // space.md defined but never referenced.
        assert!(rep.unused.iter().any(|u| u == "space.md"));
        assert!(rep.matched.contains(&"color.ink".to_string()));
        assert!(rep.matched.contains(&"copper".to_string()));
    }

    #[test]
    fn in_sync_when_all_resolve() {
        let mut refs = BTreeSet::new();
        refs.insert("color.ink".to_string());
        refs.insert("copper".to_string());
        refs.insert("space.md".to_string());
        let rep = sync(&refs, &system());
        assert!(rep.is_in_sync());
        assert!(rep.unused.is_empty());
    }
}
