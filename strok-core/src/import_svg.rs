//! SVG → `.strok` importer with structure recovery (EXP-3).
//!
//! This is the bridge that lets image-generation → vectorization pipelines land
//! in Strøk as *editable, semantic* documents — not a dumb path dump. It parses
//! SVG with a small dependency-free XML pull-parser (keeping `strok-core` free of
//! new crates) and reconstructs Strøk's two-concept model (`shape` + `place`):
//!
//! - `<rect>`/`<circle>`/`<ellipse>`/`<line>` recover their native templates
//!   (rectangle/ellipse/line) placed with `at`/`size`, so the output reads like a
//!   person wrote it.
//! - `<path>`/`<polygon>`/`<polyline>` become `path` shapes; `d` is decomposed
//!   into `addpoint` ops (lines → sharp, cubics → `mode=controls`, arcs →
//!   `mode=arc`), falling back to flattening with a warning where unrepresentable.
//! - Identical geometry used in several places collapses to ONE shape definition
//!   plus multiple `place`s (reuse detection by normalized geometry + style).
//! - Colors used ≥ 2× are lifted into a `palette` token named by hue.
//! - `<g>` becomes a `group`; transforms are recovered as `at`/`size` where
//!   clean, and baked into geometry (with a warning) otherwise.
//! - Unsupported constructs (filters, `<use>`, `<image>`, gradients beyond a flat
//!   approximation, …) are skipped with a named warning — never a panic.
//!
//! No-panic policy: malformed SVG yields warnings + a best-effort `Scene`, never
//! a panic (see the garbage-bytes unit test).

mod xml;

use crate::scene::{Group, Palette, Place, PlacePosition, Scene, SceneNode};
use crate::shape::{CornerRadii, Operation, Shape, Template};
use crate::types::{Color, Dimension, PointMode};
use xml::{local_name, parse_xml, XmlNode};

/// A non-fatal issue encountered while importing. Import always succeeds with a
/// best-effort scene; warnings tell the caller what degraded.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportWarning {
    pub message: String,
    /// 1-based source line, when known.
    pub line: Option<usize>,
}

impl ImportWarning {
    fn new(message: impl Into<String>, line: Option<usize>) -> Self {
        ImportWarning {
            message: message.into(),
            line,
        }
    }
}

/// The result of an import: the reconstructed scene, per-kind element counts, the
/// extracted palette token names, and any warnings.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub scene: Scene,
    pub warnings: Vec<ImportWarning>,
    /// Counts of recovered source elements by SVG tag (`rect`, `path`, …).
    pub counts: Vec<(String, usize)>,
    /// Names of palette tokens extracted from repeated colors.
    pub tokens: Vec<String>,
}

// ── Public entry point ─────────────────────────────────────────────────

/// Import an SVG document string into a Strøk [`Scene`] with structure recovery.
///
/// Never fails on malformed input: unrepresentable or broken constructs are
/// reported as [`ImportWarning`]s and the best-effort scene is still returned.
pub fn import_svg(src: &str) -> ImportResult {
    let mut warnings = Vec::new();

    let root = match parse_xml(src, &mut warnings) {
        Some(r) => r,
        None => {
            warnings.push(ImportWarning::new("no <svg> root element found", None));
            return ImportResult {
                scene: Scene::new(Dimension { w: 100.0, h: 100.0 }),
                warnings,
                counts: Vec::new(),
                tokens: Vec::new(),
            };
        }
    };

    // Document size + viewBox offset.
    let (size, vb_offset) = document_size(&root, &mut warnings);

    let mut ctx = ImportCtx {
        warnings,
        counts: std::collections::BTreeMap::new(),
        color_counts: std::collections::BTreeMap::new(),
        gradients: std::collections::BTreeMap::new(),
        shapes: Vec::new(),
        shape_keys: std::collections::HashMap::new(),
        used_names: std::collections::HashSet::new(),
        kind_counters: std::collections::BTreeMap::new(),
    };

    // Pre-scan <defs> for gradient first-stop colors (flat approximation).
    prescan_gradients(&root, &mut ctx);

    // Root transform = viewBox origin offset (min-x/min-y → 0,0).
    let root_m = Mat::translate(vb_offset.0, vb_offset.1);

    // First pass over the tree: count colors so we know which to tokenize.
    count_colors(&root, &Style::default(), &mut ctx);
    let palette = build_palette(&ctx);

    // Second pass: build nodes.
    let mut nodes = Vec::new();
    convert_children(
        &root,
        root_m,
        &Style::default(),
        &palette,
        &mut ctx,
        &mut nodes,
    );

    let ImportCtx {
        warnings,
        counts,
        shapes,
        ..
    } = ctx;

    let mut scene = Scene::new(size);
    scene.palette = Palette {
        tokens: palette.tokens.clone(),
        schemes: Vec::new(),
    };
    scene.shapes = shapes;
    scene.nodes = nodes;

    let counts: Vec<(String, usize)> = counts.into_iter().collect();
    let tokens: Vec<String> = palette.tokens.iter().map(|(n, _)| n.clone()).collect();

    ImportResult {
        scene,
        warnings,
        counts,
        tokens,
    }
}

// ── Import context ─────────────────────────────────────────────────────

struct ImportCtx {
    warnings: Vec<ImportWarning>,
    counts: std::collections::BTreeMap<String, usize>,
    color_counts: std::collections::BTreeMap<String, usize>,
    /// gradient id → flat approximation hex.
    gradients: std::collections::BTreeMap<String, String>,
    shapes: Vec<Shape>,
    /// canonical shape key → shape name (reuse detection).
    shape_keys: std::collections::HashMap<String, String>,
    used_names: std::collections::HashSet<String>,
    kind_counters: std::collections::BTreeMap<String, usize>,
}

impl ImportCtx {
    fn warn(&mut self, message: impl Into<String>, line: Option<usize>) {
        self.warnings.push(ImportWarning::new(message, line));
    }
    fn bump(&mut self, kind: &str) {
        *self.counts.entry(kind.to_string()).or_insert(0) += 1;
    }
}

/// The resolved palette: color hex → token name, plus the ordered token list.
#[derive(Default, Clone)]
struct ResolvedPalette {
    tokens: Vec<(String, String)>,
    by_hex: std::collections::HashMap<String, String>,
}

// ── Affine transform ───────────────────────────────────────────────────

