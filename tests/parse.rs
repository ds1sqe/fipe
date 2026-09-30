#[path = "parse/docs.rs"]
mod docs;
use fipe::{
    ast::{Entry, ExprId, ExprKind, Literal, Module},
    lex::lex,
    parse::{parse, Error},
    token::Source,
};

#[path = "parse/expressions.rs"]
mod expressions;
#[path = "parse/forms.rs"]
mod forms;
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
            magnitude: expected.into(),
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
    let corpus = include_str!("../Documentation/spec/02-syntax.fipe");
    let parsed = module(corpus);
    assert_eq!(Source::new(corpus).slice(parsed.span), Some(corpus));
    let prelude = include_str!("../prelude/prelude.fipe");
    let parsed = module(prelude);
    assert_eq!(Source::new(prelude).slice(parsed.span), Some(prelude));
    assert!(parsed.entries.len() > 40);
}

/// Every program under `Documentation/examples` parses and covers the
/// whole source; the directory listing keeps new examples from being missed.
#[test]
fn documentation_examples_parse() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples");
    let mut paths: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fipe"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 5, "{directory:?}");
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = parse(lex(&text)).unwrap_or_else(|errors| {
            let located: Vec<_> = errors
                .iter()
                .map(|error| {
                    (
                        &error.reason,
                        Source::new(&text).line_col(error.span.start),
                        Source::new(&text).slice(error.span),
                    )
                })
                .collect();
            panic!("{}: {located:?}", path.display())
        });
        assert_eq!(Source::new(&text).slice(parsed.span), Some(text.as_str()));
        assert!(!parsed.entries.is_empty(), "{}", path.display());
    }
}

/// Every example block of the type model's chapters parses whole.
#[test]
fn type_model_chapter_examples_parse() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/spec");
    for chapter in ["06-limits.md", "04-members.md", "05-containers.md"] {
        let text = std::fs::read_to_string(directory.join(chapter)).unwrap();
        let blocks: Vec<_> = text
            .split("```fipe\n")
            .skip(1)
            .map(|rest| rest.split("```").next().unwrap())
            .collect();
        assert!(!blocks.is_empty(), "{chapter}");
        for block in blocks {
            module(block);
        }
    }
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
