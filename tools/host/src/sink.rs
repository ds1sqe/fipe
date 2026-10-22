//! The sink policy: main lines, writes to `out` and Faults as bytes on
//! the success and error output.
use fipe::{
    ir::Tables,
    token::Source,
    value::{structural, Prelude, Record, Value},
    vm::Fault,
};

/// The success and error output of one run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sink {
    pub success: Vec<u8>,
    pub error: Vec<u8>,
    /// How many of the run's main lines are written.
    lines: usize,
}

impl Sink {
    /// No output yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            success: Vec::new(),
            error: Vec::new(),
            lines: 0,
        }
    }

    /// Write the main lines kept since the last call
    /// (`vm::Machine::lines`): a `String` as its text, another value as
    /// the language spells it, each with a line feed; `Ok v` and
    /// `Just v` write `v`, `()` and `Nothing` nothing, `Err e` writes
    /// `e` to the error output, a host module's `Error` as its message.
    pub fn lines(&mut self, kept: &[(usize, Value)], tables: &Tables) {
        for (_, value) in kept.get(self.lines..).unwrap_or_default() {
            self.value(value, tables);
        }
        self.lines = kept.len();
    }

    /// Write the run's final value when it is an `Err` no main line
    /// holds: a pattern binding that failed ends the main body with
    /// its failure, which exit 1 alone would leave unsaid.
    pub fn ending(
        &mut self,
        value: &Value,
        kept: &[(usize, Value)],
        tables: &Tables,
    ) {
        let prelude = Prelude::new(tables);
        let failed = matches!(value, Value::Record(record)
            if Some(record.meaning) == prelude.result
                && record.constructor != 0);
        if failed && kept.last().is_none_or(|(_, last)| last != value) {
            self.value(value, tables);
        }
    }

    /// Write one value by the sink policy.
    fn value(&mut self, value: &Value, tables: &Tables) {
        let prelude = Prelude::new(tables);
        {
            let (error, value) = match value {
                Value::Record(record)
                    if Some(record.meaning) == prelude.maybe =>
                {
                    let Some(value) = record.fields.first() else {
                        return;
                    };
                    (false, value)
                }
                Value::Record(record)
                    if Some(record.meaning) == prelude.result =>
                {
                    (record.constructor != 0, &record.fields[0])
                }
                value => (false, value),
            };
            let out = if error {
                &mut self.error
            } else {
                &mut self.success
            };
            match value {
                Value::Unit => return,
                Value::Text(text) => out.extend(text.as_bytes()),
                Value::Record(record) => {
                    let shown = host_error(record, tables).map_or_else(
                        || structural::show(value, tables),
                        String::from,
                    );
                    out.extend(shown.as_bytes());
                }
                value => {
                    out.extend(structural::show(value, tables).as_bytes());
                }
            }
            out.push(b'\n');
        }
    }

    /// Write a Fault to the error output, located in the source its
    /// namespace names: `fault in <namespace> at <line>:<column>:
    /// <reason>`. `sources` maps a namespace to its text; the host's
    /// declarations and the prelude are known already.
    pub fn fault(
        &mut self,
        fault: &Fault,
        tables: &Tables,
        sources: &[(String, String)],
    ) {
        self.error
            .extend(located(fault, tables, sources).as_bytes());
        self.error.push(b'\n');
    }
}

/// A Fault located in its namespace's source; at offset zero when no
/// source has that name.
#[must_use]
pub fn located(
    fault: &Fault,
    tables: &Tables,
    sources: &[(String, String)],
) -> String {
    let namespace = &tables.namespaces[fault.namespace];
    let text = if namespace == "prelude" {
        fipe::decl::prelude_source()
    } else {
        sources
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .chain(crate::sources().iter().copied())
            .find(|(name, _)| name == namespace)
            .map_or("", |(_, text)| text)
    };
    let (line, column) = Source::new(text).line_col(fault.span.start);
    format!(
        "fault in {namespace} at {line}:{column}: {}",
        fault.reason.describe(tables)
    )
}

/// The message of a host module's `Error` record.
fn host_error<'r>(record: &'r Record, tables: &Tables) -> Option<&'r str> {
    let meaning = &tables.meanings[record.meaning];
    let host = crate::sources()
        .iter()
        .any(|(name, _)| tables.namespaces[meaning.namespace] == *name);
    match record.fields.as_slice() {
        [Value::Text(message)] if host && meaning.label == "Error" => {
            Some(message.as_str())
        }
        _ => None,
    }
}
