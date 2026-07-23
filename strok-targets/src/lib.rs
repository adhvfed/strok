//! # strok-targets
//!
//! Code-emit targets for Strøk. Turns a `strok_core::scene::Scene` into
//! downstream artifacts — React, Solid, Vanilla, or a Tailwind `@theme` — via a
//! single neutral intermediate representation ([`ir::UiDoc`]).
//!
//! ## Architecture (design doc §4)
//!
//! ```text
//! Scene ──lower_scene()──► UiDoc ──┬─► CodeTarget<ReactBackend>   → React .tsx
//!        (one shared path)          ├─► CodeTarget<SolidBackend>   → Solid .tsx
//!                                   └─► CodeTarget<VanillaBackend> → HTML .ts
//! Scene ─────────────────────────────► TailwindTarget             → @theme css
//! ```
//!
//! The point of the [`target::CodeTarget`] harness is to make "React and Solid
//! are co-equal, with no golden reference" a *structural* guarantee: the
//! `Scene → UiDoc` lowering happens once, inside the harness, and a
//! [`target::FrameworkBackend`] only ever sees the lowered [`ir::UiDoc`]. No
//! backend can reach the `Scene`, so none can grow a private lowering path.
//!
//! No-panic policy (E1.4, extended to this crate in C2): library code must not
//! `unwrap`/`expect`/`panic!` outside `#[cfg(test)]`; fallible paths return a
//! `Result`. The lowering/emit paths were audited panic-free in C1.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod backends;
pub mod dtcg;
pub mod ir;
pub mod lower;
pub mod tailwind;
pub mod target;

pub use backends::{ReactBackend, SolidBackend, VanillaBackend};
pub use dtcg::DtcgTarget;
pub use ir::UiDoc;
pub use lower::lower_scene;
pub use tailwind::TailwindTarget;
pub use target::{
    Capabilities, CodeTarget, EmitArtifact, EmitFile, EmitOptions, FrameworkBackend, Target,
    TargetError,
};

/// Resolve a target by its stable id. The single registry used by the CLI.
pub fn target_by_id(id: &str) -> Option<Box<dyn Target>> {
    match id {
        "react" => Some(Box::new(CodeTarget::new(ReactBackend))),
        "solid" => Some(Box::new(CodeTarget::new(SolidBackend))),
        "vanilla" => Some(Box::new(CodeTarget::new(VanillaBackend))),
        "tailwind" => Some(Box::new(TailwindTarget)),
        "dtcg" => Some(Box::new(DtcgTarget)),
        _ => None,
    }
}

/// The ids of every registered target, for help text and validation.
pub const TARGET_IDS: &[&str] = &["react", "solid", "vanilla", "tailwind", "dtcg"];
