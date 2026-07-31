/// v3 line-oriented parser for the .strok DSL.
///
/// The file is the construction history. Each line is a command.
/// Indentation determines nesting (operations inside shape blocks,
/// children inside group blocks).
use crate::error::{Result, StrokError};
use crate::scene::*;
use crate::shape::*;
use crate::types::*;

use crate::diagnostics::{suggest, Diagnostic};
use crate::expr::{eval_scalar, Env};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

mod place;

use place::parse_place_line;

/// Valid top-level keywords (for "did you mean" suggestions, E3.1).
const TOP_LEVEL_KEYWORDS: &[&str] = &[
    "documentsize",
    "use",
    "palette",
    "scheme",
    "defaults",
    "let",
    "shape",
    "place",
    "group",
    "boolean",
    "createlink",
    "unlink",
    "repeat",
    "reorder",
    "tokens",
    "frame",
    "component",
    "instance",
];

/// Valid operations inside a `shape` block (for suggestions, E3.1).
const SHAPE_OPS: &[&str] = &[
    "movepoint",
    "pullpoint",
    "sculpt",
    "addpoint",
    "splitline",
    "deletepoint",
    "close",
    "open",
    "subpath",
    "smooth",
    "smooth-corner",
    "sharpen",
    "convert-point",
    "round-corners",
    "notch",
    "fill",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "opacity",
    "blur",
    "content",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "text-anchor",
    "applyeffect",
];

/// Parse a .strok v3 file into a Scene.
pub fn parse_file(input: &str) -> Result<Scene> {
    let lines = tokenize_lines(input);
    let mut parser = LineParser::new(lines);
    parser.parse_scene()
}

/// Parse with error recovery (E3.1): one malformed top-level block no longer
/// aborts the whole parse. Returns the best-effort [`Scene`] plus every
/// [`Diagnostic`] collected. On a fully clean file the diagnostic list is empty.
///
/// This feeds the GUI/MCP loop (show all problems at once, still get a partial
/// scene to render). `parse_file` keeps its fail-fast contract for callers that
/// want the first error as a single `Result`.
pub fn parse_file_recover(input: &str) -> (Scene, Vec<Diagnostic>) {
    let lines = tokenize_lines(input);
    let mut parser = LineParser::new(lines);
    parser.parse_scene_recover()
}

/// Parse a .strok file with a known file path, enabling `use` imports.
pub fn parse_file_with_path(input: &str, file_path: &Path) -> Result<Scene> {
    let lines = tokenize_lines(input);
    let mut parser = LineParser::new(lines);
    let mut scene = parser.parse_scene()?;

    // Resolve imports
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    let mut visited = HashSet::new();
    visited.insert(canonical.clone());
    resolve_imports(&mut scene, file_path, &mut visited)?;

    Ok(scene)
}

/// Resolve all `use` imports, merging imported shapes into the scene.
fn resolve_imports(
    scene: &mut Scene,
    current_file: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<()> {
    let base_dir = current_file.parent().unwrap_or(Path::new("."));

    for import in &scene.imports {
        // Embedded standard library (EXP-1): `use "std/<module>"` is
        // intercepted *before* any filesystem resolution. This must come
        // first so a std import never depends on cwd/base_dir and works from
        // any document, with no files on disk.
        if let Some(module_name) = crate::stdlib::strip_std_prefix(&import.path) {
            let Some(source) = crate::stdlib::get(module_name) else {
                return Err(StrokError::ParseError(format!(
                    "unknown standard library module 'std/{}' — available modules: {}",
                    module_name,
                    crate::stdlib::available_names()
                )));
            };

            // Std modules are self-contained (no further imports), but reuse
            // the same recursive resolver for consistency/future-proofing.
            let mut imported_scene = parse_file(source)?;
            let marker = PathBuf::from(format!("std::{}", module_name));
            resolve_imports(&mut imported_scene, &marker, visited)?;

            for mut shape in imported_scene.shapes {
                if let Some(ns) = &import.namespace {
                    shape.name = format!("{}.{}", ns, shape.name);
                }
                if !scene.shapes.iter().any(|s| s.name == shape.name) {
                    scene.imported_shape_names.insert(shape.name.clone());
                    scene.shapes.push(shape);
                }
            }
            continue;
        }

        let import_path = base_dir.join(&import.path);
        let canonical = import_path.canonicalize().map_err(|e| {
            StrokError::ParseError(format!("cannot resolve import '{}': {}", import.path, e))
        })?;

        if !visited.insert(canonical.clone()) {
            return Err(StrokError::ParseError(format!(
                "circular import detected: '{}'",
                import.path
            )));
        }

        let content = std::fs::read_to_string(&canonical).map_err(|e| {
            StrokError::ParseError(format!("cannot read import '{}': {}", import.path, e))
        })?;

        let mut imported_scene = parse_file(&content)?;
        resolve_imports(&mut imported_scene, &canonical, visited)?;

        // Merge shapes (not nodes) from imported scene
        for mut shape in imported_scene.shapes {
            if let Some(ns) = &import.namespace {
                shape.name = format!("{}.{}", ns, shape.name);
            }
            // Only add if not already defined
            if !scene.shapes.iter().any(|s| s.name == shape.name) {
                scene.imported_shape_names.insert(shape.name.clone());
                scene.shapes.push(shape);
            }
        }
    }

    Ok(())
}

// ── Line tokenization ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Line {
    indent: usize,
    tokens: Vec<String>,
    line_num: usize,
    /// The raw source line (trailing whitespace trimmed, comments retained as
    /// written) for caret-snippet diagnostics (E3.1).
    raw: String,
}

impl Line {
    /// 1-based column where `token_idx` begins in the raw line, for caret
    /// snippets. Falls back to `indent + 1` if the token can't be located.
    fn token_column(&self, token_idx: usize) -> usize {
        match self.tokens.get(token_idx) {
            Some(tok) => {
                // Find the token's stripped form in the raw line. Tokens may have
                // surrounding quotes preserved, so search the trimmed token too.
                let needle = tok.trim_matches('"');
                self.raw
                    .find(needle)
                    .map(|b| self.raw[..b].chars().count() + 1)
                    .unwrap_or(self.indent + 1)
            }
            None => self.indent + 1,
        }
    }

    /// Build a [`Diagnostic`] anchored at `token_idx` with this line's source.
    fn diag(&self, token_idx: usize, message: impl Into<String>) -> crate::diagnostics::Diagnostic {
        let col = self.token_column(token_idx);
        let width = self
            .tokens
            .get(token_idx)
            .map(|t| t.trim_matches('"').chars().count().max(1))
            .unwrap_or(1);
        crate::diagnostics::Diagnostic::new(self.line_num, message)
            .with_span(col, width)
            .with_source(self.raw.clone())
    }
}

fn tokenize_lines(input: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for (i, raw) in input.lines().enumerate() {
        let line_num = i + 1;

        // Strip comments (# at start of meaningful content).
        let content = if let Some(hash_pos) = find_comment_start(raw) {
            &raw[..hash_pos]
        } else {
            raw
        };

        let trimmed = content.trim_end();
        if trimmed.is_empty() {
            continue;
        }

        let indent = trimmed.len() - trimmed.trim_start().len();
        let tokens = tokenize_line(trimmed.trim_start());
        if tokens.is_empty() {
            continue;
        }

        lines.push(Line {
            indent,
            tokens,
            line_num,
            raw: trimmed.to_string(),
        });
    }
    lines
}

