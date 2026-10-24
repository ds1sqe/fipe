//! A test-owned backend ledger; wrappers carry identity, never payload ownership.
use fipe::{
    compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    ir,
    session::{Accepted, Session},
    value::{
        resource::{ResourceDomain, ResourceType},
        Value,
    },
    vm::{Fault, Machine},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const HEADER: &str = "pub type resource Table\npub acquire: () -> Table\npub read_table: Table -> String\npub same_table: Table -> Table\npub close_table: Table -> ()\npub hidden_then_fail: () -> ()\npub pause_table: Table -> ()";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    slot: usize,
    generation: u64,
}
impl Key {
    fn bytes(self) -> Rc<[u8]> {
        let mut bytes =
            u64::try_from(self.slot).unwrap().to_le_bytes().to_vec();
        bytes.extend(self.generation.to_le_bytes());
        Rc::from(bytes)
    }
    fn read(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != 16 {
            return Err("malformed key");
        }
        Ok(Self {
            slot: usize::try_from(u64::from_le_bytes(
                bytes[..8].try_into().unwrap(),
            ))
            .map_err(|_| "slot")?,
            generation: u64::from_le_bytes(bytes[8..].try_into().unwrap()),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owned {
    Application,
    Run(u64),
    Session(u64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Grant(Key, &'static str),
    Start(u64),
    End(bool),
    Create(Key),
    Commit(Key),
    Borrow(Key),
    Read(Key),
    Revoke(Key),
    Release(Key),
    Failed(Key),
    Invalidate,
    Abandon,
    Withdraw,
    Dispose,
}
struct TablePayload {
    rows: Vec<Vec<String>>,
    releases: Rc<Cell<usize>>,
}
impl Drop for TablePayload {
    fn drop(&mut self) {
        self.releases.set(self.releases.get() + 1);
    }
}
struct Entry {
    key: Key,
    owner: Owned,
    live: bool,
    leases: usize,
    closing: bool,
    fail_close: bool,
    payload: Option<TablePayload>,
    events: Rc<RefCell<Vec<Event>>>,
}
impl Entry {
    fn release(&mut self) {
        if !self.closing || self.leases != 0 || self.payload.is_none() {
            return;
        }
        if self.fail_close {
            self.events.borrow_mut().push(Event::Failed(self.key));
        } else {
            self.payload.take();
            self.events.borrow_mut().push(Event::Release(self.key));
        }
    }
    fn revoke(&mut self) {
        if self.live {
            self.live = false;
            self.closing = true;
            self.events.borrow_mut().push(Event::Revoke(self.key));
        }
        self.release();
    }
}
struct Lease {
    entry: Rc<RefCell<Entry>>,
    reads: Rc<Cell<usize>>,
}
impl Lease {
    fn read(&self) -> String {
        let entry = self.entry.borrow();
        self.reads.set(self.reads.get() + 1);
        entry.events.borrow_mut().push(Event::Read(entry.key));
        entry.payload.as_ref().unwrap().rows[0][0].clone()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut entry = self.entry.borrow_mut();
        entry.leases -= 1;
        entry.release();
    }
}
struct Owner {
    domain: ResourceDomain,
    ty: Option<ResourceType>,
    session: u64,
    next_run: u64,
    active: Option<(u64, Vec<Key>)>,
    entries: Vec<Rc<RefCell<Entry>>>,
    issued: Vec<Value>,
    events: Rc<RefCell<Vec<Event>>>,
    releases: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
}
impl Owner {
    fn new() -> Self {
        Self {
            domain: ResourceDomain::new(),
            ty: None,
            session: 1,
            next_run: 0,
            active: None,
            entries: Vec::new(),
            issued: Vec::new(),
            events: Rc::default(),
            releases: Rc::default(),
            reads: Rc::default(),
        }
    }
    fn start(&mut self) {
        assert!(self.active.is_none());
        self.next_run += 1;
        self.events.borrow_mut().push(Event::Start(self.next_run));
        self.active = Some((self.next_run, Vec::new()));
    }
    fn acquire(&mut self) -> Value {
        let run = self.active.as_ref().unwrap().0;
        self.insert(Owned::Run(run))
    }
    fn insert(&mut self, owner: Owned) -> Value {
        let key = Key {
            slot: self.entries.len(),
            generation: 1,
        };
        self.entries.push(Rc::new(RefCell::new(Entry {
            key,
            owner,
            live: true,
            leases: 0,
            closing: false,
            fail_close: false,
            payload: Some(TablePayload {
                rows: vec![vec!["private actual row".into()]],
                releases: self.releases.clone(),
            }),
            events: self.events.clone(),
        })));
        if matches!(owner, Owned::Run(_)) {
            self.active.as_mut().unwrap().1.push(key);
        }
        self.events.borrow_mut().push(Event::Create(key));
        let value = self.domain.issue(self.ty.as_ref().unwrap(), key.bytes());
        self.issued.push(value.clone());
        value
    }
    fn validate(
        &self,
        value: &Value,
        allowed: bool,
    ) -> Result<Rc<RefCell<Entry>>, &'static str> {
        let Value::Resource(reference) = value else {
            return Err("not resource");
        };
        if reference.domain() != &self.domain {
            return Err("domain");
        }
        if Some(reference.resource_type()) != self.ty.as_ref() {
            return Err("type");
        }
        let key = Key::read(reference.key())?;
        let entry = self.entries.get(key.slot).ok_or("slot")?;
        let state = entry.borrow();
        if state.key != key {
            return Err("generation");
        }
        if !state.live {
            return Err("revoked");
        }
        if !matches!(state.owner, Owned::Application)
            && state.owner != Owned::Session(self.session)
            && !self
                .active
                .as_ref()
                .is_some_and(|(run, _)| state.owner == Owned::Run(*run))
        {
            return Err("session");
        }
        if !allowed {
            return Err("current grant");
        }
        drop(state);
        Ok(entry.clone())
    }
    fn borrow(
        &self,
        value: &Value,
        allowed: bool,
    ) -> Result<Lease, &'static str> {
        let entry = self.validate(value, allowed)?;
        self.events
            .borrow_mut()
            .push(Event::Grant(entry.borrow().key, "Read"));
        {
            let mut state = entry.borrow_mut();
            state.leases += 1;
            self.events.borrow_mut().push(Event::Borrow(state.key));
        }
        Ok(Lease {
            entry,
            reads: self.reads.clone(),
        })
    }
    fn close(&self, value: &Value, allowed: bool) -> Result<(), &'static str> {
        let entry = self.validate(value, allowed)?;
        self.events
            .borrow_mut()
            .push(Event::Grant(entry.borrow().key, "Close"));
        entry.borrow_mut().revoke();
        Ok(())
    }
    fn finish(&mut self, success: bool) {
        let (run, created) = self.active.take().unwrap();
        self.events.borrow_mut().push(Event::End(success));
        for key in created {
            let mut entry = self.entries[key.slot].borrow_mut();
            if entry.key != key || entry.owner != Owned::Run(run) {
                continue;
            }
            if success {
                entry.owner = Owned::Session(self.session);
                self.events.borrow_mut().push(Event::Commit(key));
            } else {
                entry.revoke();
            }
        }
    }
    fn retry(&self, key: Key) -> Result<(), &'static str> {
        let mut entry = self.entries.get(key.slot).ok_or("slot")?.borrow_mut();
        if entry.key != key {
            return Err("generation");
        }
        entry.fail_close = false;
        entry.release();
        Ok(())
    }
    fn replace_released(&mut self, key: Key) -> Value {
        let old = self.entries[key.slot].borrow();
        assert_eq!(old.key, key);
        assert!(old.payload.is_none());
        assert_eq!(old.leases, 0);
        let replacement = Key {
            slot: key.slot,
            generation: key.generation.checked_add(1).unwrap(),
        };
        drop(old);
        self.entries[key.slot] = Rc::new(RefCell::new(Entry {
            key: replacement,
            owner: Owned::Session(self.session),
            live: true,
            leases: 0,
            closing: false,
            fail_close: false,
            payload: Some(TablePayload {
                rows: vec![vec!["replacement".into()]],
                releases: self.releases.clone(),
            }),
            events: self.events.clone(),
        }));
        self.domain
            .issue(self.ty.as_ref().unwrap(), replacement.bytes())
    }
    fn dispose(&self) {
        self.events.borrow_mut().push(Event::Dispose);
        for entry in &self.entries {
            let mut entry = entry.borrow_mut();
            if entry.owner == Owned::Session(self.session) {
                entry.revoke();
            }
        }
    }
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.entries
                .iter()
                .filter(|entry| entry.borrow().live)
                .count(),
            self.entries
                .iter()
                .filter(|entry| entry.borrow().payload.is_some())
                .count(),
            self.releases.get(),
        )
    }
    fn answer(
        &mut self,
        request: &Request,
        tables: &ir::Tables,
    ) -> Option<Reply> {
        let Request::Call {
            declaration,
            arguments,
            ..
        } = request
        else {
            panic!("no files");
        };
        let result = match tables.declarations[*declaration].label.as_str() {
            "acquire" => {
                self.ty = Some(
                    ResourceType::registered(tables, "commands", "Table")
                        .unwrap(),
                );
                Ok(self.acquire())
            }
            "read_table" => self
                .borrow(&arguments[0], true)
                .map(|lease| Value::text(&lease.read())),
            "same_table" => self
                .borrow(&arguments[0], true)
                .map(|_lease| arguments[0].clone()),
            "close_table" => {
                self.close(&arguments[0], true).map(|()| Value::Unit)
            }
            "hidden_then_fail" => {
                self.acquire();
                Err("backend failure after insertion")
            }
            "pause_table" => {
                assert!(self.validate(&arguments[0], true).is_ok());
                return None;
            }
            other => panic!("unexpected {other}"),
        };
        Some(Reply::Value(result.map_err(str::to_owned)))
    }
}
struct Driver {
    session: Session,
    machine: Machine,
    owner: Owner,
    program: Option<fipe::mir::Program>,
    accepted: Option<Accepted>,
    pending: Option<u64>,
    nonce: u64,
}
impl Driver {
    fn new() -> Self {
        let mut declarations = Declarations::new();
        declarations.commands(HEADER).unwrap();
        Self {
            session: Session::new(declarations),
            machine: Machine::new(None),
            owner: Owner::new(),
            program: None,
            accepted: None,
            pending: None,
            nonce: 0,
        }
    }
    fn run(&mut self, source: &str) -> Outcome<Result<Value, Fault>> {
        assert!(self.pending.is_none());
        let Outcome::Done(report) = self.session.line(source) else {
            panic!("no source requests");
        };
        assert!(
            report.diagnostics.is_empty(),
            "{source}: {:?}",
            report.diagnostics
        );
        self.accepted = report.accepted;
        self.program = Some(compile::compile(report.unit.as_ref().unwrap()));
        self.owner.start();
        let outcome = self.machine.run(self.program.as_ref().unwrap());
        self.pump(outcome)
    }
    fn pump(
        &mut self,
        mut outcome: Outcome<Result<Value, Fault>>,
    ) -> Outcome<Result<Value, Fault>> {
        loop {
            match outcome {
                Outcome::Done(result) => {
                    self.pending = None;
                    self.owner.finish(result.is_ok());
                    if result.is_err() {
                        self.withdraw();
                    }
                    return Outcome::Done(result);
                }
                Outcome::Request(ref request) => {
                    let tables = self.machine.program().unwrap().tables.clone();
                    let reply = {
                        let tables = tables.borrow();
                        if matches!(request, Request::Call { declaration, .. } if tables.declarations[*declaration].label == "acquire")
                        {
                            assert_eq!(
                                self.machine.pending_resource_type().unwrap(),
                                ResourceType::registered(
                                    &tables, "commands", "Table"
                                )
                                .unwrap()
                            );
                        }
                        self.owner.answer(request, &tables)
                    };
                    if let Some(reply) = reply {
                        outcome = self
                            .machine
                            .reply(self.program.as_ref().unwrap(), reply);
                    } else {
                        self.nonce += 1;
                        self.pending = Some(self.nonce);
                        return outcome;
                    }
                }
            }
        }
    }
    fn withdraw(&mut self) {
        let withdrawn = self
            .session
            .withdraw(self.accepted.take().unwrap())
            .unwrap();
        self.machine.withdraw(&withdrawn);
        self.owner.events.borrow_mut().push(Event::Withdraw);
    }
    fn abandon(&mut self) {
        if self.pending.take().is_none() {
            assert!(!self.machine.abandon());
            return;
        }
        self.owner.events.borrow_mut().push(Event::Invalidate);
        assert!(self.machine.abandon());
        self.owner.events.borrow_mut().push(Event::Abandon);
        self.owner.finish(false);
        self.withdraw();
    }
    fn late_reply(&self, token: u64) -> bool {
        if self.pending != Some(token) {
            return false;
        }
        panic!("test only supplies stale replies")
    }
    fn ok(&mut self, source: &str) -> Value {
        let Outcome::Done(Ok(value)) = self.run(source) else {
            panic!("successful run: {source}");
        };
        value
    }
}

