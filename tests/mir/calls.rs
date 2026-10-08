use fipe::{
    compile,
    decl::Declarations,
    embed::Outcome,
    mir::{Callee, Operand, Program, Rvalue, Statement, Terminator},
    session::Session,
    value::Value,
};

fn main_body(program: &Program) -> &fipe::mir::Body {
    &program.bodies[program.main]
}

/// The statements of every block of a body, in block order.
fn statements(body: &fipe::mir::Body) -> Vec<&Statement> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .collect()
}

/// The block index of the first call whose callee satisfies `found`.
fn call_block(
    body: &fipe::mir::Body,
    found: impl Fn(&Callee) -> bool,
) -> Option<usize> {
    body.blocks.iter().position(|block| {
        matches!(&block.terminator, Terminator::Call { callee, .. } if found(callee))
    })
}

fn label(program: &Program, member: usize) -> String {
    program.tables.borrow().dispatch[member].label.clone()
}

#[test]
fn a_literal_is_one_constant_used_once() {
    let (_, program) = super::program("16");
    assert_eq!(program.constants, [Value::Int(16)]);
    let body = main_body(&program);
    assert_eq!(body.blocks.len(), 1);
    assert_eq!(
        body.blocks[0].statements,
        [
            Statement::Charge(1),
            Statement::Assign(0, Rvalue::Use(Operand::Const(0)))
        ]
    );
    assert_eq!(body.blocks[0].terminator, Terminator::Return);
}

#[test]
fn supplies_then_defaults_then_the_call() {
    let (_, program) = super::program("f = (w = 80) (c = (w / 8)) => c\nf 16");
    let body = main_body(&program);
    let sixteen = program
        .constants
        .iter()
        .position(|constant| *constant == Value::Int(16))
        .unwrap();
    let spot = statements(body)
        .iter()
        .position(|statement| {
            matches!(statement, Statement::Assign(_, Rvalue::Use(Operand::Const(index))) if *index == sixteen)
        })
        .expect("16 is evaluated into the spot");
    let tables = program.tables.borrow();
    let default = call_block(body, |callee| {
        matches!(callee, Callee::Native(index) if tables.declarations[*index].label == "div")
    })
    .expect("the default divides by the prelude's row");
    drop(tables);
    let call = call_block(body, |callee| matches!(callee, Callee::Body(_)))
        .expect("the body is called");
    assert!(default < call, "the default runs before the call");
    assert_eq!(spot, 1, "16 is the first assignment after the charge");
    if let Terminator::Call { args, meanings, .. } =
        &body.blocks[call].terminator
    {
        assert_eq!(args.len(), 2);
        assert_eq!(meanings.len(), 1, "the body's meaning witness");
    }
}

#[test]
fn a_known_definition_is_called_by_its_body() {
    let (_, program) = super::program("add2 = x => x + 2\nadd2 4");
    let body = main_body(&program);
    let call =
        call_block(body, |callee| matches!(callee, Callee::Body(_))).unwrap();
    let Terminator::Call {
        callee: Callee::Body(index),
        args,
        ..
    } = &body.blocks[call].terminator
    else {
        panic!()
    };
    assert_eq!(args.len(), 1);
    assert_eq!(program.definitions[0], Some(*index));
    let add2 = &program.bodies[*index];
    assert_eq!(add2.arity, 1);
    assert!(call_block(add2, |callee| {
        matches!(callee, Callee::Dispatch { member, witness: fipe::mir::Witness::Value(_) } if label(&program, *member) == "add")
    })
    .is_some(), "add dispatches on its receiver in the generic body");
}

#[test]
fn an_open_spot_makes_a_thunk_capturing_the_supplied() {
    let (_, program) = super::program("pair = a b => (a, b)\npair --b 2");
    let body = main_body(&program);
    let closure = statements(body)
        .into_iter()
        .find_map(|statement| match statement {
            Statement::Assign(0, Rvalue::Closure { body, captures, .. }) => {
                Some((*body, captures.clone()))
            }
            _ => None,
        })
        .expect("the result is a closure");
    assert_eq!(closure.1.len(), 1, "the supplied b is captured");
    let thunk = &program.bodies[closure.0];
    assert_eq!(thunk.arity, 1, "a is the open spot");
    assert_eq!(thunk.captures, 1);
    assert!(statements(thunk)
        .contains(&&Statement::Assign(2, Rvalue::Use(Operand::Capture(0)))));
    let Terminator::Call {
        callee: Callee::Body(_),
        args,
        ..
    } = &thunk.blocks[0].terminator
    else {
        panic!("the thunk calls the body")
    };
    assert_eq!(args, &[Operand::Local(1), Operand::Local(2)]);
}

