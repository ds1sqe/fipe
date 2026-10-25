//! Existing-API feasibility only: fresh trusted types, not binding migration.
use fipe::{
    compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    session::Session,
    token::Span,
    value::{
        resource::{ResourceDomain, ResourceType},
        Reason, Value,
    },
    vm::{Fault, Machine},
};
use std::{cell::Cell, rc::Rc};

const A_HEADER: &str = "pub type resource BaseJson\npub load: () -> BaseJson";
const B_HEADER: &str = "pub type resource JsonRowA\npub restore: () -> JsonRowA\npub after: () -> Integer\nimpl JsonRowA { name: (self: Self) -> String,\nage: (self: Self) -> Integer }";
const KEY: &[u8] = b"json-entry-1";

struct Payload {
    name: String,
    age: i64,
    drops: Rc<Cell<usize>>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct PublishedSchema {
    epoch: u64,
    fields: Vec<(&'static str, &'static str)>,
}
struct Owner {
    domain: ResourceDomain,
    payload: Option<Payload>,
    drops: Rc<Cell<usize>>,
    loads: usize,
    inspections: usize,
    getters: usize,
    restores: usize,
    grant: bool,
    epoch: u64,
}
impl Owner {
    fn new() -> Self {
        Self {
            domain: ResourceDomain::new(),
            payload: None,
            drops: Rc::new(Cell::new(0)),
            loads: 0,
            inspections: 0,
            getters: 0,
            restores: 0,
            grant: true,
            epoch: 1,
        }
    }
    fn load(&mut self, ty: &ResourceType) -> Value {
        assert!(self.payload.is_none());
        self.loads += 1;
        self.payload = Some(Payload {
            name: "Ada".into(),
            age: 37,
            drops: self.drops.clone(),
        });
        self.domain.issue(ty, Rc::from(KEY))
    }
    fn publish(&mut self) -> PublishedSchema {
        assert!(self.grant && self.payload.is_some());
        self.inspections += 1;
        PublishedSchema {
            epoch: self.epoch,
            fields: vec![("name", "String"), ("age", "Integer")],
        }
    }
    fn restore(
        &mut self,
        ty: &ResourceType,
        epoch: u64,
    ) -> Result<Value, String> {
        if !self.grant || self.payload.is_none() || epoch != self.epoch {
            return Err(
                "entry grant or schema epoch is no longer current".into()
            );
        }
        self.restores += 1;
        Ok(self.domain.issue(ty, Rc::from(KEY)))
    }
    fn read(
        &mut self,
        value: &Value,
        ty: &ResourceType,
        field: &str,
        epoch: u64,
    ) -> Result<Value, String> {
        let Value::Resource(reference) = value else {
            panic!("a genuine resource receiver is required");
        };
        assert_eq!(reference.domain(), &self.domain);
        assert_eq!(reference.resource_type(), ty);
        assert_eq!(reference.key(), KEY);
        if !self.grant || self.payload.is_none() {
            return Err("entry is closed".into());
        }
        if epoch != self.epoch {
            return Err("member schema epoch is no longer current".into());
        }
        self.getters += 1;
        let payload = self.payload.as_ref().unwrap();
        Ok(match field {
            "name" => Value::text(&payload.name),
            "age" => Value::Int(payload.age),
            _ => panic!("unexpected field"),
        })
    }
}

fn checked(header: &str, source: &str) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("json", header).unwrap();
    let mut session = Session::new(declarations);
    let offset = session.source().len();
    assert_eq!(offset, 0, "each fixture uses a fresh Session");
    let Outcome::Done(report) = session.line(source) else {
        panic!("this fixture has no source requests");
    };
    assert_eq!(report.offset, offset);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    compile::compile(report.unit.as_ref().unwrap())
}
fn request_label<'a>(
    program: &'a fipe::mir::Program,
    outcome: &'a Outcome<Result<Value, Fault>>,
) -> (String, &'a [Value]) {
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = outcome
    else {
        panic!("expected a checked Host call: {outcome:?}");
    };
    let tables = program.tables.borrow();
    let declaration = &tables.declarations[*declaration];
    assert_eq!(tables.namespaces[declaration.namespace], "json");
    (declaration.label.clone(), arguments)
}
fn resource_type(program: &fipe::mir::Program, name: &str) -> ResourceType {
    ResourceType::registered(&program.tables.borrow(), "json", name).unwrap()
}
fn pending_type(
    machine: &Machine,
    program: &fipe::mir::Program,
    name: &str,
) -> ResourceType {
    let ty = machine.pending_resource_type().unwrap();
    assert_eq!(ty, resource_type(program, name));
    ty
}
fn call_span(source: &str, call: &str) -> Span {
    let start = source.find(call).unwrap();
    assert_eq!(source.matches(call).count(), 1);
    Span {
        start,
        end: start + call.len(),
    }
}
fn refused(
    outcome: Outcome<Result<Value, Fault>>,
    source: &str,
    call: &str,
    message: &str,
) {
    let Outcome::Done(Err(fault)) = outcome else {
        panic!("expected refusal before later effects: {outcome:?}");
    };
    assert_eq!(fault.reason, Reason::Host(message.into()));
    assert_eq!(fault.span, call_span(source, call));
}

