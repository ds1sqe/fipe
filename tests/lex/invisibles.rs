use fipe::{
    lex::{lex, LexError, Reason},
    token::{Kind, Source, Span, Token},
};

#[test]
fn initial_bom_preserves_source_offsets_and_columns() {
    let input = "\u{feff}ls\nnext";
    let result = lex(input);
    assert_eq!(result.errors, []);
    assert_eq!(
        result
            .tokens
            .iter()
            .map(|token| (token.kind.clone(), token.span))
            .collect::<Vec<_>>(),
        [
            (Kind::Name(vec!["ls".into()]), Span { start: 3, end: 5 }),
            (Kind::Newline, Span { start: 5, end: 6 }),
            (Kind::Name(vec!["next".into()]), Span { start: 6, end: 10 }),
            (Kind::Eof, Span { start: 10, end: 10 }),
        ]
    );
    let source = Source::new(input);
    assert_eq!(source.slice(result.tokens[0].span), Some("ls"));
    assert_eq!(source.line_col(3), (1, 2));
    assert_eq!(source.line_col(6), (2, 1));

    let result = lex("\u{feff}");
    assert_eq!(result.errors, []);
    assert_eq!(
        result.tokens,
        [Token {
            kind: Kind::Eof,
            span: Span { start: 3, end: 3 },
        }]
    );
}

#[test]
fn only_one_bom_at_byte_zero_is_skipped() {
    for (input, ch, start) in [
        ("\u{feff}\u{feff}ls", '\u{feff}', 3),
        (" \u{feff}ls", '\u{feff}', 1),
        ("\n\u{feff}ls", '\u{feff}', 1),
        ("\u{200b}ls", '\u{200b}', 0),
        ("\u{feff}\u{200b}ls", '\u{200b}', 3),
    ] {
        let result = lex(input);
        assert_eq!(
            result.errors,
            [LexError {
                reason: Reason::Stray(ch),
                span: Span {
                    start,
                    end: start + 3,
                },
            }],
            "{input:?}"
        );
        let ls = &result.tokens[result.tokens.len() - 2];
        assert_eq!(ls.kind, Kind::Name(vec!["ls".into()]));
        assert_eq!(Source::new(input).slice(ls.span), Some("ls"));
        assert_eq!(result.tokens.last().unwrap().kind, Kind::Eof);
    }
}

#[test]
fn invisible_marks_cannot_join_words() {
    for ch in ['\u{feff}', '\u{200b}'] {
        let input = format!("l{ch}s");
        let result = lex(&input);
        assert_eq!(
            result.errors,
            [LexError {
                reason: Reason::Stray(ch),
                span: Span { start: 1, end: 4 },
            }]
        );
        assert_eq!(
            result.tokens,
            [
                Token {
                    kind: Kind::Name(vec!["l".into()]),
                    span: Span { start: 0, end: 1 },
                },
                Token {
                    kind: Kind::Name(vec!["s".into()]),
                    span: Span { start: 4, end: 5 },
                },
                Token {
                    kind: Kind::Eof,
                    span: Span { start: 5, end: 5 },
                },
            ]
        );
        assert_eq!(
            Source::new(&input).slice(result.errors[0].span),
            Some(ch.to_string().as_str())
        );
    }
}

#[test]
fn invisible_marks_in_comments_still_diagnose() {
    for ch in ['\u{feff}', '\u{200b}'] {
        let input = format!("#한{ch}tail\nnext");
        let result = lex(&input);
        assert_eq!(
            result.errors,
            [LexError {
                reason: Reason::Stray(ch),
                span: Span { start: 4, end: 7 },
            }]
        );
        assert_eq!(Source::new(&input).line_col(4), (1, 3));
        assert_eq!(result.tokens[0].kind, Kind::Newline);
        assert_eq!(result.tokens[1].kind, Kind::Name(vec!["next".into()]));
        assert_eq!(result.tokens[2].kind, Kind::Eof);
    }
}

#[test]
fn literal_invisible_marks_fail_quotes_and_recover() {
    for ch in ['\u{feff}', '\u{200b}'] {
        for (open, close) in [("\"", "\""), ("'", "'"), ("b\"", "\"")] {
            let broken = format!("{open}a{ch}b{close}");
            let input = format!("{broken}\nnext");
            let result = lex(&input);
            assert_eq!(
                result.errors,
                [LexError {
                    reason: Reason::Stray(ch),
                    span: Span {
                        start: 0,
                        end: broken.len(),
                    },
                }]
            );
            assert_eq!(result.tokens[0].kind, Kind::Newline);
            assert_eq!(result.tokens[1].kind, Kind::Name(vec!["next".into()]));
            assert_eq!(result.tokens[2].kind, Kind::Eof);
        }
    }
}

#[test]
fn escaped_invisible_marks_remain_literal_data() {
    for (spelling, value) in
        [(r"\xEF\xBB\xBF", "\u{feff}"), (r"\xE2\x80\x8B", "\u{200b}")]
    {
        for (prefix, expected) in [
            (
                "",
                Kind::Str {
                    value: value.into(),
                    raw: false,
                },
            ),
            ("b", super::text("b", value.as_bytes(), "")),
        ] {
            let result = lex(&format!("{prefix}\"{spelling}\""));
            assert_eq!(result.errors, []);
            assert_eq!(result.tokens[0].kind, expected);
            assert_eq!(result.tokens[1].kind, Kind::Eof);
            let displayed = lex(&result.tokens[0].kind.to_string());
            assert_eq!(displayed.errors, []);
            assert_eq!(displayed.tokens[0].kind, expected);
        }
    }
}
