use super::run::{session, shown};

#[test]
fn a_written_eq_runs_as_a_body_by_the_value_tag() {
    let source = "type Mask: Eq { bits: Integer }\n\
                  impl Mask: Eq { eq = self other => (other.bits % 10) == (self.bits % 10) }\n\
                  ((Mask 1) == (Mask 11), (Mask 1) == (Mask 2))";
    assert_eq!(shown(source), "(True, False)");
}

#[test]
fn show_is_structural_or_the_written_member() {
    assert_eq!(
        shown("type Point: Show { x: Integer, y: Integer }\nshow (Point 1 2)"),
        "\"Point 1 2\""
    );
    // A written member runs its body, spelled as a member or reached
    // through the prelude's word, alone or inside a container.
    let written = "type Point: Show { x: Integer, y: Integer }\nimpl Point: Show { show = self => show (self.x * 10) }\np = Point 1 2\n";
    assert_eq!(shown(&format!("{written}p.show")), "\"10\"");
    assert_eq!(shown(&format!("{written}show p")), "\"10\"");
    assert_eq!(shown(&format!("{written}show [p, p]")), "\"[10, 10]\"");
    assert_eq!(shown("show (Just 2)"), "\"Just 2\"");
    assert_eq!(shown("show 1.5"), "\"1.5\"");
    assert_eq!(shown("show \"a\""), "\"\\\"a\\\"\"");
}

#[test]
fn conversions_check_their_limits() {
    assert_eq!(shown("1 | to<U8>"), "1");
    assert_eq!(shown("300 | try<U8>"), "Err LimitError");
    assert_eq!(shown("3 | try<U8>"), "Ok 3");
    assert_eq!(
        shown("f: Integer -> U8 = n => n | to<U8>\nf 300"),
        "fault: the cast failed"
    );
    assert_eq!(
        shown("f: Integer -> Result<U8, LimitError> = n => n | try<U8>\nf 300"),
        "Err LimitError"
    );
}

#[test]
fn a_derived_comparison_orders_by_constructor_then_fields() {
    assert_eq!(shown("(Just 1) < (Just 2)"), "True");
    assert_eq!(shown("(Just 1) < Nothing"), "True");
    assert_eq!(shown("(1, \"b\") < (1, \"a\")"), "False");
}

#[test]
fn an_impl_value_member_is_computed_on_its_first_read() {
    let source =
        "type P { x: Integer }\nimpl P { origin = Self 1 }\nP.origin.x";
    assert_eq!(shown(source), "1");
    let source = "type P { x: Integer }\nimpl P { origin = Self 2 }\n(P.origin.x, P.origin.x + 1)";
    assert_eq!(shown(source), "(2, 3)");
}

#[test]
fn a_member_of_a_packed_value_runs_its_tag_s_row_on_the_payload() {
    let shapes = "type Shape where self.area: F64\n\
                  type C: Shape { r: F64 }\n\
                  impl C: Shape { area = self => self.r * 2.0 }\n";
    assert_eq!(shown(&format!("{shapes}x: Shape = C 2.0\nx.area")), "4.0");
    assert_eq!(
        shown(&format!("{shapes}g: Shape -> F64 = s => s.area\ng (C 1.5)")),
        "3.0"
    );
    // A later line reaches the value through `Shape` alone.
    assert_eq!(
        session(&[
            "type Shape where self.area: F64",
            "type C: Shape { r: F64 }",
            "impl C: Shape { area = self => self.r }",
            "x: Shape = C 2.0",
            "x.area",
            "type Named: Shape where 0.0 < self.area",
            "n: Named = x",
            "n.area",
        ]),
        ["()", "()", "()", "()", "2.0", "()", "()", "2.0"]
    );
}

#[test]
fn packed_values_compare_tags_first_then_values() {
    let keys = "type Key: Eq & Show\n\
                type C: Key { n: Integer }\n\
                type D: Key { m: Integer }\n\
                impl C: Show { show = self => \"c\" ++ self.n.show }\n\
                k: Key = C 1\n\
                j: Key = C 1\n\
                l: Key = C 2\n\
                d: Key = D 1\n";
    assert_eq!(
        shown(&format!("{keys}(k == j, k == l, k == d, d == d, k != d)")),
        "(True, False, False, True, True)"
    );
    assert_eq!(
        shown(&format!("{keys}(k.show, [k, d] | show)")),
        "(\"c1\", \"[c1, D 1]\")"
    );
}

#[test]
fn len_and_read_run_a_written_claim_through_the_prelude_word() {
    let written = "type L: Len { n: I64 }\n\
                   impl L: Len { len = self => 99 }\n";
    assert_eq!(shown(&format!("{written}len (L 1)")), "99");
    assert_eq!(shown(&format!("{written}(L 1) | len")), "99");
    let read = "type Q: Read & Show { x: I64 }\n\
                impl Q: Read { read = s => Ok (Self 7) }\n";
    assert_eq!(shown(&format!("{read}\"zzz\" | read<Q>")), "Ok (Q 7)");
    // Inside a generic function the witness arrives at run time.
    let generic = "size = x => len x\nsize (L 1)";
    assert_eq!(shown(&format!("{written}{generic}")), "99");
}

#[test]
fn len_and_read_keep_their_built_in_rows() {
    assert_eq!(shown("len \"abc\""), "3");
    assert_eq!(shown("len [1, 2]"), "2");
    assert_eq!(shown("\"42\" | read<I64>"), "Ok 42");
    assert_eq!(shown("\"2.5\" | read<F64>"), "Ok 2.5");
    assert_eq!(shown("size = x => len x\nsize [1, 2, 3]"), "3");
}

#[test]
fn a_gate_constructor_on_an_open_value_faults_when_its_test_fails() {
    let source = "f: Integer -> U8 = x => U8 x\n(f 3, f 255)";
    assert_eq!(shown(source), "(3, 255)");
    let source = "f: Integer -> U8 = x => U8 x\nf 300";
    assert_eq!(shown(source), "fault: the cast failed");
}

#[test]
fn a_generic_bound_member_call_preserves_its_receiver() {
    let compare = "compare: Ord a => a -> a -> Ordering = a b => a.cmp b\n";
    assert_eq!(
        shown(&format!("{compare}(compare 1 2, compare 2 2, compare 3 2)")),
        "(Less, Equal, Greater)"
    );
    assert_eq!(
        shown("compare: Ord a => a -> a -> Bool = a b => a < b\ncompare 1 2"),
        "True"
    );
}

#[test]
fn a_generic_bound_member_can_be_aliased_grouped_or_returned() {
    for body in ["{ f = a.cmp\nf b }", "(a.cmp) b"] {
        assert_eq!(
            shown(&format!(
                "compare: Ord a => a -> a -> Ordering = a b => {body}\ncompare 1 2"
            )),
            "Less"
        );
    }
    assert_eq!(
        shown(
            "member: Ord a => a -> (a -> Ordering) = a => a.cmp\nf = member 1\nf 2"
        ),
        "Less"
    );
}

#[test]
fn a_bound_member_captures_an_effectful_receiver_before_its_argument() {
    use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
    use fipe::{
        compile,
        embed::{Reply, Request},
        value::structural,
    };
    let mut declarations = Declarations::new();
    declarations
        .commands("pub receiver_value: () -> Integer\npub argument_value: () -> Integer")
        .unwrap();
    let module = parse(lex("compare: Ord a => (() -> a) -> (() -> a) -> Ordering = f g => (f ()).cmp (g ())\ncompare receiver_value argument_value")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(super::run::BUDGET));
    let mut outcome = machine.run(&program);
    for (label, answer) in [("receiver_value", 1), ("argument_value", 2)] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("expected {label}: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            label
        );
        assert_eq!(arguments, [Value::Unit]);
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Int(answer))));
    }
    let Outcome::Done(Ok(value)) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(structural::show(&value, &program.tables.borrow()), "Less");
}

#[test]
fn bound_member_calls_preserve_each_written_receiver_position() {
    for (receiver_spot, parameters) in
        ["self other", "other self"].into_iter().enumerate()
    {
        let source = format!(
            "type P: Ord {{ value: Integer }}\nimpl P: Ord {{ cmp = {parameters} => if self.value < other.value {{ Less }} else {{ Greater }} }}\ncompare: Ord a => a -> a -> Ordering = a b => a.cmp b\ncompare (P 1) (P 2)"
        );
        let program = super::run::program(&source);
        let tables = program.tables.borrow();
        assert!(tables.dispatch.iter().filter(|dispatch| dispatch.label == "cmp").flat_map(|dispatch| &dispatch.rows).any(|row| matches!(&row.receiver, fipe::ir::Type::Meaning {index, ..} if tables.meanings[*index].label == "P") && row.receiver_spot == Some(receiver_spot)));
        assert_eq!(shown(&source), "Less", "receiver parameters {parameters}");
    }
}

#[test]
fn one_generic_member_function_aligns_different_runtime_rows() {
    let types = "type P: Ord { value: Integer }\nimpl P: Ord { cmp = self other => if self.value < other.value { Less } else { Greater } }\ntype Q: Ord { value: Integer }\nimpl Q: Ord { cmp = other self => if self.value < other.value { Less } else { Greater } }\ncompare: Ord a => a -> a -> Ordering = a b => a.cmp b\n";
    for results in [
        "(compare (P 1) (P 2), compare (Q 1) (Q 2), (P 1).cmp (P 2), (Q 1).cmp (Q 2))",
        "(compare (Q 1) (Q 2), compare (P 1) (P 2), (Q 1).cmp (Q 2), (P 1).cmp (P 2))",
    ] {
        assert_eq!(
            shown(&format!("{types}{results}")),
            "(Less, Less, Less, Less)"
        );
    }
}

#[test]
fn a_declared_member_receives_ordered_arguments_before_its_receiver() {
    use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
    use fipe::{
        compile,
        embed::{Reply, Request},
    };
    let self_spot = 2;
    let signature = "Integer -> Integer -> (self: Self) -> Integer";
    let mut declarations = Declarations::new();
    declarations.module("csv", &format!("pub type Measure where self.mix: Integer -> Integer -> Integer\npub type P: Measure {{ pub value: Integer }}\nimpl P: Measure {{ mix: {signature} }}")).unwrap();
    let module = parse(lex("use csv\ninvoke: csv.Measure a => a -> Integer -> Integer -> Integer = p x y => p.mix x y\ninvoke (csv.P 7) 10 20")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(super::run::BUDGET));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("expected declared member")
    };
    let tables = program.tables.borrow();
    assert_eq!(tables.declarations[declaration].label, "mix");
    assert_eq!(arguments.len(), 3);
    let Value::Record(receiver) = &arguments[self_spot] else {
        panic!("receiver at {self_spot}")
    };
    assert_eq!(tables.meanings[receiver.meaning].label, "P");
    assert_eq!(receiver.fields, [Value::Int(7)]);
    let others: Vec<_> = arguments
        .iter()
        .enumerate()
        .filter(|(spot, _)| *spot != self_spot)
        .map(|(_, value)| value.clone())
        .collect();
    assert_eq!(others, [Value::Int(10), Value::Int(20)]);
    drop(tables);
    assert_eq!(
        machine.reply(&program, Reply::Value(Ok(Value::Int(99)))),
        Outcome::Done(Ok(Value::Int(99)))
    );
}

#[test]
fn receiverless_conversion_adapters_keep_their_declared_argument_order() {
    assert_eq!(shown("type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | try<Box>"), "Ok (Box 1)");
    assert_eq!(shown("type Box: TryFrom<Integer, String> { value: Integer }\nimpl Box: TryFrom<Integer, String> { try_from = n => Ok (Self n) }\n1 | to<Box>"), "Box 1");
}

const PACK_HIDDEN_LIBRARY: &str = "type Phantom<T>: Eq {}\npack: Phantom<a> -> Eq = p => p\nignore: Phantom<a> -> Integer = _ => 1";

fn pack_hidden_programs(
    route: usize,
    reverse: bool,
) -> Vec<fipe::mir::Program> {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse,
    };
    let mut declarations = Declarations::new();
    declarations.module("csv", "pub echo: Eq -> Eq").unwrap();
    let prefix = if route == 1 { "foreign." } else { "" };
    let calls = if reverse {
        "right), csv.echo (Pleft"
    } else {
        "left), csv.echo (Pright"
    };
    let calls = calls.replace('P', &format!("{prefix}pack "));
    let body = format!("use csv\nleft: {prefix}Phantom<Integer> = {prefix}Phantom {{}}\nright: {prefix}Phantom<String> = {prefix}Phantom {{}}\n(csv.echo ({prefix}pack {calls}), {prefix}ignore left)");
    if route == 2 {
        let mut session = fipe::session::Session::new(declarations);
        let mut programs = Vec::new();
        for source in [PACK_HIDDEN_LIBRARY, &body] {
            let Outcome::Done(report) = session.line(source) else {
                panic!("no file")
            };
            assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
            programs
                .push(compile::compile(&report.unit.expect("accepted unit")));
        }
        return programs;
    }
    let (source, files) = if route == 1 {
        let library = PACK_HIDDEN_LIBRARY
            .replace("type Phantom", "pub type Phantom")
            .replace("\npack:", "\npub pack:")
            .replace("\nignore:", "\npub ignore:");
        (
            format!("use foreign\n{body}"),
            vec![(String::from("foreign"), parse(lex(&library)).unwrap())],
        )
    } else {
        (format!("{PACK_HIDDEN_LIBRARY}\n{body}"), Vec::new())
    };
    let module = parse(lex(&source)).unwrap();
    vec![compile::compile(
        &check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .unwrap(),
    )]
}

