#![no_std]
//! A small application Host, with no Standard or OS implementation.
extern crate alloc;

use alloc::{
    collections::BTreeMap,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::Cell;
use fipe::{
    compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    mir::Program,
    session::{Report, Session},
    value::{
        resource::{ResourceDomain, ResourceType},
        Value,
    },
    vm::{Fault, Machine},
};

const TEXT: &str = "pub type resource Buffer\npub acquire: () -> Buffer\npub read: Buffer -> String";
const CLIENT: &str =
    "pub read: text.open.Buffer -> Integer\npub fail: () -> Integer";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    File(String),
    Call(String, String),
    Create(u64),
    Read(u64),
    Commit(u64),
    Revoke(u64),
    Withdraw,
    Dispose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    pub created: usize,
    pub live: usize,
    pub reads: usize,
    pub released: usize,
}

struct Payload {
    row: String,
    released: Rc<Cell<usize>>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.released.set(self.released.get() + 1);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Run(u64),
    Retained,
}
struct Entry {
    key: u64,
    owner: Owner,
    witness: ResourceType,
    payload: Option<Payload>,
}
struct Kernel {
    domain: ResourceDomain,
    entries: Vec<Entry>,
    active: Option<u64>,
    serial: u64,
    read_granted: bool,
    reads: usize,
    released: Rc<Cell<usize>>,
    events: Vec<Event>,
}
impl Kernel {
    fn new() -> Self {
        Self {
            domain: ResourceDomain::new(),
            entries: Vec::new(),
            active: None,
            serial: 0,
            read_granted: true,
            reads: 0,
            released: Rc::new(Cell::new(0)),
            events: Vec::new(),
        }
    }
    fn start(&mut self) {
        assert!(self.active.is_none(), "only one application run");
        self.serial = self.serial.checked_add(1).expect("finite run count");
        self.active = Some(self.serial);
    }
    fn acquire(&mut self, witness: ResourceType) -> Value {
        let key = u64::try_from(self.entries.len())
            .unwrap()
            .checked_add(1)
            .unwrap();
        let reference =
            self.domain.issue(&witness, Rc::from(key.to_le_bytes()));
        self.entries.push(Entry {
            key,
            owner: Owner::Run(self.active.expect("active acquisition")),
            witness,
            payload: Some(Payload {
                row: String::from("row"),
                released: Rc::clone(&self.released),
            }),
        });
        self.events.push(Event::Create(key));
        reference
    }
    fn read(&mut self, value: &Value) -> Result<String, String> {
        if !self.read_granted {
            return Err(String::from("kernel read denied"));
        }
        let Value::Resource(reference) = value else {
            return Err(String::from("not a resource"));
        };
        if reference.domain() != &self.domain {
            return Err(String::from("foreign owner"));
        }
        let bytes: [u8; 8] = reference
            .key()
            .try_into()
            .map_err(|_| String::from("bad key"))?;
        let key = u64::from_le_bytes(bytes);
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .ok_or_else(|| String::from("unknown entry"))?;
        if reference.resource_type() != &entry.witness {
            return Err(String::from("wrong type"));
        }
        let payload = entry
            .payload
            .as_ref()
            .ok_or_else(|| String::from("closed entry"))?;
        self.reads += 1;
        self.events.push(Event::Read(key));
        Ok(payload.row.clone())
    }
    fn answer(
        &mut self,
        namespace: &str,
        label: &str,
        arguments: &[Value],
        witness: Option<ResourceType>,
    ) -> Result<Value, String> {
        self.events
            .push(Event::Call(namespace.into(), label.into()));
        match (namespace, label, arguments) {
            ("text.open", "acquire", [Value::Unit]) => {
                Ok(self.acquire(witness.expect("pending acquire identity")))
            }
            ("text.open", "read", [value]) => {
                self.read(value).map(|text| Value::text(&text))
            }
            ("client.open", "read", [value]) => {
                let row = self.read(value)?;
                Ok(Value::Int(i64::try_from(row.len()).unwrap()))
            }
            ("client.open", "fail", [Value::Unit]) => {
                Err(String::from("kernel refused"))
            }
            _ => Err(String::from("unsupported application call")),
        }
    }
    fn finish(&mut self, success: bool) {
        let run = self.active.take().expect("active run terminal");
        for entry in &mut self.entries {
            if entry.owner == Owner::Run(run) {
                if success {
                    entry.owner = Owner::Retained;
                    self.events.push(Event::Commit(entry.key));
                } else {
                    entry.payload.take();
                    self.events.push(Event::Revoke(entry.key));
                }
            }
        }
    }
    fn dispose(&mut self) {
        assert!(self.active.is_none());
        for entry in &mut self.entries {
            if entry.payload.take().is_some() {
                self.events.push(Event::Revoke(entry.key));
            }
        }
        self.events.push(Event::Dispose);
    }
    fn counts(&self) -> Counts {
        Counts {
            created: self.entries.len(),
            live: self
                .entries
                .iter()
                .filter(|entry| entry.payload.is_some())
                .count(),
            reads: self.reads,
            released: self.released.get(),
        }
    }
}

