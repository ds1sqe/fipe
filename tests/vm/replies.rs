//! A host's reply is checked against the type its call declared: a
//! value of another shape Faults `Host` at the call, before the run
//! uses it.
use super::{check, lex, parse, run::BUDGET, Declarations, Machine, Outcome};
use fipe::{
    compile,
    embed::Reply,
    ir::Tables,
    mir::Program,
    value::{Prelude, Reason, Record, Value},
    vm::{Fault, Stop},
};
use std::rc::Rc;

/// A host module with one declaration per shape.
const PROBE: &str = "\
## A pair of numbers.
pub pair: Integer * Integer
## Some numbers.
pub numbers: Vec<Integer>
## A number, maybe.
pub found: Maybe<Integer>
## Three bytes.
pub bytes: [U8; 3]
## A point.
pub type Point {
    ## Across.
    pub x: Integer,
    ## Up.
    pub y: F64,
}
## The point asked for.
pub point: Point
## Points, maybe.
pub points: Vec<Maybe<Point>>
## A handle the host admits.
pub type Handle
## The handle asked for.
pub handle: Handle
## A handle's number.
pub id: Handle -> Integer
";

fn program(source: &str) -> Program {
    let mut declarations = Declarations::new();
    declarations.module("probe", PROBE).unwrap();
    let source = format!("use probe\n{source}");
    let module = parse(lex(&source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    compile::compile(&unit)
}

/// The run of `source`, each host call answered by `replies` in turn.
fn run(
    source: &str,
    replies: impl Fn(&Tables) -> Vec<Value>,
) -> Result<Value, Fault> {
    let program = program(source);
    let mut replies = replies(&program.tables.borrow()).into_iter();
    let mut machine = Machine::new(Some(BUDGET));
    let mut outcome = machine.run(&program);
    loop {
        match outcome {
            Outcome::Done(result) => return result,
            Outcome::Request(_) => {
                let reply = replies.next().expect("a reply for each call");
                outcome = machine.reply(&program, Reply::Value(Ok(reply)));
            }
        }
    }
}

fn meaning(tables: &Tables, label: &str) -> usize {
    tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == label)
        .unwrap()
}

fn point(tables: &Tables, fields: Vec<Value>) -> Value {
    Value::Record(Rc::new(Record {
        meaning: meaning(tables, "Point"),
        constructor: 0,
        fields,
    }))
}

/// The Fault's reason, when it is the host's, and where it is.
fn misfit(result: Result<Value, Fault>) -> (String, (usize, usize)) {
    match result {
        Err(Fault {
            reason: Reason::Host(message),
            span,
            ..
        }) => (message, (span.start, span.end)),
        other => panic!("a Host Fault, not {other:?}"),
    }
}

/// `probe.name` on the second line, after `use probe` and its newline.
const fn call_site(name: &str) -> (usize, usize) {
    (10, 10 + "probe.".len() + name.len())
}

#[test]
fn a_reply_of_another_primitive_faults_at_the_call() {
    let result = run("probe.pair", |_| vec![Value::text("two")]);
    assert_eq!(
        misfit(result),
        (
            String::from("the reply is not an Integer * Integer"),
            call_site("pair")
        )
    );
    let result = run("probe.found", |_| vec![Value::F64(1.0)]);
    assert_eq!(misfit(result).0, "the reply is not a Maybe<Integer>");
}

#[test]
fn a_tuple_or_array_of_another_length_faults() {
    let three = Value::Tuple(Rc::new(vec![Value::Int(1); 3]));
    let result = run("probe.pair", |_| vec![three.clone()]);
    assert_eq!(misfit(result).0, "the reply is not an Integer * Integer");
    let result = run("probe.bytes", |_| vec![Value::seq(vec![Value::Int(1)])]);
    assert_eq!(misfit(result).0, "the reply is not a [Integer; 3]");
}

#[test]
fn a_record_of_another_meaning_faults() {
    // The second line names `Point`; the first line's reply Faults.
    let result = run("probe.found\nprobe.point", |tables| {
        vec![point(tables, vec![Value::Int(1), Value::F64(2.0)])]
    });
    assert_eq!(misfit(result).0, "the reply is not a Maybe<Integer>");
    let result = run("probe.point", |tables| {
        vec![point(tables, vec![Value::Int(1)])]
    });
    assert_eq!(misfit(result).0, "the reply is not a Point", "one field");
}

#[test]
fn a_constructor_out_of_range_faults() {
    let result = run("probe.found", |tables| {
        vec![Value::Record(Rc::new(Record {
            meaning: meaning(tables, "Maybe"),
            constructor: 2,
            fields: Vec::new(),
        }))]
    });
    assert_eq!(misfit(result).0, "the reply is not a Maybe<Integer>");
}

#[test]
fn a_misfit_deep_inside_a_reply_faults() {
    let result = run("probe.numbers", |_| {
        vec![Value::seq(vec![Value::Int(1), Value::text("2")])]
    });
    assert_eq!(misfit(result).0, "the reply is not a Vec<Integer>");
    let result = run("probe.points", |tables| {
        let prelude = Prelude::new(tables);
        let good = point(tables, vec![Value::Int(1), Value::F64(2.0)]);
        let bad = point(tables, vec![Value::Int(1), Value::Int(2)]);
        vec![Value::seq(vec![prelude.just(good), prelude.just(bad)])]
    });
    assert_eq!(misfit(result).0, "the reply is not a Vec<Maybe<Point>>");
}

#[test]
fn a_reply_of_the_declared_shape_is_used() {
    let pair = Value::Tuple(Rc::new(vec![Value::Int(1), Value::Int(2)]));
    assert_eq!(run("probe.pair", |_| vec![pair.clone()]), Ok(pair));
    let big = Value::integer(false, fipe::token::Magnitude::from(u64::MAX));
    let numbers = Value::seq(vec![Value::Int(1), big]);
    assert_eq!(run("probe.numbers", |_| vec![numbers.clone()]), Ok(numbers));
    let bytes = Value::bytes(&[1, 2, 3]);
    assert_eq!(run("probe.bytes", |_| vec![bytes.clone()]), Ok(bytes));
    let result = run("(probe.point).y", |tables| {
        vec![point(tables, vec![Value::Int(1), Value::F64(2.5)])]
    });
    assert_eq!(result, Ok(Value::F64(2.5)));
    let result =
        run("probe.found", |tables| vec![Prelude::new(tables).nothing()]);
    assert!(result.is_ok());
}

#[test]
fn an_opaque_type_takes_whatever_value_its_host_chooses() {
    let result = run("probe.id probe.handle", |_| {
        vec![Value::text("handle 7"), Value::Int(7)]
    });
    assert_eq!(result, Ok(Value::Int(7)));
}

#[test]
fn a_stepped_run_checks_its_replies_alike() {
    let program = program("probe.pair");
    let mut machine = Machine::new(Some(BUDGET));
    machine.begin(&program);
    let result = loop {
        match machine.next_stop() {
            Stop::Request(_) => machine.answer(Reply::Value(Ok(Value::Unit))),
            Stop::Done(result) => break result,
            Stop::Before(_) | Stop::After(..) => {}
        }
    };
    assert_eq!(misfit(result).0, "the reply is not an Integer * Integer");
}

const WRITTEN_HOST: &str = "pub type Table\npub get: Table\npub type Wrap { pub value: Table }\nimpl Wrap { () => Self { value: get } }";

fn written_program(
    declarations: &Declarations,
    source: &str,
    files: &[(String, fipe::ast::Module)],
) -> Program {
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, declarations, files, check::Options::default())
            .unwrap();
    compile::compile(&unit)
}

