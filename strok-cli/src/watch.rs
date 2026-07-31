//! `strok watch` — live preview server for humans working in an editor.
//!
//! Strøk is file-first: every CLI command reads the document and exits. Watch
//! mode is the one long-running exception, built for a person editing `.strok`
//! source by hand who wants to see the render update as they save. It serves a
//! small local page that re-renders on every file change over Server-Sent
//! Events. Plain std only — no async runtime, no watcher crate (mtime+content
//! polling survives editors that save via rename), no web framework.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use strok_core::document::Document;
use strok_core::dsl_emit;
use strok_core::dsl_parse;
use strok_core::emit;
use strok_core::json::Json;
use strok_core::path_point::{path_data_to_svg_d, CurveMode, PathData};
use strok_core::resolve;
use strok_core::shape::{Operation, Shape, Template};
use strok_core::types::PointMode;

/// How often the file is polled for changes.
const POLL_INTERVAL: Duration = Duration::from_millis(150);
/// SSE keep-alive comment interval; also bounds how fast dead clients are reaped.
const SSE_PING_INTERVAL: Duration = Duration::from_secs(15);

struct State {
    version: u64,
    /// Last successful render. Kept through parse errors so the preview never
    /// goes blank while the file is mid-edit.
    svg: Option<String>,
    /// Canonical shape geometry used by the browser-side point editor.
    editor: Json,
    error: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    /// Serialize browser edits so two tabs cannot interleave file rewrites.
    edit: Mutex<EditHistory>,
    changed: Condvar,
}

#[derive(Clone)]
struct FileEdit {
    before: Vec<u8>,
    after: Vec<u8>,
}

#[derive(Default)]
struct EditHistory {
    undo: Vec<FileEdit>,
    redo: Vec<FileEdit>,
}

const HISTORY_LIMIT: usize = 100;

struct Snapshot {
    svg: String,
    editor: Json,
}

pub fn run(file: &Path, port: u16, scheme: Option<&str>, open_browser: bool) -> Result<()> {
    if !file.exists() {
        anyhow::bail!(
            "'{}': file not found\n\nCreate it with: strok new {} 800x800",
            file.display(),
            file.display()
        );
    }

    let (svg, editor, error) = match render_snapshot(file, scheme) {
        Ok(snapshot) => (Some(snapshot.svg), snapshot.editor, None),
        Err(e) => (None, Json::array([]), Some(e)),
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            version: 1,
            svg,
            editor,
            error,
        }),
        edit: Mutex::new(EditHistory::default()),
        changed: Condvar::new(),
    });

    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("failed to bind 127.0.0.1:{}", port))?;
    let addr = listener.local_addr()?;
    let url = format!("http://{}/", addr);
    eprintln!("watching {} — live preview at {}", file.display(), url);
    eprintln!("press Ctrl-C to stop");

    {
        let shared = Arc::clone(&shared);
        let file: PathBuf = file.to_path_buf();
        let scheme = scheme.map(|s| s.to_string());
        std::thread::spawn(move || watch_loop(&shared, &file, scheme.as_deref()));
    }

    if open_browser {
        open_in_browser(&url);
    }

    let display_name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document")
        .to_string();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let shared = Arc::clone(&shared);
        let name = display_name.clone();
        let file = file.to_path_buf();
        let scheme = scheme.map(str::to_string);
        std::thread::spawn(move || {
            let _ = handle_client(stream, &shared, &name, &file, scheme.as_deref());
        });
    }
    Ok(())
}

/// Poll the file and re-render on change. Content comparison (not just mtime)
/// avoids spurious re-renders from editors that touch without writing.
fn watch_loop(shared: &Shared, file: &Path, scheme: Option<&str>) {
    let mut last_contents = std::fs::read(file).ok();
    loop {
        std::thread::sleep(POLL_INTERVAL);
        let contents = std::fs::read(file).ok();
        if contents == last_contents {
            continue;
        }
        // A rename-style save can momentarily leave the path unreadable; keep
        // the previous state and catch the real contents on the next poll.
        if contents.is_none() {
            continue;
        }
        if let Some(current) = contents.as_deref() {
            let mut history = shared.edit.lock().unwrap();
            if !history.matches(current) {
                history.clear();
            }
        }
        last_contents = contents;
        let result = render_snapshot(file, scheme);
        let mut state = shared.state.lock().unwrap();
        state.version += 1;
        match result {
            Ok(snapshot) => {
                state.svg = Some(snapshot.svg);
                state.editor = snapshot.editor;
                state.error = None;
            }
            Err(e) => state.error = Some(e),
        }
        drop(state);
        shared.changed.notify_all();
    }
}

/// Render the document to SVG exactly like `export svg` (scene documents get
/// palette tokens resolved against the requested scheme first).
fn render_snapshot(path: &Path, scheme: Option<&str>) -> std::result::Result<Snapshot, String> {
    let render = || -> Result<Snapshot> {
        let loaded =
            Document::load(path).with_context(|| format!("failed to load '{}'", path.display()))?;
        let editor = loaded
            .scene
            .as_ref()
            .map(editor_json)
            .unwrap_or_else(|| Json::array([]));
        let doc = match loaded.scene.as_ref() {
            Some(s) => Document::from_scene(resolve::apply_scheme(s, scheme)?),
            None => loaded,
        };
        Ok(Snapshot {
            svg: emit::emit_document(&doc),
            editor,
        })
    };
    render().map_err(|e| error_text(&e))
}

