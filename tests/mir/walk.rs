use fipe::{
    ir,
    mir::{Callee, Operand, Program, Rvalue, Statement, Terminator},
};

/// Every index of a program in range, every block terminated, every
/// charge first, every path to `Return` assigning `_0`.
pub fn check_program(program: &Program) {
    let tables = program.tables.borrow();
    assert!(program.main < program.bodies.len());
    assert_eq!(program.definitions.len(), tables.definitions.len());
    for body in program.definitions.iter().flatten() {
        assert!(*body < program.bodies.len());
    }
    assert_eq!(program.tests.len(), tables.tests.len());
    for body in program.tests.iter().flatten() {
        assert!(*body < program.bodies.len());
        assert_eq!(program.bodies[*body].arity, 1, "a test takes its self");
    }
    assert_eq!(program.globals.len(), tables.definitions.len());
    for body in program.globals.iter().flatten() {
        assert!(*body < program.bodies.len());
        assert_eq!(program.bodies[*body].arity, 0, "a value has no spots");
    }
    for (module, body) in &program.inits {
        assert!(*module < tables.modules.len());
        assert!(*body < program.bodies.len());
    }
    for body in &program.bodies {
        assert!(body.node < tables.nodes.len());
        assert!(
            body.arity < body.locals.len(),
            "a body has _0 and its spots"
        );
        assert_ne!(body.blocks.len(), 0, "a body has blocks");
        for local in &body.locals {
            check_type(&tables, &local.ty);
            if let Some(node) = local.node {
                assert!(node < tables.nodes.len());
            }
        }
        for block in &body.blocks {
            assert_eq!(block.nodes.len(), block.statements.len() + 1);
            for node in &block.nodes {
                assert!(*node < tables.nodes.len());
            }
            let mut lowered = Vec::new();
            for (index, statement) in block.statements.iter().enumerate() {
                match statement {
                    Statement::Charge(nodes) => {
                        assert_eq!(index, 0, "a charge opens its block");
                        assert!(!nodes.is_empty(), "a charge is never empty");
                        for node in nodes {
                            assert!(*node < tables.nodes.len());
                        }
                    }
                    Statement::Assign(dest, rvalue) => {
                        assert!(*dest < body.locals.len());
                        check_rvalue(program, &tables, body, rvalue);
                        push(&mut lowered, block.nodes[index]);
                    }
                    Statement::Store { definition, value } => {
                        assert!(*definition < tables.definitions.len());
                        check_operand(program, body, value);
                        push(&mut lowered, block.nodes[index]);
                    }
                    Statement::Enter { entry, value } => {
                        assert!(*entry < tables.entries.len());
                        check_predicate(
                            &tables,
                            &tables.entries[*entry].predicate,
                        );
                        check_operand(program, body, value);
                        push(&mut lowered, block.nodes[index]);
                    }
                    Statement::Push { key, value } => {
                        assert!(*key < tables.keys.len());
                        check_operand(program, body, value);
                        push(&mut lowered, block.nodes[index]);
                    }
                    Statement::Pop => push(&mut lowered, block.nodes[index]),
                }
            }
            if let Some(Statement::Charge(nodes)) = block.statements.first() {
                assert!(
                    nodes.len() >= lowered.len(),
                    "a charge covers every node with a statement: body of \
                     node {} charges {nodes:?} for {lowered:?} in {block:?}",
                    body.node
                );
            }
            check_terminator(program, &tables, body, &block.terminator);
        }
        check_result_paths(body);
    }
}

fn push(lowered: &mut Vec<usize>, node: usize) {
    if !lowered.contains(&node) {
        lowered.push(node);
    }
}

fn check_operand(program: &Program, body: &fipe::mir::Body, operand: &Operand) {
    match operand {
        Operand::Local(index) => assert!(*index < body.locals.len()),
        Operand::Const(index) => assert!(*index < program.constants.len()),
        Operand::Capture(index) => assert!(*index < body.captures),
        Operand::Global(index) => {
            assert!(*index < program.tables.borrow().definitions.len());
        }
        Operand::Hidden(index) => assert!(*index < body.hidden.len()),
    }
}