/// Find the position of a comment start (#) that's not inside a hex color.
fn find_comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            // Check if this is a hex color: # followed by hex digits
            let rest = &line[i + 1..];
            let hex_len = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
            if hex_len == 6 || hex_len == 8 {
                // This is a color literal, skip it
                i += 1 + hex_len;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Tokenize a single line into whitespace-separated tokens,
/// respecting `key=value` as a single token, `x,y` as a single token,
/// and `"quoted strings"` as a single token (with quotes preserved).
fn tokenize_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current = String::new();

    while let Some(&ch) = chars.peek() {
        if ch == '"' {
            // Start of quoted string — collect everything until closing quote
            current.push(ch);
            chars.next();
            while let Some(&qch) = chars.peek() {
                current.push(qch);
                chars.next();
                if qch == '"' {
                    break;
                }
            }
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            chars.next();
        } else {
            current.push(ch);
            chars.next();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

// ── Line parser ───────────────────────────────────────────────────────

struct LineParser {
    lines: Vec<Line>,
    pos: usize,
    /// Expression environment built from `let` bindings (C13). Scalar
    /// expressions on scene-node / shape-op lines resolve `$name` against this.
    env: Env,
}

impl LineParser {
    fn new(lines: Vec<Line>) -> Self {
        Self {
            lines,
            pos: 0,
            env: Env::new(),
        }
    }

    fn peek(&self) -> Option<&Line> {
        self.lines.get(self.pos)
    }

    fn advance(&mut self) -> &Line {
        let line = &self.lines[self.pos];
        self.pos += 1;
        line
    }

    /// Collect all subsequent lines with indent > base_indent.
    fn collect_body(&mut self, base_indent: usize) -> Vec<Line> {
        let mut body = Vec::new();
        while let Some(line) = self.peek() {
            if line.indent <= base_indent {
                break;
            }
            body.push(self.advance().clone());
        }
        body
    }

    /// Fail-fast parse: returns the first error as a single `Result`. Shares the
    /// per-block logic with the recovering parser via [`parse_one_top_level`].
    fn parse_scene(&mut self) -> Result<Scene> {
        let mut scene = Scene::new(Dimension { w: 100.0, h: 100.0 });

        while let Some(line) = self.peek() {
            let keyword = line.tokens[0].clone();
            let base_indent = line.indent;
            self.parse_one_top_level(&keyword, base_indent, &mut scene)?;
        }

        Ok(scene)
    }

    /// Error-recovering variant of [`parse_scene`]. Each top-level block is
    /// parsed independently; a failing block records a [`Diagnostic`] and the
    /// parser skips its body, then continues. Returns the partial scene + every
    /// diagnostic.
    fn parse_scene_recover(&mut self) -> (Scene, Vec<Diagnostic>) {
        let mut scene = Scene::new(Dimension { w: 100.0, h: 100.0 });
        let mut diags: Vec<Diagnostic> = Vec::new();

        while let Some(line) = self.peek() {
            let keyword = line.tokens[0].clone();
            let base_indent = line.indent;

            // Snapshot position so a failed block can be skipped wholesale.
            let start_pos = self.pos;
            let result = self.parse_one_top_level(&keyword, base_indent, &mut scene);
            match result {
                Ok(()) => {}
                Err(StrokError::ParseDiagnostics(mut ds)) => {
                    diags.append(&mut ds);
                    self.skip_failed_block(start_pos, base_indent);
                }
                Err(e) => {
                    // A legacy single-string error: synthesize a line diagnostic.
                    let l = self.lines.get(start_pos).cloned().unwrap_or_else(|| Line {
                        indent: 0,
                        tokens: vec![],
                        line_num: 0,
                        raw: String::new(),
                    });
                    diags.push(legacy_error_diag(&l, &e));
                    self.skip_failed_block(start_pos, base_indent);
                }
            }
        }

        (scene, diags)
    }

    /// Advance past a block that failed to parse: consume its header line (if we
    /// didn't already) plus any deeper-indented body, so the next iteration
    /// lands on the following sibling. Guarantees forward progress.
    fn skip_failed_block(&mut self, start_pos: usize, base_indent: usize) {
        // Ensure at least the header line is consumed.
        if self.pos <= start_pos {
            self.pos = start_pos + 1;
        }
        while let Some(line) = self.peek() {
            if line.indent <= base_indent {
                break;
            }
            self.pos += 1;
        }
    }

    /// Parse exactly one top-level construct, pushing into `scene`. Factored out
    /// of [`parse_scene`] so the recovering parser can call it per-block.
    fn parse_one_top_level(
        &mut self,
        keyword: &str,
        base_indent: usize,
        scene: &mut Scene,
    ) -> Result<()> {
        match keyword {
            "documentsize" => {
                let line = self.advance().clone();
                if line.tokens.len() < 2 {
                    return Err(StrokError::ParseDiagnostics(vec![line.diag(
                        0,
                        "documentsize requires WxH (e.g. `documentsize 100x100`)",
                    )]));
                }
                scene.document_size = Dimension::parse(&line.tokens[1])?;
            }
            "use" => {
                let line = self.advance().clone();
                let import = parse_use_line(&line)?;
                scene.imports.push(import);
            }
            "palette" => {
                self.advance();
                let body = self.collect_body(base_indent);
                let tokens = parse_palette_tokens(&body)?;
                scene.palette.tokens.extend(tokens);
            }
            "scheme" => {
                let line = self.advance().clone();
                if line.tokens.len() < 2 {
                    return Err(StrokError::ParseDiagnostics(vec![
                        line.diag(0, "scheme requires a name")
                    ]));
                }
                let name = line.tokens[1].clone();
                let body = self.collect_body(base_indent);
                let tokens = parse_palette_tokens(&body)?;
                scene.palette.schemes.push(ColorScheme { name, tokens });
            }
            "defaults" => {
                self.advance();
                let body = self.collect_body(base_indent);
                scene.defaults = parse_defaults_body(&body)?;
            }
            "let" => {
                let line = self.advance().clone();
                self.collect_body(base_indent); // `let` is single-line; ignore stray body
                let (name, source, value) = parse_let_line(&line, &self.env, scene)?;
                self.env.set(name.clone(), value);
                scene.lets.push((name, source, value));
            }
            "shape" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let shape = parse_shape_line(&line, &body, &self.env)?;
                scene.shapes.push(shape);
            }
            "place" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let place = parse_place_line(&line, &body, &self.env)?;
                scene.nodes.push(SceneNode::Place(place));
            }
            "group" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let group = parse_group_block(&line, &body, &self.env, &mut scene.shapes)?;
                scene.nodes.push(SceneNode::Group(group));
            }
            "boolean" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let boolean = parse_boolean_block(&line, &body, &self.env)?;
                scene.nodes.push(SceneNode::Boolean(boolean));
            }
            "createlink" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let (link, shape) = parse_link_line(&line, &body)?;
                scene.nodes.push(SceneNode::Link(link));
                scene.shapes.push(shape);
            }
            "repeat" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let mut nodes = Vec::new();
                let mut shapes = Vec::new();
                parse_repeat(&line, &body, &self.env, "", &mut nodes, &mut shapes)?;
                scene.nodes.extend(nodes);
                scene.shapes.extend(shapes);
            }
            "tokens" => {
                self.advance();
                let body = self.collect_body(base_indent);
                let tokens = parse_tokens_body(&body)?;
                scene.design_tokens.extend(tokens);
            }
            "frame" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let frame = parse_frame_block(&line, &body, &self.env)?;
                scene.nodes.push(SceneNode::Frame(frame));
            }
            "component" => {
                let line = self.advance().clone();
                let body = self.collect_body(base_indent);
                let component = parse_component_block(&line, &body, &self.env)?;
                scene.components.push(component);
            }
            "instance" => {
                let line = self.advance().clone();
                self.collect_body(base_indent); // instances are single-line; ignore stray body
                let instance = parse_instance_line(&line, &self.env)?;
                scene.nodes.push(SceneNode::Instance(instance));
            }
            "unlink" | "reorder" => {
                self.advance();
                self.collect_body(base_indent);
            }
            _ => {
                let line = self.advance().clone();
                return Err(StrokError::ParseDiagnostics(vec![top_level_keyword_diag(
                    &line, keyword,
                )]));
            }
        }
        Ok(())
    }
}

/// Diagnostic for an unknown top-level keyword, with a "did you mean" hint.
fn top_level_keyword_diag(line: &Line, keyword: &str) -> Diagnostic {
    let mut d = line.diag(0, format!("unexpected top-level keyword '{}'", keyword));
    if let Some(s) = suggest(keyword, TOP_LEVEL_KEYWORDS) {
        d = d.with_suggestion(s);
    }
    d
}

/// Convert a legacy single-string error into a positioned [`Diagnostic`] for the
/// recovery path. Pulls a `(line N)` suffix out of the message if present.
fn legacy_error_diag(line: &Line, e: &StrokError) -> Diagnostic {
    let msg = e.to_string();
    let line_num = if line.line_num > 0 { line.line_num } else { 0 };
    Diagnostic::new(line_num, msg).with_source(line.raw.clone())
}

// ── Shape parsing ─────────────────────────────────────────────────────

fn parse_shape_line(line: &Line, body: &[Line], env: &Env) -> Result<Shape> {
    // shape <name> template=<T>
    if line.tokens.len() < 2 {
        return Err(StrokError::ParseError(format!(
            "shape requires a name (line {})",
            line.line_num
        )));
    }
    let name = &line.tokens[1];
    // Reject a malformed name (e.g. an unterminated quote that swallowed the
    // rest of the line, or stray attributes absorbed into the name). Without
    // this a garbage `shape` line parsed "successfully" but re-emitted into a
    // line that no longer round-trips — found by `fuzz_roundtrip`.
    crate::types::validate_ident(name)
        .map_err(|e| parse_err(line, &format!("invalid shape name: {e}")))?;

    let mut template = Template::Path; // default
    for token in &line.tokens[2..] {
        if let Some(val) = token.strip_prefix("template=") {
            template = Template::parse(val)?;
        }
    }

    let mut shape = Shape::new(name, template);

    // Parse body lines as operations
    for body_line in body {
        if body_line.tokens.is_empty() {
            continue;
        }
        let keyword = body_line.tokens[0].as_str();
        match keyword {
            "movepoint" => shape.operations.push(parse_movepoint(body_line)?),
            "pullpoint" => shape.operations.push(parse_pullpoint(body_line)?),
            "sculpt" => shape.operations.push(parse_sculpt(body_line)?),
            "addpoint" => shape.operations.push(parse_addpoint(body_line, env)?),
            "splitline" => shape.operations.push(parse_splitline(body_line)?),
            "deletepoint" => shape.operations.push(parse_deletepoint(body_line)?),
            "close" => shape.operations.push(Operation::Close),
            "open" => shape.operations.push(Operation::Open),
            "subpath" => shape.operations.push(Operation::Subpath),
            "smooth" => shape.operations.push(parse_smooth(body_line)?),
            "smooth-corner" => shape.operations.push(parse_smooth_corner(body_line)?),
            "sharpen" => shape.operations.push(parse_sharpen(body_line)?),
            "convert-point" => shape.operations.push(parse_convert_point(body_line)?),
            "round-corners" => shape.operations.push(parse_round_corners(body_line, env)?),
            "notch" => shape.operations.push(parse_notch(body_line)?),
            "fill" => shape.operations.push(parse_fill(body_line)?),
            "fill-rule" => shape.operations.push(parse_fill_rule(body_line)?),
            "stroke" => shape.operations.push(parse_stroke(body_line)?),
            "stroke-width" => shape.operations.push(parse_stroke_width(body_line)?),
            "stroke-linecap" => shape.operations.push(parse_stroke_linecap(body_line)?),
            "stroke-linejoin" => shape.operations.push(parse_stroke_linejoin(body_line)?),
            "stroke-miterlimit" => shape.operations.push(parse_stroke_miterlimit(body_line)?),
            "stroke-dasharray" => shape.operations.push(parse_stroke_dasharray(body_line)?),
            "opacity" => shape.operations.push(parse_opacity(body_line)?),
            "blur" => shape.operations.push(parse_blur(body_line)?),
            "content" => shape.operations.push(parse_content(body_line)?),
            "font-size" => shape.operations.push(parse_font_size(body_line)?),
            "font-family" => shape.operations.push(parse_font_family(body_line)?),
            "font-weight" => shape.operations.push(parse_font_weight(body_line)?),
            "font-style" => shape.operations.push(parse_font_style(body_line)?),
            "text-anchor" => shape.operations.push(parse_text_anchor(body_line)?),
            "applyeffect" => shape.effects.push(parse_effect(body_line)?),
            _ => {
                let mut d =
                    body_line.diag(0, format!("unknown operation '{}' in shape block", keyword));
                if let Some(s) = suggest(keyword, SHAPE_OPS) {
                    d = d.with_suggestion(s);
                }
                return Err(StrokError::ParseDiagnostics(vec![d]));
            }
        }
    }

    Ok(shape)
}

