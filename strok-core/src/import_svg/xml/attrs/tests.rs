use super::*;

#[test]
fn decodes_named_and_numeric_entities() {
    assert_eq!(
        decode_entities("&lt;&gt;&amp;&quot;&apos; &#65; &#x42; &#X43;"),
        "<>&\"' A B C"
    );
}

#[test]
fn preserves_unicode_around_entities() {
    assert_eq!(decode_entities("Strøk &amp; blåbær"), "Strøk & blåbær");
}

#[test]
fn preserves_unknown_and_malformed_entities() {
    assert_eq!(
        decode_entities("a &unknown; b &broken"),
        "a &unknown; b &broken"
    );
    assert_eq!(decode_entities("&unknown; &amp;"), "&unknown; &");
}

#[test]
fn rejects_invalid_numeric_references() {
    assert_eq!(decode_entities("&#x110000; &#wat;"), "&#x110000; &#wat;");
}

#[test]
fn parses_quoted_unquoted_and_boolean_attributes() {
    let attrs: Vec<_> =
        AttrParser::new(r#" width = "10 &amp; 20" height='30' viewBox=0,0,10,10 hidden "#)
            .collect();
    assert_eq!(
        attrs,
        vec![
            ("width".into(), "10 & 20".into()),
            ("height".into(), "30".into()),
            ("viewBox".into(), "0,0,10,10".into()),
            ("hidden".into(), String::new()),
        ]
    );
}

#[test]
fn parses_unicode_attributes_and_unterminated_values() {
    let attrs: Vec<_> = AttrParser::new(" navn='blåbær' title=\"Strøk").collect();
    assert_eq!(
        attrs,
        vec![
            ("navn".into(), "blåbær".into()),
            ("title".into(), "Strøk".into()),
        ]
    );
}