fn pack_hidden_observe(programs: &[fipe::mir::Program], reverse: bool) {
    use fipe::{
        embed::{Outcome, Reply, Request},
        ir, mir,
        value::Value,
        vm::{Frame, Machine, Stop},
    };
    let mut machine = Machine::new(Some(super::run::BUDGET));
    for program in &programs[..programs.len() - 1] {
        assert!(matches!(machine.run(program), Outcome::Done(Ok(_))));
    }
    let program = programs.last().unwrap();
    machine.begin(program);
    let mut labels = Vec::new();
    let mut echoes = Vec::new();
    loop {
        match machine.next_stop() {
            Stop::Before(_) => {
                let Some(Frame::Body(frame)) = machine.frames().last() else {
                    continue;
                };
                let joined = machine.program().unwrap();
                let body = &joined.bodies[frame.body];
                let Some(mir::Statement::Assign(
                    _,
                    mir::Rvalue::Pack {
                        tag: ir::Type::Meaning { arguments, .. },
                        ..
                    },
                )) = body.blocks[frame.block].statements.get(frame.statement)
                else {
                    continue;
                };
                let [ir::Type::Parameter(parameter)] = arguments.as_slice()
                else {
                    panic!("actual generic Pack: {arguments:?}")
                };
                let position =
                    body.hidden.iter().position(|p| p == parameter).expect(
                        "Pack tag parameter must have a runtime witness",
                    );
                let witness = &frame.meanings[position];
                let ir::Type::Meaning { index, arguments } = witness else {
                    panic!("concrete witness {witness:?}")
                };
                assert!(arguments.is_empty());
                labels.push(
                    joined.tables.borrow().meanings[*index].label.clone(),
                );
            }
            Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) => {
                assert_eq!(
                    machine.program().unwrap().tables.borrow().declarations
                        [declaration]
                        .label,
                    "echo"
                );
                assert_eq!(arguments.len(), 1);
                assert!(matches!(arguments[0], Value::Packed(_)));
                echoes.push(arguments[0].clone());
                machine.answer(Reply::Value(Ok(arguments[0].clone())));
            }
            Stop::Request(Request::File { .. }) => {
                panic!("files resolved before execution")
            }
            Stop::Done(result) => {
                let expected = if reverse {
                    ["String", "Integer"]
                } else {
                    ["Integer", "String"]
                };
                assert_eq!(labels, expected);
                assert_eq!(echoes.len(), 2);
                echoes.push(Value::Int(1));
                assert_eq!(result.unwrap(), Value::Tuple(echoes.into()));
                break;
            }
        }
    }
}

#[test]
fn pack_hidden_supplies_each_concrete_phantom_witness_through_references() {
    for route in 0..3 {
        for reverse in [false, true] {
            pack_hidden_observe(&pack_hidden_programs(route, reverse), reverse);
        }
    }
}

fn written_packed_wrapper_shown(
    header: &str,
    source: &str,
    member: &str,
) -> String {
    use super::{check, lex, parse, Declarations, Machine, Outcome};
    use fipe::{
        compile,
        ir::{Implementation, Type},
        value::structural,
    };
    let mut declarations = Declarations::new();
    declarations.module("probe", header).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    {
        let tables = program.tables.borrow();
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        assert!(tables.dispatch.iter().filter(|dispatch| dispatch.label == member).flat_map(|dispatch| &dispatch.rows).any(|row| {
            matches!(row.receiver, Type::Meaning { index, .. } if index == key)
                && matches!(row.target, Implementation::Definition { .. })
        }), "the wrapper must own an executable written {member} body");
    }
    let mut machine = Machine::new(Some(super::run::BUDGET));
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("written wrapper body must complete without a Host callback")
    };
    let shown = structural::show(&value, &program.tables.borrow());
    shown
}

#[test]
fn packed_written_wrapper_ord_precedes_payload_order() {
    let header = "pub type Key: Ord\nimpl Key: Ord { eq: Self -> (self: Self) -> Bool = other self => False, cmp: Self -> (self: Self) -> Ordering = other self => Greater }";
    let source = "use probe\ntype P: probe.Key { n: Integer }\na: probe.Key = P 1\nb: probe.Key = P 2\na < b";
    assert_eq!(written_packed_wrapper_shown(header, source, "cmp"), "False");
}

#[test]
fn packed_nested_widening_keeps_each_actual_layer_and_member() {
    use super::{Machine, Outcome, Value};
    use fipe::ir::Type;
    let source = "type Shape where self.area: Integer\ntype Child: Shape & Show\ntype P: Child { n: Integer }\nimpl P: Shape { area = self => self.n }\nimpl P: Show { show = self => self.n.show }\nx: Child = P 7\ny: Shape = x\n(x, y, y.area)";
    let program = super::run::program(source);
    let mut machine = Machine::new(Some(super::run::BUDGET));
    let Outcome::Done(Ok(Value::Tuple(parts))) = machine.run(&program) else {
        panic!("nested widening")
    };
    assert_eq!(parts[2], Value::Int(7));
    let (Value::Packed(child), Value::Packed(shape)) = (&parts[0], &parts[1])
    else {
        panic!("both actual wrappers")
    };
    let label = |ty: &Type| {
        let Type::Meaning { index, .. } = ty else {
            panic!("nominal source")
        };
        program.tables.borrow().meanings[*index].label.clone()
    };
    assert_eq!(label(child.source_type()), "P");
    assert_eq!(label(child.target_type()), "Child");
    assert_eq!(label(shape.source_type()), "Child");
    assert_eq!(label(shape.target_type()), "Shape");
    let Value::Packed(retained) = shape.value() else {
        panic!("outer widening retains inner Child")
    };
    assert!(std::rc::Rc::ptr_eq(retained, child));
}

#[test]
fn packed_nested_written_wrapper_eq_precedes_payload_inequality() {
    let header = "pub type Key: Eq\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool = other self => True }";
    let source = "use probe\ntype P: probe.Key { n: Integer }\nx: probe.Key = P 1\ny: probe.Key = P 2\n(x, 0) == (y, 0)";
    assert_eq!(written_packed_wrapper_shown(header, source, "eq"), "True");
}

#[test]
fn packed_nested_written_wrapper_show_preserves_argument_parentheses() {
    let header = "pub type Key: Show\nimpl Key: Show { show: (self: Self) -> String = self => \"key word\" }";
    let source = "use probe\ntype P: probe.Key { n: Integer }\ntype Holder: Show { item: probe.Key }\nx: probe.Key = P 7\nshow (x, Holder x)";
    assert_eq!(
        written_packed_wrapper_shown(header, source, "show"),
        "\"(key word, Holder (key word))\""
    );
}

#[test]
fn field_dispatch_receiver_keeps_generic_and_anonymous_accessors() {
    for (source, expected) in [
        ("type Record { n: Integer }\nget = r => r.n\na = get (Record 4)\na + 1", "5"),
        ("type R { name: String, size: Integer }\nxs = [R \"a\" 2, R \"b\" 1]\n(xs | map .name, xs | sort -by .size | map .name)", "([\"a\", \"b\"], [\"b\", \"a\"])") ,
        ("type F { op: Integer -> Integer }\nf = F (x => x + 1)\nget = r => r.op\n(get f) 7", "8"),
    ] {
        assert_eq!(shown(source), expected, "{source}");
    }
}

#[test]
fn field_dispatch_receiver_keeps_function_field_and_written_member_order() {
    let source = "type F { op: Integer -> Integer }\ntype M { n: Integer }\nimpl M { op = delta self => self.n + delta }\ncall = x => x.op 7\n(call (F (x => x + 1)), call (M 2))";
    assert_eq!(shown(source), "(8, 9)");
}

#[test]
fn field_dispatch_receiver_applies_multiple_and_partial_arguments() {
    let source = "type F { op: Integer -> Integer -> Integer }\ntype M { n: Integer }\nimpl M { op = a b self => self.n + a * 10 + b }\ncall = x => x.op 3 4\npartial = x => x.op 3\nf = F (a b => a * 10 + b)\n(call f, call (M 2), (partial f) 4, (partial (M 2)) 4)";
    assert_eq!(shown(source), "(34, 36, 34, 36)");
}

#[test]
fn field_dispatch_receiver_runs_supplies_once_and_preserves_body_fault() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut host = Declarations::new();
    host.module("host", "pub receiver: () -> Integer\npub a: () -> Integer\npub b: () -> Integer\npub body: Integer -> Integer -> Integer\npub after: Integer -> Integer").unwrap();
    let source = "use host\ntype F { op: Integer -> Integer -> Integer }\nmake = _ => { ignored = host.receiver ()\nF (a b => host.body a b) }\ncall = x => x.op (host.a ()) (host.b ())\nhost.after (call (make ()))";
    let module = parse(lex(source)).unwrap();
    let checked =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&checked);
    for fault in [false, true] {
        let mut machine = Machine::new(None);
        let mut outcome = machine.run(&program);
        for (label, arguments, reply) in [
            ("receiver", vec![Value::Unit], Ok(Value::Int(99))),
            ("a", vec![Value::Unit], Ok(Value::Int(3))),
            ("b", vec![Value::Unit], Ok(Value::Int(4))),
            (
                "body",
                vec![Value::Int(3), Value::Int(4)],
                if fault {
                    Err("body denied".into())
                } else {
                    Ok(Value::Int(34))
                },
            ),
        ] {
            let Outcome::Request(Request::Call {
                declaration,
                arguments: actual,
                ..
            }) = outcome
            else {
                panic!("expected {label}: {outcome:?}")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                label
            );
            assert_eq!(actual, arguments);
            outcome = machine.reply(&program, Reply::Value(reply));
        }
        if fault {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: Reason::Host(ref message), .. })) if message == "body denied")
            );
        } else {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments, vec![Value::Int(34)]);
            assert!(matches!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(34)))),
                Outcome::Done(Ok(Value::Int(34)))
            ));
        }
    }
}

#[test]
fn receiver_adapters_preserve_static_prefix_and_self_positions() {
    let prefix = "type M { n: Integer }\nimpl M { first = self a b => self.n + a * 10 + b }\nimpl M { middle = a self b => self.n + a * 10 + b }\nimpl M { last = a b self => self.n + a * 10 + b }\n";
    for tail in [
        "p = M.first (M 2)\np 3 4",
        "p = M.middle 3\np (M 2) 4",
        "p = M.last 3\np 4 (M 2)",
        "get = m => m.last\np = get (M 2)\np 3 4",
    ] {
        assert_eq!(shown(&format!("{prefix}{tail}")), "36", "{tail}");
    }
}

#[test]
fn receiver_adapters_keep_declared_function_result_out_of_formal_arity() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type M { pub n: Integer }\npub save: (Integer -> Integer) -> ()\nimpl M { op: (self: Self) -> a }").unwrap();
    let source = "use host\nhost.save (n => n + 1)\ngetInt = m => { x: Integer = m.op\nx }\ngetFunction = m => { x: Integer -> Integer = m.op\nx }\nx = host.M 7\nfirst = getInt x\nsecond = getFunction x\nsecond 3";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    assert!(
        !program.receiver_adapters.is_empty(),
        "exercise actual dynamic declared adapters"
    );
    let rows = &program.tables.borrow().dispatch;
    assert!(rows
        .iter()
        .filter(|member| member.label == "op")
        .flat_map(|member| &member.rows)
        .all(|row| row.declared_arity == Some(1)));
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("save")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "save"
    );
    let [function @ Value::Function(_)] = arguments.as_slice() else {
        panic!("function")
    };
    let function = function.clone();
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(Value::Unit)))
    else {
        panic!("first op")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "op"
    );
    assert_eq!(arguments.len(), 1);
    let receiver = arguments[0].clone();
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(Value::Int(10))))
    else {
        panic!("second op")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "op"
    );
    assert_eq!(arguments, [receiver]);
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(function))),
        Outcome::Done(Ok(Value::Int(4)))
    ));
    let mut bad = Machine::new(Some(100_000));
    assert!(matches!(bad.run(&program), Outcome::Request(_)));
    assert!(matches!(
        bad.reply(&program, Reply::Value(Ok(Value::Unit))),
        Outcome::Request(_)
    ));
    let refused = bad.reply(&program, Reply::Value(Ok(Value::text("wrong"))));
    assert!(
        matches!(
            refused,
            Outcome::Done(Err(fipe::vm::Fault {
                reason: fipe::value::Reason::Host(_),
                ..
            }))
        ),
        "{refused:?}"
    );
}

#[test]
fn receiver_adapters_run_self_only_body_before_returned_function_is_called() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub tick: () -> ()\npub after: () -> ()")
        .unwrap();
    let source = "use host\ntype M { n: Integer }\nimpl M { op = self => { host.tick ()\nx => self.n + x } }\nget = m => m.op\nf = get (M 7)\nhost.after ()\nf 3";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    for expected in ["tick", "after"] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("{expected}: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            expected
        );
        assert_eq!(arguments, [Value::Unit]);
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Unit)));
    }
    assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(10)))));
}

#[test]
fn receiver_adapters_preserve_direct_definition_concrete_result() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a").unwrap();
    let source = "use host\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => host.get () }\nw: W<Integer> = W 1\nw.op";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    assert!(
        program.receiver_adapters.is_empty(),
        "concrete member resolves directly to a definition"
    );
    for reply in [Value::Int(7), Value::text("wrong")] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, [Value::Unit]);
        let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        if reply == Value::Int(7) {
            assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(7)))));
        } else {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn receiver_adapters_keep_genuine_resource_prefix_and_generic_protocol() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{resource::ResourceDomain, Reason, Value},
        vm::Machine,
    };
    use std::rc::Rc;
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type resource Rd<T>\npub open: Rd<Integer>\npub after: () -> ()\nimpl Rd<a> { op: Integer -> Integer -> (self: Self) -> a }").unwrap();
    let source =
        "use host\nget = r => r.op\nf = get host.open\nhost.after ()\nf 3 4";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let adapter = program
        .receiver_adapters
        .iter()
        .find(|adapter| {
            program.tables.borrow().dispatch[adapter.member].label == "op"
        })
        .unwrap();
    assert!(matches!(
        adapter.target,
        fipe::mir::ReceiverTarget::Declared(_)
    ));
    assert_eq!(program.bodies[adapter.body].arity, 3);
    assert_eq!(adapter.receiver_spot, 2);
    assert_eq!(
        program.tables.borrow().dispatch[adapter.member].rows[adapter.row]
            .declared_arity,
        Some(3)
    );
    for reply in [Value::Int(7), Value::text("wrong")] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call { declaration, .. }) =
            machine.run(&program)
        else {
            panic!("open")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "open"
        );
        let alias = ResourceDomain::new().issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"entry"[..]),
        );
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.reply(&program, Reply::Value(Ok(alias.clone())))
        else {
            panic!("after")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "after"
        );
        assert_eq!(arguments, [Value::Unit]);
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.reply(&program, Reply::Value(Ok(Value::Unit)))
        else {
            panic!("op")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "op"
        );
        assert_eq!(arguments, [Value::Int(3), Value::Int(4), alias.clone()]);
        let (Value::Resource(expected), Value::Resource(actual)) =
            (&alias, &arguments[2])
        else {
            panic!("genuine alias")
        };
        assert!(Rc::ptr_eq(expected, actual));
        let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        if reply == Value::Int(7) {
            assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(7)))));
        } else {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn bound_member_generic_owner_reply_is_checked() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a").unwrap();
    let source="use host\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => host.get () }\nget = w => w.op\nget (W 1)";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    for reply in [Value::Int(7), Value::text("wrong")] {
        let mut machine = Machine::new(Some(100_000));
        let outcome = machine.run(&program);
        assert!(matches!(outcome, Outcome::Request(Request::Call { .. })));
        let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        if reply == Value::Int(7) {
            assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(7)))));
        } else {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn bound_member_owner_witness_reaches_host_before_later_effects() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::{Frame, Machine},
    };
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub get: () -> a\npub after: a -> Integer")
        .unwrap();
    let source="use host\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => host.get () }\nget = w => w.op\nx = get (W 1)\nhost.after x";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let integer = program
        .tables
        .borrow()
        .meanings
        .iter()
        .position(|meaning| meaning.label == "Integer")
        .unwrap();
    for wrong in [false, true] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("expected Host get")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, [Value::Unit]);
        let active = machine.program().unwrap();
        let raw = active
            .tables
            .borrow()
            .definitions
            .iter()
            .find(|definition| definition.label == "op")
            .unwrap()
            .body;
        assert!(machine.frames().iter().any(|frame|matches!(frame,Frame::Body(frame) if active.bodies[frame.body].node==raw && frame.meanings==[fipe::ir::Type::Meaning{index:integer,arguments:vec![]}])));
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        } else {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("expected after")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments, [Value::Int(7)]);
            assert!(matches!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(77)))),
                Outcome::Done(Ok(Value::Int(77)))
            ));
        }
    }
}

