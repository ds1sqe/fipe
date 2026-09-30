//! `:ast`: an entry's syntax as a tree, one node per line, children
//! indented two spaces; names and literals shown, arena ids never.

use alloc::{format, string::String, vec, vec::Vec};
use core::fmt::Write as _;
use fipe::ast::{
    ArgumentKind, Arm, BinaryOp, CallStyle, ConstraintKind, Entry, ExprKind,
    ImplMember, ItemKind, Limit, Literal, Module, Name, PatternKind, PipeOp,
    TypeKind,
};
use fipe::ast::{ExprId as E, PatternId as P, TypeId as T};

/// A child: a node of the module's arenas, or a label over children.
enum Node {
    E(E),
    P(P),
    T(T),
    L(String, Vec<Self>),
}

pub fn entry(module: &Module, entry: &Entry) -> String {
    let root = match entry {
        Entry::Line(id) => Node::E(*id),
        Entry::Item(item) => item_node(&item.kind),
    };
    let mut out = String::new();
    walk(module, root, 0, &mut out);
    out
}

fn walk(module: &Module, node: Node, depth: usize, out: &mut String) {
    let (label, children) = match node {
        Node::E(id) => module.expr(id).map(|e| expr(&e.kind)),
        Node::P(id) => module.pattern(id).map(|p| pattern(&p.kind)),
        Node::T(id) => module.ty(id).map(|t| ty(&t.kind)),
        Node::L(label, children) => Some((label, children)),
    }
    .unwrap_or_default();
    // Padding by repetition: a format width panics above 65535.
    out.push_str(&" ".repeat(depth * 2));
    let _ = writeln!(out, "{label}");
    for child in children {
        walk(module, child, depth + 1, out);
    }
}

fn dotted(name: &Name) -> String {
    name.segments.join(".")
}

const fn leaf(label: String) -> (String, Vec<Node>) {
    (label, Vec::new())
}

fn es(ids: &[E]) -> Vec<Node> {
    ids.iter().map(|id| Node::E(*id)).collect()
}

fn ts(ids: &[T]) -> Vec<Node> {
    ids.iter().map(|id| Node::T(*id)).collect()
}

fn expr(kind: &ExprKind) -> (String, Vec<Node>) {
    match kind {
        ExprKind::Literal(value) => leaf(literal(value)),
        ExprKind::Name(name) => leaf(format!("Name {}", dotted(name))),
        ExprKind::Word(word) => leaf(format!("Word {word}")),
        ExprKind::Path(path) => leaf(format!("Path {path}")),
        ExprKind::Selector(name) => leaf(format!("Selector .{}", dotted(name))),
        ExprKind::Group(lines) => ("Group".into(), es(lines)),
        ExprKind::Block(lines) => ("Block".into(), es(lines)),
        ExprKind::Tuple(parts) => ("Tuple".into(), es(parts)),
        ExprKind::List(items) => ("List".into(), es(items)),
        ExprKind::Outcome { good, payload } => {
            let side = if *good { ":" } else { "!" };
            (format!("Outcome [{side}]"), es(payload.as_slice()))
        }
        ExprKind::Instance { name, arguments } => (
            format!("Instance {}", dotted(name)),
            arguments.iter().map(|t| Node::T(*t)).collect(),
        ),
        ExprKind::Record { name, fields } => {
            let fields = fields.iter().map(|field| {
                let label = format!("Field {}", dotted(&field.name));
                Node::L(label, vec![Node::E(field.value)])
            });
            (format!("Record {}", dotted(name)), fields.collect())
        }
        ExprKind::Call {
            callee,
            arguments,
            style,
        } => {
            let mut children = vec![Node::E(*callee)];
            children.extend(arguments.iter().map(|a| match &a.kind {
                ArgumentKind::Positional(value) => Node::E(*value),
                ArgumentKind::Flag { name, value } => {
                    Node::L(format!("Flag -{name}"), es(value.as_slice()))
                }
            }));
            let touching = matches!(style, CallStyle::Parenthesized);
            let label = if touching { "Call ()" } else { "Call" };
            (label.into(), children)
        }
        ExprKind::Field { value, field } => {
            (format!("Field .{}", dotted(field)), vec![Node::E(*value)])
        }
        ExprKind::Not { value, .. } => ("Not".into(), vec![Node::E(*value)]),
        ExprKind::Binary {
            op, left, right, ..
        } => (format!("Binary {}", binary(*op)), es(&[*left, *right])),
        ExprKind::Pipe {
            op, left, right, ..
        } => (format!("Pipe {}", pipe(*op)), es(&[*left, *right])),
        ExprKind::Bind {
            public,
            pattern,
            annotation,
            value,
        } => {
            let mut children = vec![Node::P(*pattern)];
            children.extend(annotation.map(Node::T));
            children.push(Node::E(*value));
            let label = if *public { "Bind pub" } else { "Bind" };
            (label.into(), children)
        }
        ExprKind::With { key, value, body } => {
            (format!("With {}", dotted(key)), es(&[*value, *body]))
        }
        ExprKind::If {
            condition,
            then_body,
            else_body,
        } => {
            let mut children = es(&[*condition, *then_body]);
            children.extend(else_body.map(Node::E));
            ("If".into(), children)
        }
        ExprKind::Arms(arms) => ("Arms".into(), arms.iter().map(arm).collect()),
    }
}

