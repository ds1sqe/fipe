//! Literals: a prelude reader's value is prepared now; another reader
//! runs on the written form.
use super::{
    builder::Builder,
    calls::{Arguments, Target},
    dispatch,
};
use crate::{
    ir,
    mir::Operand,
    token::Magnitude,
    value::{BigInt, Record, Value},
};
use alloc::{format, rc::Rc, string::String};

impl Builder<'_, '_> {
    /// A literal node, counted by the caller.
    pub(super) fn literal(
        &mut self,
        node: usize,
        form: &ir::Form,
        reader: Option<usize>,
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let prepared = reader.map_or_else(
            || Some(plain(form)),
            |reader| self.prepared(reader, form),
        );
        if let Some(value) = prepared {
            let constant = self.lowering.constant(value);
            return self.place(Operand::Const(constant), dest, node);
        }
        let reader = reader.expect("an unprepared literal has a reader");
        let ir::Type::Function { parameter, .. } = &tables.nodes[reader].ty
        else {
            unreachable!("a reader takes the form")
        };
        let ir::Type::Meaning { index: meaning, .. } = &**parameter else {
            unreachable!("the form is a record")
        };
        let record = form_record(form, *meaning, tables);
        let constant = self.lowering.constant(record);
        let ty = tables.nodes[node].ty.clone();
        let dest = dest.unwrap_or_else(|| self.local(ty, None));
        let target = self.callee(reader);
        assert!(
            !matches!(target, Target::Construct { .. } | Target::Value(_)),
            "a literal reader is a member or a declaration"
        );
        let arguments = Arguments {
            args: alloc::vec![Operand::Const(constant)],
            receiver: None,
            argument: Some((**parameter).clone()),
        };
        self.apply(target, arguments, dest, node)
    }

    /// The value a reader prepares now.
    fn prepared(&self, reader: usize, form: &ir::Form) -> Option<Value> {
        prepared(self.tables(), self.lowering.prelude, reader, form)
    }
}

/// Whether a literal node runs its reader, a program's or a prelude
/// reader whose value is not finite, rather than being a constant now.
pub fn runs_reader(tables: &ir::Tables, node: usize) -> bool {
    let ir::Kind::Literal {
        form,
        reader: Some(reader),
    } = &tables.nodes[node].kind
    else {
        return false;
    };
    let prelude = tables
        .namespaces
        .iter()
        .position(|label| label == "prelude");
    prepared(tables, prelude, *reader, form).is_none()
}

/// Whether a literal node's reader is a host's declaration, an effect
/// that runs when the literal does.
pub fn host_reader(tables: &ir::Tables, node: usize) -> bool {
    let ir::Kind::Literal {
        reader: Some(reader),
        ..
    } = &tables.nodes[node].kind
    else {
        return false;
    };
    prelude_declaration(tables, *reader).is_some()
        && !prelude_operation(tables, *reader)
}

/// Whether an operation node names a declaration of the prelude, by name
/// or selected by a concrete meaning: native code, never the host's.
pub fn prelude_operation(tables: &ir::Tables, node: usize) -> bool {
    let prelude = tables
        .namespaces
        .iter()
        .position(|label| label == "prelude");
    prelude_declaration(tables, node).is_some_and(|index| {
        Some(tables.declarations[index].namespace) == prelude
    })
}

