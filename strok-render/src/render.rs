use strok_core::document::Document;
use strok_core::emit;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("svg parse error: {0}")]
    SvgParse(String),
    #[error("render error: {0}")]
    Render(String),
    #[error("png encode error: {0}")]
    PngEncode(String),
}

#[derive(Default)]
pub struct RenderOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub background: Option<String>,
    /// Concrete color to substitute for `currentColor` when rasterizing. PNG has
    /// no inherited `color` context, so icons authored with `currentColor` need a
    /// value here (default black). SVG export keeps `currentColor` verbatim.
    pub color: Option<String>,
}

pub fn render_to_png(doc: &Document, opts: &RenderOptions) -> Result<Vec<u8>, RenderError> {
    let svg_str = emit::emit_document(doc);
    let (w, h) = target_dimensions(doc.width, doc.height, opts.width, opts.height);
    render_svg_string(&svg_str, w, h, doc.width, doc.height, opts)
}

fn target_dimensions(
    doc_width: f64,
    doc_height: f64,
    requested_width: Option<u32>,
    requested_height: Option<u32>,
) -> (u32, u32) {
    match (requested_width, requested_height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => {
            let height = (width as f64 * doc_height / doc_width).round().max(1.0) as u32;
            (width, height)
        }
        (None, Some(height)) => {
            let width = (height as f64 * doc_width / doc_height).round().max(1.0) as u32;
            (width, height)
        }
        (None, None) => (
            doc_width.round().max(1.0) as u32,
            doc_height.round().max(1.0) as u32,
        ),
    }
}

/// Render an SVG string to PNG bytes.
/// `target_w`/`target_h` are the output pixel dimensions.
/// `doc_w`/`doc_h` are the SVG document dimensions (for scaling).
pub fn render_svg_string(
    svg: &str,
    target_w: u32,
    target_h: u32,
    doc_w: f64,
    doc_h: f64,
    opts: &RenderOptions,
) -> Result<Vec<u8>, RenderError> {
    // `currentColor` has no meaning to a standalone rasterizer (no inherited
    // `color`). Substitute the caller's color (default black) before parsing so
    // icons authored with `currentColor` render with a concrete ink.
    let ink = opts.color.as_deref().unwrap_or("#000000");
    let svg_owned;
    let svg = if svg.contains("currentColor") {
        svg_owned = svg.replace("currentColor", ink);
        svg_owned.as_str()
    } else {
        svg
    };

    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree =
        resvg::usvg::Tree::from_str(svg, &opt).map_err(|e| RenderError::SvgParse(e.to_string()))?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(target_w, target_h)
        .ok_or_else(|| RenderError::Render("failed to create pixmap".to_string()))?;

    if let Some(ref bg) = opts.background {
        if let Some(color) = parse_color(bg) {
            pixmap.fill(color);
        }
    }

    let sx = target_w as f32 / doc_w as f32;
    let sy = target_h as f32 / doc_h as f32;
    let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap
        .encode_png()
        .map_err(|e| RenderError::PngEncode(e.to_string()))
}

fn parse_color(s: &str) -> Option<resvg::tiny_skia::Color> {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(resvg::tiny_skia::Color::from_rgba8(r, g, b, 255))
    } else {
        None
    }
}
