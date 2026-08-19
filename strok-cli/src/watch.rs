//! `strok watch` — live preview server for humans working in an editor.
//!
//! Strøk is file-first: every CLI command reads the document and exits. Watch
//! mode is the one long-running exception, built for a person editing `.strok`
//! source by hand who wants to see the render update as they save. It serves a
//! small local page that re-renders on every file change over Server-Sent
//! Events. Plain std only — no async runtime, no watcher crate (mtime+content
//! polling survives editors that save via rename), no web framework.

mod editing;
pub(crate) mod notes;
mod projection;

use anyhow::{Context, Result};
use editing::FileEdit;
use notes::Note;
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
    /// Colorscheme applied to renders; switchable live from the preview page.
    /// `None` renders the base palette.
    scheme: Option<String>,
    /// Scheme names the document defines (drives the page's scheme picker).
    schemes: Vec<String>,
    /// Pinned review notes, mirrored to the `<file>.notes.json` sidecar.
    notes: Vec<Note>,
}

struct Shared {
    state: Mutex<State>,
    /// Serialize browser edits so two tabs cannot interleave file rewrites.
    edit: Mutex<EditHistory>,
    changed: Condvar,
    /// `--on-note` hook: run this command for each newly pinned note.
    on_note: Option<String>,
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
    schemes: Vec<String>,
}

pub fn run(
    file: &Path,
    port: u16,
    scheme: Option<&str>,
    open_browser: bool,
    on_note: Option<&str>,
) -> Result<()> {
    if !file.exists() {
        anyhow::bail!(
            "'{}': file not found\n\nCreate it with: strok new {} 800x800",
            file.display(),
            file.display()
        );
    }

    let (svg, editor, targets, schemes, error) = match render_snapshot(file, scheme) {
        Ok(snapshot) => (
            Some(snapshot.svg),
            snapshot.editor,
            snapshot.targets,
            snapshot.schemes,
            None,
        ),
        Err(e) => (None, Json::array([]), Json::array([]), Vec::new(), Some(e)),
    };
    let notes = notes::load(&notes::sidecar_path(file)).unwrap_or_default();
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            version: 1,
            svg,
            editor,
            targets,
            error,
            scheme: scheme.map(str::to_string),
            schemes,
            notes,
        }),
        edit: Mutex::new(EditHistory::default()),
        changed: Condvar::new(),
        on_note: on_note.map(str::to_string),
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
        std::thread::spawn(move || watch_loop(&shared, &file));
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
        std::thread::spawn(move || {
            let _ = handle_client(stream, &shared, &name, &file);
        });
    }
    Ok(())
}

/// Poll the file and re-render on change. Content comparison (not just mtime)
/// avoids spurious re-renders from editors that touch without writing. The
/// notes sidecar is polled the same way so an agent deleting addressed notes
/// clears the pins live in the browser.
fn watch_loop(shared: &Shared, file: &Path) {
    let sidecar = notes::sidecar_path(file);
    let mut last_contents = std::fs::read(file).ok();
    let mut last_notes_raw = std::fs::read(&sidecar).ok();
    loop {
        std::thread::sleep(POLL_INTERVAL);
        let contents = std::fs::read(file).ok();
        // A rename-style save can momentarily leave the path unreadable; keep
        // the previous state and catch the real contents on the next poll.
        if contents != last_contents && contents.is_some() {
            if let Some(current) = contents.as_deref() {
                let mut history = shared.edit.lock().unwrap();
                if !history.matches(current) {
                    history.clear();
                }
            }
            last_contents = contents;
            rerender_into_state(shared, file);
        }

        let notes_raw = std::fs::read(&sidecar).ok();
        if notes_raw != last_notes_raw {
            last_notes_raw = notes_raw;
            // `None` means the sidecar is unreadable as a note list (likely
            // another writer mid-save); keep the current set and retry when
            // its contents move again.
            if let Some(parsed) = notes::load(&sidecar) {
                let mut state = shared.state.lock().unwrap();
                if parsed != state.notes {
                    let fresh: Vec<Note> = parsed
                        .iter()
                        .filter(|note| !state.notes.iter().any(|known| known.id == note.id))
                        .cloned()
                        .collect();
                    state.notes = parsed;
                    state.version += 1;
                    drop(state);
                    shared.changed.notify_all();
                    for note in &fresh {
                        notify_note(shared, file, note);
                    }
                }
            }
        }
    }
}

