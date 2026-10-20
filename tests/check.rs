use fipe::{
    ast::{Entry, ExprKind, Module},
    decl::Declarations,
    infer::{infer_partial, Error},
    lex::lex,
    parse::parse,
    resolve::resolve,
    typed::Typed,
};

fn check(
    text: &str,
    declarations: &Declarations,
) -> (Module, Typed, Vec<Error>) {
    let module = parse(lex(text)).unwrap();
    let resolved = resolve(&module, declarations, &[]).unwrap();
    let (typed, errors) = infer_partial(&module, &resolved, declarations, &[]);
    (module, typed, errors)
}

fn ty(text: &str) -> String {
    let (module, typed, errors) = check(text, &Declarations::new());
    assert!(errors.is_empty(), "{text}: {errors:?}");
    let Some(Entry::Line(id)) = module.entries.last() else {
        panic!()
    };
    if matches!(module.expr(*id).unwrap().kind, ExprKind::Bind { .. }) {
        typed.binding(*id).unwrap().to_string()
    } else {
        typed.expr(*id).unwrap().to_string()
    }
}

#[path = "check/binding.rs"]
mod binding;
#[path = "check/ir.rs"]
mod ir;

#[path = "check/casts.rs"]
mod casts;
#[path = "check/compile_phase.rs"]
mod compile_phase;
#[path = "check/coverage.rs"]
mod coverage;
#[path = "check/outcomes.rs"]
mod outcomes;
#[path = "check/words.rs"]
mod words;

#[test]
fn a_module_with_a_malformed_line_types_the_rest_and_is_no_unit() {
    use fipe::{
        check::{check, Error as CheckError, Options},
        parse::parse_partial,
        resolve::resolve_partial,
    };
    let declarations = Declarations::new();
    let input = "n = 1 + 2\nq = )\nm = n * 2\n";
    let module = parse_partial(lex(input));
    let (resolved, _) = resolve_partial(&module, &declarations, &[]);
    let (typed, errors) = infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    let schemes: Vec<_> = typed
        .bindings()
        .map(|(_, scheme)| scheme.to_string())
        .collect();
    assert_eq!(schemes.len(), 2, "{schemes:?}");
    let refused = check(&module, &declarations, &[], Options::default())
        .expect_err("a module with an error node is no unit");
    assert!(
        matches!(refused.first(), Some(CheckError::Syntax(error)) if Some(error.span.start) == input.find(')')),
        "{refused:?}"
    );
}
