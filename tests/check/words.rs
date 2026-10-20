//! The words a reader reads for each reason family, through the
//! public reason types.
use super::check;
use fipe::{
    decl::Declarations,
    embed::Outcome,
    lex::lex,
    parse::parse,
    resolve::resolve,
    session::{Diagnostic, Session},
};

fn diagnostics(lines: &[&str]) -> Vec<Diagnostic> {
    let mut session = Session::new(Declarations::new());
    let mut last = Vec::new();
    for text in lines {
        let Outcome::Done(report) = session.line(text) else {
            panic!("{text}: a request")
        };
        last = report.diagnostics;
    }
    last
}

#[test]
fn a_syntax_error_says_what_was_expected_in_words() {
    let errors = parse(lex("x = (1, 2")).unwrap_err();
    assert_eq!(errors[0].reason.to_string(), "expected )");
    let errors = parse(lex("a < b < c")).unwrap_err();
    assert_eq!(errors[0].reason.to_string(), "group chained comparisons");
}

#[test]
fn a_resolution_error_names_the_problem_and_leaves_the_site_to_the_reader() {
    let module = parse(lex("y = nope + 1")).unwrap();
    let errors = resolve(&module, &Declarations::new(), &[]).unwrap_err();
    assert_eq!(errors[0].reason.to_string(), "unknown name");
    let module = parse(lex("type A { n: I64 }\ntype A { m: I64 }")).unwrap();
    let errors = resolve(&module, &Declarations::new(), &[]).unwrap_err();
    assert_eq!(errors[0].reason.to_string(), "already defined");
}

#[test]
fn a_type_error_writes_both_types_in_the_spec_syntax() {
    let (_, _, errors) =
        check("f: I64 -> I64 = n => n\nf True", &Declarations::new());
    assert_eq!(errors[0].reason.to_string(), "expected I64, found Bool");
    let (_, _, errors) = check("f = n => n\nf 1 2", &Declarations::new());
    assert_eq!(errors[0].reason.to_string(), "one argument too many");
}

#[test]
fn a_declaration_error_reads_as_the_file_problem() {
    let mut declarations = Declarations::new();
    let error = declarations
        .module("m", "pub f: (a: I64) -> (a: I64) -> I64")
        .unwrap_err();
    assert_eq!(error[0].reason.to_string(), "spot `a` is declared twice");
}

#[test]
fn a_compile_time_refutation_names_the_limit_and_the_value() {
    let found = diagnostics(&[
        "even = n => n % 2 == 0",
        "type Even: Integer where even self",
        "e: Even = 3",
    ]);
    let [Diagnostic::Compile { what, reason, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_eq!(what.describe(reason), "refuted entry against Even: 3");
}

#[test]
fn a_run_time_fault_reads_with_what_the_tables_know() {
    let mut session = Session::new(Declarations::new());
    let Outcome::Done(report) = session.line("1 / ([0] | head)") else {
        panic!("a request")
    };
    let program = fipe::compile::compile(report.unit.as_ref().unwrap());
    let mut machine = fipe::vm::Machine::new(Some(1 << 20));
    let Outcome::Done(Err(fault)) = machine.run(&program) else {
        panic!("no fault")
    };
    let tables = program.tables.borrow();
    assert_eq!(
        fault.reason.describe(&tables),
        "division by zero: divisor is 0"
    );
}