#[test]
fn successful_runs_retain_entries_and_aliases_do_not_own() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    assert_eq!(driver.owner.counts(), (1, 1, 0));
    let value = driver.owner.issued[0].clone();
    driver.ok("reader: () -> String = _ => read_table t\n(reader (), same_table t, (t, t))");
    assert_eq!(driver.owner.entries.len(), 1);
    assert_eq!(driver.owner.reads.get(), 1);
    assert_eq!(
        driver.owner.borrow(&value, true).unwrap().read(),
        "private actual row"
    );
    driver.ok("discarded = acquire ()\n7");
    assert_eq!(driver.owner.counts(), (2, 2, 0));
    driver.owner.issued.clear();
    drop(value);
    drop(driver.machine);
    drop(driver.session);
    assert_eq!(driver.owner.counts(), (2, 2, 0));
    let events = driver.owner.events.borrow();
    assert_eq!(
        &events[..4],
        &[
            Event::Start(1),
            Event::Create(Key {
                slot: 0,
                generation: 1
            }),
            Event::End(true),
            Event::Commit(Key {
                slot: 0,
                generation: 1
            })
        ]
    );
}

#[test]
fn fault_revokes_only_entries_created_by_that_run() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    assert!(matches!(
        driver.run(
            "seen = read_table t\nfresh = acquire ()\nhidden_then_fail ()"
        ),
        Outcome::Done(Err(_))
    ));
    assert_eq!(driver.owner.counts(), (1, 1, 2));
    assert_eq!(driver.owner.reads.get(), 1);
    assert_eq!(driver.ok("read_table t"), Value::text("private actual row"));
    let language_error = driver.ok("discarded = acquire ()\nresult: Result<Integer, String> = Err \"language error\"\nresult");
    let Value::Record(error) = language_error else {
        panic!("language Err value");
    };
    let tables = driver.machine.program().unwrap().tables.borrow();
    assert_eq!(
        tables.meanings[error.meaning].constructors[error.constructor].label,
        "Err"
    );
    drop(tables);
    assert_eq!(driver.owner.counts(), (2, 2, 2));
    let events = driver.owner.events.borrow();
    assert!(
        events
            .iter()
            .position(|event| matches!(event, Event::Read(_)))
            .unwrap()
            < events
                .iter()
                .position(|event| matches!(event, Event::Revoke(_)))
                .unwrap()
    );
    assert!(matches!(
        events.iter().find(|event| matches!(event, Event::Withdraw)),
        Some(Event::Withdraw)
    ));
}