#[test]
fn bound_member_dynamic_function_field_and_reversed_owner_types_work() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    assert_eq!(
        shown(
            "type F<T> { op: () -> T }\nget = w => w.op\nget (F (_ => 7)) ()"
        ),
        "7"
    );
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a").unwrap();
    for (calls, replies) in [
        (
            "(get (W 1), get (W \"x\"))",
            vec![Value::Int(7), Value::text("ok")],
        ),
        (
            "(get (W \"x\"), get (W 1))",
            vec![Value::text("ok"), Value::Int(7)],
        ),
    ] {
        let source=format!("use host\ntype W<T> {{ value: T }}\nimpl W<a> {{ op: Self -> a = self => host.get () }}\nget = w => w.op\n{calls}");
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        let mut machine = Machine::new(Some(100_000));
        let mut outcome = machine.run(&program);
        for reply in &replies {
            assert!(matches!(outcome, Outcome::Request(Request::Call { .. })));
            outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        }
        assert!(
            matches!(outcome,Outcome::Done(Ok(Value::Tuple(actual))) if *actual==replies)
        );
    }
}

#[test]
fn independent_bound_member_result_must_be_integer() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub type M { pub n: Integer }\nimpl M { op: (self: Self) -> a }",
        )
        .unwrap();
    let source = "use host\nget = m => m.op\nx: Integer = get (host.M 7)\nx";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    for reply in [Value::Int(7), Value::text("wrong")] {
        let mut machine = Machine::new(Some(100_000));
        let request = machine.run(&program);
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = request
        else {
            panic!("expected op")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "op"
        );
        assert_eq!(arguments.len(), 1);
        assert!(
            matches!(&arguments[0],Value::Record(record) if record.fields==[Value::Int(7)])
        );
        let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        if reply == Value::Int(7) {
            assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(7)))));
        } else {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn independent_result_survives_supplied_grouped_and_partial_member_reads() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host","pub type M { pub n: Integer }\nimpl M { op: Integer -> Integer -> (self: Self) -> a }").unwrap();
    for ending in [
        "x: Integer = get (host.M 7) 3 4\nx",
        "x: Integer = (get (host.M 7)) 3 4\nx",
        "f = get (host.M 7)\ng = f 3\nx: Integer = g 4\nx",
    ] {
        let source = format!("use host\nget = m => m.op\n{ending}");
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        for wrong in [false, true] {
            let mut machine = Machine::new(Some(100_000));
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = machine.run(&program)
            else {
                panic!("expected op")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "op"
            );
            assert_eq!(&arguments[..2], &[Value::Int(3), Value::Int(4)]);
            assert!(
                matches!(&arguments[2],Value::Record(record) if record.fields==[Value::Int(7)])
            );
            let outcome = machine.reply(
                &program,
                Reply::Value(Ok(if wrong {
                    Value::text("wrong")
                } else {
                    Value::Int(9)
                })),
            );
            if wrong {
                assert!(
                    matches!(
                        outcome,
                        Outcome::Done(Err(fipe::vm::Fault {
                            reason: Reason::Host(_),
                            ..
                        }))
                    ),
                    "{ending}: {outcome:?}"
                );
            } else {
                assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(9)))));
            }
        }
    }
}

#[test]
fn adapter_control_concat_protocol_returns_function() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut host = Declarations::new();
    host.module("host", "pub type M { pub n: Integer }\npub save: (String -> Integer) -> ()\nimpl M { concat: (self: Self) -> a }").unwrap();
    let source="use host\nhost.save (s => 9)\ntext: String = \"x\"\nanswer: Integer = (host.M 7) ++ text\nanswer";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let first = machine.run(&program);
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = first
    else {
        panic!("save")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "save"
    );
    let [function @ Value::Function(_)] = arguments.as_slice() else {
        panic!("function")
    };
    let function = function.clone();
    let second = machine.reply(&program, Reply::Value(Ok(Value::Unit)));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = second
    else {
        panic!("concat")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "concat"
    );
    assert_eq!(arguments.len(), 1, "self-only protocol");
    let finish = machine.reply(&program, Reply::Value(Ok(function)));
    assert!(matches!(finish, Outcome::Done(Ok(Value::Int(9)))));
}

fn declared_member_order_case(generic: bool) {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut host = Declarations::new();
    host.module("host","pub type M { pub n: Integer }\npub save: (String -> Integer) -> ()\npub text: () -> String\nimpl M { concat: (self: Self) -> a }").unwrap();
    let tail = if generic {
        "cat = lhs => lhs ++ (host.text ())\nanswer: Integer = cat (host.M 7)\nanswer"
    } else {
        "answer: Integer = (host.M 7) ++ (host.text ())\nanswer"
    };
    let source = format!("use host\nhost.save (s => 9)\n{tail}");
    let module = parse(lex(&source)).unwrap();
    let unit =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    let mut saved = None;
    let mut order = Vec::new();
    for _ in 0..5 {
        match outcome {
            Outcome::Done(_) => break,
            Outcome::Request(Request::Call {
                declaration,
                ref arguments,
                ..
            }) => {
                let label = program.tables.borrow().declarations[declaration]
                    .label
                    .clone();
                order.push(label.clone());
                let reply = match label.as_str() {
                    "save" => {
                        let [function @ Value::Function(_)] =
                            arguments.as_slice()
                        else {
                            panic!("saveFn")
                        };
                        saved = Some(function.clone());
                        Value::Unit
                    }
                    "text" => {
                        assert_eq!(arguments.as_slice(), [Value::Unit]);
                        Value::text("x")
                    }
                    "concat" => {
                        assert_eq!(arguments.len(), 1);
                        assert!(
                            matches!(&arguments[0],Value::Record(record)if record.fields==[Value::Int(7)])
                        );
                        saved.clone().unwrap()
                    }
                    _ => panic!("unexpected {label}"),
                };
                outcome = machine.reply(&program, Reply::Value(Ok(reply)));
            }
            Outcome::Request(_) => panic!("unexpectedFile"),
        }
    }
    assert_eq!(
        order,
        ["save", "concat", "text"],
        "callee must execute before returned-function rhs"
    );
    assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(9)))));
}
#[test]
fn effectful_rhs_order_direct() {
    declared_member_order_case(false);
}
#[test]
fn effectful_rhs_order_generic() {
    declared_member_order_case(true);
}

#[test]
fn declared_function_reply_failure_precedes_rhs_for_direct_and_generic_calls() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut host = Declarations::new();
    host.module("host","pub type M { pub n: Integer }\npub text: () -> String\nimpl M { concat: (self: Self) -> a }").unwrap();
    for ending in ["answer: Integer = (host.M 7) ++ (host.text ())\nanswer","cat = lhs => lhs ++ (host.text ())\nanswer: Integer = cat (host.M 7)\nanswer"] {
        let source=format!("use host\n{ending}");let module=parse(lex(&source)).unwrap();let unit=check::check(&module,&host,&[],check::Options::default()).unwrap();let program=compile::compile(&unit);let mut machine=Machine::new(Some(100_000));
        let Outcome::Request(Request::Call{declaration,arguments,..})=machine.run(&program)else{panic!("concat")};assert_eq!(program.tables.borrow().declarations[declaration].label,"concat");assert_eq!(arguments.len(),1);
        let result=machine.reply(&program,Reply::Value(Ok(Value::Int(9))));assert!(matches!(result,Outcome::Done(Err(fipe::vm::Fault{reason:Reason::Host(_),..}))),"{ending}: {result:?}");
    }
}

#[test]
fn a_two_formal_generic_operator_waits_for_rhs_before_host_call() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut host = Declarations::new();
    host.module("host","pub type M { pub n: Integer }\npub text: () -> String\nimpl M { concat: String -> (self: Self) -> Integer }").unwrap();
    let source = "use host\ncat = lhs => lhs ++ (host.text ())\ncat (host.M 7)";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("text")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "text"
    );
    assert_eq!(arguments, [Value::Unit]);
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(Value::text("x"))))
    else {
        panic!("concat")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "concat"
    );
    assert_eq!(arguments[0], Value::text("x"));
    assert!(
        matches!(&arguments[1],Value::Record(record)if record.fields==[Value::Int(7)])
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Int(9)))),
        Outcome::Done(Ok(Value::Int(9)))
    ));
}

#[test]
fn receiver_adapter_resource_free_callable_copy_spends_budget_before_host() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type M<T> { pub n: Integer }\nimpl M<a> { op: (self: Self) -> Integer }").unwrap();
    let program = |depth: usize| {
        let mut phantom = "()".to_owned();
        for _ in 0..depth {
            phantom = format!("Vec<{phantom}>");
        }
        let source = format!(
            "use host\nget = m => m.op\nx: host.M<{phantom}> = host.M 7\nget x"
        );
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        assert!(!program.tables.borrow().has_resources);
        assert!(program
            .receiver_contexts
            .iter()
            .flatten()
            .any(|context| context.protocol_result.is_some()));
        program
    };
    let shallow = program(0);
    let deep = program(32);
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call { declaration, .. }) =
        machine.run(&shallow)
    else {
        panic!("shallow op")
    };
    assert_eq!(
        shallow.tables.borrow().declarations[declaration].label,
        "op"
    );
    let shallow_cost = machine.charged();
    assert!(matches!(
        machine.reply(&shallow, Reply::Value(Ok(Value::Int(7)))),
        Outcome::Done(Ok(Value::Int(7)))
    ));
    let Outcome::Request(Request::Call { declaration, .. }) =
        machine.run(&deep)
    else {
        panic!("deep op")
    };
    assert_eq!(deep.tables.borrow().declarations[declaration].label, "op");
    let deep_cost = machine.charged();
    assert!(deep_cost > shallow_cost + 32, "new callable copy must charge phantom syntax: {shallow_cost}/{deep_cost}");
    // Abandon a genuine deep pending request, then force failure during its
    // copied witness before another Host call can be presented.
    machine.set_budget(Some(shallow_cost));
    assert!(matches!(
        machine.run(&deep),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: Reason::Budget,
            ..
        }))
    ));
    machine.set_budget(Some(100_000));
    assert!(matches!(
        machine.run(&shallow),
        Outcome::Request(Request::Call { .. })
    ));
    assert!(matches!(
        machine.reply(&shallow, Reply::Value(Ok(Value::Int(7)))),
        Outcome::Done(Ok(Value::Int(7)))
    ));
}

#[test]
fn host_written_defaults_deliver_each_generic_instance_to_the_host() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let header="pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: () -> (Integer * String) = _ => { first: Integer = entry 7\nsecond: String = entry \"word\"\n(first, second) } }\npub entry: (seed: a) -> (n: a = seed) -> a";
    let mut host = Declarations::new();
    host.module("probe", header).unwrap();
    let module = parse(lex("use probe\nprobe.A.f ()")).unwrap();
    let unit =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    for expected in [Value::Int(7), Value::text("word")] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("entry request: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "entry"
        );
        assert_eq!(arguments, vec![expected.clone(), expected.clone()]);
        outcome = machine.reply(&program, Reply::Value(Ok(expected)));
    }
    assert!(
        matches!(outcome,Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice()==[Value::Int(7),Value::text("word")])
    );
}

#[test]
fn host_written_default_queues_keep_late_plain_and_family_results() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{structural, Value},
        vm::Machine,
    };
    for stage in ["g = x => x", "g = x => Just x"] {
        let header = format!("pub type A {{ pub n: Integer }}\nimpl A {{ n => Self {{ n: n }}, f: Integer -> Maybe<Integer> = seed => entry seed, {stage} }}\npub entry: (seed: Integer) -> (value: Maybe<Integer> = (Just seed |: A.g)) -> Maybe<Integer>");
        let mut declarations = Declarations::new();
        declarations.module("probe", &header).expect("Host header");
        let module = parse(lex("use probe\nprobe.A.f 7")).expect("main");
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .expect("late default stage checks");
        let program = compile::compile(&unit);
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("one actual entry request")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "entry"
        );
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[0], Value::Int(7));
        assert_eq!(
            structural::show(&arguments[1], &program.tables.borrow()),
            "Just 7"
        );
        let Outcome::Done(Ok(value)) =
            machine.reply(&program, Reply::Value(Ok(arguments[1].clone())))
        else {
            panic!("entry reply is terminal")
        };
        assert_eq!(
            structural::show(&value, &program.tables.borrow()),
            "Just 7"
        );
    }
}

#[test]
fn host_written_default_owners_keep_late_field_accesses_per_invocation() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let header = "pub type Box { pub n: Integer }\npub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: () -> (Integer * Integer) = _ => (entry 7, entry 8), g = x => Box x }\npub entry: (seed: Integer) -> (n: Integer = (A.g seed).n) -> Integer";
    let mut declarations = Declarations::new();
    declarations.module("probe", header).expect("Host header");
    let module = parse(lex("use probe\nprobe.A.f ()")).expect("main");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("late field defaults");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    for expected in [7, 8] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("one entry per invocation: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "entry"
        );
        assert_eq!(arguments, vec![Value::Int(expected), Value::Int(expected)]);
        outcome =
            machine.reply(&program, Reply::Value(Ok(Value::Int(expected))));
    }
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == [Value::Int(7), Value::Int(8)])
    );
}

