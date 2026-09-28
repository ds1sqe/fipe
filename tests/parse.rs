use fipe::{
    ast::{Entry, ExprId, ExprKind, Literal, Module},
    lex::lex,
    parse::{parse, Error},
    token::Source,
};

#[path = "parse/expressions.rs"]
mod expressions;
#[path = "parse/items.rs"]
mod items;
#[path = "parse/lines.rs"]
mod lines;
#[path = "parse/recovery.rs"]
mod recovery;

fn module(input: &str) -> Module {
    parse(lex(input)).unwrap_or_else(|errors| {
        let located: Vec<_> = errors
            .iter()
            .map(|error| {
                (
                    &error.reason,
                    Source::new(input).line_col(error.span.start),
                    Source::new(input).slice(error.span),
                )
            })
            .collect();
        panic!("{input:?}: {located:?}")
    })
}

fn only(input: &str) -> (Module, ExprId) {
    let module = module(input);
    let [Entry::Line(id)] = module.entries.as_slice() else {
        panic!("expected one expression: {:?}", module.entries);
    };
    let id = *id;
    (module, id)
}

fn node(module: &Module, id: ExprId) -> &ExprKind {
    &module.expr(id).unwrap().kind
}

fn name(module: &Module, id: ExprId, expected: &str) {
    let ExprKind::Name(name) = node(module, id) else {
        panic!("expected name {expected}, got {:?}", node(module, id));
    };
    assert_eq!(name.segments, expected.split('.').collect::<Vec<_>>());
}

fn integer(module: &Module, id: ExprId, expected: u64) {
    assert_eq!(
        node(module, id),
        &ExprKind::Literal(Literal::Int {
            negative: false,
            magnitude: expected,
        })
    );
}

fn errors(input: &str) -> Vec<Error> {
    parse(lex(input)).expect_err(input)
}

#[test]
fn opening_module_and_formal_corpus_parse() {
    let opening = include_str!("fixtures/opening.fipe");
    let parsed = module(opening);
    assert_eq!(Source::new(opening).slice(parsed.span), Some(opening));
    let corpus = include_str!("../Documentation/spec/examples.fipe");
    let parsed = module(corpus);
    assert_eq!(Source::new(corpus).slice(parsed.span), Some(corpus));
}

#[test]
fn empty_sources_and_bom_preserve_module_identity() {
    for input in ["", "# comment\n", "\u{feff}"] {
        let parsed = module(input);
        assert_eq!(parsed.entries, []);
        assert_eq!(Source::new(input).slice(parsed.span), Some(input));
    }
    let (parsed, id) = only("\u{feff}ls");
    name(&parsed, id, "ls");
    assert_eq!(parsed.expr(id).unwrap().span.start, 3);
}

#[test]
fn arithmetic_precedence_and_argument_rejection_are_distinct() {
    use fipe::ast::BinaryOp;
    let (parsed, id) = only("1 + 2 * 3");
    let ExprKind::Binary {
        op: BinaryOp::Add,
        left,
        right,
        ..
    } = node(&parsed, id)
    else {
        panic!("expected addition");
    };
    integer(&parsed, *left, 1);
    let ExprKind::Binary {
        op: BinaryOp::Multiply,
        left,
        right,
        ..
    } = node(&parsed, *right)
    else {
        panic!("expected multiplication");
    };
    integer(&parsed, *left, 2);
    integer(&parsed, *right, 3);
    let input = "add 2 - 1";
    let errors = errors(input);
    assert_eq!(errors[0].reason, fipe::parse::Reason::OperatorInArguments);
    assert_eq!(Source::new(input).slice(errors[0].span), Some("-"));
}
