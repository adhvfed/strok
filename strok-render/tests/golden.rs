//! Golden-image render harness (E1.2).
//!
//! For each fixture `.strok` in `tests/golden/src/`, this harness has two gates:
//!
//!   1. **Textual-SVG snapshot (PRIMARY, authoritative).** `resolve_scene` output
//!      is snapshotted with `insta`. Deterministic, no anti-aliasing noise — this
//!      is the real fidelity gate. A one-character change to a `d` string fails
//!      here. Re-bless deliberately with `cargo insta review` / `cargo insta accept`.
//!
//!   2. **Perceptual PNG comparison (SECONDARY).** Geometry-only fixtures are
//!      rendered to PNG at sizes 24/96/512 and compared against
//!      `tests/golden/expected/<name>-<size>.png`
//!      with a *perceptual tolerance* (mean per-channel ΔE + fraction-of-changed-
//!      pixels). NEVER byte-equality — AA is platform-sensitive (E1.2 risk). On
//!      failure a diff PNG is written to `tests/golden/diff/` for human review.
//!      `STROK_BLESS=1` (re)generates the expected PNGs.
//!
//! The render crates (resvg/usvg/tiny-skia) are pinned in the workspace Cargo.toml
//! so the geometry PNG output is reproducible. Text fixtures are intentionally
//! excluded from pixel comparisons because system font availability and
//! rasterization differ across platforms; their SVG snapshots remain covered.

// Test harness: unwrap/expect/panic are fine here (a failed setup IS a test
// failure). The no-panic policy applies to library code, not test scaffolding.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use strok_core::document::Document;
use strok_core::resolve::resolve_scene;
// The perceptual comparator is the SHARED library function (E3.3): the golden
// gate and `strok diff` use the exact same metric — never two that can drift.
use strok_render::{
    compare, contact_sheet, decode_png as decode_png_lib, render_to_png, RenderOptions,
    SheetOptions, SheetTile, GOLDEN_FRACTION_TOLERANCE as FRACTION_TOLERANCE,
    GOLDEN_MEAN_TOLERANCE as MEAN_TOLERANCE,
};

/// Output sizes the perceptual PNG gate renders each fixture at.
const SIZES: [u32; 3] = [24, 96, 512];

