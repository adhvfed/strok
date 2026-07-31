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
    #[error("invalid render region: {0}")]
    InvalidRegion(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
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
    /// Optional crop in document coordinates. Rendering a focused region at a
    /// large output size is useful for inspecting fine geometry and materials.
    pub region: Option<RenderRegion>,
}

pub fn render_to_png(doc: &Document, opts: &RenderOptions) -> Result<Vec<u8>, RenderError> {
    let svg_str = emit::emit_document(doc);
    let (source_w, source_h) = opts
        .region
        .map(|region| (region.width, region.height))
        .unwrap_or((doc.width, doc.height));
    let (w, h) = target_dimensions(source_w, source_h, opts.width, opts.height);
    render_svg_string(&svg_str, w, h, doc.width, doc.height, opts)
}

pub fn target_dimensions(
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

    let (source_x, source_y, source_w, source_h) = match opts.region {
        Some(region) => {
            validate_region(region, doc_w, doc_h)?;
            (region.x, region.y, region.width, region.height)
        }
        None => (0.0, 0.0, doc_w, doc_h),
    };
    let sx = target_w as f32 / source_w as f32;
    let sy = target_h as f32 / source_h as f32;
    let transform = resvg::tiny_skia::Transform::from_row(
        sx,
        0.0,
        0.0,
        sy,
        -(source_x as f32) * sx,
        -(source_y as f32) * sy,
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap
        .encode_png()
        .map_err(|e| RenderError::PngEncode(e.to_string()))
}

fn validate_region(
    region: RenderRegion,
    doc_width: f64,
    doc_height: f64,
) -> Result<(), RenderError> {
    let values = [
        region.x,
        region.y,
        region.width,
        region.height,
        doc_width,
        doc_height,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(RenderError::InvalidRegion(
            "coordinates and dimensions must be finite".to_string(),
        ));
    }
    if region.x < 0.0 || region.y < 0.0 {
        return Err(RenderError::InvalidRegion(
            "x and y must be zero or greater".to_string(),
        ));
    }
    if region.width <= 0.0 || region.height <= 0.0 {
        return Err(RenderError::InvalidRegion(
            "width and height must be greater than zero".to_string(),
        ));
    }
    const EPSILON: f64 = 1e-9;
    if region.x + region.width > doc_width + EPSILON
        || region.y + region.height > doc_height + EPSILON
    {
        return Err(RenderError::InvalidRegion(format!(
            "region {},{},{},{} exceeds the {}x{} document",
            region.x, region.y, region.width, region.height, doc_width, doc_height
        )));
    }
    Ok(())
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
