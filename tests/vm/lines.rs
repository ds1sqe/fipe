//! Values one session line stores and a later line uses, on one
//! machine: each line is its own program.
use super::{run::BUDGET, structural, Declarations, Machine, Outcome};
use fipe::{compile, session::Session};

/// One session and one machine; each line is checked, then run, and
/// withdrawn when its run Faults.
struct Lines {
    session: Session,
    machine: Machine,
}

impl Lines {
    fn new() -> Self {
        Self {
            session: Session::new(Declarations::new()),
            machine: Machine::new(Some(BUDGET)),
        }
    }

    /// The line's value, or its Fault, or its diagnostics when refused.
    fn line(&mut self, text: &str) -> String {
        let Outcome::Done(report) = self.session.line(text) else {
            panic!("{text}: a request")
        };
        let Some(unit) = &report.unit else {
            return format!("refused: {:?}", report.diagnostics);
        };
        let program = compile::compile(unit);
        match self.machine.run(&program) {
            Outcome::Done(Ok(value)) => {
                structural::show(&value, &program.tables.borrow())
            }
            Outcome::Done(Err(fault)) => {
                let line = report.accepted.expect("an accepted line");
                let withdrawn = self.session.withdraw(line).expect("the last");
                self.machine.withdraw(&withdrawn);
                format!("fault: {}", fault.reason)
            }
            Outcome::Request(request) => panic!("{text}: {request:?}"),
        }
    }
}

#[test]
fn a_function_value_from_an_earlier_line_runs_its_own_body() {
    let mut lines = Lines::new();
    lines.line("g = (x => x) (y => y)");
    assert_eq!(lines.line("g 1.0"), "1.0");
}

#[test]
fn a_closure_in_a_vector_from_an_earlier_line_runs_its_own_body() {
    let mut lines = Lines::new();
    lines.line("sq = x => x * x");
    lines.line("a = [(x => x + 100)]");
    assert_eq!(lines.line("a | map (f => sq (f 5))"), "[11025]");
    lines.line("[f] = [(x => x)]");
    assert_eq!(lines.line("f True"), "True");
}

#[test]
fn a_partial_application_stored_by_one_line_is_called_by_the_next() {
    let mut lines = Lines::new();
    lines.line("add = a b => a + b");
    lines.line("inc: Integer -> Integer = add 1");
    assert_eq!(lines.line("inc 41"), "42");
}

#[test]
fn a_definition_numbered_again_after_a_withdrawal_runs_its_new_body() {
    let mut lines = Lines::new();
    let faulting = "h = x => x * 2\ny = h 5\nz = 1.0 / 0.0";
    assert_eq!(lines.line(faulting), "fault: the result is not finite");
    lines.line("h = x => x + 1");
    assert_eq!(lines.line("h 5"), "6");
}

#[test]
fn a_generic_value_binding_runs_at_each_use_not_at_its_line() {
    let mut lines = Lines::new();
    lines.line("add = a b => a + b");
    assert_eq!(lines.line("inc = add 1"), "()");
    assert_eq!(lines.line("inc 41"), "42");
    assert_eq!(lines.line("inc 1.5"), "2.5");
    lines.line("mk = n => (x => x + n)");
    assert_eq!(lines.line("add10 = mk 10"), "()");
    assert_eq!(lines.line("add10 5"), "15");
}

#[test]
fn a_later_line_converts_by_a_claim_of_a_type_that_also_shows() {
    let mut lines = Lines::new();
    let unit = "type A { n: I64 }\n\
                type B: Show & From<A> { m: I64 }\n\
                impl B: Show { show = self => \"b\" }\n\
                impl B: From<A> { from: A -> Self = a => Self a.n }";
    assert_eq!(lines.line(unit), "()");
    assert_eq!(lines.line("(A 2) | B.from"), "B 2");
}

#[test]
fn a_later_line_tries_back_by_a_claim_beside_show_claims() {
    let mut lines = Lines::new();
    let unit = "type A: Show & TryFrom<B, E> { n: I64 }\n\
                type B: Show & From<A> { m: I64 }\n\
                type E: Show { r: I64 }\n\
                impl A: Show { show = self => \"a\" }\n\
                impl B: Show { show = self => \"b\" }\n\
                impl E: Show { show = self => \"e\" }\n\
                impl B: From<A> { from: A -> Self = a => Self a.n }\n\
                impl A: TryFrom<B, E> { try_from: B -> Result<Self, E> = b => Ok (A b.m) }";
    assert_eq!(lines.line(unit), "()");
    lines.line("b = (A 1) | B.from");
    assert_eq!(lines.line("b | try<A>"), "Ok (A 1)");
}

#[test]
fn an_empty_vector_runs_and_a_later_show_names_its_unfixed_element() {
    let mut lines = Lines::new();
    assert_eq!(lines.line("x = []"), "()");
    assert_eq!(lines.line("x"), "[]");
    // The element meaning is unfixed, so `show` has no claim to choose.
    let refused = lines.line("x | show");
    assert!(
        refused.starts_with("refused: ")
            && refused.contains("Ambiguous { class: \"Show\" }"),
        "{refused}"
    );
}
