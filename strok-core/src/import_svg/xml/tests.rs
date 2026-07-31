use super::*;

#[test]
fn builds_a_tree_from_xml_events() {
    let mut warnings = Vec::new();
    let root = parse_xml(
        "<?xml version='1.0'?>\n<svg width='10'>before &amp; after<g><path /></g></svg>",
        &mut warnings,
    )
    .unwrap();

    assert!(warnings.is_empty());
    assert_eq!(root.name, "svg");
    assert_eq!(root.attr("width"), Some("10"));
    assert_eq!(root.text, "before & after");
    assert_eq!(root.children[0].name, "g");
    assert_eq!(root.children[0].children[0].name, "path");
    assert_eq!(root.line, Some(2));
}

#[test]
fn skips_declarations_and_tracks_lines() {
    let mut warnings = Vec::new();
    let root = parse_xml(
        "<!DOCTYPE svg>\n<!-- comment\nline -->\n<svg>\n<![CDATA[ignored]]>\n<path/>\n</svg>",
        &mut warnings,
    )
    .unwrap();

    assert_eq!(root.line, Some(4));
    assert_eq!(root.children[0].line, Some(6));
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0].line, Some(1));
    assert_eq!(warnings[1].line, Some(5));
}