/// The declaration an operation node names, by name or by the row a
/// concrete meaning selects.
fn prelude_declaration(tables: &ir::Tables, node: usize) -> Option<usize> {
    match &tables.nodes[node].kind {
        ir::Kind::Declared(index) => Some(*index),
        ir::Kind::Dispatch {
            receiver: ir::Receiver::Meaning(ty),
            member,
        } => {
            let ty = dispatch::fixed(tables, ty);
            if !dispatch::concrete(&ty) {
                return None;
            }
            let ir::Type::Function { parameter, .. } = &tables.nodes[node].ty
            else {
                return None;
            };
            let selected =
                dispatch::select(tables, *member, &ty, Some(parameter))?;
            match tables.dispatch[*member].rows[selected.row].target {
                ir::Implementation::Declared(index) => Some(index),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The value a reader prepares now: the prelude's reader, named or
/// selected by a concrete meaning.
fn prepared(
    tables: &ir::Tables,
    prelude_namespace: Option<usize>,
    reader: usize,
    form: &ir::Form,
) -> Option<Value> {
    let declaration = prelude_declaration(tables, reader)?;
    if Some(tables.declarations[declaration].namespace) != prelude_namespace {
        return None;
    }
    let label = result_label(&tables.declarations[declaration].ty, tables);
    prelude(form, label)
}

/// The label of a declaration's result meaning, when it is the prelude's.
fn result_label<'a>(ty: &ir::Type, tables: &'a ir::Tables) -> &'a str {
    let mut ty = ty;
    while let ir::Type::Function { result, .. } = ty {
        ty = result;
    }
    match ty {
        ir::Type::Meaning { index, .. } => &tables.meanings[*index].label,
        ir::Type::Unit => "()",
        other => unreachable!("a prelude reader's result {other:?}"),
    }
}

/// A literal with no reader: a word, a path, the unit.
fn plain(form: &ir::Form) -> Value {
    match form {
        ir::Form::Unit => Value::Unit,
        ir::Form::Text(text) => Value::Text(Rc::new(text.clone())),
        other => unreachable!("a {other:?} literal has a reader"),
    }
}

/// A prelude reader's value, by the meaning it reads to; `None` when
/// the nearest float is not finite, so the reader runs and Faults
/// (`08-evaluation.md` §Arithmetic) until the compile phase reports
/// it.
fn prelude(form: &ir::Form, label: &str) -> Option<Value> {
    Some(match (label, form) {
        (
            "Integer",
            ir::Form::Whole {
                negative,
                magnitude,
                ..
            },
        ) => integer(*negative, magnitude),
        ("F64", ir::Form::Whole { .. } | ir::Form::Fraction { .. }) => {
            let value: f64 = decimal(form).parse().expect("a decimal spelling");
            Value::F64(value.is_finite().then_some(value)?)
        }
        ("F32", ir::Form::Whole { .. } | ir::Form::Fraction { .. }) => {
            let value: f32 = decimal(form).parse().expect("a decimal spelling");
            Value::F32(value.is_finite().then_some(value)?)
        }
        ("String", ir::Form::Text(text)) => Value::Text(Rc::new(text.clone())),
        (_, ir::Form::Bytes(bytes)) => Value::bytes(bytes),
        (_, ir::Form::Unit) => Value::Unit,
        (label, form) => unreachable!("the prelude reads {form:?} as {label}"),
    })
}

/// An integer that fits is `Int`; any other is `Big`.
fn integer(negative: bool, magnitude: &Magnitude) -> Value {
    if let Some(small) = magnitude.to_u64() {
        if let Ok(value) = i64::try_from(small) {
            return Value::Int(if negative { -value } else { value });
        }
        if negative && small == 1 << 63 {
            return Value::Int(i64::MIN);
        }
    }
    Value::Big(Rc::new(BigInt {
        negative,
        magnitude: magnitude.clone(),
    }))
}

/// The decimal spelling `core` parses with correct rounding.
fn decimal(form: &ir::Form) -> String {
    match form {
        ir::Form::Whole {
            negative,
            magnitude,
            ..
        } => format!("{}{magnitude}", if *negative { "-" } else { "" }),
        ir::Form::Fraction {
            negative,
            digits,
            exponent_negative,
            exponent,
        } => format!(
            "{}{digits}e{}{exponent}",
            if *negative { "-" } else { "" },
            if *exponent_negative { "-" } else { "" }
        ),
        other => unreachable!("a number spelling of {other:?}"),
    }
}

/// The written form as the prelude's `Form` record, for a program's or a
/// generic body's reader. Affixes are empty until L1.
fn form_record(form: &ir::Form, meaning: usize, tables: &ir::Tables) -> Value {
    let constructor = |label: &str| {
        tables.meanings[meaning]
            .constructors
            .iter()
            .position(|constructor| constructor.label == label)
            .expect("Form's constructors")
    };
    let text = |text: &str| Value::Text(Rc::new(String::from(text)));
    let (constructor, fields) = match form {
        ir::Form::Whole {
            negative,
            magnitude,
            radix,
            digits,
        } => (
            constructor("Whole"),
            alloc::vec![
                integer(*negative, magnitude),
                Value::Int(i64::from(*radix)),
                Value::Int(i64::try_from(*digits).expect("digit count fits")),
                text(""),
                text(""),
            ],
        ),
        ir::Form::Fraction {
            negative,
            digits,
            exponent_negative,
            exponent,
        } => (
            constructor("Fraction"),
            alloc::vec![
                integer(*negative, digits),
                integer(*exponent_negative, exponent),
                text(""),
                text(""),
            ],
        ),
        ir::Form::Text(value) => (
            constructor("Text"),
            alloc::vec![text(value), text(""), text("")],
        ),
        ir::Form::Bytes(bytes) => (
            constructor("Bytes"),
            alloc::vec![Value::bytes(bytes), text(""), text("")],
        ),
        ir::Form::Unit => unreachable!("the unit has no reader"),
    };
    Value::Record(Rc::new(Record {
        meaning,
        constructor,
        fields,
    }))
}
