//! The trace: a run's observation as text, one item per line. The
//! result first, a value in the language's spelling or a Fault located
//! as line:column in the source it names; then each host call with its
//! arguments, the `with` environment open at the call and the reply, in
//! order; then the success and error output.
use fipe::{
    embed::{Reply, Request},
    ir::Tables,
    token::Source,
    value::structural::show,
    vm::Observation,
};
use std::fmt::Write as _;

/// The text of each source a Fault may name, by namespace: the
/// example (`main`), its file modules, the host declaration files and
/// the prelude.
pub struct Sources<'a>(pub Vec<(&'a str, &'a str)>);

impl Sources<'_> {
    fn text(&self, namespace: &str) -> &str {
        if namespace == "prelude" {
            return fipe::decl::prelude_source();
        }
        let host = [
            ("commands", include_str!("../fixtures/host/commands.fipe")),
            ("fs", include_str!("../fixtures/host/fs.fipe")),
            ("sys", include_str!("../fixtures/host/sys.fipe")),
            ("cli", include_str!("../fixtures/host/cli.fipe")),
            ("text", include_str!("../fixtures/host/text.fipe")),
            ("config", include_str!("../fixtures/host/config.fipe")),
        ];
        self.0
            .iter()
            .chain(host.iter())
            .find(|(name, _)| *name == namespace)
            .map_or_else(
                || panic!("no source for {namespace}"),
                |(_, text)| text,
            )
    }
}

/// The trace of an observation over the tables of the run's program.
pub fn write(
    observation: &Observation,
    tables: &Tables,
    sources: &Sources,
) -> String {
    let mut out = String::new();
    match &observation.result {
        Ok(value) => writeln!(out, "value {}", show(value, tables)),
        Err(fault) => {
            let namespace = &tables.namespaces[fault.namespace];
            let (line, column) =
                Source::new(sources.text(namespace)).line_col(fault.span.start);
            writeln!(
                out,
                "fault in {namespace} at {line}:{column}: {}",
                fault.reason
            )
        }
    }
    .unwrap();
    for (request, reply) in &observation.trace {
        let Request::Call {
            declaration,
            arguments,
            environment,
        } = request
        else {
            panic!("a run asks only for host calls")
        };
        let declared = &tables.declarations[*declaration];
        let arguments: Vec<String> =
            arguments.iter().map(|value| show(value, tables)).collect();
        write!(
            out,
            "call {}.{} ({})",
            tables.namespaces[declared.namespace],
            declared.label,
            arguments.join(", ")
        )
        .unwrap();
        for (key, value) in environment {
            write!(out, " with {} {}", tables.keys[*key], show(value, tables))
                .unwrap();
        }
        match reply {
            Reply::Value(Ok(value)) => {
                writeln!(out, " -> {}", show(value, tables))
            }
            Reply::Value(Err(message)) => {
                writeln!(out, " -> failure {message:?}")
            }
            Reply::File(_) => panic!("a host call takes a value reply"),
        }
        .unwrap();
    }
    writeln!(
        out,
        "output {:?}",
        String::from_utf8_lossy(&observation.output)
    )
    .unwrap();
    out
}