#[test]
fn abandon_invalidates_driver_reply_before_cleanup() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    assert!(matches!(
        driver.run("fresh = acquire ()\npause_table fresh"),
        Outcome::Request(_)
    ));
    let token = driver.pending.unwrap();
    let offset = driver.owner.events.borrow().len();
    driver.abandon();
    driver.abandon();
    assert!(!driver.late_reply(token));
    assert_eq!(driver.owner.counts(), (1, 1, 1));
    assert_eq!(
        &driver.owner.events.borrow()[offset..],
        &[
            Event::Invalidate,
            Event::Abandon,
            Event::End(false),
            Event::Revoke(Key {
                slot: 1,
                generation: 1
            }),
            Event::Release(Key {
                slot: 1,
                generation: 1
            }),
            Event::Withdraw
        ]
    );
    assert_eq!(driver.ok("read_table t"), Value::text("private actual row"));
    driver.ok("next = acquire ()");
    assert_eq!(driver.owner.counts(), (2, 2, 1));
}

#[test]
fn close_revokes_aliases_before_release_and_failure_is_not_authority() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    let alias = driver.owner.issued[0].clone();
    let held = driver.owner.borrow(&alias, true).unwrap();
    driver.ok("close_table t");
    assert_eq!(driver.owner.counts(), (0, 1, 0));
    assert!(driver.owner.borrow(&alias, true).is_err());
    assert_eq!(held.read(), "private actual row");
    drop(held);
    assert_eq!(driver.owner.counts(), (0, 0, 1));
    driver.ok("a = acquire ()\nb = acquire ()");
    driver.owner.entries[1].borrow_mut().fail_close = true;
    driver.owner.dispose();
    assert_eq!(driver.owner.counts(), (0, 1, 2));
    assert!(driver.owner.borrow(&driver.owner.issued[1], true).is_err());
    let key = driver.owner.entries[1].borrow().key;
    driver.owner.retry(key).unwrap();
    driver.owner.retry(key).unwrap();
    assert_eq!(driver.owner.counts(), (0, 0, 3));
    let events = driver.owner.events.borrow();
    assert!(events.contains(&Event::Failed(key)));
    assert!(
        events
            .iter()
            .position(|event| *event == Event::Revoke(key))
            .unwrap()
            < events
                .iter()
                .position(|event| *event == Event::Failed(key))
                .unwrap()
    );
}

