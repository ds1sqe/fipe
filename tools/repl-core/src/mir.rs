use super::{ir::meaning, Outs};
use alloc::{format, string::String, vec::Vec};
use fipe::{
    ir::{Derived, Kind, Predicate, Tables, Unit},
    mir::{Callee, Operand, Program, Rvalue, Statement, Terminator, Witness},
    value::Value,
};

pub(super) fn render(unit: Option<&Unit>, out: &mut Outs) {
    let Some(unit) = unit else {
        out.write(b"no checked unit\n");
        return;
    };
    let program = fipe::compile::compile(unit);
    let tables = unit.tables();
    out.write(
        format!(
            "program: {} bodies, {} constants\n",
            program.bodies.len(),
            program.constants.len()
        )
        .as_bytes(),
    );
    for (index, constant) in program.constants.iter().enumerate() {
        let text = format!("constant {index} = {}\n", value(constant, &tables));
        out.write(text.as_bytes());
    }
    for (index, body) in program.bodies.iter().enumerate() {
        let what = role(&program, index, &tables);
        out.write(
            format!(
                "body {index}: {what}, {} locals, {} blocks\n",
                body.locals.len(),
                body.blocks.len()
            )
            .as_bytes(),
        );
        for (index, local) in body.locals.iter().enumerate() {
            let spot = if (1..=body.arity).contains(&index) {
                ", spot"
            } else {
                ""
            };
            let ty = meaning(&local.ty, &tables);
            out.write(format!("  _{index}: {ty}{spot}\n").as_bytes());
        }
        for (index, block) in body.blocks.iter().enumerate() {
            let mut head = format!("  block {index}");
            let mut statements = block.statements.iter();
            if let Some(Statement::Charge(count)) = block.statements.first() {
                head.push_str(&format!(" charges {count}"));
                statements.next();
            }
            out.write(format!("{head}\n").as_bytes());
            for statement in statements {
                let text = self::statement(statement, &tables);
                out.write(format!("    {text}\n").as_bytes());
            }
            let text = terminator(&block.terminator, body, &tables);
            out.write(format!("    {text}\n").as_bytes());
        }
    }
}

/// What a body is for: the main, a definition, a function or a thunk.
fn role(program: &Program, body: usize, tables: &Tables) -> String {
    if program.main == body {
        return String::from("main");
    }
    if let Some(definition) = program
        .definitions
        .iter()
        .position(|known| *known == Some(body))
    {
        return format!("definition {}", tables.definitions[definition].label);
    }
    if let Some((module, _)) =
        program.inits.iter().find(|(_, init)| *init == body)
    {
        return format!("init {}", tables.modules[*module].label);
    }
    if let Some(test) =
        program.tests.iter().position(|known| *known == Some(body))
    {
        return tables
            .entries
            .iter()
            .find(|entry| mentions(&entry.predicate, test))
            .map_or_else(
                || format!("test {test}"),
                |entry| format!("test {}", entry.label),
            );
    }
    match &tables.nodes[program.bodies[body].node].kind {
        Kind::Function { .. } => String::from("function"),
        Kind::Call { .. } => String::from("partial call"),
        Kind::Declared(index) => {
            format!("declared {}", declared(*index, tables))
        }
        Kind::Constructor {
            meaning,
            constructor,
            ..
        } => format!("constructor {}", label(*meaning, *constructor, tables)),
        _ => String::from("body"),
    }
}

/// Whether a predicate composes the test.
fn mentions(predicate: &Predicate, test: usize) -> bool {
    match predicate {
        Predicate::Nonzero { .. } => false,
        Predicate::Function { parameter, result } => {
            mentions(parameter, test) || mentions(result, test)
        }
        Predicate::All(parts) | Predicate::Any(parts) => {
            parts.iter().any(|part| mentions(part, test))
        }
        Predicate::Not(inner) => mentions(inner, test),
        Predicate::Test { index, .. } => *index == test,
    }
}

const fn derived(kind: Derived) -> &'static str {
    match kind {
        Derived::Eq => "equality",
        Derived::Cmp => "comparison",
        Derived::Show => "display",
        Derived::Default => "default",
    }
}

fn declared(index: usize, tables: &Tables) -> String {
    let declaration = &tables.declarations[index];
    format!(
        "{}.{}",
        tables.namespaces[declaration.namespace], declaration.label
    )
}

fn label(meaning: usize, constructor: usize, tables: &Tables) -> String {
    let meaning = &tables.meanings[meaning];
    format!(
        "{}.{}",
        meaning.label, meaning.constructors[constructor].label
    )
}

fn operand(operand: &Operand, tables: &Tables) -> String {
    match operand {
        Operand::Local(index) => format!("_{index}"),
        Operand::Const(index) => format!("constant {index}"),
        Operand::Capture(index) => format!("capture {index}"),
        Operand::Global(index) => {
            format!("global {}", tables.definitions[*index].label)
        }
        Operand::Hidden(index) => format!("witness {index}"),
    }
}

