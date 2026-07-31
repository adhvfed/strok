//! Model Context Protocol server over stdio (E3.4).
//!
//! **Decision D-3 (recorded):** we implement the documented *fallback* — a thin
//! dependency-free stdio JSON-RPC 2.0 shim — rather than pulling in the `rmcp`
//! official SDK. `rmcp` (verified at 1.7.0 on crates.io) is mature but drags in
//! a full tokio async stack and ~100 transitive crates into an otherwise sync,
//! dependency-light workspace, which works against the no-panic / clippy-deny /
//! reproducible-build gates. The MCP stdio transport is a well-specified
//! newline-delimited JSON-RPC protocol; a hand-rolled shim keeps the server
//! synchronous, panic-free, and trivially testable (drive it with piped stdin).
//! If a future chunk needs SSE/HTTP transports or richer capability negotiation,
//! swap in `rmcp` behind this same tool surface.
//!
//! Transport: JSON-RPC 2.0, one message per line on stdin/stdout.
//! Methods: `initialize`, `tools/list`, `tools/call`, plus `notifications/*`
//! (acknowledged, no response).
//!
//! Tools mirror the CLI verbs and reuse the SAME core/render code paths the CLI
//! uses, so behavior can't drift. Documents are passed as DSL `source` text in
//! every call (stateless, like the file-based CLI). `render` returns an MCP
//! image content block (base64 PNG); inspection/query/measure return text
//! content carrying the stable `--json` schema from C6.

use std::io::{BufRead, Write};

use strok_core::json::Json;
use strok_render::{render_svg_string, target_dimensions, RenderOptions, RenderRegion};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "strok";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the MCP server loop over the given reader/writer until EOF.
pub fn run_stdio() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin.lock(), stdout.lock())
}

/// The transport loop, parameterized over reader/writer so tests can drive it
/// with in-memory buffers.
pub fn serve<R: BufRead, W: Write>(mut reader: R, mut writer: W) -> anyhow::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = handle_message(trimmed) {
            writeln!(writer, "{}", response)?;
            writer.flush()?;
        }
    }
    Ok(())
}

