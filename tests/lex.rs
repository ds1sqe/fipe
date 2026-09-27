mod common;

use common::{int, name, negative, string, tokens};
use fipe::{
    lex::lex,
    token::{Keyword as K, Kind, Kind::Symbol as S, Source, Symbol::*},
};

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
        (r#"b"text""#, vec![Kind::Bytes(b"text".to_vec())]),
        ("rd:4", vec![Kind::Rd(4)]),
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
    for keyword in [
        K::Let,
        K::If,
        K::Else,
        K::With,
        K::Fn,
        K::Struct,
        K::Enum,
        K::Trait,
        K::Impl,
        K::For,
        K::Type,
        K::Use,
        K::Ver,
        K::As,
    ] {
        tokens(keyword.spelling(), vec![Kind::Kw(keyword)]);
        assert_eq!(keyword.to_string(), keyword.spelling());
    }
    for symbol in [
        Plus, Minus, Star, Slash, Percent, LParen, RParen, LBrace, RBrace,
        LBracket, RBracket, Comma, Semicolon, Colon, Assign, Eq, Not, NotEq,
        Less, LessEq, Greater, GreaterEq, Append, Arrow, FatArrow, Pipe,
        PipeGood, PipeBad, Or, And,
    ] {
        tokens(symbol.spelling(), vec![S(symbol)]);
        assert_eq!(symbol.to_string(), symbol.spelling());
    }
    tokens(
        "| |: |! || && = => == ! != < <= > >= >> - ->",
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
            S(Append),
            S(Minus),
            S(Arrow),
        ],
    );
    tokens("!b", vec![S(Not), name("b")]);
}

#[test]
fn word_boundaries_are_not_arithmetic_or_name_resolution() {
    for text in [
        "2024-01-01",
        "1st",
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
    tokens("rd:4x", vec![name("rd"), S(Colon), Kind::Word("4x".into())]);
    tokens("rd:-4", vec![name("rd"), S(Colon), negative(4)]);
    tokens("rd:4_0", vec![name("rd"), S(Colon), int(40)]);
    tokens("rd:", vec![name("rd"), S(Colon)]);
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
        "Vec<Vec<u8>>",
        vec![
            name("Vec"),
            S(Less),
            name("Vec"),
            S(Less),
            name("u8"),
            S(Append),
        ],
    );
    tokens(
        "[u8; 4096]",
        vec![
            S(LBracket),
            name("u8"),
            S(Semicolon),
            int(4096),
            S(RBracket),
        ],
    );
    tokens(
        "fn(a: A, b: B) -> C",
        vec![
            Kind::Kw(K::Fn),
            S(LParen),
            name("a"),
            S(Colon),
            name("A"),
            S(Comma),
            name("b"),
            S(Colon),
            name("B"),
            S(RParen),
            S(Arrow),
            name("C"),
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
        "let ten = 10\nfn add(x1,y1) { x1 + y1 }\n\
            if x>y { x * y } else { x / y }\n\
            ten<=two ten>=two ten==two ten!=two True&&False True||False []",
        vec![
            Kind::Kw(K::Let),
            name("ten"),
            S(Assign),
            int(10),
            Kind::Newline,
            Kind::Kw(K::Fn),
            name("add"),
            S(LParen),
            name("x1"),
            S(Comma),
            name("y1"),
            S(RParen),
            S(LBrace),
            name("x1"),
            S(Plus),
            name("y1"),
            S(RBrace),
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
        --width rd:40 ../x .field fs.Entry + - * / % | |: |! || &&"#;
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
            Kind::Kw(K::Use),
            name("fs"),
            Kind::Newline,
            Kind::Kw(K::Let),
            name("dir"),
            S(Assign),
            S(LParen),
            name("sys.lookup"),
            name("root"),
            Kind::Path("/module/fs/root".into()),
            S(Pipe),
            Kind::Kw(K::As),
            name("Rd"),
            S(Less),
            name("fs"),
            S(Greater),
            S(RParen),
            Kind::Newline,
            name("ls"),
            Kind::Flag("l".into()),
            name("dir"),
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
            Kind::Kw(K::Let),
            name("add"),
            S(Assign),
            Kind::Kw(K::Fn),
            S(LParen),
            name("a"),
            S(Comma),
            name("b"),
            S(RParen),
            S(LBrace),
            name("a"),
            S(Plus),
            name("b"),
            S(RBrace),
            Kind::Newline,
            Kind::Kw(K::Let),
            name("add2"),
            S(Assign),
            name("add"),
            int(2),
            Kind::Newline,
            name("add2"),
            int(4),
            Kind::Newline,
            Kind::Newline,
            Kind::Kw(K::Fn),
            name("big"),
            S(LParen),
            name("dir"),
            S(Colon),
            name("Rd"),
            S(Less),
            name("fs"),
            S(Greater),
            S(Comma),
            name("min"),
            S(Colon),
            name("u64"),
            S(RParen),
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
            S(LBrace),
            Kind::Newline,
            Kind::Kw(K::Let),
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
            S(Or),
            name("root"),
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
