use super::Outs;
use alloc::{format, string::String, vec::Vec};
use fipe::ir::{Kind, Tables, Type, Unit};

pub(super) fn render(unit: Option<&Unit>, out: &mut Outs) {
    let Some(unit) = unit else {
        out.write(b"no checked unit\n");
        return;
    };
    let tables = unit.tables();
    let namespaces: Vec<_> = unit
        .main
        .iter()
        .map(|index| tables.nodes[*index].namespace)
        .collect();
    out.write(
        format!(
            "checked unit: {} main stages, {} open entries\n",
            unit.main.len(),
            unit.plan.len()
        )
        .as_bytes(),
    );
    for (index, node) in tables.nodes.iter().enumerate() {
        if !namespaces.contains(&node.namespace) {
            continue;
        }
        let operation = operation(&node.kind, &tables);
        out.write(
            format!(
                "value {index}: {} = {operation}\n",
                meaning(&node.ty, &tables)
            )
            .as_bytes(),
        );
    }
    out.write(format!("main: {}\n", indices(&unit.main)).as_bytes());
}

fn indices(values: &[usize]) -> String {
    values
        .iter()
        .map(|value| format!("value {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn operation(kind: &Kind, tables: &Tables) -> String {
    match kind {
        Kind::Timing { phase, value } => format!("{} value {value}", if *phase == fipe::ir::Phase::Compile { "compile" } else { "initialize" }),
        Kind::Literal { reader, .. } => reader.map_or_else(|| String::from("literal"), |reader| format!("literal prepared by value {reader}")),
        Kind::Local { slot, meanings } => format!("local slot {slot}, {} hidden meanings", meanings.len()),
        Kind::Capture { slot, meanings } => format!("captured slot {slot}, {} hidden meanings", meanings.len()),
        Kind::Definition { index, meanings } => format!("definition {}, {} hidden meanings", tables.definitions[*index].label, meanings.len()),
        Kind::Declared(index) => format!("declared {}.{}", tables.namespaces[tables.declarations[*index].namespace], tables.declarations[*index].label),
        Kind::Derived { kind, .. } => format!("derived {}", match kind { fipe::ir::Derived::Eq => "equality", fipe::ir::Derived::Cmp => "comparison", fipe::ir::Derived::Show => "display", fipe::ir::Derived::Default => "default" }),
        Kind::Call { callee, supplies, defaults, open, .. } => format!("call value {callee}; {} supplies, {} defaults, {} open spots", supplies.len(), defaults.len(), open.len()),
        Kind::Dispatch { member, .. } => format!("dispatch {}, {} indexed choices", tables.dispatch[*member].label, tables.dispatch[*member].rows.len()),
        Kind::Function { arms, captures, hidden, .. } => format!("function with {} arms, {} captures, {} hidden meanings", arms.len(), captures.len(), hidden.len()),
        Kind::Construct { meaning, constructor, .. } | Kind::Constructor { meaning, constructor, .. } => format!("constructor {}.{}", tables.meanings[*meaning].label, tables.meanings[*meaning].constructors[*constructor].label),
        Kind::Field { value, index } => format!("field {index} of value {value}"),
        Kind::Tuple(values) => format!("tuple ({})", indices(values)),
        Kind::Repeat { value, length } => format!("repeat value {value}, length {}", meaning(length, tables)),
        Kind::Array(values) => format!("array [{}]", indices(values)),
        Kind::Block(values) => format!("sequence {}", indices(values)),
        Kind::Bind { value, .. } => format!("bind pattern from value {value}"),
        Kind::Pipe { left, stage } => format!("pipe value {left} through value {stage}"),
        Kind::Marked { good, left, stage, .. } => format!("{} side of value {left} through value {stage}", if *good { "good" } else { "bad" }),
        Kind::Or { left, right, .. } => format!("or values {left}, {right}"),
        Kind::And { left, right, .. } => format!("and values {left}, {right}"),
        Kind::If { condition, then_body, else_body } => format!("if value {condition}, value {then_body}, otherwise value {else_body}"),
        Kind::With { key, value, body } => format!("with {} from value {value}, body value {body}", tables.keys[*key]),
        Kind::Lift { value, body, .. } => format!("lift value {value} into body value {body}"),
        Kind::Ordering { value, .. } => format!("ordering from value {value}"),
        Kind::Cast { value, to, .. } => format!("cast value {value} to {}", meaning(to, tables)),
        Kind::Pack { value, tag } => format!("pack value {value} tagged {}", meaning(tag, tables)),
        Kind::Enter { entry, value } => format!("enter {} with value {value}", tables.entries[*entry].label),
        Kind::Convert { from, to, .. } => format!("conversion from {} to {}", meaning(from, tables), meaning(to, tables)),
    }
}

fn meaning(ty: &Type, tables: &Tables) -> String {
    match ty {
        Type::Unit => String::from("()"),
        Type::Meaning { index, arguments } => {
            let label = &tables.meanings[*index].label;
            if arguments.is_empty() {
                label.clone()
            } else {
                format!("{label}<{}>", meanings(arguments, tables))
            }
        }
        Type::Parameter(index) => format!("meaning parameter {index}"),
        Type::Session(index) => {
            tables.session_meanings[*index].as_ref().map_or_else(
                || format!("unfixed meaning {index}"),
                |ty| meaning(ty, tables),
            )
        }
        Type::Function { parameter, result } => format!(
            "{} -> {}",
            meaning(parameter, tables),
            meaning(result, tables)
        ),
        Type::Tuple(parts) => format!("({})", meanings(parts, tables)),
        Type::Array { element, length } => format!(
            "[{}; {}]",
            meaning(element, tables),
            meaning(length, tables)
        ),
        Type::Number(number) => format!("{number}"),
        Type::Packed(parts) => format!("packed {}", meanings(parts, tables)),
        Type::All(parts) => format!("all {}", meanings(parts, tables)),
        Type::Sum(parts) => format!("sum {}", meanings(parts, tables)),
        Type::Not(inner) => format!("not {}", meaning(inner, tables)),
    }
}

fn meanings(parts: &[Type], tables: &Tables) -> String {
    parts
        .iter()
        .map(|ty| meaning(ty, tables))
        .collect::<Vec<_>>()
        .join(", ")
}