/// A 2×3 affine matrix in SVG convention: `x' = a*x + c*y + e`,
/// `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy)]
struct Mat {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Mat {
    fn identity() -> Self {
        Mat {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
    fn translate(x: f64, y: f64) -> Self {
        Mat {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
    fn mul(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    /// True when there is no rotation or shear (only translate + axis scale/flip).
    fn is_axis_aligned(&self) -> bool {
        self.b.abs() < 1e-9 && self.c.abs() < 1e-9
    }
    fn scale_x(&self) -> f64 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
    fn scale_y(&self) -> f64 {
        (self.c * self.c + self.d * self.d).sqrt()
    }
}

/// Parse an SVG `transform` attribute into a composed matrix.
fn parse_transform(s: &str) -> Mat {
    let mut m = Mat::identity();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // read function name
        while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if name_start == i {
            break;
        }
        let name = &s[name_start..i];
        // read args in parens
        while i < bytes.len() && bytes[i] != b'(' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let args_start = i + 1;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        let args = &s[args_start..i.min(s.len())];
        i += 1; // past ')'
        let nums: Vec<f64> = args
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        let t = match name {
            "translate" => Mat::translate(
                nums.first().copied().unwrap_or(0.0),
                nums.get(1).copied().unwrap_or(0.0),
            ),
            "scale" => {
                let sx = nums.first().copied().unwrap_or(1.0);
                let sy = nums.get(1).copied().unwrap_or(sx);
                Mat {
                    a: sx,
                    b: 0.0,
                    c: 0.0,
                    d: sy,
                    e: 0.0,
                    f: 0.0,
                }
            }
            "rotate" => {
                let deg = nums.first().copied().unwrap_or(0.0);
                let r = deg.to_radians();
                let (sin, cos) = (r.sin(), r.cos());
                let rot = Mat {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                };
                if nums.len() >= 3 {
                    let (cx, cy) = (nums[1], nums[2]);
                    Mat::translate(cx, cy)
                        .mul(&rot)
                        .mul(&Mat::translate(-cx, -cy))
                } else {
                    rot
                }
            }
            "matrix" if nums.len() == 6 => Mat {
                a: nums[0],
                b: nums[1],
                c: nums[2],
                d: nums[3],
                e: nums[4],
                f: nums[5],
            },
            "skewX" => {
                let t = nums.first().copied().unwrap_or(0.0).to_radians().tan();
                Mat {
                    a: 1.0,
                    b: 0.0,
                    c: t,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                }
            }
            "skewY" => {
                let t = nums.first().copied().unwrap_or(0.0).to_radians().tan();
                Mat {
                    a: 1.0,
                    b: t,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                }
            }
            _ => Mat::identity(),
        };
        m = m.mul(&t);
    }
    m
}

// ── Style inheritance ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct Style {
    fill: Option<String>,
    stroke: Option<String>,
    stroke_width: Option<f64>,
    opacity: Option<f64>,
    stroke_linecap: Option<String>,
    stroke_linejoin: Option<String>,
    stroke_dasharray: Option<String>,
    font_size: Option<f64>,
    font_family: Option<String>,
    font_weight: Option<String>,
    text_anchor: Option<String>,
}

impl Style {
    /// Layer this element's presentation attributes over the inherited style.
    fn inherit(parent: &Style, el: &XmlNode) -> Style {
        let mut s = parent.clone();
        // presentation attributes then style="" (style wins).
        for (k, v) in &el.attrs {
            apply_style_prop(&mut s, k, v);
        }
        if let Some(style) = el.attr("style") {
            for decl in style.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    apply_style_prop(&mut s, k.trim(), v.trim());
                }
            }
        }
        s
    }
}

fn apply_style_prop(s: &mut Style, k: &str, v: &str) {
    match k {
        "fill" => s.fill = Some(v.to_string()),
        "stroke" => s.stroke = Some(v.to_string()),
        "stroke-width" => s.stroke_width = parse_len(v),
        "opacity" => s.opacity = v.parse().ok(),
        "stroke-linecap" => s.stroke_linecap = Some(v.to_string()),
        "stroke-linejoin" => s.stroke_linejoin = Some(v.to_string()),
        "stroke-dasharray" => s.stroke_dasharray = Some(v.to_string()),
        "font-size" => s.font_size = parse_len(v),
        "font-family" => s.font_family = Some(v.trim_matches(['"', '\'']).to_string()),
        "font-weight" => s.font_weight = Some(v.to_string()),
        "text-anchor" => s.text_anchor = Some(v.to_string()),
        _ => {}
    }
}

/// Build the style operations for a shape, resolving colors to tokens.
fn style_ops(
    style: &Style,
    palette: &ResolvedPalette,
    is_line: bool,
    ctx: &mut ImportCtx,
) -> Vec<Operation> {
    let mut ops = Vec::new();

    // Fill: default SVG fill is black; a line/stroke-only shape should stay hollow.
    let fill = style.fill.clone().unwrap_or_else(|| {
        if is_line {
            "none".to_string()
        } else {
            "#000000".to_string()
        }
    });
    if let Some(c) = resolve_color(&fill, palette, ctx) {
        ops.push(Operation::Fill(c));
    }
    if let Some(sw) = &style.stroke {
        if let Some(c) = resolve_color(sw, palette, ctx) {
            if c != Color::None {
                ops.push(Operation::Stroke(c));
                let w = style.stroke_width.unwrap_or(1.0);
                ops.push(Operation::StrokeWidth(crate::types::AbsoluteSize(w)));
                if let Some(cap) = &style.stroke_linecap {
                    if let Ok(c) = crate::types::LineCap::parse(cap) {
                        ops.push(Operation::StrokeLinecap(c));
                    }
                }
                if let Some(join) = &style.stroke_linejoin {
                    if let Ok(j) = crate::types::LineJoin::parse(join) {
                        ops.push(Operation::StrokeLinejoin(j));
                    }
                }
                if let Some(da) = &style.stroke_dasharray {
                    let vals: Vec<f64> = da
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|t| !t.is_empty())
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if !vals.is_empty() {
                        ops.push(Operation::StrokeDasharray(vals));
                    }
                }
            }
        }
    }
    if let Some(o) = style.opacity {
        if o < 1.0 {
            ops.push(Operation::Opacity(crate::types::NormalizedAmount(
                o.clamp(0.0, 1.0),
            )));
        }
    }
    ops
}

// ── Color handling ─────────────────────────────────────────────────────

/// Resolve a CSS color string to a Strøk [`Color`], tokenizing repeated colors.
/// Returns `None` when the color is unparseable (already warned during counting).
fn resolve_color(raw: &str, palette: &ResolvedPalette, ctx: &mut ImportCtx) -> Option<Color> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("none") {
        return Some(Color::None);
    }
    if raw.eq_ignore_ascii_case("currentColor") {
        return Some(Color::CurrentColor);
    }
    if let Some(hex) = normalize_color(raw, &ctx.gradients) {
        if let Some(tok) = palette.by_hex.get(&hex) {
            return Some(Color::Token(tok.clone()));
        }
        return Some(Color::Hex(hex));
    }
    None
}