#[test]
fn supplied_written_constructor_keeps_host_effects_inside_the_call() {
    let mut declarations = Declarations::new();
    declarations.commands(WRITTEN_HOST).unwrap();
    let unused = written_program(&declarations, "f = _ => Wrap ()", &[]);
    let mut machine = Machine::new(Some(BUDGET));
    assert!(matches!(machine.run(&unused), Outcome::Done(Ok(_))));
    let program = written_program(&declarations, "Wrap ()", &[]);
    for denied in [false, true] {
        let mut machine = Machine::new(Some(BUDGET));
        let Outcome::Request(fipe::embed::Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request")
        };
        let tables = program.tables.borrow();
        assert_eq!(tables.declarations[declaration].label, "get");
        assert_eq!(
            tables.namespaces[tables.declarations[declaration].namespace],
            "commands"
        );
        assert!(arguments.is_empty());
        drop(tables);
        let reply = if denied {
            Err("denied".to_owned())
        } else {
            Ok(Value::Int(42))
        };
        match machine.reply(&program, Reply::Value(reply)) {
            Outcome::Done(Ok(Value::Record(record))) if !denied => {
                assert_eq!(record.fields, [Value::Int(42)]);
            }
            Outcome::Done(Err(fault)) if denied => {
                assert_eq!(fault.reason, Reason::Host("denied".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn imported_written_constructor_executes_body_instead_of_raw_fields() {
    let file = "pub type Point { pub x: Integer }\nimpl Point { x => Self { x: x + 1 } }\npub make: Integer -> Point = x => Point x";
    let files = [("point".to_owned(), parse(lex(file)).unwrap())];
    for source in ["use point\npoint.Point 1", "use point\npoint.make 1"] {
        let program = written_program(&Declarations::new(), source, &files);
        let mut machine = Machine::new(Some(BUDGET));
        let Outcome::Done(Ok(Value::Record(record))) = machine.run(&program)
        else {
            panic!("Point result")
        };
        assert_eq!(record.fields, [Value::Int(2)]);
    }
}

#[test]
fn supplied_constructor_cache_survives_session_boundaries_and_forks() {
    let mut declarations = Declarations::new();
    declarations.commands("pub type Wrap { pub value: Integer }\nimpl Wrap { x => Self { value: x + 1 } }").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for source in ["f = x => Wrap x", "f 2", "Wrap 3"] {
        let Outcome::Done(report) = session.line(source) else {
            panic!("checked line")
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(report.unit.is_some());
    }
    let mut fork = session.clone();
    for session in [&mut session, &mut fork] {
        let Outcome::Done(report) = session.line("Wrap 4") else {
            panic!("checked line")
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let program = compile::compile(report.unit.as_ref().unwrap());
        let mut machine = Machine::new(Some(BUDGET));
        let Outcome::Done(Ok(Value::Record(record))) = machine.run(&program)
        else {
            panic!("Wrap result")
        };
        assert_eq!(record.fields, [Value::Int(5)]);
    }
}

#[test]
fn supplied_constructor_identity_includes_namespace_and_generics() {
    let mut declarations = Declarations::new();
    for (name, offset) in [("left", 1), ("right", 10)] {
        declarations.module(name, &format!("pub type Wrap {{ pub value: Integer }}\nimpl Wrap {{ x => Self {{ value: x + {offset} }} }}")).unwrap();
    }
    let program =
        written_program(&declarations, "(left.Wrap 2, right.Wrap 2)", &[]);
    let mut machine = Machine::new(Some(BUDGET));
    let Outcome::Done(Ok(Value::Tuple(values))) = machine.run(&program) else {
        panic!("two wrappers")
    };
    let [Value::Record(left), Value::Record(right)] = values.as_slice() else {
        panic!("records")
    };
    assert_ne!(left.meaning, right.meaning);
    assert_eq!(left.fields, [Value::Int(3)]);
    assert_eq!(right.fields, [Value::Int(12)]);
    declarations.commands("pub type Box<T> { pub value: T }\nimpl Box<T> { x => Self { value: x } }").unwrap();
    let program = written_program(&declarations, "(Box 2, Box \"text\")", &[]);
    let mut machine = Machine::new(Some(BUDGET));
    let Outcome::Done(Ok(Value::Tuple(values))) = machine.run(&program) else {
        panic!("generic boxes")
    };
    let [Value::Record(integer), Value::Record(text)] = values.as_slice()
    else {
        panic!("records")
    };
    assert_eq!(integer.fields, [Value::Int(2)]);
    assert_eq!(text.fields, [Value::text("text")]);
}

#[test]
fn withdrawn_file_constructor_is_rechecked_when_its_source_returns() {
    let mut session = fipe::session::Session::new(Declarations::new());
    for offset in [1, 10] {
        let Outcome::Request(fipe::embed::Request::File { stem }) =
            session.line("use point\npoint.Point 2")
        else {
            panic!("file request")
        };
        assert_eq!(stem, "point");
        let text = format!(
            "pub type Point {{ pub x: Integer }}\nimpl Point {{ x => Self {{ x: x + {offset} }} }}"
        );
        let Outcome::Done(report) = session.reply(Reply::File(Some(text)))
        else {
            panic!("checked line")
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let program = compile::compile(report.unit.as_ref().unwrap());
        let mut machine = Machine::new(Some(BUDGET));
        let Outcome::Done(Ok(Value::Record(record))) = machine.run(&program)
        else {
            panic!("Point result")
        };
        assert_eq!(record.fields, [Value::Int(2 + offset)]);
        assert!(session.withdraw(report.accepted.unwrap()).is_some());
    }
}

#[test]
fn recursive_supplied_constructor_dependency_survives_withdrawal() {
    for (declarations, first, later) in [
        (
            "pub type A { pub value: Integer }\npub type B { pub value: Integer }\nimpl A { x => Self { value: (B x).value } }\nimpl B { x => Self { value: (A x).value } }",
            "f = _ => A 1",
            "g = _ => B 1",
        ),
        (
            "pub type A<T> { pub value: T }\npub type B<T> { pub value: T }\nimpl A<T> { x => Self { value: (B x).value } }\nimpl B<T> { x => Self { value: (A x).value } }",
            "f = x => A x",
            "g = _ => (B 1, B \"text\")",
        ),
    ] {
        let mut declarations_host = Declarations::new();
        declarations_host.commands(declarations).unwrap();
        let mut session = fipe::session::Session::new(declarations_host);
        let Outcome::Done(first) = session.line(first) else {
            panic!("first line")
        };
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        assert!(first.unit.is_some());
        assert!(session.withdraw(first.accepted.unwrap()).is_some());
        let Outcome::Done(second) = session.line(later) else {
            panic!("second line")
        };
        assert!(second.diagnostics.is_empty(), "{:?}", second.diagnostics);
        assert!(second.unit.is_some());
    }
}