#[test]
fn host_written_default_owners_stamp_constrained_callee_and_constructor_work() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::structural,
        vm::Machine,
    };
    let header = "pub type Box<T: Eq>: Eq { pub n: T }\npub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: Integer -> Box<Integer> = seed => entry seed }\npub take: Eq a => a -> a = x => x\npub entry: (seed: Integer) -> (n: Box<Integer> = (take (Box seed))) -> Box<Integer>";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("constrained Host header");
    let module = parse(lex("use probe\nprobe.A.f 7")).expect("main");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("callee and constructor obligations");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("entry follows written take")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "entry"
    );
    assert_eq!(arguments.len(), 2);
    assert_eq!(
        structural::show(&arguments[1], &program.tables.borrow()),
        "Box 7"
    );
    let Outcome::Done(Ok(value)) =
        machine.reply(&program, Reply::Value(Ok(arguments[1].clone())))
    else {
        panic!("terminal boxed result")
    };
    assert_eq!(structural::show(&value, &program.tables.borrow()), "Box 7");
}

#[test]
fn foreign_file_written_defaults_keep_their_checked_member_context() {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::structural, vm::Machine,
    };
    for stage in ["g = x => x", "g = x => Just x"] {
        let source = format!("pub type A {{ pub n: Integer }}\nimpl A {{ pub f: Integer -> Maybe<Integer> -> Maybe<Integer> = seed (value = (Just seed |: A.g)) => value, pub {stage} }}");
        let files =
            vec![("probe".into(), parse(lex(&source)).expect("File library"))];
        let module = parse(lex("use probe\nprobe.A.f 7")).expect("main");
        let unit = check::check(
            &module,
            &Declarations::new(),
            &files,
            check::Options::default(),
        )
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        let program = compile::compile(&unit);
        let Outcome::Done(Ok(value)) =
            Machine::new(Some(100_000)).run(&program)
        else {
            panic!("File default must complete without Host effects")
        };
        assert_eq!(
            structural::show(&value, &program.tables.borrow()),
            "Just 7"
        );
    }
}

fn foreign_choose_default(module: &fipe::ast::Module) -> fipe::ast::ExprId {
    use fipe::ast::TypeKind;
    let signature = module
        .entries
        .iter()
        .find_map(|entry| {
            let fipe::ast::Entry::Item(item) = entry else {
                return None;
            };
            let fipe::ast::ItemKind::Declare { name, ty } = &item.kind else {
                return None;
            };
            (name.segments.as_slice() == ["choose"]).then_some(*ty)
        })
        .expect("choose signature");
    let mut pending_types = vec![signature];
    loop {
        let id = pending_types.pop().expect("actual signature default");
        match &module.ty(id).expect("signature type node").kind {
            TypeKind::Function { parameter, result } => {
                pending_types.push(*result);
                pending_types.push(*parameter);
            }
            TypeKind::Labeled {
                default: Some(value),
                ..
            } => break *value,
            TypeKind::Labeled { ty, .. } | TypeKind::Qualified { ty, .. } => {
                pending_types.push(*ty);
            }
            _ => (),
        }
    }
}

#[test]
fn foreign_default_hidden_reads_use_their_own_ast_and_actual_type() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Prelude, Value},
        vm::Machine,
    };
    use std::fmt::Write;
    let mut padding = String::new();
    for index in 0..20 {
        writeln!(&mut padding, "pad{index} = 0").unwrap();
    }
    let header = format!("{padding}pub choose: (seed: a) -> (parsed: Result<a, ParseError> = (read \"7\")) -> Result<a, ParseError>");
    let mut declarations = Declarations::new();
    declarations
        .module("foreign", &header)
        .expect("foreign header");
    let file = parse(lex("use foreign\npub f = seed => foreign.choose seed"))
        .expect("generic File caller");
    let foreign_module = &declarations
        .get_module("foreign")
        .expect("foreign namespace")
        .module;
    let default = foreign_choose_default(foreign_module);
    assert!(foreign_module.expr(default).is_some());
    assert!(
        file.expr(default).is_none(),
        "actual default exists only in its foreign arena"
    );
    let files = vec![("caller".into(), file)];
    for source in [
        "use caller\n(caller.f 0, caller.f 0.0)",
        "use caller\n(caller.f 0.0, caller.f 0)",
    ] {
        let module = parse(lex(source)).expect("main");
        let unit = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .expect("foreign default checks");
        assert!(
            unit.tables()
                .definitions
                .iter()
                .any(|definition| definition.label == "f"
                    && definition.hidden.len() == 1),
            "generic caller transports the default's Read witness"
        );
        let program = compile::compile(&unit);
        let expected = if source.contains("f 0.0, caller.f 0)") {
            [Value::F64(7.0), Value::Int(7)]
        } else {
            [Value::Int(7), Value::F64(7.0)]
        };
        let mut machine = Machine::new(Some(100_000));
        let mut outcome = machine.run(&program);
        let mut replies = Vec::new();
        for expected in expected {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("foreign choose request: {outcome:?}")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "choose"
            );
            assert_eq!(arguments.len(), 2);
            let parsed = Prelude::new(&program.tables.borrow()).ok(expected);
            assert_eq!(arguments[1], parsed);
            replies.push(parsed.clone());
            outcome = machine.reply(&program, Reply::Value(Ok(parsed)));
        }
        assert!(
            matches!(outcome, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == replies.as_slice())
        );
    }
}

fn assert_record_default_listings(typed: &fipe::typed::Typed) {
    let mut listings = 0;
    for instance in &typed.declared {
        for (site, value) in &instance.typed.derived_defaults {
            let fipe::typed::DefaultValue::Record { ty, fields } = value else {
                continue;
            };
            if !matches!(ty, fipe::types::Type::Named { name, .. } if name.name == "R")
            {
                continue;
            }
            assert_eq!(fields.len(), 2);
            assert!(
                fields.iter().all(|(_, _, value)| matches!(
                    value,
                    fipe::typed::DefaultValue::Expression(_)
                )),
                "listing retains both actual expression snapshots"
            );
            assert!(
                !instance
                    .typed
                    .default_instances
                    .keys()
                    .any(|(start, end, _)| (*start, *end) == *site),
                "temporary listing lookup map is selectively restored"
            );
            listings += 1;
        }
    }
    assert!(
        listings > 0,
        "actual Derived::Default Record listing route must be checked"
    );
}

#[test]
fn host_written_record_default_listing_keeps_instances_and_effect_order() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let header = "pub seed: () -> Integer\npub type R: Default { pub n: Integer = (seed ()), pub doubled: Integer = (n + n) }\npub entry: (value: R = (R.default)) -> R\npub type A { pub n: Integer }\nimpl A { make: () -> (R * R) = _ => (entry, entry) }";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("record default header");
    let module = parse(lex("use probe\nprobe.A.make ()")).expect("main");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("resolved main");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_record_default_listings(&typed);
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("embedded record defaults");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    let mut expected = Vec::new();
    for seed in [7, 8] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("seed precedes entry: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "seed"
        );
        assert_eq!(arguments, vec![Value::Unit]);
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Int(seed))));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("entry follows its own record construction: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "entry"
        );
        assert_eq!(arguments.len(), 1);
        let Value::Record(record) = &arguments[0] else {
            panic!("R argument")
        };
        assert_eq!(
            record.fields.as_slice(),
            [Value::Int(seed), Value::Int(seed + seed)]
        );
        expected.push(arguments[0].clone());
        outcome =
            machine.reply(&program, Reply::Value(Ok(arguments[0].clone())));
    }
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == expected.as_slice())
    );
}

#[test]
fn file_default_private_members_keep_distinct_files_with_identical_local_sites()
{
    use fipe::{
        ast::{Entry, ImplMember, ItemKind},
        check, compile,
        decl::Declarations,
        embed::Outcome,
        lex::lex,
        parse::parse,
        value::structural,
        vm::Machine,
    };
    let mut files = Vec::new();
    let mut sites = Vec::new();
    for (stem, increment) in [("one", 1), ("two", 2)] {
        let text = format!("pub type A {{ pub n: Integer }}\nimpl A {{ pub f: Integer -> Maybe<Integer> -> Maybe<Integer> = seed (value = (Just seed |: A.g)) => value, g = x => x + {increment} }}");
        let module = parse(lex(&text)).expect("File with private helper");
        let site = module
            .entries
            .iter()
            .find_map(|entry| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let ItemKind::Impl { members, .. } = &item.kind else {
                    return None;
                };
                members.iter().find_map(|member| {
                    let ImplMember::Named {
                        name,
                        value: Some(value),
                        ..
                    } = member
                    else {
                        return None;
                    };
                    (name.segments.as_slice() == ["g"])
                        .then_some((name.span, *value))
                })
            })
            .expect("actual g site and body");
        sites.push(site);
        files.push((String::from(stem), module));
    }
    assert_eq!(
        sites[0], sites[1],
        "different files deliberately share local span and ExprId"
    );
    let declarations = Declarations::new();
    for (source, expected) in [
        (
            "use one\nuse two\n(one.A.f 7, two.A.f 7)",
            "(Just 8, Just 9)",
        ),
        (
            "use one\nuse two\n(two.A.f 7, one.A.f 7)",
            "(Just 9, Just 8)",
        ),
    ] {
        let module = parse(lex(source)).expect("main");
        let unit = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .expect("private helpers visible in their own defaults");
        let program = compile::compile(&unit);
        let Outcome::Done(Ok(value)) =
            Machine::new(Some(100_000)).run(&program)
        else {
            panic!("ordinary File result")
        };
        assert_eq!(
            structural::show(&value, &program.tables.borrow()),
            expected
        );
    }
    let module = parse(lex("use one\none.A.g 7")).expect("outside private use");
    let errors =
        check::check(&module, &declarations, &files, check::Options::default())
            .expect_err("private helper remains inaccessible outside its File");
    assert!(errors.iter().any(|error| match error {
        check::Error::Resolve(error) => matches!(&error.reason, fipe::resolve::Reason::PrivateMember { member } if member == "g"),
        check::Error::Type(error) => matches!(&error.reason, fipe::infer::Reason::PrivateMember { member } if member == "g"),
        _ => false,
    }), "{errors:?}");
}

#[test]
fn nested_same_file_defaults_keep_owner_bearing_types_in_local_context() {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    let file = "pub type A<T> { pub value: T }\nimpl A<T> { pub f: A<T> -> A<T> -> T = seed (value = (A.g seed)) => value.value, g: A<T> -> A<T> -> A<T> = seed (copy = seed) => copy }";
    let files =
        vec![("one".into(), parse(lex(file)).expect("nested File default"))];
    for source in [
        "use one\n(one.A.f (one.A 7), one.A.f (one.A \"word\"))",
        "use one\n(one.A.f (one.A \"word\"), one.A.f (one.A 7))",
    ] {
        let module = parse(lex(source)).expect("main");
        let unit = check::check(
            &module,
            &Declarations::new(),
            &files,
            check::Options::default(),
        )
        .expect("nested nominal defaults");
        let program = compile::compile(&unit);
        let Outcome::Done(Ok(Value::Tuple(values))) =
            Machine::new(Some(100_000)).run(&program)
        else {
            panic!("nested defaults complete")
        };
        let expected = if source.contains("f (one.A 7),") {
            vec![Value::Int(7), Value::text("word")]
        } else {
            vec![Value::text("word"), Value::Int(7)]
        };
        assert_eq!(values.as_slice(), expected.as_slice());
    }
}

#[test]
fn only_checked_files_initialize_once_in_dependency_order_with_nested_defaults()
{
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub tick: Integer -> Integer")
        .expect("initializer Host");
    let one = "use host\npub stamp = host.tick 1\npub type A<T> { pub value: T }\nimpl A<T> { pub f: A<T> -> A<T> -> T = seed (value = (A.g seed)) => value.value, g: A<T> -> A<T> -> A<T> = seed (copy = seed) => copy }";
    let two = "use one host\npub stamp = host.tick 2";
    let files = vec![
        ("two".into(), parse(lex(two)).expect("dependent File")),
        ("one".into(), parse(lex(one)).expect("dependency File")),
    ];
    let module = parse(lex("use two one\n(one.stamp, two.stamp, one.A.f (one.A 7), one.A.f (one.A \"word\"))"))
        .expect("main");
    let unit =
        check::check(&module, &declarations, &files, check::Options::default())
            .expect("nested defaults and true File initializers");
    assert_eq!(
        unit.tables()
            .modules
            .iter()
            .map(|module| module.label.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    for (argument, reply) in [(1, 11), (2, 22)] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("expected initializer tick {argument}: {outcome:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "tick"
        );
        assert_eq!(arguments, vec![Value::Int(argument)]);
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Int(reply))));
    }
    let expected = vec![
        Value::Int(11),
        Value::Int(22),
        Value::Int(7),
        Value::text("word"),
    ];
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == expected.as_slice())
    );
    let repeated = machine.run(&program);
    assert!(
        matches!(repeated, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == expected.as_slice()),
        "the same Machine reuses initialized files without more Host calls"
    );
}

fn assert_intermediate_access_snapshot(typed: &fipe::typed::Typed) {
    use fipe::typed::DefaultValue;
    let mut pending = vec![typed];
    let mut seen = std::collections::BTreeSet::new();
    let mut intermediate_defaults = 0;
    while let Some(tree) = pending.pop() {
        if !seen.insert(std::ptr::from_ref(tree)) {
            continue;
        }
        for instance in
            tree.declared.iter().chain(tree.default_instances.values())
        {
            pending.push(&instance.typed);
        }
        for steps in tree.accesses.values() {
            for (index, step) in steps.iter().enumerate() {
                let Some(bound) = &step.bound else { continue };
                for (_, default) in &bound.defaults {
                    let DefaultValue::Expression(instance) = default else {
                        continue;
                    };
                    if index + 1 < steps.len() {
                        intermediate_defaults += 1;
                    }
                    pending.push(&instance.typed);
                }
            }
        }
    }
    assert!(
        intermediate_defaults > 0,
        "actual intermediate Access.bound retains its expression default"
    );
}

#[test]
fn intermediate_access_defaults_keep_pending_snapshots_and_generic_witnesses() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let header = "pub echo: a -> a\npub type B { pub n: Integer }\npub type A { pub seed: Integer }\nimpl A { pub step: (self: Self) -> (n: Integer = (A.g 7)) -> B = self n => B n, g = x => echo x }\npub entry: (value: Integer = ((A 0).step.n)) -> Integer";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("intermediate default header");
    let module = parse(lex("use probe\nprobe.entry")).expect("main");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("resolved main");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_intermediate_access_snapshot(&typed);
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("intermediate default checks");
    let program = compile::compile(&unit);
    for wrong in [false, true] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("generic echo precedes entry")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "echo"
        );
        assert_eq!(arguments, vec![Value::Int(7)]);
        let result = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(matches!(&result, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(message), .. })) if message == "the reply is not an Integer"), "wrong String reply must be refused as Integer before entry: {result:?}");
        } else {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = result
            else {
                panic!(
                    "entry follows completed intermediate default: {result:?}"
                )
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "entry"
            );
            assert_eq!(arguments, vec![Value::Int(7)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        }
    }
}

