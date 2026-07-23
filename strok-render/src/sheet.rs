//! Contact-sheet compositor (C10 / E5.1).
//!
//! A *contact sheet* is a single PNG laying every icon of a set out on a grid —
//! the at-a-glance overview a design-system icon pipeline needs (feedback #4: we
//! used to shell out to ImageMagick `montage`). This composites the already-
//! rendered per-icon PNG tiles into one image in pure Rust on top of the `image`
//! crate, so there is no external dependency and the output is deterministic
//! (and therefore golden-testable).
//!
//! Tiles are placed left-to-right, top-to-bottom, in the order they are given
//! (the `batch` path sorts files, so the sheet order is stable). Each tile is
//! drawn into a fixed `cell` square; tiles smaller than the cell are centered.
//! The background is configurable (default transparent).

use image::{Rgba, RgbaImage};

use crate::diff::{decode_png, encode_png, DiffError};

/// One labelled tile for the contact sheet: a name (for ordering/debugging) and
/// the icon's PNG bytes.
pub struct SheetTile {
    pub name: String,
    pub png: Vec<u8>,
}

/// Layout/style options for the contact sheet.
pub struct SheetOptions {
    /// Number of columns. Tiles wrap to the next row after this many.
    pub columns: u32,
    /// Padding (px) around every tile inside its cell.
    pub padding: u32,
    /// Background color `#rrggbb` (and optional `#rrggbbaa`); `None` = transparent.
    pub background: Option<String>,
}

impl Default for SheetOptions {
    fn default() -> Self {
        SheetOptions {
            columns: 8,
            padding: 8,
            background: None,
        }
    }
}

/// Composite `tiles` into one contact-sheet PNG. Tiles are assumed square-ish but
/// any size is handled: the cell is the max tile dimension, and each tile is
/// centered in its cell. Returns the encoded PNG bytes.
pub fn contact_sheet(tiles: &[SheetTile], opts: &SheetOptions) -> Result<Vec<u8>, DiffError> {
    if tiles.is_empty() {
        return Err(DiffError::Decode("contact sheet: no tiles".to_string()));
    }
    let columns = opts.columns.max(1);

    // Decode every tile and find the cell size (max width/height across tiles).
    let decoded: Vec<RgbaImage> = tiles
        .iter()
        .map(|t| decode_png(&t.png))
        .collect::<Result<_, _>>()?;
    let mut cell_w = 0u32;
    let mut cell_h = 0u32;
    for img in &decoded {
        let (w, h) = img.dimensions();
        cell_w = cell_w.max(w);
        cell_h = cell_h.max(h);
    }

    let n = decoded.len() as u32;
    let rows = n.div_ceil(columns);
    let pad = opts.padding;
    let stride_w = cell_w + pad * 2;
    let stride_h = cell_h + pad * 2;
    let sheet_w = stride_w * columns;
    let sheet_h = stride_h * rows;

    let bg = opts
        .background
        .as_deref()
        .and_then(parse_rgba)
        .unwrap_or(Rgba([0, 0, 0, 0]));
    let mut sheet = RgbaImage::from_pixel(sheet_w, sheet_h, bg);

    for (i, img) in decoded.iter().enumerate() {
        let col = (i as u32) % columns;
        let row = (i as u32) / columns;
        let (w, h) = img.dimensions();
        // Center the tile in its cell.
        let ox = col * stride_w + pad + (cell_w - w) / 2;
        let oy = row * stride_h + pad + (cell_h - h) / 2;
        blit(&mut sheet, img, ox, oy);
    }

    encode_png(&sheet)
}

/// Alpha-over composite `src` onto `dst` at `(ox, oy)`. Straight-alpha source
/// over straight-alpha destination (the icons render premultiplied-free RGBA
/// from tiny-skia's `encode_png`, which writes straight alpha).
fn blit(dst: &mut RgbaImage, src: &RgbaImage, ox: u32, oy: u32) {
    let (sw, sh) = src.dimensions();
    let (dw, dh) = dst.dimensions();
    for sy in 0..sh {
        let dy = oy + sy;
        if dy >= dh {
            break;
        }
        for sx in 0..sw {
            let dx = ox + sx;
            if dx >= dw {
                break;
            }
            let s = src.get_pixel(sx, sy);
            let d = dst.get_pixel(dx, dy);
            dst.put_pixel(dx, dy, over(*s, *d));
        }
    }
}

/// Porter-Duff "source over" for straight-alpha RGBA8.
fn over(s: Rgba<u8>, d: Rgba<u8>) -> Rgba<u8> {
    let sa = s.0[3] as f32 / 255.0;
    if sa >= 1.0 {
        return s;
    }
    if sa <= 0.0 {
        return d;
    }
    let da = d.0[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let blend = |sc: u8, dc: u8| -> u8 {
        let sc = sc as f32 / 255.0;
        let dc = dc as f32 / 255.0;
        let v = (sc * sa + dc * da * (1.0 - sa)) / out_a;
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Rgba([
        blend(s.0[0], d.0[0]),
        blend(s.0[1], d.0[1]),
        blend(s.0[2], d.0[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

/// Parse `#rrggbb` or `#rrggbbaa` into an RGBA pixel. `None` on a malformed value
/// (the caller falls back to transparent).
fn parse_rgba(s: &str) -> Option<Rgba<u8>> {
    let s = s.trim_start_matches('#');
    let hx = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    match s.len() {
        6 => Some(Rgba([hx(0)?, hx(2)?, hx(4)?, 255])),
        8 => Some(Rgba([hx(0)?, hx(2)?, hx(4)?, hx(6)?])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_png(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba(px));
        encode_png(&img).unwrap()
    }

    #[test]
    fn empty_tiles_is_error() {
        assert!(contact_sheet(&[], &SheetOptions::default()).is_err());
    }

    #[test]
    fn composites_grid_with_expected_dimensions() {
        let tiles: Vec<SheetTile> = (0..5)
            .map(|i| SheetTile {
                name: format!("t{i}"),
                png: solid_png(24, 24, [10 + i as u8, 0, 0, 255]),
            })
            .collect();
        let opts = SheetOptions {
            columns: 3,
            padding: 4,
            background: Some("#0d1117".to_string()),
        };
        let png = contact_sheet(&tiles, &opts).unwrap();
        let img = decode_png(&png).unwrap();
        // 5 tiles, 3 cols => 2 rows. stride = 24 + 8 = 32.
        assert_eq!(img.dimensions(), (32 * 3, 32 * 2));
        // Background present at a corner (opaque since cells are smaller? no —
        // tile fills cell minus padding; corner padding is background).
        assert_eq!(img.get_pixel(0, 0), &Rgba([0x0d, 0x11, 0x17, 255]));
    }

    #[test]
    fn transparent_background_default() {
        let tiles = vec![SheetTile {
            name: "x".into(),
            png: solid_png(10, 10, [255, 0, 0, 255]),
        }];
        let png = contact_sheet(&tiles, &SheetOptions::default()).unwrap();
        let img = decode_png(&png).unwrap();
        // Corner is padding => transparent.
        assert_eq!(img.get_pixel(0, 0)[3], 0);
    }
}
