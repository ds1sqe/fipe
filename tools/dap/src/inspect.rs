//! What a stop shows: named values, nested by constructor and field,
//! each compound one behind a `variablesReference` the client expands.
use fipe::{ir::Tables, value::structural::show, value::Value};
use serde_json::{json, Value as Json};

/// One shown item: a value of the run, or words the adapter wrote.
#[derive(Clone, Debug)]
pub enum Item {
    Value(Value),
    Text(String),
}

/// The containers a client may expand, valid until the run resumes.
/// Reference `n` is the container at index `n - 1`.
#[derive(Debug, Default)]
pub struct References {
    containers: Vec<Vec<(String, Item)>>,
}

impl References {
    /// Forget every container: the run moved.
    pub fn clear(&mut self) {
        self.containers.clear();
    }

    /// Keep a container and give its reference.
    pub fn add(&mut self, items: Vec<(String, Item)>) -> u64 {
        self.containers.push(items);
        self.containers.len() as u64
    }

    /// The variables of a reference, each compound value given a
    /// reference of its own.
    pub fn variables(
        &mut self,
        reference: u64,
        tables: &Tables,
    ) -> Option<Json> {
        let index = usize::try_from(reference).ok()?.checked_sub(1)?;
        let items = self.containers.get(index)?.clone();
        let variables: Vec<Json> = items
            .into_iter()
            .map(|(name, item)| self.variable(&name, &item, tables))
            .collect();
        Some(json!({ "variables": variables }))
    }

    /// One variable: its name, its value in the language's spelling and
    /// a reference to its parts when it has any.
    pub fn variable(
        &mut self,
        name: &str,
        item: &Item,
        tables: &Tables,
    ) -> Json {
        match item {
            Item::Text(text) => json!({
                "name": name,
                "value": text,
                "variablesReference": 0,
            }),
            Item::Value(value) => {
                let parts = parts(value, tables);
                let reference =
                    if parts.is_empty() { 0 } else { self.add(parts) };
                json!({
                    "name": name,
                    "value": show(value, tables),
                    "variablesReference": reference,
                })
            }
        }
    }
}

/// The parts of a value: a record's fields by label, a sequence's or a
/// tuple's elements by index, a packed value's own parts.
#[must_use]
pub fn parts(value: &Value, tables: &Tables) -> Vec<(String, Item)> {
    match value {
        Value::Record(record) => {
            let labels = tables
                .meanings
                .get(record.meaning)
                .and_then(|meaning| {
                    meaning.constructors.get(record.constructor)
                })
                .map(|constructor| &constructor.fields);
            record
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let label = labels
                        .and_then(|fields| fields.get(index))
                        .map_or_else(|| index.to_string(), |f| f.label.clone());
                    (label, Item::Value(field.clone()))
                })
                .collect()
        }
        Value::Seq(parts) | Value::Tuple(parts) => parts
            .iter()
            .enumerate()
            .map(|(index, part)| (index.to_string(), Item::Value(part.clone())))
            .collect(),
        Value::Packed(packed) => parts(&packed.value, tables),
        _ => Vec::new(),
    }
}

/// The side of an outcome or a `Bool`: `Some(true)` for `Just`, `Ok` and
/// `True`, `Some(false)` for `Nothing`, `Err` and `False`.
#[must_use]
pub fn side(value: &Value, tables: &Tables) -> Option<bool> {
    let Value::Record(record) = value else {
        return None;
    };
    let constructor = &tables
        .meanings
        .get(record.meaning)?
        .constructors
        .get(record.constructor)?
        .label;
    match constructor.as_str() {
        "Just" | "Ok" | "True" => Some(true),
        "Nothing" | "Err" | "False" => Some(false),
        _ => None,
    }
}
