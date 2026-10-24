//! Genuine checked resources stay opaque, including recorded replies.
use fipe::{
    check, compile,
    decl::Declarations,
    embed::{Outcome, Reply},
    lex, parse,
    value::{
        resource::{ResourceDomain, ResourceType},
        Value,
    },
    vm::Machine,
};
use fipe_dap::{
    inspect::{parts, Item, References},
    replay::{self, Replay},
};
use std::rc::Rc;

fn resource_program() -> fipe::mir::Program {
    let mut host = Declarations::new();
    host.commands("pub type resource Table\npub acquire: () -> Table\npub type Packet\npub packet: Packet\npub type ResourceBox { pub value: Table }\npub boxed: ResourceBox").unwrap();
    let module = parse::parse(lex::lex("(acquire (), packet, boxed)")).unwrap();
    compile::compile(
        &check::check(&module, &host, &[], check::Options::default()).unwrap(),
    )
}

#[test]
fn resource_tool_inspection_is_opaque_inside_useful_containers() {
    let program = resource_program();
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let ty = machine.pending_resource_type().unwrap();
    assert_eq!(
        ty,
        ResourceType::registered(&program.tables.borrow(), "commands", "Table")
            .unwrap()
    );
    let value = ResourceDomain::new()
        .issue(&ty, Rc::from(&b"SECRET-resource-tool-key"[..]));
    let tables = program.tables.borrow();
    assert!(parts(&value, &tables).is_empty());
    let mut references = References::default();
    let shown =
        references.variable("table", &Item::Value(value.clone()), &tables);
    assert_eq!(shown["value"], "<resource>");
    assert_eq!(shown["variablesReference"], 0);
    for outer in [
        Value::Tuple(Rc::new(vec![Value::Int(7), value.clone()])),
        Value::Seq(Rc::new(vec![Value::Int(7), value.clone()])),
    ] {
        let shown = references.variable("outer", &Item::Value(outer), &tables);
        let reference = shown["variablesReference"].as_u64().unwrap();
        assert_ne!(reference, 0);
        let children = references.variables(reference, &tables).unwrap();
        assert_eq!(children["variables"][0]["value"], "7");
        assert_eq!(children["variables"][1]["value"], "<resource>");
        assert_eq!(children["variables"][1]["variablesReference"], 0);
        assert!(!children.to_string().contains("SECRET"));
    }
    let meaning = tables
        .meanings
        .iter()
        .position(|m| m.label == "ResourceBox")
        .unwrap();
    assert_eq!(
        tables.meanings[meaning].constructors[0].fields[0].label,
        "value"
    );
    let boxed = Value::Record(Rc::new(fipe::value::Record {
        meaning,
        constructor: 0,
        fields: vec![value.clone()],
    }));
    let shown = references.variable("box", &Item::Value(boxed), &tables);
    let children = references
        .variables(shown["variablesReference"].as_u64().unwrap(), &tables)
        .unwrap();
    assert_eq!(children["variables"][0]["name"], "value");
    assert_eq!(children["variables"][0]["value"], "<resource>");
    assert_eq!(children["variables"][0]["variablesReference"], 0);
    assert_eq!(format!("{value:?}"), "Resource(<resource>)");
    assert!(!format!("{ty:?}").contains("SECRET"));
}

#[test]
fn resource_tool_replay_cannot_construct_marked_types() {
    let program = resource_program();
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let tables = program.tables.borrow();
    assert!(ResourceType::registered(&tables, "commands", "Table").is_ok());
    for spelling in ["<resource>", "Table"] {
        assert!(replay::read(spelling, &tables).is_err());
        let mut recorded =
            Replay::parse(&format!("call commands.acquire () -> {spelling}"))
                .unwrap();
        assert!(recorded.answer("commands.acquire", &tables).is_err());
    }
    let packet = replay::read("Packet", &tables).unwrap();
    assert!(matches!(packet, Value::Record(_)));
    assert_eq!(
        replay::read("(7, [1, 2])", &tables).unwrap(),
        Value::Tuple(Rc::new(vec![
            Value::Int(7),
            Value::Seq(Rc::new(vec![Value::Int(1), Value::Int(2)]))
        ]))
    );
    let meaning = tables
        .meanings
        .iter()
        .position(|m| m.resource && m.label == "Table")
        .unwrap();
    let forged = Value::Record(Rc::new(fipe::value::Record {
        meaning,
        constructor: 0,
        fields: Vec::new(),
    }));
    drop(tables);
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(forged))),
        Outcome::Done(Err(_))
    ));
    let mut refused = Machine::new(None);
    assert!(matches!(refused.run(&program), Outcome::Request(_)));
    assert!(matches!(
        refused.reply(
            &program,
            Reply::Value(
                Err("replay: resource cannot be reconstructed".into())
            )
        ),
        Outcome::Done(Err(_))
    ));
}
