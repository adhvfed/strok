//! `strok-render` — rasterizes resolved SVG to PNG via the pinned resvg chain.
//!
//! No-panic policy (E1.4, extended to this crate in C2): library code must not
//! `unwrap`/`expect`/`panic!` outside `#[cfg(test)]`; fallible paths return a
//! `Result`. The render hot path was audited panic-free in C1; this deny keeps
//! it that way.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod diff;
mod render;
mod sheet;

pub use diff::{
    compare, decode_png, diff_png_bytes, encode_png, DiffError, DiffStats,
    GOLDEN_FRACTION_TOLERANCE, GOLDEN_MEAN_TOLERANCE, PER_PIXEL_CHANGE_THRESHOLD,
};
pub use render::{
    render_svg_string, render_to_png, target_dimensions, RenderError, RenderOptions, RenderRegion,
};
pub use sheet::{contact_sheet, SheetOptions, SheetTile};