/// Test application boundary. The test executable supplies only its allocator.
pub struct Host {
    session: Session,
    machine: Machine,
    files: BTreeMap<String, String>,
    kernel: Kernel,
    output: Vec<Value>,
}
impl Default for Host {
    fn default() -> Self {
        let mut declarations = Declarations::new();
        declarations
            .module("text.open", TEXT)
            .expect("text contract");
        declarations
            .module("client.open", CLIENT)
            .expect("client contract");
        Self {
            session: Session::new(declarations),
            machine: Machine::new(Some(100_000)),
            files: BTreeMap::new(),
            kernel: Kernel::new(),
            output: Vec::new(),
        }
    }
}
impl Host {
    pub fn source(&mut self, stem: &str, text: &str) {
        self.files.insert(stem.to_string(), text.to_string());
    }
    fn checked(&mut self, text: &str) -> Report {
        let mut outcome = self.session.line(text);
        loop {
            match outcome {
                Outcome::Done(report) => {
                    assert!(
                        report.diagnostics.is_empty(),
                        "check: {:?}",
                        report.diagnostics
                    );
                    return report;
                }
                Outcome::Request(Request::File { stem }) => {
                    self.kernel.events.push(Event::File(stem.clone()));
                    outcome = self
                        .session
                        .reply(Reply::File(self.files.get(&stem).cloned()));
                }
                Outcome::Request(other) => {
                    panic!("unexpected compile request: {other:?}")
                }
            }
        }
    }
    fn answer(&mut self, declaration: usize, arguments: &[Value]) -> Reply {
        let tables = self
            .machine
            .program()
            .expect("joined program")
            .tables
            .clone();
        let (namespace, label) = {
            let tables = tables.borrow();
            let row = &tables.declarations[declaration];
            (tables.namespaces[row.namespace].clone(), row.label.clone())
        };
        let witness =
            (namespace == "text.open" && label == "acquire").then(|| {
                self.machine
                    .pending_resource_type()
                    .expect("pending resource result")
            });
        Reply::Value(self.kernel.answer(&namespace, &label, arguments, witness))
    }
    fn pump(
        &mut self,
        program: &Program,
        mut outcome: Outcome<Result<Value, Fault>>,
    ) -> Result<Value, Fault> {
        loop {
            match outcome {
                Outcome::Done(result) => return result,
                Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    let reply = self.answer(declaration, &arguments);
                    outcome = self.machine.reply(program, reply);
                }
                Outcome::Request(other) => {
                    panic!("unexpected runtime request: {other:?}")
                }
            }
        }
    }
    /// Check and execute one source cell through the application Kernel.
    ///
    /// # Errors
    /// Returns the actual source-located VM Fault after NEW-only cleanup.
    ///
    /// # Panics
    /// On invalid fixture source, an unsupported request or a broken fixture invariant.
    pub fn run(&mut self, text: &str) -> Result<Value, Fault> {
        let report = self.checked(text);
        let program =
            compile::compile(report.unit.as_ref().expect("checked unit"));
        self.output.clear();
        self.kernel.start();
        let outcome = self.machine.run(&program);
        let result = self.pump(&program, outcome);
        self.output.extend(
            self.machine.lines().iter().map(|(_, value)| value.clone()),
        );
        self.kernel.finish(result.is_ok());
        if result.is_err() {
            let withdrawn = self
                .session
                .withdraw(report.accepted.expect("accepted before run"))
                .expect("latest failed line");
            self.machine.withdraw(&withdrawn);
            self.kernel.events.push(Event::Withdraw);
        }
        result
    }
    #[must_use]
    pub fn output(&self) -> &[Value] {
        &self.output
    }
    #[must_use]
    pub fn counts(&self) -> Counts {
        self.kernel.counts()
    }
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.kernel.events
    }
    pub const fn grant_read(&mut self, granted: bool) {
        self.kernel.read_granted = granted;
    }
    pub fn discard_language(&mut self) {
        self.output.clear();
        self.machine = Machine::new(Some(100_000));
        self.session = Session::new(Declarations::new());
    }
    #[must_use]
    pub fn withdrawn(&self, name: &str) -> bool {
        self.session.withdrawn(name)
    }
    pub fn dispose(&mut self) {
        self.kernel.dispose();
    }
    /// Direct Host-policy check, also usable after language disposal.
    ///
    /// # Errors
    /// Refuses absent grant, wrong domain/type/key or a closed entry before payload access.
    pub fn read_alias(&mut self, alias: &Value) -> Result<String, String> {
        self.kernel.read(alias)
    }
}
