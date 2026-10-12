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