#[test]
fn element_supplies_collect_into_one_array() {
    let (_, program) =
        super::program("g: Vec<Integer> -> Integer = xs => len xs\ng 1 2 3");
    let body = main_body(&program);
    let arrays = statements(body)
        .into_iter()
        .filter(|statement| {
            matches!(statement, Statement::Assign(_, Rvalue::Array(parts)) if parts.len() == 3)
        })
        .count();
    assert_eq!(arrays, 1);
    assert!(program.constants.ends_with(&[
        Value::Int(1),
        Value::Int(2),
        Value::Int(3)
    ]));
}

#[test]
fn containers_hold_their_constants() {
    let (_, program) = super::program("[1, 2]");
    let body = main_body(&program);
    assert_eq!(
        body.blocks[0].statements[1],
        Statement::Assign(
            0,
            Rvalue::Array(vec![Operand::Const(0), Operand::Const(1)])
        )
    );
    let (_, program) = super::program("(1, \"a\")");
    let body = main_body(&program);
    assert_eq!(
        body.blocks[0].statements[1],
        Statement::Assign(
            0,
            Rvalue::Tuple(vec![Operand::Const(0), Operand::Const(1)])
        )
    );
    assert_eq!(program.constants[1], Value::Text("a".to_string().into()));
}

#[test]
fn a_conditional_switches_on_bool_without_otherwise() {
    let (_, program) = super::program("if 1 == 1 { 2 } else { 3 }");
    let body = main_body(&program);
    let switch = body
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Terminator::Switch {
                cases, otherwise, ..
            } => Some((cases.clone(), *otherwise)),
            _ => None,
        })
        .expect("a switch");
    assert_eq!(switch.0.len(), 2);
    assert_eq!(switch.1, None);
    let (then_block, else_block) = (switch.0[0].1, switch.0[1].1);
    assert_ne!(then_block, else_block);
    for block in [then_block, else_block] {
        assert!(matches!(body.blocks[block].terminator, Terminator::Goto(_)));
        assert!(statements(body).len() > 2);
        assert!(body.blocks[block]
            .statements
            .iter()
            .any(|statement| matches!(statement, Statement::Assign(0, _))));
    }
    assert!(
        call_block(body, |callee| matches!(callee, Callee::Native(_)))
            .is_some(),
        "eq is native"
    );
}

#[test]
fn with_pushes_then_pops_around_its_body() {
    let (_, program) = super::program("with cwd /tmp { 1 }");
    let body = main_body(&program);
    let statements = statements(body);
    let push = statements
        .iter()
        .position(|statement| matches!(statement, Statement::Push { .. }))
        .unwrap();
    let pop = statements
        .iter()
        .position(|statement| matches!(statement, Statement::Pop))
        .unwrap();
    let result = statements
        .iter()
        .position(|statement| matches!(statement, Statement::Assign(0, _)))
        .unwrap();
    assert!(push < result && result < pop);
    assert_eq!(program.tables.borrow().keys, ["cwd"]);
}

#[test]
fn a_block_binds_a_local_and_reads_it() {
    let (_, program) = super::program("{\n    x = 1\n    x\n}");
    let body = main_body(&program);
    assert_eq!(
        body.blocks[0].statements,
        [
            Statement::Charge(4),
            Statement::Assign(1, Rvalue::Use(Operand::Const(0))),
            Statement::Assign(0, Rvalue::Use(Operand::Local(1))),
        ]
    );
    assert!(
        body.locals[1].node.is_some(),
        "a named local keeps its binding"
    );
}

#[test]
fn a_timing_is_a_use_of_its_value() {
    let (_, program) = super::program("Compile 1");
    let body = main_body(&program);
    assert_eq!(
        body.blocks[0].statements,
        [
            Statement::Charge(2),
            Statement::Assign(0, Rvalue::Use(Operand::Const(0)))
        ]
    );
}

#[test]
fn a_value_definition_is_stored_and_read_as_a_global() {
    let (_, program) = super::program("k = 1\n(k, k)");
    let body = main_body(&program);
    assert!(statements(body).iter().any(|statement| matches!(
        statement,
        Statement::Store { definition: 0, .. }
    )));
    assert_eq!(program.definitions, [None]);
    assert!(statements(body).contains(&&Statement::Assign(
        0,
        Rvalue::Tuple(vec![Operand::Global(0), Operand::Global(0)])
    )));
}

