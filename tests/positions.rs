use fipe::{
    lex::{lex, Reason},
    token::{Kind, Source},
};

#[test]
fn diagnostic_after_unicode_points_to_its_quote_and_recovers() {
    let text = "# header\n한글 \"bad\nnext";
    let source = Source::new(text);
    let result = lex(text);
    assert_eq!(result.errors.len(), 1);
    let error = &result.errors[0];
    assert_eq!(error.reason, Reason::UnterminatedQuote);
    assert_eq!(source.slice(error.span), Some("\"bad"));
    assert_eq!(source.line_col(error.span.start), (2, 4));
    let next = result
        .tokens
        .iter()
        .find(|token| token.kind == Kind::Name(vec!["next".into()]))
        .unwrap();
    assert_eq!(source.line_col(next.span.start), (3, 1));
    assert_eq!(source.slice(next.span), Some("next"));
}