#[test]
fn generation_domain_session_and_per_call_grants_are_checked() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    let alias = driver.owner.issued[0].clone();
    let owner = &mut driver.owner;
    let key = owner.entries[0].borrow().key;
    let baseline = owner.reads.get();
    let event_count = owner.events.borrow().len();
    let ty = owner.ty.as_ref().unwrap();
    let foreign = ResourceDomain::new().issue(ty, key.bytes());
    let malformed = owner.domain.issue(ty, Rc::from(&b"broken"[..]));
    let stale = owner.domain.issue(
        ty,
        Key {
            slot: 0,
            generation: 0,
        }
        .bytes(),
    );
    for value in [&foreign, &malformed, &stale] {
        assert!(owner.borrow(value, true).is_err());
    }
    owner.session = 2;
    assert!(owner.borrow(&alias, true).is_err());
    owner.session = 1;
    assert!(owner.borrow(&alias, false).is_err());
    assert!(owner.close(&alias, false).is_err());
    assert_eq!(owner.reads.get(), baseline);
    assert_eq!(owner.events.borrow().len(), event_count);
    assert_eq!(
        owner.borrow(&alias, true).unwrap().read(),
        "private actual row"
    );
    owner.close(&alias, true).unwrap();
    // Slot reuse is backend policy: replace only fully released storage, never an active lease.
    let replacement = owner.replace_released(key);
    assert!(owner.close(&alias, true).is_err());
    assert!(owner.retry(key).is_err());
    assert!(owner.entries[0].borrow().live);
    assert!(owner.entries[0].borrow().payload.is_some());
    assert_eq!(
        owner.borrow(&replacement, true).unwrap().read(),
        "replacement"
    );
}

