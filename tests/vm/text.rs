use super::run::shown;

#[test]
fn text_words_split_join_and_case() {
    assert_eq!(shown("\"a b  c\" | words"), "[\"a\", \"b\", \"c\"]");
    assert_eq!(shown("\"x\\ny\\n\" | lines"), "[\"x\", \"y\"]");
    assert_eq!(shown("\"a,b\" | split --at \",\""), "[\"a\", \"b\"]");
    assert_eq!(shown("[\"a\", \"b\"] | join --sep \"-\""), "\"a-b\"");
    assert_eq!(shown("\"Ab\" | lower"), "\"ab\"");
    assert_eq!(shown("\"Ab\" | upper"), "\"AB\"");
    assert_eq!(shown("\"a\" ++ \"b\""), "\"ab\"");
}

#[test]
fn bytes_and_utf8_round_trip() {
    assert_eq!(shown("\"hé\" | bytes"), "[104, 195, 169]");
    assert_eq!(shown("\"hé\" | bytes | utf8"), "Ok \"hé\"");
    assert_eq!(
        shown("x: Vec<U8> = [104, 255]\nx | utf8"),
        "Err (Utf8Error 1)"
    );
}