/// Normalize a CSS color / gradient ref to a `#rrggbb`(`aa`) hex string.
fn normalize_color(
    raw: &str,
    gradients: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("transparent") {
        return Some("#00000000".to_string());
    }
    if let Some(rest) = s.strip_prefix("url(") {
        // url(#id) → flat gradient approximation, if we captured one.
        let id = rest.trim_end_matches(')').trim().trim_matches(['"', '\'']);
        let id = id.strip_prefix('#').unwrap_or(id);
        return gradients.get(id).cloned();
    }
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
            3 | 4 => {
                let mut out = String::from("#");
                for c in hex.chars() {
                    out.push(c);
                    out.push(c);
                }
                Some(out.to_lowercase())
            }
            6 | 8 if hex.chars().all(|c| c.is_ascii_hexdigit()) => Some(s.to_lowercase()),
            _ => None,
        };
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split(',')
            .filter_map(|t| t.trim().trim_end_matches('%').parse::<f64>().ok())
            .collect();
        if parts.len() >= 3 {
            let to = |v: f64| -> u8 { v.clamp(0.0, 255.0).round() as u8 };
            return Some(format!(
                "#{:02x}{:02x}{:02x}",
                to(parts[0]),
                to(parts[1]),
                to(parts[2])
            ));
        }
        return None;
    }
    named_color(s)
}

/// A small CSS named-color table (the common ones seen in real assets).
fn named_color(s: &str) -> Option<String> {
    let hex = match s.to_ascii_lowercase().as_str() {
        "black" => "#000000",
        "white" => "#ffffff",
        "red" => "#ff0000",
        "green" => "#008000",
        "lime" => "#00ff00",
        "blue" => "#0000ff",
        "yellow" => "#ffff00",
        "cyan" | "aqua" => "#00ffff",
        "magenta" | "fuchsia" => "#ff00ff",
        "gray" | "grey" => "#808080",
        "silver" => "#c0c0c0",
        "maroon" => "#800000",
        "olive" => "#808000",
        "navy" => "#000080",
        "teal" => "#008080",
        "purple" => "#800080",
        "orange" => "#ffa500",
        "pink" => "#ffc0cb",
        "brown" => "#a52a2a",
        "gold" => "#ffd700",
        "indigo" => "#4b0082",
        "violet" => "#ee82ee",
        _ => return None,
    };
    Some(hex.to_string())
}

/// Count concrete colors across the tree so we can tokenize the ones used ≥ 2×.
fn count_colors(node: &XmlNode, parent: &Style, ctx: &mut ImportCtx) {
    let local = local_name(&node.name);
    let style = Style::inherit(parent, node);
    // Only count colors on drawable leaves (not <defs>/<g> themselves, whose
    // colors are inherited and counted on the leaf).
    if is_drawable(local) {
        for raw in [style.fill.as_deref(), style.stroke.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Some(hex) = normalize_color(raw, &ctx.gradients) {
                if hex.len() == 7 {
                    *ctx.color_counts.entry(hex).or_insert(0) += 1;
                }
            }
        }
    }
    if local == "defs" {
        return; // don't count colors inside defs
    }
    for child in &node.children {
        count_colors(child, &style, ctx);
    }
}

/// Build the palette from colors used ≥ 2×, naming each by hue.
fn build_palette(ctx: &ImportCtx) -> ResolvedPalette {
    let mut pal = ResolvedPalette::default();
    let mut hue_counters: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    // Deterministic order: by descending count then hex.
    let mut colors: Vec<(String, usize)> = ctx
        .color_counts
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (hex, count) in colors {
        if count < 2 {
            continue;
        }
        let base = hue_name(&hex);
        let n = hue_counters.entry(base.clone()).or_insert(0);
        *n += 1;
        let name = format!("{}-{}", base, n);
        pal.by_hex.insert(hex.clone(), name.clone());
        pal.tokens.push((name, hex));
    }
    pal
}

