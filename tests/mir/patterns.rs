use fipe::{
    decl::Declarations,
    embed::Outcome,
    ir,
    mir::{Body, Callee, Operand, Program, Rvalue, Statement, Terminator},
    session::Session,
};

/// Sources with arm lists, patterns and dispatch: lowered and walked.
pub const ARM_LISTS: &[&str] = &[
    "{ Just v => v, Nothing => 0 }",
    "g = xs => xs | map (n => n + 1)\ng [1]",
    "1 + 2",
    "f: Integer -> String = { 0 => \"a\", _ => \"b\" }\nf 1",
    "k: Integer -> Integer = { x if x == 1 => 1, _ => 2 }\nk 1",
    "type Even: Integer where self % 2 == 0\nm: Integer -> Integer = { Even n => n, _ => 0 }\nm 1",
    "type Even: Integer where self % 2 == 0\nn: Maybe<Integer> -> Integer = { Just (Even v) => v, _ => 0 }\nn (Just 1)",
    "type Point { x: Integer, y: Integer }\nq: Point -> Integer = { Point { x, .. } => x }\nq (Point 1 2)",
    "p: Integer | String -> Integer = { Integer n => n, String _ => 0 }",
    "s: Maybe<Integer> -> Integer = { [:v] => v, [!] => 0 }\ns (Just 1)",
    "type Point: Show { x: Integer, y: Integer }\np = Point 1 2\np.show",
    "type Point { x: Integer, y: Integer }\nget = p => p.x\nget (Point 1 2)",
    "[x, y] = [1, 2]\nx",
    "(a, b) = (1, 2)\nb",
    "r: Integer * Integer -> Integer = { (a, b) if a == b => 1, _ => 2 }\nr (1, 2)",
    "w: Vec<Integer> -> Integer = { [x, y] => x, [] => 0, _ => 1 }",
    "two: Maybe<Integer> -> Maybe<Integer> -> Integer = { (Just a) (Just b) => a, _ (Nothing) => 0, (Nothing) _ => 1 }\ntwo (Just 1) Nothing",
    "e: Integer -> Integer = { 0 => 1, 1 => 2, _ => 3 }\ne 2",
    "type Mask: Literal & Eq { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nmk: Mask -> String = { 0 => \"a\", _ => \"b\" }",
    "eqs = a b => a == b\neqs 1 2",
];

fn body_of_definition<'p>(program: &'p Program, label: &str) -> &'p Body {
    let tables = program.tables.borrow();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| definition.label == label)
        .unwrap_or_else(|| panic!("no definition {label}"));
    let body = program.definitions[definition]
        .unwrap_or_else(|| panic!("{label} has no body"));
    &program.bodies[body]
}

/// Each switch's cases and its `otherwise`, in block order.
type Switches<'b> = Vec<(&'b [(usize, usize)], Option<usize>)>;

fn switches(body: &Body) -> Switches<'_> {
    body.blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Terminator::Switch {
                cases, otherwise, ..
            } => Some((cases.as_slice(), *otherwise)),
            _ => None,
        })
        .collect()
}

fn callees(body: &Body) -> Vec<&Callee> {
    body.blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Terminator::Call { callee, .. } => Some(callee),
            _ => None,
        })
        .collect()
}

fn native_label(program: &Program, callee: &Callee) -> Option<String> {
    match callee {
        Callee::Native(index) => {
            Some(program.tables.borrow().declarations[*index].label.clone())
        }
        _ => None,
    }
}

fn dispatch_label(program: &Program, callee: &Callee) -> Option<String> {
    match callee {
        Callee::Dispatch { member, .. } => {
            Some(program.tables.borrow().dispatch[*member].label.clone())
        }
        _ => None,
    }
}

fn main_body(program: &Program) -> &Body {
    &program.bodies[program.main]
}

#[test]
fn a_sum_arm_list_is_one_switch_without_otherwise() {
    let (_, program) = super::program("{ Just v => v, Nothing => 0 }");
    let body = &program.bodies[0];
    assert_eq!(body.arity, 1);
    let switches = switches(body);
    assert_eq!(switches.len(), 1, "{switches:?}");
    let (cases, otherwise) = switches[0];
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].0, 0, "Just first, as written");
    assert_eq!(cases[1].0, 1, "then Nothing");
    assert_eq!(otherwise, None, "coverage is proved");
    assert!(body
        .blocks
        .iter()
        .all(|block| !matches!(block.terminator, Terminator::Unreachable)));
}

#[test]
fn a_generic_body_dispatches_at_run_time() {
    let (_, program) = super::program("g = xs => xs | map (n => n + 1)\ng [1]");
    let lambda = program
        .bodies
        .iter()
        .find(|body| {
            callees(body).iter().any(|callee| {
                dispatch_label(&program, callee).as_deref() == Some("add")
            })
        })
        .expect("the lambda adds by dispatch");
    let witnesses: Vec<_> = callees(lambda)
        .into_iter()
        .filter_map(|callee| match callee {
            Callee::Dispatch { witness, .. } => Some(witness.clone()),
            _ => None,
        })
        .collect();
    assert!(
        witnesses.iter().any(|witness| matches!(
            witness,
            fipe::mir::Witness::Meaning(ir::Type::Parameter(_))
        )),
        "the literal reads by the meaning witness: {witnesses:?}"
    );
    assert!(
        witnesses
            .iter()
            .any(|witness| matches!(witness, fipe::mir::Witness::Value(_))),
        "add dispatches on the value's tag: {witnesses:?}"
    );
}