fn check_operands(
    program: &Program,
    body: &fipe::mir::Body,
    operands: &[Operand],
) {
    for operand in operands {
        check_operand(program, body, operand);
    }
}

fn check_constructor(
    tables: &ir::Tables,
    meaning: usize,
    constructor: usize,
    fields: usize,
) {
    assert!(meaning < tables.meanings.len());
    let layout = &tables.meanings[meaning].constructors;
    assert!(constructor < layout.len());
    assert_eq!(layout[constructor].fields.len(), fields);
}

fn check_rvalue(
    program: &Program,
    tables: &ir::Tables,
    body: &fipe::mir::Body,
    rvalue: &Rvalue,
) {
    match rvalue {
        Rvalue::Use(value)
        | Rvalue::Field(value, _)
        | Rvalue::Length(value) => {
            check_operand(program, body, value);
        }
        Rvalue::Construct {
            meaning,
            constructor,
            fields,
        } => {
            check_constructor(tables, *meaning, *constructor, fields.len());
            check_operands(program, body, fields);
        }
        Rvalue::Tuple(parts) | Rvalue::Array(parts) => {
            check_operands(program, body, parts);
        }
        Rvalue::Repeat { value, length } => {
            check_operand(program, body, value);
            if let fipe::mir::Length::Hidden(index) = length {
                assert!(*index < body.hidden.len());
            }
        }
        Rvalue::Closure {
            body: target,
            captures,
            meanings,
        } => {
            assert!(*target < program.bodies.len());
            let callee = &program.bodies[*target];
            assert_eq!(captures.len(), callee.captures);
            assert_eq!(meanings.len(), callee.hidden.len());
            check_operands(program, body, captures);
            for ty in meanings {
                check_type(tables, ty);
            }
        }
        Rvalue::Wrap {
            meaning,
            constructor,
            value,
        } => {
            check_constructor(tables, *meaning, *constructor, 1);
            check_operand(program, body, value);
        }
        Rvalue::Cast {
            from, to, value, ..
        } => {
            check_type(tables, from);
            check_type(tables, to);
            check_operand(program, body, value);
        }
        Rvalue::Convert {
            predicate, value, ..
        } => {
            check_predicate(tables, predicate);
            check_operand(program, body, value);
        }
        Rvalue::Pack { tag, value } => {
            check_type(tables, tag);
            check_operand(program, body, value);
        }
        Rvalue::Native { op, args } => {
            assert!(*op < tables.declarations.len());
            check_operands(program, body, args);
        }
    }
}

fn check_terminator(
    program: &Program,
    tables: &ir::Tables,
    body: &fipe::mir::Body,
    terminator: &Terminator,
) {
    let block =
        |index: usize| assert!(index < body.blocks.len(), "block {index}");
    match terminator {
        Terminator::Goto(target) => block(*target),
        Terminator::Switch {
            value,
            cases,
            otherwise,
        } => {
            check_operand(program, body, value);
            assert!(!cases.is_empty() || otherwise.is_some());
            for (_, target) in cases {
                block(*target);
            }
            if let Some(target) = otherwise {
                block(*target);
            }
        }
        Terminator::Test {
            value, pass, fail, ..
        } => {
            check_operand(program, body, value);
            block(*pass);
            block(*fail);
        }
        Terminator::Call {
            callee,
            args,
            meanings,
            dest,
            target,
        } => {
            check_operands(program, body, args);
            assert!(*dest < body.locals.len());
            block(*target);
            for ty in meanings {
                check_type(tables, ty);
            }
            match callee {
                Callee::Body(index) => {
                    assert!(*index < program.bodies.len());
                    let target = &program.bodies[*index];
                    assert_eq!(
                        args.len(),
                        target.arity,
                        "a body takes its spots"
                    );
                    assert_eq!(meanings.len(), target.hidden.len());
                }
                Callee::Native(index) => {
                    assert!(*index < tables.declarations.len());
                    let namespace = tables.declarations[*index].namespace;
                    assert_eq!(tables.namespaces[namespace], "prelude");
                }
                Callee::Host(index) => {
                    assert!(*index < tables.declarations.len());
                    let namespace = tables.declarations[*index].namespace;
                    assert_ne!(tables.namespaces[namespace], "prelude");
                }
                Callee::Derived { ty, .. } => check_type(tables, ty),
                Callee::Dispatch { member, witness } => {
                    assert!(*member < tables.dispatch.len());
                    match witness {
                        fipe::mir::Witness::Value(value) => {
                            check_operand(program, body, value);
                        }
                        fipe::mir::Witness::Meaning(ty) => {
                            check_type(tables, ty);
                        }
                    }
                }
                Callee::Value(value) => check_operand(program, body, value),
            }
        }
        Terminator::Return | Terminator::Unreachable => (),
    }
}