/// Fixtures whose pixels depend on fonts installed on the host.
const FONT_DEPENDENT_FIXTURES: [&str; 3] = ["design-system-card", "text", "text-on-path"];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn blessing() -> bool {
    std::env::var("STROK_BLESS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Discover fixture stems (filenames without `.strok`), sorted for determinism.
fn fixture_stems() -> Vec<String> {
    let src = golden_dir().join("src");
    let mut stems: Vec<String> = fs::read_dir(&src)
        .unwrap_or_else(|e| panic!("read golden src dir {src:?}: {e}"))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("strok") {
                path.file_stem()?.to_str().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    stems.sort();
    assert!(!stems.is_empty(), "no fixtures found in {src:?}");
    stems
}

fn geometry_fixture_stems() -> Vec<String> {
    fixture_stems()
        .into_iter()
        .filter(|stem| !FONT_DEPENDENT_FIXTURES.contains(&stem.as_str()))
        .collect()
}

fn load_doc(stem: &str) -> Document {
    let path = golden_dir().join("src").join(format!("{stem}.strok"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path:?}: {e}"));
    Document::load_str_with_path(&text, &path)
        .unwrap_or_else(|e| panic!("parse fixture {path:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Gate 1: textual-SVG insta snapshots (authoritative).
// ---------------------------------------------------------------------------

#[test]
fn golden_svg_snapshots() {
    for stem in fixture_stems() {
        let doc = load_doc(&stem);
        let scene = doc
            .scene
            .as_ref()
            .unwrap_or_else(|| panic!("fixture {stem} has no scene"));
        let svg = resolve_scene(scene);
        // One snapshot per fixture, named after the fixture stem.
        insta::with_settings!({ snapshot_path => "golden/svg", prepend_module_to_snapshot => false }, {
            insta::assert_snapshot!(stem.as_str(), svg);
        });
    }
}

// ---------------------------------------------------------------------------
// Gate 2: perceptual PNG comparison.
// ---------------------------------------------------------------------------

/// Decode PNG bytes into an RGBA8 image (thin wrapper over the library decoder).
fn decode_png(bytes: &[u8]) -> image::RgbaImage {
    decode_png_lib(bytes).expect("decode PNG")
}

#[test]
fn golden_png_perceptual() {
    let dir = golden_dir();
    let expected_dir = dir.join("expected");
    let diff_dir = dir.join("diff");
    fs::create_dir_all(&expected_dir).unwrap();
    fs::create_dir_all(&diff_dir).unwrap();

    let mut failures: Vec<String> = Vec::new();

    for stem in geometry_fixture_stems() {
        let doc = load_doc(&stem);
        for &size in &SIZES {
            let opts = RenderOptions {
                width: Some(size),
                height: Some(size),
                background: Some("#ffffff".into()),
                color: Some("#000000".into()),
                region: None,
            };
            let png =
                render_to_png(&doc, &opts).unwrap_or_else(|e| panic!("render {stem}@{size}: {e}"));

            let expected_path = expected_dir.join(format!("{stem}-{size}.png"));

            if blessing() || !expected_path.exists() {
                fs::write(&expected_path, &png).unwrap();
                continue;
            }

            let expected = decode_png(&fs::read(&expected_path).unwrap());
            let actual = decode_png(&png);
            if expected.dimensions() != actual.dimensions() {
                failures.push(format!(
                    "{stem}@{size}: dimension mismatch {:?} vs {:?}",
                    expected.dimensions(),
                    actual.dimensions()
                ));
                continue;
            }

            let (stats, diff) = compare(&expected, &actual).expect("compare");
            if !stats.within_golden_tolerance() {
                let diff_path = diff_dir.join(format!("{stem}-{size}.diff.png"));
                diff.save(&diff_path).ok();
                failures.push(format!(
                    "{stem}@{size}: mean_abs={:.2} (max {MEAN_TOLERANCE}), changed_fraction={:.4} (max {FRACTION_TOLERANCE}) — diff at {}",
                    stats.mean_abs,
                    stats.changed_fraction,
                    diff_path.display()
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "perceptual golden mismatches:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Gate 3: contact-sheet compositor golden (C10 / E5.1).
// ---------------------------------------------------------------------------

/// Composite every golden fixture into one contact sheet and compare it against
/// a blessed expected PNG, with the same perceptual tolerance + bless flow the
/// per-fixture gate uses. This locks the grid compositor (cell sizing, centering,
/// alpha-over, ordering) against drift while tolerating cross-build AA noise in
/// the tiles themselves.
#[test]
fn golden_contact_sheet() {
    let dir = golden_dir();
    let expected_path = dir.join("expected").join("contact-sheet.png");
    fs::create_dir_all(dir.join("expected")).unwrap();
    fs::create_dir_all(dir.join("diff")).unwrap();

    // Tiles in stable fixture order, rendered at a fixed size, transparent so the
    // sheet's own background does the compositing.
    let tiles: Vec<SheetTile> = geometry_fixture_stems()
        .into_iter()
        .map(|stem| {
            let doc = load_doc(&stem);
            let opts = RenderOptions {
                width: Some(64),
                height: Some(64),
                background: None,
                color: Some("#000000".into()),
                region: None,
            };
            let png = render_to_png(&doc, &opts)
                .unwrap_or_else(|e| panic!("render {stem} for sheet: {e}"));
            SheetTile { name: stem, png }
        })
        .collect();

    let opts = SheetOptions {
        columns: 4,
        padding: 8,
        background: Some("#ffffff".into()),
    };
    let sheet = contact_sheet(&tiles, &opts).expect("composite contact sheet");

    if blessing() || !expected_path.exists() {
        fs::write(&expected_path, &sheet).unwrap();
        return;
    }

    let expected = decode_png(&fs::read(&expected_path).unwrap());
    let actual = decode_png(&sheet);
    assert_eq!(
        expected.dimensions(),
        actual.dimensions(),
        "contact-sheet dimension mismatch"
    );
    let (stats, diff) = compare(&expected, &actual).expect("compare");
    if !stats.within_golden_tolerance() {
        let diff_path = dir.join("diff").join("contact-sheet.diff.png");
        diff.save(&diff_path).ok();
        panic!(
            "contact-sheet golden mismatch: mean_abs={:.2} (max {MEAN_TOLERANCE}), changed_fraction={:.4} (max {FRACTION_TOLERANCE}) — diff at {}",
            stats.mean_abs,
            stats.changed_fraction,
            diff_path.display()
        );
    }
}