#[test]
fn a_concrete_operator_is_native() {
    let (_, program) = super::program("1 + 2");
    let labels: Vec<_> = callees(main_body(&program))
        .into_iter()
        .filter_map(|callee| native_label(&program, callee))
        .collect();
    assert_eq!(labels, ["add"]);
}

#[test]
fn a_literal_arm_compares_then_switches() {
    let (_, program) = super::program(
        "f: Integer -> String = { 0 => \"a\", _ => \"b\" }\nf 1",
    );
    let body = body_of_definition(&program, "f");
    let labels: Vec<_> = callees(body)
        .into_iter()
        .filter_map(|callee| native_label(&program, callee))
        .collect();
    assert_eq!(labels, ["eq"], "the recorded equality");
    let switches = switches(body);
    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0].0.len(), 2, "True and False");
    assert_eq!(switches[0].1, None);
    let unreachable = body
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, Terminator::Unreachable))
        .count();
    assert_eq!(unreachable, 0, "the wildcard takes the miss");
}

#[test]
fn a_guard_switches_after_the_binds() {
    let (_, program) = super::program(
        "k: Integer -> Integer = { x if x == 1 => 1, _ => 2 }\nk 1",
    );
    let body = body_of_definition(&program, "k");
    let entry = &body.blocks[0];
    let bind = entry.statements.iter().position(|statement| {
        matches!(
            statement,
            Statement::Assign(_, Rvalue::Use(Operand::Local(1)))
        )
    });
    assert!(bind.is_some(), "x is bound to the spot: {entry:?}");
    assert!(!switches(body).is_empty(), "the guard switches");
    assert!(
        body.blocks
            .iter()
            .all(|block| !matches!(block.terminator, Terminator::Unreachable)),
        "the wildcard follows the guard"
    );
}

#[test]
fn a_last_guard_leaves_an_unreachable_miss() {
    let (_, program) = super::program(
        "g: Integer -> Integer = { _ => 1, x if x == 1 => 2 }\ng 1",
    );
    let body = body_of_definition(&program, "g");
    assert!(
        switches(body).is_empty(),
        "the wildcard wins first: {body:?}"
    );
}

#[test]
fn a_gate_tests_and_reaches_its_limit_tests() {
    let (unit, program) =
        super::program("type Even: Integer where self % 2 == 0\nm: Integer -> Integer = { Even n => n, _ => 0 }\nm 1");
    let body = body_of_definition(&program, "m");
    let tests: Vec<_> = body
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Terminator::Test { predicate, .. } => Some(predicate.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tests.len(), 1, "{body:?}");
    assert!(mentions_a_test(&tests[0]));
    assert_eq!(program.tests.len(), unit.tables().tests.len());
    let reached = program.tests.iter().flatten().count();
    assert!(reached > 0, "the gate's tests have bodies");
    for body in program.tests.iter().flatten() {
        assert_eq!(program.bodies[*body].arity, 1);
    }
}

fn mentions_a_test(predicate: &ir::Predicate) -> bool {
    match predicate {
        ir::Predicate::Nonzero { .. } => false,
        ir::Predicate::Function { parameter, result } => {
            mentions_a_test(parameter) || mentions_a_test(result)
        }
        ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
            parts.iter().any(mentions_a_test)
        }
        ir::Predicate::Not(inner) => mentions_a_test(inner),
        ir::Predicate::Test { .. } => true,
    }
}

#[test]
fn a_record_pattern_reads_fields_by_index() {
    let (_, program) = super::program(
        "type Point { x: Integer, y: Integer }\nq: Point -> Integer = { Point { x, .. } => x }\nq (Point 1 2)",
    );
    let body = body_of_definition(&program, "q");
    assert!(switches(body).is_empty(), "one constructor needs no switch");
    assert!(body.blocks.iter().any(|block| block.statements.iter().any(
        |statement| matches!(
            statement,
            Statement::Assign(_, Rvalue::Field(Operand::Local(1), 0))
        )
    )));
}

#[test]
fn a_packed_arm_list_switches_by_meaning() {
    let (unit, program) = super::program(
        "p: Integer | String -> Integer = { Integer n => n, String _ => 0 }",
    );
    let body = body_of_definition(&program, "p");
    let switches = switches(body);
    assert_eq!(switches.len(), 1);
    let tables = unit.tables();
    let labels: Vec<_> = switches[0]
        .0
        .iter()
        .map(|(meaning, _)| tables.meanings[*meaning].label.as_str())
        .collect();
    assert_eq!(labels, ["Integer", "String"]);
    assert_eq!(switches[0].1, None);
}