fn editor_json(scene: &strok_core::scene::Scene) -> Json {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    let mut linked_shapes = Vec::new();
    collect_linked_shapes(&scene.nodes, &mut linked_shapes);
    Json::array(scene.shapes.iter().filter_map(|shape| {
        if shape.template == Template::Text
            || scene.imported_shape_names.contains(&shape.name)
            || linked_shapes.contains(&shape.name)
        {
            return None;
        }
        let data = shape.resolve(coord_space);
        if data.points.is_empty() {
            return None;
        }
        let starts = data.subpath_starts.clone();
        let points = data
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let (mode, c1, c2, tension, arc) = match point.mode {
                    CurveMode::Sharp => ("sharp", None, None, None, None),
                    CurveMode::CatmullRom(tension) => ("smooth", None, None, Some(tension), None),
                    CurveMode::Arc {
                        rx,
                        ry,
                        sweep,
                        large,
                    } => ("arc", None, None, None, Some((rx, ry, sweep, large))),
                    CurveMode::Controls { c1, c2 } => ("controls", Some(c1), Some(c2), None, None),
                    CurveMode::ControlsRelative { c1, c2 } => {
                        let previous = previous_point(&data, index).unwrap_or(point);
                        (
                            "controls-relative",
                            Some((previous.x + c1.0, previous.y + c1.1)),
                            Some((point.x + c2.0, point.y + c2.1)),
                            None,
                            None,
                        )
                    }
                };
                Json::obj([
                    ("name", Json::str(point.name.clone())),
                    ("x", Json::num(point.x)),
                    ("y", Json::num(point.y)),
                    ("mode", Json::str(mode)),
                    ("start", Json::Bool(index == 0 || starts.contains(&index))),
                    (
                        "controlsEditable",
                        Json::Bool(control_source(shape, &point.name).is_some()),
                    ),
                    (
                        "canSymmetrize",
                        Json::Bool(can_symmetrize(shape, &data, index)),
                    ),
                    ("c1", pair_json(c1)),
                    ("c2", pair_json(c2)),
                    ("tension", tension.map(Json::num).unwrap_or(Json::Null)),
                    (
                        "arc",
                        arc.map(|(rx, ry, sweep, large)| {
                            Json::obj([
                                ("rx", Json::num(rx)),
                                ("ry", Json::num(ry)),
                                ("sweep", Json::Bool(sweep)),
                                ("large", Json::Bool(large)),
                            ])
                        })
                        .unwrap_or(Json::Null),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        Some(Json::obj([
            ("name", Json::str(shape.name.clone())),
            ("d", Json::str(path_data_to_svg_d(&data, None))),
            ("closed", Json::Bool(data.closed)),
            ("points", Json::array(points)),
        ]))
    }))
}

fn collect_linked_shapes(nodes: &[strok_core::scene::SceneNode], names: &mut Vec<String>) {
    use strok_core::scene::SceneNode;
    for node in nodes {
        match node {
            SceneNode::Link(link) => names.push(link.name.clone()),
            SceneNode::Group(group) => collect_linked_shapes(&group.children, names),
            SceneNode::Boolean(boolean) => collect_linked_shapes(&boolean.children, names),
            SceneNode::Frame(frame) => collect_linked_shapes(&frame.children, names),
            SceneNode::Place(_) | SceneNode::Instance(_) => {}
        }
    }
}

fn pair_json(value: Option<(f64, f64)>) -> Json {
    value
        .map(|(x, y)| Json::array([Json::num(x), Json::num(y)]))
        .unwrap_or(Json::Null)
}

fn previous_point(data: &PathData, index: usize) -> Option<&strok_core::path_point::NamedPoint> {
    let (begin, end) = contour_bounds(data, index)?;
    if index > begin {
        data.points.get(index - 1)
    } else if data.closed {
        data.points.get(end.saturating_sub(1))
    } else {
        data.points.get(index)
    }
}

fn previous_neighbor(data: &PathData, index: usize) -> Option<&strok_core::path_point::NamedPoint> {
    let (begin, end) = contour_bounds(data, index)?;
    if index > begin {
        data.points.get(index - 1)
    } else if data.closed {
        data.points.get(end.saturating_sub(1))
    } else {
        None
    }
}

fn contour_bounds(data: &PathData, index: usize) -> Option<(usize, usize)> {
    if index >= data.points.len() {
        return None;
    }
    let begin = data
        .subpath_starts
        .iter()
        .copied()
        .take_while(|start| *start <= index)
        .last()
        .unwrap_or(0);
    let end = data
        .subpath_starts
        .iter()
        .copied()
        .find(|start| *start > index)
        .unwrap_or(data.points.len());
    Some((begin, end))
}

fn control_source(shape: &Shape, point: &str) -> Option<usize> {
    shape.operations.iter().rposition(|operation| {
        matches!(
            operation,
            Operation::AddPoint {
                name,
                mode: Some(PointMode::Controls | PointMode::ControlsRelative),
                control_c1: Some(_),
                control_c2: Some(_),
                ..
            } if name == point
        )
    })
}

fn addpoint_source(shape: &Shape, point: &str) -> Option<usize> {
    shape.operations.iter().rposition(
        |operation| matches!(operation, Operation::AddPoint { name, .. } if name == point),
    )
}

fn can_symmetrize(shape: &Shape, data: &PathData, index: usize) -> bool {
    let Some(point) = data.points.get(index) else {
        return false;
    };
    let Some(next) = next_point(data, index) else {
        return false;
    };
    previous_neighbor(data, index).is_some()
        && addpoint_source(shape, &point.name).is_some()
        && addpoint_source(shape, &next.name).is_some()
}

impl EditHistory {
    fn matches(&self, current: &[u8]) -> bool {
        self.undo.last().is_some_and(|edit| edit.after == current)
            || self.redo.last().is_some_and(|edit| edit.before == current)
            || (self.undo.is_empty() && self.redo.is_empty())
    }

    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn push(&mut self, edit: FileEdit) {
        self.undo.push(edit);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

/// Positioned parse diagnostics render their own caret snippets; everything
/// else gets the plain error chain — mirroring the CLI's top-level handler.
fn error_text(e: &anyhow::Error) -> String {
    if let Some(strok_core::error::StrokError::ParseDiagnostics(diags)) =
        e.downcast_ref::<strok_core::error::StrokError>()
    {
        diags
            .iter()
            .map(|d| d.render())
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        format!("{:#}", e)
    }
}

fn handle_client(
    stream: TcpStream,
    shared: &Shared,
    display_name: &str,
    file: &Path,
    scheme: Option<&str>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = value.trim().parse().unwrap_or(0).min(64 * 1024);
        }
    }
    let mut request_body = vec![0u8; content_length];
    reader.read_exact(&mut request_body)?;
    let method = request_line.split_whitespace().next().unwrap_or("GET");
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");

    let mut stream = stream;
    match (method, path) {
        ("GET", "/") => respond(&mut stream, "text/html; charset=utf-8", PREVIEW_HTML),
        ("GET", "/state.json") => {
            let history = shared.edit.lock().unwrap();
            let can_undo = !history.undo.is_empty();
            let can_redo = !history.redo.is_empty();
            let state = shared.state.lock().unwrap();
            let body = Json::obj([
                ("version", Json::num(state.version as f64)),
                ("file", Json::str(display_name)),
                ("canUndo", Json::Bool(can_undo)),
                ("canRedo", Json::Bool(can_redo)),
                (
                    "svg",
                    match &state.svg {
                        Some(svg) => Json::str(svg.clone()),
                        None => Json::Null,
                    },
                ),
                ("editor", state.editor.clone()),
                (
                    "error",
                    match &state.error {
                        Some(e) => Json::str(e.clone()),
                        None => Json::Null,
                    },
                ),
            ])
            .to_string_compact();
            respond(&mut stream, "application/json", &body)
        }
        ("GET", "/events") => serve_events(&mut stream, shared),
        ("POST", "/edit") => {
            let body = String::from_utf8_lossy(&request_body);
            let fields = parse_form(&body);
            let mut history = shared.edit.lock().unwrap();
            if let Ok(current) = std::fs::read(file) {
                if !history.matches(&current) {
                    history.clear();
                }
            }
            let result = match fields.get("action").map(String::as_str) {
                Some("undo") => restore_history(file, &mut history, true),
                Some("redo") => restore_history(file, &mut history, false),
                _ => apply_edit(file, &fields).map(|edit| history.push(edit)),
            }
            .and_then(|()| render_snapshot(file, scheme).map_err(anyhow::Error::msg));
            match result {
                Ok(snapshot) => {
                    let mut state = shared.state.lock().unwrap();
                    state.version += 1;
                    state.svg = Some(snapshot.svg);
                    state.editor = snapshot.editor;
                    state.error = None;
                    drop(state);
                    shared.changed.notify_all();
                    respond(&mut stream, "application/json", "{\"ok\":true}")
                }
                Err(error) => respond_status(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    &error_text(&error),
                ),
            }
        }
        _ => respond_status(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
}

fn respond(stream: &mut TcpStream, content_type: &str, body: &str) -> std::io::Result<()> {
    respond_status(stream, "200 OK", content_type, body)
}

fn respond_status(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn parse_form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|field| {
            let (key, value) = field.split_once('=')?;
            Some((url_decode(key), url_decode(value)))
        })
        .collect()
}

fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let parsed = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(byte) = parsed {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn required<'a>(fields: &'a HashMap<String, String>, name: &str) -> Result<&'a str> {
    fields
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing '{}'", name))
}

fn coordinate(fields: &HashMap<String, String>, name: &str) -> Result<f64> {
    let raw = required(fields, name)?;
    let value = raw
        .parse::<f64>()
        .with_context(|| format!("'{}' is not a number", name))?;
    if !value.is_finite() {
        anyhow::bail!("'{}' must be finite", name);
    }
    Ok(value)
}

fn apply_edit(file: &Path, fields: &HashMap<String, String>) -> Result<FileEdit> {
    let action = required(fields, "action")?;
    let shape_name = required(fields, "shape")?;
    let original =
        std::fs::read(file).with_context(|| format!("failed to read '{}'", file.display()))?;
    let loaded =
        Document::load(file).with_context(|| format!("failed to load '{}'", file.display()))?;
    let mut scene = loaded
        .scene
        .ok_or_else(|| anyhow::anyhow!("visual editing requires a text .strok document"))?;
    if scene.imported_shape_names.contains(shape_name) {
        anyhow::bail!(
            "'{}' comes from an imported module and cannot be edited here",
            shape_name
        );
    }
    let mut linked_shapes = Vec::new();
    collect_linked_shapes(&scene.nodes, &mut linked_shapes);
    if linked_shapes.iter().any(|name| name == shape_name) {
        anyhow::bail!(
            "'{}' is generated by createlink; edit its source shape instead",
            shape_name
        );
    }

    let coord_space = (scene.document_size.w, scene.document_size.h);
    let shape_index = scene
        .shapes
        .iter()
        .position(|shape| shape.name == shape_name)
        .ok_or_else(|| anyhow::anyhow!("shape '{}' no longer exists", shape_name))?;
    let before = scene.shapes[shape_index].resolve(coord_space);
    let shape = &mut scene.shapes[shape_index];

    match action {
        "move" => {
            let point_name = required(fields, "point")?;
            let x = coordinate(fields, "x")?;
            let y = coordinate(fields, "y")?;
            move_anchor(shape, &before, point_name, (x, y))?;
        }
        "control" => {
            let point_name = required(fields, "point")?;
            let handle = required(fields, "handle")?;
            let x = coordinate(fields, "x")?;
            let y = coordinate(fields, "y")?;
            move_control(shape, &before, point_name, handle, (x, y))?;
            if let Some(opposite_point) = fields.get("oppositePoint") {
                let opposite_handle = required(fields, "oppositeHandle")?;
                let opposite_x = coordinate(fields, "oppositeX")?;
                let opposite_y = coordinate(fields, "oppositeY")?;
                move_control(
                    shape,
                    &before,
                    opposite_point,
                    opposite_handle,
                    (opposite_x, opposite_y),
                )?;
            }
        }
        "retract-control" => {
            let point_name = required(fields, "point")?;
            let handle = required(fields, "handle")?;
            retract_control(shape, &before, point_name, handle)?;
        }
        "symmetric" => {
            let point_name = required(fields, "point")?;
            symmetrize_anchor(shape, &before, point_name)?;
        }
        "add" => {
            let after = required(fields, "after")?;
            insert_midpoint(shape, &before, after)?;
        }
        "delete" => {
            let point_name = required(fields, "point")?;
            delete_anchor(shape, &before, point_name)?;
        }
        _ => anyhow::bail!("unknown edit action '{}'", action),
    }

    let output = dsl_emit::emit_scene(&scene);
    dsl_parse::parse_file_with_path(&output, file)
        .with_context(|| "edited document failed to parse; the file was not changed")?;
    let after = output.into_bytes();
    std::fs::write(file, &after).with_context(|| format!("failed to save '{}'", file.display()))?;
    Ok(FileEdit {
        before: original,
        after,
    })
}

fn restore_history(file: &Path, history: &mut EditHistory, undo: bool) -> Result<()> {
    let current =
        std::fs::read(file).with_context(|| format!("failed to read '{}'", file.display()))?;
    let edit = if undo {
        history.undo.last()
    } else {
        history.redo.last()
    }
    .ok_or_else(|| {
        anyhow::anyhow!(if undo {
            "nothing to undo"
        } else {
            "nothing to redo"
        })
    })?;
    let expected = if undo { &edit.after } else { &edit.before };
    if current != *expected {
        history.clear();
        anyhow::bail!("the file changed outside the shape editor; undo history was cleared");
    }
    let target = if undo { &edit.before } else { &edit.after };
    let source =
        String::from_utf8(target.clone()).with_context(|| "history entry is not valid UTF-8")?;
    dsl_parse::parse_file_with_path(&source, file)
        .with_context(|| "history entry failed to parse; the file was not changed")?;
    std::fs::write(file, target).with_context(|| format!("failed to save '{}'", file.display()))?;
    if undo {
        let edit = history.undo.pop().unwrap();
        history.redo.push(edit);
    } else {
        let edit = history.redo.pop().unwrap();
        history.undo.push(edit);
    }
    Ok(())
}

fn move_anchor(
    shape: &mut Shape,
    data: &PathData,
    point_name: &str,
    target: (f64, f64),
) -> Result<()> {
    let point_index = data
        .points
        .iter()
        .position(|point| point.name == point_name)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", point_name))?;
    let point = &data.points[point_index];
    let delta = (target.0 - point.x, target.1 - point.y);
    let mut moved = false;

    if let Some(index) = shape.operations.iter().rposition(
        |operation| matches!(operation, Operation::AddPoint { name, .. } if name == point_name),
    ) {
        if let Operation::AddPoint { at, .. } = &mut shape.operations[index] {
            at.0 += delta.0;
            at.1 += delta.1;
            moved = true;
        }
    }

    if !moved {
        if let Some(index) = shape.operations.iter().rposition(
            |operation| matches!(operation, Operation::MovePointTo { point, .. } if point == point_name),
        ) {
            if let Operation::MovePointTo { to, .. } = &mut shape.operations[index] {
                to.0 += delta.0;
                to.1 += delta.1;
                moved = true;
            }
        }
    }

    if !moved {
        shape.operations.push(Operation::MovePointTo {
            point: point_name.to_string(),
            to: target,
        });
    }

    // Keep explicit handles attached to the anchor, matching vector-editor
    // expectations. Relative controls follow automatically through their DSL
    // offsets; absolute controls need their stored coordinate nudged too.
    shift_absolute_control(shape, point_name, "c2", delta);
    if let Some(next) = next_point(data, point_index) {
        shift_absolute_control(shape, &next.name, "c1", delta);
    }
    Ok(())
}

fn next_point(data: &PathData, index: usize) -> Option<&strok_core::path_point::NamedPoint> {
    let (begin, end) = contour_bounds(data, index)?;
    if index + 1 < end {
        data.points.get(index + 1)
    } else if data.closed {
        data.points.get(begin)
    } else {
        None
    }
}

fn shift_absolute_control(shape: &mut Shape, point: &str, handle: &str, delta: (f64, f64)) {
    let Some(index) = control_source(shape, point) else {
        return;
    };
    if let Operation::AddPoint {
        mode: Some(PointMode::Controls),
        control_c1,
        control_c2,
        ..
    } = &mut shape.operations[index]
    {
        let control = if handle == "c1" {
            control_c1
        } else {
            control_c2
        };
        if let Some(value) = control {
            value.0 += delta.0;
            value.1 += delta.1;
        }
    }
}

fn move_control(
    shape: &mut Shape,
    data: &PathData,
    point_name: &str,
    handle: &str,
    target: (f64, f64),
) -> Result<()> {
    if handle != "c1" && handle != "c2" {
        anyhow::bail!("handle must be 'c1' or 'c2'");
    }
    let point_index = data
        .points
        .iter()
        .position(|point| point.name == point_name)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", point_name))?;
    let point = &data.points[point_index];
    let previous = previous_point(data, point_index)
        .ok_or_else(|| anyhow::anyhow!("point '{}' has no incoming segment", point_name))?;
    let source = control_source(shape, point_name).ok_or_else(|| {
        anyhow::anyhow!(
            "the handles for '{}' are generated by another operation and are read-only",
            point_name
        )
    })?;

    if let Operation::AddPoint {
        mode,
        control_c1,
        control_c2,
        ..
    } = &mut shape.operations[source]
    {
        let value = match mode {
            Some(PointMode::Controls) => target,
            Some(PointMode::ControlsRelative) if handle == "c1" => {
                (target.0 - previous.x, target.1 - previous.y)
            }
            Some(PointMode::ControlsRelative) => (target.0 - point.x, target.1 - point.y),
            _ => anyhow::bail!("point '{}' does not have explicit controls", point_name),
        };
        if handle == "c1" {
            *control_c1 = Some(value);
        } else {
            *control_c2 = Some(value);
        }
    }
    Ok(())
}

fn retract_control(
    shape: &mut Shape,
    data: &PathData,
    point_name: &str,
    handle: &str,
) -> Result<()> {
    let point_index = data
        .points
        .iter()
        .position(|point| point.name == point_name)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", point_name))?;
    let target = match handle {
        "c1" => {
            let previous = previous_neighbor(data, point_index)
                .ok_or_else(|| anyhow::anyhow!("point '{}' has no incoming control", point_name))?;
            (previous.x, previous.y)
        }
        "c2" => {
            let point = &data.points[point_index];
            (point.x, point.y)
        }
        _ => anyhow::bail!("handle must be 'c1' or 'c2'"),
    };
    move_control(shape, data, point_name, handle, target)
}

fn symmetrize_anchor(shape: &mut Shape, data: &PathData, point_name: &str) -> Result<()> {
    let index = data
        .points
        .iter()
        .position(|point| point.name == point_name)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", point_name))?;
    let point = &data.points[index];
    let previous = previous_neighbor(data, index)
        .ok_or_else(|| anyhow::anyhow!("the endpoint '{}' cannot have two controls", point_name))?;
    let next = next_point(data, index)
        .ok_or_else(|| anyhow::anyhow!("the endpoint '{}' cannot have two controls", point_name))?;
    if addpoint_source(shape, &point.name).is_none() || addpoint_source(shape, &next.name).is_none()
    {
        anyhow::bail!(
            "the controls at '{}' are generated and cannot be edited directly",
            point_name
        );
    }

    let dx = next.x - previous.x;
    let dy = next.y - previous.y;
    let length = dx.hypot(dy);
    let direction = if length > 1e-9 {
        (dx / length, dy / length)
    } else {
        (1.0, 0.0)
    };
    let incoming_span = (point.x - previous.x).hypot(point.y - previous.y);
    let outgoing_span = (next.x - point.x).hypot(next.y - point.y);
    let handle_length = (incoming_span.min(outgoing_span) / 3.0).max(1e-3);
    let incoming = (
        point.x - direction.0 * handle_length,
        point.y - direction.1 * handle_length,
    );
    let outgoing = (
        point.x + direction.0 * handle_length,
        point.y + direction.1 * handle_length,
    );

    let incoming_c1 = explicit_controls(data, index)
        .map(|controls| controls.0)
        .unwrap_or((
            previous.x + (point.x - previous.x) / 3.0,
            previous.y + (point.y - previous.y) / 3.0,
        ));
    let outgoing_c2 = data
        .points
        .iter()
        .position(|candidate| candidate.name == next.name)
        .and_then(|next_index| explicit_controls(data, next_index))
        .map(|controls| controls.1)
        .unwrap_or((
            next.x - (next.x - point.x) / 3.0,
            next.y - (next.y - point.y) / 3.0,
        ));

    materialize_controls(shape, &point.name, incoming_c1, incoming)?;
    materialize_controls(shape, &next.name, outgoing, outgoing_c2)?;
    Ok(())
}

fn explicit_controls(data: &PathData, index: usize) -> Option<((f64, f64), (f64, f64))> {
    let point = data.points.get(index)?;
    match point.mode {
        CurveMode::Controls { c1, c2 } => Some((c1, c2)),
        CurveMode::ControlsRelative { c1, c2 } => {
            let previous = previous_point(data, index)?;
            Some((
                (previous.x + c1.0, previous.y + c1.1),
                (point.x + c2.0, point.y + c2.1),
            ))
        }
        CurveMode::Sharp | CurveMode::CatmullRom(_) | CurveMode::Arc { .. } => None,
    }
}

fn materialize_controls(
    shape: &mut Shape,
    point_name: &str,
    c1: (f64, f64),
    c2: (f64, f64),
) -> Result<()> {
    let source = addpoint_source(shape, point_name).ok_or_else(|| {
        anyhow::anyhow!(
            "point '{}' is generated and cannot own controls",
            point_name
        )
    })?;
    let Operation::AddPoint {
        mode,
        tension,
        arc_rx,
        arc_ry,
        arc_sweep,
        arc_large,
        arc_bulge,
        control_c1,
        control_c2,
        ..
    } = &mut shape.operations[source]
    else {
        unreachable!();
    };
    *mode = Some(PointMode::Controls);
    *tension = None;
    *arc_rx = None;
    *arc_ry = None;
    *arc_sweep = None;
    *arc_large = None;
    *arc_bulge = None;
    *control_c1 = Some(c1);
    *control_c2 = Some(c2);
    Ok(())
}

fn insert_midpoint(shape: &mut Shape, data: &PathData, after: &str) -> Result<()> {
    let from_index = data
        .points
        .iter()
        .position(|point| point.name == after)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", after))?;
    let (begin, end) = contour_bounds(data, from_index)
        .ok_or_else(|| anyhow::anyhow!("point '{}' is outside its path", after))?;
    let to_index = if from_index + 1 < end {
        from_index + 1
    } else if data.closed {
        begin
    } else {
        anyhow::bail!("the last point of an open path has no following segment");
    };
    let from = &data.points[from_index];
    let to = &data.points[to_index];
    let mut at = midpoint((from.x, from.y), (to.x, to.y));
    let mut mode = None;
    let mut tension = None;
    let mut control_c1 = None;
    let mut control_c2 = None;

    match to.mode {
        CurveMode::CatmullRom(value) => {
            mode = Some(PointMode::CatmullRom);
            tension = Some(value);
        }
        CurveMode::Controls { c1, c2 } => {
            let (a, d, e, c, point) = split_cubic((from.x, from.y), c1, c2, (to.x, to.y));
            if update_control_source(shape, &to.name, e, c) {
                at = point;
                mode = Some(PointMode::Controls);
                control_c1 = Some(a);
                control_c2 = Some(d);
            }
        }
        CurveMode::ControlsRelative { c1, c2 } => {
            let absolute_c1 = (from.x + c1.0, from.y + c1.1);
            let absolute_c2 = (to.x + c2.0, to.y + c2.1);
            let (a, d, e, c, point) =
                split_cubic((from.x, from.y), absolute_c1, absolute_c2, (to.x, to.y));
            if update_control_source(
                shape,
                &to.name,
                (e.0 - point.0, e.1 - point.1),
                (c.0 - to.x, c.1 - to.y),
            ) {
                at = point;
                mode = Some(PointMode::ControlsRelative);
                control_c1 = Some((a.0 - from.x, a.1 - from.y));
                control_c2 = Some((d.0 - point.0, d.1 - point.1));
            }
        }
        CurveMode::Sharp | CurveMode::Arc { .. } => {}
    }

    let name = unique_point_name(data);
    shape.operations.push(Operation::AddPoint {
        name,
        at,
        after: Some(after.to_string()),
        mode,
        tension,
        arc_rx: None,
        arc_ry: None,
        arc_sweep: None,
        arc_large: None,
        arc_bulge: None,
        control_c1,
        control_c2,
    });
    Ok(())
}

fn update_control_source(shape: &mut Shape, point: &str, c1: (f64, f64), c2: (f64, f64)) -> bool {
    let Some(index) = control_source(shape, point) else {
        return false;
    };
    if let Operation::AddPoint {
        control_c1,
        control_c2,
        ..
    } = &mut shape.operations[index]
    {
        *control_c1 = Some(c1);
        *control_c2 = Some(c2);
        true
    } else {
        false
    }
}

type CubicSplit = ((f64, f64), (f64, f64), (f64, f64), (f64, f64), (f64, f64));

fn split_cubic(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64)) -> CubicSplit {
    let a = midpoint(p0, c1);
    let b = midpoint(c1, c2);
    let c = midpoint(c2, p3);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    (a, d, e, c, midpoint(d, e))
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn unique_point_name(data: &PathData) -> String {
    for index in 1..=usize::MAX {
        let candidate = format!("p{}", index);
        if data.points.iter().all(|point| point.name != candidate) {
            return candidate;
        }
    }
    "point".to_string()
}

fn delete_anchor(shape: &mut Shape, data: &PathData, point_name: &str) -> Result<()> {
    let index = data
        .points
        .iter()
        .position(|point| point.name == point_name)
        .ok_or_else(|| anyhow::anyhow!("point '{}' no longer exists", point_name))?;
    let (begin, end) = contour_bounds(data, index)
        .ok_or_else(|| anyhow::anyhow!("point '{}' is outside its path", point_name))?;
    let minimum = if data.closed { 3 } else { 2 };
    if end - begin <= minimum {
        anyhow::bail!(
            "this {} contour needs at least {} points",
            if data.closed { "closed" } else { "open" },
            minimum
        );
    }
    shape.operations.push(Operation::DeletePoint {
        point: point_name.to_string(),
        reconnect: None,
    });
    Ok(())
}

/// Server-Sent Events: emit the current version immediately, then again on
/// every change. The client fetches `/state.json` when the number moves.
fn serve_events(stream: &mut TcpStream, shared: &Shared) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n"
    )?;
    let mut last_sent = {
        let state = shared.state.lock().unwrap();
        state.version
    };
    write!(stream, "data: {}\n\n", last_sent)?;
    stream.flush()?;
    loop {
        let state = shared.state.lock().unwrap();
        let (state, timed_out) = shared
            .changed
            .wait_timeout_while(state, SSE_PING_INTERVAL, |s| s.version == last_sent)
            .unwrap();
        let version = state.version;
        drop(state);
        if timed_out.timed_out() {
            // Comment line keeps the connection alive and detects dead peers.
            write!(stream, ": ping\n\n")?;
        } else {
            last_sent = version;
            write!(stream, "data: {}\n\n", version)?;
        }
        stream.flush()?;
    }
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if result.is_err() {
        eprintln!("note: couldn't open a browser automatically — open {url} yourself");
    }
}

const PREVIEW_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>strøk</title>
<style>
  :root {
    color-scheme: dark;
    --bg: #15171a; --panel: #1d2024; --raised: #25292e; --edge: #343940;
    --text: #f0f2f4; --dim: #a3a9b2; --quiet: #777e88;
    --accent: #7bd5b4; --control: #f6bd60; --ok: #56d393; --err: #ff6b70;
  }
  * { box-sizing: border-box; margin: 0; }
  html, body { height: 100%; }
  body {
    background: var(--bg); color: var(--text);
    font: 13px/1.5 ui-sans-serif, system-ui, -apple-system, sans-serif;
    display: flex; flex-direction: column; overflow: hidden;
  }
  header {
    min-height: 46px; display: flex; align-items: center; gap: 10px;
    padding: 7px 12px; border-bottom: 1px solid var(--edge);
    background: var(--panel); flex: none;
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--quiet); flex: none; }
  .dot.ok { background: var(--ok); }
  .dot.err { background: var(--err); }
  .name { font-weight: 650; }
  .meta { color: var(--dim); font-variant-numeric: tabular-nums; }
  .spacer { flex: 1; }
  button, select {
    min-height: 30px; border: 1px solid var(--edge); border-radius: 7px;
    background: var(--raised); color: var(--text); font: inherit;
  }
  button { padding: 3px 11px; cursor: pointer; }
  button:hover:not(:disabled), select:hover { border-color: var(--quiet); }
  button:disabled { cursor: not-allowed; color: var(--quiet); opacity: .65; }
  button.primary { border-color: #4b7f6c; background: #24483b; color: #dffbef; }
  button.danger { color: #ffc4c7; }
  button:focus-visible, select:focus-visible, [tabindex]:focus-visible {
    outline: 2px solid var(--accent); outline-offset: 2px;
  }
  #errorbar {
    flex: none; max-height: 40vh; overflow: auto;
    border-bottom: 1px solid var(--err); background: #32191c; color: #ffc4c7;
    padding: 10px 14px; white-space: pre; font: 12px/1.5 ui-monospace, monospace;
  }
  #errorbar[hidden], #inspector[hidden] { display: none; }
  #workspace { flex: 1; min-height: 0; display: flex; }
  #inspector {
    width: 252px; flex: none; overflow: auto; background: var(--panel);
    border-right: 1px solid var(--edge); padding: 18px 16px;
  }
  #inspector h1 { font-size: 15px; line-height: 1.25; margin-bottom: 4px; }
  #inspector .intro { color: var(--dim); margin-bottom: 18px; }
  label { display: block; color: var(--dim); font-size: 12px; margin-bottom: 6px; }
  select { width: 100%; padding: 4px 8px; margin-bottom: 20px; }
  #point-panel { border-top: 1px solid var(--edge); padding-top: 16px; }
  #point-name { color: var(--text); font-weight: 650; }
  #point-coords { display: block; color: var(--dim); font: 12px/1.5 ui-monospace, monospace; margin: 3px 0 13px; }
  .inspector-actions { display: flex; gap: 7px; }
  .inspector-actions + .inspector-actions { margin-top: 7px; }
  .hint { color: var(--quiet); font-size: 12px; margin-top: 22px; }
  kbd { border: 1px solid var(--edge); border-bottom-color: var(--quiet); border-radius: 4px; padding: 0 4px; font: inherit; color: var(--dim); }
  main { flex: 1; min-width: 0; min-height: 0; padding: 20px; }
  #stage {
    width: 100%; height: 100%; min-height: 180px;
    display: flex; align-items: center; justify-content: center; overflow: hidden;
    border-radius: 10px; position: relative;
  }
  #stage.stale { opacity: .45; }
  #stage > svg { width: 100%; height: 100%; max-width: 100%; max-height: 100%; display: block; }
  #stage.checker {
    background: repeating-conic-gradient(#24272b 0% 25%, #1b1e22 0% 50%) 0 0 / 20px 20px;
  }
  #stage.white { background: #fff; }
  #stage.black { background: #000; }
  #empty { color: var(--dim); }
  #toast {
    position: fixed; left: 50%; bottom: 20px; translate: -50% 8px;
    max-width: min(520px, calc(100vw - 28px)); padding: 8px 12px;
    border: 1px solid #87454a; border-radius: 8px; background: #32191c; color: #ffd9db;
    opacity: 0; pointer-events: none; transition: opacity 120ms ease-out, translate 120ms ease-out;
  }
  #toast.show { opacity: 1; translate: -50% 0; }
  .edit-path { fill: rgba(123, 213, 180, .13); stroke: var(--accent); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
  .control-line { stroke: var(--control); stroke-width: 1; vector-effect: non-scaling-stroke; opacity: .7; }
  .tangent-guide { stroke: var(--control); stroke-width: 1; stroke-dasharray: 4 3; vector-effect: non-scaling-stroke; opacity: .55; pointer-events: none; }
  .snap-guide { stroke: var(--accent); stroke-width: 1; stroke-dasharray: 3 3; vector-effect: non-scaling-stroke; opacity: .8; pointer-events: none; }
  .anchor { fill: var(--panel); stroke: var(--accent); stroke-width: 2; vector-effect: non-scaling-stroke; cursor: grab; }
  .anchor:hover, .anchor.selected { fill: var(--accent); stroke: var(--panel); }
  .control { fill: var(--control); stroke: var(--panel); stroke-width: 1.5; vector-effect: non-scaling-stroke; cursor: grab; }
  .control:hover, .control.selected { fill: var(--panel); stroke: var(--control); stroke-width: 2.5; }
  .control.readonly { opacity: .45; cursor: not-allowed; }
  .insert { fill: var(--raised); stroke: var(--accent); stroke-width: 1.5; vector-effect: non-scaling-stroke; cursor: pointer; }
  .insert-mark { stroke: var(--accent); stroke-width: 1.5; vector-effect: non-scaling-stroke; pointer-events: none; }
  @media (max-width: 700px) {
    header .meta:not(#save-status) { display: none; }
    #bgbtn { display: none; }
    #workspace { flex-direction: column; }
    #inspector { width: 100%; max-height: 190px; border-right: 0; border-bottom: 1px solid var(--edge); padding: 12px; }
    #inspector .intro, .hint { display: none; }
    #inspector h1 { display: inline; margin-right: 8px; }
    select { width: min(55%, 260px); margin: 0 0 10px; }
    #point-panel { padding-top: 10px; }
    main { padding: 10px; }
  }
  @media (prefers-reduced-motion: reduce) { #toast { transition: none; } }
</style>
</head>
<body>
<header>
  <span class="dot" id="dot" aria-hidden="true"></span>
  <span class="name" id="name">…</span>
  <span class="meta" id="size"></span>
  <span class="spacer"></span>
  <span class="meta" id="save-status" role="status"></span>
  <span class="meta" id="rev"></span>
  <button id="undobtn" title="Undo shape edit (Ctrl/⌘Z)" disabled>Undo</button>
  <button id="redobtn" title="Redo shape edit (Ctrl/⌘ Shift+Z)" disabled>Redo</button>
  <button id="editbtn" class="primary" disabled>Edit shape</button>
  <button id="bgbtn" title="Cycle checkerboard, white, and black backdrops">Backdrop</button>
</header>
<pre id="errorbar" hidden></pre>
<div id="workspace">
  <aside id="inspector" hidden>
    <h1>Shape editor</h1>
    <p class="intro">Changes save directly to the watched file.</p>
    <label for="shape-select">Shape definition</label>
    <select id="shape-select"></select>
    <section id="point-panel" aria-live="polite">
      <span id="point-name">Select a point</span>
      <span id="point-coords">Drag an anchor to move it</span>
      <div class="inspector-actions">
        <button id="symmetricbtn" disabled>Equalize handles <kbd>⇧C</kbd></button>
      </div>
      <div class="inspector-actions">
        <button id="deletebtn" class="danger" disabled>Delete point</button>
      </div>
    </section>
    <p class="hint">Handles stay linked by default. Hold <kbd>Alt</kbd> to move one, <kbd>Shift</kbd> for 45°, or press <kbd>⇧C</kbd> to create an equal pair. Alignment guides snap anchors. Select exactly what you want to remove, then press <kbd>Delete</kbd>. Use <kbd>Ctrl/⌘Z</kbd> to undo.</p>
  </aside>
  <main><div id="stage" class="checker"><span id="empty">waiting for first render…</span></div></main>
</div>
<div id="toast" role="alert"></div>
<script>
  const $ = (id) => document.getElementById(id);
  const ns = 'http://www.w3.org/2000/svg';
  const backdrops = ['checker', 'white', 'black'];
  let backdrop = 0, version = -1, state = null, editing = false;
  let shapeName = null, selection = null, drag = null, editorSvg = null, toastTimer = null;

  $('bgbtn').onclick = () => {
    $('stage').classList.remove(backdrops[backdrop]);
    backdrop = (backdrop + 1) % backdrops.length;
    $('stage').classList.add(backdrops[backdrop]);
  };
  $('editbtn').onclick = () => editing ? leaveEditor() : enterEditor();
  $('shape-select').onchange = (event) => {
    shapeName = event.target.value; selection = null; renderEditor();
  };
  $('deletebtn').onclick = deleteSelected;
  $('symmetricbtn').onclick = equalizeSelected;
  $('undobtn').onclick = () => edit({ action: 'undo' });
  $('redobtn').onclick = () => edit({ action: 'redo' });

  function showToast(message) {
    $('toast').textContent = message;
    $('toast').classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => $('toast').classList.remove('show'), 4200);
  }

  async function refresh() {
    try {
      const response = await fetch('/state.json');
      state = await response.json();
      version = state.version;
      document.title = state.file + ' — strøk';
      $('name').textContent = state.file;
      $('rev').textContent = 'rev ' + state.version;
      $('undobtn').disabled = !state.canUndo;
      $('redobtn').disabled = !state.canRedo;
      $('errorbar').hidden = !state.error;
      $('errorbar').textContent = state.error || '';
      $('stage').classList.toggle('stale', !!state.error);
      $('dot').className = 'dot ' + (state.error ? 'err' : 'ok');
      $('editbtn').disabled = !state.editor.length;
      $('editbtn').title = state.editor.length ? 'Edit named shape geometry' : 'This document has no local editable shapes';
      updateShapeSelect();
      if (editing && state.editor.length) renderEditor();
      else if (editing) leaveEditor();
      else renderDocument();
      $('save-status').textContent = '';
    } catch (_) {
      $('dot').className = 'dot';
    }
  }

  function updateShapeSelect() {
    const names = state.editor.map((shape) => shape.name);
    if (!shapeName || !names.includes(shapeName)) shapeName = names[0] || null;
    $('shape-select').replaceChildren(...names.map((name) => {
      const option = document.createElement('option');
      option.value = name; option.textContent = name; option.selected = name === shapeName;
      return option;
    }));
  }

  function renderDocument() {
    if (!state || !state.svg) return;
    $('stage').innerHTML = state.svg;
    const svg = $('stage').querySelector('svg');
    if (!svg) return;
    const width = svg.getAttribute('width'), height = svg.getAttribute('height');
    if (!svg.getAttribute('viewBox') && width && height) svg.setAttribute('viewBox', `0 0 ${parseFloat(width)} ${parseFloat(height)}`);
    if (width && height) $('size').textContent = parseFloat(width) + '×' + parseFloat(height);
    svg.removeAttribute('width'); svg.removeAttribute('height');
    svg.setAttribute('aria-label', 'Document preview');
  }

  function enterEditor() {
    if (!state?.editor.length) return;
    editing = true; selection = null;
    $('inspector').hidden = false;
    $('editbtn').textContent = 'Done';
    $('editbtn').classList.remove('primary');
    renderEditor();
  }

  function leaveEditor() {
    editing = false; drag = null; editorSvg = null;
    $('inspector').hidden = true;
    $('editbtn').textContent = 'Edit shape';
    $('editbtn').classList.add('primary');
    $('point-name').textContent = 'Select a point';
    $('deletebtn').disabled = true;
    $('symmetricbtn').disabled = true;
    renderDocument();
  }

  function currentShape() { return state?.editor.find((shape) => shape.name === shapeName); }
  function contours(shape) {
    const starts = shape.points.map((point, index) => point.start ? index : -1).filter((index) => index >= 0);
    return starts.map((begin, index) => [begin, starts[index + 1] ?? shape.points.length]);
  }
  function contourFor(shape, index) { return contours(shape).find(([begin, end]) => index >= begin && index < end); }
  function nextIndex(shape, index) {
    const [begin, end] = contourFor(shape, index);
    return index + 1 < end ? index + 1 : (shape.closed ? begin : null);
  }
  function previousIndex(shape, index) {
    const [begin, end] = contourFor(shape, index);
    return index > begin ? index - 1 : (shape.closed ? end - 1 : index);
  }
  function pointAt(shape, index) { return shape.points[index]; }
  function attachedAnchorIndex(shape, index, handle) {
    return handle === 'c1' ? previousIndex(shape, index) : index;
  }
  function handlesAtAnchor(shape, anchorIndex) {
    const handles = [];
    const anchor = pointAt(shape, anchorIndex);
    if (anchor?.c2 && anchor.controlsEditable) handles.push({ index: anchorIndex, handle: 'c2' });
    const next = nextIndex(shape, anchorIndex);
    const outgoing = next === null ? null : pointAt(shape, next);
    if (outgoing?.c1 && outgoing.controlsEditable) handles.push({ index: next, handle: 'c1' });
    return handles;
  }
  function oppositeHandle(shape, index, handle) {
    const anchorIndex = attachedAnchorIndex(shape, index, handle);
    return handlesAtAnchor(shape, anchorIndex).find((candidate) => candidate.index !== index || candidate.handle !== handle) || null;
  }
  function selectedAnchorIndex(shape) {
    if (!selection) return null;
    const name = selection.kind === 'anchor' ? selection.name : selection.anchor;
    const index = shape.points.findIndex((point) => point.name === name);
    return index >= 0 ? index : null;
  }
  function isSelectedControl(point, handle) {
    return selection?.kind === 'control' && selection.point === point.name && selection.handle === handle;
  }
  function isRetracted(shape, index, handle) {
    const point = pointAt(shape, index), anchor = pointAt(shape, attachedAnchorIndex(shape, index, handle));
    return Math.hypot(point[handle][0] - anchor.x, point[handle][1] - anchor.y) < 1e-7;
  }
  const lerp = (a, b, t) => a + (b - a) * t;

  function smoothControls(shape, targetIndex) {
    const target = pointAt(shape, targetIndex), tension = target.tension ?? 0;
    const previous = previousIndex(shape, targetIndex);
    const previousPrevious = previousIndex(shape, previous);
    const nextOfPrevious = nextIndex(shape, previous) ?? previous;
    const nextOfTarget = nextIndex(shape, targetIndex) ?? targetIndex;
    const p = pointAt(shape, previous), pPrev = pointAt(shape, previousPrevious), pNext = pointAt(shape, nextOfPrevious);
    const tPrev = (1 - tension) / 6;
    const c1 = [p.x + (pNext.x - pPrev.x) * tPrev, p.y + (pNext.y - pPrev.y) * tPrev];
    const tNext = (1 - tension) / 6;
    const targetPrev = pointAt(shape, previous), targetNext = pointAt(shape, nextOfTarget);
    const c2 = [target.x - (targetNext.x - targetPrev.x) * tNext, target.y - (targetNext.y - targetPrev.y) * tNext];
    return [c1, c2];
  }

  function segmentCommand(shape, targetIndex) {
    const point = pointAt(shape, targetIndex);
    if (point.mode === 'controls' || point.mode === 'controls-relative') return `C${point.c1[0]} ${point.c1[1]} ${point.c2[0]} ${point.c2[1]} ${point.x} ${point.y}`;
    if (point.mode === 'smooth') {
      const [c1, c2] = smoothControls(shape, targetIndex);
      return `C${c1[0]} ${c1[1]} ${c2[0]} ${c2[1]} ${point.x} ${point.y}`;
    }
    if (point.mode === 'arc' && point.arc) return `A${point.arc.rx} ${point.arc.ry} 0 ${point.arc.large ? 1 : 0} ${point.arc.sweep ? 1 : 0} ${point.x} ${point.y}`;
    return `L${point.x} ${point.y}`;
  }

  function buildPath(shape) {
    return contours(shape).map(([begin, end]) => {
      let path = `M${shape.points[begin].x} ${shape.points[begin].y}`;
      for (let index = begin + 1; index < end; index++) path += ' ' + segmentCommand(shape, index);
      if (shape.closed) path += ' ' + segmentCommand(shape, begin) + ' Z';
      return path;
    }).join(' ');
  }

  function cubicMidpoint(p0, c1, c2, p3) {
    const a = [lerp(p0[0], c1[0], .5), lerp(p0[1], c1[1], .5)];
    const b = [lerp(c1[0], c2[0], .5), lerp(c1[1], c2[1], .5)];
    const c = [lerp(c2[0], p3[0], .5), lerp(c2[1], p3[1], .5)];
    const d = [lerp(a[0], b[0], .5), lerp(a[1], b[1], .5)];
    const e = [lerp(b[0], c[0], .5), lerp(b[1], c[1], .5)];
    return [lerp(d[0], e[0], .5), lerp(d[1], e[1], .5)];
  }

  function segmentMidpoint(shape, fromIndex) {
    const toIndex = nextIndex(shape, fromIndex);
    if (toIndex === null) return null;
    const from = pointAt(shape, fromIndex), to = pointAt(shape, toIndex);
    if (to.mode === 'controls' || to.mode === 'controls-relative') return cubicMidpoint([from.x, from.y], to.c1, to.c2, [to.x, to.y]);
    if (to.mode === 'smooth') {
      const [c1, c2] = smoothControls(shape, toIndex);
      return cubicMidpoint([from.x, from.y], c1, c2, [to.x, to.y]);
    }
    return [(from.x + to.x) / 2, (from.y + to.y) / 2];
  }

  function svgElement(tag, attrs = {}) {
    const element = document.createElementNS(ns, tag);
    Object.entries(attrs).forEach(([name, value]) => element.setAttribute(name, value));
    return element;
  }

  function renderEditor() {
    const shape = currentShape();
    if (!shape) return;
    const svg = svgElement('svg', { role: 'img', 'aria-label': `Editing ${shape.name}` });
    editorSvg = svg;
    const path = svgElement('path', { class: 'edit-path', d: buildPath(shape) });
    path.dataset.role = 'path'; svg.append(path);
    const allCoordinates = shape.points.flatMap((point) => [[point.x, point.y], point.c1, point.c2].filter(Boolean));
    let minX = Math.min(...allCoordinates.map((point) => point[0])), maxX = Math.max(...allCoordinates.map((point) => point[0]));
    let minY = Math.min(...allCoordinates.map((point) => point[1])), maxY = Math.max(...allCoordinates.map((point) => point[1]));
    const span = Math.max(maxX - minX, maxY - minY, 1), pad = Math.max(span * .14, 8), radius = Math.max(span / 105, .8);
    svg.setAttribute('viewBox', `${minX - pad} ${minY - pad} ${Math.max(maxX - minX, 1) + 2 * pad} ${Math.max(maxY - minY, 1) + 2 * pad}`);

    const guideLayer = svgElement('g', { 'data-role': 'guides', 'aria-hidden': 'true' });
    guideLayer.append(svgElement('line', { class: 'snap-guide', 'data-guide': 'x', visibility: 'hidden' }));
    guideLayer.append(svgElement('line', { class: 'snap-guide', 'data-guide': 'y', visibility: 'hidden' }));
    svg.append(guideLayer);

    shape.points.forEach((point, index) => {
      if (point.c1 && point.c2) {
        const previous = pointAt(shape, previousIndex(shape, index));
        if (!isRetracted(shape, index, 'c1')) svg.append(svgElement('line', { class: 'control-line', 'data-line': `${index}-c1`, x1: previous.x, y1: previous.y, x2: point.c1[0], y2: point.c1[1] }));
        if (!isRetracted(shape, index, 'c2')) svg.append(svgElement('line', { class: 'control-line', 'data-line': `${index}-c2`, x1: point.x, y1: point.y, x2: point.c2[0], y2: point.c2[1] }));
      }
    });
    const selectedAnchor = selectedAnchorIndex(shape);
    if (selectedAnchor !== null) {
      const pair = handlesAtAnchor(shape, selectedAnchor).filter((item) => !isRetracted(shape, item.index, item.handle));
      if (pair.length === 2) {
        const first = pointAt(shape, pair[0].index)[pair[0].handle], second = pointAt(shape, pair[1].index)[pair[1].handle];
        svg.append(svgElement('line', { class: 'tangent-guide', 'data-role': 'tangent-guide', x1: first[0], y1: first[1], x2: second[0], y2: second[1] }));
      }
    }
    shape.points.forEach((point, index) => {
      const mid = segmentMidpoint(shape, index);
      if (!mid) return;
      const group = svgElement('g', { tabindex: '0', role: 'button', 'aria-label': `Add point after ${point.name}`, 'data-insert': index });
      group.append(svgElement('circle', { class: 'insert', cx: mid[0], cy: mid[1], r: radius * .82 }));
      group.append(svgElement('line', { class: 'insert-mark', x1: mid[0] - radius * .38, y1: mid[1], x2: mid[0] + radius * .38, y2: mid[1] }));
      group.append(svgElement('line', { class: 'insert-mark', x1: mid[0], y1: mid[1] - radius * .38, x2: mid[0], y2: mid[1] + radius * .38 }));
      group.onclick = () => addAfter(index);
      group.onkeydown = (event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); addAfter(index); } };
      svg.append(group);
    });
    shape.points.forEach((point, index) => {
      if (point.c1 && point.c2) {
        ['c1', 'c2'].forEach((handle) => {
          if (isRetracted(shape, index, handle)) return;
          const selected = isSelectedControl(point, handle) ? ' selected' : '';
          const control = svgElement('rect', { class: `control${point.controlsEditable ? '' : ' readonly'}${selected}`, 'data-control': `${index}-${handle}`, x: point[handle][0] - radius * .7, y: point[handle][1] - radius * .7, width: radius * 1.4, height: radius * 1.4, rx: radius * .18, tabindex: point.controlsEditable ? '0' : '-1', role: 'button', 'aria-label': `${handle === 'c1' ? 'Outgoing' : 'Incoming'} control for ${pointAt(shape, attachedAnchorIndex(shape, index, handle)).name}` });
          if (point.controlsEditable) {
            control.onpointerdown = (event) => startDrag(event, 'control', index, handle);
            control.onkeydown = (event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectControl(index, handle); } };
          }
          svg.append(control);
        });
      }
      const anchor = svgElement('circle', { class: `anchor${selection?.kind === 'anchor' && point.name === selection.name ? ' selected' : ''}`, 'data-anchor': index, cx: point.x, cy: point.y, r: radius, tabindex: '0', role: 'button', 'aria-label': `Point ${point.name}` });
      anchor.onpointerdown = (event) => startDrag(event, 'anchor', index, null);
      anchor.onkeydown = (event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectAnchor(index); } };
      svg.append(anchor);
    });
    $('stage').replaceChildren(svg);
    $('size').textContent = shape.points.length + (shape.points.length === 1 ? ' point' : ' points');
    updatePointPanel();
  }

  function selectAnchor(index) {
    const shape = currentShape(), point = pointAt(shape, index);
    selection = { kind: 'anchor', name: point.name };
    renderEditor();
  }

  function selectControl(index, handle) {
    const shape = currentShape(), point = pointAt(shape, index), anchor = pointAt(shape, attachedAnchorIndex(shape, index, handle));
    selection = { kind: 'control', point: point.name, handle, anchor: anchor.name };
    renderEditor();
  }

  function updatePointPanel() {
    const shape = currentShape(), anchorIndex = shape ? selectedAnchorIndex(shape) : null;
    if (anchorIndex === null) {
      $('point-name').textContent = 'Select a point';
      $('point-coords').textContent = 'Drag an anchor to move it';
      $('deletebtn').disabled = true;
      $('symmetricbtn').disabled = true;
      return;
    }
    const anchor = pointAt(shape, anchorIndex);
    if (selection.kind === 'control') {
      const segment = shape.points.find((point) => point.name === selection.point);
      const value = segment?.[selection.handle];
      $('point-name').textContent = `${anchor.name} · ${selection.handle === 'c1' ? 'outgoing' : 'incoming'} control`;
      $('point-coords').textContent = value ? `${formatNumber(value[0])}, ${formatNumber(value[1])}` : 'Control no longer exists';
      $('deletebtn').textContent = 'Retract control';
      $('deletebtn').disabled = !value;
    } else {
      $('point-name').textContent = anchor.name;
      $('point-coords').textContent = `${formatNumber(anchor.x)}, ${formatNumber(anchor.y)} · ${anchor.mode}`;
      const [begin, end] = contourFor(shape, anchorIndex);
      $('deletebtn').textContent = 'Delete point';
      $('deletebtn').disabled = end - begin <= (shape.closed ? 3 : 2);
    }
    $('symmetricbtn').disabled = !anchor.canSymmetrize;
  }
  const formatNumber = (value) => Number(value.toFixed(3)).toString();

  function screenPoint(event) {
    const point = new DOMPoint(event.clientX, event.clientY);
    return point.matrixTransform(editorSvg.getScreenCTM().inverse());
  }

  function startDrag(event, kind, index, handle) {
    event.preventDefault(); event.stopPropagation();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const shape = currentShape(), point = pointAt(shape, index);
    if (kind === 'anchor') {
      selection = { kind: 'anchor', name: point.name };
      drag = { kind, index, moved: false };
    } else {
      const anchorIndex = attachedAnchorIndex(shape, index, handle), anchor = pointAt(shape, anchorIndex);
      selection = { kind: 'control', point: point.name, handle, anchor: anchor.name };
      const opposite = event.altKey ? null : oppositeHandle(shape, index, handle);
      const oppositePoint = opposite ? pointAt(shape, opposite.index)[opposite.handle] : null;
      drag = {
        kind, index, handle, anchorIndex, opposite, moved: false,
        oppositeLength: oppositePoint ? Math.hypot(oppositePoint[0] - anchor.x, oppositePoint[1] - anchor.y) : 0,
      };
    }
    editorSvg.querySelectorAll('.anchor').forEach((anchor) => anchor.classList.toggle('selected', selection.kind === 'anchor' && pointAt(shape, Number(anchor.dataset.anchor)).name === selection.name));
    editorSvg.querySelectorAll('.control').forEach((control) => {
      const [controlIndex, controlHandle] = control.dataset.control.split('-');
      control.classList.toggle('selected', selection.kind === 'control' && Number(controlIndex) === index && controlHandle === handle);
    });
    updateTangentGuide();
    updatePointPanel();
  }

  function constrain45(anchor, target) {
    const dx = target.x - anchor.x, dy = target.y - anchor.y, length = Math.hypot(dx, dy);
    if (length < 1e-9) return target;
    const angle = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4);
    return { x: anchor.x + Math.cos(angle) * length, y: anchor.y + Math.sin(angle) * length };
  }

  function snapAnchor(shape, index, target) {
    const viewBox = editorSvg.viewBox.baseVal;
    const threshold = Math.max(viewBox.width / Math.max(editorSvg.clientWidth, 1), viewBox.height / Math.max(editorSvg.clientHeight, 1)) * 7;
    let x = target.x, y = target.y, guideX = null, guideY = null, bestX = threshold, bestY = threshold;
    shape.points.forEach((candidate, candidateIndex) => {
      if (candidateIndex === index) return;
      const dx = Math.abs(candidate.x - target.x), dy = Math.abs(candidate.y - target.y);
      if (dx < bestX) { bestX = dx; x = candidate.x; guideX = candidate.x; }
      if (dy < bestY) { bestY = dy; y = candidate.y; guideY = candidate.y; }
    });
    showSnapGuides(guideX, guideY);
    return { x, y };
  }

  function showSnapGuides(x, y) {
    if (!editorSvg) return;
    const box = editorSvg.viewBox.baseVal, xGuide = editorSvg.querySelector('[data-guide="x"]'), yGuide = editorSvg.querySelector('[data-guide="y"]');
    xGuide.setAttribute('visibility', x === null ? 'hidden' : 'visible');
    if (x !== null) { xGuide.setAttribute('x1', x); xGuide.setAttribute('x2', x); xGuide.setAttribute('y1', box.y); xGuide.setAttribute('y2', box.y + box.height); }
    yGuide.setAttribute('visibility', y === null ? 'hidden' : 'visible');
    if (y !== null) { yGuide.setAttribute('x1', box.x); yGuide.setAttribute('x2', box.x + box.width); yGuide.setAttribute('y1', y); yGuide.setAttribute('y2', y); }
  }

  window.addEventListener('pointermove', (event) => {
    if (!drag || !editing || !editorSvg) return;
    const shape = currentShape(), point = pointAt(shape, drag.index), next = nextIndex(shape, drag.index);
    let target = screenPoint(event);
    drag.moved = true;
    if (drag.kind === 'anchor') {
      target = snapAnchor(shape, drag.index, target);
      const dx = target.x - point.x, dy = target.y - point.y;
      point.x = target.x; point.y = target.y;
      if (point.controlsEditable && point.c2) { point.c2[0] += dx; point.c2[1] += dy; }
      if (next !== null) {
        const following = pointAt(shape, next);
        if (following.controlsEditable && following.c1) { following.c1[0] += dx; following.c1[1] += dy; }
      }
    } else {
      const anchor = pointAt(shape, drag.anchorIndex);
      if (event.shiftKey) target = constrain45(anchor, target);
      point[drag.handle] = [target.x, target.y];
      if (drag.opposite) {
        const dx = target.x - anchor.x, dy = target.y - anchor.y, length = Math.hypot(dx, dy);
        if (length > 1e-9) {
          const oppositePoint = pointAt(shape, drag.opposite.index);
          oppositePoint[drag.opposite.handle] = [anchor.x - dx / length * drag.oppositeLength, anchor.y - dy / length * drag.oppositeLength];
        }
      }
    }
    updateGeometry(); updatePointPanel();
  });

  window.addEventListener('pointerup', async () => {
    if (!drag) return;
    const shape = currentShape(), point = pointAt(shape, drag.index), pending = drag;
    drag = null;
    showSnapGuides(null, null);
    if (!pending.moved) return;
    if (pending.kind === 'anchor') await edit({ action: 'move', shape: shape.name, point: point.name, x: point.x, y: point.y });
    else {
      const fields = { action: 'control', shape: shape.name, point: point.name, handle: pending.handle, x: point[pending.handle][0], y: point[pending.handle][1] };
      if (pending.opposite) {
        const oppositePoint = pointAt(shape, pending.opposite.index);
        Object.assign(fields, { oppositePoint: oppositePoint.name, oppositeHandle: pending.opposite.handle, oppositeX: oppositePoint[pending.opposite.handle][0], oppositeY: oppositePoint[pending.opposite.handle][1] });
      }
      await edit(fields);
    }
  });

  window.addEventListener('pointercancel', () => {
    if (!drag) return;
    drag = null;
    showSnapGuides(null, null);
    refresh();
  });

  function updateGeometry() {
    const shape = currentShape();
    editorSvg.querySelector('[data-role="path"]').setAttribute('d', buildPath(shape));
    shape.points.forEach((point, index) => {
      const anchor = editorSvg.querySelector(`[data-anchor="${index}"]`);
      anchor.setAttribute('cx', point.x); anchor.setAttribute('cy', point.y);
      const mid = segmentMidpoint(shape, index), insert = editorSvg.querySelector(`[data-insert="${index}"]`);
      if (mid && insert) {
        const circle = insert.querySelector('circle'), lines = insert.querySelectorAll('line'), r = Number(circle.getAttribute('r'));
        circle.setAttribute('cx', mid[0]); circle.setAttribute('cy', mid[1]);
        lines[0].setAttribute('x1', mid[0] - r * .46); lines[0].setAttribute('x2', mid[0] + r * .46); lines[0].setAttribute('y1', mid[1]); lines[0].setAttribute('y2', mid[1]);
        lines[1].setAttribute('x1', mid[0]); lines[1].setAttribute('x2', mid[0]); lines[1].setAttribute('y1', mid[1] - r * .46); lines[1].setAttribute('y2', mid[1] + r * .46);
      }
      if (!point.c1 || !point.c2) return;
      ['c1', 'c2'].forEach((handle) => {
        const control = editorSvg.querySelector(`[data-control="${index}-${handle}"]`);
        if (!control) return;
        const width = Number(control.getAttribute('width'));
        control.setAttribute('x', point[handle][0] - width / 2); control.setAttribute('y', point[handle][1] - width / 2);
      });
      const previous = pointAt(shape, previousIndex(shape, index));
      const line1 = editorSvg.querySelector(`[data-line="${index}-c1"]`), line2 = editorSvg.querySelector(`[data-line="${index}-c2"]`);
      if (line1) { line1.setAttribute('x1', previous.x); line1.setAttribute('y1', previous.y); line1.setAttribute('x2', point.c1[0]); line1.setAttribute('y2', point.c1[1]); }
      if (line2) { line2.setAttribute('x1', point.x); line2.setAttribute('y1', point.y); line2.setAttribute('x2', point.c2[0]); line2.setAttribute('y2', point.c2[1]); }
    });
    updateTangentGuide();
  }

  function updateTangentGuide() {
    if (!editorSvg) return;
    const shape = currentShape(), anchorIndex = selectedAnchorIndex(shape);
    let guide = editorSvg.querySelector('[data-role="tangent-guide"]');
    const pair = anchorIndex === null ? [] : handlesAtAnchor(shape, anchorIndex).filter((item) => !isRetracted(shape, item.index, item.handle));
    if (pair.length !== 2) { guide?.remove(); return; }
    if (!guide) { guide = svgElement('line', { class: 'tangent-guide', 'data-role': 'tangent-guide' }); editorSvg.append(guide); }
    const first = pointAt(shape, pair[0].index)[pair[0].handle], second = pointAt(shape, pair[1].index)[pair[1].handle];
    guide.setAttribute('x1', first[0]); guide.setAttribute('y1', first[1]); guide.setAttribute('x2', second[0]); guide.setAttribute('y2', second[1]);
  }

  async function addAfter(index) {
    const shape = currentShape();
    await edit({ action: 'add', shape: shape.name, after: pointAt(shape, index).name });
  }
  async function deleteSelected() {
    const shape = currentShape();
    if (!selection || $('deletebtn').disabled) return;
    const pending = selection;
    selection = null;
    if (pending.kind === 'control') {
      await edit({ action: 'retract-control', shape: shape.name, point: pending.point, handle: pending.handle });
    } else {
      await edit({ action: 'delete', shape: shape.name, point: pending.name });
    }
  }

  async function equalizeSelected() {
    const shape = currentShape(), anchorIndex = shape ? selectedAnchorIndex(shape) : null;
    if (anchorIndex === null || $('symmetricbtn').disabled) return;
    const anchor = pointAt(shape, anchorIndex);
    selection = { kind: 'anchor', name: anchor.name };
    await edit({ action: 'symmetric', shape: shape.name, point: anchor.name });
  }

  async function edit(fields) {
    $('save-status').textContent = 'Saving…';
    try {
      const response = await fetch('/edit', { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded;charset=UTF-8' }, body: new URLSearchParams(fields) });
      if (!response.ok) throw new Error(await response.text());
      $('save-status').textContent = 'Saved';
      await refresh();
    } catch (error) {
      $('save-status').textContent = 'Not saved';
      showToast(error.message || 'The edit could not be saved.');
      await refresh();
    }
  }

  window.addEventListener('keydown', (event) => {
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)) return;
    const command = event.metaKey || event.ctrlKey;
    if (command && event.key.toLowerCase() === 'z') {
      event.preventDefault();
      edit({ action: event.shiftKey ? 'redo' : 'undo' });
      return;
    }
    if (!editing) return;
    if ((event.key === 'Delete' || event.key === 'Backspace') && !$('deletebtn').disabled) { event.preventDefault(); deleteSelected(); }
    if (event.shiftKey && !command && event.key.toLowerCase() === 'c' && !$('symmetricbtn').disabled) { event.preventDefault(); equalizeSelected(); }
    if (event.key === 'Escape') leaveEditor();
  });

  const es = new EventSource('/events');
  es.onmessage = (event) => { if (Number(event.data) !== version) refresh(); };
  es.onerror = () => { $('dot').className = 'dot'; };
  refresh();
</script>
</body>
</html>
"##;