/// Rough hue bucket for a `#rrggbb` color (for palette token naming).
fn hue_name(hex: &str) -> String {
    let (r, g, b) = match hex_rgb(hex) {
        Some(v) => v,
        None => return "color".to_string(),
    };
    let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    let sat = if d < 1e-6 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };
    if sat < 0.12 {
        return if l < 0.2 {
            "black".to_string()
        } else if l > 0.85 {
            "white".to_string()
        } else {
            "gray".to_string()
        };
    }
    let mut h = if (max - rf).abs() < 1e-9 {
        60.0 * (((gf - bf) / d) % 6.0)
    } else if (max - gf).abs() < 1e-9 {
        60.0 * (((bf - rf) / d) + 2.0)
    } else {
        60.0 * (((rf - gf) / d) + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    let name = match h as u32 {
        0..=20 | 340..=360 => "red",
        21..=45 => "orange",
        46..=65 => "yellow",
        66..=160 => "green",
        161..=200 => "cyan",
        201..=255 => "blue",
        256..=290 => "purple",
        _ => "magenta",
    };
    name.to_string()
}

fn hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Pre-scan `<defs>` for gradient definitions and record a flat first-stop
/// approximation per id (the DSL supports gradients, but importing arbitrary
/// gradient stop/transform combos is out of scope — a flat fill keeps the
/// preview close and is honestly warned about at use sites).
fn prescan_gradients(root: &XmlNode, ctx: &mut ImportCtx) {
    fn walk(node: &XmlNode, ctx: &mut ImportCtx) {
        let local = local_name(&node.name);
        if local == "linearGradient" || local == "radialGradient" {
            if let Some(id) = node.attr("id") {
                // first <stop stop-color=…>
                for c in &node.children {
                    if local_name(&c.name) == "stop" {
                        let col = c.attr("stop-color").map(|s| s.to_string()).or_else(|| {
                            c.attr("style").and_then(|st| {
                                st.split(';').find_map(|d| {
                                    d.split_once(':').and_then(|(k, v)| {
                                        (k.trim() == "stop-color").then(|| v.trim().to_string())
                                    })
                                })
                            })
                        });
                        if let Some(col) = col {
                            if let Some(hex) =
                                normalize_color(&col, &std::collections::BTreeMap::new())
                            {
                                ctx.gradients.insert(id.to_string(), hex);
                                break;
                            }
                        }
                    }
                }
            }
        }
        for c in &node.children {
            walk(c, ctx);
        }
    }
    walk(root, ctx);
}

// ── Tree → nodes ───────────────────────────────────────────────────────

fn is_drawable(local: &str) -> bool {
    matches!(
        local,
        "rect" | "circle" | "ellipse" | "line" | "path" | "polygon" | "polyline" | "text"
    )
}

/// Convert the children of a container node into scene nodes.
fn convert_children(
    node: &XmlNode,
    m: Mat,
    style: &Style,
    palette: &ResolvedPalette,
    ctx: &mut ImportCtx,
    out: &mut Vec<SceneNode>,
) {
    for child in &node.children {
        convert_node(child, m, style, palette, ctx, out);
    }
}

fn convert_node(
    node: &XmlNode,
    parent_m: Mat,
    parent_style: &Style,
    palette: &ResolvedPalette,
    ctx: &mut ImportCtx,
    out: &mut Vec<SceneNode>,
) {
    let local = local_name(&node.name);
    let m = match node.attr("transform") {
        Some(t) => parent_m.mul(&parse_transform(t)),
        None => parent_m,
    };
    let style = Style::inherit(parent_style, node);

    match local {
        "g" => {
            let mut children = Vec::new();
            convert_children(node, m, &style, palette, ctx, &mut children);
            if children.is_empty() {
                return;
            }
            ctx.bump("group");
            // Flatten a group that carries no id and just one child (no structure
            // value): inline the child so the output isn't needlessly nested.
            let has_id = node.attr("id").is_some();
            if !has_id && children.len() == 1 {
                out.extend(children);
                return;
            }
            let name = ctx_unique_name(ctx, node, "group");
            let opacity = style.opacity.filter(|o| *o < 1.0);
            out.push(SceneNode::Group(Group {
                name,
                children,
                position: None,
                rotation: None,
                flip: None,
                skew: None,
                clip: None,
                mask: None,
                opacity,
            }));
        }
        "rect" | "circle" | "ellipse" | "line" | "path" | "polygon" | "polyline" => {
            convert_shape(local, node, m, &style, palette, ctx, out);
        }
        "text" => convert_text(node, m, &style, palette, ctx, out),
        "defs" | "symbol" | "metadata" | "title" | "desc" | "style" | "linearGradient"
        | "radialGradient" | "marker" | "pattern" => {
            // Definitions / paint servers — not emitted directly (gradients are
            // captured as flat approximations in the prescan).
        }
        "clipPath" | "mask" | "filter" => {
            ctx.warn(
                format!("<{}> is not supported and was skipped", local),
                node.line,
            );
        }
        "use" | "image" | "foreignObject" | "switch" => {
            ctx.warn(
                format!("<{}> is not supported and was skipped", local),
                node.line,
            );
        }
        "svg" => {
            // nested svg — treat as a group container.
            convert_children(node, m, &style, palette, ctx, out);
        }
        other => {
            if !other.is_empty() {
                ctx.warn(format!("<{}> was skipped (unrecognized)", other), node.line);
            }
        }
    }
}

/// Recover a drawable element into a shape definition (deduped) + a place.
fn convert_shape(
    local: &str,
    node: &XmlNode,
    m: Mat,
    style: &Style,
    palette: &ResolvedPalette,
    ctx: &mut ImportCtx,
    out: &mut Vec<SceneNode>,
) {
    ctx.bump(local);
    let is_line = local == "line";
    let sty = style_ops(style, palette, is_line, ctx);

    // Try the readable, template-preserving path for axis-aligned transforms.
    if m.is_axis_aligned() {
        if let Some((shape_ops, template, place)) =
            axis_aligned_primitive(local, node, m, &sty, ctx)
        {
            emit_shape_place(template, shape_ops, place, node, local, ctx, out);
            return;
        }
    }

    // General path: bake the full transform into geometry.
    match build_path_shape(local, node, m, &sty, ctx) {
        Some((ops, at)) => {
            let place = PlaceSpec {
                at,
                size: None,
                from_to: None,
            };
            emit_shape_place(Template::Path, ops, place, node, local, ctx, out);
        }
        None => {
            ctx.warn(
                format!("<{}> produced no geometry and was skipped", local),
                node.line,
            );
        }
    }
}

/// Placement recovered for a shape.
struct PlaceSpec {
    at: (f64, f64),
    size: Option<(f64, f64)>,
    from_to: Option<((f64, f64), (f64, f64))>,
}

/// Recover rect/circle/ellipse/line as their native template under an
/// axis-aligned transform (translate + scale/flip). Returns shape ops + template
/// + placement, or `None` if the element can't be expressed this way.
fn axis_aligned_primitive(
    local: &str,
    node: &XmlNode,
    m: Mat,
    style_ops: &[Operation],
    ctx: &mut ImportCtx,
) -> Option<(Vec<Operation>, Template, PlaceSpec)> {
    let sx = if m.a < 0.0 { -m.scale_x() } else { m.scale_x() };
    let sy = if m.d < 0.0 { -m.scale_y() } else { m.scale_y() };
    match local {
        "rect" => {
            let x = attr_f(node, "x").unwrap_or(0.0);
            let y = attr_f(node, "y").unwrap_or(0.0);
            let w = attr_f(node, "width")?;
            let h = attr_f(node, "height")?;
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let (px, py) = m.apply(x, y);
            let pw = w * sx;
            let ph = h * sy;
            // Normalize a flipped extent so at is the top-left, size positive.
            let (px, pw) = if pw < 0.0 { (px + pw, -pw) } else { (px, pw) };
            let (py, ph) = if ph < 0.0 { (py + ph, -ph) } else { (py, ph) };
            let mut ops = Vec::new();
            let rx = attr_f(node, "rx").or_else(|| attr_f(node, "ry"));
            if let Some(rx) = rx {
                if rx > 0.0 {
                    ops.push(Operation::RoundCorners {
                        radii: CornerRadii::Uniform(rx * sx.abs()),
                    });
                }
            }
            ops.extend(style_ops.iter().cloned());
            Some((
                ops,
                Template::Rectangle,
                PlaceSpec {
                    at: (px, py),
                    size: Some((pw, ph)),
                    from_to: None,
                },
            ))
        }
        "circle" | "ellipse" => {
            let cx = attr_f(node, "cx").unwrap_or(0.0);
            let cy = attr_f(node, "cy").unwrap_or(0.0);
            let (rx, ry) = if local == "circle" {
                let r = attr_f(node, "r")?;
                (r, r)
            } else {
                (attr_f(node, "rx")?, attr_f(node, "ry")?)
            };
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            let (pcx, pcy) = m.apply(cx, cy);
            let pw = (2.0 * rx * sx).abs();
            let ph = (2.0 * ry * sy).abs();
            let mut ops = Vec::new();
            ops.extend(style_ops.iter().cloned());
            let _ = ctx;
            Some((
                ops,
                Template::Ellipse,
                PlaceSpec {
                    at: (pcx - pw / 2.0, pcy - ph / 2.0),
                    size: Some((pw, ph)),
                    from_to: None,
                },
            ))
        }
        "line" => {
            let x1 = attr_f(node, "x1").unwrap_or(0.0);
            let y1 = attr_f(node, "y1").unwrap_or(0.0);
            let x2 = attr_f(node, "x2").unwrap_or(0.0);
            let y2 = attr_f(node, "y2").unwrap_or(0.0);
            let p1 = m.apply(x1, y1);
            let p2 = m.apply(x2, y2);
            let mut ops = Vec::new();
            ops.extend(style_ops.iter().cloned());
            let _ = ctx;
            Some((
                ops,
                Template::Line,
                PlaceSpec {
                    at: p1,
                    size: None,
                    from_to: Some((p1, p2)),
                },
            ))
        }
        _ => None,
    }
}

/// Build a `path` shape by baking `m` into geometry. Points are authored in
/// document space, offset so the geometry min is at the returned `at` (a
/// translate-only placement) — this keeps identical geometry at different
/// positions collapsible to one shape by the reuse detector.
fn build_path_shape(
    local: &str,
    node: &XmlNode,
    m: Mat,
    style_ops: &[Operation],
    ctx: &mut ImportCtx,
) -> Option<(Vec<Operation>, (f64, f64))> {
    let mut sub = SubBuilder::default();
    match local {
        "rect" => {
            let x = attr_f(node, "x").unwrap_or(0.0);
            let y = attr_f(node, "y").unwrap_or(0.0);
            let w = attr_f(node, "width")?;
            let h = attr_f(node, "height")?;
            sub.moveto((x, y));
            sub.lineto((x + w, y));
            sub.lineto((x + w, y + h));
            sub.lineto((x, y + h));
            sub.close();
        }
        "polygon" | "polyline" => {
            let pts = parse_points(node.attr("points").unwrap_or(""));
            if pts.is_empty() {
                return None;
            }
            for (i, p) in pts.iter().enumerate() {
                if i == 0 {
                    sub.moveto(*p);
                } else {
                    sub.lineto(*p);
                }
            }
            if local == "polygon" {
                sub.close();
            }
        }
        "circle" | "ellipse" => {
            // Under rotation/shear, approximate with 4 cubic quadrant curves.
            let cx = attr_f(node, "cx").unwrap_or(0.0);
            let cy = attr_f(node, "cy").unwrap_or(0.0);
            let (rx, ry) = if local == "circle" {
                let r = attr_f(node, "r")?;
                (r, r)
            } else {
                (attr_f(node, "rx")?, attr_f(node, "ry")?)
            };
            ellipse_to_cubics(&mut sub, cx, cy, rx, ry);
        }
        "line" => {
            let x1 = attr_f(node, "x1").unwrap_or(0.0);
            let y1 = attr_f(node, "y1").unwrap_or(0.0);
            let x2 = attr_f(node, "x2").unwrap_or(0.0);
            let y2 = attr_f(node, "y2").unwrap_or(0.0);
            sub.moveto((x1, y1));
            sub.lineto((x2, y2));
        }
        "path" => {
            let d = node.attr("d").unwrap_or("");
            if !parse_path_d(d, &mut sub, m.is_axis_aligned()) {
                ctx.warn(
                    "path had unrepresentable segments; flattened best-effort".to_string(),
                    node.line,
                );
            }
        }
        _ => return None,
    }
    if sub.segments.is_empty() {
        return None;
    }
    if !m.is_axis_aligned() {
        ctx.warn(
            format!(
                "<{}> has a rotation/shear transform — baked into path geometry",
                local
            ),
            node.line,
        );
    }
    sub.into_ops(m, style_ops)
}

// ── Sub-path builder ───────────────────────────────────────────────────

#[derive(Clone)]
enum Seg {
    Move((f64, f64)),
    Line((f64, f64)),
    Cubic {
        c1: (f64, f64),
        c2: (f64, f64),
        to: (f64, f64),
    },
    Arc {
        rx: f64,
        ry: f64,
        large: bool,
        sweep: bool,
        to: (f64, f64),
    },
    Close,
}

#[derive(Default)]
struct SubBuilder {
    segments: Vec<Seg>,
}

impl SubBuilder {
    fn moveto(&mut self, p: (f64, f64)) {
        self.segments.push(Seg::Move(p));
    }
    fn lineto(&mut self, p: (f64, f64)) {
        self.segments.push(Seg::Line(p));
    }
    fn cubic(&mut self, c1: (f64, f64), c2: (f64, f64), to: (f64, f64)) {
        self.segments.push(Seg::Cubic { c1, c2, to });
    }
    fn arc(&mut self, rx: f64, ry: f64, large: bool, sweep: bool, to: (f64, f64)) {
        self.segments.push(Seg::Arc {
            rx,
            ry,
            large,
            sweep,
            to,
        });
    }
    fn close(&mut self) {
        self.segments.push(Seg::Close);
    }

    /// Lower the accumulated segments into `addpoint`/`subpath`/`close` ops,
    /// transformed by `m` and offset so geometry min → returned `at`.
    fn into_ops(self, m: Mat, style_ops: &[Operation]) -> Option<(Vec<Operation>, (f64, f64))> {
        let tx = |p: (f64, f64)| m.apply(p.0, p.1);
        let mut all: Vec<(f64, f64)> = Vec::new();
        for s in &self.segments {
            match s {
                Seg::Move(p) | Seg::Line(p) => all.push(tx(*p)),
                Seg::Cubic { c1, c2, to } => {
                    all.push(tx(*c1));
                    all.push(tx(*c2));
                    all.push(tx(*to));
                }
                Seg::Arc { to, .. } => all.push(tx(*to)),
                Seg::Close => {}
            }
        }
        if all.is_empty() {
            return None;
        }
        let minx = all.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let miny = all.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let off = |p: (f64, f64)| {
            let t = tx(p);
            (t.0 - minx, t.1 - miny)
        };

        let mut ops: Vec<Operation> = Vec::new();
        let mut idx = 0usize;
        let mut started = false;
        let mut closed_any = false;
        for s in &self.segments {
            match s {
                Seg::Move(p) => {
                    if started {
                        ops.push(Operation::Subpath);
                    }
                    started = true;
                    ops.push(addpoint(&format!("p{}", idx), off(*p), None, None, None));
                    idx += 1;
                }
                Seg::Line(p) => {
                    ops.push(addpoint(&format!("p{}", idx), off(*p), None, None, None));
                    idx += 1;
                }
                Seg::Cubic { c1, c2, to } => {
                    ops.push(addpoint(
                        &format!("p{}", idx),
                        off(*to),
                        Some(PointMode::Controls),
                        Some(off(*c1)),
                        Some(off(*c2)),
                    ));
                    idx += 1;
                }
                Seg::Arc {
                    rx,
                    ry,
                    large,
                    sweep,
                    to,
                } => {
                    // Arc only reached under axis-aligned transforms; scale radii.
                    let (rxs, rys) = (rx * m.scale_x(), ry * m.scale_y());
                    ops.push(arc_point(
                        &format!("p{}", idx),
                        off(*to),
                        rxs,
                        rys,
                        *large,
                        *sweep,
                    ));
                    idx += 1;
                }
                Seg::Close => {
                    closed_any = true;
                }
            }
        }
        if closed_any {
            ops.push(Operation::Close);
        }
        ops.extend(style_ops.iter().cloned());
        Some((ops, (minx, miny)))
    }
}

fn addpoint(
    name: &str,
    at: (f64, f64),
    mode: Option<PointMode>,
    c1: Option<(f64, f64)>,
    c2: Option<(f64, f64)>,
) -> Operation {
    Operation::AddPoint {
        name: name.to_string(),
        at,
        after: None,
        mode,
        tension: None,
        arc_rx: None,
        arc_ry: None,
        arc_sweep: None,
        arc_large: None,
        arc_bulge: None,
        control_c1: c1,
        control_c2: c2,
    }
}

fn arc_point(name: &str, at: (f64, f64), rx: f64, ry: f64, large: bool, sweep: bool) -> Operation {
    Operation::AddPoint {
        name: name.to_string(),
        at,
        after: None,
        mode: Some(PointMode::Arc),
        tension: None,
        arc_rx: Some(rx),
        arc_ry: Some(ry),
        arc_sweep: Some(sweep),
        arc_large: Some(large),
        arc_bulge: None,
        control_c1: None,
        control_c2: None,
    }
}

/// Approximate an ellipse with 4 cubic quadrants (used when baking a rotated
/// ellipse into a path).
fn ellipse_to_cubics(sub: &mut SubBuilder, cx: f64, cy: f64, rx: f64, ry: f64) {
    const K: f64 = 0.5522847498307936; // 4/3 * (sqrt(2) - 1)
    let (kx, ky) = (rx * K, ry * K);
    sub.moveto((cx + rx, cy));
    sub.cubic((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry));
    sub.cubic((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy));
    sub.cubic((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry));
    sub.cubic((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy));
    sub.close();
}

// ── Path `d` parsing ───────────────────────────────────────────────────

/// Parse an SVG path `d` string into segments. Returns `false` if any segment
/// was unrepresentable and required a fallback. `allow_arcs` keeps `A` as arc
/// segments (axis-aligned transforms); otherwise arcs are flattened to cubics.
fn parse_path_d(d: &str, sub: &mut SubBuilder, allow_arcs: bool) -> bool {
    let mut clean = true;
    let mut cur = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let mut last_ctrl: Option<(f64, f64)> = None;
    let mut prev_cmd = ' ';
    let toks = tokenize_path(d);
    let mut i = 0;
    while i < toks.len() {
        let cmd = match &toks[i] {
            PathTok::Cmd(c) => {
                i += 1;
                *c
            }
            PathTok::Num(_) => prev_cmd, // implicit repeat of previous command
        };
        prev_cmd = cmd;
        let rel = cmd.is_ascii_lowercase();
        let up = cmd.to_ascii_uppercase();
        let num = |i: &mut usize| -> Option<f64> {
            if *i < toks.len() {
                if let PathTok::Num(n) = toks[*i] {
                    *i += 1;
                    return Some(n);
                }
            }
            None
        };
        let base = if rel { cur } else { (0.0, 0.0) };
        match up {
            'M' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else {
                    break;
                };
                cur = (base.0 + x, base.1 + y);
                start = cur;
                sub.moveto(cur);
                last_ctrl = None;
                prev_cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else {
                    break;
                };
                cur = (base.0 + x, base.1 + y);
                sub.lineto(cur);
                last_ctrl = None;
            }
            'H' => {
                let Some(x) = num(&mut i) else { break };
                cur = (if rel { cur.0 + x } else { x }, cur.1);
                sub.lineto(cur);
                last_ctrl = None;
            }
            'V' => {
                let Some(y) = num(&mut i) else { break };
                cur = (cur.0, if rel { cur.1 + y } else { y });
                sub.lineto(cur);
                last_ctrl = None;
            }
            'C' => {
                let vals = [
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                ];
                if vals.iter().any(|v| v.is_none()) {
                    break;
                }
                let v: Vec<f64> = vals.into_iter().flatten().collect();
                let c1 = (base.0 + v[0], base.1 + v[1]);
                let c2 = (base.0 + v[2], base.1 + v[3]);
                let to = (base.0 + v[4], base.1 + v[5]);
                sub.cubic(c1, c2, to);
                last_ctrl = Some(c2);
                cur = to;
            }
            'S' => {
                let vals = [num(&mut i), num(&mut i), num(&mut i), num(&mut i)];
                if vals.iter().any(|v| v.is_none()) {
                    break;
                }
                let v: Vec<f64> = vals.into_iter().flatten().collect();
                let c2 = (base.0 + v[0], base.1 + v[1]);
                let to = (base.0 + v[2], base.1 + v[3]);
                let c1 = match last_ctrl {
                    Some(lc) => (2.0 * cur.0 - lc.0, 2.0 * cur.1 - lc.1),
                    _ => cur,
                };
                sub.cubic(c1, c2, to);
                last_ctrl = Some(c2);
                cur = to;
            }
            'Q' => {
                let vals = [num(&mut i), num(&mut i), num(&mut i), num(&mut i)];
                if vals.iter().any(|v| v.is_none()) {
                    break;
                }
                let v: Vec<f64> = vals.into_iter().flatten().collect();
                let qc = (base.0 + v[0], base.1 + v[1]);
                let to = (base.0 + v[2], base.1 + v[3]);
                let (c1, c2) = quad_to_cubic(cur, qc, to);
                sub.cubic(c1, c2, to);
                last_ctrl = Some(qc);
                cur = to;
            }
            'T' => {
                let vals = [num(&mut i), num(&mut i)];
                if vals.iter().any(|v| v.is_none()) {
                    break;
                }
                let v: Vec<f64> = vals.into_iter().flatten().collect();
                let to = (base.0 + v[0], base.1 + v[1]);
                let qc = match last_ctrl {
                    Some(lc) => (2.0 * cur.0 - lc.0, 2.0 * cur.1 - lc.1),
                    None => cur,
                };
                let (c1, c2) = quad_to_cubic(cur, qc, to);
                sub.cubic(c1, c2, to);
                last_ctrl = Some(qc);
                cur = to;
            }
            'A' => {
                let vals = [
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                    num(&mut i),
                ];
                if vals.iter().any(|v| v.is_none()) {
                    break;
                }
                let v: Vec<f64> = vals.into_iter().flatten().collect();
                let (rx, ry, xrot, large, sweep) = (v[0], v[1], v[2], v[3] != 0.0, v[4] != 0.0);
                let to = (base.0 + v[5], base.1 + v[6]);
                if allow_arcs && xrot.abs() < 1e-6 && rx > 0.0 && ry > 0.0 {
                    sub.arc(rx, ry, large, sweep, to);
                } else {
                    for (c1, c2, e) in arc_to_cubic_list(cur, rx, ry, xrot, large, sweep, to) {
                        sub.cubic(c1, c2, e);
                    }
                    if xrot.abs() >= 1e-6 {
                        clean = false;
                    }
                }
                last_ctrl = None;
                cur = to;
            }
            'Z' => {
                sub.close();
                cur = start;
                last_ctrl = None;
            }
            _ => {
                clean = false;
                i += 1;
            }
        }
    }
    clean
}

