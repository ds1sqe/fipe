//! The recording host: it answers each host call from a script keyed
//! by the declaration's `namespace.label`, in order per key, records
//! every request with its reply, and assembles the observation.
use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
use fipe::{
    compile,
    embed::{Reply, Request},
    mir::Program,
    vm::Observation,
};

/// The conformance host's declarations.
pub fn declarations() -> Declarations {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    for (name, source) in [
        ("fs", include_str!("../fixtures/host/fs.fipe")),
        ("sys", include_str!("../fixtures/host/sys.fipe")),
        ("cli", include_str!("../fixtures/host/cli.fipe")),
        ("text", include_str!("../fixtures/host/text.fipe")),
    ] {
        declarations.module(name, source).unwrap();
    }
    declarations
}

/// A source checked against the conformance host, lowered.
pub fn program(source: &str) -> Program {
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations(), &[], check::Options::default())
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    compile::compile(&unit)
}

/// Replies by `namespace.label`, each taken once, in order per key.
#[derive(Clone, Debug, Default)]
pub struct Script(pub Vec<(String, Result<Value, String>)>);

impl Script {
    pub fn reply(mut self, key: &str, reply: Result<Value, String>) -> Self {
        self.0.push((key.into(), reply));
        self
    }

    fn answer(&mut self, key: &str) -> Result<Value, String> {
        let at = self
            .0
            .iter()
            .position(|(scripted, _)| scripted == key)
            .unwrap_or_else(|| panic!("no scripted reply for {key}"));
        self.0.remove(at).1
    }
}

/// The `namespace.label` of a requested declaration.
pub fn key(program: &Program, request: &Request) -> String {
    let Request::Call { declaration, .. } = request else {
        panic!("a host call, not {request:?}")
    };
    let tables = program.tables.borrow();
    let declaration = &tables.declarations[*declaration];
    format!(
        "{}.{}",
        tables.namespaces[declaration.namespace], declaration.label
    )
}

/// Run a program under a budget, or none, against the script.
pub fn observe(
    program: &Program,
    budget: Option<u64>,
    script: Script,
) -> Observation {
    observe_charged(program, budget, script).0
}

/// [`observe`], with the units the run charged, which are the VM's
/// metering and not part of the observation.
pub fn observe_charged(
    program: &Program,
    budget: Option<u64>,
    mut script: Script,
) -> (Observation, u64) {
    let mut machine = Machine::new(budget);
    let mut trace = Vec::new();
    let mut outcome = machine.run(program);
    let result = loop {
        match outcome {
            Outcome::Done(result) => break result,
            Outcome::Request(request) => {
                let reply =
                    Reply::Value(script.answer(&key(program, &request)));
                trace.push((request, reply.clone()));
                outcome = machine.reply(program, reply);
            }
        }
    };
    let observation = Observation {
        result,
        trace,
        output: Vec::new(),
    };
    (observation, machine.charged())
}
