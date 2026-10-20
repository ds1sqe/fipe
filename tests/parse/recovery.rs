use super::{errors, module};
use fipe::{
    lex::{lex, Lexed},
    parse::{parse, Reason, MAX_DEPTH},
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
        ("x: [U8; -1] = []", "-1"),
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
        ("impl T { a }", "}"),
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
            comments: Vec::new(),
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
        format!("x: {}U8{} = y", "Vec<".repeat(1_000), ">".repeat(1_000)),
        format!("{}x = value", "Just ".repeat(1_000)),
        format!("f: {}A{} = g", "(".repeat(1_000), " -> B)".repeat(1_000)),
        format!("{}x", "(\n".repeat(5_000)),
        format!("{}a", "!".repeat(20_000)),
    ] {
        assert!(errors(&input)
            .iter()
            .any(|error| error.reason == Reason::NestingLimit));
    }
}

#[test]
fn long_flat_syntax_parses_and_drops_without_recursive_ownership() {
    let deepest = MAX_DEPTH - 1;
    for input in [
        format!("a{}", " | a".repeat(deepest)),
        format!("f{}", "()".repeat(deepest)),
        format!("f{}", " a".repeat(20_000)),
        format!("[1{}]", ", 1".repeat(20_000)),
        format!("1{}", " + 1".repeat(deepest)),
    ] {
        let parsed = module(&input);
        let copy = parsed.clone();
        assert_eq!(parsed, copy);
        drop(copy);
        drop(parsed);
    }
    // A chain past the depth bound is refused without recursing.
    for input in [
        format!("a{}", " | a".repeat(20_000)),
        format!("f{}", "()".repeat(20_000)),
        format!("1{}", " + 1".repeat(20_000)),
    ] {
        assert_eq!(errors(&input)[0].reason, Reason::DepthLimit);
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

#[test]
fn a_chain_past_the_depth_bound_is_refused_at_its_operator() {
    // `0 + 1 + …`: the k-th `+` is k + 1 deep, its operator at 4k - 2.
    let deepest = format!("0{}", " + 1".repeat(MAX_DEPTH - 1));
    assert!(parse(lex(&deepest)).is_ok());
    let past = format!("0{}", " + 1".repeat(MAX_DEPTH));
    let operator = 4 * MAX_DEPTH - 2;
    assert_eq!(
        errors(&past),
        vec![fipe::parse::Error {
            span: Span {
                start: operator,
                end: operator + 1,
            },
            reason: Reason::DepthLimit,
        }]
    );
    let pipeline = format!("0{}", " | f".repeat(MAX_DEPTH));
    assert_eq!(errors(&pipeline)[0].reason, Reason::DepthLimit);
}

#[test]
fn a_deep_expression_is_refused_in_its_own_entry_only() {
    let input = format!("a = 0{}\nb = 1\n", " + 1".repeat(MAX_DEPTH));
    let result = errors(&input);
    assert_eq!(result.len(), 1, "{result:?}");
    assert_eq!(result[0].reason, Reason::DepthLimit);
}

#[test]
fn a_partial_parse_keeps_the_readable_entries_and_marks_the_others() {
    let input = "a = 1\n[ = 3\nb = a\n";
    let module = fipe::parse::parse_partial(lex(input));
    assert_eq!(module.entries.len(), 2);
    let source = Source::new(input);
    let malformed: Vec<_> = module
        .malformed
        .iter()
        .map(|span| source.slice(*span))
        .collect();
    assert_eq!(malformed, [Some("[ = 3")]);
    assert_eq!(module.errors.len(), 1);
    assert_eq!(errors(input), module.errors);
}

#[test]
fn an_error_recovered_inside_an_entry_makes_the_entry_malformed() {
    let input = "f = () => {\nadd 2 * 1\n}\ng = 2";
    let module = fipe::parse::parse_partial(lex(input));
    assert_eq!(module.entries.len(), 1, "{:?}", module.malformed);
    assert_eq!(module.malformed.len(), 1);
    let span = module.malformed[0];
    assert_eq!(span.start, 0);
    assert!(Source::new(input).slice(span).unwrap().ends_with('}'));
    assert_eq!(errors(input), module.errors);
}

#[test]
fn an_entry_holding_a_lexical_error_is_malformed() {
    let input = "x = 1\nls\u{200b}next\ny = 2";
    let module = fipe::parse::parse_partial(lex(input));
    assert_eq!(module.entries.len(), 2);
    assert_eq!(module.malformed.len(), 1);
    assert!(matches!(
        module.errors.first().map(|error| error.reason),
        Some(Reason::Lexical(fipe::lex::Reason::Stray('\u{200b}')))
    ));
}

#[test]
fn a_partial_parse_of_an_invalid_token_stream_is_an_empty_module() {
    let module = fipe::parse::parse_partial(Lexed {
        tokens: Vec::new(),
        comments: Vec::new(),
        errors: Vec::new(),
    });
    assert_eq!(module.entries, [] as [fipe::ast::Entry; 0]);
    assert_eq!(
        module.errors.iter().map(|e| e.reason).collect::<Vec<_>>(),
        [Reason::InvalidTokenStream]
    );
}

#[test]
fn an_offset_finds_the_innermost_node_holding_it() {
    use fipe::ast::{ExprKind, Node, PatternKind};
    let input = "f = x => x + 1\n[ = 3\n";
    let module = fipe::parse::parse_partial(lex(input));
    let Some(Node::Expr(id)) = module.at(9) else {
        panic!("{:?}", module.at(9))
    };
    assert!(matches!(module.expr(id).unwrap().kind, ExprKind::Name(_)));
    assert_eq!(module.expr(id).unwrap().span, Span { start: 9, end: 10 });
    let Some(Node::Pattern(id)) = module.at(4) else {
        panic!("{:?}", module.at(4))
    };
    assert!(matches!(
        module.pattern(id).unwrap().kind,
        PatternKind::Name(_)
    ));
    assert_eq!(module.at(16), None, "a malformed entry has no node");
    assert_eq!(module.at(input.len()), None);
}