fn healthy_b(owner: &mut Owner, schema: &PublishedSchema) -> Value {
    let source = "use json\nj = json.restore ()\n(j.name, j.age)";
    let program = checked(B_HEADER, source);
    let mut machine = Machine::new(None);
    let outcome = machine.run(&program);
    assert_eq!(
        request_label(&program, &outcome),
        ("restore".into(), &[Value::Unit][..])
    );
    let ty = pending_type(&machine, &program, "JsonRowA");
    let value = owner.restore(&ty, schema.epoch).unwrap();
    let mut outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
    for field in ["name", "age"] {
        let (label, arguments) = request_label(&program, &outcome);
        assert_eq!(label, field);
        assert_eq!(arguments, std::slice::from_ref(&value));
        let (Value::Resource(receiver), Value::Resource(issued)) =
            (&arguments[0], &value)
        else {
            panic!("resource receiver");
        };
        assert!(Rc::ptr_eq(receiver, issued));
        let reply =
            owner.read(&arguments[0], &ty, field, schema.epoch).unwrap();
        outcome = machine.reply(&program, Reply::Value(Ok(reply)));
    }
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Tuple(ref values)))
        if values.as_slice() == [Value::text("Ada"), Value::Int(37)])
    );
    value
}
fn foreign_reply(old: &Value, owner: &Owner) {
    let source = "use json\nj = json.restore ()\nj.name";
    let program = checked(B_HEADER, source);
    let mut machine = Machine::new(None);
    assert_eq!(request_label(&program, &machine.run(&program)).0, "restore");
    let ty = pending_type(&machine, &program, "JsonRowA");
    let Value::Resource(reference) = old else {
        panic!("resource");
    };
    assert_ne!(reference.resource_type(), &ty);
    let before = owner.getters;
    let outcome = machine.reply(&program, Reply::Value(Ok(old.clone())));
    let Outcome::Done(Err(fault)) = outcome else {
        panic!("foreign reply");
    };
    assert_eq!(
        fault.reason,
        Reason::Host("the reply is not a JsonRowA".into())
    );
    assert_eq!(fault.span, call_span(source, "json.restore ()"));
    assert_eq!(owner.getters, before);
}
fn wrong_member_reply(owner: &mut Owner, schema: &PublishedSchema) {
    let source = "use json\nj = json.restore ()\nx = j.age\njson.after ()\nx";
    let program = checked(B_HEADER, source);
    let mut machine = Machine::new(None);
    assert_eq!(request_label(&program, &machine.run(&program)).0, "restore");
    let ty = pending_type(&machine, &program, "JsonRowA");
    let value = owner.restore(&ty, schema.epoch).unwrap();
    let outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
    let (label, arguments) = request_label(&program, &outcome);
    assert_eq!(label, "age");
    assert_eq!(arguments, std::slice::from_ref(&value));
    let (Value::Resource(receiver), Value::Resource(issued)) =
        (&arguments[0], &value)
    else {
        panic!("resource receiver");
    };
    assert!(Rc::ptr_eq(receiver, issued));
    assert_eq!(receiver.resource_type(), &ty);
    assert_eq!(receiver.domain(), &owner.domain);
    assert_eq!(receiver.key(), KEY);
    refused(
        machine.reply(&program, Reply::Value(Ok(Value::text("wrong")))),
        source,
        "j.age",
        "the reply is not an Integer",
    );
    assert!(owner.payload.is_some());
    assert_eq!(owner.drops.get(), 0);
}
fn admitted_getter_epoch_refusal(owner: &mut Owner, schema: &PublishedSchema) {
    let source = "use json\nj = json.restore ()\nx = j.age\njson.after ()\nx";
    let program = checked(B_HEADER, source);
    let mut machine = Machine::new(None);
    assert_eq!(request_label(&program, &machine.run(&program)).0, "restore");
    let ty = pending_type(&machine, &program, "JsonRowA");
    let dispatched_epoch = schema.epoch;
    let value = owner.restore(&ty, dispatched_epoch).unwrap();
    let outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
    let (label, arguments) = request_label(&program, &outcome);
    assert_eq!(label, "age");
    assert_eq!(arguments, std::slice::from_ref(&value));
    let (Value::Resource(receiver), Value::Resource(issued)) =
        (&arguments[0], &value)
    else {
        panic!("resource receiver");
    };
    assert!(Rc::ptr_eq(receiver, issued));
    let before = (
        owner.loads,
        owner.inspections,
        owner.getters,
        owner.restores,
    );
    owner.epoch += 1;
    let error = owner
        .read(&arguments[0], &ty, "age", dispatched_epoch)
        .unwrap_err();
    assert_eq!(
        before,
        (
            owner.loads,
            owner.inspections,
            owner.getters,
            owner.restores
        )
    );
    assert_eq!(owner.payload.as_ref().unwrap().age, 37);
    refused(
        machine.reply(&program, Reply::Value(Err(error))),
        source,
        "j.age",
        "member schema epoch is no longer current",
    );
    owner.epoch = dispatched_epoch;
}
fn restore_refusal(owner: &mut Owner, schema: &PublishedSchema) {
    let source = "use json\nj = json.restore ()\nj.name";
    let program = checked(B_HEADER, source);
    let mut machine = Machine::new(None);
    assert_eq!(request_label(&program, &machine.run(&program)).0, "restore");
    let ty = pending_type(&machine, &program, "JsonRowA");
    let before = (owner.getters, owner.inspections, owner.restores);
    let error = owner.restore(&ty, schema.epoch).unwrap_err();
    refused(
        machine.reply(&program, Reply::Value(Err(error))),
        source,
        "json.restore ()",
        "entry grant or schema epoch is no longer current",
    );
    assert_eq!(before, (owner.getters, owner.inspections, owner.restores));
}

