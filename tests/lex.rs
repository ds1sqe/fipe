use fipe::{
    lex::lex,
    token::{
        Affixed, Keyword as K, Kind, Kind::Symbol as S, Magnitude, Source,
        Span, Symbol::*, Written,
    },
};

#[path = "lex/invisibles.rs"]
mod invisibles;
#[path = "lex/literals.rs"]
mod literals;
#[path = "lex/positions.rs"]
mod positions;
#[path = "lex/recovery.rs"]
mod recovery;

fn name(text: &str) -> Kind {
    Kind::Name(text.split('.').map(str::to_owned).collect())
}

fn int(magnitude: u64) -> Kind {
    Kind::Int {
        negative: false,
        magnitude: Magnitude::from(magnitude),
    }
}

fn negative(magnitude: u64) -> Kind {
    Kind::Int {
        negative: true,
        magnitude: Magnitude::from(magnitude),
    }
}

/// A written form with a prefix and a suffix, each empty when absent.
fn affixed(prefix: &str, form: Written, suffix: &str) -> Kind {
    Kind::Affixed(Box::new(Affixed {
        prefix: prefix.into(),
        form,
        suffix: suffix.into(),
    }))
}

/// Decoded text with a prefix and a suffix.
fn text(prefix: &str, value: &[u8], suffix: &str) -> Kind {
    let form = Written::Text {
        value: value.to_vec(),
        raw: false,
    };
    affixed(prefix, form, suffix)
}

fn string(value: &str, raw: bool) -> Kind {
    Kind::Str {
        value: value.to_owned(),
        raw,
    }
}

fn tokens(input: &str, expected: Vec<Kind>) {
    let result = lex(input);
    assert!(result.errors.is_empty(), "{input:?}: {:?}", result.errors);
    let eof = result.tokens.last().unwrap();
    assert_eq!(eof.kind, Kind::Eof);
    assert_eq!(
        eof.span,
        Span {
            start: input.len(),
            end: input.len()
        }
    );
    let mut expected = expected;
    expected.push(Kind::Eof);
    assert_eq!(
        result
            .tokens
            .iter()
            .map(|token| &token.kind)
            .collect::<Vec<_>>(),
        expected.iter().collect::<Vec<_>>(),
        "{input:?}"
    );
    let source = Source::new(input);
    let mut end = 0;
    for token in result.tokens {
        assert!(end <= token.span.start);
        assert!(source.slice(token.span).is_some());
        end = token.span.end;
    }
}