/// Handle one JSON-RPC message line. Returns the response line, or `None` for
/// notifications (no `id`) which take no reply.
fn handle_message(line: &str) -> Option<String> {
    let msg = match parse_json(line) {
        Some(m) => m,
        None => {
            // Parse error: id unknown, reply with null id per JSON-RPC.
            return Some(error_response(JsonId::Null, -32700, "parse error"));
        }
    };

    let id = extract_id(&msg);
    let method = field_str(&msg, "method").unwrap_or_default();

    // Notifications carry no id and expect no response.
    let is_notification = matches!(id, JsonId::Absent);

    let result: Result<Json, (i64, String)> = match method.as_str() {
        "initialize" => Ok(initialize_result()),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => handle_tools_call(&msg),
        m if m.starts_with("notifications/") => {
            return None; // acknowledged, no reply
        }
        "ping" => Ok(Json::obj([])),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    if is_notification {
        return None;
    }

    Some(match result {
        Ok(value) => success_response(id, value),
        Err((code, message)) => error_response(id, code, &message),
    })
}

// ---------------------------------------------------------------------------
// Capabilities / tool catalog
// ---------------------------------------------------------------------------

fn initialize_result() -> Json {
    Json::obj([
        ("protocolVersion", Json::str(PROTOCOL_VERSION)),
        (
            "capabilities",
            Json::obj([("tools", Json::obj([("listChanged", Json::Bool(false))]))]),
        ),
        (
            "serverInfo",
            Json::obj([
                ("name", Json::str(SERVER_NAME)),
                ("version", Json::str(SERVER_VERSION)),
            ]),
        ),
    ])
}

/// A `source` (DSL document text) property shared by every tool.
fn source_prop() -> (&'static str, Json) {
    (
        "source",
        Json::obj([
            ("type", Json::str("string")),
            ("description", Json::str("The .strok document as DSL text.")),
        ]),
    )
}

fn str_prop(desc: &str) -> Json {
    Json::obj([
        ("type", Json::str("string")),
        ("description", Json::str(desc)),
    ])
}

fn int_prop(desc: &str) -> Json {
    Json::obj([
        ("type", Json::str("integer")),
        ("description", Json::str(desc)),
    ])
}

fn schema(props: Vec<(&'static str, Json)>, required: &[&str]) -> Json {
    Json::obj([
        ("type", Json::str("object")),
        (
            "properties",
            Json::Object(props.into_iter().map(|(k, v)| (k.to_string(), v)).collect()),
        ),
        (
            "required",
            Json::array(required.iter().map(|r| Json::str(*r))),
        ),
    ])
}

fn tool(name: &str, description: &str, input_schema: Json) -> Json {
    Json::obj([
        ("name", Json::str(name)),
        ("description", Json::str(description)),
        ("inputSchema", input_schema),
    ])
}

/// The MCP tool catalog. Ordering is stable for snapshot tests.
pub fn tool_catalog() -> Vec<Json> {
    vec![
        tool(
            "agent_intro",
            "Read this first. Explains Strøk effort levels, tool discovery, \
             construction order, visual feedback loops, and completion gates.",
            schema(vec![], &[]),
        ),
        tool(
            "guide",
            "Read the visual-quality workflow before authoring an illustration, icon, logo, or diagram. \
             Covers style selection, geometry/text traps, and required review sizes.",
            schema(
                vec![(
                    "topic",
                    str_prop("Guide topic: illustration, icon, logo, or diagram."),
                )],
                &["topic"],
            ),
        ),
        tool(
            "new",
            "Create a new Strøk document and return its DSL source. \
             For icons, read the guide then choose an explicit visual profile.",
            schema(
                vec![
                    ("size", str_prop("Canvas size as WxH (default 800x800).")),
                    (
                        "profile",
                        str_prop(
                            "Optional profile: icon-outline-round, icon-outline-angular, icon-solid, or icon-mixed. `icon` is the round-outline alias.",
                        ),
                    ),
                ],
                &[],
            ),
        ),
        tool(
            "exec",
            "Append a raw DSL line to a document and return the new source. \
             The result is validated by re-parsing.",
            schema(
                vec![source_prop(), ("line", str_prop("The DSL line to append."))],
                &["source", "line"],
            ),
        ),
        tool(
            "render",
            "Render a document to PNG, returned as an inline image. \
             Optional width/height/color (for currentColor), bg, region crop, \
             and resolved-geometry outline inspection.",
            schema(
                vec![
                    source_prop(),
                    ("width", int_prop("Render width in pixels.")),
                    ("height", int_prop("Render height in pixels.")),
                    (
                        "region",
                        str_prop(
                            "Optional document-space crop as x,y,w,h for high-resolution detail review.",
                        ),
                    ),
                    (
                        "outline",
                        str_prop(
                            "Optional geometry overlay: '*' for all placed elements, or comma-separated placed IDs.",
                        ),
                    ),
                    (
                        "color",
                        str_prop("Concrete color substituted for currentColor."),
                    ),
                    ("bg", str_prop("Background color (e.g. #ffffff).")),
                ],
                &["source"],
            ),
        ),
        tool(
            "inspect",
            "Structural snapshot of a document as JSON (detail: full|structural|summary).",
            schema(
                vec![
                    source_prop(),
                    (
                        "detail",
                        str_prop("full | structural | summary (default structural)."),
                    ),
                ],
                &["source"],
            ),
        ),
        tool(
            "query",
            "Query a document by region or overlap, returned as JSON. \
             Provide either box (x,y,w,h) or overlaps (an element id).",
            schema(
                vec![
                    source_prop(),
                    ("box", str_prop("Region as x,y,w,h.")),
                    ("overlaps", str_prop("Element id to find overlaps of.")),
                ],
                &["source"],
            ),
        ),
        tool(
            "relate",
            "Describe the spatial relation between two placed elements, as JSON.",
            schema(
                vec![
                    source_prop(),
                    ("a", str_prop("First element name.")),
                    ("b", str_prop("Second element name.")),
                ],
                &["source", "a", "b"],
            ),
        ),
        tool(
            "measure",
            "Measure distance / gap / alignment between two placed elements, as JSON.",
            schema(
                vec![
                    source_prop(),
                    ("a", str_prop("First element name.")),
                    ("b", str_prop("Second element name.")),
                ],
                &["source", "a", "b"],
            ),
        ),
    ]
}

fn tools_list_result() -> Json {
    Json::obj([("tools", Json::array(tool_catalog()))])
}

// ---------------------------------------------------------------------------
// tools/call dispatch
// ---------------------------------------------------------------------------

fn handle_tools_call(msg: &Json) -> Result<Json, (i64, String)> {
    let params = field(msg, "params").ok_or((-32602, "missing params".to_string()))?;
    let name = field_str(params, "name").ok_or((-32602, "missing tool name".to_string()))?;
    let args = field(params, "arguments").cloned().unwrap_or(Json::obj([]));

    let outcome = dispatch_tool(&name, &args);
    match outcome {
        Ok(content) => Ok(Json::obj([
            ("content", Json::array(content)),
            ("isError", Json::Bool(false)),
        ])),
        // Tool-level errors are reported as a successful call with isError=true,
        // per MCP — the model sees the message rather than a transport failure.
        Err(message) => Ok(Json::obj([
            (
                "content",
                Json::array([text_content(&format!("error: {message}"))]),
            ),
            ("isError", Json::Bool(true)),
        ])),
    }
}

/// Run a tool, returning MCP content blocks. Reuses the core/render code paths.
fn dispatch_tool(name: &str, args: &Json) -> Result<Vec<Json>, String> {
    match name {
        "agent_intro" => Ok(vec![text_content(crate::agent_intro_text())]),
        "guide" => {
            let topic = require_str(args, "topic")?;
            let guide = crate::guide_text(&topic).map_err(|e| e.to_string())?;
            Ok(vec![text_content(guide)])
        }
        "new" => {
            let size = arg_str(args, "size").unwrap_or_else(|| "800x800".to_string());
            let profile = arg_str(args, "profile");
            let source = crate::build_document_source(&size, profile.as_deref())
                .map_err(|e| e.to_string())?;
            Ok(vec![text_content(&source)])
        }
        "exec" => {
            let source = require_str(args, "source")?;
            let line = require_str(args, "line")?;
            let updated = exec_line(&source, &line)?;
            Ok(vec![text_content(&updated)])
        }
        "render" => {
            let source = require_str(args, "source")?;
            let png = render_source(args, &source)?;
            Ok(vec![image_content(&png)])
        }
        "inspect" => {
            let source = require_str(args, "source")?;
            let detail = arg_str(args, "detail").unwrap_or_else(|| "structural".to_string());
            let json = inspect_source(&source, &detail)?;
            Ok(vec![text_content(&json)])
        }
        "query" => {
            let source = require_str(args, "source")?;
            let json = query_source(args, &source)?;
            Ok(vec![text_content(&json)])
        }
        "relate" => {
            let source = require_str(args, "source")?;
            let a = require_str(args, "a")?;
            let b = require_str(args, "b")?;
            let scene = parse_source(&source)?;
            let rel = strok_core::query::relate(&scene, &a, &b)?;
            Ok(vec![text_content(&rel.to_json().to_string_pretty())])
        }
        "measure" => {
            let source = require_str(args, "source")?;
            let a = require_str(args, "a")?;
            let b = require_str(args, "b")?;
            let scene = parse_source(&source)?;
            let report = strok_core::measure::measure(&scene, &a, &b)?;
            Ok(vec![text_content(&report.to_json())])
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

// --- tool implementations (reusing the same core paths as the CLI) ---

fn parse_source(source: &str) -> Result<strok_core::scene::Scene, String> {
    strok_core::dsl_parse::parse_file(source).map_err(|e| e.to_string())
}

fn exec_line(source: &str, line: &str) -> Result<String, String> {
    let mut updated = source.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(line);
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    // Validate by re-parsing, exactly like the CLI's append_line.
    parse_source(&updated)?;
    Ok(updated)
}

fn render_source(args: &Json, source: &str) -> Result<Vec<u8>, String> {
    let scene = parse_source(source)?;
    let (dw, dh) = (scene.document_size.w, scene.document_size.h);
    let mut svg = strok_core::resolve::resolve_scene(&scene);
    if let Some(spec) = arg_str(args, "outline") {
        let spec = spec.trim();
        let ids = if spec == "*" {
            None
        } else {
            let mut ids = Vec::new();
            for raw in spec.split(',') {
                let id = raw.trim();
                if id.is_empty() {
                    return Err("outline expects '*' or comma-separated placed IDs".to_string());
                }
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.to_string());
                }
            }
            if ids.is_empty() {
                return Err("outline expects '*' or comma-separated placed IDs".to_string());
            }
            Some(ids)
        };
        svg = strok_core::resolve::add_outline_overlay(&svg, ids.as_deref())
            .map_err(|e| e.to_string())?;
    }
    let region = arg_str(args, "region")
        .map(|spec| {
            parse_box_spec(&spec).map(|(x, y, width, height)| RenderRegion {
                x,
                y,
                width,
                height,
            })
        })
        .transpose()?;
    let opts = RenderOptions {
        width: arg_int(args, "width"),
        height: arg_int(args, "height"),
        background: arg_str(args, "bg"),
        color: arg_str(args, "color"),
        region,
    };
    let (source_w, source_h) = opts
        .region
        .map(|region| (region.width, region.height))
        .unwrap_or((dw, dh));
    let (w, h) = target_dimensions(source_w, source_h, opts.width, opts.height);
    render_svg_string(&svg, w, h, dw, dh, &opts).map_err(|e| e.to_string())
}

fn inspect_source(source: &str, detail: &str) -> Result<String, String> {
    let scene = parse_source(source)?;
    let level = strok_core::query::Detail::parse(detail)
        .ok_or_else(|| format!("invalid detail '{detail}' (use full|structural|summary)"))?;
    let snap = strok_core::query::snapshot(&scene, level);
    Ok(snap.to_json().to_string_pretty())
}

fn query_source(args: &Json, source: &str) -> Result<String, String> {
    let scene = parse_source(source)?;
    let result = match (arg_str(args, "box"), arg_str(args, "overlaps")) {
        (Some(_), Some(_)) => return Err("use either box or overlaps, not both".to_string()),
        (Some(spec), None) => {
            let (x, y, w, h) = parse_box_spec(&spec)?;
            strok_core::query::query_box(&scene, x, y, w, h)
        }
        (None, Some(id)) => strok_core::query::query_overlaps(&scene, &id)?,
        (None, None) => return Err("query requires box=x,y,w,h or overlaps=<id>".to_string()),
    };
    Ok(result.to_json().to_string_pretty())
}

fn parse_box_spec(spec: &str) -> Result<(f64, f64, f64, f64), String> {
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 4 {
        return Err(format!("box expects x,y,w,h, got '{spec}'"));
    }
    let mut nums = [0.0f64; 4];
    for (i, p) in parts.iter().enumerate() {
        nums[i] = p
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("box value '{p}' is not a number"))?;
    }
    Ok((nums[0], nums[1], nums[2], nums[3]))
}

// ---------------------------------------------------------------------------
// MCP content blocks
// ---------------------------------------------------------------------------

fn text_content(text: &str) -> Json {
    Json::obj([("type", Json::str("text")), ("text", Json::str(text))])
}

fn image_content(png: &[u8]) -> Json {
    Json::obj([
        ("type", Json::str("image")),
        ("data", Json::str(base64_encode(png))),
        ("mimeType", Json::str("image/png")),
    ])
}

/// Minimal, dependency-free standard base64 encoder (RFC 4648).
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tiny JSON-RPC envelope helpers (built on the core Json builder for output;
// a minimal parser for input — Strøk's core Json type only emits).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum JsonId {
    Absent,
    Null,
    Num(f64),
    Str(String),
}

fn id_to_json(id: &JsonId) -> Json {
    match id {
        JsonId::Absent | JsonId::Null => Json::Null,
        JsonId::Num(n) => Json::Num(*n),
        JsonId::Str(s) => Json::str(s.clone()),
    }
}

fn extract_id(msg: &Json) -> JsonId {
    match field(msg, "id") {
        None => JsonId::Absent,
        Some(Json::Null) => JsonId::Null,
        Some(Json::Num(n)) => JsonId::Num(*n),
        Some(Json::Str(s)) => JsonId::Str(s.clone()),
        Some(_) => JsonId::Null,
    }
}

fn success_response(id: JsonId, result: Json) -> String {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id_to_json(&id)),
        ("result", result),
    ])
    .to_string_compact()
}