// ── Operation parsers ─────────────────────────────────────────────────

fn parse_movepoint(line: &Line) -> Result<Operation> {
    // movepoint <point> dx=N dy=N
    // movepoint <point> to=x,y
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "movepoint requires a point name"));
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    if let Some(to_val) = attrs.get("to") {
        let (x, y) = parse_point_coord(to_val)?;
        Ok(Operation::MovePointTo { point, to: (x, y) })
    } else {
        let dx = parse_kv_f64(&attrs, "dx")?;
        let dy = parse_kv_f64(&attrs, "dy")?;
        Ok(Operation::MovePointDelta { point, dx, dy })
    }
}

fn parse_pullpoint(line: &Line) -> Result<Operation> {
    // pullpoint <point> dir=<Direction> <RelativeSize>
    // pullpoint <point> dx=N dy=N radius=N falloff=N
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "pullpoint requires a point name"));
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    if let Some(dir_val) = attrs.get("dir") {
        let dir = Direction::parse(dir_val)?;
        // Find the non-kv token (the RelativeSize)
        let amount = find_positional_token(&line.tokens[2..], &attrs)
            .ok_or_else(|| parse_err(line, "pullpoint dir requires a RelativeSize amount"))?;
        let rs = RelativeSize::parse(&amount)?;
        Ok(Operation::PullPointDir {
            point,
            dir,
            amount: rs,
        })
    } else {
        let dx = parse_kv_f64(&attrs, "dx")?;
        let dy = parse_kv_f64(&attrs, "dy")?;
        let radius = attrs
            .get("radius")
            .map(|v| v.parse::<usize>())
            .transpose()
            .map_err(|_| parse_err(line, "invalid radius"))?
            .unwrap_or(1);
        let falloff = attrs
            .get("falloff")
            .map(|v| NormalizedAmount::parse(v))
            .transpose()?
            .unwrap_or(NormalizedAmount(1.0));
        Ok(Operation::PullPointDelta {
            point,
            dx,
            dy,
            radius,
            falloff,
        })
    }
}

fn parse_sculpt(line: &Line) -> Result<Operation> {
    let attrs = parse_kv_attrs(&line.tokens[1..]);
    let at_val = attrs
        .get("at")
        .ok_or_else(|| parse_err(line, "sculpt requires at="))?;
    let at = parse_sculpt_target(at_val)?;
    let dx = parse_kv_f64(&attrs, "dx")?;
    let dy = parse_kv_f64(&attrs, "dy")?;
    let radius = attrs
        .get("radius")
        .map(|v| AbsoluteSize::parse(v))
        .transpose()?;
    let falloff = attrs
        .get("falloff")
        .map(|v| NormalizedAmount::parse(v))
        .transpose()?;
    let axis = attrs
        .get("axis")
        .map(|v| SculptAxis::parse(v))
        .transpose()?;
    let lock_endpoints = attrs.contains_key("lock-endpoints");

    Ok(Operation::Sculpt {
        at,
        dx,
        dy,
        radius,
        falloff,
        axis,
        lock_endpoints,
    })
}

fn parse_addpoint(line: &Line, env: &Env) -> Result<Operation> {
    // addpoint <name> at=x,y [after=<point>] [mode=sharp|catmull-rom|arc|controls|controls-relative]
    //   arc mode: [rx=N] [ry=N] [sweep=0|1] [large=0|1]
    //   controls modes: c1=x,y c2=x,y
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "addpoint requires a name"));
    }
    let name = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    let at_val = attrs.get("at").ok_or_else(|| {
        // If the author wrote a bare `x,y` positional (no `at=`), point them at
        // the keyed form — the most common addpoint mistake (E3.1 example).
        if let Some((idx, tok)) = line
            .tokens
            .iter()
            .enumerate()
            .skip(2)
            .find(|(_, t)| looks_like_coord(t))
        {
            StrokError::ParseDiagnostics(vec![line
                .diag(
                    idx,
                    "addpoint requires `at=x,y` (coordinates must be keyed)",
                )
                .with_suggestion(format!("at={tok}"))])
        } else {
            StrokError::ParseDiagnostics(vec![line.diag(0, "addpoint requires at=x,y")])
        }
    })?;
    let (x, y) = eval_coord(line, at_val, env)?;
    let after = attrs.get("after").cloned();
    let mode = attrs.get("mode").map(|v| PointMode::parse(v)).transpose()?;
    let tension = attrs
        .get("tension")
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| StrokError::ParseError(format!("invalid tension: '{}'", v)))
        })
        .transpose()?;

    let arc_rx = attrs
        .get("rx")
        .map(|v| v.parse::<f64>())
        .transpose()
        .map_err(|_| parse_err(line, "invalid rx value"))?;
    let arc_ry = attrs
        .get("ry")
        .map(|v| v.parse::<f64>())
        .transpose()
        .map_err(|_| parse_err(line, "invalid ry value"))?;
    // `sweep` is the raw SVG arc flag. We also accept `cw`/`ccw` as readable
    // synonyms for `1`/`0` (still order-dependent, like the numeric flag).
    let arc_sweep = attrs
        .get("sweep")
        .map(|v| match v.as_str() {
            "0" | "ccw" => Ok(false),
            "1" | "cw" => Ok(true),
            _ => Err(parse_err(line, "sweep must be 0/1 or cw/ccw")),
        })
        .transpose()?;
    let arc_large = attrs
        .get("large")
        .map(|v| match v.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(parse_err(line, "large must be 0 or 1")),
        })
        .transpose()?;
    // `bulge=left|right` is the order-independent way to choose the arc side.
    let arc_bulge = attrs.get("bulge").map(|v| ArcBulge::parse(v)).transpose()?;
    let control_c1 = attrs.get("c1").map(|v| parse_point_coord(v)).transpose()?;
    let control_c2 = attrs.get("c2").map(|v| parse_point_coord(v)).transpose()?;

    if matches!(
        mode,
        Some(PointMode::Controls | PointMode::ControlsRelative)
    ) && (control_c1.is_none() || control_c2.is_none())
    {
        return Err(parse_err(
            line,
            "controls modes require both c1=x,y and c2=x,y",
        ));
    }

    Ok(Operation::AddPoint {
        name,
        at: (x, y),
        after,
        mode,
        tension,
        arc_rx,
        arc_ry,
        arc_sweep,
        arc_large,
        arc_bulge,
        control_c1,
        control_c2,
    })
}

fn parse_splitline(line: &Line) -> Result<Operation> {
    // splitline <segment_ref> name=<ident> [t=N]
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "splitline requires a segment reference"));
    }
    let segment = SegmentRef::parse(&line.tokens[1])?;
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let name = attrs
        .get("name")
        .ok_or_else(|| parse_err(line, "splitline requires name="))?
        .clone();
    let t = attrs
        .get("t")
        .map(|v| NormalizedAmount::parse(v))
        .transpose()?;

    Ok(Operation::SplitLine { segment, name, t })
}

fn parse_deletepoint(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "deletepoint requires a point name"));
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let reconnect = attrs
        .get("reconnect")
        .map(|v| ReconnectMode::parse(v))
        .transpose()?;

    Ok(Operation::DeletePoint { point, reconnect })
}

fn parse_smooth(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "smooth requires a point name or 'all'"));
    }
    if line.tokens[1] == "all" {
        let attrs = parse_kv_attrs(&line.tokens[2..]);
        let tension = attrs
            .get("tension")
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| StrokError::ParseError(format!("invalid tension: '{}'", v)))
            })
            .transpose()?;
        return Ok(Operation::SmoothAll { tension });
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let tension = attrs
        .get("tension")
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| StrokError::ParseError(format!("invalid tension: '{}'", v)))
        })
        .transpose()?;

    Ok(Operation::Smooth { point, tension })
}

fn parse_smooth_corner(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "smooth-corner requires a point name"));
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let tension = attrs
        .get("tension")
        .map(|v| {
            v.parse::<f64>()
                .map_err(|_| StrokError::ParseError(format!("invalid tension: '{}'", v)))
        })
        .transpose()?;
    Ok(Operation::SmoothCorner { point, tension })
}

fn parse_sharpen(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "sharpen requires a point name or 'all'"));
    }
    if line.tokens[1] == "all" {
        return Ok(Operation::SharpenAll);
    }
    Ok(Operation::Sharpen {
        point: line.tokens[1].clone(),
    })
}

fn parse_convert_point(line: &Line) -> Result<Operation> {
    // convert-point <point> to=sharp|smooth|arc|controls
    if line.tokens.len() < 2 {
        return Err(parse_err(
            line,
            "convert-point requires a point name and to=sharp|smooth|arc|controls",
        ));
    }
    let point = line.tokens[1].clone();
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let to_val = attrs
        .get("to")
        .ok_or_else(|| parse_err(line, "convert-point requires to=sharp|smooth|arc|controls"))?;
    let to = ConvertTarget::parse(to_val).ok_or_else(|| {
        parse_err(
            line,
            &format!(
                "'{}' is not a valid convert target — use sharp, smooth, arc, or controls",
                to_val
            ),
        )
    })?;
    Ok(Operation::ConvertPoint { point, to })
}

fn parse_round_corners(line: &Line, env: &Env) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(
            line,
            "round-corners requires a radius (e.g. `round-corners 8`) or per-corner radii (e.g. `round-corners tl=8 tr=8 br=0 bl=0`)",
        ));
    }
    // Per-corner form: any token contains `=` → CornerRadii::PerCorner.
    if line.tokens[1..].iter().any(|t| t.contains('=')) {
        let mut list = Vec::new();
        for tok in &line.tokens[1..] {
            let (name, val) = tok.split_once('=').ok_or_else(|| {
                parse_err(
                    line,
                    "mixed round-corners form — use either a single radius or all `corner=radius` pairs",
                )
            })?;
            let r = eval_scalar_l(line, val, env)?;
            list.push((name.to_string(), r));
        }
        return Ok(Operation::RoundCorners {
            radii: CornerRadii::PerCorner(list),
        });
    }
    let radius = eval_scalar_l(line, &line.tokens[1], env)?;
    Ok(Operation::RoundCorners {
        radii: CornerRadii::uniform(radius),
    })
}