#[test]
fn retained_default_witness_checks_host_reply_before_later_effect() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n";
    const SOURCE: &str = "use host\nmake = w => host.R { w: w }\nr = make (host.W 1)\nhost.after r.result\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    for valid in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request");
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments.as_slice(), [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if valid {
                Value::Int(7)
            } else {
                Value::text("wrong")
            })),
        );
        if valid {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after exact healthy result: {outcome:?}");
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments.as_slice(), [Value::Int(7)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: Reason::Host(ref message), .. })) if message.contains("Integer")),
                "specific Integer reply refusal before after: {outcome:?}"
            );
        }
    }
}
#[test]
fn retained_private_default_witness_checks_host_reply_before_later_effect() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, result: T = w.op }\npub after: Integer -> Integer\npub read: R<a> -> a = r => r.result\n";
    const SOURCE: &str = "use host\nmake = w => host.R { w: w }\nr = make (host.W 1)\nhost.after (host.read r)\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    for valid in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request");
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments.as_slice(), [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if valid {
                Value::Int(7)
            } else {
                Value::text("wrong")
            })),
        );
        if valid {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after exact healthy result: {outcome:?}");
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments.as_slice(), [Value::Int(7)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: Reason::Host(ref message), .. })) if message.contains("Integer")),
                "specific Integer reply refusal before after: {outcome:?}"
            );
        }
    }
}

#[test]
fn annotated_default_witness_keeps_caller_rigid_before_later_effect() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n";
    const SOURCE: &str = "use host\nmake: host.W<a> -> host.R<a> = w => host.R { w: w }\nr = make (host.W 1)\nhost.after r.result\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    for valid in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request");
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments.as_slice(), [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if valid {
                Value::Int(7)
            } else {
                Value::text("wrong")
            })),
        );
        if valid {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after exact healthy result: {outcome:?}");
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments.as_slice(), [Value::Int(7)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: Reason::Host(ref message), .. })) if message.contains("Integer")),
                "specific Integer reply refusal before after: {outcome:?}"
            );
        }
    }
}

#[test]
fn inherited_foreign_predicates_keep_actual_nominal_arguments_before_effects() {
    const SOURCE: &str = "use host\ntype Box { n: Integer }\nother = host.Box \"host\"\nxs: host.Derived<Box> = host.items (Box 7)\nhost.after xs\n";
    assert_inherited_nominal_runtime(SOURCE, false);
}

#[test]
fn local_alias_discovers_inherited_predicates_before_later_effects() {
    const SOURCE: &str = "use host\ntype Box { n: Integer }\nother = host.Box \"host\"\ntype Alias<T>: host.Derived<T> where self.len < 4\nxs: Alias<Box> = host.items (Box 7)\nhost.after xs\n";
    assert_inherited_nominal_runtime(SOURCE, true);
}

fn assert_inherited_nominal_runtime(source: &str, local_alias: bool) {
    use fipe::{check, compile, decl::Declarations, lex::lex, parse::parse};
    const HOST: &str = "pub type Box { pub tag: String }\npub type Base<T>: Vec<T> where self.len > 0\npub type Derived<T>: Base<T> where self.len < 3\npub items: a -> Vec<a>\npub after: Vec<a> -> Integer\n";

    let mut declarations = Declarations::new();
    declarations
        .module("host", HOST)
        .expect("inherited Host limits");
    let module = parse(lex(source)).expect("caller nominal");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("checked inherited limits");
    let program = compile::compile(&unit);
    let (base, derived, local_box) = {
        let tables = program.tables.borrow();
        let host = tables
            .namespaces
            .iter()
            .position(|name| name == "host")
            .unwrap();
        let main = tables
            .namespaces
            .iter()
            .position(|name| name == "main")
            .unwrap();
        if local_alias {
            let own = tables
                .tests
                .iter()
                .filter(|test| {
                    test.namespace == main
                        && source.get(test.span.start..test.span.end)
                            == Some("self.len < 4")
                })
                .count();
            assert_eq!(own, 1, "local predicate is registered exactly once");
        }
        let test = |text: &str| {
            tables
                .tests
                .iter()
                .position(|test| {
                    test.namespace == host
                        && HOST.get(test.span.start..test.span.end)
                            == Some(text)
                })
                .expect("actual checked test")
        };
        (
            test("self.len > 0"),
            test("self.len < 3"),
            tables
                .meanings
                .iter()
                .position(|meaning| {
                    meaning.namespace == main && meaning.label == "Box"
                })
                .unwrap(),
        )
    };
    assert_inherited_program_runtime(
        &program,
        &[],
        base,
        derived,
        local_box,
        None,
    );
}

fn observe_inherited_witness_frames(
    machine: &fipe::vm::Machine,
    base: usize,
    derived: usize,
    local_box: usize,
    own: Option<usize>,
    visited: &mut Vec<usize>,
    own_seen: &mut bool,
) {
    use fipe::{ir, vm::Frame};
    let joined = machine.program().expect("joined runtime tests");
    for frame in machine.frames() {
        let Frame::Body(frame) = frame else { continue };
        if own.is_some_and(|test| joined.tests[test] == Some(frame.body)) {
            assert_eq!(
                frame.meanings,
                [ir::Type::Meaning {
                    index: local_box,
                    arguments: vec![]
                }],
                "canonical File/Line predicate uses actual caller Box"
            );
            *own_seen = true;
        }
        for test in [base, derived] {
            if joined.tests[test] == Some(frame.body) {
                assert_eq!(
                    frame.meanings,
                    [ir::Type::Meaning {
                        index: local_box,
                        arguments: vec![]
                    }]
                );
                if !visited.contains(&test) {
                    visited.push(test);
                }
            }
        }
    }
}

fn assert_inherited_verdict(
    result: &Result<fipe::value::Value, fipe::vm::Fault>,
    nonempty: bool,
    calls: &[String],
    visited: &[usize],
    base: usize,
    own_seen: bool,
) {
    use fipe::value::{Reason, Value};
    if nonempty {
        assert_eq!(calls, ["items", "after"]);
        assert_eq!(result, &Ok(Value::Int(7)));
    } else {
        assert!(!own_seen, "Base rejects before own predicate");
        assert_eq!(calls, ["items"]);
        assert_eq!(
            visited,
            [base],
            "only the inherited Base test executes before refusal"
        );
        assert!(
            matches!(
                result,
                Err(fipe::vm::Fault {
                    reason: Reason::Failed { .. },
                    ..
                })
            ),
            "inherited Base refusal: {result:?}"
        );
    }
}

fn assert_inherited_program_runtime(
    program: &fipe::mir::Program,
    prefix: &[fipe::mir::Program],
    base: usize,
    derived: usize,
    local_box: usize,
    own: Option<usize>,
) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::{Machine, Stop},
    };
    for nonempty in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        for initial in prefix {
            assert!(
                matches!(
                    machine.run(initial),
                    fipe::embed::Outcome::Done(Ok(_))
                ),
                "accepted alias declaration has no runtime requests"
            );
        }
        machine.begin(program);
        let mut visited = Vec::new();
        let mut own_seen = false;
        let mut calls = Vec::new();
        let mut finished = false;
        for _ in 0..10_000 {
            match machine.next_stop() {
                Stop::Before(_) => {
                    observe_inherited_witness_frames(
                        &machine,
                        base,
                        derived,
                        local_box,
                        own,
                        &mut visited,
                        &mut own_seen,
                    );
                }
                Stop::After(_, _) => (),
                Stop::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    let label = program.tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                    let reply = match label.as_str() {
                        "items" => {
                            assert!(calls.is_empty());
                            let [value @ Value::Record(item)] =
                                arguments.as_slice()
                            else {
                                panic!("genuine script-created Box")
                            };
                            assert_eq!(item.meaning, local_box);
                            assert_eq!(item.fields, [Value::Int(7)]);
                            Value::seq(if nonempty {
                                vec![value.clone()]
                            } else {
                                vec![]
                            })
                        }
                        "after" => {
                            assert!(
                                nonempty,
                                "empty Base must refuse before after"
                            );
                            assert_eq!(calls, ["items"]);
                            assert_eq!(visited, [base, derived]);
                            assert!(
                                own.is_none() || own_seen,
                                "own predicate executes before after"
                            );
                            let [Value::Seq(values)] = arguments.as_slice()
                            else {
                                panic!("checked sequence")
                            };
                            let [Value::Record(item)] = values.as_slice()
                            else {
                                panic!("returned Box")
                            };
                            assert_eq!(item.meaning, local_box);
                            assert_eq!(item.fields, [Value::Int(7)]);
                            Value::Int(7)
                        }
                        _ => panic!("unexpected Host call {label}"),
                    };
                    calls.push(label);
                    machine.answer(Reply::Value(Ok(reply)));
                }
                Stop::Done(result) => {
                    assert_inherited_verdict(
                        &result, nonempty, &calls, &visited, base, own_seen,
                    );
                    finished = true;
                    break;
                }
                stop @ Stop::Request(_) => panic!("unexpected stop {stop:?}"),
            }
        }
        assert!(finished, "finite stepped proof must finish");
    }
}

#[test]
fn inherited_entries_preserve_nongeneric_gates_and_known_refusals() {
    assert_eq!(shown("type Small: Integer where self < 8\n3 | Small"), "3");
    assert_eq!(shown("type Small: Integer where self < 8\nf: Integer -> Small = n => n | Small\nf 3"), "3");
    let source = "type Small: Integer where self < 8\nx: Small = 9\nx";
    let module = fipe::parse::parse(fipe::lex::lex(source))
        .expect("valid static limit source");
    assert!(
        fipe::check::check(
            &module,
            &fipe::decl::Declarations::new(),
            &[],
            fipe::check::Options::default()
        )
        .is_err(),
        "known false limit remains a checker refusal"
    );
}

#[test]
fn file_and_accepted_line_aliases_discover_inherited_predicates_once() {
    assert_file_and_line_aliases(&[true, false]);
}

#[test]
fn accepted_line_alias_discovers_inherited_predicates_independently() {
    assert_file_and_line_aliases(&[false]);
}

fn load_inherited_alias_use(
    session: &mut fipe::session::Session,
    source: &str,
    file: bool,
    library: &str,
) -> fipe::session::Report {
    use fipe::embed::{Outcome, Reply, Request};
    match session.line(source) {
        Outcome::Request(Request::File { stem }) => {
            assert!(file);
            assert_eq!(stem, "foreign");
            let Outcome::Done(report) =
                session.reply(Reply::File(Some(library.into())))
            else {
                panic!("one public file reply completes loading")
            };
            report
        }
        Outcome::Done(report) => {
            assert!(!file, "File control must actually request its source");
            report
        }
        other @ Outcome::Request(_) => {
            panic!("unexpected Session outcome: {other:?}")
        }
    }
}

fn assert_file_and_line_aliases(routes: &[bool]) {
    use fipe::{compile, decl::Declarations, embed::Outcome, session::Session};
    const HOST: &str = "pub type Box { pub tag: String }\npub type Base<T>: Vec<T> where self.len > 0\npub type Derived<T>: Base<T> where self.len < 3\npub items: a -> Vec<a>\npub after: Vec<a> -> Integer\n";
    const LIBRARY: &str =
        "use host\npub type Alias<T>: host.Derived<T> where self.len < 4\n";
    for &file in routes {
        let mut declarations = Declarations::new();
        declarations
            .module("host", HOST)
            .expect("inherited Host predicates");
        let mut session = Session::new(declarations);
        let mut prefix = Vec::new();
        let alias = if file { "foreign.Alias" } else { "Alias" };
        if !file {
            let Outcome::Done(report) = session.line(LIBRARY) else {
                panic!("accepted alias line does not load a file")
            };
            assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
            assert!(report.accepted.is_some());
            prefix
                .push(compile::compile(&report.unit.expect("alias line unit")));
        }
        let source = format!("use host{}\ntype Box {{ n: Integer }}\nother = host.Box \"host\"\nxs: {alias}<Box> = host.items (Box 7)\nhost.after xs\n", if file { " foreign" } else { "" });
        let report =
            load_inherited_alias_use(&mut session, &source, file, LIBRARY);
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(report.accepted.is_some());
        let program = compile::compile(
            &report.unit.expect("alias use emits executable unit"),
        );
        let (base, derived, local_box, own) = {
            let tables = program.tables.borrow();
            let host = tables
                .namespaces
                .iter()
                .position(|name| name == "host")
                .unwrap();
            let test = |text: &str| {
                tables
                    .tests
                    .iter()
                    .position(|test| {
                        test.namespace == host
                            && HOST.get(test.span.start..test.span.end)
                                == Some(text)
                    })
                    .expect("inherited supplied test is retained")
            };
            let own: Vec<_> = tables
                .tests
                .iter()
                .filter(|test| {
                    test.namespace != host
                        && LIBRARY.get(test.span.start..test.span.end)
                            == Some("self.len < 4")
                })
                .collect();
            assert_eq!(own.len(), 1, "File/Line predicate retains exactly one canonical registration");
            let namespace = &tables.namespaces[own[0].namespace];
            assert_eq!(namespace, if file { "foreign" } else { "line 1" });
            let local_box = tables
                .meanings
                .iter()
                .position(|meaning| {
                    meaning.label == "Box" && meaning.namespace != host
                })
                .expect("genuine caller Box");
            (
                test("self.len > 0"),
                test("self.len < 3"),
                local_box,
                tables
                    .tests
                    .iter()
                    .position(|test| std::ptr::eq(test, own[0]))
                    .unwrap(),
            )
        };
        assert_inherited_program_runtime(
            &program,
            &prefix,
            base,
            derived,
            local_box,
            Some(own),
        );
    }
}

