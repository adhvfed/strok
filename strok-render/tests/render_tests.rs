use strok_core::document::Document;
use strok_render::{render_svg_string, render_to_png, RenderOptions, RenderRegion};

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory(bytes).unwrap();
    (image.width(), image.height())
}

#[test]
fn current_color_substituted_at_raster_time() {
    // An icon SVG using currentColor must rasterize (resvg has no inherited
    // color), and the --color option must actually change the output pixels.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M4 12 L20 12" fill="none" stroke="currentColor" stroke-width="4"/></svg>"##;

    let black = render_svg_string(
        svg,
        24,
        24,
        24.0,
        24.0,
        &RenderOptions {
            color: Some("#000000".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let white = render_svg_string(
        svg,
        24,
        24,
        24.0,
        24.0,
        &RenderOptions {
            color: Some("#ffffff".into()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(&black[0..4], &[0x89, 0x50, 0x4e, 0x47]);
    assert_eq!(&white[0..4], &[0x89, 0x50, 0x4e, 0x47]);
    // Different ink → different pixels.
    assert_ne!(
        black, white,
        "expected --color to change the rendered output"
    );
}

#[test]
fn render_produces_valid_png() {
    let mut doc = Document::new(200.0, 200.0);
    doc.append_svg(
        "root",
        r##"<rect id="bg" width="200" height="200" fill="#333"/>"##,
    )
    .unwrap();

    let png = render_to_png(&doc, &RenderOptions::default()).unwrap();

    // PNG magic bytes
    assert!(png.len() > 8);
    assert_eq!(&png[0..4], &[0x89, 0x50, 0x4e, 0x47]);
}

#[test]
fn render_with_custom_size() {
    let mut doc = Document::new(200.0, 200.0);
    doc.append_svg(
        "root",
        r#"<rect id="bg" width="200" height="200" fill="red"/>"#,
    )
    .unwrap();

    let opts = RenderOptions {
        width: Some(100),
        height: Some(100),
        background: None,
        color: None,
        region: None,
    };
    let png = render_to_png(&doc, &opts).unwrap();
    assert_eq!(png_dimensions(&png), (100, 100));
}

#[test]
fn width_only_preserves_document_aspect_ratio() {
    let doc = Document::new(200.0, 100.0);
    let opts = RenderOptions {
        width: Some(120),
        ..Default::default()
    };

    let png = render_to_png(&doc, &opts).unwrap();

    assert_eq!(png_dimensions(&png), (120, 60));
}

#[test]
fn height_only_preserves_document_aspect_ratio() {
    let doc = Document::new(100.0, 200.0);
    let opts = RenderOptions {
        height: Some(90),
        ..Default::default()
    };

    let png = render_to_png(&doc, &opts).unwrap();

    assert_eq!(png_dimensions(&png), (45, 90));
}

#[test]
fn explicit_dimensions_allow_intentional_stretching() {
    let doc = Document::new(200.0, 100.0);
    let opts = RenderOptions {
        width: Some(120),
        height: Some(120),
        ..Default::default()
    };

    let png = render_to_png(&doc, &opts).unwrap();

    assert_eq!(png_dimensions(&png), (120, 120));
}

#[test]
fn render_empty_document() {
    let doc = Document::new(100.0, 100.0);
    let png = render_to_png(&doc, &RenderOptions::default()).unwrap();
    assert!(png.len() > 8);
    assert_eq!(&png[0..4], &[0x89, 0x50, 0x4e, 0x47]);
}

#[test]
fn region_render_crops_in_document_coordinates_and_preserves_aspect_ratio() {
    let mut doc = Document::new(200.0, 100.0);
    doc.append_svg(
        "root",
        r##"<rect id="left" width="100" height="100" fill="#ff0000"/>"##,
    )
    .unwrap();
    doc.append_svg(
        "root",
        r##"<rect id="right" x="100" width="100" height="100" fill="#0000ff"/>"##,
    )
    .unwrap();
    let opts = RenderOptions {
        width: Some(80),
        region: Some(RenderRegion {
            x: 100.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }),
        ..Default::default()
    };

    let png = render_to_png(&doc, &opts).unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();

    assert_eq!(image.dimensions(), (80, 80));
    assert_eq!(image.get_pixel(40, 40).0, [0, 0, 255, 255]);
}

#[test]
fn region_render_rejects_a_crop_outside_the_document() {
    let doc = Document::new(200.0, 100.0);
    let opts = RenderOptions {
        region: Some(RenderRegion {
            x: 150.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }),
        ..Default::default()
    };

    let error = render_to_png(&doc, &opts).unwrap_err().to_string();

    assert!(error.contains("exceeds the 200x100 document"), "{error}");
}
