use fipe::{
    lex::{lex, Reason},
    token::{Kind, Magnitude, Source, Span, Symbol, Token},
};

#[test]
fn all_quote_errors_keep_the_following_line() {
    let cases = [
        ("\"unfinished", Reason::UnterminatedQuote),
        ("'unfinished", Reason::UnterminatedQuote),
        ("b\"unfinished", Reason::UnterminatedQuote),
        (r#""bad\q""#, Reason::BadEscape),
        (r#""bad\x""#, Reason::BadEscape),
        (r#""bad\x1""#, Reason::BadEscape),
        (r#""bad\xg1""#, Reason::BadEscape),
        ("\"bad\\ ", Reason::BadEscape),
        ("\"bad\0text\"", Reason::Stray('\0')),
        ("'bad\rtext'", Reason::Stray('\r')),
        (r#""\xFF""#, Reason::InvalidUtf8),
    ];
    for (broken, reason) in cases {
        if broken == "'unfinished" {
            let result = lex("'unfinished\nnext");
            assert_eq!(result.errors[0].reason, reason);
            assert_eq!(result.errors[0].span.end, 16);
            assert_eq!(result.tokens[0].kind, Kind::Eof);
            continue;
        }
        let input = format!("{broken}\nnext");
        let result = lex(&input);
        assert_eq!(result.errors.len(), 1, "{input:?}");
        assert_eq!(result.errors[0].reason, reason, "{input:?}");
        assert_eq!(
            result.errors[0].span,
            Span {
                start: 0,
                end: broken.len()
            }
        );
        assert_eq!(result.tokens[0].kind, Kind::Newline);
        assert_eq!(
            result.tokens[0].span,
            Span {
                start: broken.len(),
                end: broken.len() + 1
            }
        );
        assert_eq!(result.tokens[1].kind, Kind::Name(vec!["next".into()]));
        assert_eq!(result.tokens[2].kind, Kind::Eof);
    }
}

#[test]
fn recovery_respects_escaped_quote_and_reports_first_error() {
    let input = r#""bad\q\" still bad\x" next"#;
    let result = lex(input);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].reason, Reason::BadEscape);
    assert_eq!(
        Source::new(input).slice(result.errors[0].span),
        Some(r#""bad\q\" still bad\x""#)
    );
    assert_eq!(result.tokens[0].kind, Kind::Name(vec!["next".into()]));
    let result = lex("\"\\q\nnext");
    assert_eq!(result.errors[0].reason, Reason::BadEscape);
}

#[test]
fn control_errors_never_truncate_the_source() {
    let result = lex("a\0b & c\u{85}d");
    assert_eq!(
        result.errors.iter().map(|e| e.reason).collect::<Vec<_>>(),
        vec![Reason::Stray('\0'), Reason::Stray('\u{85}')]
    );
    assert_eq!(
        result
            .tokens
            .iter()
            .map(|t| t.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            Kind::Name(vec!["a".into()]),
            Kind::Name(vec!["b".into()]),
            Kind::Symbol(Symbol::Ampersand),
            Kind::Name(vec!["c".into()]),
            Kind::Name(vec!["d".into()]),
            Kind::Eof
        ]
    );
    for error in &result.errors {
        assert_eq!(
            Source::new("a\0b & c\u{85}d").slice(error.span),
            Some(match error.reason {
                Reason::Stray('\0') => "\0",
                _ => "\u{85}",
            })
        );
    }
}

#[test]
fn pipes_suppress_one_physical_newline_only() {
    for pipe in [
        Symbol::Pipe,
        Symbol::PipeGood,
        Symbol::PipeBad,
        Symbol::Or,
        Symbol::And,
    ] {
        for gap in ["\n", " \t\r\n", " # comment\n"] {
            let input = format!("a {pipe}{gap} b");
            let result = lex(&input);
            assert_eq!(result.errors, []);
            assert_eq!(
                result.tokens.iter().map(|t| &t.kind).collect::<Vec<_>>(),
                [
                    &Kind::Name(vec!["a".into()]),
                    &Kind::Symbol(pipe),
                    &Kind::Name(vec!["b".into()]),
                    &Kind::Eof
                ]
            );
        }
        let result = lex(&format!("a {pipe}\n\n# comment\nb"));
        assert_eq!(result.tokens[2].kind, Kind::Newline);
        assert_eq!(result.tokens[3].kind, Kind::Newline);
        assert_eq!(result.tokens[4].kind, Kind::Name(vec!["b".into()]));
    }
    let result = lex("a | b\n");
    assert_eq!(result.tokens[3].kind, Kind::Newline);
    let result = lex("a |\0\nb");
    assert_eq!(result.tokens[2].kind, Kind::Newline);
}

#[test]
fn comments_quotes_and_crlf_have_distinct_meaning() {
    let input = "cat \"#x\"# tail\r\n\n# only comment\n'>'";
    let result = lex(input);
    assert_eq!(result.errors, []);
    assert_eq!(
        result
            .tokens
            .iter()
            .map(|t| t.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            Kind::Name(vec!["cat".into()]),
            Kind::Str {
                value: "#x".into(),
                raw: false
            },
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Str {
                value: ">".into(),
                raw: true
            },
            Kind::Eof
        ]
    );
    let result = lex("# invalid\0 comment\nnext");
    assert_eq!(result.errors[0].reason, Reason::Stray('\0'));
    assert_eq!(result.tokens[0].kind, Kind::Newline);
    assert_eq!(result.tokens[1].kind, Kind::Name(vec!["next".into()]));
}

#[test]
fn byte_spans_and_scalar_columns_agree() {
    let input = "cat héllo \"한글\"\n\tx";
    let result = lex(input);
    let expected = [
        ("cat", (1, 1)),
        ("héllo", (1, 5)),
        ("\"한글\"", (1, 11)),
        ("\n", (1, 15)),
        ("x", (2, 2)),
        ("", (2, 3)),
    ];
    assert_eq!(
        result
            .tokens
            .iter()
            .map(|t| (
                Source::new(input).slice(t.span).unwrap(),
                Source::new(input).line_col(t.span.start)
            ))
            .collect::<Vec<_>>(),
        expected
    );
    let source = Source::new("é\n한");
    assert_eq!(source.line_col(1), (1, 1));
    assert_eq!(source.line_col(2), (1, 2));
    assert_eq!(source.line_col(4), (2, 1));
    assert_eq!(source.line_col(usize::MAX), (2, 2));
    assert_eq!(source.slice(Span { start: 1, end: 2 }), None);
    assert_eq!(source.slice(Span { start: 3, end: 1 }), None);
    assert_eq!(
        source.slice(Span {
            start: 0,
            end: usize::MAX
        }),
        None
    );
}

#[test]
fn empty_and_comment_only_sources_still_end_once() {
    for input in ["", "  \t\r", "#comment", "#comment\0"] {
        assert_eq!(
            lex(input).tokens,
            vec![Token {
                kind: Kind::Eof,
                span: Span {
                    start: input.len(),
                    end: input.len()
                }
            }]
        );
    }
}

fn assert_spans(input: &str) {
    let result = lex(input);
    assert_eq!(result.tokens.last().unwrap().kind, Kind::Eof);
    assert_eq!(result.tokens.last().unwrap().span.end, input.len());
    let mut spans: Vec<_> = result
        .tokens
        .iter()
        .map(|t| t.span)
        .chain(result.errors.iter().map(|e| e.span))
        .collect();
    spans.sort_by_key(|span| (span.start, span.end));
    let mut end = 0;
    for span in spans {
        assert!(end <= span.start, "{input:?}: {span:?}");
        assert!(
            Source::new(input).slice(span).is_some(),
            "{input:?}: {span:?}"
        );
        end = span.end;
    }
}

#[test]
fn malformed_combinations_always_progress_with_valid_spans() {
    let pieces = [
        "", "a", "rd:", "b", "\"", "'", "\\", "\\x", "\\\n", "é", "한", "😀",
        "\n", "\r", "\t", "\0", "\u{85}", "\u{a0}", "\u{2028}", "\u{feff}",
        "#", "|", ":", "!", "&", "=", ">", "-", "0", "_", "\u{200b}",
    ];
    for a in pieces {
        for b in pieces {
            for c in pieces {
                assert_spans(&format!("{a}{b}{c}"));
            }
        }
    }
}

#[test]
fn whitespace_and_controls_have_fixed_source_meaning() {
    let spacing = [
        '\t', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}',
        '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
        '\u{205f}', '\u{3000}',
    ];
    for ch in spacing {
        let result = lex(&format!("a{ch}b"));
        assert_eq!(result.errors, []);
        assert_eq!(result.tokens[0].kind, Kind::Name(vec!["a".into()]));
        assert_eq!(result.tokens[1].kind, Kind::Name(vec!["b".into()]));
        assert_eq!(result.tokens[2].kind, Kind::Eof);
    }
    for ch in (0_u8..=31).chain(127..=159).map(char::from) {
        if !matches!(ch, '\t' | '\r' | '\n') {
            let result = lex(&format!("a{ch}b"));
            assert_eq!(result.errors[0].reason, Reason::Stray(ch));
            assert_eq!(result.tokens[1].kind, Kind::Name(vec!["b".into()]));
        }
    }
}

#[test]
fn large_literals_and_deep_delimiters_do_not_need_recursion() {
    let digits = "9".repeat(100_000);
    let input = format!("{digits} -{digits} 1e{digits} \"{digits}\" next");
    let result = lex(&input);
    assert_eq!(result.errors, []);
    let exact = Magnitude::parse(&digits, 10).unwrap();
    for (token, negative) in result.tokens[..2].iter().zip([false, true]) {
        let Kind::Int {
            negative: sign,
            magnitude,
        } = &token.kind
        else {
            panic!("expected an integer: {:?}", token.kind);
        };
        assert_eq!((*sign, magnitude), (negative, &exact));
    }
    assert_eq!(result.tokens[2].kind, Kind::Float(format!("1e{digits}")));
    assert_eq!(
        result.tokens[3].kind,
        Kind::Str {
            value: digits,
            raw: false
        }
    );
    assert_eq!(result.tokens[4].kind, Kind::Name(vec!["next".into()]));
    let nested = format!("{}x{}", "(".repeat(10_000), ")".repeat(10_000));
    let result = lex(&nested);
    assert_eq!(result.errors, []);
    assert_eq!(result.tokens[10_000].kind, Kind::Name(vec!["x".into()]));
    assert!(result.tokens[..10_000]
        .iter()
        .all(|token| token.kind == Kind::Symbol(Symbol::LParen)));
    assert!(result.tokens[10_001..20_001]
        .iter()
        .all(|token| token.kind == Kind::Symbol(Symbol::RParen)));
}