#[test]
fn grouped_receivers_keep_whole_chains_single_fields_and_one_evaluation() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{structural, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations
        .module("probe", "pub tick: () -> Integer")
        .expect("effect header");
    let text = "use probe\ntype Inner { n: Integer }\ntype Outer { n: Integer, inner: Inner }\n((Outer 9 (Inner (probe.tick ()))).inner.n, (Inner 3).n)";
    let module = parse(lex(text)).expect("grouped chains");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("checked chains");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("one receiver effect")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "tick"
    );
    assert_eq!(arguments, vec![Value::Unit]);
    let Outcome::Done(Ok(value)) =
        machine.reply(&program, Reply::Value(Ok(Value::Int(7))))
    else {
        panic!("no duplicate receiver effect")
    };
    assert_eq!(structural::show(&value, &program.tables.borrow()), "(7, 3)");
    let file = parse(lex("pub type Inner { pub n: Integer }\npub type Outer { inner: Inner }\npub item = Outer (Inner 7)\npub inside = (item).inner.n")).expect("private field source");
    let files = vec![("one".into(), file)];
    let module = parse(lex("use one\none.inside")).expect("own-source access");
    let unit = check::check(
        &module,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .expect("private access allowed in owner");
    let program = compile::compile(&unit);
    assert_eq!(
        Machine::new(Some(100_000)).run(&program),
        Outcome::Done(Ok(Value::Int(7)))
    );
    let module = parse(lex("use one\n(one.item).inner.n"))
        .expect("outside private access");
    let errors = check::check(
        &module,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .expect_err("intermediate private field remains private");
    assert!(errors.iter().any(|error| matches!(error, check::Error::Type(error) if matches!(error.reason, fipe::infer::Reason::PrivateField { .. }))), "{errors:?}");
}

#[test]
fn declared_written_labeled_defaults_bind_earlier_spots_at_each_call() {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("probe", "pub keep: Eq a => (seed: a) -> (copy: a = seed) -> a = seed copy => copy").expect("qualified declaration");
    let module = parse(lex("use probe\n(probe.keep 7, probe.keep \"word\")"))
        .expect("two instantiations");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("labeled defaults");
    let program = compile::compile(&unit);
    let Outcome::Done(Ok(Value::Tuple(values))) =
        Machine::new(Some(100_000)).run(&program)
    else {
        panic!("written bodies/defaults do not request Host callbacks")
    };
    assert_eq!(values.as_slice(), [Value::Int(7), Value::text("word")]);
}

#[test]
fn declared_signature_metadata_survives_cold_cached_and_same_source_calls() {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    let header = "pub keep: Eq a => (seed: a) -> (copy: a = seed) -> a = x y => y\npub type A { pub n: Integer }\nimpl A { pub route: Integer -> Integer = n => keep n, pub member: (seed: Integer) -> (copy: Integer = seed) -> Integer = x y => y, pub local: Integer -> Integer = n => A.member n }\npub narrow: (seed: U8) -> (copy: U8 = seed) -> U8 = x y => y";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("public signature labels differ from body binders");
    for (source, expected) in [
        ("use probe\n(probe.keep 7, probe.keep \"word\", probe.A.route 8, probe.A.local 9, probe.A.member 10, probe.narrow 7)", vec![Value::Int(7), Value::text("word"), Value::Int(8), Value::Int(9), Value::Int(10), Value::Int(7)]),
        ("use probe\n(probe.keep \"word\", probe.keep 7, probe.A.local 9, probe.A.route 8, probe.A.member 10, probe.narrow 7)", vec![Value::text("word"), Value::Int(7), Value::Int(9), Value::Int(8), Value::Int(10), Value::Int(7)]),
    ] {
        let module = parse(lex(source)).expect("main");
        let unit = check::check(&module, &declarations, &[], check::Options::default()).expect("cold/cache/same-source defaults");
        let program = compile::compile(&unit);
        let Outcome::Done(Ok(Value::Tuple(values))) = Machine::new(Some(100_000)).run(&program) else { panic!("no checking/runtime callbacks for written identities") };
        assert_eq!(values.as_slice(), expected.as_slice());
    }
}

#[test]
fn declared_signature_metadata_keeps_explicit_overrides_and_required_slot_partials(
) {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    let header = "pub keep: Eq a => (seed: a) -> (copy: a = seed) -> a = x y => y\npub combine: Eq a => (seed: a) -> (other: a) -> (copy: a = seed) -> a = x y z => z";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("written annotation metadata");
    for source in [
        "use probe\np = probe.combine 7\nq = probe.combine \"word\"\n(probe.keep 7 9, probe.keep \"seed\" \"override\", p 99, q \"ignored\")",
        "use probe\nq = probe.combine \"word\"\np = probe.combine 7\n(probe.keep 7 9, probe.keep \"seed\" \"override\", p 99, q \"ignored\")",
    ] {
        let module = parse(lex(source)).expect("explicit and partial calls");
        let unit = check::check(&module, &declarations, &[], check::Options::default()).expect("checked annotation metadata");
        let program = compile::compile(&unit);
        let Outcome::Done(Ok(Value::Tuple(values))) = Machine::new(Some(100_000)).run(&program) else { panic!("explicit overrides and stored partial calls finish without Host callbacks") };
        assert_eq!(values.as_slice(), [Value::Int(9), Value::text("override"), Value::Int(7), Value::text("word")]);
    }
}

fn assert_distinct_intermediate_snapshots(typed: &fipe::typed::Typed) {
    use fipe::{resolve::Origin, typed::DefaultValue, types::Type};
    let mut pending = vec![typed];
    let mut seen = std::collections::BTreeSet::new();
    let mut observed = false;
    while let Some(tree) = pending.pop() {
        if !seen.insert(std::ptr::from_ref(tree)) {
            continue;
        }
        for instance in
            tree.declared.iter().chain(tree.default_instances.values())
        {
            pending.push(&instance.typed);
        }
        for (site, steps) in &tree.accesses {
            if steps.len() != 3
                || !matches!(&steps[0].head, Type::Named { name, .. } if name.name == "A")
            {
                continue;
            }
            let first_bound =
                steps[0].bound.as_ref().expect("first intermediate Bound");
            let next_bound =
                steps[1].bound.as_ref().expect("second intermediate Bound");
            let [(first_spot, DefaultValue::Expression(first))] =
                first_bound.defaults.as_slice()
            else {
                panic!("first owns exact Expression snapshot")
            };
            let [(next_spot, DefaultValue::Expression(next))] =
                next_bound.defaults.as_slice()
            else {
                panic!("second owns exact Expression snapshot")
            };
            assert_eq!(
                (*first_spot, *next_spot),
                (1, 1),
                "both defaults reuse the same full-span/spot key"
            );
            assert_ne!(
                first_bound.spots[1].0, next_bound.spots[1].0,
                "actual signature source sites differ"
            );
            assert_eq!(first.origin, Origin::Host("probe".into()));
            assert_eq!(next.origin, first.origin);
            assert_ne!(first.value, next.value);
            assert!(!std::rc::Rc::ptr_eq(&first.typed, &next.typed));
            assert!(
                matches!(first.typed.expr(first.value), Some(Type::Named { name, .. }) if name.origin == Origin::Prelude && name.name == "Integer")
            );
            assert!(
                matches!(next.typed.expr(next.value), Some(Type::Named { name, .. }) if name.origin == Origin::Prelude && name.name == "String")
            );
            let final_map = &tree.default_instances[&(site.0, site.1, 1)];
            assert_eq!(final_map.value, next.value);
            assert_ne!(
                final_map.value, first.value,
                "earlier step cannot use the final overwritten map"
            );
            observed = true;
        }
    }
    assert!(
        observed,
        "actual three-step chain and both owned default snapshots"
    );
}

fn intermediate_snapshot_reply_case(
    program: &fipe::mir::Program,
    wrong: usize,
) {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(program)
    else {
        panic!("first Integer tick")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "tick"
    );
    assert_eq!(arguments, vec![Value::Int(7)]);
    let outcome = machine.reply(
        program,
        Reply::Value(Ok(if wrong == 1 {
            Value::text("wrong")
        } else {
            Value::Int(7)
        })),
    );
    if wrong == 1 {
        assert!(
            matches!(&outcome, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(message), .. })) if message == "the reply is not an Integer"),
            "{outcome:?}"
        );
        return;
    }
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = outcome
    else {
        panic!("second String tick: {outcome:?}")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "tick"
    );
    assert_eq!(arguments, vec![Value::text("word")]);
    let outcome = machine.reply(
        program,
        Reply::Value(Ok(if wrong == 2 {
            Value::Int(9)
        } else {
            Value::text("word")
        })),
    );
    if wrong == 2 {
        assert!(
            matches!(&outcome, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(message), .. })) if message == "the reply is not a String"),
            "{outcome:?}"
        );
        return;
    }
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = outcome
    else {
        panic!("entry after both defaults: {outcome:?}")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "entry"
    );
    assert_eq!(
        arguments,
        vec![Value::Tuple(std::rc::Rc::new(vec![
            Value::Int(7),
            Value::text("word")
        ]))]
    );
    let expected = arguments[0].clone();
    assert_eq!(
        machine.reply(program, Reply::Value(Ok(expected.clone()))),
        Outcome::Done(Ok(expected))
    );
}

#[test]
fn intermediate_member_defaults_keep_distinct_same_key_snapshots_and_effect_order(
) {
    use fipe::{check, compile, decl::Declarations, lex::lex, parse::parse};
    let header = "pub tick: a -> a\npub type A { pub seed: Integer }\npub type B { pub n: Integer }\npub type C { pub pair: Integer * String }\nimpl A { pub first: (self: Self) -> (value: Integer = (tick 7)) -> B = self value => B value }\nimpl B { pub next: (self: Self) -> (value: String = (tick \"word\")) -> C = self value => C (self.n, value) }\npub entry: (value: (Integer * String) = ((A 0).first.next.pair)) -> (Integer * String)";
    let mut declarations = Declarations::new();
    declarations
        .module("probe", header)
        .expect("two intermediate defaults");
    let module = parse(lex("use probe\nprobe.entry")).expect("main");
    let resolved =
        fipe::resolve::resolve(&module, &declarations, &[]).expect("resolved");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_distinct_intermediate_snapshots(&typed);
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("checked chain");
    let program = compile::compile(&unit);
    for wrong in [0, 1, 2] {
        intermediate_snapshot_reply_case(&program, wrong);
    }
}

#[test]
fn default_replay_preserves_independent_host_helper_binders() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Reason, Value},
        vm::Machine,
    };
    const HOST: &str = "pub get: () -> a\npub helper: () -> a = _ => get ()\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => helper () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n";
    const SOURCE: &str = "use host\nmake = w => host.R { w: w }\nr = make (host.W 1)\nhost.after r.result\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let tables = program.tables.borrow();
    let helper = tables
        .definitions
        .iter()
        .position(|definition| definition.label == "helper")
        .expect("canonical raw Host helper definition");
    let maker = tables
        .definitions
        .iter()
        .position(|definition| definition.label == "make")
        .expect("caller maker definition");
    drop(tables);
    let helper =
        &program.bodies[program.definitions[helper].expect("helper body")];
    let maker =
        &program.bodies[program.definitions[maker].expect("maker body")];
    assert_eq!(
        helper.hidden.len(),
        1,
        "Host helper really consumes its result witness"
    );
    assert_eq!(maker.hidden.len(), 1);
    assert_ne!(
        helper.hidden[0], maker.hidden[0],
        "raw Host and caller binders stay distinct"
    );
    for valid in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request");
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments.as_slice(), [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if valid {
                Value::Int(7)
            } else {
                Value::text("wrong")
            })),
        );
        if valid {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after exact healthy result: {outcome:?}");
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments.as_slice(), [Value::Int(7)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: Reason::Host(ref message), .. })) if message.contains("Integer")),
                "specific Integer reply refusal before after: {outcome:?}"
            );
        }
    }
}

#[test]
fn constructor_arm_default_parameters_own_their_nested_bindings() {
    use fipe::{
        ast, check, compile, decl::Declarations, embed::Outcome, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    let source = "type R<T> { value: T }\nimpl R<a> {\nseed (copy = { id = x => x\nid seed }) => Self { value: copy }\n}\n((R 7).value, (R \"word\").value)\n";
    let module = parse(lex(source))
        .expect("constructor arm with a nested default lambda");
    let arm = module
        .entries
        .iter()
        .find_map(|entry| match entry {
            ast::Entry::Item(item) => match &item.kind {
                ast::ItemKind::Impl { members, .. } => {
                    members.iter().find_map(|member| match member {
                        ast::ImplMember::Constructor { arm, .. } => Some(arm),
                        ast::ImplMember::Named { .. } => None,
                    })
                }
                _ => None,
            },
            ast::Entry::Line(_) => None,
        })
        .expect("actual constructor-only arm, not a named method lambda");
    assert_eq!(arm.patterns.len(), 2);
    assert!(
        !matches!(
            &module.expr(arm.body).expect("constructor body").kind,
            ast::ExprKind::Arms(_)
        ),
        "the constructor arm itself has no expression wrapper"
    );
    let ast::PatternKind::Default { name, value } = &module
        .pattern(arm.patterns[1])
        .expect("default parameter")
        .kind
    else {
        panic!("actual default parameter pattern")
    };
    assert_eq!(name.segments, ["copy"]);
    let ast::ExprKind::Block(parts) =
        &module.expr(*value).expect("nested expression").kind
    else {
        panic!("default expression owns local bindings")
    };
    let ast::ExprKind::Bind { pattern, value, .. } =
        &module.expr(parts[0]).expect("nested binding").kind
    else {
        panic!("nested id binding")
    };
    assert!(
        matches!(&module.pattern(*pattern).expect("nested binder").kind, ast::PatternKind::Name(name) if name.segments == ["id"])
    );
    let ast::ExprKind::Arms(nested) =
        &module.expr(*value).expect("nested expression").kind
    else {
        panic!("nested lambda arm")
    };
    assert!(
        matches!(&module.pattern(nested[0].patterns[0]).expect("nested lambda parameter").kind, ast::PatternKind::Name(name) if name.segments == ["x"])
    );
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .expect("constructor/default binder ownership");
    let program = compile::compile(&unit);
    let outcome = Machine::new(Some(100_000)).run(&program);
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Tuple(ref values))) if values.as_slice() == [Value::Int(7), Value::text("word")]),
        "{outcome:?}"
    );
}

fn raw_file_formal_sites(
    library: &fipe::ast::Module,
) -> (fipe::token::Span, fipe::token::Span) {
    use fipe::ast;
    let g = library
        .entries
        .iter()
        .find_map(|entry| match entry {
            ast::Entry::Item(item) => match &item.kind {
                ast::ItemKind::Impl { members, .. } => {
                    members.iter().find_map(|member| match member {
                        ast::ImplMember::Named {
                            name,
                            value: Some(value),
                            ..
                        } if name.segments == ["g"] => Some(*value),
                        _ => None,
                    })
                }
                _ => None,
            },
            ast::Entry::Line(_) => None,
        })
        .expect("raw g body");
    let ast::ExprKind::Arms(arms) =
        &library.expr(g).expect("raw g expression").kind
    else {
        panic!("raw g callable")
    };
    let seed_site = library
        .pattern(arms[0].patterns[0])
        .expect("raw seed parameter")
        .span;
    let ast::PatternKind::Default {
        value: seed_read, ..
    } = &library
        .pattern(arms[0].patterns[1])
        .expect("raw copy parameter")
        .kind
    else {
        panic!("copy defaults from the earlier actual seed")
    };
    let seed_read_site =
        library.expr(*seed_read).expect("default seed read").span;
    (seed_site, seed_read_site)
}

