mod common;

use common::{int, name, negative, string, tokens};
use fipe::{
    lex::{lex, Reason},
    token::Kind,
};

#[test]
fn integer_radices_separators_and_limits() {
    for (input, expected) in [
        ("0", int(0)), ("-0", negative(0)),
        ("18446744073709551615", int(u64::MAX)),
        ("-18446744073709551615", negative(u64::MAX)),
        ("0xffff_ffff_ffff_ffff", int(u64::MAX)),
        ("0o1777777777777777777777", int(u64::MAX)),
        ("0b1111111111111111111111111111111111111111111111111111111111111111",
         int(u64::MAX)),
        ("-0xA_f", negative(175)), ("0b1_0", int(2)),
    ] {
        tokens(input, vec![expected]);
    }
    for input in [
        "18446744073709551616",
        "-18446744073709551616",
        "0x10000000000000000",
        "0o2000000000000000000000",
        "0b10000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let result = lex(&format!("{input} after"));
        assert_eq!(result.errors[0].reason, Reason::IntegerRange);
        assert_eq!(result.tokens[0].kind, name("after"));
    }
}

#[test]
fn malformed_numeric_words_do_not_report_overflow() {
    for input in [
        "0x",
        "0x_1",
        "0o8",
        "0b2",
        "1_",
        "1__2",
        "1_e3",
        "1e_3",
        "1e+",
        "1e--2",
        "1e2e3",
        "1.e3",
        "1.2.3",
        "-1st",
        "1f64",
        "18446744073709551616x",
        "0xFFFFFFFFFFFFFFFFFFFFg",
    ] {
        tokens(input, vec![Kind::Word(input.into())]);
    }
}

#[test]
fn floats_preserve_exact_decimal_spelling() {
    for input in [
        "1.5",
        "1e3",
        "-1.0",
        "-0.0",
        "1E+3",
        "1.5e-3",
        "1_000.0_01e1_0",
        "1e999",
        "1e-999",
        "1.000000000000000000000001",
    ] {
        tokens(input, vec![Kind::Float(input.into())]);
    }
}

#[test]
fn resource_ids_are_decimal_unsigned_and_bounded() {
    tokens(
        "rd:0004 rd:18446744073709551615",
        vec![Kind::Rd(4), Kind::Rd(u64::MAX)],
    );
    let result = lex("rd:18446744073709551616 tail");
    assert_eq!(result.errors[0].reason, Reason::ResourceIdRange);
    assert_eq!(result.tokens[0].kind, name("tail"));
}

#[test]
fn strings_decode_only_the_specified_escapes() {
    tokens(
        r#""\n\t\r\\\"\'\0\x41\x414""#,
        vec![string("\n\t\r\\\"'\0AA4", false)],
    );
    tokens(r"'a\nb'", vec![string(r"a\nb", true)]);
    tokens(
        r#""\xC3\xA9" "한글" b"\xFF\x00é""#,
        vec![
            string("é", false),
            string("한글", false),
            Kind::Bytes(vec![255, 0, 195, 169]),
        ],
    );
    tokens(
        "\"a\tb\" 'a\tb'",
        vec![string("a\tb", false), string("a\tb", true)],
    );
    tokens(
        r#""" '' b"""#,
        vec![string("", false), string("", true), Kind::Bytes(vec![])],
    );
}

#[test]
fn invalid_utf8_is_rejected_only_for_strings() {
    for escape in [r"\xFF", r"\xC3", r"\xC0\xAF", r"\xED\xA0\x80"] {
        let result = lex(&format!("\"{escape}\" next"));
        assert_eq!(result.errors[0].reason, Reason::InvalidUtf8);
        assert_eq!(result.tokens[0].kind, name("next"));
        let result = lex(&format!("b\"{escape}\""));
        assert_eq!(result.errors, []);
        assert!(matches!(result.tokens[0].kind, Kind::Bytes(_)));
    }
}

#[test]
fn all_byte_escape_values_are_exact() {
    use std::fmt::Write;
    let mut source = String::from("b\"");
    for byte in 0..=u8::MAX {
        write!(source, "\\x{byte:02X}").unwrap();
    }
    source.push('"');
    tokens(&source, vec![Kind::Bytes((0..=u8::MAX).collect())]);
}
