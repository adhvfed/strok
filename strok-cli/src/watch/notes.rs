//! Pinned review notes: the human-to-agent feedback channel of watch mode.
//!
//! The preview page lets the reviewer pin short notes onto the render. They
//! persist to a sidecar next to the watched document (`<file>.notes.json`) so
//! an agent can read the feedback when the file is handed back, and delete
//! entries (or the whole sidecar) as each note is addressed. The sidecar is
//! the source of truth: watch reloads it when it changes on disk, so pins
//! clear live in the browser while an agent works through them.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use strok_core::json::Json;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Note {
    pub(crate) id: u64,
    /// Pin position in document coordinates.
    pub(crate) x: f64,
    pub(crate) y: f64,
    /// Named placement the pin was dropped on, when it hit one.
    pub(crate) target: Option<String>,
    pub(crate) text: String,
    /// True once an agent has read the note (via `strok notes` or the MCP
    /// notes_list tool). The reviewer's pin shows it as seen.
    pub(crate) ack: bool,
}

/// `art.strok` → `art.strok.notes.json`, next to the document.
pub(crate) fn sidecar_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".notes.json");
    file.with_file_name(name)
}

/// Read notes from the sidecar. A missing file means no notes; a file that
/// does not parse as a note list (e.g. another writer mid-save) returns `None`
/// so the caller keeps the current set and retries on the next poll.
pub(crate) fn load(sidecar: &Path) -> Option<Vec<Note>> {
    match std::fs::read_to_string(sidecar) {
        Ok(raw) => parse(&raw),
        Err(_) => Some(Vec::new()),
    }
}

fn parse(raw: &str) -> Option<Vec<Note>> {
    let root = crate::mcp::parse_json(raw)?;
    let Json::Object(pairs) = root else {
        return None;
    };
    let list = pairs
        .iter()
        .find(|(key, _)| key == "notes")
        .map(|(_, v)| v)?;
    let Json::Array(items) = list else {
        return None;
    };
    let mut notes = Vec::new();
    for item in items {
        let Json::Object(fields) = item else {
            return None;
        };
        let get = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, v)| v);
        let id = match get("id") {
            Some(Json::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => *n as u64,
            _ => return None,
        };
        let (x, y) = match get("at") {
            Some(Json::Array(at)) if at.len() == 2 => match (&at[0], &at[1]) {
                (Json::Num(x), Json::Num(y)) if x.is_finite() && y.is_finite() => (*x, *y),
                _ => return None,
            },
            _ => return None,
        };
        let text = match get("text") {
            Some(Json::Str(text)) => text.clone(),
            _ => return None,
        };
        let target = match get("target") {
            Some(Json::Str(target)) => Some(target.clone()),
            None | Some(Json::Null) => None,
            _ => return None,
        };
        let ack = match get("ack") {
            Some(Json::Bool(ack)) => *ack,
            None | Some(Json::Null) => false,
            _ => return None,
        };
        notes.push(Note {
            id,
            x,
            y,
            target,
            text,
            ack,
        });
    }
    Some(notes)
}

/// Write the sidecar, or remove it when the last note is gone so a finished
/// review leaves nothing behind.
pub(crate) fn save(sidecar: &Path, notes: &[Note]) -> Result<()> {
    if notes.is_empty() {
        return match std::fs::remove_file(sidecar) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("failed to remove '{}'", sidecar.display())),
        };
    }
    let body = Json::obj([("notes", to_json(notes))]).to_string_pretty();
    std::fs::write(sidecar, body)
        .with_context(|| format!("failed to write '{}'", sidecar.display()))
}

pub(crate) fn to_json(notes: &[Note]) -> Json {
    Json::Array(
        notes
            .iter()
            .map(|note| {
                let mut fields = vec![
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
                if note.ack {
                    fields.push(("ack".to_string(), Json::Bool(true)));
                }
                Json::Object(fields)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_roundtrips_through_parse() {
        let notes = vec![
            Note {
                id: 1,
                x: 12.5,
                y: -3.0,
                target: Some("cup-rim".to_string()),
                text: "make this rounder".to_string(),
                ack: true,
            },
            Note {
                id: 2,
                x: 0.0,
                y: 0.0,
                target: None,
                text: "too much empty space".to_string(),
                ack: false,
            },
        ];
        let raw = Json::obj([("notes", to_json(&notes))]).to_string_pretty();
        assert_eq!(parse(&raw).unwrap(), notes);
    }

    #[test]
    fn malformed_sidecar_is_rejected_not_emptied() {
        assert_eq!(parse("{\"notes\": [{\"id\": 1}]}"), None);
        assert_eq!(parse("not json"), None);
        assert_eq!(parse("{\"other\": []}"), None);
    }

    #[test]
    fn sidecar_path_appends_to_the_document_name() {
        assert_eq!(
            sidecar_path(Path::new("art/cup.strok")),
            Path::new("art/cup.strok.notes.json")
        );
    }
}