fn operands(parts: &[Operand], tables: &Tables) -> String {
    parts
        .iter()
        .map(|part| operand(part, tables))
        .collect::<Vec<_>>()
        .join(", ")
}

fn meanings(parts: &[fipe::ir::Type], tables: &Tables) -> String {
    if parts.is_empty() {
        return String::new();
    }
    let parts: Vec<_> = parts.iter().map(|ty| meaning(ty, tables)).collect();
    format!(" with meanings ({})", parts.join(", "))
}

fn rvalue(rvalue: &Rvalue, tables: &Tables) -> String {
    match rvalue {
        Rvalue::Use(value) => operand(value, tables),
        Rvalue::Construct {
            meaning,
            constructor,
            fields,
        } => format!(
            "construct {} ({})",
            label(*meaning, *constructor, tables),
            operands(fields, tables)
        ),
        Rvalue::Tuple(parts) => format!("tuple ({})", operands(parts, tables)),
        Rvalue::Array(parts) => format!("array [{}]", operands(parts, tables)),
        Rvalue::Repeat { value, length } => format!(
            "repeat {}, length {}",
            operand(value, tables),
            match length {
                fipe::mir::Length::Fixed(length) => format!("{length}"),
                fipe::mir::Length::Hidden(index) => format!("witness {index}"),
            }
        ),
        Rvalue::Field(value, index) => {
            format!("field {index} of {}", operand(value, tables))
        }
        Rvalue::Length(value) => {
            format!("length of {}", operand(value, tables))
        }
        Rvalue::Closure {
            body,
            captures,
            meanings: hidden,
        } => format!(
            "closure of body {body} capturing ({}){}",
            operands(captures, tables),
            meanings(hidden, tables)
        ),
        Rvalue::Wrap {
            meaning,
            constructor,
            value,
        } => format!(
            "wrap {} {}",
            label(*meaning, *constructor, tables),
            operand(value, tables)
        ),
        Rvalue::Cast {
            row,
            from,
            to,
            value,
        } => format!(
            "cast {} from {} to {} by the {} row",
            operand(value, tables),
            meaning(from, tables),
            meaning(to, tables),
            match row {
                fipe::ir::CastRow::Number => "number",
                fipe::ir::CastRow::Read => "read",
                fipe::ir::CastRow::Show => "show",
                fipe::ir::CastRow::Codec => "codec",
                fipe::ir::CastRow::Bytes => "bytes",
            }
        ),
        Rvalue::Convert {
            mode,
            predicate,
            value,
        } => format!(
            "convert {} by {}{}",
            operand(value, tables),
            match mode {
                fipe::ir::ConversionMode::Direct => "direct",
                fipe::ir::ConversionMode::Wrap => "wrap",
                fipe::ir::ConversionMode::Unwrap => "unwrap",
                fipe::ir::ConversionMode::Check => "check",
                fipe::ir::ConversionMode::TryCheck => "try check",
            },
            self::predicate(predicate, tables)
        ),
        Rvalue::Pack { tag, value } => format!(
            "pack {} tagged {}",
            operand(value, tables),
            meaning(tag, tables)
        ),
        Rvalue::Native { op, args } => format!(
            "native {} ({})",
            declared(*op, tables),
            operands(args, tables)
        ),
    }
}

/// The tests a predicate composes, in words; nothing for one every
/// value passes.
fn predicate(predicate: &fipe::ir::Predicate, tables: &Tables) -> String {
    use fipe::ir::Predicate;
    fn words(predicate: &Predicate, tables: &Tables) -> Option<String> {
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                let parts: Vec<_> = parts
                    .iter()
                    .filter_map(|part| words(part, tables))
                    .collect();
                if parts.is_empty() {
                    return None;
                }
                let glue = if matches!(predicate, Predicate::All(_)) {
                    " and "
                } else {
                    " or "
                };
                Some(parts.join(glue))
            }
            Predicate::Not(part) => {
                words(part, tables).map(|part| format!("not {part}"))
            }
            Predicate::Test { index, .. } => {
                let test = &tables.tests[*index];
                Some(format!(
                    "test {index} of {}",
                    tables.namespaces[test.namespace]
                ))
            }
            Predicate::Function { .. } => Some(String::from("a function")),
            Predicate::Nonzero { .. } => Some(String::from("nonzero")),
        }
    }
    words(predicate, tables)
        .map_or_else(String::new, |text| format!(" testing {text}"))
}