#[test]
fn a_side_arm_list_switches_on_constructors_zero_and_one() {
    let (_, program) = super::program(
        "s: Maybe<Integer> -> Integer = { [:v] => v, [!] => 0 }\ns (Just 1)",
    );
    let body = body_of_definition(&program, "s");
    let switches = switches(body);
    assert_eq!(switches.len(), 1);
    let keys: Vec<_> = switches[0].0.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, [0, 1]);
    assert_eq!(switches[0].1, None);
}

#[test]
fn a_derived_member_is_a_derived_callee() {
    let (_, program) = super::program(
        "type Point: Show { x: Integer, y: Integer }\np = Point 1 2\np.show",
    );
    let derived: Vec<_> = callees(main_body(&program))
        .into_iter()
        .filter_map(|callee| match callee {
            Callee::Derived { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(derived, [ir::Derived::Show]);
}

#[test]
fn a_field_member_in_a_generic_body_keeps_its_rows() {
    let (unit, program) = super::program(
        "type Point { x: Integer, y: Integer }\nget = p => p.x\nget (Point 1 2)",
    );
    let body = body_of_definition(&program, "get");
    let members: Vec<_> = callees(body)
        .into_iter()
        .filter_map(|callee| match callee {
            Callee::Dispatch { member, .. } => Some(*member),
            _ => None,
        })
        .collect();
    assert_eq!(members.len(), 1, "p.x waits for p's tag");
    let tables = unit.tables();
    assert!(tables.dispatch[members[0]]
        .rows
        .iter()
        .any(|row| matches!(row.target, ir::Implementation::Field(0))));
}

#[test]
fn a_binding_with_parts_reads_its_parts() {
    for source in ["[x, y] = [1, 2]\ny", "(a, b) = (1, 2)\na"] {
        let (_, program) = super::program(source);
        let body = main_body(&program);
        let fields: Vec<_> = body
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement {
                Statement::Assign(_, Rvalue::Field(_, index)) => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(fields, [0, 1], "{source}");
        // A line's names are its globals, stored for later lines.
        let stored = body
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(statement, Statement::Store { .. }))
            .count();
        assert_eq!(stored, 2, "{source}");
        assert!(
            matches!(
                body.blocks.last().unwrap().statements.last(),
                Some(Statement::Assign(0, Rvalue::Use(Operand::Global(_))))
            ),
            "{source}: {body:?}"
        );
    }
}

#[test]
fn a_binding_with_parts_in_a_body_binds_locals() {
    let (_, program) = super::program(
        "f: Integer * Integer -> Integer = p => {\n    (a, b) = p\n    a\n}",
    );
    assert!(program
        .bodies
        .iter()
        .flat_map(|body| &body.blocks)
        .flat_map(|block| &block.statements)
        .all(|statement| !matches!(statement, Statement::Store { .. })));
}

#[test]
fn a_list_arm_list_switches_on_the_length() {
    let (_, program) = super::program(
        "w: Vec<Integer> -> Integer = { [x, y] => x, [] => 0, _ => 1 }",
    );
    let body = body_of_definition(&program, "w");
    assert!(body.blocks.iter().any(|block| block.statements.iter().any(
        |statement| matches!(
            statement,
            Statement::Assign(_, Rvalue::Length(_))
        )
    )));
    let switches = switches(body);
    assert_eq!(switches.len(), 1);
    let keys: Vec<_> = switches[0].0.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, [2, 0]);
    assert!(switches[0].1.is_some(), "a length never completes a column");
}

#[test]
fn two_columns_share_one_switch_per_place() {
    let (_, program) = super::program(
        "two: Maybe<Integer> -> Maybe<Integer> -> Integer = { (Just a) (Just b) => a, _ (Nothing) => 0, (Nothing) _ => 1 }\ntwo (Just 1) Nothing",
    );
    let body = body_of_definition(&program, "two");
    let bodies = body
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, Terminator::Return))
        .count();
    assert_eq!(bodies, 3, "each arm's body once: {body:?}");
    assert!(body
        .blocks
        .iter()
        .all(|block| !matches!(block.terminator, Terminator::Unreachable)));
}

#[test]
fn a_program_reader_literal_pattern_calls_its_reader() {
    let (_, program) = super::program(
        "type Mask: Literal & Eq { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nmk: Mask -> String = { 0 => \"a\", _ => \"b\" }",
    );
    let body = body_of_definition(&program, "mk");
    let bodies = callees(body)
        .into_iter()
        .filter(|callee| matches!(callee, Callee::Body(_)))
        .count();
    assert_eq!(bodies, 1, "the reader runs on the written form");
}

#[test]
fn an_unfixed_session_slot_lowers_without_a_dispatch() {
    let mut session = Session::new(Declarations::new());
    let Outcome::Done(report) = session.line("n = Nothing") else {
        panic!("a request")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let Outcome::Done(report) = session.query("n") else {
        panic!("a request")
    };
    let unit = report.unit.expect("checked");
    let program = fipe::compile::compile(&unit);
    super::walk::check_program(&program);
    assert!(matches!(
        main_body(&program).blocks[0].statements.last(),
        Some(Statement::Assign(0, Rvalue::Use(Operand::Global(_))))
    ));
}
