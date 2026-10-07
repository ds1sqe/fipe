use super::{affixed, form, int, name, negative, string, text, tokens};
use fipe::{
    lex::{lex, Reason},
    token::{Kind, Magnitude, Written},
};

fn big(digits: &str, negative: bool) -> Kind {
    Kind::Int {
        negative,
        magnitude: Magnitude::parse(digits, 10).unwrap(),
        radix: 10,
        digits: digits.len(),
    }
}

#[test]
fn integer_radices_separators_and_limits() {
    for (input, expected) in [
        ("0", int(0)), ("-0", negative(0)),
        ("18446744073709551615", int(u64::MAX)),
        ("-18446744073709551615", negative(u64::MAX)),
        ("0xffff_ffff_ffff_ffff", form(int(u64::MAX),16,16)),
        ("0o1777777777777777777777", form(int(u64::MAX),8,22)),
        ("0b1111111111111111111111111111111111111111111111111111111111111111",
         form(int(u64::MAX),2,64)),
        ("-0xA_f", form(negative(175),16,2)), ("0b1_0", form(int(2),2,2)),
    ] {
        tokens(input, vec![expected]);
    }
    // The lexer has no bound: one past `u64` is one exact integer.
    let past = "18446744073709551616";
    for (input, radix, digits) in [
        (past,10,20),
        ("0x10000000000000000",16,17),
        ("0o2000000000000000000000",8,22),
        ("0b10000000000000000000000000000000000000000000000000000000000000000",2,65),
    ] {
        tokens(input, vec![form(big(past, false),radix,digits)]);
    }
    tokens(&format!("-{past}"), vec![big(past, true)]);
    let huge = "340282366920938463463374607431768211457";
    tokens(huge, vec![big(huge, false)]);
    tokens(
        &format!("0x1{}1", "0".repeat(31)),
        vec![form(big(huge, false), 16, 33)],
    );
    assert_eq!(Magnitude::parse(past, 10).unwrap().to_string(), past);
    assert_eq!(Magnitude::parse(huge, 10).unwrap().to_u64(), None);
    assert_eq!(Magnitude::parse("1_000", 10).unwrap().to_u64(), Some(1000));
    assert!(Magnitude::from(u64::MAX) < Magnitude::parse(past, 10).unwrap());
}

#[test]
fn malformed_numeric_words_are_words() {
    for input in [
        "0x",
        "0x_1",
        "0o8",
        "0b2",
        "0b102",
        "0xZZ",
        "1_",
        "1__2",
        "1_e3",
        "1e+",
        "1e--2",
        "1.e3",
        "1.2.3",
        "1.",
        "2024-01-01",
        "1-ms",
    ] {
        tokens(input, vec![Kind::Word(input.into())]);
    }
}

/// Rule 21: a suffix is a letter, then name characters, where the
/// longest number ends.
#[test]
fn a_number_suffix_begins_where_the_longest_number_ends() {
    let int = |negative, value: u64| Written::Int {
        negative,
        magnitude: Magnitude::from(value),
        radix: 10,
        digits: value.to_string().len(),
    };
    let float = |spelling: &str| Written::Float(spelling.into());
    for (input, form, suffix) in [
        ("10ms", int(false, 10), "ms"),
        ("2i", int(false, 2), "i"),
        ("1st", int(false, 1), "st"),
        ("-1st", int(true, 1), "st"),
        ("1f64", int(false, 1), "f64"),
        ("4x", int(false, 4), "x"),
        ("1e", int(false, 1), "e"),
        ("1e_3", int(false, 1), "e_3"),
        ("1_000ms", int(false, 1000), "ms"),
        ("0i", int(false, 0), "i"),
        ("0r_7", int(false, 0), "r_7"),
        (
            "0x1Fg",
            Written::Int {
                negative: false,
                magnitude: 31.into(),
                radix: 16,
                digits: 2,
            },
            "g",
        ),
        (
            "0b10z",
            Written::Int {
                negative: false,
                magnitude: 2.into(),
                radix: 2,
                digits: 2,
            },
            "z",
        ),
        ("2.5ms", float("2.5"), "ms"),
        ("1e2e3", float("1e2"), "e3"),
        ("-1.5e3kg", float("-1.5e3"), "kg"),
    ] {
        tokens(input, vec![affixed("", form, suffix)]);
    }
    // The longest number wins: an exponent, hex digits, a plain float.
    tokens("1e5", vec![Kind::Float("1e5".into())]);
    tokens("0x1Fe", vec![form(super::int(0x1fe), 16, 3)]);
    tokens("0e5", vec![Kind::Float("0e5".into())]);
    let past = Written::Int {
        negative: false,
        magnitude: Magnitude::parse("18446744073709551616", 10).unwrap(),
        radix: 10,
        digits: 20,
    };
    tokens("18446744073709551616x", vec![affixed("", past, "x")]);
}