/// Every path from the entry to a `Return` assigns `_0` on the way.
fn check_result_paths(body: &fipe::mir::Body) {
    let mut seen = vec![[false; 2]; body.blocks.len()];
    let mut stack = vec![(0, false)];
    while let Some((index, assigned)) = stack.pop() {
        if seen[index][usize::from(assigned)] {
            continue;
        }
        seen[index][usize::from(assigned)] = true;
        let block = &body.blocks[index];
        let assigned = assigned
            || block
                .statements
                .iter()
                .any(|statement| matches!(statement, Statement::Assign(0, _)));
        match &block.terminator {
            Terminator::Goto(target) => stack.push((*target, assigned)),
            Terminator::Switch {
                cases, otherwise, ..
            } => {
                for (_, target) in cases {
                    stack.push((*target, assigned));
                }
                if let Some(target) = otherwise {
                    stack.push((*target, assigned));
                }
            }
            Terminator::Test { pass, fail, .. } => {
                stack.push((*pass, assigned));
                stack.push((*fail, assigned));
            }
            Terminator::Call { dest, target, .. } => {
                stack.push((*target, assigned || *dest == 0));
            }
            Terminator::Return => {
                assert!(assigned, "block {index} returns without a result");
            }
            Terminator::Unreachable => (),
        }
    }
}

fn check_predicate(tables: &ir::Tables, predicate: &ir::Predicate) {
    match predicate {
        ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
            for part in parts {
                check_predicate(tables, part);
            }
        }
        ir::Predicate::Not(part) => check_predicate(tables, part),
        ir::Predicate::Test { index, meanings } => {
            assert!(*index < tables.tests.len());
            assert_eq!(meanings.len(), tables.tests[*index].parameters.len());
            for ty in meanings {
                check_type(tables, ty);
            }
        }
        ir::Predicate::Function { parameter, result } => {
            check_predicate(tables, parameter);
            check_predicate(tables, result);
        }
        ir::Predicate::Nonzero { .. } => (),
    }
}

fn check_type(tables: &ir::Tables, ty: &ir::Type) {
    match ty {
        ir::Type::Unit | ir::Type::Parameter(_) | ir::Type::Number(_) => (),
        ir::Type::Session(index) => {
            assert!(*index < tables.session_meanings.len());
        }
        ir::Type::Meaning { index, arguments } => {
            assert!(*index < tables.meanings.len());
            for ty in arguments {
                check_type(tables, ty);
            }
        }
        ir::Type::Tuple(parts)
        | ir::Type::Packed(parts)
        | ir::Type::All(parts)
        | ir::Type::Sum(parts) => {
            for ty in parts {
                check_type(tables, ty);
            }
        }
        ir::Type::Array { element, length } => {
            check_type(tables, element);
            check_type(tables, length);
        }
        ir::Type::Function { parameter, result } => {
            check_type(tables, parameter);
            check_type(tables, result);
        }
        ir::Type::Not(inner) => check_type(tables, inner),
    }
}

#[test]
fn straight_line_programs_walk() {
    for source in super::STRAIGHT_LINES {
        let (_, program) = super::program(source);
        check_program(&program);
    }
}

#[test]
fn arm_list_programs_walk() {
    for source in super::patterns::ARM_LISTS {
        let (_, program) = super::program(source);
        check_program(&program);
    }
}
