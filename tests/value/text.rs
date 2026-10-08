use super::unit;
use core::cmp::Ordering;
use fipe::value::{text, Prelude, Value};

fn t(text: &str) -> Value {
    Value::text(text)
}

fn seq(parts: &[&str]) -> Value {
    Value::seq(parts.iter().map(|part| t(part)).collect())
}

#[test]
fn text_concatenates_counts_scalars_and_orders_by_bytes() {
    assert_eq!(text::concat(&t("a"), &t("b")), t("ab"));
    assert_eq!(text::len(&t("héllo")), Value::Int(5));
    assert_eq!(text::len(&t("")), Value::Int(0));
    assert!(text::eq(&t("a"), &t("a")));
    assert!(!text::eq(&t("a"), &t("b")));
    assert_eq!(text::cmp(&t("a"), &t("b")), Ordering::Less);
    assert_eq!(text::cmp(&t("é"), &t("z")), Ordering::Greater);
    assert_eq!(text::cmp(&t("ab"), &t("a")), Ordering::Greater);
}

#[test]
fn show_quotes_with_the_lexer_escapes() {
    assert_eq!(text::show(&t("a\"b\n")), "\"a\\\"b\\n\"");
    assert_eq!(text::show(&t("\\\t\r\0")), "\"\\\\\\t\\r\\0\"");
    assert_eq!(text::show(&t("\u{1}é")), "\"\\x01é\"");
    assert_eq!(text::show(&t("")), "\"\"");
}

#[test]
fn bytes_and_utf8_convert_both_ways() {
    let unit = unit();
    let prelude = Prelude::new(&unit.tables());
    assert_eq!(
        text::bytes(&t("é")),
        Value::seq(vec![Value::Int(195), Value::Int(169)])
    );
    assert_eq!(
        text::utf8(&text::bytes(&t("é")), &prelude),
        prelude.ok(t("é"))
    );
    assert_eq!(
        text::utf8(
            &Value::seq(vec![Value::Int(97), Value::Int(255)]),
            &prelude
        ),
        prelude.utf8_error(1)
    );
    assert_eq!(text::utf8(&Value::seq(vec![]), &prelude), prelude.ok(t("")));
}

#[test]
fn lines_words_split_join_and_cases() {
    assert_eq!(text::lines(&t("a\r\nb\n")), seq(&["a", "b"]));
    assert_eq!(text::lines(&t("")), seq(&[]));
    assert_eq!(text::words(&t(" a  b\tc\n")), seq(&["a", "b", "c"]));
    assert_eq!(
        text::split(&t(","), &t("a,b,,c")),
        seq(&["a", "b", "", "c"])
    );
    assert_eq!(text::join(&t(", "), &seq(&["a", "b"])), t("a, b"));
    assert_eq!(text::join(&t(", "), &seq(&[])), t(""));
    assert_eq!(text::lower(&t("ÀB")), t("àb"));
    assert_eq!(text::upper(&t("ßa")), t("SSA"));
}