#[test]
fn withdrawal_and_disposal_obey_explicit_host_ownership() {
    let mut driver = Driver::new();
    driver.ok("t = acquire ()");
    let value = driver.owner.issued[0].clone();
    let language_clone = driver.session.clone();
    driver.withdraw();
    drop(language_clone);
    assert_eq!(driver.owner.counts(), (1, 1, 0));
    driver.ok("a = acquire ()");
    let loan = driver.owner.insert(Owned::Application);
    driver.owner.entries[0].borrow_mut().fail_close = true;
    drop(value);
    driver.owner.dispose();
    assert_eq!(driver.owner.counts(), (1, 2, 1));
    driver.owner.session = 2;
    assert!(driver.owner.borrow(&driver.owner.issued[1], true).is_err());
    assert_eq!(
        driver.owner.borrow(&loan, true).unwrap().read(),
        "private actual row"
    );
    let key = driver.owner.entries[0].borrow().key;
    driver.owner.retry(key).unwrap();
    assert_eq!(driver.owner.counts(), (1, 1, 2));
}

#[test]
fn domain_separation_is_host_policy_after_language_type_fit() {
    let mut driver = Driver::new();
    let Outcome::Done(report) =
        driver.session.line("t = acquire ()\nread_table t")
    else {
        panic!("source");
    };
    assert!(report.diagnostics.is_empty());
    let program = compile::compile(report.unit.as_ref().unwrap());
    driver.owner.start();
    assert!(matches!(driver.machine.run(&program), Outcome::Request(_)));
    let ty = driver.machine.pending_resource_type().unwrap();
    assert_eq!(
        ty,
        ResourceType::registered(&program.tables.borrow(), "commands", "Table")
            .unwrap()
    );
    driver.owner.ty = Some(ty.clone());
    let foreign = ResourceDomain::new().issue(
        &ty,
        Key {
            slot: 0,
            generation: 1,
        }
        .bytes(),
    );
    let Outcome::Request(request) =
        driver.machine.reply(&program, Reply::Value(Ok(foreign)))
    else {
        panic!("same checked type must reach Host read");
    };
    let Request::Call { declaration, .. } = &request else {
        panic!("read call");
    };
    assert_eq!(
        driver
            .machine
            .program()
            .unwrap()
            .tables
            .borrow()
            .declarations[*declaration]
            .label,
        "read_table"
    );
    let tables = driver.machine.program().unwrap().tables.clone();
    let answer = driver.owner.answer(&request, &tables.borrow()).unwrap();
    assert_eq!(answer, Reply::Value(Err("domain".into())));
    assert_eq!(driver.owner.reads.get(), 0);
    assert!(matches!(
        driver.machine.reply(&program, answer),
        Outcome::Done(Err(_))
    ));
    driver.owner.finish(false);
    assert_eq!(driver.owner.counts(), (0, 0, 0));
}
