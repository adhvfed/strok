//! DTCG token target — emit the design-token system as a W3C **Design Tokens
//! Community Group** (DTCG) `design-tokens.json` file (C9 / E4.3).
//!
//! This is the co-equal sibling of the Tailwind target: the *same* generalized
//! token set ([`Scene::all_tokens`]) projected into the interoperable DTCG draft
//! format instead of a Tailwind `@theme` block. A DTCG file round-trips through
//! Style Dictionary, Tokens Studio, and the broader token tooling ecosystem, so
//! a Strøk design system is portable beyond Tailwind.
//!
//! ## Format (Decision D-5: conformance scope)
//!
//! Per the DTCG draft (`https://tr.designtokens.org/format/`, Aug-2024 editor's
//! draft), a token is an object with a `$value` and an optional `$type`; tokens
//! are organized into nested groups. We emit one **group per category** and one
//! token per `(name, value)`, with the `$type` set from the category:
//!
//! | Strøk category      | DTCG `$type`   |
//! |---------------------|----------------|
//! | `color`             | `color`        |
//! | `space` / `spacing` | `dimension`    |
//! | `radius`            | `dimension`    |
//! | `font`              | `fontFamily`   |
//! | `shadow`            | `shadow`       |
//! | `motion` / `ease`   | `cubicBezier`* |
//! | (other)             | (omitted)      |
//!
//! *`motion` values that aren't a 4-tuple are emitted with no `$type` (the draft
//! permits a typeless token; the group `$type` is not asserted). We **start with
//! color/space/radius/font/shadow** as the spec'd scope (D-5) and let any other
//! category pass through as a typeless token rather than dropping it — graceful
//! degradation, surfaced in diagnostics.

use strok_core::json::Json;
use strok_core::scene::Scene;

use crate::target::{Capabilities, EmitArtifact, EmitFile, EmitOptions, Result, Target};

pub struct DtcgTarget;

/// Map a Strøk token category to a DTCG `$type`. `None` ⇒ emit the token without
/// a `$type` (still valid DTCG — the value is then untyped).
fn dtcg_type(category: &str) -> Option<&'static str> {
    match category {
        "color" => Some("color"),
        "space" | "spacing" | "radius" => Some("dimension"),
        "font" => Some("fontFamily"),
        "shadow" => Some("shadow"),
        _ => None,
    }
}

/// Render a token value into its DTCG `$value`. Dimensions get a unit (`px`) when
/// the value is a bare number; colors and everything else stay strings; quoted
/// font names are unquoted.
fn dtcg_value(category: &str, value: &str) -> Json {
    match dtcg_type(category) {
        Some("dimension") => {
            // A bare number becomes a px dimension string; a value that already
            // carries a unit is kept verbatim.
            if value.parse::<f64>().is_ok() {
                Json::str(format!("{value}px"))
            } else {
                Json::str(value)
            }
        }
        _ => Json::str(value.trim_matches('"')),
    }
}

impl Target for DtcgTarget {
    fn id(&self) -> &'static str {
        "dtcg"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            vector: false,
            raster: false,
            components: false,
            auto_layout: false,
            interactivity: false,
        }
    }

    fn emit(&self, scene: &Scene, _opts: &EmitOptions) -> Result<EmitArtifact> {
        // Group tokens by category, preserving first-seen order for both groups
        // and members → deterministic, snapshot-stable output.
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for token in scene.all_tokens() {
            match groups.iter_mut().find(|(cat, _)| *cat == token.category) {
                Some((_, members)) => members.push((token.name, token.value)),
                None => groups.push((token.category, vec![(token.name, token.value)])),
            }
        }

        let mut root: Vec<(String, Json)> = Vec::new();
        for (category, members) in &groups {
            let mut group: Vec<(String, Json)> = Vec::new();
            // Per the DTCG draft a group MAY carry a `$type` its members inherit;
            // we set it when the category has a known type so tooling reads the
            // group's intent even before per-token `$type`.
            if let Some(ty) = dtcg_type(category) {
                group.push(("$type".to_string(), Json::str(ty)));
            }
            for (name, value) in members {
                group.push((
                    name.clone(),
                    Json::Object(vec![("$value".to_string(), dtcg_value(category, value))]),
                ));
            }
            root.push((category.clone(), Json::Object(group)));
        }

        let json = Json::Object(root).to_string_pretty();

        let mut diagnostics = Vec::new();
        if scene.all_tokens().is_empty() {
            diagnostics
                .push("scene has no design tokens; emitted an empty DTCG document".to_string());
        }
        for (category, _) in &groups {
            if dtcg_type(category).is_none() {
                diagnostics.push(format!(
                    "token category '{category}' has no DTCG $type (D-5 scope is \
                     color/space/radius/font/shadow); emitted typeless"
                ));
            }
        }
        if !scene.palette.schemes.is_empty() {
            diagnostics.push(format!(
                "{} colorscheme(s) present; only the base palette is emitted to DTCG \
                 (per-scheme emission is a follow-up)",
                scene.palette.schemes.len()
            ));
        }

        Ok(EmitArtifact {
            files: vec![EmitFile {
                path: "design-tokens.json".to_string(),
                contents: json,
            }],
            assets: Vec::new(),
            diagnostics,
        })
    }
}