#[test]
fn raw_file_formal_aliases_reuse_owned_slots_and_keep_replay_arguments() {
    use fipe::{
        check, compile, decl::Declarations, embed::Outcome, ir, lex::lex,
        parse::parse, value::Value, vm::Machine,
    };
    // Keep the original nested File fixture byte-for-byte; this is a companion identity oracle.
    let file = "pub type A<T> { pub value: T }\nimpl A<T> { pub f: A<T> -> A<T> -> T = seed (value = (A.g seed)) => value.value, g: A<T> -> A<T> -> A<T> = seed (copy = seed) => copy }";
    let library = parse(lex(file)).expect("original nested File source");
    let (seed_site, seed_read_site) = raw_file_formal_sites(&library);
    let files = vec![("one".into(), library)];
    for source in [
        "use one\n(one.A.f (one.A 7), one.A.f (one.A \"word\"))",
        "use one\n(one.A.f (one.A \"word\"), one.A.f (one.A 7))",
    ] {
        let module = parse(lex(source)).expect("original main");
        let unit = check::check(
            &module,
            &Declarations::new(),
            &files,
            check::Options::default(),
        )
        .expect("nested defaults");
        let tables = unit.tables();
        let namespace = tables
            .namespaces
            .iter()
            .position(|name| name == "one")
            .unwrap();
        let definition = tables
            .definitions
            .iter()
            .find(|definition| {
                definition.namespace == namespace && definition.label == "g"
            })
            .expect("canonical raw g definition");
        let mut node = definition.body;
        while let ir::Kind::Block(parts) = &tables.nodes[node].kind {
            let [part] = parts.as_slice() else {
                panic!("single raw g body")
            };
            node = *part;
        }
        let ir::Kind::Function { arms, .. } = &tables.nodes[node].kind else {
            panic!("raw g function")
        };
        let ir::PatternKind::Slot(seed_slot) = arms[0].patterns[0].kind else {
            panic!("actual first formal slot")
        };
        assert_eq!(tables.slots[seed_slot].name, Some((namespace, seed_site)));
        assert_eq!(tables.slots.iter().filter(|slot| slot.name == Some((namespace, seed_site))).count(), 1, "auxiliary snapshots cannot allocate/publish a duplicate raw formal");
        assert!(
            matches!(&tables.slots[seed_slot].ty, ir::Type::Meaning { arguments, .. } if arguments == &definition.parameters.iter().copied().map(ir::Type::Parameter).collect::<Vec<_>>())
        );
        let mut replay_reads = 0;
        for node in &tables.nodes {
            if node.namespace != namespace || node.span != seed_read_site {
                continue;
            }
            if let ir::Kind::Local { slot, .. }
            | ir::Kind::Capture { slot, .. } = &node.kind
            {
                assert_eq!(tables.slots[*slot].ty, node.ty, "default seed reads use the correctly typed supplied earlier-call slot");
                if *slot != seed_slot {
                    assert!(tables.nodes.iter().any(|call| matches!(&call.kind, ir::Kind::Call { slots, .. } if slots.first() == Some(slot))), "replay override is the actual first earlier-call argument slot");
                }
                replay_reads += 1;
            }
        }
        assert!(
            replay_reads > 0,
            "observe actual seed reads in replayed copy defaults"
        );
        drop(tables);
        let program = compile::compile(&unit);
        let outcome = Machine::new(Some(100_000)).run(&program);
        let expected = if source.contains("f (one.A 7),") {
            vec![Value::Int(7), Value::text("word")]
        } else {
            vec![Value::text("word"), Value::Int(7)]
        };
        assert!(
            matches!(outcome, Outcome::Done(Ok(Value::Tuple(ref values))) if values.as_slice() == expected),
            "{outcome:?}"
        );
    }
}

fn local_inherited_test_identities(
    unit: &fipe::ir::Unit,
    program: &fipe::mir::Program,
    source: &str,
) -> (usize, usize, usize, Vec<fipe::ir::Type>) {
    use fipe::ir;
    let tables = program.tables.borrow();
    let host = tables
        .namespaces
        .iter()
        .position(|name| name == "host")
        .unwrap();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let host_box = tables
        .meanings
        .iter()
        .position(|meaning| meaning.namespace == host && meaning.label == "Box")
        .unwrap();
    let main_box = tables
        .meanings
        .iter()
        .position(|meaning| meaning.namespace == main && meaning.label == "Box")
        .unwrap();
    assert_ne!(host_box, main_box);
    let test = |text: &str| {
        tables
            .tests
            .iter()
            .position(|test| {
                test.namespace == main
                    && source.get(test.span.start..test.span.end) == Some(text)
            })
            .expect("actual local test")
    };
    let base = test("self.len > 0");
    let derived = test("self.len < 3");
    let expected = vec![ir::Type::Meaning {
        index: host_box,
        arguments: vec![],
    }];
    let mut leaves = Vec::new();
    for entry in &unit.plan {
        let mut pending = vec![&entry.predicate];
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Test { index, meanings }
                    if [base, derived].contains(index) =>
                {
                    assert_eq!(meanings, &expected, "every local inherited/own leaf uses Host Box, never shadow main Box or prelude.T");
                    leaves.push(*index);
                }
                _ => (),
            }
        }
    }
    assert!(leaves.contains(&base) && leaves.contains(&derived));
    drop(tables);
    (base, derived, host_box, expected)
}

fn observe_host_nominal_tests(
    machine: &fipe::vm::Machine,
    tests: [usize; 2],
    expected: &[fipe::ir::Type],
    visited: &mut Vec<usize>,
) {
    use fipe::vm::Frame;
    let [base, derived] = tests;
    let joined = machine.program().expect("authoritative joined test bodies");
    for frame in machine.frames() {
        let Frame::Body(frame) = frame else { continue };
        for test in [base, derived] {
            if joined.tests[test] == Some(frame.body) {
                assert_eq!(
                    frame.meanings.as_slice(),
                    expected,
                    "actual local predicate receives Host nominal witness"
                );
                if !visited.contains(&test) {
                    visited.push(test);
                }
            }
        }
    }
}

fn assert_host_nominal_verdict(
    result: &Result<fipe::value::Value, fipe::vm::Fault>,
    nonempty: bool,
    calls: &[String],
    visited: &[usize],
    tests: [usize; 2],
) {
    use fipe::value::{Reason, Value};
    let [base, derived] = tests;
    if nonempty {
        assert_eq!(calls, ["items", "after"]);
        assert_eq!(visited, [base, derived]);
        assert_eq!(result, &Ok(Value::Int(7)));
    } else {
        assert_eq!(calls, ["items"]);
        assert_eq!(visited, [base]);
        assert!(
            matches!(
                result,
                Err(fipe::vm::Fault {
                    reason: Reason::Failed { .. },
                    ..
                })
            ),
            "Base refusal: {result:?}"
        );
    }
}

fn assert_returned_host_nominal_box(
    arguments: &[fipe::value::Value],
    host_box: usize,
) {
    use fipe::value::Value;
    let [Value::Seq(values)] = arguments else {
        panic!("checked sequence")
    };
    let [Value::Record(item)] = values.as_slice() else {
        panic!("returned Host Box")
    };
    assert_eq!(item.meaning, host_box);
    assert_eq!(item.fields, [Value::text("word")]);
}

#[test]
fn local_inherited_predicates_keep_host_nominal_arguments_before_effects() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::{Machine, Stop},
    };
    const HOST: &str = "pub type Box { pub tag: String }\npub items: a -> Vec<a>\npub after: Vec<a> -> Integer\n";
    const SOURCE: &str = "use host\ntype Box { n: Integer }\nshadow = Box 7\ntype Base<T>: Vec<T> where self.len > 0\ntype Derived<T>: Base<T> where self.len < 3\nxs: Derived<host.Box> = host.items (host.Box \"word\")\nhost.after xs\n";
    let mut declarations = Declarations::new();
    declarations
        .module("host", HOST)
        .expect("Host argument type");
    let module = parse(lex(SOURCE)).expect("local inherited declarations");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("local Base/Derived with actual Host Box");
    let program = compile::compile(&unit);
    let (base, derived, host_box, expected) =
        local_inherited_test_identities(&unit, &program, SOURCE);
    for nonempty in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        machine.begin(&program);
        let mut visited = Vec::new();
        let mut calls = Vec::new();
        let mut finished = false;
        for _ in 0..10_000 {
            match machine.next_stop() {
                Stop::Before(_) => {
                    observe_host_nominal_tests(
                        &machine,
                        [base, derived],
                        &expected,
                        &mut visited,
                    );
                }
                Stop::After(_, _) => (),
                Stop::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    let label = program.tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                    let reply = match label.as_str() {
                        "items" => {
                            assert!(calls.is_empty());
                            let [value @ Value::Record(item)] =
                                arguments.as_slice()
                            else {
                                panic!("actual checked Host Box argument")
                            };
                            assert_eq!(item.meaning, host_box);
                            assert_eq!(item.fields, [Value::text("word")]);
                            Value::seq(if nonempty {
                                vec![value.clone()]
                            } else {
                                vec![]
                            })
                        }
                        "after" => {
                            assert!(
                                nonempty,
                                "empty Base refuses before after"
                            );
                            assert_eq!(calls, ["items"]);
                            assert_eq!(visited, [base, derived]);
                            assert_returned_host_nominal_box(
                                &arguments, host_box,
                            );
                            Value::Int(7)
                        }
                        _ => panic!("unexpected Host effect {label}"),
                    };
                    calls.push(label);
                    machine.answer(Reply::Value(Ok(reply)));
                }
                Stop::Done(result) => {
                    assert_host_nominal_verdict(
                        &result,
                        nonempty,
                        &calls,
                        &visited,
                        [base, derived],
                    );
                    finished = true;
                    break;
                }
                stop @ Stop::Request(_) => panic!("unexpected stop {stop:?}"),
            }
        }
        assert!(finished, "bounded stepped proof terminates");
    }
}

#[test]
fn declared_generic_replies_check_scalar_and_structured_results_before_later_effects(
) {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::{Prelude, Value},
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub echo: a -> a\npub wrapped: a -> Maybe<a>\npub after: Integer -> Integer\npub after_maybe: Maybe<Integer> -> Maybe<Integer>").expect("generic reply contracts");
    for (source, target, after, structured) in [
        ("use host\nforward = x => host.echo x\nhost.after (forward 7)", "echo", "after", false),
        ("use host\nforward = x => host.wrapped x\nhost.after_maybe (forward 7)", "wrapped", "after_maybe", true),
    ] {
        let module = parse(lex(source)).expect("wrapper source");
        let unit = check::check(&module, &declarations, &[], check::Options::default()).expect("checked wrapper");
        let program = compile::compile(&unit);
        for wrong in [false, true] {
            let mut machine = Machine::new(Some(100_000));
            let Outcome::Request(Request::Call { declaration, arguments, .. }) = machine.run(&program) else { panic!("first generic request") };
            assert_eq!(program.tables.borrow().declarations[declaration].label, target);
            assert_eq!(arguments, vec![Value::Int(7)]);
            let reply = if wrong { Value::text("wrong") } else { Value::Int(7) };
            let reply = if structured { Prelude::new(&program.tables.borrow()).just(reply) } else { reply };
            let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
            if wrong {
                let expected = if structured { "the reply is not a Maybe<Integer>" } else { "the reply is not an Integer" };
                assert!(matches!(&outcome, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(message), .. })) if message == expected), "wrong generic result must fail before {after}: {outcome:?}");
            } else {
                let Outcome::Request(Request::Call { declaration, arguments, .. }) = outcome else { panic!("later effect after valid reply: {outcome:?}") };
                assert_eq!(program.tables.borrow().declarations[declaration].label, after);
                assert_eq!(arguments, vec![reply.clone()]);
                assert_eq!(machine.reply(&program, Reply::Value(Ok(reply.clone()))), Outcome::Done(Ok(reply)));
            }
        }
    }
}

#[test]
fn declared_reply_witnesses_survive_partial_and_returned_functions() {
    use fipe::{
        check, compile,
        decl::Declarations,
        embed::{Outcome, Reply, Request},
        lex::lex,
        parse::parse,
        value::Value,
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub echo: a -> a\npub choose: a -> a -> a\npub after: Integer -> Integer").expect("generic partial contracts");
    for (source, target, expected_args, result) in [
        ("use host\nforward = x y => host.choose x y\npartial = forward 7\nhost.after (partial 9)", "choose", vec![Value::Int(7), Value::Int(9)], 9),
        ("use host\nmake = _ => (x => host.echo x)\nf = make ()\nhost.after (f 7)", "echo", vec![Value::Int(7)], 7),
    ] {
        let module = parse(lex(source)).expect("partial/returned ordinary function");
        let unit = check::check(&module, &declarations, &[], check::Options::default()).expect("checked transport");
        let program = compile::compile(&unit);
        for wrong in [false, true] {
            let mut machine = Machine::new(Some(100_000));
            let Outcome::Request(Request::Call { declaration, arguments, .. }) = machine.run(&program) else { panic!("generic request after applying retained function") };
            assert_eq!(program.tables.borrow().declarations[declaration].label, target);
            assert_eq!(arguments, expected_args);
            let outcome = machine.reply(&program, Reply::Value(Ok(if wrong { Value::text("wrong") } else { Value::Int(result) })));
            if wrong {
                assert!(matches!(&outcome, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(message), .. })) if message == "the reply is not an Integer"), "{outcome:?}");
            } else {
                let Outcome::Request(Request::Call { declaration, arguments, .. }) = outcome else { panic!("after valid function reply: {outcome:?}") };
                assert_eq!(program.tables.borrow().declarations[declaration].label, "after");
                assert_eq!(arguments, vec![Value::Int(result)]);
                assert_eq!(machine.reply(&program, Reply::Value(Ok(Value::Int(result)))), Outcome::Done(Ok(Value::Int(result))));
            }
        }
    }
}

// Historical generic written-member programs, preserved without rewriting.
const HOST41_LIBRARY: &str = "use effects\npub type Probe where self.inspect: Integer\npub type Box<T>: Probe { pub value: T }\nimpl Box<T>: Probe { inspect: Self -> Integer = self => effects.tick }\npub factory: Box<Integer> -> Integer = value => value.inspect";
const HOST41_DECLARED: &str = "pub type Probe where self.inspect: Integer\npub type Box<T>: Probe { pub value: T }\nimpl Box<T>: Probe { inspect: (self: Self) -> Integer }";
const HOST41_DIRECT: &str =
    "use probe\nx: probe.Box<Integer> = probe.Box 1\nx.inspect";
