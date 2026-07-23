//! `strok-core` — the model, parser, resolver and SVG emitter for Strøk.
//!
//! Library code must not `unwrap`/`expect`/`panic!` —
//! fallible paths return `Result` via [`error`]. These lints are denied below;
//! test modules opt out via `#[cfg_attr(test, allow(...))]`. A small number of
//! provably-infallible sites carry a narrowly-scoped `#[allow]` with an
//! invariant note.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod attrs;
pub mod audit;
pub mod bool_ops;
pub mod diagnostics;
pub mod document;
pub mod dsl_emit;
pub mod dsl_parse;
pub mod emit;
pub mod error;
pub mod expr;
pub mod id;
pub mod import_svg;
pub mod json;
pub mod manifest;
pub mod measure;
pub mod node;
pub mod ops;
pub mod parse;
pub mod path_ops;
pub mod path_point;
pub mod query;
pub mod resolve;
pub mod scene;
pub mod shape;
pub mod stdlib;
pub mod stroke_outline;
pub mod text_metrics;
pub mod token_sync;
pub mod tree;
pub mod types;