fn parse_notch(line: &Line) -> Result<Operation> {
    // notch edge=top dir=out shape=square pos=0.3 width=10 depth=4
    // edge= is `top|bottom|left|right` or `startpt,endpt` for path edges.
    let attrs = parse_kv_attrs(&line.tokens[1..]);
    let edge_raw = attrs.get("edge").ok_or_else(|| {
        parse_err(
            line,
            "notch requires edge=top|bottom|left|right or edge=p1,p2",
        )
    })?;
    let edge = if let Some(e) = Edge::parse(edge_raw) {
        NotchEdge::Named(e)
    } else if let Some((a, b)) = edge_raw.split_once(',') {
        NotchEdge::Segment(a.to_string(), b.to_string())
    } else {
        return Err(parse_err(
            line,
            "notch edge must be top|bottom|left|right or a point pair `p1,p2`",
        ));
    };
    let dir = attrs
        .get("dir")
        .and_then(|s| NotchDir::parse(s))
        .unwrap_or(NotchDir::Out);
    let shape = attrs
        .get("shape")
        .and_then(|s| NotchShape::parse(s))
        .unwrap_or(NotchShape::Square);
    let pos = attrs
        .get("pos")
        .map(|s| s.parse::<f64>())
        .transpose()
        .map_err(|_| parse_err(line, "invalid notch pos"))?
        .unwrap_or(0.5);
    let width =
        parse_kv_f64(&attrs, "width").map_err(|_| parse_err(line, "notch requires width=N"))?;
    let depth =
        parse_kv_f64(&attrs, "depth").map_err(|_| parse_err(line, "notch requires depth=N"))?;
    Ok(Operation::Notch(NotchSpec {
        edge,
        dir,
        shape,
        pos,
        width,
        depth,
    }))
}

fn parse_fill(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "fill requires a color"));
    }
    if line.tokens[1].starts_with("radial(") || line.tokens[1].starts_with("linear(") {
        let full = line.tokens[1..].join(" ");
        return Ok(Operation::Fill(Color::parse_gradient(&full)?));
    }
    Ok(Operation::Fill(Color::parse(&line.tokens[1])?))
}

fn parse_stroke(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "stroke requires a color"));
    }
    if line.tokens[1].starts_with("radial(") || line.tokens[1].starts_with("linear(") {
        let full = line.tokens[1..].join(" ");
        return Ok(Operation::Stroke(Color::parse_gradient(&full)?));
    }
    Ok(Operation::Stroke(Color::parse(&line.tokens[1])?))
}

fn parse_stroke_width(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "stroke-width requires a value"));
    }
    Ok(Operation::StrokeWidth(AbsoluteSize::parse(
        &line.tokens[1],
    )?))
}

fn parse_stroke_linecap(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "stroke-linecap requires a value"));
    }
    Ok(Operation::StrokeLinecap(LineCap::parse(&line.tokens[1])?))
}

fn parse_stroke_linejoin(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "stroke-linejoin requires a value"));
    }
    Ok(Operation::StrokeLinejoin(LineJoin::parse(&line.tokens[1])?))
}

fn parse_stroke_miterlimit(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "stroke-miterlimit requires a value"));
    }
    let m: f64 = line.tokens[1]
        .parse()
        .map_err(|_| parse_err(line, "stroke-miterlimit must be a number"))?;
    Ok(Operation::StrokeMiterlimit(m))
}

fn parse_fill_rule(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "fill-rule requires a value"));
    }
    Ok(Operation::FillRule(FillRule::parse(&line.tokens[1])?))
}

fn parse_text_anchor(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "text-anchor requires a value"));
    }
    Ok(Operation::TextAnchor(TextAnchor::parse(&line.tokens[1])?))
}

fn parse_stroke_dasharray(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(
            line,
            "stroke-dasharray requires at least one value",
        ));
    }
    let values: Vec<f64> = line.tokens[1..]
        .iter()
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| parse_err(line, &format!("invalid dasharray value '{}'", t)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Operation::StrokeDasharray(values))
}

fn parse_opacity(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "opacity requires a value"));
    }
    Ok(Operation::Opacity(NormalizedAmount::parse(
        &line.tokens[1],
    )?))
}

fn parse_blur(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "blur requires a radius value"));
    }
    let radius: f64 = line.tokens[1]
        .parse()
        .map_err(|_| parse_err(line, "invalid blur radius"))?;
    Ok(Operation::Blur(radius))
}

fn parse_content(line: &Line) -> Result<Operation> {
    // content "text string" or content bare-text
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "content requires a value"));
    }
    // Join all tokens after "content" and strip surrounding quotes
    let raw = line.tokens[1..].join(" ");
    let text = unquote_string_value(line, &raw)?;
    Ok(Operation::Content(text))
}

/// Strip a surrounding pair of quotes from a DSL string value, rejecting
/// anything that cannot round-trip: an unterminated quote (starts with `"` but
/// does not end with one) or an embedded `"` (the DSL has no escape syntax, so
/// an interior quote would re-tokenize differently). Found by `fuzz_roundtrip`
/// on malformed `content`/`font-family` strings.
fn unquote_string_value(line: &Line, raw: &str) -> Result<String> {
    if let Some(inner) = raw.strip_prefix('"') {
        let Some(inner) = inner.strip_suffix('"') else {
            return Err(parse_err(
                line,
                "unterminated string (missing closing quote)",
            ));
        };
        if inner.contains('"') {
            return Err(parse_err(
                line,
                "string value may not contain a literal quote character",
            ));
        }
        Ok(inner.to_string())
    } else {
        if raw.contains('"') {
            return Err(parse_err(
                line,
                "string value may not contain a literal quote character",
            ));
        }
        Ok(raw.to_string())
    }
}

fn parse_font_size(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "font-size requires a value"));
    }
    let v: f64 = line.tokens[1]
        .parse()
        .map_err(|_| parse_err(line, "invalid font-size"))?;
    Ok(Operation::FontSize(v))
}

fn parse_font_family(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "font-family requires a value"));
    }
    let raw = line.tokens[1..].join(" ");
    let text = unquote_string_value(line, &raw)?;
    Ok(Operation::FontFamily(text))
}

fn parse_font_weight(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "font-weight requires a value"));
    }
    Ok(Operation::FontWeight(line.tokens[1].clone()))
}

fn parse_font_style(line: &Line) -> Result<Operation> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "font-style requires a value"));
    }
    Ok(Operation::FontStyle(line.tokens[1].clone()))
}

fn parse_effect(line: &Line) -> Result<Effect> {
    // applyeffect <type> <amount> [key=value...]
    if line.tokens.len() < 3 {
        return Err(parse_err(line, "applyeffect requires type and amount"));
    }
    let effect_type = &line.tokens[1];
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    match effect_type.as_str() {
        "droop" => {
            let amount = NormalizedAmount::parse(&line.tokens[2])?;
            let direction = attrs
                .get("direction")
                .map(|v| Direction::parse(v))
                .transpose()?;
            Ok(Effect::Droop { amount, direction })
        }
        "curl" => {
            let amount = NormalizedAmount::parse(&line.tokens[2])?;
            let from = attrs.get("from").map(|v| PointRef::parse(v)).transpose()?;
            Ok(Effect::Curl { amount, from })
        }
        "taper" => {
            let start = attrs
                .get("start")
                .ok_or_else(|| parse_err(line, "taper requires start="))?;
            let end = attrs
                .get("end")
                .ok_or_else(|| parse_err(line, "taper requires end="))?;
            Ok(Effect::Taper {
                start: RelativeSize::parse(start)?,
                end: RelativeSize::parse(end)?,
            })
        }
        "jitter" => {
            let amount = NormalizedAmount::parse(&line.tokens[2])?;
            let seed = attrs
                .get("seed")
                .map(|v| {
                    v.parse::<u32>()
                        .map_err(|_| parse_err(line, "invalid seed"))
                })
                .transpose()?;
            Ok(Effect::Jitter { amount, seed })
        }
        _ => Err(parse_err(
            line,
            &format!("unknown effect type '{}'", effect_type),
        )),
    }
}

/// Parse a `skew=` value: `deg` (x only) or `degx,degy`. Degrees, no unit suffix.
fn parse_skew_value(line: &Line, v: &str) -> Result<(f64, f64)> {
    let parts: Vec<&str> = v.split(',').collect();
    let parse = |s: &str| -> Result<f64> {
        s.trim()
            .trim_end_matches("deg")
            .parse::<f64>()
            .map_err(|_| parse_err(line, &format!("invalid skew value '{}'", v)))
    };
    match parts.len() {
        1 => Ok((parse(parts[0])?, 0.0)),
        2 => Ok((parse(parts[0])?, parse(parts[1])?)),
        _ => Err(parse_err(line, "skew takes deg or degx,degy")),
    }
}

// ── Group parsing ─────────────────────────────────────────────────────

/// Parse a `group` header line (name + transforms) into a childless [`Group`].
/// `expand_nodes` / `parse_group_block` fill in the children.
fn parse_group_header(line: &Line, env: &Env) -> Result<Group> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "group requires a name"));
    }
    let name = line.tokens[1].clone();
    validate_ident(&name).map_err(|e| parse_err(line, &format!("invalid group name: {e}")))?;
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    // Parse transform attributes (same as Place). Coordinates/rotation may be
    // scalar expressions (C13).
    let position = if let Some(at_val) = attrs.get("at") {
        Some(eval_coord(line, at_val, env)?)
    } else {
        None
    };
    let rotation = attrs
        .get("rotation")
        .map(|v| eval_rotation(line, v, env))
        .transpose()?;
    let flip = attrs.get("flip").map(|v| Flip::parse(v)).transpose()?;
    let skew = attrs
        .get("skew")
        .map(|v| parse_skew_value(line, v))
        .transpose()?;

    let clip = attrs
        .get("clip")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let mask = attrs.get("mask").cloned();
    let opacity = attrs
        .get("opacity")
        .map(|v| eval_scalar_l(line, v, env))
        .transpose()?;

    Ok(Group {
        name,
        children: Vec::new(),
        position,
        rotation,
        flip,
        skew,
        clip,
        mask,
        opacity,
    })
}

