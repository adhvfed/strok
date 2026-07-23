//! Solid backend, co-equal with the React target.
//!
//! Differs from React in exactly two knobs (`class`, `innerHTML`). Everything
//! else comes from the shared walker in [`crate::backends::jsx`].

use crate::backends::jsx::{escape_js, pascal_case, JsxDialect};
use crate::ir::UiDoc;
use crate::target::{Capabilities, EmitArtifact, EmitFile, EmitOptions, FrameworkBackend};

pub struct SolidBackend;

fn solid_svg_attr(svg: &str) -> String {
    format!("innerHTML={{\"{}\"}}", escape_js(svg))
}

const DIALECT: JsxDialect = JsxDialect {
    class_attr: "class",
    svg_attr: solid_svg_attr,
    header: "",
};

impl FrameworkBackend for SolidBackend {
    fn id(&self) -> &'static str {
        "solid"
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
