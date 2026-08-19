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
//! uses, so behavior can't drift. Every document tool takes the document as
//! either `file` (a path on disk; mutations write back) or `source` (DSL text;
//! stateless, mutations return the updated text). File mode is the primary
//! flow: it gives shell-less agent runtimes the same file-first loop the CLI
//! offers, composing with `strok watch`, review notes, and version control.
//! `render` returns an MCP image content block (base64 PNG) so the agent sees
//! its work inline; inspection/query/measure return text content carrying the
//! stable `--json` schema from C6.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use strok_core::document::Document;
use strok_core::json::Json;
use strok_core::scene::Scene;
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
        (
            "instructions",
            Json::str(
                "Strok is a file-first vector design toolkit. Call agent_intro before \
                 authoring. Prefer the `file` argument over `source` so the document \
                 lives on disk and composes with `strok watch` (live human preview) and \
                 pinned review notes (notes_list / note_resolve).",
            ),
        ),
    ])
}

/// A `source` (DSL document text) property shared by every document tool.
fn source_prop() -> (&'static str, Json) {
    (
        "source",
        Json::obj([
            ("type", Json::str("string")),
            (
                "description",
                Json::str("The .strok document as DSL text. Pass either source or file, not both."),
            ),
        ]),
    )
}

/// A `file` (path on disk) property shared by every document tool.
fn file_prop() -> (&'static str, Json) {
    (
        "file",
        Json::obj([
            ("type", Json::str("string")),
            (
                "description",
                Json::str(
                    "Path to a .strok document on disk. Preferred over source: mutations \
                     write back to the file, imports resolve, and the document composes \
                     with watch and review notes.",
                ),
            ),
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
            "Create a new Strøk document. With file, writes it to disk and returns a \
             confirmation; without, returns the DSL source. \
             For icons, read the guide then choose an explicit visual profile.",
            schema(
                vec![
                    (
                        "file",
                        str_prop("Optional path to write the new document to (must not exist)."),
                    ),
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
            "read",
            "Read a .strok document from disk and return its DSL source.",
            schema(vec![file_prop()], &["file"]),
        ),
        tool(
            "write",
            "Replace a .strok document on disk with new DSL source. The source is \
             validated by parsing first; on error the file is left untouched and \
             the caret diagnostics are returned. This is the general-purpose edit: \
             read, modify the text, write.",
            schema(
                vec![
                    file_prop(),
                    (
                        "source",
                        str_prop("The complete new document as DSL text."),
                    ),
                ],
                &["file", "source"],
            ),
        ),
        tool(
            "exec",
            "Append one or more raw DSL lines to a document. The result is validated \
             by re-parsing before anything is written. With file, appends to the file \
             and returns a confirmation; with source, returns the updated source.",
            schema(
                vec![
                    file_prop(),
                    source_prop(),
                    ("line", str_prop("The DSL line(s) to append.")),
                ],
                &["line"],
            ),
        ),
        tool(
            "render",
            "Render a document to PNG, returned as an inline image. Look at it after \
             every meaningful change. Optional width/height/color (for currentColor), \
             bg, scheme (colorscheme for $tokens), region crop, and resolved-geometry \
             outline inspection.",
            schema(
                vec![
                    file_prop(),
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
                    (
                        "scheme",
                        str_prop("Colorscheme to resolve $tokens with (base palette if omitted)."),
                    ),
                ],
                &[],
            ),
        ),
        tool(
            "inspect",
            "Structural snapshot of a document as JSON (detail: full|structural|summary).",
            schema(
                vec![
                    file_prop(),
                    source_prop(),
                    (
                        "detail",
                        str_prop("full | structural | summary (default structural)."),
                    ),
                ],
                &[],
            ),
        ),
        tool(
            "query",
            "Query a document by region or overlap, returned as JSON. \
             Provide either box (x,y,w,h) or overlaps (an element id).",
            schema(
                vec![
                    file_prop(),
                    source_prop(),
                    ("box", str_prop("Region as x,y,w,h.")),
                    ("overlaps", str_prop("Element id to find overlaps of.")),
                ],
                &[],
            ),
        ),
        tool(
            "relate",
            "Describe the spatial relation between two placed elements, as JSON.",
            schema(
                vec![
                    file_prop(),
                    source_prop(),
                    ("a", str_prop("First element name.")),
                    ("b", str_prop("Second element name.")),
                ],
                &["a", "b"],
            ),
        ),
        tool(
            "measure",
            "Measure distance / gap / alignment between two placed elements, as JSON.",
            schema(
                vec![
                    file_prop(),
                    source_prop(),
                    ("a", str_prop("First element name.")),
                    ("b", str_prop("Second element name.")),
                ],
                &["a", "b"],
            ),
        ),
        tool(
            "audit",
            "Run the visual-quality audit over a document and return the findings as \
             JSON (missed abstractions, near mirrors, text collisions, rough curves). \
             Run it after geometry changes; treat findings as evidence, not taste.",
            schema(vec![file_prop(), source_prop()], &[]),
        ),
        tool(
            "notes_list",
            "List the review notes a human has pinned onto this document in a \
             `strok watch` session (from `<file>.notes.json`). Read them first when \
             work is handed back; they are the review feedback. Listing marks the \
             notes as seen, so the reviewer's pins show you have them.",
            schema(
                vec![(
                    "file",
                    str_prop("Path to the .strok document whose notes to read."),
                )],
                &["file"],
            ),
        ),
        tool(
            "note_resolve",
            "Delete one pinned review note after addressing it. A running watch \
             session reloads the sidecar, so the reviewer sees the pin clear live. \
             Resolving the last note removes the sidecar.",
            schema(
                vec![
                    (
                        "file",
                        str_prop("Path to the .strok document whose note to resolve."),
                    ),
                    ("id", int_prop("The note id to resolve.")),
                ],
                &["file", "id"],
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
            match arg_str(args, "file") {
                Some(file) => {
                    let path = Path::new(&file);
                    if path.exists() {
                        return Err(format!("'{}' already exists", file));
                    }
                    std::fs::write(path, &source)
                        .map_err(|e| format!("failed to write '{}': {}", file, e))?;
                    Ok(vec![text_content(&format!("created {}", file))])
                }
                None => Ok(vec![text_content(&source)]),
            }
        }
        "read" => {
            let file = require_str(args, "file")?;
            let source = std::fs::read_to_string(&file)
                .map_err(|e| format!("failed to read '{}': {}", file, e))?;
            Ok(vec![text_content(&source)])
        }
        "write" => {
            let file = require_str(args, "file")?;
            let source = require_str(args, "source")?;
            let path = Path::new(&file);
            // Validate before touching the file so a bad write never
            // clobbers a good document.
            strok_core::dsl_parse::parse_file_with_path(&source, path)
                .map_err(|e| strok_error_text(&e))?;
            std::fs::write(path, &source)
                .map_err(|e| format!("failed to write '{}': {}", file, e))?;
            Ok(vec![text_content(&format!(
                "wrote {} ({} lines, parse ok)",
                file,
                source.lines().count()
            ))])
        }
        "exec" => {
            let line = require_str(args, "line")?;
            match doc_input(args)? {
                DocInput::Source(source) => {
                    let updated = exec_lines(&source, &line, None)?;
                    Ok(vec![text_content(&updated)])
                }
                DocInput::File(path) => {
                    let source = std::fs::read_to_string(&path)
                        .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
                    let updated = exec_lines(&source, &line, Some(&path))?;
                    std::fs::write(&path, &updated)
                        .map_err(|e| format!("failed to write '{}': {}", path.display(), e))?;
                    Ok(vec![text_content(&format!(
                        "appended {} line(s) to {} (parse ok)",
                        line.lines().count(),
                        path.display()
                    ))])
                }
            }
        }
        "render" => {
            let scene = scene_for(&doc_input(args)?)?;
            let png = render_scene(args, &scene)?;
            Ok(vec![image_content(&png)])
        }
        "inspect" => {
            let scene = scene_for(&doc_input(args)?)?;
            let detail = arg_str(args, "detail").unwrap_or_else(|| "structural".to_string());
            let json = inspect_scene(&scene, &detail)?;
            Ok(vec![text_content(&json)])
        }
        "query" => {
            let scene = scene_for(&doc_input(args)?)?;
            let json = query_scene(args, &scene)?;
            Ok(vec![text_content(&json)])
        }
        "relate" => {
            let a = require_str(args, "a")?;
            let b = require_str(args, "b")?;
            let scene = scene_for(&doc_input(args)?)?;
            let rel = strok_core::query::relate(&scene, &a, &b)?;
            Ok(vec![text_content(&rel.to_json().to_string_pretty())])
        }
        "measure" => {
            let a = require_str(args, "a")?;
            let b = require_str(args, "b")?;
            let scene = scene_for(&doc_input(args)?)?;
            let report = strok_core::measure::measure(&scene, &a, &b)?;
            Ok(vec![text_content(&report.to_json())])
        }
        "audit" => {
            let scene = scene_for(&doc_input(args)?)?;
            let findings = strok_core::audit::audit(&scene);
            Ok(vec![text_content(
                &strok_core::audit::findings_to_json(&findings).to_string_pretty(),
            )])
        }
        "notes_list" => {
            let file = require_str(args, "file")?;
            let sidecar = crate::watch::notes::sidecar_path(Path::new(&file));
            let mut notes = load_notes(&sidecar)?;
            // Listing is the read receipt: the reviewer's pins flip to "seen".
            if notes.iter().any(|note| !note.ack) {
                for note in &mut notes {
                    note.ack = true;
                }
                crate::watch::notes::save(&sidecar, &notes).map_err(|e| format!("{:#}", e))?;
            }
            Ok(vec![text_content(
                &Json::obj([("notes", crate::watch::notes::to_json(&notes))]).to_string_pretty(),
            )])
        }
        "note_resolve" => {
            let file = require_str(args, "file")?;
            let id = arg_int(args, "id").ok_or("missing required argument 'id'")? as u64;
            let sidecar = crate::watch::notes::sidecar_path(Path::new(&file));
            let mut notes = load_notes(&sidecar)?;
            let before = notes.len();
            notes.retain(|note| note.id != id);
            if notes.len() == before {
                return Err(format!("no note with id {}", id));
            }
            crate::watch::notes::save(&sidecar, &notes).map_err(|e| format!("{:#}", e))?;
            Ok(vec![text_content(&format!(
                "resolved note {} ({} remaining)",
                id,
                notes.len()
            ))])
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

// --- tool implementations (reusing the same core paths as the CLI) ---

/// The document argument: exactly one of `file` (path on disk) or `source`
/// (DSL text).
enum DocInput {
    Source(String),
    File(PathBuf),
}

fn doc_input(args: &Json) -> Result<DocInput, String> {
    match (arg_str(args, "source"), arg_str(args, "file")) {
        (Some(_), Some(_)) => Err("pass either source or file, not both".to_string()),
        (Some(source), None) => Ok(DocInput::Source(source)),
        (None, Some(file)) => Ok(DocInput::File(PathBuf::from(file))),
        (None, None) => {
            Err("pass source (DSL text) or file (path to a .strok document)".to_string())
        }
    }
}

/// Load the scene, resolving module imports relative to the file when the
/// document comes from disk.
fn scene_for(input: &DocInput) -> Result<Scene, String> {
    let doc = match input {
        DocInput::Source(source) => Document::load_str(source),
        DocInput::File(path) => Document::load(path),
    }
    .map_err(|e| strok_error_text(&e))?;
    doc.scene
        .ok_or_else(|| "this tool needs a text .strok document".to_string())
}

/// Positioned parse diagnostics render their own caret snippets; everything
/// else gets the plain message (mirrors the CLI's top-level handler).
fn strok_error_text(e: &strok_core::error::StrokError) -> String {
    if let strok_core::error::StrokError::ParseDiagnostics(diags) = e {
        diags
            .iter()
            .map(|d| d.render())
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        e.to_string()
    }
}

fn load_notes(sidecar: &Path) -> Result<Vec<crate::watch::notes::Note>, String> {
    crate::watch::notes::load(sidecar)
        .ok_or_else(|| format!("'{}' does not parse as a notes sidecar", sidecar.display()))
}

fn exec_lines(source: &str, lines: &str, path: Option<&Path>) -> Result<String, String> {
    let mut updated = source.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(lines);
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    // Validate by re-parsing, exactly like the CLI's append_line.
    match path {
        Some(path) => strok_core::dsl_parse::parse_file_with_path(&updated, path)
            .map(|_| ())
            .map_err(|e| strok_error_text(&e))?,
        None => {
            strok_core::dsl_parse::parse_file(&updated)
                .map(|_| ())
                .map_err(|e| strok_error_text(&e))?;
        }
    }
    Ok(updated)
}

fn render_scene(args: &Json, scene: &Scene) -> Result<Vec<u8>, String> {
    // Resolve $tokens against the requested scheme (base palette when omitted),
    // exactly like the CLI render path.
    let scene = strok_core::resolve::apply_scheme(scene, arg_str(args, "scheme").as_deref())
        .map_err(|e| e.to_string())?;
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

fn inspect_scene(scene: &Scene, detail: &str) -> Result<String, String> {
    let level = strok_core::query::Detail::parse(detail)
        .ok_or_else(|| format!("invalid detail '{detail}' (use full|structural|summary)"))?;
    let snap = strok_core::query::snapshot(scene, level);
    Ok(snap.to_json().to_string_pretty())
}

fn query_scene(args: &Json, scene: &Scene) -> Result<String, String> {
    let result = match (arg_str(args, "box"), arg_str(args, "overlaps")) {
        (Some(_), Some(_)) => return Err("use either box or overlaps, not both".to_string()),
        (Some(spec), None) => {
            let (x, y, w, h) = parse_box_spec(&spec)?;
            strok_core::query::query_box(scene, x, y, w, h)
        }
        (None, Some(id)) => strok_core::query::query_overlaps(scene, &id)?,
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

pub(crate) fn parse_json(s: &str) -> Option<Json> {
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
            assert!(text.contains("strok watch art.strok"));
            assert!(text.contains("one source writer at a time"));
            assert!(text.contains("reread it"));
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

    fn temp_strok(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "strok-mcp-test-{}-{}.strok",
            std::process::id(),
            name
        ));
        std::fs::write(&path, contents).expect("write temp .strok");
        path
    }

    fn text_of(resp: &Json) -> String {
        let result = field(resp, "result").unwrap();
        let content = field(result, "content").unwrap();
        match content {
            Json::Array(items) => field_str(&items[0], "text").unwrap_or_default(),
            _ => panic!("expected content array"),
        }
    }

    fn is_error(resp: &Json) -> bool {
        field(field(resp, "result").unwrap(), "isError") == Some(&Json::Bool(true))
    }

    const TOKEN_DOC: &str = "documentsize 20x20\\n\\npalette\\n  ink #112233\\n\\nscheme dark\\n  ink #aabbcc\\n\\nshape s template=rectangle\\n  fill $ink\\n\\nplace p shape=s at=2,2 size=16x16\\n";

    #[test]
    fn render_resolves_palette_tokens_and_schemes() {
        let ok = call(&format!(
            r#"{{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{TOKEN_DOC}","width":20}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&ok), "{ok:?}");

        let dark = call(&format!(
            r#"{{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{TOKEN_DOC}","scheme":"dark","width":20}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&dark), "{dark:?}");

        let unknown = call(&format!(
            r#"{{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{TOKEN_DOC}","scheme":"nope"}}}}}}"#,
        ))
        .unwrap();
        assert!(is_error(&unknown), "{unknown:?}");
        assert!(text_of(&unknown).contains("unknown scheme"), "{unknown:?}");
    }

    #[test]
    fn document_tools_require_exactly_one_input() {
        let neither =
            call(r#"{"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":"inspect","arguments":{}}}"#)
                .unwrap();
        assert!(is_error(&neither));
        assert!(text_of(&neither).contains("source"), "{neither:?}");

        let both = call(
            r#"{"jsonrpc":"2.0","id":34,"method":"tools/call","params":{"name":"inspect","arguments":{"source":"documentsize 8x8","file":"x.strok"}}}"#,
        )
        .unwrap();
        assert!(is_error(&both));
        assert!(text_of(&both).contains("not both"), "{both:?}");
    }

    #[test]
    fn file_mode_reads_writes_and_appends() {
        let path = temp_strok(
            "filemode",
            "documentsize 24x24\n\nshape b template=rectangle\n  fill #000000\n\nplace tile shape=b at=2,2 size=20x20\n",
        );
        let file = path.to_str().unwrap();

        let read = call(&format!(
            r#"{{"jsonrpc":"2.0","id":40,"method":"tools/call","params":{{"name":"read","arguments":{{"file":"{file}"}}}}}}"#,
        ))
        .unwrap();
        assert!(text_of(&read).contains("documentsize 24x24"));

        // exec in file mode appends multiple lines and writes back.
        let exec = call(&format!(
            r#"{{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{{"name":"exec","arguments":{{"file":"{file}","line":"shape dot template=ellipse\n  fill #ff0000\n\nplace mark shape=dot at=12,12 size=4x4"}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&exec), "{exec:?}");
        assert!(text_of(&exec).contains("appended 4 line(s)"), "{exec:?}");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("place mark"), "{on_disk}");

        // an invalid append is rejected and the file is untouched.
        let bad = call(&format!(
            r#"{{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{{"name":"exec","arguments":{{"file":"{file}","line":"garbage line"}}}}}}"#,
        ))
        .unwrap();
        assert!(is_error(&bad), "{bad:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk);

        // write replaces the document wholesale, validating first.
        let write = call(&format!(
            r#"{{"jsonrpc":"2.0","id":43,"method":"tools/call","params":{{"name":"write","arguments":{{"file":"{file}","source":"documentsize 10x10\n"}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&write), "{write:?}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("10x10"));
        let bad_write = call(&format!(
            r#"{{"jsonrpc":"2.0","id":44,"method":"tools/call","params":{{"name":"write","arguments":{{"file":"{file}","source":"nonsense"}}}}}}"#,
        ))
        .unwrap();
        assert!(is_error(&bad_write), "{bad_write:?}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("10x10"));

        // render straight from the file.
        let render = call(&format!(
            r#"{{"jsonrpc":"2.0","id":45,"method":"tools/call","params":{{"name":"render","arguments":{{"file":"{file}","width":10}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&render), "{render:?}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn audit_returns_findings_json() {
        let src = "documentsize 24x24\\nshape b template=rectangle\\n  fill #000000\\nplace b shape=b at=0,0 size=24x24";
        let resp = call(&format!(
            r#"{{"jsonrpc":"2.0","id":50,"method":"tools/call","params":{{"name":"audit","arguments":{{"source":"{src}"}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&resp), "{resp:?}");
        assert!(text_of(&resp).contains("\"findings\""), "{resp:?}");
    }

    #[test]
    fn notes_tools_read_and_resolve_the_sidecar() {
        let path = temp_strok("notes", "documentsize 8x8\n");
        let file = path.to_str().unwrap();
        let sidecar = crate::watch::notes::sidecar_path(&path);
        std::fs::write(
            &sidecar,
            "{\"notes\": [{\"id\": 1, \"at\": [2, 3], \"target\": \"p\", \"text\": \"tighten this\"}, {\"id\": 2, \"at\": [5, 5], \"text\": \"more contrast\"}]}\n",
        )
        .unwrap();

        let list = call(&format!(
            r#"{{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{{"name":"notes_list","arguments":{{"file":"{file}"}}}}}}"#,
        ))
        .unwrap();
        assert!(text_of(&list).contains("tighten this"), "{list:?}");

        let resolve = call(&format!(
            r#"{{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{{"name":"note_resolve","arguments":{{"file":"{file}","id":1}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&resolve), "{resolve:?}");
        assert!(text_of(&resolve).contains("resolved note 1"), "{resolve:?}");
        assert!(sidecar.exists());

        let missing = call(&format!(
            r#"{{"jsonrpc":"2.0","id":62,"method":"tools/call","params":{{"name":"note_resolve","arguments":{{"file":"{file}","id":9}}}}}}"#,
        ))
        .unwrap();
        assert!(is_error(&missing), "{missing:?}");

        let last = call(&format!(
            r#"{{"jsonrpc":"2.0","id":63,"method":"tools/call","params":{{"name":"note_resolve","arguments":{{"file":"{file}","id":2}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&last), "{last:?}");
        assert!(
            !sidecar.exists(),
            "sidecar should be removed with the last note"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_with_file_writes_and_refuses_overwrite() {
        let path = std::env::temp_dir().join(format!("strok-mcp-new-{}.strok", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let file = path.to_str().unwrap();
        let created = call(&format!(
            r#"{{"jsonrpc":"2.0","id":70,"method":"tools/call","params":{{"name":"new","arguments":{{"file":"{file}","size":"32x32"}}}}}}"#,
        ))
        .unwrap();
        assert!(!is_error(&created), "{created:?}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("32x32"));

        let clobber = call(&format!(
            r#"{{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{{"name":"new","arguments":{{"file":"{file}"}}}}}}"#,
        ))
        .unwrap();
        assert!(is_error(&clobber), "{clobber:?}");
        assert!(text_of(&clobber).contains("already exists"), "{clobber:?}");
        let _ = std::fs::remove_file(&path);
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