const HOST41_PACKED: &str = "use probe\ninspect: probe.Probe -> Integer = value => value.inspect\nx: probe.Probe = probe.Box 1\ninspect x";
const HOST41_FACTORY: &str = "use probe\nprobe.factory (probe.Box 1)";

#[derive(Clone, Copy, Debug)]
enum Host41Origin {
    Host,
    File,
    Local,
    Declared,
}
fn host41_program(origin: Host41Origin, source: &str) -> fipe::mir::Program {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("effects", "pub tick: Integer").unwrap();
    let files = match origin {
        Host41Origin::Host => {
            declarations.module("probe", HOST41_LIBRARY).unwrap();
            Vec::new()
        }
        Host41Origin::Declared => {
            declarations.module("probe", HOST41_DECLARED).unwrap();
            Vec::new()
        }
        Host41Origin::File => vec![(
            "probe".into(),
            fipe::parse::parse(fipe::lex::lex(HOST41_LIBRARY)).unwrap(),
        )],
        Host41Origin::Local => Vec::new(),
    };
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let unit = fipe::check::check(
        &module,
        &declarations,
        &files,
        fipe::check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{origin:?}: {errors:?}"));
    fipe::compile::compile(&unit)
}
#[derive(Clone, Copy, Debug)]
enum Host41Route {
    Direct,
    Packed,
    Factory,
}
fn host41_definition(
    program: &fipe::mir::Program,
    namespace: &str,
    label: &str,
    origin: Host41Origin,
    route: Host41Route,
) -> (usize, usize) {
    let tables = program.tables.borrow();
    let definitions: Vec<_> = tables
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(index, definition)| {
            (definition.label == label
                && tables.namespaces[definition.namespace] == namespace)
                .then_some(index)
        })
        .collect();
    let [index] = definitions.as_slice() else {
        panic!("{origin:?}/{route:?}: exact {namespace}.{label} definition");
    };
    let body = program.definitions[*index].unwrap_or_else(|| {
        panic!("{origin:?}/{route:?}: compiled {namespace}.{label} body")
    });
    assert!(body < program.bodies.len(), "{origin:?}/{route:?}");
    (*index, body)
}
fn host41_call(
    program: &fipe::mir::Program,
    body_index: usize,
    callee: &fipe::mir::Callee,
    origin: Host41Origin,
    route: Host41Route,
) {
    assert!(
        program.bodies[body_index].blocks.iter().any(|block| {
            matches!(&block.terminator,
                fipe::mir::Terminator::Call { callee: actual, .. }
                    if actual == callee)
        }),
        "{origin:?}/{route:?}: actual MIR call {body_index} -> {callee:?}"
    );
}
fn host41_packed_rows(
    program: &fipe::mir::Program,
    meaning: usize,
    definition: usize,
    origin: Host41Origin,
    route: Host41Route,
) {
    let tables = program.tables.borrow();
    let targets: Vec<_> = tables
        .dispatch
        .iter()
        .filter(|dispatch| dispatch.label == "inspect")
        .flat_map(|dispatch| &dispatch.rows)
        .filter(|row| {
            matches!(&row.receiver,
                fipe::ir::Type::Meaning { index, .. } if *index == meaning)
        })
        .map(|row| &row.target)
        .collect();
    assert!(
        !targets.is_empty(),
        "{origin:?}/{route:?}: actual dynamic Box inspect row"
    );
    for target in targets {
        assert!(
            matches!(target,
                fipe::ir::Implementation::Definition { index, .. }
                    if *index == definition),
            "{origin:?}/{route:?}: row -> inspect definition: {target:?}"
        );
    }
}
fn host41_selection(
    program: &fipe::mir::Program,
    origin: Host41Origin,
    route: Host41Route,
) -> usize {
    let tables = program.tables.borrow();
    let namespace = if matches!(origin, Host41Origin::Local) {
        "main"
    } else {
        "probe"
    };
    let meanings: Vec<_> = tables
        .meanings
        .iter()
        .enumerate()
        .filter_map(|(index, meaning)| {
            (meaning.label == "Box"
                && tables.namespaces[meaning.namespace] == namespace)
                .then_some(index)
        })
        .collect();
    let [meaning] = meanings.as_slice() else {
        panic!("{origin:?}/{route:?}: exact Box owner");
    };
    if matches!(origin, Host41Origin::Declared) {
        let declared: Vec<_> = tables
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                (declaration.label == "inspect"
                    && tables.namespaces[declaration.namespace] == "probe")
                    .then_some(index)
            })
            .collect();
        let [declared] = declared.as_slice() else {
            panic!("{origin:?}/{route:?}: actual supplied inspect");
        };
        host41_call(
            program,
            program.main,
            &fipe::mir::Callee::Host(*declared),
            origin,
            route,
        );
    } else {
        let (definition, body) =
            host41_definition(program, namespace, "inspect", origin, route);
        match route {
            Host41Route::Direct => host41_call(
                program,
                program.main,
                &fipe::mir::Callee::Body(body),
                origin,
                route,
            ),
            Host41Route::Factory => {
                let (_, factory) = host41_definition(
                    program, namespace, "factory", origin, route,
                );
                host41_call(
                    program,
                    program.main,
                    &fipe::mir::Callee::Body(factory),
                    origin,
                    route,
                );
                host41_call(
                    program,
                    factory,
                    &fipe::mir::Callee::Body(body),
                    origin,
                    route,
                );
            }
            Host41Route::Packed => {
                host41_packed_rows(
                    program, *meaning, definition, origin, route,
                );
            }
        }
    }
    *meaning
}
fn host41_observe(origin: Host41Origin, source: &str, route: Host41Route) {
    let mut callbacks = 0;
    // No Machine exists before public registration/checking/compilation finish.
    let program = host41_program(origin, source);
    assert_eq!(callbacks, 0);
    let mut machine = fipe::vm::Machine::new(Some(100_000));
    let fipe::embed::Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("{origin:?}: {route:?}: expected exactly one callback");
    };
    let joined = machine.program().unwrap();
    let meaning = host41_selection(joined, origin, route);
    let tables = joined.tables.borrow();
    let declaration = &tables.declarations[declaration];
    if matches!(origin, Host41Origin::Declared) {
        assert_eq!(tables.namespaces[declaration.namespace], "probe");
        assert_eq!(declaration.label, "inspect");
        let [fipe::value::Value::Record(receiver)] = arguments.as_slice()
        else {
            panic!("actual supplied receiver");
        };
        assert_eq!(receiver.meaning, meaning);
        assert_eq!(receiver.fields, [fipe::value::Value::Int(1)]);
    } else {
        assert_eq!(tables.namespaces[declaration.namespace], "effects");
        assert_eq!(declaration.label, "tick");
        assert!(arguments.is_empty());
    }
    drop(tables);
    callbacks += 1;
    assert_eq!(
        machine.reply(
            &program,
            fipe::embed::Reply::Value(Ok(fipe::value::Value::Int(41)))
        ),
        fipe::embed::Outcome::Done(Ok(fipe::value::Value::Int(41)))
    );
    assert_eq!(callbacks, 1);
}
#[test]
fn original_generic_host_written_members_execute_direct_packed_and_factory() {
    for origin in [Host41Origin::Host, Host41Origin::File] {
        host41_observe(origin, HOST41_DIRECT, Host41Route::Direct);
        host41_observe(origin, HOST41_PACKED, Host41Route::Packed);
        host41_observe(origin, HOST41_FACTORY, Host41Route::Factory);
    }
    let local = format!(
        "{}\nx: Box<Integer> = Box 1\nx.inspect",
        HOST41_LIBRARY.replace("pub ", "")
    );
    host41_observe(Host41Origin::Local, &local, Host41Route::Direct);
    host41_observe(Host41Origin::Declared, HOST41_DIRECT, Host41Route::Direct);
}

// Finite original same-Host reentry controls, distinct from check-only cycles.
const REENTRY_TERMINATING: &str = "use effects\npub type A { pub n: Integer }\npub type B { pub n: Integer }\nimpl A {\n f: Integer -> Integer = n => {\n  ignored = effects.visit n\n  if n == 0 { effects.tick } else { B.g (n - 1) }\n }\n}\nimpl B {\n g: Integer -> Integer = n => {\n  ignored = effects.visit (100 + n)\n  A.f n\n }\n}\n";
const REENTRY_COMPATIBLE: &str = "use effects\npub type A { pub n: Integer }\npub type B { pub n: Integer }\nimpl A {\n f = n x => {\n  ignored = effects.visit n\n  if n == 0 { x } else { B.g (n - 1) x }\n }\n}\nimpl B {\n g = n x => {\n  ignored = effects.visit (100 + n)\n  first = A.f 0 \"a\"\n  second = A.f 0 \"b\"\n  A.f n x\n }\n}\n";

fn reentry_inputs(
    header: &str,
    source: &str,
) -> (
    fipe::decl::Declarations,
    fipe::ast::Module,
    [fipe::token::Span; 2],
) {
    use fipe::ast::{Entry, ImplMember, ItemKind, TypeKind};
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("effects", "pub visit: Integer -> ()\npub tick: Integer")
        .expect("effect declarations");
    declarations
        .module("probe", header)
        .expect("actual Host header");
    let module = &declarations.get_module("probe").unwrap().module;
    let mut sites = Vec::new();
    for (owner, member) in [("A", "f"), ("B", "g")] {
        let mut found = Vec::new();
        for entry in &module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl {
                target, members, ..
            } = &item.kind
            else {
                continue;
            };
            let TypeKind::Named { name, .. } =
                &module.ty(*target).expect("actual impl target").kind
            else {
                continue;
            };
            if name.segments.as_slice() != [owner] {
                continue;
            }
            for entry in members {
                if let ImplMember::Named {
                    name,
                    value: Some(_),
                    ..
                } = entry
                {
                    if name.segments.as_slice() == [member] {
                        found.push(name.span);
                    }
                }
            }
        }
        let [site] = found.as_slice() else {
            panic!("one actual written {owner}.{member} name");
        };
        sites.push(*site);
    }
    let sites: [_; 2] = sites.try_into().unwrap();
    let main = fipe::parse::parse(fipe::lex::lex(source)).expect("main parses");
    (declarations, main, sites)
}

fn reentry_bodies(
    program: &fipe::mir::Program,
    sites: [fipe::token::Span; 2],
    header_len: usize,
) {
    let tables = program.tables.borrow();
    for (label, site) in ["f", "g"].into_iter().zip(sites) {
        let found: Vec<_> = tables
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(index, definition)| {
                (definition.label == label
                    && definition.span == site
                    && tables.namespaces[definition.namespace] == "probe")
                    .then_some((index, definition))
            })
            .collect();
        let [(index, definition)] = found.as_slice() else {
            panic!("one actual checked Host {label} definition at {site:?}");
        };
        let body = program.definitions[*index]
            .expect("actual joined written function body");
        let compiled = &program.bodies[body];
        assert!(compiled.arity > 0);
        let source = &tables.nodes[definition.body];
        assert_eq!(tables.namespaces[source.namespace], "probe");
        assert!(source.span.start < source.span.end);
        assert!(source.span.end <= header_len);
        let lowered = &tables.nodes[compiled.node];
        assert_eq!(tables.namespaces[lowered.namespace], "probe");
        assert!(lowered.span.start < lowered.span.end);
        assert!(lowered.span.end <= header_len);
    }
}

fn reentry_observe(
    header: &str,
    source: &str,
    visits: &[i64],
    tick: bool,
    expected: fipe::value::Value,
) {
    use fipe::embed::{Outcome, Reply, Request};
    use fipe::value::Value;
    let (declarations, main, sites) = reentry_inputs(header, source);
    let mut callbacks = 0;
    let unit = fipe::check::check(
        &main,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    let program = fipe::compile::compile(&unit);
    assert_eq!(callbacks, 0, "no Machine/callback before public checking");
    let mut machine = fipe::vm::Machine::new(Some(100_000));
    let mut outcome = machine.run(&program);
    assert!(matches!(&outcome, Outcome::Request(Request::Call { .. })));
    reentry_bodies(machine.program().unwrap(), sites, header.len());
    for expected in visits {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("visit {expected}: {outcome:?}");
        };
        {
            let tables = machine.program().unwrap().tables.borrow();
            let declared = &tables.declarations[declaration];
            assert_eq!(tables.namespaces[declared.namespace], "effects");
            assert_eq!(declared.label, "visit");
        }
        assert_eq!(arguments, [Value::Int(*expected)]);
        callbacks += 1;
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Unit)));
    }
    if tick {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        else {
            panic!("one terminal tick: {outcome:?}");
        };
        {
            let tables = machine.program().unwrap().tables.borrow();
            let declared = &tables.declarations[declaration];
            assert_eq!(tables.namespaces[declared.namespace], "effects");
            assert_eq!(declared.label, "tick");
        }
        assert!(arguments.is_empty());
        callbacks += 1;
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Int(41))));
    }
    assert_eq!(outcome, Outcome::Done(Ok(expected)));
    assert_eq!(callbacks, visits.len() + usize::from(tick));
}

#[test]
fn same_host_written_reentry_terminates_after_actual_a_b_a_calls() {
    reentry_observe(
        REENTRY_TERMINATING,
        "use probe\nprobe.A.f 1",
        &[1, 100, 0],
        true,
        fipe::value::Value::Int(41),
    );
}

#[test]
fn same_host_pending_member_compatible_uses_execute_one_shared_source() {
    reentry_observe(
        REENTRY_COMPATIBLE,
        "use probe\nprobe.A.f 1 \"done\"",
        &[1, 100, 0, 0, 0],
        false,
        fipe::value::Value::text("done"),
    );
}

#[test]
fn same_host_pending_member_incompatible_use_is_refused_before_execution() {
    let from = "second = A.f 0 \"b\"";
    assert_eq!(REENTRY_COMPATIBLE.matches(from).count(), 1);
    let header = REENTRY_COMPATIBLE.replace(from, "second = A.f 0 True");
    let (declarations, main, sites) =
        reentry_inputs(&header, "use probe\nprobe.A.f 1 \"done\"");
    let callbacks = 0;
    let errors = fipe::check::check(
        &main,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .expect_err("incompatible uses must not freshen pending A.f");
    assert!(
        errors.iter().any(|error| matches!(
            error,
            fipe::check::Error::Type(fipe::infer::Error {
                span,
                reason: fipe::infer::Reason::DeclarationValue {
                    name, origin, declaration,
                },
            }) if *span == main.span && name == "A"
                && *origin == fipe::resolve::Origin::Host("probe".into())
                && *declaration == sites[0]
        )),
        "predicted public Host failure, preserving all diagnostics: {errors:?}"
    );
    assert_eq!(callbacks, 0, "no Machine/reply on checker refusal");
}