#[test]
fn a_declaration_alone_is_a_thunk_of_its_arity() {
    let (_, program) = super::program("id = len\nid [1]");
    let thunk = program
        .bodies
        .iter()
        .find(|body| {
            matches!(
                body.blocks[0].terminator,
                Terminator::Call {
                    callee: Callee::Native(_),
                    ..
                }
            )
        })
        .expect("a thunk calling the native");
    assert_eq!(thunk.arity, 1);
    let body = main_body(&program);
    assert!(call_block(body, |callee| matches!(
        callee,
        Callee::Value(Operand::Global(0))
    ))
    .is_some());
}

#[test]
fn a_constructor_alone_constructs_in_a_thunk() {
    let (_, program) = super::program("Just");
    let thunk = program
        .bodies
        .iter()
        .find(|body| body.arity == 1)
        .expect("a thunk of one spot");
    assert!(matches!(
        thunk.blocks[0].statements[1],
        Statement::Assign(0, Rvalue::Construct { ref fields, .. }) if fields == &[Operand::Local(1)]
    ));
}

#[test]
fn a_closure_copies_its_captures() {
    let (_, program) = super::program("outer = x => (y => (x, y))\nouter 1");
    let outer = &program.bodies[program.definitions[0].unwrap()];
    let closure = statements(outer)
        .into_iter()
        .find_map(|statement| match statement {
            Statement::Assign(_, Rvalue::Closure { body, captures, .. }) => {
                Some((*body, captures.clone()))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(closure.1, [Operand::Local(1)], "x is copied from its spot");
    let inner = &program.bodies[closure.0];
    assert_eq!(inner.captures, 1);
    assert!(statements(inner).contains(&&Statement::Assign(
        0,
        Rvalue::Tuple(vec![Operand::Capture(0), Operand::Local(1)])
    )));
}

#[test]
fn prelude_literals_are_prepared_by_their_reader() {
    let (_, program) = super::program("2.5");
    assert_eq!(program.constants, [Value::F64(2.5)]);
    let (_, program) = super::program("x: F32 = 1.5\nx");
    assert!(program.constants.contains(&Value::F32(1.5)));
    let (_, program) = super::program("n = 9223372036854775808\nn");
    assert!(matches!(&program.constants[0], Value::Big(big) if !big.negative));
    let (_, program) = super::program("-9223372036854775808");
    assert_eq!(program.constants, [Value::Int(i64::MIN)]);
}

#[test]
fn a_literal_beyond_its_float_is_read_at_run_time() {
    for source in ["1e400", "x: F32 = 1e39\nx"] {
        let (_, program) = super::program(source);
        assert!(
            program.constants.iter().all(|constant| !matches!(
                constant,
                Value::F32(_) | Value::F64(_)
            )),
            "{source}: {:?}",
            program.constants
        );
        let read =
            program
                .bodies
                .iter()
                .flat_map(|body| &body.blocks)
                .any(|block| {
                    matches!(
                        &block.terminator,
                        Terminator::Call {
                            callee: Callee::Native(_),
                            args,
                            ..
                        } if matches!(args.as_slice(), [Operand::Const(_)])
                    )
                });
        assert!(read, "{source}");
    }
}

#[test]
fn a_field_reads_its_index() {
    let (_, program) = super::program(
        "p = Point 1 2\np.x\ntype Point { x: Integer, y: Integer }",
    );
    let body = main_body(&program);
    assert!(statements(body).contains(&&Statement::Assign(
        0,
        Rvalue::Field(Operand::Global(0), 0)
    )));
    assert!(statements(body).iter().any(|statement| matches!(statement, Statement::Assign(_, Rvalue::Construct { fields, .. }) if fields.len() == 2)));
}

#[test]
fn a_later_line_lowers_the_earlier_definition_it_calls() {
    let mut session = Session::new(Declarations::new());
    let Outcome::Done(first) = session.line("add2 = x => x + 2") else {
        panic!()
    };
    assert_eq!(first.diagnostics, []);
    let Outcome::Done(second) = session.line("add2 4") else {
        panic!()
    };
    assert_eq!(second.diagnostics, []);
    let program = compile::compile(second.unit.as_ref().unwrap());
    super::walk::check_program(&program);
    let body = main_body(&program);
    let Terminator::Call {
        callee: Callee::Body(index),
        ..
    } = &body.blocks[0].terminator
    else {
        panic!("the second line calls the first's body")
    };
    assert_eq!(
        program
            .definitions
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        [*index]
    );
    assert_eq!(program.bodies[*index].arity, 1);
}