fn arm(arm: &Arm) -> Node {
    let mut children: Vec<_> =
        arm.patterns.iter().map(|p| Node::P(*p)).collect();
    if let Some(guard) = arm.guard {
        children.push(Node::L("Guard".into(), vec![Node::E(guard)]));
    }
    children.push(Node::E(arm.body));
    Node::L("Arm".into(), children)
}

fn pattern(kind: &PatternKind) -> (String, Vec<Node>) {
    match kind {
        PatternKind::Wildcard => leaf("_".into()),
        PatternKind::Name(name) => leaf(format!("Name {}", dotted(name))),
        PatternKind::Literal(value) => leaf(literal(value)),
        PatternKind::Constructor { name, payload } => (
            format!("Constructor {}", dotted(name)),
            vec![Node::P(*payload)],
        ),
        PatternKind::Default { name, value } => {
            (format!("Default {}", dotted(name)), vec![Node::E(*value)])
        }
        PatternKind::Group(inner) => ("Group".into(), vec![Node::P(*inner)]),
        PatternKind::Tuple(parts) => {
            ("Tuple".into(), parts.iter().map(|p| Node::P(*p)).collect())
        }
        PatternKind::List(items) => {
            ("List".into(), items.iter().map(|p| Node::P(*p)).collect())
        }
        PatternKind::Outcome { good, payload } => {
            let side = if *good { ":" } else { "!" };
            (
                format!("Outcome [{side}]"),
                payload.iter().map(|p| Node::P(*p)).collect(),
            )
        }
        PatternKind::Record { name, fields, rest } => {
            let mut children: Vec<_> = fields
                .iter()
                .map(|field| {
                    let label = format!("Field {}", dotted(&field.name));
                    Node::L(
                        label,
                        field.pattern.iter().map(|p| Node::P(*p)).collect(),
                    )
                })
                .collect();
            children.extend(rest.map(|_| Node::L("..".into(), Vec::new())));
            (format!("Record {}", dotted(name)), children)
        }
    }
}

