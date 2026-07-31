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
