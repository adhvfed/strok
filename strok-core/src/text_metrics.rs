//! Estimated text metrics, so text elements get bounding boxes.
//!
//! Strøk's core is dependency-free and never shapes real glyphs — the raster
//! stack (usvg/fontdb) does that at render time. But anchors, `measure`,
//! `query` and relative placement all run in core, and before this module a
//! text element simply had **no bbox** (`bbox: null`), making labels invisible
//! to the whole verification layer.
//!
//! The estimate embeds the Helvetica AFM advance widths (the metrics Arial,
//! Liberation Sans and most default UI sans faces are built to match), scaled
//! by font-size, with a flat multiplier for bold. For the default sans stack
//! this is accurate to a few percent; for other faces it is an approximation.
//! Treat text bboxes as ≈, good for layout/alignment — not for hit-testing
//! glyph outlines.

/// Helvetica advance widths for ASCII 32..=126, in 1/1000 em units.
#[rustfmt::skip]
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, // ' ' ! " # $ % & ' ( )
    389, 584, 278, 333, 278, 278, 556, 556, 556, 556, // * + , - . / 0 1 2 3
    556, 556, 556, 556, 556, 556, 278, 278, 584, 584, // 4 5 6 7 8 9 : ; < =
    584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, // > ? @ A B C D E F G
    722, 278, 500, 667, 556, 833, 722, 778, 667, 778, // H I J K L M N O P Q
    722, 667, 611, 722, 667, 944, 667, 667, 611, 278, // R S T U V W X Y Z [
    278, 278, 469, 556, 333, 556, 556, 500, 556, 556, // \ ] ^ _ ` a b c d e
    278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // f g h i j k l m n o
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, // p q r s t u v w x y
    500, 334, 260, 334, 584,                          // z { | } ~
];

/// Fallback advance for characters outside the table (non-ASCII), 1/1000 em.
const DEFAULT_ADVANCE: u16 = 600;

/// Helvetica vertical metrics, 1/1000 em.
const ASCENT: f64 = 718.0;
const DESCENT: f64 = 207.0;

/// Extra width factor applied for bold weights (Helvetica-Bold runs ≈5% wider).
const BOLD_FACTOR: f64 = 1.05;

/// The font-size used when a text shape declares none — matches the usvg
/// default the raster stack applies to a `<text>` without `font-size`.
pub const DEFAULT_FONT_SIZE: f64 = 12.0;

/// Estimated metrics for a single-line run of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    /// Advance width of the run, in document units.
    pub width: f64,
    /// Distance from the baseline up to the top of the em box.
    pub ascent: f64,
    /// Distance from the baseline down to the bottom of the em box.
    pub descent: f64,
}

/// `true` for weights that should use the bold width factor. Accepts numeric
/// (`600`+) and keyword (`bold`, `bolder`) forms.
fn is_bold(weight: Option<&str>) -> bool {
    match weight {
        Some(w) => match w.trim().parse::<u32>() {
            Ok(n) => n >= 600,
            Err(_) => matches!(w.trim(), "bold" | "bolder"),
        },
        None => false,
    }
}

/// Estimate metrics for `content` at `font_size` (document units per em).
pub fn measure(content: &str, font_size: f64, font_weight: Option<&str>) -> TextMetrics {
    let mut units: f64 = 0.0;
    for ch in content.chars() {
        let code = ch as u32;
        units += if (32..=126).contains(&code) {
            f64::from(HELVETICA_WIDTHS[(code - 32) as usize])
        } else {
            f64::from(DEFAULT_ADVANCE)
        };
    }
    if is_bold(font_weight) {
        units *= BOLD_FACTOR;
    }
    TextMetrics {
        width: units * font_size / 1000.0,
        ascent: ASCENT * font_size / 1000.0,
        descent: DESCENT * font_size / 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_measure_at_helvetica_advances() {
        // "46%" at 20px: (556 + 556 + 889) / 1000 * 20 = 40.02
        let m = measure("46%", 20.0, None);
        assert!((m.width - 40.02).abs() < 1e-9, "{}", m.width);
        assert!((m.ascent - 14.36).abs() < 1e-9);
        assert!((m.descent - 4.14).abs() < 1e-9);
    }

    #[test]
    fn bold_widens_by_factor() {
        let regular = measure("abc", 16.0, Some("400")).width;
        let bold = measure("abc", 16.0, Some("700")).width;
        assert!((bold / regular - BOLD_FACTOR).abs() < 1e-9);
        assert_eq!(measure("abc", 16.0, Some("bold")).width, bold);
    }

    #[test]
    fn non_ascii_uses_fallback_advance() {
        let m = measure("ø", 10.0, None);
        assert!((m.width - 6.0).abs() < 1e-9);
    }

    #[test]
    fn empty_content_has_zero_width() {
        assert_eq!(measure("", 20.0, None).width, 0.0);
    }
}