fn ty(kind: &TypeKind) -> (String, Vec<Node>) {
    match kind {
        TypeKind::Unit => leaf("Type ()".into()),
        TypeKind::Named { name, arguments } => (
            format!("Type {}", dotted(name)),
            arguments.iter().map(|t| Node::T(*t)).collect(),
        ),
        TypeKind::Array { element, length } => {
            ("Array".into(), vec![Node::T(*element), Node::T(*length)])
        }
        TypeKind::Number(value) => leaf(format!("Number {value}")),
        TypeKind::Tuple(parts) => ("Tuple".into(), ts(parts)),
        TypeKind::Both(parts) => ("Both &".into(), ts(parts)),
        TypeKind::Either(parts) => ("Either |".into(), ts(parts)),
        TypeKind::Not(inner) => ("Not !".into(), vec![Node::T(*inner)]),
        TypeKind::Function { parameter, result } => (
            "Function".into(),
            vec![Node::T(*parameter), Node::T(*result)],
        ),
        TypeKind::Qualified { constraints, ty } => {
            let mut children: Vec<_> = constraints
                .iter()
                .map(|c| match &c.kind {
                    ConstraintKind::Named { class, variable } => {
                        let label = format!("Constraint {}", dotted(variable));
                        Node::L(label, vec![Node::T(*class)])
                    }
                    ConstraintKind::Member { member, ty } => {
                        let label = format!("Constraint {}", dotted(member));
                        Node::L(label, vec![Node::T(*ty)])
                    }
                })
                .collect();
            children.push(Node::T(*ty));
            ("Qualified".into(), children)
        }
        TypeKind::Labeled { name, ty, default } => {
            let mut children = vec![Node::T(*ty)];
            children.extend(default.map(Node::E));
            (format!("Labeled {}", dotted(name)), children)
        }
    }
}

/// Items show their names; an impl also its members' values.
fn item_node(kind: &ItemKind) -> Node {
    let label = |word: &str, name: &Name| format!("{word} {}", dotted(name));
    match kind {
        ItemKind::Use { names, .. } => {
            let names: Vec<_> = names.iter().map(dotted).collect();
            Node::L(format!("Use {}", names.join(" ")), Vec::new())
        }
        ItemKind::Type {
            name,
            parameters,
            base,
            limits,
            ..
        } => {
            let mut children: Vec<_> = parameters
                .iter()
                .map(|p| {
                    let mut bound: Vec<_> =
                        p.bound.iter().map(|t| Node::T(*t)).collect();
                    if let Some(default) = p.default {
                        bound.push(Node::L(
                            "Default".into(),
                            vec![Node::T(default)],
                        ));
                    }
                    Node::L(label("Parameter", &p.name), bound)
                })
                .collect();
            children
                .extend(base.map(|t| Node::L("Base".into(), vec![Node::T(t)])));
            children.extend(limits.iter().map(|limit| match limit {
                Limit::Member { name, ty, .. } => {
                    Node::L(label("Where", name), vec![Node::T(*ty)])
                }
                Limit::Test(test) => {
                    Node::L("Where".into(), vec![Node::E(*test)])
                }
            }));
            Node::L(label("Type", name), children)
        }
        ItemKind::Declare { name, ty } => {
            Node::L(label("Declare", name), vec![Node::T(*ty)])
        }
        ItemKind::Impl {
            target, members, ..
        } => {
            let mut children = vec![Node::T(*target)];
            children.extend(members.iter().map(|member| match member {
                ImplMember::Named {
                    name,
                    annotation,
                    value,
                    ..
                } => {
                    let mut children: Vec<_> =
                        annotation.iter().map(|t| Node::T(*t)).collect();
                    children.extend(value.map(Node::E));
                    Node::L(label("Member", name), children)
                }
                ImplMember::Constructor(member) => arm(member),
            }));
            Node::L("Impl".into(), children)
        }
    }
}

fn literal(literal: &Literal) -> String {
    match literal {
        Literal::Unit => "()".into(),
        Literal::Int {
            negative,
            magnitude,
        } => format!("Int {}{magnitude}", if *negative { "-" } else { "" }),
        Literal::Float(spelling) => format!("Float {spelling}"),
        Literal::Str { value, .. } => {
            format!("Str \"{}\"", value.escape_default())
        }
        Literal::Affixed(affixed) => format!("Affixed {affixed}"),
    }
}

const fn binary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Remainder => "%",
        BinaryOp::Concat => "++",
        BinaryOp::Equal => "==",
        BinaryOp::NotEqual => "!=",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
    }
}

const fn pipe(op: PipeOp) -> &'static str {
    match op {
        PipeOp::Plain => "|",
        PipeOp::Good => "|:",
        PipeOp::Bad => "|!",
        PipeOp::Or => "||",
        PipeOp::And => "&&",
    }
}
