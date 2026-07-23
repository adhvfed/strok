//! React backend. Co-equal with Solid — no privileged status.
//!
//! Differs from Solid in exactly two knobs (`className`, `dangerouslySetInnerHTML`).
//! Everything else comes from the shared walker in [`crate::backends::jsx`].

use crate::backends::jsx::{escape_js, pascal_case, JsxDialect};
use crate::ir::UiDoc;
use crate::target::{Capabilities, EmitArtifact, EmitFile, EmitOptions, FrameworkBackend};

pub struct ReactBackend;

fn react_svg_attr(svg: &str) -> String {
    format!(
        "dangerouslySetInnerHTML={{{{ __html: \"{}\" }}}}",
        escape_js(svg)
    )
}

const DIALECT: JsxDialect = JsxDialect {
    class_attr: "className",
    svg_attr: react_svg_attr,
    header: "",
};

impl FrameworkBackend for ReactBackend {
    fn id(&self) -> &'static str {
        "react"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            vector: true,
            raster: true,
            components: true,
            auto_layout: true,
            interactivity: true,
        }
    }

    fn render(&self, doc: &UiDoc, _opts: &EmitOptions) -> EmitArtifact {
        let source = DIALECT.render_component(&doc.name, &doc.root);
        EmitArtifact {
            files: vec![EmitFile {
                path: format!("{}.tsx", pascal_case(&doc.name)),
                contents: source,
            }],
            assets: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}
