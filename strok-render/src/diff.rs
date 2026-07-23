//! Perceptual image diff (E3.3).
//!
//! This is the *same* comparator the golden harness uses (E1.2), promoted from
//! the `tests/golden.rs` test into the library so both the golden suite and the
//! `strok diff` CLI verb share one implementation — never two that can drift.
//!
//! The metric is perceptual, not byte-equality: anti-aliasing differs across
//! resvg/OS builds, so a clean re-render must still compare equal. A pair passes
//! the golden tolerance when BOTH the mean absolute per-channel difference and
//! the fraction of "materially changed" pixels are within their thresholds.

use image::RgbaImage;

/// A pixel is "materially changed" if any channel moves by more than this.
/// Shared with the golden harness so the visible diff and the gate agree.
pub const PER_PIXEL_CHANGE_THRESHOLD: u8 = 40;

/// Golden-suite mean tolerance (mean absolute per-channel delta, out of 255).
pub const GOLDEN_MEAN_TOLERANCE: f64 = 6.0;

/// Golden-suite changed-fraction tolerance (fraction of pixels that may differ).
pub const GOLDEN_FRACTION_TOLERANCE: f64 = 0.06;

/// Perceptual statistics for a pair of equally-sized images.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffStats {
    /// Mean absolute per-channel difference (0..=255).
    pub mean_abs: f64,
    /// Fraction of pixels that changed materially (0..=1).
    pub changed_fraction: f64,
    /// Count of materially-changed pixels.
    pub changed_pixels: u64,
    /// Total pixel count.
    pub total_pixels: u64,
    /// Bounding box of changed pixels: `(x0, y0, x1, y1)` inclusive, or `None`
    /// if nothing changed. Lets callers highlight / crop the changed region.
    pub changed_bbox: Option<(u32, u32, u32, u32)>,
}

impl DiffStats {
    /// Whether the pair is within the golden suite's perceptual tolerance.
    pub fn within_golden_tolerance(&self) -> bool {
        self.mean_abs <= GOLDEN_MEAN_TOLERANCE && self.changed_fraction <= GOLDEN_FRACTION_TOLERANCE
    }
}

/// Error from a diff operation.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("image decode error: {0}")]
    Decode(String),
    #[error("image encode error: {0}")]
    Encode(String),
    #[error("size mismatch: {0:?} vs {1:?}")]
    SizeMismatch((u32, u32), (u32, u32)),
}

/// Decode PNG bytes into an RGBA8 image.
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, DiffError> {
    image::load_from_memory(bytes)
        .map(|img| img.to_rgba8())
        .map_err(|e| DiffError::Decode(e.to_string()))
}

/// Encode an RGBA8 image as PNG bytes.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, DiffError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| DiffError::Encode(e.to_string()))?;
    Ok(buf.into_inner())
}

/// Compare two equally-sized RGBA images, returning perceptual stats and a
/// visible diff image (red where pixels changed materially; the unchanged
/// background dimmed so changes stand out). This is the exact computation the
/// golden harness performs.
pub fn compare(a: &RgbaImage, b: &RgbaImage) -> Result<(DiffStats, RgbaImage), DiffError> {
    if a.dimensions() != b.dimensions() {
        return Err(DiffError::SizeMismatch(a.dimensions(), b.dimensions()));
    }
    let (w, h) = a.dimensions();
    let mut total_abs: f64 = 0.0;
    let mut changed: u64 = 0;
    let mut diff = RgbaImage::new(w, h);
    let n_channels = (w as u64) * (h as u64) * 4;
    let mut bbox: Option<(u32, u32, u32, u32)> = None;

    for (x, y, pa) in a.enumerate_pixels() {
        let pb = b.get_pixel(x, y);
        let mut max_chan_delta = 0u8;
        for c in 0..4 {
            let d = (pa[c] as i32 - pb[c] as i32).unsigned_abs() as u8;
            total_abs += d as f64;
            if d > max_chan_delta {
                max_chan_delta = d;
            }
        }
        if max_chan_delta > PER_PIXEL_CHANGE_THRESHOLD {
            changed += 1;
            diff.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        } else {
            // Dim the unchanged background so the red changes stand out.
            diff.put_pixel(x, y, image::Rgba([pb[0] / 3, pb[1] / 3, pb[2] / 3, 255]));
        }
    }

    let total_pixels = (w as u64) * (h as u64);
    let stats = DiffStats {
        mean_abs: total_abs / n_channels as f64,
        changed_fraction: changed as f64 / total_pixels as f64,
        changed_pixels: changed,
        total_pixels,
        changed_bbox: bbox,
    };
    Ok((stats, diff))
}

/// Convenience: diff two PNG byte buffers, returning stats and the diff PNG.
pub fn diff_png_bytes(a: &[u8], b: &[u8]) -> Result<(DiffStats, Vec<u8>), DiffError> {
    let ia = decode_png(a)?;
    let ib = decode_png(b)?;
    let (stats, diff) = compare(&ia, &ib)?;
    let png = encode_png(&diff)?;
    Ok((stats, png))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(px))
    }

    #[test]
    fn identical_images_have_no_change() {
        let a = solid(8, 8, [10, 20, 30, 255]);
        let (stats, _) = compare(&a, &a).unwrap();
        assert_eq!(stats.changed_pixels, 0);
        assert_eq!(stats.changed_fraction, 0.0);
        assert!(stats.changed_bbox.is_none());
        assert!(stats.within_golden_tolerance());
    }

    #[test]
    fn change_region_is_localized_in_bbox() {
        let a = solid(10, 10, [0, 0, 0, 255]);
        let mut b = a.clone();
        // Flip a 2x2 block at (3,4)..(4,5) to white.
        for y in 4..6 {
            for x in 3..5 {
                b.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let (stats, _) = compare(&a, &b).unwrap();
        assert_eq!(stats.changed_pixels, 4);
        assert_eq!(stats.changed_bbox, Some((3, 4, 4, 5)));
        assert!(
            !stats.within_golden_tolerance() || stats.changed_fraction <= GOLDEN_FRACTION_TOLERANCE
        );
    }

    #[test]
    fn size_mismatch_errors() {
        let a = solid(4, 4, [0, 0, 0, 255]);
        let b = solid(5, 4, [0, 0, 0, 255]);
        assert!(matches!(compare(&a, &b), Err(DiffError::SizeMismatch(..))));
    }

    #[test]
    fn png_roundtrip_diff() {
        let a = solid(6, 6, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        let pa = encode_png(&a).unwrap();
        let pb = encode_png(&b).unwrap();
        let (stats, diff_png) = diff_png_bytes(&pa, &pb).unwrap();
        assert_eq!(stats.changed_pixels, 1);
        // The diff PNG decodes and is the same size.
        let decoded = decode_png(&diff_png).unwrap();
        assert_eq!(decoded.dimensions(), (6, 6));
    }
}