fn statement(statement: &Statement, tables: &Tables) -> String {
    match statement {
        Statement::Assign(dest, value) => {
            format!("_{dest} = {}", rvalue(value, tables))
        }
        Statement::Charge(count) => format!("charge {count}"),
        Statement::Store { definition, value } => format!(
            "global {} = {}",
            tables.definitions[*definition].label,
            operand(value, tables)
        ),
        Statement::Enter { entry, value } => format!(
            "enter {} with {}",
            tables.entries[*entry].label,
            operand(value, tables)
        ),
        Statement::Push { key, value } => {
            format!("push {} = {}", tables.keys[*key], operand(value, tables))
        }
        Statement::Pop => String::from("pop"),
    }
}

fn callee(callee: &Callee, tables: &Tables) -> String {
    match callee {
        Callee::Body(body) => format!("body {body}"),
        Callee::Native(index) => format!("native {}", declared(*index, tables)),
        Callee::Host(index) => format!("host {}", declared(*index, tables)),
        Callee::Derived { kind, ty } => {
            format!("derived {} of {}", derived(*kind), meaning(ty, tables))
        }
        Callee::Dispatch { member, witness } => format!(
            "dispatch {} by {}",
            tables.dispatch[*member].label,
            match witness {
                Witness::Value(value) => {
                    format!("value {}", operand(value, tables))
                }
                Witness::Meaning(ty) =>
                    format!("meaning {}", meaning(ty, tables)),
            }
        ),
        Callee::Value(value) => format!("value {}", operand(value, tables)),
    }
}

fn terminator(
    terminator: &Terminator,
    body: &fipe::mir::Body,
    tables: &Tables,
) -> String {
    match terminator {
        Terminator::Goto(block) => format!("goto block {block}"),
        Terminator::Switch {
            value,
            cases,
            otherwise,
        } => {
            let ty = match value {
                Operand::Local(index) => Some(&body.locals[*index].ty),
                _ => None,
            };
            let cases: Vec<_> = cases
                .iter()
                .map(|(key, block)| {
                    let name = match ty {
                        Some(fipe::ir::Type::Meaning { index, .. })
                            if tables.meanings[*index].label == "Integer" =>
                        {
                            format!("{key}")
                        }
                        Some(fipe::ir::Type::Meaning { index, .. }) => tables
                            .meanings[*index]
                            .constructors
                            .get(*key)
                            .map_or_else(
                                || format!("case {key}"),
                                |constructor| constructor.label.clone(),
                            ),
                        Some(fipe::ir::Type::Packed(_)) => {
                            tables.meanings[*key].label.clone()
                        }
                        _ => format!("case {key}"),
                    };
                    format!("{name} to block {block}")
                })
                .chain(
                    otherwise
                        .iter()
                        .map(|block| format!("otherwise block {block}")),
                )
                .collect();
            format!("switch {}: {}", operand(value, tables), cases.join(", "))
        }
        Terminator::Test {
            value, pass, fail, ..
        } => format!(
            "test {}: pass block {pass}, fail block {fail}",
            operand(value, tables)
        ),
        Terminator::Call {
            callee: target,
            args,
            meanings: hidden,
            dest,
            target: block,
        } => format!(
            "call {} ({}){} into _{dest}, then block {block}",
            callee(target, tables),
            operands(args, tables),
            meanings(hidden, tables)
        ),
        Terminator::Return => String::from("return"),
        Terminator::Unreachable => String::from("unreachable"),
    }
}

/// A constant in the language's spelling.
fn value(value: &Value, tables: &Tables) -> String {
    match value {
        Value::Unit => String::from("()"),
        Value::Int(value) => format!("{value}"),
        Value::Big(big) => {
            format!("{}{}", if big.negative { "-" } else { "" }, big.magnitude)
        }
        Value::F32(value) => float(f64::from(*value)),
        Value::F64(value) => float(*value),
        Value::Text(text) => quoted(text),
        Value::Seq(parts) => format!("[{}]", values(parts, tables)),
        Value::Tuple(parts) => format!("({})", values(parts, tables)),
        Value::Record(record) => {
            let name = label(record.meaning, record.constructor, tables);
            if record.fields.is_empty() {
                name
            } else {
                let fields: Vec<_> = record
                    .fields
                    .iter()
                    .map(|field| {
                        let text = self::value(field, tables);
                        if text.contains(' ') {
                            format!("({text})")
                        } else {
                            text
                        }
                    })
                    .collect();
                format!("{name} {}", fields.join(" "))
            }
        }
        Value::Function(closure) => {
            format!("function of body {}", closure.body)
        }
        Value::Meaning(ty) => meaning(ty, tables),
        Value::Packed(packed) => format!(
            "{} {}",
            tables.meanings[packed.meaning].label,
            self::value(&packed.value, tables)
        ),
    }
}

fn values(parts: &[Value], tables: &Tables) -> String {
    parts
        .iter()
        .map(|part| value(part, tables))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The shortest spelling that reads back, always with a fraction part.
fn float(value: f64) -> String {
    let text = format!("{value}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
