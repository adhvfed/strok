//! The `Target` trait and the `CodeTarget<B>` strategy harness.
//!
//! `Target` is the one outbound extension point (design doc §4): it turns a
//! `Scene` into a downstream artifact — serialization, image, or framework
//! code. `CodeTarget<B>` is the harness that makes the "co-equal backends, no
//! golden reference" rule structural rather than aspirational:
//!
//! - It owns the *single* `Scene → UiDoc` lowering.
//! - Backends implement [`FrameworkBackend`], which only ever sees the
//!   already-lowered [`UiDoc`]. A backend literally cannot reach the `Scene`,
//!   so it cannot grow a private lowering path that drifts from the others.

use std::fmt;

use strok_core::scene::Scene;

use crate::ir::UiDoc;
use crate::lower;

/// What a target can express. Drives graceful, *declared* degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub vector: bool,
    pub raster: bool,
    pub components: bool,
    pub auto_layout: bool,
    pub interactivity: bool,
}

/// Options threaded into emission.
#[derive(Debug, Clone, Default)]
pub struct EmitOptions {
    /// Override the emitted component name (defaults to a name derived from the doc).
    pub component_name: Option<String>,
    /// Resolve palette tokens against this colorscheme for the rasterizable
    /// (inline-SVG) parts. `None` = base palette.
    pub scheme: Option<String>,
}

/// One emitted file, path relative to an output root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitFile {
    pub path: String,
    pub contents: String,
}

/// A raster asset that must be written alongside the emitted files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRef {
    pub name: String,
}

/// The result of an emission: files, side-car assets, and diagnostics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmitArtifact {
    pub files: Vec<EmitFile>,
    pub assets: Vec<AssetRef>,
    pub diagnostics: Vec<String>,
}

/// Lowering / emission errors.
#[derive(Debug)]
pub enum TargetError {
    /// A `strok-core` operation failed (e.g. unknown colorscheme).
    Core(String),
    /// The target can't express something and there's no fallback.
    Unsupported(String),
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetError::Core(m) => write!(f, "{m}"),
            TargetError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for TargetError {}

impl From<strok_core::error::StrokError> for TargetError {
    fn from(e: strok_core::error::StrokError) -> Self {
        TargetError::Core(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TargetError>;

/// A projection of a document to a downstream artifact.
pub trait Target {
    /// Stable id: `"react"`, `"solid"`, `"vanilla"`, `"tailwind"`.
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn emit(&self, scene: &Scene, opts: &EmitOptions) -> Result<EmitArtifact>;
}

/// Renders the neutral [`UiDoc`] into one framework's source.
///
/// Backends receive only the lowered IR — never the `Scene`. This is the
/// enforcement mechanism for co-equal targets: shared lowering is not a
/// convention a backend could bypass, it's the only input a backend is given.
pub trait FrameworkBackend {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn render(&self, doc: &UiDoc, opts: &EmitOptions) -> EmitArtifact;
}

/// A `Target` that lowers once, then delegates to a swappable backend.
pub struct CodeTarget<B: FrameworkBackend> {
    backend: B,
}

impl<B: FrameworkBackend> CodeTarget<B> {
    pub fn new(backend: B) -> Self {
        CodeTarget { backend }
    }
}

impl<B: FrameworkBackend> Target for CodeTarget<B> {
    fn id(&self) -> &'static str {
        self.backend.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    fn emit(&self, scene: &Scene, opts: &EmitOptions) -> Result<EmitArtifact> {
        // The single, shared lowering path. Every framework backend is fed
        // the same UiDoc — there is no per-backend Scene access by design.
        // Each `component` lowers to its own UiDoc → its own emitted file
        // (C8 / E4.2), still through the one lowering, still backend-blind.
        let mut docs = vec![lower::lower_scene(scene, opts)?];
        docs.extend(lower::lower_components(scene, opts)?);

        let mut artifact = EmitArtifact::default();
        for doc in &docs {
            let mut sub = self.backend.render(doc, opts);
            artifact.files.append(&mut sub.files);
            artifact.assets.append(&mut sub.assets);
            artifact.diagnostics.extend(doc.diagnostics.clone());
            artifact.diagnostics.append(&mut sub.diagnostics);
        }
        Ok(artifact)
    }
}
