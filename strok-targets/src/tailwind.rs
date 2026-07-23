//! Tailwind token target — the design-token half of the "no integration pain"
//! bridge (design doc §4.2, §8).
//!
//! Emits a Tailwind v4 `@theme` block from the scene's palette. Per the project
//! Tailwind v4 rules: plain `@theme` (not `@theme inline`, so the custom
//! properties are emitted and available at runtime) and `--color-<name>`
//! directly (no `:root` + `@theme inline` indirection). This is the *same*
//! shape a frontend `@theme` consumes — edit a token here, and the app's
//! theme moves with it, no re-export.

use strok_core::scene::Scene;

use crate::target::{Capabilities, EmitArtifact, EmitFile, EmitOptions, Result, Target};

pub struct TailwindTarget;

/// Map a Strøk token category to its Tailwind v4 `@theme` namespace. Unknown
/// categories pass through verbatim so the bridge degrades gracefully rather
/// than dropping tokens.
fn theme_namespace(category: &str) -> String {
    match category {
        "color" => "color".to_string(),
        "space" | "spacing" => "spacing".to_string(),
        "radius" => "radius".to_string(),
        "font" => "font".to_string(),
        "shadow" => "shadow".to_string(),
        "motion" | "ease" => "ease".to_string(),
        other => other.to_string(),
    }
}

impl Target for TailwindTarget {
    fn id(&self) -> &'static str {
        "tailwind"
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
        // Every design token, across categories (C8 / E4.1): palette colors are
        // surfaced under `color`, plus the generalized `tokens` block. Each
        // category maps to a Tailwind v4 `@theme` namespace.
        let mut theme = String::from("@theme {\n");
        for token in scene.all_tokens() {
            let var = format!("--{}-{}", theme_namespace(&token.category), token.name);
            theme.push_str(&format!("  {}: {};\n", var, token.value));
        }
        theme.push_str("}\n");

        let mut diagnostics = Vec::new();
        if scene.all_tokens().is_empty() {
            diagnostics
                .push("scene has no design tokens; emitted an empty @theme block".to_string());
        }
        if !scene.palette.schemes.is_empty() {
            diagnostics.push(format!(
                "{} colorscheme(s) present; only the base palette is emitted to @theme \
                 (per-scheme emission is a follow-up)",
                scene.palette.schemes.len()
            ));
        }

        Ok(EmitArtifact {
            files: vec![EmitFile {
                path: "theme.css".to_string(),
                contents: theme,
            }],
            assets: Vec::new(),
            diagnostics,
        })
    }
}
