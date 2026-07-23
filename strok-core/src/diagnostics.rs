//! DSL diagnostics (C6 / E3.1).
//!
//! A [`Diagnostic`] carries a human message plus *position* (1-based line and
//! column, byte span into the source) and an optional "did you mean …"
//! [`suggestion`](Diagnostic::suggestion). [`Diagnostic::render`] formats it
//! with a caret snippet pointing at the offending column:
//!
//! ```text
//! error: unknown operation 'storke'
//!  --> line 4, column 3
//!   |
//! 4 |   storke #f00
//!   |   ^^^^^^ did you mean `stroke`?
//! ```
//!
//! [`suggest`] is a small Levenshtein-based "did you mean" engine used at every
//! dispatch site where a keyword/attribute is rejected.
//!
//! Error *recovery* (`parse_file_recover`) collects many [`Diagnostic`]s and
//! keeps parsing, instead of aborting on the first bad line — so a GUI/MCP can
//! show every problem at once and still get a partial scene back.

/// One diagnostic: a positioned, optionally-suggested message.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// 1-based line number (matches editor gutters). 0 ⇒ unknown / whole-file.
    pub line: usize,
    /// 1-based column of the offending token. 0 ⇒ unknown.
    pub column: usize,
    /// Number of columns to underline with carets (>=1 when `column` > 0).
    pub width: usize,
    /// The human-readable problem.
    pub message: String,
    /// Optional "did you mean `X`?" replacement suggestion.
    pub suggestion: Option<String>,
    /// The raw source line text (for the caret snippet). Empty ⇒ no snippet.
    pub source_line: String,
}

impl Diagnostic {
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Diagnostic {
            line,
            column: 0,
            width: 0,
            message: message.into(),
            suggestion: None,
            source_line: String::new(),
        }
    }

    pub fn with_span(mut self, column: usize, width: usize) -> Self {
        self.column = column;
        self.width = width.max(1);
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_source(mut self, source_line: impl Into<String>) -> Self {
        self.source_line = source_line.into();
        self
    }

    /// Render the diagnostic as a multi-line message with a caret snippet.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("error: ");
        out.push_str(&self.message);
        if self.line > 0 {
            if self.column > 0 {
                out.push_str(&format!(
                    "\n --> line {}, column {}",
                    self.line, self.column
                ));
            } else {
                out.push_str(&format!("\n --> line {}", self.line));
            }
        }
        if !self.source_line.is_empty() && self.line > 0 {
            let gutter = self.line.to_string();
            let pad = " ".repeat(gutter.len());
            out.push_str(&format!("\n{} |", pad));
            out.push_str(&format!("\n{} | {}", gutter, self.source_line));
            if self.column > 0 {
                // Caret line: account for the leading column offset (1-based).
                let lead = " ".repeat(self.column.saturating_sub(1));
                let carets = "^".repeat(self.width.max(1));
                out.push_str(&format!("\n{} | {}{}", pad, lead, carets));
                if let Some(s) = &self.suggestion {
                    out.push_str(&format!(" did you mean `{}`?", s));
                }
            } else if let Some(s) = &self.suggestion {
                out.push_str(&format!("\n{} | did you mean `{}`?", pad, s));
            }
        } else if let Some(s) = &self.suggestion {
            out.push_str(&format!("\n  = did you mean `{}`?", s));
        }
        out
    }
}

/// Levenshtein edit distance (small inputs; classic two-row DP).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Pick the closest candidate to `input` from `candidates` if it is "close
/// enough" (edit distance within a third of the input length, min 1). Returns
/// the best match or `None`. This powers every "did you mean" hint.
pub fn suggest<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let max_dist = (input.chars().count() / 3).max(1) + 1;
    let mut best: Option<(&str, usize)> = None;
    for &cand in candidates {
        let d = levenshtein(input, cand);
        if d == 0 {
            continue; // identical — not a typo
        }
        if d <= max_dist {
            match best {
                Some((_, bd)) if bd <= d => {}
                _ => best = Some((cand, d)),
            }
        }
    }
    best.map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("storke", "stroke"), 2);
        assert_eq!(levenshtein("fill", "fill"), 0);
    }

    #[test]
    fn suggest_finds_close_typo() {
        let kw = ["fill", "stroke", "stroke-width", "opacity"];
        assert_eq!(suggest("storke", &kw), Some("stroke"));
        assert_eq!(suggest("fil", &kw), Some("fill"));
        assert_eq!(suggest("opacty", &kw), Some("opacity"));
    }

    #[test]
    fn suggest_rejects_far() {
        let kw = ["fill", "stroke"];
        assert_eq!(suggest("xyzzy", &kw), None);
    }

    #[test]
    fn render_has_caret_and_suggestion() {
        let d = Diagnostic::new(4, "unknown operation 'storke'")
            .with_span(3, 6)
            .with_suggestion("stroke")
            .with_source("  storke #f00");
        let r = d.render();
        assert!(r.contains("line 4, column 3"), "{r}");
        assert!(r.contains("^^^^^^"), "{r}");
        assert!(r.contains("did you mean `stroke`?"), "{r}");
    }
}