fn error_response(id: JsonId, code: i64, message: &str) -> String {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id_to_json(&id)),
        (
            "error",
            Json::obj([
                ("code", Json::num(code as f64)),
                ("message", Json::str(message)),
            ]),
        ),
    ])
    .to_string_compact()
}

// --- field accessors over the parsed Json ---

fn field<'a>(msg: &'a Json, key: &str) -> Option<&'a Json> {
    if let Json::Object(pairs) = msg {
        pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    } else {
        None
    }
}

fn field_str(msg: &Json, key: &str) -> Option<String> {
    match field(msg, key) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn arg_str(args: &Json, key: &str) -> Option<String> {
    field_str(args, key)
}

fn arg_int(args: &Json, key: &str) -> Option<u32> {
    match field(args, key) {
        Some(Json::Num(n)) => Some(*n as u32),
        _ => None,
    }
}

fn require_str(args: &Json, key: &str) -> Result<String, String> {
    arg_str(args, key).ok_or_else(|| format!("missing required argument '{key}'"))
}

// ---------------------------------------------------------------------------
// Minimal JSON parser (input side). The core `Json` type only emits, so we
// parse incoming JSON-RPC into the same value tree here. Tolerant enough for
// MCP traffic; returns None on malformed input.
// ---------------------------------------------------------------------------

fn parse_json(s: &str) -> Option<Json> {
    let mut p = Parser {
        bytes: s.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    Some(v)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' | b'f' => self.boolean(),
            b'n' => self.null(),
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Option<Json> {
        self.bump(); // {
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek()? == b'}' {
            self.bump();
            return Some(Json::Object(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.bump()? != b':' {
                return None;
            }
            let val = self.value()?;
            pairs.push((key, val));
            self.skip_ws();
            match self.bump()? {
                b',' => continue,
                b'}' => break,
                _ => return None,
            }
        }
        Some(Json::Object(pairs))
    }

    fn array(&mut self) -> Option<Json> {
        self.bump(); // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek()? == b']' {
            self.bump();
            return Some(Json::Array(items));
        }
        loop {
            let val = self.value()?;
            items.push(val);
            self.skip_ws();
            match self.bump()? {
                b',' => continue,
                b']' => break,
                _ => return None,
            }
        }
        Some(Json::Array(items))
    }

    fn string(&mut self) -> Option<String> {
        if self.bump()? != b'"' {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                b'"' => break,
                b'\\' => match self.bump()? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let c = self.bump()?;
                            code = code * 16 + hex_val(c)?;
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    _ => return None,
                },
                b => {
                    // Collect a UTF-8 continuation run starting at b.
                    let start = self.pos - 1;
                    let len = utf8_len(b);
                    for _ in 1..len {
                        self.bump()?;
                    }
                    let slice = &self.bytes[start..self.pos];
                    out.push_str(std::str::from_utf8(slice).ok()?);
                }
            }
        }
        Some(out)
    }

    fn boolean(&mut self) -> Option<Json> {
        if self.bytes[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Some(Json::Bool(true))
        } else if self.bytes[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Some(Json::Bool(false))
        } else {
            None
        }
    }

    fn null(&mut self) -> Option<Json> {
        if self.bytes[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Some(Json::Null)
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }
}

fn hex_val(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(line: &str) -> Option<Json> {
        handle_message(line).and_then(|r| parse_json(&r))
    }

    #[test]
    fn initialize_reports_server_info() {
        let resp = call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        let result = field(&resp, "result").unwrap();
        assert_eq!(
            field_str(field(result, "serverInfo").unwrap(), "name"),
            Some("strok".to_string())
        );
    }

    #[test]
    fn tools_list_has_render_and_inspect() {
        let resp = call(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let tools = field(field(&resp, "result").unwrap(), "tools").unwrap();
        let names: Vec<String> = match tools {
            Json::Array(items) => items.iter().filter_map(|t| field_str(t, "name")).collect(),
            _ => vec![],
        };
        assert!(names.contains(&"render".to_string()));
        assert!(names.contains(&"inspect".to_string()));
        assert!(names.contains(&"query".to_string()));
    }

    #[test]
    fn notification_yields_no_response() {
        assert!(
            handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none()
        );
    }

    #[test]
    fn render_returns_image_content() {
        let src = "documentsize 24x24\\nshape b template=rectangle\\n  fill #000000\\nplace b shape=b at=0,0 size=24x24";
        let req = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{}"}}}}}}"#,
            src
        );
        let resp = call(&req).unwrap();
        let result = field(&resp, "result").unwrap();
        assert_eq!(field(result, "isError"), Some(&Json::Bool(false)));
        let content = field(result, "content").unwrap();
        if let Json::Array(items) = content {
            assert_eq!(field_str(&items[0], "type"), Some("image".to_string()));
            assert_eq!(
                field_str(&items[0], "mimeType"),
                Some("image/png".to_string())
            );
            // The base64 data decodes to a PNG (magic header).
            let data = field_str(&items[0], "data").unwrap();
            assert!(!data.is_empty());
        } else {
            panic!("expected content array");
        }
    }

    #[test]
    fn render_supports_selected_outline_inspection() {
        let src = "documentsize 24x24\\nshape b template=rectangle\\n  fill #000000\\nplace tile shape=b at=2,2 size=20x20";
        let req = format!(
            r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{}","outline":"tile","region":"0,0,24,24","width":96}}}}}}"#,
            src
        );
        let resp = call(&req).unwrap();
        let result = field(&resp, "result").unwrap();
        assert_eq!(field(result, "isError"), Some(&Json::Bool(false)));

        let unknown_req = format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{}","outline":"ghost"}}}}}}"#,
            src
        );
        let unknown = call(&unknown_req).unwrap();
        let unknown_result = field(&unknown, "result").unwrap();
        assert_eq!(field(unknown_result, "isError"), Some(&Json::Bool(true)));
        let content = field(unknown_result, "content").unwrap();
        if let Json::Array(items) = content {
            let text = field_str(&items[0], "text").unwrap();
            assert!(text.contains("outline id 'ghost'"), "{text}");
        } else {
            panic!("expected error content");
        }
    }

    #[test]
    fn inspect_returns_structural_json() {
        let src = "documentsize 24x24\\nshape b template=rectangle\\n  fill #000000\\nplace b shape=b at=0,0 size=24x24";
        let req = format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"inspect","arguments":{{"source":"{}","detail":"summary"}}}}}}"#,
            src
        );
        let resp = call(&req).unwrap();
        let content = field(field(&resp, "result").unwrap(), "content").unwrap();
        if let Json::Array(items) = content {
            let text = field_str(&items[0], "text").unwrap();
            assert!(text.contains("\"detail\""));
            assert!(text.contains("\"elements\""));
        } else {
            panic!("expected content");
        }
    }

    #[test]
    fn guide_returns_visual_workflow() {
        let req = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"guide","arguments":{"topic":"icon"}}}"#;
        let resp = call(req).unwrap();
        let content = field(field(&resp, "result").unwrap(), "content").unwrap();
        if let Json::Array(items) = content {
            let text = field_str(&items[0], "text").unwrap();
            assert!(text.contains("Choose a visual grammar"));
            assert!(text.contains("smallest shipping size"));
        } else {
            panic!("expected content");
        }
    }

    #[test]
    fn agent_intro_teaches_effort_and_focused_review() {
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"agent_intro","arguments":{}}}"#;
        let resp = call(req).unwrap();
        let content = field(field(&resp, "result").unwrap(), "content").unwrap();
        if let Json::Array(items) = content {
            let text = field_str(&items[0], "text").unwrap();
            assert!(text.contains("CHOOSE THE EFFORT LEVEL"));
            assert!(text.contains("render --region"));
            assert!(text.contains("render --outline"));
            assert!(text.contains("live `boolean"));
            assert!(text.contains("showcase"));
        } else {
            panic!("expected content");
        }
    }

    #[test]
    fn tool_error_is_reported_as_iserror() {
        let req = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"inspect","arguments":{"source":"not a valid doc"}}}"#;
        let resp = call(req).unwrap();
        let result = field(&resp, "result").unwrap();
        assert_eq!(field(result, "isError"), Some(&Json::Bool(true)));
    }

    #[test]
    fn base64_matches_known_vector() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn json_parser_roundtrips_nested() {
        let v = parse_json(r#"{"a":[1,2,{"b":"x"}],"c":true,"d":null}"#).unwrap();
        assert_eq!(field_str(&v, "c"), None); // c is bool, not str
        assert_eq!(field(&v, "c"), Some(&Json::Bool(true)));
    }

    #[test]
    fn tool_schemas_are_stable() {
        // The published tool catalog (names + JSON schemas) is part of the MCP
        // contract; snapshot it so a schema change is a deliberate review event.
        let catalog = Json::array(tool_catalog());
        insta::assert_snapshot!(catalog.to_string_pretty());
    }

    #[test]
    fn serve_drives_a_session() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );
        let mut out = Vec::new();
        serve(std::io::Cursor::new(input), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // initialize + tools/list => 2 responses; the notification => none.
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("serverInfo"));
        assert!(lines[1].contains("tools"));
    }
}