/// Rule 21: after a lone `0`, a letter other than `x`, `o` and `b`, then
/// letters and digits, is a prefix with its digits as written.
#[test]
fn a_number_prefix_keeps_its_digits_as_written() {
    let digits = |negative, text: &str| Written::Digits {
        negative,
        digits: text.into(),
    };
    tokens("0r7Z", vec![affixed("r", digits(false, "7Z"), "")]);
    tokens("-0r7Z", vec![affixed("r", digits(true, "7Z"), "")]);
    tokens("0ms", vec![affixed("m", digits(false, "s"), "")]);
    tokens("0ex", vec![affixed("e", digits(false, "x"), "")]);
    // A name before digits is a name.
    tokens("x10", vec![name("x10")]);
}

/// Rule 21: a name touching a quote is its prefix or suffix; a space
/// makes it a name of its own.
#[test]
fn a_quote_takes_a_touching_prefix_and_suffix() {
    tokens(r#"re"a+b""#, vec![text("re", b"a+b", "")]);
    tokens(r#"re"a+b"i"#, vec![text("re", b"a+b", "i")]);
    tokens(r#""a+b"i"#, vec![text("", b"a+b", "i")]);
    tokens(
        r"re'a\d'",
        vec![affixed(
            "re",
            Written::Text {
                value: br"a\d".to_vec(),
                raw: true,
            },
            "",
        )],
    );
    tokens(r#"f "abc""#, vec![name("f"), string("abc", false)]);
    tokens(r#"f"abc""#, vec![text("f", b"abc", "")]);
    tokens(r#""abc" i"#, vec![string("abc", false), name("i")]);
    // Not a prefix: uppercase, qualified or a keyword; not a suffix: a
    // qualified name.
    tokens(r#"Re"a""#, vec![name("Re"), string("a", false)]);
    tokens(r#"x.re"a""#, vec![name("x.re"), string("a", false)]);
    tokens(
        r#"if"a""#,
        vec![Kind::Kw(fipe::token::Keyword::If), string("a", false)],
    );
    tokens(r#""a"x.y"#, vec![string("a", false), name("x.y")]);
    // With a prefix or suffix the bytes need not be UTF-8.
    tokens(r#""\xFF"i"#, vec![text("", &[255], "i")]);
    // A failed quote keeps its suffix inside its one diagnostic.
    let result = lex(r#""\q"i next"#);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].span.end, 5);
    assert_eq!(result.tokens[0].kind, name("next"));
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
            text("b", &[255, 0, 195, 169], ""),
        ],
    );
    tokens(
        "\"a\tb\" 'a\tb'",
        vec![string("a\tb", false), string("a\tb", true)],
    );
    tokens(
        r#""" '' b"""#,
        vec![string("", false), string("", true), text("b", &[], "")],
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
        assert!(matches!(result.tokens[0].kind, Kind::Affixed(_)));
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
    let all: Vec<u8> = (0..=u8::MAX).collect();
    tokens(&source, vec![text("b", &all, "")]);
}
