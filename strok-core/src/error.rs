use thiserror::Error;

#[derive(Debug, Error)]
pub enum StrokError {
    #[error("id '{0}' already exists")]
    IdConflict(String),

    #[error("id '{0}' not found")]
    IdNotFound(String),

    #[error("ambiguous name '{name}' — did you mean:\n{}", candidates.iter().map(|c| format!("  {}", c)).collect::<Vec<_>>().join("\n"))]
    AmbiguousName {
        name: String,
        candidates: Vec<String>,
    },

    #[error("'root' is a reserved id")]
    ReservedId,

    #[error("parse error: {0}")]
    ParseError(String),

    /// Multiple parse diagnostics collected via error recovery (E3.1). The
    /// `Display` lists each rendered diagnostic; structured access is via the
    /// `Vec<Diagnostic>` for callers (CLI, GUI/MCP) that want positions.
    #[error("{}", .0.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n\n"))]
    ParseDiagnostics(Vec<crate::diagnostics::Diagnostic>),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("invalid node index: {0}")]
    InvalidNodeIndex(u32),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, StrokError>;
