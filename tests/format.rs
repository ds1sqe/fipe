//! Shared syntax formatting preserves the original token content.
use fipe::format as existing;

fn kinds(text: &str) -> Vec<fipe::token::Kind> {
    let lexed = fipe::lex::lex(text);
    assert!(lexed.errors.is_empty(), "{text:?}: {:?}", lexed.errors);
    lexed.tokens.into_iter().map(|token| token.kind).collect()
}

#[test]
fn ordinary_lf_and_crlf_sources_keep_their_tokens() {
    for source in [
        "x  =  1\ns = re'one\ntwo'\n",
        "x  =  1\r\ns = re'one\r\ntwo'\r\n",
    ] {
        fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        let formatted = existing::format(source).unwrap();
        assert_eq!(kinds(source), kinds(&formatted));
        assert_eq!(existing::format(&formatted).unwrap(), formatted);
    }
}

#[test]
fn mixed_newline_raw_quote_preserves_original_literal_content() {
    let source = "x = 1\r\ns = re'one\ntwo'\r\n";
    fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    assert!(
        existing::format(source).is_err(),
        "content-changing normalization must be refused"
    );
}

#[test]
fn syntax_only_formatting_preserves_touching_docs_and_comment_alignment() {
    for source in [
        "unknown  =   missing\n",
        "## doc  \nx  =  1  # comment  \n",
        "x = f(1)\ny = x.name\n",
    ] {
        let formatted = existing::format(source).unwrap();
        assert_eq!(kinds(source), kinds(&formatted));
        assert_eq!(existing::format(&formatted).unwrap(), formatted);
    }
    assert_eq!(
        existing::format("x = 1  # note  \n").unwrap(),
        "x = 1  # note\n"
    );
    assert!(existing::format("x = = 1\n").is_err());
}