fn parse_group_block(
    line: &Line,
    body: &[Line],
    env: &Env,
    out_shapes: &mut Vec<Shape>,
) -> Result<Group> {
    let mut group = parse_group_header(line, env)?;
    // A top-level (non-repeat) group expands its children with an empty suffix /
    // rename set — so `expand_nodes` is a no-op renamer but still handles env
    // expressions and any nested `repeat`/`createlink` uniformly.
    let empty: HashSet<String> = HashSet::new();
    let mut children = Vec::new();
    let mut context = ExpansionContext {
        env,
        suffix: "",
        rename: &empty,
        out_nodes: &mut children,
        out_shapes,
    };
    expand_nodes(body, &mut context)?;
    group.children = children;
    Ok(group)
}

// ── Live boolean parsing ────────────────────────────────────────────────────

/// Parse a non-destructive boolean composition:
///
/// ```text
/// boolean silhouette op=union
///   place head shape=head center=20,15 size=12x14
///   place neck shape=neck at=15,18
///   fill #f7f3ea
/// ```
fn parse_boolean_block(line: &Line, body: &[Line], env: &Env) -> Result<Boolean> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "boolean requires a name"));
    }
    let name = line.tokens[1].clone();
    validate_ident(&name).map_err(|e| parse_err(line, &format!("invalid boolean name: {e}")))?;
    let attrs = parse_kv_attrs(&line.tokens[2..]);
    for key in attrs.keys() {
        if key != "op" {
            return Err(parse_err(
                line,
                &format!("unknown boolean attribute '{}=' (valid: op=)", key),
            ));
        }
    }
    let op_raw = attrs
        .get("op")
        .ok_or_else(|| parse_err(line, "boolean requires op=union|subtract|intersect|exclude"))?;
    let op = crate::bool_ops::BoolOp::parse(op_raw).map_err(|e| parse_err(line, &e.to_string()))?;

    let mut children = Vec::new();
    let mut operations = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let (body_line, sub_body, next) = next_block(body, i);
        match body_line.tokens[0].as_str() {
            "place" => children.push(SceneNode::Place(parse_place_line(
                body_line, sub_body, env,
            )?)),
            "fill" => operations.push(parse_fill(body_line)?),
            "fill-rule" => operations.push(parse_fill_rule(body_line)?),
            "stroke" => operations.push(parse_stroke(body_line)?),
            "stroke-width" => operations.push(parse_stroke_width(body_line)?),
            "stroke-linecap" => operations.push(parse_stroke_linecap(body_line)?),
            "stroke-linejoin" => operations.push(parse_stroke_linejoin(body_line)?),
            "stroke-miterlimit" => operations.push(parse_stroke_miterlimit(body_line)?),
            "stroke-dasharray" => operations.push(parse_stroke_dasharray(body_line)?),
            "opacity" => operations.push(parse_opacity(body_line)?),
            "blur" => operations.push(parse_blur(body_line)?),
            keyword => {
                return Err(parse_err(
                    body_line,
                    &format!(
                        "unexpected '{}' in boolean (allowed: place and fill/stroke/opacity/blur styles)",
                        keyword
                    ),
                ));
            }
        }
        i = next;
    }
    if children.len() < 2 {
        return Err(parse_err(
            line,
            "boolean requires at least two placed operands",
        ));
    }
    Ok(Boolean {
        name,
        op,
        children,
        operations,
    })
}

// ── Link parsing ──────────────────────────────────────────────────────

/// Parse createlink: returns (Link, Shape).
/// The Link tracks inheritance. The Shape is a placeable entry
/// with override attrs that resolves geometry from its source.
fn parse_link_line(line: &Line, body: &[Line]) -> Result<(Link, Shape)> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "createlink requires a name"));
    }
    let name = line.tokens[1].clone();
    crate::types::validate_ident(&name)
        .map_err(|e| parse_err(line, &format!("invalid createlink name: {e}")))?;
    let attrs = parse_kv_attrs(&line.tokens[2..]);

    let source = attrs
        .get("from")
        .ok_or_else(|| parse_err(line, "createlink requires from="))?
        .clone();
    crate::types::validate_ident(&source)
        .map_err(|e| parse_err(line, &format!("invalid createlink source: {e}")))?;

    let mut operations = Vec::new();
    let mut effects = Vec::new();

    for body_line in body {
        if body_line.tokens.is_empty() {
            continue;
        }
        match body_line.tokens[0].as_str() {
            "fill" => operations.push(parse_fill(body_line)?),
            "fill-rule" => operations.push(parse_fill_rule(body_line)?),
            "stroke" => operations.push(parse_stroke(body_line)?),
            "stroke-width" => operations.push(parse_stroke_width(body_line)?),
            "stroke-linecap" => operations.push(parse_stroke_linecap(body_line)?),
            "stroke-linejoin" => operations.push(parse_stroke_linejoin(body_line)?),
            "stroke-miterlimit" => operations.push(parse_stroke_miterlimit(body_line)?),
            "stroke-dasharray" => operations.push(parse_stroke_dasharray(body_line)?),
            "opacity" => operations.push(parse_opacity(body_line)?),
            "blur" => operations.push(parse_blur(body_line)?),
            "text-anchor" => operations.push(parse_text_anchor(body_line)?),
            "applyeffect" => effects.push(parse_effect(body_line)?),
            _ => {
                return Err(StrokError::ParseError(format!(
                    "unexpected override '{}' in createlink (line {})\n\n\
Valid overrides: fill  stroke  stroke-width  stroke-linecap  stroke-linejoin\n\
                 stroke-dasharray  opacity  blur  text-anchor",
                    body_line.tokens[0], body_line.line_num
                )));
            }
        }
    }

    let link = Link {
        name: name.clone(),
        source,
        overrides: operations.clone(),
        effects: effects.clone(),
    };

    let mut shape = Shape::new(&name, Template::Path);
    shape.operations = operations;
    shape.effects = effects;

    Ok((link, shape))
}

// ── Use/import parsing ────────────────────────────────────────────────

fn parse_use_line(line: &Line) -> Result<Import> {
    // use "./path.strok"
    // use "./path.strok" as namespace
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "use requires a path"));
    }
    let raw_path = &line.tokens[1];
    // Strip surrounding quotes, rejecting an unterminated or quote-bearing path
    // that would not survive re-emission (emit re-wraps the path in quotes).
    let path = unquote_string_value(line, raw_path)?;

    let namespace = if line.tokens.len() >= 4 && line.tokens[2] == "as" {
        let ns = line.tokens[3].clone();
        crate::types::validate_ident(&ns)
            .map_err(|e| parse_err(line, &format!("invalid import namespace: {e}")))?;
        Some(ns)
    } else {
        None
    };

    Ok(Import { path, namespace })
}

// ── Palette / scheme parsing ──────────────────────────────────────────

fn parse_palette_tokens(body: &[Line]) -> Result<Vec<(String, String)>> {
    let mut tokens = Vec::new();
    for line in body {
        if line.tokens.len() < 2 {
            return Err(parse_err(
                line,
                "palette entry requires: <name> <color> (e.g. hero #e8a840)",
            ));
        }
        let name = line.tokens[0].clone();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(parse_err(
                line,
                &format!("invalid token name '{}' (use letters, digits, -, _)", name),
            ));
        }
        let value = line.tokens[1].clone();
        // Tokens map to concrete colors only — not gradients or other tokens.
        match Color::parse(&value)? {
            Color::Hex(_) | Color::None | Color::CurrentColor => {}
            _ => {
                return Err(parse_err(
                    line,
                    &format!(
                        "token '{}' must be a hex color, currentColor, or none",
                        name
                    ),
                ))
            }
        }
        tokens.push((name, value));
    }
    Ok(tokens)
}

// ── Generalized design tokens (C8 / E4.1) ─────────────────────────────

/// Parse a `tokens` block body: `category.name value` lines (dotted spelling).
/// Bare `name value` (no dot) is accepted and filed under the `color` category
/// for ergonomic parity with `palette`.
fn parse_tokens_body(body: &[Line]) -> Result<Vec<DesignToken>> {
    let mut tokens = Vec::new();
    for line in body {
        if line.tokens.len() < 2 {
            return Err(parse_err(
                line,
                "token entry requires: <category.name> <value> (e.g. radius.md 12)",
            ));
        }
        let key = &line.tokens[0];
        let (category, name) = match key.split_once('.') {
            Some((c, n)) => (c.to_string(), n.to_string()),
            None => ("color".to_string(), key.clone()),
        };
        if !is_token_ident(&category) || !is_token_ident(&name) {
            return Err(parse_err(
                line,
                &format!(
                    "invalid token key '{}' (use letters, digits, -, _ in category.name)",
                    key
                ),
            ));
        }
        // The value is the remainder of the line after the key, so quoted font
        // names with spaces survive (e.g. `font.body "IBM Plex Sans"`).
        let value = line.tokens[1..].join(" ");
        tokens.push(DesignToken {
            category,
            name,
            value,
        });
    }
    Ok(tokens)
}

fn is_token_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ── Frames (C8 / E4.1) ────────────────────────────────────────────────