#[test]
fn literal_forms_and_payloads() {
    let cases = [
        ("()", vec![S(LParen), S(RParen)]),
        ("True False", vec![name("True"), name("False")]),
        (
            "199 -5 0x199 0o17 0b1010 1_000",
            vec![int(199), negative(5), int(409), int(15), int(10), int(1000)],
        ),
        (
            "1.5 1e3",
            vec![Kind::Float("1.5".into()), Kind::Float("1e3".into())],
        ),
        ("my-file", vec![Kind::Word("my-file".into())]),
        (
            r#""a b" 'a b'"#,
            vec![string("a b", false), string("a b", true)],
        ),
        (r#"b"text""#, vec![text("b", b"text", "")]),
        ("rd:4", vec![name("rd"), S(Colon), int(4)]),
        (
            "/module/fs/root ./x ../x",
            vec![
                Kind::Path("/module/fs/root".into()),
                Kind::Path("./x".into()),
                Kind::Path("../x".into()),
            ],
        ),
        (
            "fs.Entry(file_name, 200)",
            vec![
                name("fs.Entry"),
                S(LParen),
                name("file_name"),
                S(Comma),
                int(200),
                S(RParen),
            ],
        ),
        (
            "fs.Entry { name: file_name }",
            vec![
                name("fs.Entry"),
                S(LBrace),
                name("name"),
                S(Colon),
                name("file_name"),
                S(RBrace),
            ],
        ),
        ("fs.Error.NotFound", vec![name("fs.Error.NotFound")]),
        (
            "Just x Nothing Left x Right x Ok x Err e",
            vec![
                name("Just"),
                name("x"),
                name("Nothing"),
                name("Left"),
                name("x"),
                name("Right"),
                name("x"),
                name("Ok"),
                name("x"),
                name("Err"),
                name("e"),
            ],
        ),
        (".name", vec![Kind::Selector(vec!["name".into()])]),
        ("Width 80", vec![name("Width"), int(80)]),
        ("(line)", vec![S(LParen), name("line"), S(RParen)]),
        (
            "[a,b,c]",
            vec![
                S(LBracket),
                name("a"),
                S(Comma),
                name("b"),
                S(Comma),
                name("c"),
                S(RBracket),
            ],
        ),
    ];
    for (input, expected) in cases {
        tokens(input, expected);
    }
}

#[test]
fn every_flag_form_has_one_value_grammar() {
    for spelling in ["-c", "--c"] {
        tokens(
            &format!("{spelling} 80"),
            vec![Kind::Flag("c".into()), int(80)],
        );
        tokens(
            &format!("{spelling}=80"),
            vec![Kind::Flag("c".into()), S(Assign), int(80)],
        );
    }
    let cases = [
        (
            "-width 80 --width 80",
            vec![
                Kind::Flag("width".into()),
                int(80),
                Kind::Flag("width".into()),
                int(80),
            ],
        ),
        ("-l", vec![Kind::Flag("l".into())]),
        (
            "-l=False",
            vec![Kind::Flag("l".into()), S(Assign), name("False")],
        ),
        (
            "-la --la -tx",
            vec![
                Kind::Flag("la".into()),
                Kind::Flag("la".into()),
                Kind::Flag("tx".into()),
            ],
        ),
        (
            "-n=-5",
            vec![Kind::Flag("n".into()), S(Assign), negative(5)],
        ),
        (
            "-5 - --",
            vec![negative(5), S(Minus), Kind::Word("--".into())],
        ),
        (r#""-x""#, vec![string("-x", false)]),
        (
            "-t [a,b]",
            vec![
                Kind::Flag("t".into()),
                S(LBracket),
                name("a"),
                S(Comma),
                name("b"),
                S(RBracket),
            ],
        ),
        (
            r#"-c="a b""#,
            vec![Kind::Flag("c".into()), S(Assign), string("a b", false)],
        ),
        (
            "-t=[1,2]",
            vec![
                Kind::Flag("t".into()),
                S(Assign),
                S(LBracket),
                int(1),
                S(Comma),
                int(2),
                S(RBracket),
            ],
        ),
    ];
    for (input, expected) in cases {
        tokens(input, expected);
    }
}

#[test]
fn all_keywords_and_symbols() {
    for keyword in [K::If, K::Else, K::With, K::Type, K::Impl, K::Use, K::Pub] {
        tokens(keyword.spelling(), vec![Kind::Kw(keyword)]);
        assert_eq!(keyword.to_string(), keyword.spelling());
    }
    for symbol in [
        Plus, Minus, Star, Slash, Percent, Concat, LParen, RParen, LBrace,
        RBrace, LBracket, RBracket, Comma, Semicolon, Colon, Assign, Eq, Not,
        NotEq, Less, LessEq, Greater, GreaterEq, Arrow, FatArrow, Pipe,
        PipeGood, PipeBad, Or, And, Ampersand,
    ] {
        tokens(symbol.spelling(), vec![S(symbol)]);
        assert_eq!(symbol.to_string(), symbol.spelling());
    }
    tokens(
        "| |: |! || && = => == ! != < <= > >= >> - -> as",
        vec![
            S(Pipe),
            S(PipeGood),
            S(PipeBad),
            S(Or),
            S(And),
            S(Assign),
            S(FatArrow),
            S(Eq),
            S(Not),
            S(NotEq),
            S(Less),
            S(LessEq),
            S(Greater),
            S(GreaterEq),
            S(Greater),
            S(Greater),
            S(Minus),
            S(Arrow),
            name("as"),
        ],
    );
    tokens("!b", vec![S(Not), name("b")]);
    tokens(
        "A & B && C &B a ++ b a++b",
        vec![
            name("A"),
            S(Ampersand),
            name("B"),
            S(And),
            name("C"),
            S(Ampersand),
            name("B"),
            name("a"),
            S(Concat),
            name("b"),
            Kind::Word("a++b".into()),
        ],
    );
}

#[test]
fn word_boundaries_are_not_arithmetic_or_name_resolution() {
    for text in [
        "2024-01-01",
        "0xZZ",
        "a+b",
        "x*2",
        "1__0",
        "_1_",
        "fs..Entry",
        ".5",
        "1.",
        "...",
        "héllo",
        "한글",
        "---x",
        "+1",
    ] {
        let expected = if text == "_1_" {
            name(text)
        } else {
            Kind::Word(text.into())
        };
        tokens(text, vec![expected]);
    }
    tokens(
        "_ _foo fs.Entry x * 2 /",
        vec![
            name("_"),
            name("_foo"),
            name("fs.Entry"),
            name("x"),
            S(Star),
            int(2),
            S(Slash),
        ],
    );
    tokens(
        "rd:4-x",
        vec![name("rd"), S(Colon), Kind::Word("4-x".into())],
    );
    tokens("rd:-4", vec![name("rd"), S(Colon), negative(4)]);
    tokens("rd:0004", vec![name("rd"), S(Colon), int(4)]);
    tokens("rd:", vec![name("rd"), S(Colon)]);
}

#[test]
fn backslash_joins_lines_outside_quotes_and_comments() {
    for input in [
        "cat x \\\ny",
        "cat x \\\r\ny",
        "cat x \\ \t \ny",
        "cat x\\\ny",
    ] {
        tokens(input, vec![name("cat"), name("x"), name("y")]);
    }
    tokens("cat x \\", vec![name("cat"), name("x")]);
    tokens(
        "cat x # note \\\ny",
        vec![name("cat"), name("x"), Kind::Newline, name("y")],
    );
    tokens(
        "a\\b \\x '\\' \\\n",
        vec![
            Kind::Word("a\\b".into()),
            Kind::Word("\\x".into()),
            string("\\", true),
        ],
    );
    tokens(
        "a | \\\n\nb",
        vec![name("a"), S(Pipe), Kind::Newline, name("b")],
    );
    let source = "x \\\ny";
    let result = lex(source);
    assert_eq!(result.tokens[1].span, Span { start: 4, end: 5 });
    assert_eq!(Source::new(source).line_col(4), (2, 1));
}

#[test]
fn type_spellings_remain_context_free_tokens() {
    tokens("Rd<fs>", vec![name("Rd"), S(Less), name("fs"), S(Greater)]);
    tokens(
        "Result<Vec<String>, fs.Error>",
        vec![
            name("Result"),
            S(Less),
            name("Vec"),
            S(Less),
            name("String"),
            S(Greater),
            S(Comma),
            name("fs.Error"),
            S(Greater),
        ],
    );
    tokens(
        "Vec<Vec<U8>>",
        vec![
            name("Vec"),
            S(Less),
            name("Vec"),
            S(Less),
            name("U8"),
            S(Greater),
            S(Greater),
        ],
    );
    tokens(
        "[U8; 4096]",
        vec![
            S(LBracket),
            name("U8"),
            S(Semicolon),
            int(4096),
            S(RBracket),
        ],
    );
    tokens(
        "(A -> B) -> C",
        vec![
            S(LParen),
            name("A"),
            S(Arrow),
            name("B"),
            S(RParen),
            S(Arrow),
            name("C"),
        ],
    );
    tokens(
        "let fn struct enum for ver as trait where",
        vec![
            name("let"),
            name("fn"),
            name("struct"),
            name("enum"),
            name("for"),
            name("ver"),
            name("as"),
            name("trait"),
            name("where"),
        ],
    );
}

#[test]
fn opening_example_matches_each_token_and_spelling() {
    let source = "ls -l dir | filter (e => e.size > 4096) | map .name";
    let expected = [
        ("ls", name("ls")),
        ("-l", Kind::Flag("l".into())),
        ("dir", name("dir")),
        ("|", S(Pipe)),
        ("filter", name("filter")),
        ("(", S(LParen)),
        ("e", name("e")),
        ("=>", S(FatArrow)),
        ("e.size", name("e.size")),
        (">", S(Greater)),
        ("4096", int(4096)),
        (")", S(RParen)),
        ("|", S(Pipe)),
        ("map", name("map")),
        (".name", Kind::Selector(vec!["name".into()])),
    ];
    let result = lex(source);
    assert_eq!(result.errors, []);
    let actual: Vec<_> = result
        .tokens
        .iter()
        .filter(|t| t.kind != Kind::Eof)
        .map(|t| (Source::new(source).slice(t.span).unwrap(), t.kind.clone()))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn migrated_donor_lexer_forms() {
    tokens(
        "ten = 10\nadd = x1 y1 => x1 + y1\n\
            if x>y { x * y } else { x / y }\n\
            ten<=two ten>=two ten==two ten!=two True&&False True||False []",
        vec![
            name("ten"),
            S(Assign),
            int(10),
            Kind::Newline,
            name("add"),
            S(Assign),
            name("x1"),
            name("y1"),
            S(FatArrow),
            name("x1"),
            S(Plus),
            name("y1"),
            Kind::Newline,
            Kind::Kw(K::If),
            name("x"),
            S(Greater),
            name("y"),
            S(LBrace),
            name("x"),
            S(Star),
            name("y"),
            S(RBrace),
            Kind::Kw(K::Else),
            S(LBrace),
            name("x"),
            S(Slash),
            name("y"),
            S(RBrace),
            Kind::Newline,
            name("ten"),
            S(LessEq),
            name("two"),
            name("ten"),
            S(GreaterEq),
            name("two"),
            name("ten"),
            S(Eq),
            name("two"),
            name("ten"),
            S(NotEq),
            name("two"),
            name("True"),
            S(And),
            name("False"),
            name("True"),
            S(Or),
            name("False"),
            S(LBracket),
            S(RBracket),
        ],
    );
}

#[test]
fn canonical_display_retains_semantic_payloads() {
    let source = r#"let _ = -0 0xFF 1_0.5e-2 b"\xFF\0" "한글\n" 'raw \xFF'
        --width ../x .field fs.Entry + - * / % ++ & | |: |! || &&
        10ms -0x1Fg 2.5e re"a+b"i re'a\d' "\xFF"i 0r7Z -0ms 0i
        18446744073709551616"#;
    for token in lex(source).tokens {
        if token.kind != Kind::Eof {
            tokens(&token.kind.to_string(), vec![token.kind]);
        }
    }
    assert_eq!(Kind::Eof.to_string(), "<eof>");
}

#[test]
#[allow(clippy::too_many_lines, reason = "One complete module token oracle")]
fn opening_module_is_one_complete_token_stream() {
    tokens(
        include_str!("fixtures/opening.fipe"),
        vec![
            Kind::Newline,
            Kind::Kw(K::Use),
            name("fs"),
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            name("ls"),
            Kind::Flag("l".into()),
            Kind::Path("/etc".into()),
            S(Pipe),
            name("filter"),
            S(LParen),
            name("e"),
            S(FatArrow),
            name("e.size"),
            S(Greater),
            int(4096),
            S(RParen),
            S(Pipe),
            name("map"),
            Kind::Selector(vec!["name".into()]),
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            name("dir"),
            S(Assign),
            name("sys.lookup"),
            name("root"),
            Kind::Path("/module/fs/root".into()),
            S(Pipe),
            name("Rd"),
            S(Less),
            name("fs"),
            S(Greater),
            Kind::Newline,
            name("fs.list"),
            name("dir"),
            Kind::Path("/etc".into()),
            S(Pipe),
            name("map"),
            Kind::Selector(vec!["name".into()]),
            S(Pipe),
            name("put"),
            Kind::Path("./names".into()),
            Kind::Newline,
            name("cat"),
            S(LParen),
            name("Rd"),
            int(4),
            S(RParen),
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            name("add"),
            S(Assign),
            name("a"),
            name("b"),
            S(FatArrow),
            name("a"),
            S(Plus),
            name("b"),
            Kind::Newline,
            name("add2"),
            S(Assign),
            name("add"),
            int(2),
            Kind::Newline,
            name("add2"),
            int(4),
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Kw(K::Pub),
            name("big"),
            S(Colon),
            name("Rd"),
            S(Less),
            name("fs"),
            S(Greater),
            S(Arrow),
            name("U64"),
            S(Arrow),
            name("Result"),
            S(Less),
            name("Vec"),
            S(Less),
            name("String"),
            S(Greater),
            S(Comma),
            name("fs.Error"),
            S(Greater),
            S(Assign),
            name("dir"),
            name("min"),
            S(FatArrow),
            S(LBrace),
            Kind::Newline,
            name("Ok"),
            name("_"),
            S(Assign),
            name("fs.remove"),
            name("dir"),
            Kind::Path("/tmp/x".into()),
            Kind::Newline,
            name("fs.list"),
            name("dir"),
            Kind::Path("/etc".into()),
            S(PipeGood),
            name("filter"),
            S(LParen),
            name("e"),
            S(FatArrow),
            name("e.size"),
            S(Greater),
            name("min"),
            S(RParen),
            S(PipeGood),
            name("map"),
            Kind::Selector(vec!["name".into()]),
            Kind::Newline,
            S(RBrace),
            Kind::Newline,
            Kind::Newline,
            name("find"),
            name("x"),
            S(PipeGood),
            name("cat"),
            S(PipeBad),
            name("log"),
            string("gone", false),
            S(Or),
            name("root"),
            Kind::Newline,
            name("find"),
            name("x"),
            Kind::Newline,
            S(PipeGood),
            name("cat"),
            Kind::Newline,
            S(PipeBad),
            name("log"),
            string("gone", false),
            Kind::Newline,
            S(Or),
            name("root"),
            Kind::Newline,
            name("find"),
            name("x"),
            S(PipeGood),
            name("cat"),
            S(Or),
            name("fault"),
            string("no x", false),
            S(Pipe),
            name("append"),
            Kind::Path("./log".into()),
            Kind::Newline,
            name("find"),
            name("x"),
            S(Pipe),
            S(LBrace),
            Kind::Newline,
            S(LBracket),
            S(Colon),
            name("Entry"),
            S(LBrace),
            name("name"),
            S(Comma),
            Kind::Word("..".into()),
            S(RBrace),
            S(RBracket),
            Kind::Kw(K::If),
            name("name"),
            S(NotEq),
            string("", false),
            S(FatArrow),
            name("name"),
            S(Comma),
            Kind::Newline,
            S(LBracket),
            S(Not),
            S(RBracket),
            S(FatArrow),
            string("-", false),
            S(Comma),
            Kind::Newline,
            S(RBrace),
            Kind::Newline,
            Kind::Newline,
            Kind::Newline,
            Kind::Kw(K::Pub),
            Kind::Kw(K::Type),
            name("Point"),
            S(LBrace),
            Kind::Kw(K::Pub),
            name("x"),
            S(Colon),
            name("I64"),
            S(Comma),
            Kind::Kw(K::Pub),
            name("y"),
            S(Colon),
            name("I64"),
            S(Assign),
            int(0),
            S(Comma),
            name("len"),
            S(Colon),
            name("U64"),
            S(RBrace),
            Kind::Newline,
            Kind::Kw(K::Impl),
            name("Point"),
            S(LBrace),
            Kind::Newline,
            name("x"),
            S(LParen),
            name("y"),
            S(Assign),
            int(0),
            S(RParen),
            S(FatArrow),
            name("Self"),
            S(LBrace),
            name("x"),
            S(Comma),
            name("y"),
            S(Comma),
            name("len"),
            S(Colon),
            name("hypot"),
            name("x"),
            name("y"),
            S(RBrace),
            S(Comma),
            Kind::Newline,
            name("dist"),
            S(Assign),
            name("self"),
            name("q"),
            S(FatArrow),
            name("hypot"),
            S(LParen),
            name("self.x"),
            S(Minus),
            name("q.x"),
            S(RParen),
            S(LParen),
            name("self.y"),
            S(Minus),
            name("q.y"),
            S(RParen),
            S(Comma),
            Kind::Newline,
            S(RBrace),
            Kind::Newline,
            Kind::Kw(K::Impl),
            name("Point"),
            S(LBrace),
            name("show"),
            S(Assign),
            name("self"),
            S(FatArrow),
            name("join"),
            string(",", false),
            S(LBracket),
            name("show"),
            name("self.x"),
            S(Comma),
            name("show"),
            name("self.y"),
            S(RBracket),
            S(RBrace),
            Kind::Newline,
            name("Point"),
            int(3),
            int(4),
            S(Pipe),
            name("show"),
            Kind::Newline,
            Kind::Newline,
            Kind::Kw(K::With),
            name("cwd"),
            Kind::Path("/tmp".into()),
            S(LBrace),
            name("ls"),
            S(RBrace),
            Kind::Newline,
        ],
    );
}

#[test]
fn specification_corpus_preserves_every_emitted_spelling() {
    let source = include_str!("../Documentation/spec/examples.fipe");
    let result = lex(source);
    assert_eq!(result.errors, []);
    for token in result.tokens {
        let spelling = Source::new(source).slice(token.span).unwrap();
        if token.kind != Kind::Eof {
            tokens(spelling, vec![token.kind]);
        }
    }
}
