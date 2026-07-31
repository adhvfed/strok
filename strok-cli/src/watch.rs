//! `strok watch` — live preview server for humans working in an editor.
//!
//! Strøk is file-first: every CLI command reads the document and exits. Watch
//! mode is the one long-running exception, built for a person editing `.strok`
//! source by hand who wants to see the render update as they save. It serves a
//! small local page that re-renders on every file change over Server-Sent
//! Events. Plain std only — no async runtime, no watcher crate (mtime+content
//! polling survives editors that save via rename), no web framework.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use strok_core::document::Document;
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
    error: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

pub fn run(file: &Path, port: u16, scheme: Option<&str>, open_browser: bool) -> Result<()> {
    if !file.exists() {
        anyhow::bail!(
            "'{}': file not found\n\nCreate it with: strok new {} 800x800",
            file.display(),
            file.display()
        );
    }

    let (svg, error) = match render_svg(file, scheme) {
        Ok(svg) => (Some(svg), None),
        Err(e) => (None, Some(e)),
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            version: 1,
            svg,
            error,
        }),
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
        std::thread::spawn(move || {
            let _ = handle_client(stream, &shared, &name);
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
        last_contents = contents;
        let result = render_svg(file, scheme);
        let mut state = shared.state.lock().unwrap();
        state.version += 1;
        match result {
            Ok(svg) => {
                state.svg = Some(svg);
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
fn render_svg(path: &Path, scheme: Option<&str>) -> std::result::Result<String, String> {
    let render = || -> Result<String> {
        let loaded =
            Document::load(path).with_context(|| format!("failed to load '{}'", path.display()))?;
        let doc = match loaded.scene.as_ref() {
            Some(s) => Document::from_scene(resolve::apply_scheme(s, scheme)?),
            None => loaded,
        };
        Ok(emit::emit_document(&doc))
    };
    render().map_err(|e| error_text(&e))
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

fn handle_client(stream: TcpStream, shared: &Shared, display_name: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain headers; this server only ever cares about the request target.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");

    let mut stream = stream;
    match path {
        "/" => respond(&mut stream, "text/html; charset=utf-8", PREVIEW_HTML),
        "/state.json" => {
            let state = shared.state.lock().unwrap();
            let body = Json::obj([
                ("version", Json::num(state.version as f64)),
                ("file", Json::str(display_name)),
                (
                    "svg",
                    match &state.svg {
                        Some(svg) => Json::str(svg.clone()),
                        None => Json::Null,
                    },
                ),
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
        "/events" => serve_events(&mut stream, shared),
        _ => {
            let body = "not found";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
        }
    }
}

fn respond(stream: &mut TcpStream, content_type: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        content_type,
        body.len()
    )?;
    stream.write_all(body.as_bytes())
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
    --bg: #17181b; --panel: #1f2126; --edge: #2c2f36;
    --text: #d6d8de; --dim: #7d818c; --ok: #4cc38a; --err: #e5484d;
  }
  * { box-sizing: border-box; margin: 0; }
  html, body { height: 100%; }
  body {
    background: var(--bg); color: var(--text);
    font: 13px/1.5 ui-sans-serif, system-ui, sans-serif;
    display: flex; flex-direction: column;
  }
  header {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 14px; border-bottom: 1px solid var(--edge);
    background: var(--panel); flex: none;
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--dim); flex: none; }
  .dot.ok { background: var(--ok); }
  .dot.err { background: var(--err); }
  .name { font-weight: 600; }
  .meta { color: var(--dim); font-variant-numeric: tabular-nums; }
  .spacer { flex: 1; }
  button {
    background: none; border: 1px solid var(--edge); border-radius: 6px;
    color: var(--text); font: inherit; padding: 2px 10px; cursor: pointer;
  }
  button:hover { border-color: var(--dim); }
  #errorbar {
    flex: none; max-height: 40vh; overflow: auto;
    border-bottom: 1px solid var(--err); background: #2a1518; color: #f2b8ba;
    padding: 10px 14px; white-space: pre; font: 12px/1.5 ui-monospace, monospace;
  }
  #errorbar[hidden] { display: none; }
  main { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; padding: 24px; }
  #stage {
    max-width: 100%; max-height: 100%; width: 100%; height: 100%;
    display: flex; align-items: center; justify-content: center;
    border-radius: 8px;
  }
  #stage.stale { opacity: 0.45; }
  #stage svg { max-width: 100%; max-height: 100%; display: block; }
  /* Backdrop cycle: checkerboard (transparency), white, black */
  #stage.checker {
    background:
      repeating-conic-gradient(#232529 0% 25%, #1b1d21 0% 50%) 0 0 / 20px 20px;
  }
  #stage.white { background: #ffffff; }
  #stage.black { background: #000000; }
  #empty { color: var(--dim); }
</style>
</head>
<body>
<header>
  <span class="dot" id="dot"></span>
  <span class="name" id="name">…</span>
  <span class="meta" id="size"></span>
  <span class="spacer"></span>
  <span class="meta" id="rev"></span>
  <button id="bgbtn" title="Cycle backdrop">backdrop</button>
</header>
<pre id="errorbar" hidden></pre>
<main><div id="stage" class="checker"><span id="empty">waiting for first render…</span></div></main>
<script>
  const $ = (id) => document.getElementById(id);
  const backdrops = ['checker', 'white', 'black'];
  let backdrop = 0;
  $('bgbtn').onclick = () => {
    $('stage').classList.remove(backdrops[backdrop]);
    backdrop = (backdrop + 1) % backdrops.length;
    $('stage').classList.add(backdrops[backdrop]);
  };

  let version = -1;
  async function refresh() {
    const res = await fetch('/state.json');
    const s = await res.json();
    version = s.version;
    document.title = s.file + ' — strøk';
    $('name').textContent = s.file;
    $('rev').textContent = 'rev ' + s.version;
    if (s.svg) {
      $('stage').innerHTML = s.svg;
      const svg = $('stage').querySelector('svg');
      if (svg) {
        const w = svg.getAttribute('width'), h = svg.getAttribute('height');
        if (!svg.getAttribute('viewBox') && w && h) {
          svg.setAttribute('viewBox', `0 0 ${parseFloat(w)} ${parseFloat(h)}`);
        }
        if (w && h) $('size').textContent = parseFloat(w) + '×' + parseFloat(h);
        svg.removeAttribute('width');
        svg.removeAttribute('height');
        svg.style.width = '100%';
        svg.style.height = '100%';
      }
    }
    $('errorbar').hidden = !s.error;
    $('errorbar').textContent = s.error || '';
    $('stage').classList.toggle('stale', !!s.error);
    $('dot').className = 'dot ' + (s.error ? 'err' : 'ok');
  }

  const es = new EventSource('/events');
  es.onmessage = (ev) => { if (Number(ev.data) !== version) refresh(); };
  es.onerror = () => { $('dot').className = 'dot'; };
  refresh();
</script>
</body>
</html>
"##;