/// Parse a `frame <name> [layout=…] [size=WxH] [at=x,y]` block plus its
/// indented attrs (`fill`, `round-corners`, `opacity`) and children.
fn parse_frame_block(line: &Line, body: &[Line], env: &Env) -> Result<Frame> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "frame requires a name"));
    }
    let name = line.tokens[1].clone();
    crate::types::validate_ident(&name)
        .map_err(|e| parse_err(line, &format!("invalid frame name: {e}")))?;

    let attrs = parse_kv_attrs(&line.tokens[2..]);
    // `layout=` may contain spaces/parens, so pull it from the raw line.
    let layout = match extract_paren_attr(&line.raw, "layout") {
        Some(v) => parse_layout(line, &v)?,
        None => match attrs.get("layout") {
            Some(v) => parse_layout(line, v)?,
            None => Layout::None,
        },
    };
    let size = attrs
        .get("size")
        .map(|v| eval_dimension(line, v, env))
        .transpose()?;
    let position = attrs
        .get("at")
        .map(|v| eval_coord(line, v, env))
        .transpose()?;

    let (fill, radius, opacity, children) = parse_container_body(body, env)?;

    Ok(Frame {
        name,
        layout,
        size,
        position,
        fill,
        radius,
        opacity,
        children,
    })
}

/// Shared body parser for frames (and component bodies): collects container
/// attrs (`fill`/`round-corners`/`opacity`) and child nodes.
#[allow(clippy::type_complexity)]
fn parse_container_body(
    body: &[Line],
    env: &Env,
) -> Result<(
    Option<Color>,
    Option<RadiusValue>,
    Option<f64>,
    Vec<SceneNode>,
)> {
    let mut fill = None;
    let mut radius = None;
    let mut opacity = None;
    let mut children = Vec::new();

    let mut i = 0;
    while i < body.len() {
        let body_line = &body[i];
        let keyword = body_line.tokens[0].as_str();
        let base_indent = body_line.indent;

        // Collect this line's deeper-indented sub-body.
        let mut sub_body = Vec::new();
        let mut j = i + 1;
        while j < body.len() && body[j].indent > base_indent {
            sub_body.push(body[j].clone());
            j += 1;
        }

        match keyword {
            "fill" => {
                if body_line.tokens.len() < 2 {
                    return Err(parse_err(body_line, "fill requires a color"));
                }
                fill = Some(Color::parse(&body_line.tokens[1])?);
            }
            "round-corners" => {
                if body_line.tokens.len() < 2 {
                    return Err(parse_err(body_line, "round-corners requires a value"));
                }
                radius = Some(parse_radius_value(body_line, &body_line.tokens[1])?);
            }
            "opacity" => {
                if body_line.tokens.len() < 2 {
                    return Err(parse_err(body_line, "opacity requires a value"));
                }
                opacity = Some(
                    body_line.tokens[1]
                        .parse::<f64>()
                        .map_err(|_| parse_err(body_line, "invalid opacity value"))?,
                );
            }
            "place" => children.push(SceneNode::Place(parse_place_line(
                body_line, &sub_body, env,
            )?)),
            "group" => {
                // Frames/components don't sink createlink shapes; a group here
                // that produces shapes (createlink) is unsupported.
                let mut throwaway = Vec::new();
                let group = parse_group_block(body_line, &sub_body, env, &mut throwaway)?;
                if !throwaway.is_empty() {
                    return Err(parse_err(
                        body_line,
                        "createlink inside a frame/component group is not supported",
                    ));
                }
                children.push(SceneNode::Group(group));
            }
            "frame" => children.push(SceneNode::Frame(parse_frame_block(
                body_line, &sub_body, env,
            )?)),
            "instance" => children.push(SceneNode::Instance(parse_instance_line(body_line, env)?)),
            _ => {
                return Err(parse_err(
                    body_line,
                    &format!(
                        "unexpected '{}' in frame/component (allowed: fill, round-corners, \
                         opacity, place, group, frame, instance)",
                        keyword
                    ),
                ));
            }
        }
        i = j;
    }

    Ok((fill, radius, opacity, children))
}

fn parse_radius_value(line: &Line, raw: &str) -> Result<RadiusValue> {
    if let Some(tok) = raw.strip_prefix('$') {
        if !tok.is_empty() {
            return Ok(RadiusValue::Token(tok.to_string()));
        }
    }
    let n = raw
        .parse::<f64>()
        .map_err(|_| parse_err(line, "round-corners requires a number or $token"))?;
    Ok(RadiusValue::Literal(n))
}

/// Parse a `layout=` value: `none`, `flow`, `flex(...)`, or `grid(...)`.
fn parse_layout(line: &Line, raw: &str) -> Result<Layout> {
    let raw = raw.trim();
    match raw {
        "none" | "" => return Ok(Layout::None),
        "flow" => return Ok(Layout::Flow),
        _ => {}
    }
    if let Some(inner) = paren_inner(raw, "flex") {
        return parse_flex_layout(line, &inner);
    }
    if let Some(inner) = paren_inner(raw, "grid") {
        return parse_grid_layout(line, &inner);
    }
    Err(parse_err(
        line,
        &format!(
            "unknown layout '{}' (expected none|flow|flex(...)|grid(...))",
            raw
        ),
    ))
}

fn parse_flex_layout(line: &Line, inner: &str) -> Result<Layout> {
    let mut direction = FlexDirection::Row;
    let mut gap = 0.0;
    let mut padding = (0.0, 0.0, 0.0, 0.0);
    let mut align = FlexAlign::Start;
    let mut justify = FlexJustify::Start;

    // `padding=` itself takes comma-separated numbers (`N,N` / `N,N,N,N`), which
    // collide with the comma that separates flex args. Reassemble: any bare
    // numeric arg immediately following a `padding=…` arg is folded back into it.
    let raw_args = split_layout_args(inner);
    let mut args: Vec<String> = Vec::new();
    for part in raw_args {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_bare_number = !trimmed.contains('=') && trimmed.parse::<f64>().is_ok();
        let prev_is_padding = args
            .last()
            .map(|a| a.trim_start().starts_with("padding="))
            .unwrap_or(false);
        if is_bare_number && prev_is_padding {
            if let Some(last) = args.last_mut() {
                last.push(',');
                last.push_str(trimmed);
            }
        } else {
            args.push(trimmed.to_string());
        }
    }

    for part in args {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            None => {
                // Bare direction keyword.
                match part {
                    "row" => direction = FlexDirection::Row,
                    "col" | "column" => direction = FlexDirection::Col,
                    _ => return Err(parse_err(line, &format!("unexpected flex arg '{}'", part))),
                }
            }
            Some((k, v)) => match k.trim() {
                "gap" => gap = parse_f64(line, v.trim())?,
                "padding" => padding = parse_padding(line, v.trim())?,
                "align" => {
                    align = match v.trim() {
                        "start" => FlexAlign::Start,
                        "center" => FlexAlign::Center,
                        "end" => FlexAlign::End,
                        "stretch" => FlexAlign::Stretch,
                        other => {
                            return Err(parse_err(line, &format!("invalid align '{}'", other)))
                        }
                    }
                }
                "justify" => {
                    justify = match v.trim() {
                        "start" => FlexJustify::Start,
                        "center" => FlexJustify::Center,
                        "end" => FlexJustify::End,
                        "between" => FlexJustify::Between,
                        other => {
                            return Err(parse_err(line, &format!("invalid justify '{}'", other)))
                        }
                    }
                }
                other => return Err(parse_err(line, &format!("unknown flex arg '{}'", other))),
            },
        }
    }

    Ok(Layout::Flex {
        direction,
        gap,
        padding,
        align,
        justify,
    })
}

fn parse_grid_layout(line: &Line, inner: &str) -> Result<Layout> {
    let mut columns = 1u32;
    let mut gap = 0.0;
    for part in split_layout_args(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((k, v)) => match k.trim() {
                "columns" | "cols" => {
                    columns = v
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| parse_err(line, "grid columns must be an integer"))?
                }
                "gap" => gap = parse_f64(line, v.trim())?,
                other => return Err(parse_err(line, &format!("unknown grid arg '{}'", other))),
            },
            None => {
                return Err(parse_err(
                    line,
                    &format!("unexpected grid arg '{}' (use columns=, gap=)", part),
                ))
            }
        }
    }
    Ok(Layout::Grid { columns, gap })
}

/// Parse a padding value: `N`, `x,y`, or `t,r,b,l` → `(t, r, b, l)`.
fn parse_padding(line: &Line, raw: &str) -> Result<(f64, f64, f64, f64)> {
    let nums: Result<Vec<f64>> = raw.split(',').map(|s| parse_f64(line, s.trim())).collect();
    let nums = nums?;
    match nums.as_slice() {
        [n] => Ok((*n, *n, *n, *n)),
        [x, y] => Ok((*y, *x, *y, *x)),
        [t, r, b, l] => Ok((*t, *r, *b, *l)),
        _ => Err(parse_err(
            line,
            "padding takes 1 (all), 2 (x,y), or 4 (t,r,b,l) numbers",
        )),
    }
}

fn parse_f64(line: &Line, s: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| parse_err(line, &format!("invalid number '{}'", s)))
}

/// Split layout args on commas that are NOT inside nested parens (defensive).
fn split_layout_args(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// `paren_inner("flex(row, gap=8)", "flex")` → `Some("row, gap=8")`.
fn paren_inner(raw: &str, prefix: &str) -> Option<String> {
    let rest = raw.strip_prefix(prefix)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(')?;
    let rest = rest.strip_suffix(')')?;
    Some(rest.to_string())
}

/// Extract a `key=<balanced-paren value>` from the raw source line, since the
/// whitespace tokenizer would otherwise split `layout=flex(row, gap=8)` apart.
/// Returns the value text (e.g. `flex(row, gap=8)`), or `None` if not present.
fn extract_paren_attr(raw: &str, key: &str) -> Option<String> {
    let needle = format!("{}=", key);
    let start = raw.find(&needle)? + needle.len();
    let bytes = raw[start..].char_indices();
    let mut depth = 0i32;
    let mut end = None;
    for (i, ch) in bytes {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let value = match end {
        Some(e) => &raw[start..start + e],
        None => &raw[start..],
    };
    // Only treat as a paren-attr if it actually contains parens (else the simple
    // kv map handles it and we avoid surprising precedence).
    if value.contains('(') {
        Some(value.to_string())
    } else {
        None
    }
}

// ── Components / instances (C8 / E4.2) ─────────────────────────────────

/// Parse a `component <name> [variants=[a,b]] [props=[n:t]]` block.
fn parse_component_block(line: &Line, body: &[Line], env: &Env) -> Result<Component> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "component requires a name"));
    }
    let name = line.tokens[1].clone();
    crate::types::validate_ident(&name)
        .map_err(|e| parse_err(line, &format!("invalid component name: {e}")))?;

    let variants = match extract_bracket_attr(&line.raw, "variants") {
        Some(v) => parse_ident_list(line, &v)?,
        None => Vec::new(),
    };
    let props = match extract_bracket_attr(&line.raw, "props") {
        Some(v) => parse_prop_list(line, &v)?,
        None => Vec::new(),
    };

    // A component body is a container body (its attrs/children). Container-level
    // fill/radius/opacity are not meaningful on a bare component, so reject them
    // to keep components purely structural; children carry their own styling.
    let (fill, radius, opacity, children) = parse_container_body(body, env)?;
    if fill.is_some() || radius.is_some() || opacity.is_some() {
        return Err(parse_err(
            line,
            "component-level fill/round-corners/opacity are not allowed; \
             put styling on a child frame",
        ));
    }

    Ok(Component {
        name,
        variants,
        props,
        children,
    })
}

