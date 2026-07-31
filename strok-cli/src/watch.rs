//! `strok watch` — live preview server for humans working in an editor.
//!
//! Strøk is file-first: every CLI command reads the document and exits. Watch
//! mode is the one long-running exception, built for a person editing `.strok`
//! source by hand who wants to see the render update as they save. It serves a
//! small local page that re-renders on every file change over Server-Sent
//! Events. Plain std only — no async runtime, no watcher crate (mtime+content
//! polling survives editors that save via rename), no web framework.

mod editing;
mod projection;

use anyhow::{Context, Result};
use editing::FileEdit;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use strok_core::document::Document;
use strok_core::dsl_parse;
use strok_core::emit;
use strok_core::json::Json;
use strok_core::resolve;

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
    /// Named placements that map editable shape geometry into document space.
    targets: Json,
    error: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    /// Serialize browser edits so two tabs cannot interleave file rewrites.
    edit: Mutex<EditHistory>,
    changed: Condvar,
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
    targets: Json,
}

pub fn run(file: &Path, port: u16, scheme: Option<&str>, open_browser: bool) -> Result<()> {
    if !file.exists() {
        anyhow::bail!(
            "'{}': file not found\n\nCreate it with: strok new {} 800x800",
            file.display(),
            file.display()
        );
    }

    let (svg, editor, targets, error) = match render_snapshot(file, scheme) {
        Ok(snapshot) => (Some(snapshot.svg), snapshot.editor, snapshot.targets, None),
        Err(e) => (None, Json::array([]), Json::array([]), Some(e)),
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            version: 1,
            svg,
            editor,
            targets,
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
                state.targets = snapshot.targets;
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
        let editor = loaded.scene.as_ref().map(projection::editor_projection);
        let (editor, targets) = editor
            .map(|projection| (projection.shapes, projection.targets))
            .unwrap_or_else(|| (Json::array([]), Json::array([])));
        let doc = match loaded.scene.as_ref() {
            Some(s) => Document::from_scene(resolve::apply_scheme(s, scheme)?),
            None => loaded,
        };
        Ok(Snapshot {
            svg: emit::emit_document(&doc),
            editor,
            targets,
        })
    };
    render().map_err(|e| error_text(&e))
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
        ("GET", "/watch.css") => respond(&mut stream, "text/css; charset=utf-8", PREVIEW_CSS),
        ("GET", "/watch.js") => respond(&mut stream, "text/javascript; charset=utf-8", PREVIEW_JS),
        ("GET", "/path-geometry.js") => respond(
            &mut stream,
            "text/javascript; charset=utf-8",
            PATH_GEOMETRY_JS,
        ),
        ("GET", "/viewport.js") => {
            respond(&mut stream, "text/javascript; charset=utf-8", VIEWPORT_JS)
        }
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
                ("targets", state.targets.clone()),
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
                _ => editing::apply(file, &fields).map(|edit| history.push(edit)),
            }
            .and_then(|()| render_snapshot(file, scheme).map_err(anyhow::Error::msg));
            match result {
                Ok(snapshot) => {
                    let mut state = shared.state.lock().unwrap();
                    state.version += 1;
                    state.svg = Some(snapshot.svg);
                    state.editor = snapshot.editor;
                    state.targets = snapshot.targets;
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

const PREVIEW_HTML: &str = include_str!("watch_ui/index.html");
const PREVIEW_CSS: &str = include_str!("watch_ui/watch.css");
const PREVIEW_JS: &str = include_str!("watch_ui/watch.js");
const PATH_GEOMETRY_JS: &str = include_str!("watch_ui/path-geometry.mjs");
const VIEWPORT_JS: &str = include_str!("watch_ui/viewport.mjs");
