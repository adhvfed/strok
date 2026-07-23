//! Cross-backend parity — the acceptance test for "co-equal targets".
//!
//! React and Solid must produce the *same* component structure from the same
//! `UiDoc`. They differ only in two syntactic knobs (`className` vs `class`,
//! `dangerouslySetInnerHTML` vs `innerHTML`). After canonicalizing exactly
//! those two knobs, the two sources must be byte-identical. If they ever
//! aren't, a backend has grown a structural special-case and the neutral IR
//! has sprung a leak — which is precisely what this test exists to catch.

use strok_targets::backends::{ReactBackend, SolidBackend, VanillaBackend};
use strok_targets::ir::*;
use strok_targets::target::{EmitOptions, FrameworkBackend};

/// A deliberately rich tree: layout, tokens, nested element, text, and an SVG
/// leaf — enough surface that a divergent backend would show up.
fn sample_doc() -> UiDoc {
    UiDoc {
        name: "Button".into(),
        tokens: TokenSet {
            colors: vec![("accent".into(), "#c8863a".into())],
            ..Default::default()
        },
        root: UiNode::Element {
            tag: Tag::Button,
            layout: Layout::Flex {
                direction: FlexDirection::Row,
                gap: 12.0,
                padding: Edges::symmetric(16.0, 10.0),
                align: Align::Center,
                justify: Justify::Center,
            },
            style: Style {
                fill: Some(StyleValue::Token("accent".into())),
                radius: Some(8.0),
                ..Style::default()
            },
            children: vec![
                UiNode::Text("Click me".into()),
                UiNode::Element {
                    tag: Tag::Span,
                    layout: Layout::Flow,
                    style: Style {
                        opacity: Some(0.8),
                        ..Style::default()
                    },
                    children: vec![UiNode::Svg(
                        "<svg viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"4\"/></svg>"
                            .into(),
                    )],
                },
            ],
        },
        diagnostics: vec![],
    }
}

/// Canonicalize React source onto Solid's two syntactic knobs.
fn react_to_canonical(src: &str) -> String {
    src.replace("className=", "class=")
        .replace("dangerouslySetInnerHTML={{ __html: ", "innerHTML={")
        .replace(" }}", "}")
}

#[test]
fn react_and_solid_are_structurally_identical() {
    let doc = sample_doc();
    let opts = EmitOptions::default();

    let react = ReactBackend.render(&doc, &opts);
    let solid = SolidBackend.render(&doc, &opts);

    let react_src = &react.files[0].contents;
    let solid_src = &solid.files[0].contents;

    // The two outputs must NOT already be identical (otherwise the knobs aren't
    // being exercised and the test proves nothing).
    assert_ne!(
        react_src, solid_src,
        "expected dialect differences to be present"
    );

    assert_eq!(
        react_to_canonical(react_src),
        *solid_src,
        "React and Solid diverge by more than the two known dialect knobs"
    );
}

#[test]
fn both_jsx_backends_emit_the_same_filename() {
    let doc = sample_doc();
    let opts = EmitOptions::default();
    let react = ReactBackend.render(&doc, &opts);
    let solid = SolidBackend.render(&doc, &opts);
    assert_eq!(react.files[0].path, "Button.tsx");
    assert_eq!(solid.files[0].path, "Button.tsx");
}

#[test]
fn vanilla_is_driven_by_the_same_ir() {
    // The non-JSX backend is the third neutrality pressure-test: the same UiDoc
    // must reach it with the same layout/style/vector content.
    let doc = sample_doc();
    let vanilla = VanillaBackend.render(&doc, &EmitOptions::default());
    let src = &vanilla.files[0].contents;

    // Same utility classes the JSX backends produced from the same Style/Layout.
    assert!(
        src.contains("flex flex-row gap-[12px]"),
        "missing layout classes: {src}"
    );
    assert!(
        src.contains("bg-accent"),
        "missing token-derived fill class: {src}"
    );
    assert!(src.contains("rounded-[8px]"), "missing radius class: {src}");
    // Same vector content, inlined.
    assert!(
        src.contains("<circle cx=\"5\" cy=\"5\" r=\"4\"/>"),
        "missing inlined svg: {src}"
    );
}
