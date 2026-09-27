use fipe::{
    lex::lex,
    token::{Kind, Source, Span},
};

pub fn name(text: &str) -> Kind {
    Kind::Name(text.split('.').map(str::to_owned).collect())
}

pub const fn int(magnitude: u64) -> Kind {
    Kind::Int {
        negative: false,
        magnitude,
    }
}

pub const fn negative(magnitude: u64) -> Kind {
    Kind::Int {
        negative: true,
        magnitude,
    }
}

pub fn string(value: &str, raw: bool) -> Kind {
    Kind::Str {
        value: value.to_owned(),
        raw,
    }
}

pub fn tokens(input: &str, expected: Vec<Kind>) {
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