/// Re-render the document with the currently selected scheme and publish the
/// result (or the error) to every connected preview.
fn rerender_into_state(shared: &Shared, file: &Path) {
    let scheme = shared.state.lock().unwrap().scheme.clone();
    let result = render_snapshot(file, scheme.as_deref());
    let mut state = shared.state.lock().unwrap();
    state.version += 1;
    match result {
        Ok(snapshot) => {
            state.svg = Some(snapshot.svg);
            state.editor = snapshot.editor;
            state.targets = snapshot.targets;
            state.schemes = snapshot.schemes;
            state.error = None;
        }
        Err(e) => state.error = Some(e),
    }
    drop(state);
    shared.changed.notify_all();
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
        let schemes = loaded
            .scene
            .as_ref()
            .map(|s| s.palette.schemes.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let doc = match loaded.scene.as_ref() {
            Some(s) => Document::from_scene(resolve::apply_scheme(s, scheme)?),
            None => loaded,
        };
        Ok(Snapshot {
            svg: emit::emit_document(&doc),
            editor,
            targets,
            schemes,
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
        ("GET", "/selection.js") => {
            respond(&mut stream, "text/javascript; charset=utf-8", SELECTION_JS)
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
                (
                    "scheme",
                    match &state.scheme {
                        Some(s) => Json::str(s.clone()),
                        None => Json::Null,
                    },
                ),
                (
                    "schemes",
                    Json::array(state.schemes.iter().map(|s| Json::str(s.clone()))),
                ),
                ("notes", notes::to_json(&state.notes)),
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
            let scheme = shared.state.lock().unwrap().scheme.clone();
            let result = match fields.get("action").map(String::as_str) {
                Some("undo") => restore_history(file, &mut history, true),
                Some("redo") => restore_history(file, &mut history, false),
                _ => editing::apply(file, &fields).map(|edit| history.push(edit)),
            }
            .and_then(|()| render_snapshot(file, scheme.as_deref()).map_err(anyhow::Error::msg));
            match result {
                Ok(snapshot) => {
                    let mut state = shared.state.lock().unwrap();
                    state.version += 1;
                    state.svg = Some(snapshot.svg);
                    state.editor = snapshot.editor;
                    state.targets = snapshot.targets;
                    state.schemes = snapshot.schemes;
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
        ("POST", "/note") => {
            let body = String::from_utf8_lossy(&request_body);
            let fields = parse_form(&body);
            match apply_note_action(file, shared, &fields) {
                Ok(()) => respond(&mut stream, "application/json", "{\"ok\":true}"),
                Err(error) => respond_status(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    &error_text(&error),
                ),
            }
        }
        ("POST", "/scheme") => {
            let body = String::from_utf8_lossy(&request_body);
            let fields = parse_form(&body);
            match apply_scheme_choice(shared, &fields) {
                Ok(()) => {
                    rerender_into_state(shared, file);
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

/// Add, remove, or clear pinned review notes and mirror the result to the
/// sidecar before publishing it, so the file an agent reads never lags what
/// the reviewer sees.
fn apply_note_action(file: &Path, shared: &Shared, fields: &HashMap<String, String>) -> Result<()> {
    let mut state = shared.state.lock().unwrap();
    let mut updated = state.notes.clone();
    let mut added = None;
    match fields.get("action").map(String::as_str) {
        Some("add") => {
            let text = fields
                .get("text")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or_else(|| anyhow::anyhow!("a note needs text"))?;
            let coordinate = |name: &str| -> Result<f64> {
                let value: f64 = fields
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("missing '{}'", name))?
                    .parse()
                    .with_context(|| format!("'{}' is not a number", name))?;
                anyhow::ensure!(value.is_finite(), "'{}' must be finite", name);
                Ok(value)
            };
            let x = coordinate("x")?;
            let y = coordinate("y")?;
            let target = fields
                .get("target")
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string());
            let id = updated.iter().map(|note| note.id).max().unwrap_or(0) + 1;
            let note = Note {
                id,
                x,
                y,
                target,
                text,
                ack: false,
            };
            added = Some(note.clone());
            updated.push(note);
        }
        Some("remove") => {
            let id: u64 = fields
                .get("id")
                .ok_or_else(|| anyhow::anyhow!("missing 'id'"))?
                .parse()
                .with_context(|| "'id' is not a note id")?;
            let before = updated.len();
            updated.retain(|note| note.id != id);
            anyhow::ensure!(updated.len() < before, "no note with id {}", id);
        }
        Some("clear") => updated.clear(),
        _ => anyhow::bail!("unknown note action"),
    }
    notes::save(&notes::sidecar_path(file), &updated)?;
    state.notes = updated;
    state.version += 1;
    drop(state);
    shared.changed.notify_all();
    if let Some(note) = added {
        notify_note(shared, file, &note);
    }
    Ok(())
}

/// Run the `--on-note` hook with the note as a compact JSON object on stdin.
/// Fire-and-forget: a failing hook is reported to stderr and never blocks the
/// review loop.
fn notify_note(shared: &Shared, file: &Path, note: &Note) {
    let Some(cmd) = shared.on_note.clone() else {
        return;
    };
    let mut fields = vec![
        ("file".to_string(), Json::str(file.display().to_string())),
        ("id".to_string(), Json::num(note.id as f64)),
        (
            "at".to_string(),
            Json::array([Json::num(note.x), Json::num(note.y)]),
        ),
    ];
    if let Some(target) = &note.target {
        fields.push(("target".to_string(), Json::str(target.clone())));
    }
    fields.push(("text".to_string(), Json::str(note.text.clone())));
    let payload = Json::Object(fields).to_string_compact();
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", &cmd]);
            c
        };
        #[cfg(not(target_os = "windows"))]
        let mut command = {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", &cmd]);
            c
        };
        let spawned = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.take() {
                    let mut stdin = stdin;
                    let _ = writeln!(stdin, "{}", payload);
                }
                let _ = child.wait();
            }
            Err(e) => eprintln!("note: --on-note hook failed to start: {e}"),
        }
    });
}

/// Switch the colorscheme used for renders. An empty value returns to the base
/// palette; unknown names are rejected against the document's defined schemes.
fn apply_scheme_choice(shared: &Shared, fields: &HashMap<String, String>) -> Result<()> {
    let choice = fields
        .get("scheme")
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'scheme'"))?;
    let mut state = shared.state.lock().unwrap();
    if choice.is_empty() {
        state.scheme = None;
        return Ok(());
    }
    anyhow::ensure!(
        state.schemes.iter().any(|name| name == choice),
        "unknown scheme '{}' (defined schemes: {})",
        choice,
        if state.schemes.is_empty() {
            "(none)".to_string()
        } else {
            state.schemes.join(", ")
        }
    );
    state.scheme = Some(choice.to_string());
    Ok(())
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
const SELECTION_JS: &str = include_str!("watch_ui/selection.mjs");