/// Parse `instance <name> from=<component> [variant=v] [prop=value ...] [at=] [size=]`.
fn parse_instance_line(line: &Line, env: &Env) -> Result<Instance> {
    if line.tokens.len() < 2 {
        return Err(parse_err(line, "instance requires a name"));
    }
    let name = line.tokens[1].clone();
    crate::types::validate_ident(&name)
        .map_err(|e| parse_err(line, &format!("invalid instance name: {e}")))?;

    let attrs = parse_kv_attrs(&line.tokens[2..]);
    let component = attrs
        .get("from")
        .ok_or_else(|| parse_err(line, "instance requires from=<component>"))?
        .clone();
    let variant = attrs.get("variant").cloned();
    let position = attrs
        .get("at")
        .map(|v| eval_coord(line, v, env))
        .transpose()?;
    let size = attrs
        .get("size")
        .map(|v| eval_dimension(line, v, env))
        .transpose()?;

    // Remaining key=value tokens (minus the structural keys) are props, kept in
    // source order. Unquote string values so emit can requote consistently.
    let reserved = ["from", "variant", "at", "size"];
    let mut props = Vec::new();
    for token in &line.tokens[2..] {
        if let Some((k, v)) = token.split_once('=') {
            if reserved.contains(&k) {
                continue;
            }
            let value = if v.starts_with('"') {
                unquote_string_value(line, v)?
            } else {
                v.to_string()
            };
            props.push((k.to_string(), value));
        }
    }

    Ok(Instance {
        name,
        component,
        variant,
        props,
        position,
        size,
    })
}

/// Extract a `key=[...]` bracket list from the raw source line (the tokenizer
/// would split `[primary, ghost]` on the space).
fn extract_bracket_attr(raw: &str, key: &str) -> Option<String> {
    let needle = format!("{}=[", key);
    let start = raw.find(&needle)? + needle.len();
    let end = raw[start..].find(']')? + start;
    Some(raw[start..end].to_string())
}

fn parse_ident_list(line: &Line, raw: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if !is_token_ident(p) {
            return Err(parse_err(line, &format!("invalid variant name '{}'", p)));
        }
        out.push(p.to_string());
    }
    Ok(out)
}

fn parse_prop_list(line: &Line, raw: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let (n, ty) = p
            .split_once(':')
            .ok_or_else(|| parse_err(line, &format!("prop '{}' must be name:type", p)))?;
        let (n, ty) = (n.trim(), ty.trim());
        if !is_token_ident(n) || !is_token_ident(ty) {
            return Err(parse_err(line, &format!("invalid prop '{}'", p)));
        }
        out.push((n.to_string(), ty.to_string()));
    }
    Ok(out)
}

// ── Defaults parsing ─────────────────────────────────────────────────

fn parse_defaults_body(body: &[Line]) -> Result<Vec<Operation>> {
    let mut ops = Vec::new();
    for line in body {
        if line.tokens.is_empty() {
            continue;
        }
        match line.tokens[0].as_str() {
            "fill" => ops.push(parse_fill(line)?),
            "fill-rule" => ops.push(parse_fill_rule(line)?),
            "stroke" => ops.push(parse_stroke(line)?),
            "stroke-width" => ops.push(parse_stroke_width(line)?),
            "stroke-linecap" => ops.push(parse_stroke_linecap(line)?),
            "stroke-linejoin" => ops.push(parse_stroke_linejoin(line)?),
            "stroke-miterlimit" => ops.push(parse_stroke_miterlimit(line)?),
            "stroke-dasharray" => ops.push(parse_stroke_dasharray(line)?),
            "opacity" => ops.push(parse_opacity(line)?),
            "blur" => ops.push(parse_blur(line)?),
            "text-anchor" => ops.push(parse_text_anchor(line)?),
            _ => {
                return Err(StrokError::ParseError(format!(
                    "unexpected '{}' in defaults block (line {})\n\n\
Defaults accept attribute operations only:\n  \
fill  stroke  stroke-width  stroke-linecap  stroke-linejoin\n  \
stroke-dasharray  opacity  blur  text-anchor",
                    line.tokens[0], line.line_num
                )));
            }
        }
    }
    Ok(ops)
}

// ── KV attribute helpers ──────────────────────────────────────────────

/// Parse `key=value` pairs from a token slice into a map.
fn parse_kv_attrs(tokens: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for token in tokens {
        if let Some(eq_pos) = token.find('=') {
            let key = &token[..eq_pos];
            let value = &token[eq_pos + 1..];
            map.insert(key.to_string(), value.to_string());
        } else if token
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            // Bare identifier tokens are flags like "lock-endpoints"
            map.insert(token.clone(), String::new());
        }
        // Non-identifier bare tokens (like "15%", "0.3") are positional values
    }
    map
}

fn parse_kv_f64(attrs: &std::collections::HashMap<String, String>, key: &str) -> Result<f64> {
    attrs
        .get(key)
        .ok_or_else(|| StrokError::ParseError(format!("missing required attribute '{}'", key)))?
        .parse::<f64>()
        .map_err(|_| StrokError::ParseError(format!("invalid number for '{}'", key)))
}

fn parse_sculpt_target(s: &str) -> Result<SculptTarget> {
    // Could be a PointRef (shape.point), SegmentRef (shape.{p1,p2}), or coordinate (x,y)
    if s.contains('{') {
        Ok(SculptTarget::Segment(SegmentRef::parse(s)?))
    } else if s.contains('.') {
        Ok(SculptTarget::Point(PointRef::parse(s)?))
    } else {
        let (x, y) = parse_point_coord(s)?;
        Ok(SculptTarget::Coord(x, y))
    }
}

/// Find a positional (non key=value) token.
fn find_positional_token(
    tokens: &[String],
    kv: &std::collections::HashMap<String, String>,
) -> Option<String> {
    for token in tokens {
        if !token.contains('=') && !kv.contains_key(token) {
            return Some(token.clone());
        }
    }
    None
}

fn parse_err(line: &Line, msg: &str) -> StrokError {
    StrokError::ParseError(format!("{} (line {})", msg, line.line_num))
}

// ── Scalar expressions (C13) ──────────────────────────────────────────
//
// Every scalar on a scene-node / shape-op line routes through `eval_scalar`,
// which fast-paths a plain number (so existing files parse & re-emit
// byte-identically) and otherwise evaluates a space-free arithmetic expression
// against the `let`/`repeat` environment. The scene stores the resulting plain
// `f64` — expressions are evaluated eagerly at parse time and do not survive in
// the model (only `let` keeps its source, for round-trip).

/// Evaluate a scalar expression, attaching the line number to any error.
fn eval_scalar_l(line: &Line, s: &str, env: &Env) -> Result<f64> {
    eval_scalar(s, env).map_err(|e| match e {
        StrokError::ParseError(m) => parse_err(line, &m),
        other => other,
    })
}

/// Evaluate an `x,y` coordinate pair where each component may be an expression.
fn eval_coord(line: &Line, s: &str, env: &Env) -> Result<(f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(parse_err(
            line,
            &format!("expected PointCoord (x,y), got '{}'", s),
        ));
    }
    Ok((
        eval_scalar_l(line, parts[0], env)?,
        eval_scalar_l(line, parts[1], env)?,
    ))
}

/// Evaluate a `WxH` dimension where each component may be an expression. Plain
/// `WxH` fast-paths through [`Dimension::parse`] (byte-identical round-trip); an
/// expression form splits on `x`. **Caveat:** because the separator is the
/// literal `x`, a dimension expression must not reference a `$name` containing a
/// literal `x` — write `$w*2` rather than `$boxwidth` in a `size=` expression.
fn eval_dimension(line: &Line, s: &str, env: &Env) -> Result<Dimension> {
    if let Ok(d) = Dimension::parse(s) {
        return Ok(d);
    }
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        return Err(parse_err(
            line,
            &format!("expected Dimension (WxH), got '{}'", s),
        ));
    }
    Ok(Dimension {
        w: eval_scalar_l(line, parts[0], env)?,
        h: eval_scalar_l(line, parts[1], env)?,
    })
}

/// Evaluate a `rotation=` value: plain `N` / `Ndeg` fast-path through
/// [`Rotation::parse`]; anything else is a unitless expression (no `deg` suffix
/// on an expression — the suffix is only valid after a plain number).
fn eval_rotation(line: &Line, v: &str, env: &Env) -> Result<Rotation> {
    if let Ok(r) = Rotation::parse(v) {
        return Ok(r);
    }
    Ok(Rotation(eval_scalar_l(line, v, env)?))
}

// ── `let` bindings (C13) ──────────────────────────────────────────────

