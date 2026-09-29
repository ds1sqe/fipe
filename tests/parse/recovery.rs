use super::{errors, module};
use fipe::{
    lex::{lex, Lexed},
    parse::{parse, Reason},
    token::{Kind, Source, Span, Symbol, Token},
};

#[test]
fn lexical_errors_are_returned_without_partial_parsing() {
    let input = "x =\nls\u{200b}next";
    let result = errors(input);
    assert_eq!(
        result,
        vec![fipe::parse::Error {
            span: Span { start: 6, end: 9 },
            reason: Reason::Lexical(fipe::lex::Reason::Stray('\u{200b}')),
        }]
    );
}

#[test]
fn independent_line_errors_keep_their_source_locations() {
    let input = "# 한글\nadd 2 - 1\n[ = 3\nf = () => {\nadd 2 * 1\n}\nnext";
    let result = errors(input);
    assert_eq!(
        result
            .iter()
            .map(|error| Source::new(input).slice(error.span).unwrap())
            .collect::<Vec<_>>(),
        ["-", "=", "*"]
    );
    assert_eq!(
        result
            .iter()
            .map(|error| Source::new(input).line_col(error.span.start))
            .collect::<Vec<_>>(),
        [(2, 7), (3, 3), (5, 7)]
    );
}

#[test]
fn invalid_grammar_is_rejected_at_the_offending_token() {
    for (input, offending) in [
        ("1 < 2 < 3", "<"),
        ("1 == 2 == 3", "=="),
        ("cat x > y | next", ">"),
        ("[x => x]", "=>"),
        ("{ x => x\nls }", "\n"),
        ("{ x => x, ls }", "ls"),
        ("a<b", ""),
        ("P { .., x } = value", "x"),
        ("type qualified.name {}", "qualified.name"),
        ("use fs -1", "-1"),
        ("x: [u8; -1] = []", "-1"),
        ("{ use fs }", "use"),
        ("x; y", ";"),
        ("f(1", ""),
        ("= 1", "="),
        ("[1,,2]", ","),
        ("pub use fs", "use"),
        ("pub ls", "ls"),
        ("f x = 3", "x"),
        ("Just e rest = v", "rest"),
        ("type T { pub A }", "}"),
        ("impl T { a: b }", "}"),
    ] {
        let result = errors(input);
        assert_eq!(
            Source::new(input).slice(result[0].span),
            Some(offending),
            "{input}"
        );
    }
    module("[(x => x)]\nf((x => x))\nP { field: (x => x) }");
    // The first depth-zero `=>` wins over a later `=`: an arm, not a binding.
    let (parsed, id) = super::only("x => y = 1");
    assert!(matches!(
        super::node(&parsed, id),
        fipe::ast::ExprKind::Arms(_)
    ));
}

#[test]
fn malformed_token_streams_are_diagnostics() {
    let eof = Token {
        kind: Kind::Eof,
        span: Span { start: 2, end: 2 },
    };
    for tokens in [
        vec![],
        vec![Token {
            kind: Kind::Name(vec!["x".into()]),
            span: Span { start: 0, end: 1 },
        }],
        vec![eof.clone(), eof.clone()],
        vec![Token {
            kind: Kind::Eof,
            span: Span { start: 2, end: 1 },
        }],
        vec![
            Token {
                kind: Kind::Symbol(Symbol::Or),
                span: Span { start: 0, end: 1 },
            },
            eof.clone(),
        ],
        vec![
            Token {
                kind: Kind::Name(vec!["x".into()]),
                span: Span { start: 0, end: 3 },
            },
            eof,
        ],
    ] {
        let result = parse(Lexed {
            tokens,
            errors: vec![],
        })
        .unwrap_err();
        assert_eq!(result[0].reason, Reason::InvalidTokenStream);
    }
}

#[test]
fn deep_sources_return_a_nesting_diagnostic() {
    for input in [
        format!("{}x{}", "(".repeat(1_000), ")".repeat(1_000)),
        format!("x: {}u8{} = y", "Vec<".repeat(1_000), ">".repeat(1_000)),
        format!("{}x = value", "Just ".repeat(1_000)),
        format!("f: {}A{} = g", "(".repeat(1_000), " -> B)".repeat(1_000)),
        format!("{}x", "(\n".repeat(5_000)),
    ] {
        assert!(errors(&input)
            .iter()
            .any(|error| error.reason == Reason::NestingLimit));
    }
}

#[test]
fn long_flat_syntax_parses_and_drops_without_recursive_ownership() {
    for input in [
        format!("a{}", " | a".repeat(20_000)),
        format!("f{}", "()".repeat(20_000)),
        format!("f{}", " a".repeat(20_000)),
        format!("{}a", "!".repeat(20_000)),
        format!("1{}", " + 1".repeat(20_000)),
    ] {
        let parsed = module(&input);
        let copy = parsed.clone();
        assert_eq!(parsed, copy);
        drop(copy);
        drop(parsed);
    }
}

#[test]
fn malformed_short_combinations_terminate_with_valid_diagnostic_spans() {
    let pieces = [
        "", "a", "pub ", "type ", "impl ", "if ", "with ", "(", ")", "[", "]",
        "{", "}", "=>", " = ", " | ", " > ", " - ", " -c=", "\n", ",", "1",
        "[:", "[!", "\\\n", "<", "->", "A",
    ];
    for a in pieces {
        for b in pieces {
            for c in pieces {
                let input = format!("{a}{b}{c}");
                if let Err(errors) = parse(lex(&input)) {
                    assert_ne!(errors, []);
                    for error in errors {
                        assert!(
                            Source::new(&input).slice(error.span).is_some(),
                            "{input:?}: {error:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn blocks_in_heads_require_grouping() {
    for input in ["if { True } { x }", "with key { value } { body }"] {
        let result = errors(input);
        assert_eq!(Source::new(input).slice(result[0].span), Some("{"));
    }
    module("if ({ True }) { x }\nwith key ({ value }) { body }");
}

#[test]
fn a_recovered_error_reports_no_cascade() {
    let deep = format!("{}1{}", "(".repeat(70), ")".repeat(70));
    let result = errors(&deep);
    assert_eq!(result.len(), 1, "{result:?}");
    assert_eq!(result[0].reason, Reason::NestingLimit);
    let block = format!("f = {{\n    {deep}\n    y\n}}");
    let result = errors(&block);
    assert_eq!(result.len(), 1, "{result:?}");
    assert_eq!(result[0].reason, Reason::NestingLimit);
    let input = "q = (1 ]";
    let result = errors(input);
    assert_eq!(result.len(), 1, "{result:?}");
    assert_eq!(
        result[0].reason,
        Reason::Expected(fipe::parse::Expected::Symbol(Symbol::RParen))
    );
    assert_eq!(Source::new(input).slice(result[0].span), Some("]"));
    // A later entry's own error is still reported.
    assert_eq!(errors("q = (1 ]\ny = (2").len(), 2);
}