#[test]
fn host_attested_nominal_members_rewrap_one_live_json_payload() {
    let mut owner = Owner::new();
    let program = checked(A_HEADER, "use json\njson.load ()");
    let mut machine = Machine::new(None);
    let outcome = machine.run(&program);
    assert_eq!(
        request_label(&program, &outcome),
        ("load".into(), &[Value::Unit][..])
    );
    let a_type = pending_type(&machine, &program, "BaseJson");
    let old = owner.load(&a_type);
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(old.clone()))),
        Outcome::Done(Ok(ref value)) if value == &old
    ));
    let schema = owner.publish();
    let before = (
        owner.loads,
        owner.inspections,
        owner.getters,
        owner.restores,
    );
    for _ in 0..8 {
        assert_eq!(
            schema
                .fields
                .iter()
                .filter(|(name, _)| name.starts_with('a'))
                .copied()
                .collect::<Vec<_>>(),
            [("age", "Integer")]
        );
    }
    assert_eq!(
        before,
        (
            owner.loads,
            owner.inspections,
            owner.getters,
            owner.restores
        )
    );
    let fresh = healthy_b(&mut owner, &schema);
    let Value::Resource(a) = &old else {
        panic!("A resource");
    };
    let Value::Resource(b) = &fresh else {
        panic!("B resource");
    };
    assert_eq!(a.domain(), b.domain());
    assert_eq!(a.key(), b.key());
    assert_ne!(a.resource_type(), b.resource_type());
    foreign_reply(&old, &owner);
    wrong_member_reply(&mut owner, &schema);
    admitted_getter_epoch_refusal(&mut owner, &schema);
    owner.grant = false;
    restore_refusal(&mut owner, &schema);
    owner.grant = true;
    owner.epoch = 2;
    restore_refusal(&mut owner, &schema);
    owner.epoch = 1;
    owner.payload.take();
    assert_eq!(owner.drops.get(), 1);
    assert!(owner.payload.is_none());
    assert_eq!(
        owner.read(&old, &a_type, "name", schema.epoch),
        Err("entry is closed".into())
    );
    assert_eq!(
        owner.read(&fresh, b.resource_type(), "age", schema.epoch),
        Err("entry is closed".into())
    );
    restore_refusal(&mut owner, &schema);
    assert_eq!(schema.fields, [("name", "String"), ("age", "Integer")]);
    assert_eq!(
        (
            owner.loads,
            owner.inspections,
            owner.getters,
            owner.restores
        ),
        (1, 1, 2, 3)
    );
    drop((old, fresh));
    assert_eq!(owner.drops.get(), 1);
    // Snapshot data is not production binding eligibility or completion wiring.
}