/// Parse a `let <name> <expr>` line into `(name, source, value)`. The name must
/// be a valid identifier and must not shadow a design-token / palette name; the
/// expression is evaluated against `env` (earlier lets are already bound).
fn parse_let_line(line: &Line, env: &Env, scene: &Scene) -> Result<(String, String, f64)> {
    if line.tokens.len() < 3 {
        return Err(parse_err(
            line,
            "let requires a name and an expression (e.g. `let col 310`)",
        ));
    }
    let name = line.tokens[1].clone();
    validate_ident(&name).map_err(|e| parse_err(line, &format!("invalid let name: {e}")))?;

    // A `let` must not shadow a design-token / palette entry name — a `$name`
    // would then be ambiguous between the numeric binding and the color token.
    if token_name_defined(scene, &name) {
        return Err(parse_err(
            line,
            &format!("let '{}' shadows a design token of the same name", name),
        ));
    }

    // Expressions are space-free; the tokenizer split on spaces, so re-join any
    // stray-spaced remainder into one source string.
    let source = line.tokens[2..].join("");
    let value = eval_scalar_l(line, &source, env)?;
    Ok((name, source, value))
}

/// True if `name` collides with a palette token or a generalized design token
/// (bare or dotted spelling).
fn token_name_defined(scene: &Scene, name: &str) -> bool {
    scene.palette.tokens.iter().any(|(n, _)| n == name)
        || scene
            .design_tokens
            .iter()
            .any(|t| t.name == name || t.dotted() == name)
}

// ── `repeat` blocks (C13) — parse-time macro expansion ────────────────
//
// `repeat <var> <count>` expands its body `count` times. Each iteration binds
// `$<var>` to the loop index and appends a `-<i>` suffix to every place/group/
// createlink NAME defined in the body (keeping names unique). Sibling references
// (`at=dot.center`, `clip=`, `textpath=`, `below=`/`above=`) to a name defined in
// the same body are rewritten to the suffixed name for that iteration. Nested
// repeats append their index after the outer one (`dot-0-1`). The Scene contains
// only the expanded nodes — a repeat block does not survive re-emit.

/// Expand a `repeat` header + body into concrete nodes (and any createlink
/// shapes), appending to `out_nodes` / `out_shapes`. `base_suffix` is the
/// accumulated suffix of any enclosing repeats (empty at the top level).
fn parse_repeat(
    header: &Line,
    body: &[Line],
    env: &Env,
    base_suffix: &str,
    out_nodes: &mut Vec<SceneNode>,
    out_shapes: &mut Vec<Shape>,
) -> Result<()> {
    if header.tokens.len() < 3 {
        return Err(parse_err(
            header,
            "repeat requires a variable and a count (e.g. `repeat i 4`)",
        ));
    }
    let var = header.tokens[1].clone();
    validate_ident(&var).map_err(|e| parse_err(header, &format!("invalid repeat var: {e}")))?;
    if env.contains(&var) {
        return Err(parse_err(
            header,
            &format!(
                "repeat variable '{}' shadows a let binding or an enclosing repeat variable",
                var
            ),
        ));
    }

    let count_src = header.tokens[2..].join("");
    let count_f = eval_scalar_l(header, &count_src, env)?;
    if !count_f.is_finite() || count_f < 0.0 || count_f.fract() != 0.0 {
        return Err(parse_err(
            header,
            &format!(
                "repeat count must be a non-negative integer, got {}",
                count_f
            ),
        ));
    }
    if count_f > 10000.0 {
        return Err(parse_err(
            header,
            &format!("repeat count {} exceeds the maximum of 10000", count_f),
        ));
    }
    let count = count_f as usize;

    // Names defined at this body's level (through groups, not nested repeats) —
    // the set of references eligible for per-iteration rewriting.
    let rename = collect_defined_names(body);

    for i in 0..count {
        let iter_env = env.child(var.clone(), i as f64);
        let suffix = format!("{}-{}", base_suffix, i);
        let mut context = ExpansionContext {
            env: &iter_env,
            suffix: &suffix,
            rename: &rename,
            out_nodes,
            out_shapes,
        };
        expand_nodes(body, &mut context)?;
    }
    Ok(())
}

/// Return one line, its indentation-defined body, and the index of the next
/// sibling. Keeping the body borrowed avoids cloning every nested [`Line`].
fn next_block(body: &[Line], index: usize) -> (&Line, &[Line], usize) {
    let line = &body[index];
    let mut next = index + 1;
    while next < body.len() && body[next].indent > line.indent {
        next += 1;
    }
    (line, &body[index + 1..next], next)
}

/// Collect the names defined by `place`/`group`/`createlink` at this repeat
/// scope: direct children plus those nested in groups. Nested `repeat` blocks
/// manage their own naming, so they are opaque here.
fn collect_defined_names(body: &[Line]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut i = 0;
    while i < body.len() {
        let (line, child_body, next) = next_block(body, i);
        match line.tokens[0].as_str() {
            "place" | "createlink" => {
                if let Some(name) = line.tokens.get(1) {
                    names.insert(name.clone());
                }
            }
            "group" => {
                if let Some(name) = line.tokens.get(1) {
                    names.insert(name.clone());
                }
                names.extend(collect_defined_names(child_body));
            }
            // `repeat` is opaque: its names are suffixed within its own scope.
            _ => {}
        }
        i = next;
    }
    names
}

struct ExpansionContext<'a> {
    env: &'a Env,
    suffix: &'a str,
    rename: &'a HashSet<String>,
    out_nodes: &'a mut Vec<SceneNode>,
    out_shapes: &'a mut Vec<Shape>,
}

/// Expand a body of scene-node lines for one repeat iteration.
fn expand_nodes(body: &[Line], context: &mut ExpansionContext<'_>) -> Result<()> {
    let mut i = 0;
    while i < body.len() {
        let (line, child_body, next) = next_block(body, i);
        expand_node(line, child_body, context)?;
        i = next;
    }
    Ok(())
}

/// Parse and rewrite one node in a repeat body.
fn expand_node(line: &Line, body: &[Line], context: &mut ExpansionContext<'_>) -> Result<()> {
    match line.tokens[0].as_str() {
        "place" => {
            let mut place = parse_place_line(line, body, context.env)?;
            suffix_place(&mut place, context.suffix, context.rename);
            context.out_nodes.push(SceneNode::Place(place));
        }
        "group" => {
            let mut group = parse_group_header(line, context.env)?;
            let mut children = Vec::new();
            let mut child_context = ExpansionContext {
                env: context.env,
                suffix: context.suffix,
                rename: context.rename,
                out_nodes: &mut children,
                out_shapes: context.out_shapes,
            };
            expand_nodes(body, &mut child_context)?;
            group.children = children;
            group.name = format!("{}{}", group.name, context.suffix);
            suffix_group_refs(&mut group, context.rename, context.suffix);
            context.out_nodes.push(SceneNode::Group(group));
        }
        "createlink" => {
            let (mut link, mut shape) = parse_link_line(line, body)?;
            let new_name = format!("{}{}", link.name, context.suffix);
            rewrite_ref(&mut link.source, context.rename, context.suffix);
            link.name = new_name.clone();
            shape.name = new_name;
            context.out_nodes.push(SceneNode::Link(link));
            context.out_shapes.push(shape);
        }
        "repeat" => parse_repeat(
            line,
            body,
            context.env,
            context.suffix,
            context.out_nodes,
            context.out_shapes,
        )?,
        keyword => {
            return Err(parse_err(
                line,
                &format!(
                    "unexpected '{}' in repeat body (allowed: place, group, createlink, repeat)",
                    keyword
                ),
            ));
        }
    }
    Ok(())
}

/// Append `suffix` to a place's own name and rewrite any sibling references
/// (`at=target`, `below=`/`above=`, `clip=`, `mask=`, `textpath=`) that match a
/// name defined in the same repeat body.
fn suffix_place(p: &mut Place, suffix: &str, rename: &HashSet<String>) {
    p.name = format!("{}{}", p.name, suffix);
    if let PlacePosition::RelativeTo { target, .. } = &mut p.position {
        rewrite_ref(target, rename, suffix);
    }
    if let Some(anchor) = &mut p.anchor {
        match anchor {
            PlaceAnchor::Below { target, .. } | PlaceAnchor::Above { target, .. } => {
                rewrite_ref(target, rename, suffix)
            }
        }
    }
    if let Some(clip) = &mut p.clip {
        for c in clip.iter_mut() {
            rewrite_ref(c, rename, suffix);
        }
    }
    if let Some(mask) = &mut p.mask {
        rewrite_ref(mask, rename, suffix);
    }
    if let Some(tp) = &mut p.text_path {
        rewrite_ref(tp, rename, suffix);
    }
}

/// Rewrite the group's own references (clip/mask) — its name and children are
/// handled by the caller.
fn suffix_group_refs(g: &mut Group, rename: &HashSet<String>, suffix: &str) {
    if let Some(clip) = &mut g.clip {
        for c in clip.iter_mut() {
            rewrite_ref(c, rename, suffix);
        }
    }
    if let Some(mask) = &mut g.mask {
        rewrite_ref(mask, rename, suffix);
    }
}

fn rewrite_ref(name: &mut String, rename: &HashSet<String>, suffix: &str) {
    if rename.contains(name.as_str()) {
        *name = format!("{}{}", name, suffix);
    }
}

/// Heuristic: does a token look like a bare `x,y` coordinate pair (no `=`)?
/// Used to suggest the keyed `at=x,y` form (E3.1).
fn looks_like_coord(tok: &str) -> bool {
    if tok.contains('=') {
        return false;
    }
    match tok.split_once(',') {
        Some((a, b)) => {
            !a.is_empty() && !b.is_empty() && a.parse::<f64>().is_ok() && b.parse::<f64>().is_ok()
        }
        None => false,
    }
}

/// Validate a shape reference: a bare identifier, or a namespaced `ns.name`
/// where both parts are valid identifiers (module import form, DSL_SPEC §Modules).
fn validate_shape_ref(line: &Line, r: &str) -> Result<()> {
    let parts: Vec<&str> = r.split('.').collect();
    if parts.len() > 2 {
        return Err(parse_err(line, &format!("invalid shape reference '{r}'")));
    }
    for part in parts {
        crate::types::validate_ident(part)
            .map_err(|e| parse_err(line, &format!("invalid shape reference '{r}': {e}")))?;
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "dsl_parse/tests.rs"]
mod tests;