enum PathTok {
    Cmd(char),
    Num(f64),
}

fn tokenize_path(d: &str) -> Vec<PathTok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() {
            out.push(PathTok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let startn = i;
            let mut seen_dot = false;
            let mut seen_e = false;
            if b[i] == b'-' || b[i] == b'+' {
                i += 1;
            }
            while i < b.len() {
                let ch = b[i];
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == b'.' && !seen_dot && !seen_e {
                    seen_dot = true;
                    i += 1;
                } else if (ch == b'e' || ch == b'E') && !seen_e {
                    seen_e = true;
                    i += 1;
                    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(n) = d[startn..i].parse::<f64>() {
                out.push(PathTok::Num(n));
            }
        } else {
            i += 1; // whitespace / comma
        }
    }
    out
}

fn quad_to_cubic(p0: (f64, f64), qc: (f64, f64), p1: (f64, f64)) -> ((f64, f64), (f64, f64)) {
    let c1 = (
        p0.0 + 2.0 / 3.0 * (qc.0 - p0.0),
        p0.1 + 2.0 / 3.0 * (qc.1 - p0.1),
    );
    let c2 = (
        p1.0 + 2.0 / 3.0 * (qc.0 - p1.0),
        p1.1 + 2.0 / 3.0 * (qc.1 - p1.1),
    );
    (c1, c2)
}

/// Convert an SVG endpoint-parameterized elliptical arc into a list of cubic
/// bezier control triples. Handles x-axis rotation (unlike the DSL arc op).
#[allow(clippy::type_complexity)]
fn arc_to_cubic_list(
    p0: (f64, f64),
    mut rx: f64,
    mut ry: f64,
    xrot_deg: f64,
    large: bool,
    sweep: bool,
    p1: (f64, f64),
) -> Vec<((f64, f64), (f64, f64), (f64, f64))> {
    let mut out = Vec::new();
    if rx == 0.0 || ry == 0.0 {
        out.push((p0, p1, p1));
        return out;
    }
    rx = rx.abs();
    ry = ry.abs();
    let phi = xrot_deg.to_radians();
    let (sin_phi, cos_phi) = (phi.sin(), phi.cos());
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let sign = if large != sweep { 1.0 } else { -1.0 };
    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let co = if den == 0.0 {
        0.0
    } else {
        sign * (num / den).sqrt()
    };
    let cxp = co * rx * y1p / ry;
    let cyp = -co * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (p0.1 + p1.1) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = if len == 0.0 {
            0.0
        } else {
            (dot / len).clamp(-1.0, 1.0).acos()
        };
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dtheta = ang(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * std::f64::consts::PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * std::f64::consts::PI;
    }
    let segs = (dtheta.abs() / (std::f64::consts::PI / 2.0))
        .ceil()
        .max(1.0) as usize;
    let delta = dtheta / segs as f64;
    let t = 4.0 / 3.0 * (delta / 4.0).tan();
    let mut th = theta1;
    let point = |a: f64| -> (f64, f64) {
        let x = rx * a.cos();
        let y = ry * a.sin();
        (
            cos_phi * x - sin_phi * y + cx,
            sin_phi * x + cos_phi * y + cy,
        )
    };
    let deriv = |a: f64| -> (f64, f64) {
        let x = -rx * a.sin();
        let y = ry * a.cos();
        (cos_phi * x - sin_phi * y, sin_phi * x + cos_phi * y)
    };
    for _ in 0..segs {
        let th2 = th + delta;
        let p_start = point(th);
        let p_end = point(th2);
        let d1 = deriv(th);
        let d2 = deriv(th2);
        let c1 = (p_start.0 + t * d1.0, p_start.1 + t * d1.1);
        let c2 = (p_end.0 - t * d2.0, p_end.1 - t * d2.1);
        out.push((c1, c2, p_end));
        th = th2;
    }
    out
}

fn parse_points(s: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

// ── Text ───────────────────────────────────────────────────────────────

fn convert_text(
    node: &XmlNode,
    m: Mat,
    style: &Style,
    palette: &ResolvedPalette,
    ctx: &mut ImportCtx,
    out: &mut Vec<SceneNode>,
) {
    let content = collect_text(node);
    if content.trim().is_empty() {
        return;
    }
    ctx.bump("text");
    let x = attr_f(node, "x").unwrap_or(0.0);
    let y = attr_f(node, "y").unwrap_or(0.0);
    let (px, py) = m.apply(x, y);

    let mut ops = vec![Operation::Content(content)];
    let fs = style.font_size.unwrap_or(16.0) * m.scale_y();
    ops.push(Operation::FontSize(fs));
    if let Some(ff) = &style.font_family {
        ops.push(Operation::FontFamily(ff.clone()));
    }
    if let Some(fw) = &style.font_weight {
        ops.push(Operation::FontWeight(fw.clone()));
    }
    if let Some(ta) = &style.text_anchor {
        if let Ok(a) = crate::types::TextAnchor::parse(ta) {
            ops.push(Operation::TextAnchor(a));
        }
    }
    // Text fill defaults to black.
    let fill = style.fill.clone().unwrap_or_else(|| "#000000".to_string());
    if let Some(c) = resolve_color(&fill, palette, ctx) {
        ops.push(Operation::Fill(c));
    }
    if !m.is_axis_aligned() {
        ctx.warn(
            "text has a rotation/shear transform — position approximated".to_string(),
            node.line,
        );
    }
    ctx.warn(
        "text metrics are estimated (embedded Helvetica); bbox is layout-grade".to_string(),
        node.line,
    );

    emit_shape_place(
        Template::Text,
        ops,
        PlaceSpec {
            at: (px, py),
            size: None,
            from_to: None,
        },
        node,
        "text",
        ctx,
        out,
    );
}

fn collect_text(node: &XmlNode) -> String {
    let mut s = node.text.clone();
    for c in &node.children {
        if matches!(local_name(&c.name), "tspan" | "textPath") {
            s.push_str(&collect_text(c));
        }
    }
    s.trim().to_string()
}

// ── Shape/place emission + reuse ───────────────────────────────────────

/// Emit a deduplicated shape definition and a place referencing it.
fn emit_shape_place(
    template: Template,
    ops: Vec<Operation>,
    place: PlaceSpec,
    node: &XmlNode,
    kind: &str,
    ctx: &mut ImportCtx,
    out: &mut Vec<SceneNode>,
) {
    let shape = Shape {
        name: String::new(),
        template,
        operations: ops,
        effects: Vec::new(),
    };
    let key = shape_key(&shape);
    let shape_name = if let Some(existing) = ctx.shape_keys.get(&key) {
        existing.clone()
    } else {
        let base = ctx_shape_name(ctx, node, kind);
        let mut named = shape.clone();
        named.name = base.clone();
        ctx.shapes.push(named);
        ctx.shape_keys.insert(key, base.clone());
        base
    };

    let place_name = ctx_unique_name(ctx, node, kind);
    let mut p = Place {
        name: place_name,
        shape_ref: shape_name,
        position: PlacePosition::At(place.at.0, place.at.1),
        size: place.size.map(|(w, h)| Dimension { w, h }),
        rotation: None,
        flip: None,
        skew: None,
        clip: None,
        mask: None,
        anchor: None,
        overrides: Vec::new(),
        align: None,
        offset: None,
        text_path: None,
    };
    // A line uses from/to sugar → stored as At + (possibly negative) size.
    if let Some((a, b)) = place.from_to {
        p.position = PlacePosition::At(a.0, a.1);
        p.size = Some(Dimension {
            w: b.0 - a.0,
            h: b.1 - a.1,
        });
    }
    out.push(SceneNode::Place(p));
}

/// A canonical structural key for reuse detection: template + operations, with
/// point names normalized (only positions/modes/style matter for identity).
fn shape_key(shape: &Shape) -> String {
    let tmp = Shape {
        name: "_".to_string(),
        template: shape.template.clone(),
        operations: shape.operations.clone(),
        effects: Vec::new(),
    };
    crate::dsl_emit::emit_shape_block(&tmp)
}

// ── Name helpers ───────────────────────────────────────────────────────

fn ctx_shape_name(ctx: &mut ImportCtx, node: &XmlNode, kind: &str) -> String {
    let base = sanitized_id(node).unwrap_or_else(|| next_kind(ctx, kind));
    let base = format!("{}-shape", base);
    unique(ctx, base)
}

fn ctx_unique_name(ctx: &mut ImportCtx, node: &XmlNode, kind: &str) -> String {
    let base = sanitized_id(node).unwrap_or_else(|| next_kind(ctx, kind));
    unique(ctx, base)
}

fn next_kind(ctx: &mut ImportCtx, kind: &str) -> String {
    let n = ctx.kind_counters.entry(kind.to_string()).or_insert(0);
    *n += 1;
    format!("{}-{}", kind, n)
}

fn unique(ctx: &mut ImportCtx, base: String) -> String {
    if ctx.used_names.insert(base.clone()) {
        return base;
    }
    for i in 1..usize::MAX {
        let cand = format!("{}-{}", base, i);
        if ctx.used_names.insert(cand.clone()) {
            return cand;
        }
    }
    base
}

/// Derive a valid Strøk identifier from an element's `id` or `class`.
fn sanitized_id(node: &XmlNode) -> Option<String> {
    let raw = node.attr("id").or_else(|| {
        node.attr("class")
            .map(|c| c.split_whitespace().next().unwrap_or(""))
    })?;
    let s = sanitize_ident(raw);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Coerce an arbitrary string to `[a-z][a-z0-9-]*` (Strøk ident rules).
fn sanitize_ident(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() {
            out.push(lc);
        } else if lc == '-' || lc == '_' || lc == ' ' {
            out.push('-');
        }
    }
    // strip leading non-letters
    while out
        .chars()
        .next()
        .map(|c| !c.is_ascii_lowercase())
        .unwrap_or(false)
    {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    // collapse repeated dashes
    let mut collapsed = String::new();
    let mut prev_dash = false;
    for c in out.chars() {
        if c == '-' {
            if !prev_dash {
                collapsed.push(c);
            }
            prev_dash = true;
        } else {
            collapsed.push(c);
            prev_dash = false;
        }
    }
    collapsed
}

// ── Document size ──────────────────────────────────────────────────────

fn document_size(root: &XmlNode, warnings: &mut Vec<ImportWarning>) -> (Dimension, (f64, f64)) {
    let vb = root.attr("viewBox").and_then(|s| {
        let n: Vec<f64> = s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        (n.len() == 4).then(|| (n[0], n[1], n[2], n[3]))
    });
    let w = root.attr("width").and_then(parse_len_opt);
    let h = root.attr("height").and_then(parse_len_opt);
    if let Some((minx, miny, vw, vh)) = vb {
        if vw > 0.0 && vh > 0.0 {
            return (Dimension { w: vw, h: vh }, (-minx, -miny));
        }
    }
    match (w, h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => (Dimension { w, h }, (0.0, 0.0)),
        _ => {
            warnings.push(ImportWarning::new(
                "no viewBox/width/height on <svg> — defaulting to 100x100",
                root.line,
            ));
            (Dimension { w: 100.0, h: 100.0 }, (0.0, 0.0))
        }
    }
}

fn parse_len(s: &str) -> Option<f64> {
    parse_len_opt(s)
}

/// Parse a length, tolerating a `px` suffix and ignoring other units.
fn parse_len_opt(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t);
    t.parse::<f64>().ok()
}

fn attr_f(node: &XmlNode, name: &str) -> Option<f64> {
    node.attr(name).and_then(parse_len_opt)
}

#[cfg(test)]
#[path = "import_svg/tests.rs"]
mod tests;
