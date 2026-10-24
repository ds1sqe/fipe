//! The grammar's trees against `fipe::parse`: for every source the crate
//! accepts, the named nodes of the stated kinds agree by kind and byte
//! span; binary operators and pipes agree by operator span, so nesting
//! direction is not asserted. For a source the crate rejects, the tree
//! has an error or missing node overlapping the crate's first
//! diagnostic.

use fipe::ast::{
    ArgumentKind, Arm, Entry, ExprId, ExprKind, ImplMember, ItemKind, Limit,
    Literal, Member, Module, Name, PatternId, PatternKind, TypeId, TypeKind,
};
use fipe::lex::lex;
use fipe::parse::parse;
use fipe::token::Written;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A compared node: its oracle kind and byte span.
type Node = (&'static str, usize, usize);

/// The language repository holding the corpus: `FIPE_ROOT`, else the
/// checkout this package sits in.
fn root() -> PathBuf {
    std::env::var_os("FIPE_ROOT").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    )
}

// The crate's side: every node of the compared kinds, from the module's
// entries down.

struct Crate<'a> {
    module: &'a Module,
    nodes: Vec<Node>,
}

impl Crate<'_> {
    fn push(&mut self, kind: &'static str, span: fipe::token::Span) {
        self.nodes.push((kind, span.start, span.end));
    }

    fn name(&mut self, name: &Name) {
        let kind = if name.is_uppercase() {
            "upper_name"
        } else {
            "name"
        };
        self.push(kind, name.span);
    }

    fn literal(&mut self, literal: &Literal, span: fipe::token::Span) {
        let kind = match literal {
            Literal::Unit => "unit",
            Literal::Int { .. } => "integer",
            Literal::Float(_) => "float",
            Literal::Str { raw: false, .. } => "string",
            Literal::Str { raw: true, .. } => "raw_string",
            Literal::Affixed(affixed) => match &affixed.form {
                Written::Int { .. } | Written::Digits { .. } => "integer",
                Written::Float(_) => "float",
                Written::Text { raw: false, .. } => "string",
                Written::Text { raw: true, .. } => "raw_string",
            },
        };
        self.push(kind, span);
    }

    fn exprs(&mut self, ids: &[ExprId]) {
        for id in ids {
            self.expr(*id);
        }
    }

    fn patterns(&mut self, ids: &[PatternId]) {
        for id in ids {
            self.pattern(*id);
        }
    }

    fn types(&mut self, ids: &[TypeId]) {
        for id in ids {
            self.ty(*id);
        }
    }

    fn names(&mut self, names: &[Name]) {
        for name in names {
            self.name(name);
        }
    }

    fn fields(&mut self, fields: &[fipe::ast::Field]) {
        for field in fields {
            self.field(field);
        }
    }

    fn arms(&mut self, arms: &[Arm]) {
        for arm in arms {
            self.arm(arm);
        }
    }

    fn arm(&mut self, arm: &Arm) {
        self.push("arm", arm.span);
        self.patterns(&arm.patterns);
        if let Some(guard) = arm.guard {
            self.expr(guard);
        }
        self.expr(arm.body);
    }

    fn expr(&mut self, id: ExprId) {
        let expr = self.module.expr(id).expect("an expression");
        let span = expr.span;
        match &expr.kind {
            ExprKind::Literal(literal) => self.literal(literal, span),
            ExprKind::Name(name) => self.name(name),
            ExprKind::Word(_) => self.push("word", span),
            ExprKind::Path(_) => self.push("path", span),
            ExprKind::Selector(_) => self.push("selector", span),
            ExprKind::Group(lines) => {
                self.push("group", span);
                self.exprs(lines);
            }
            ExprKind::Tuple(parts) => {
                self.push("tuple", span);
                self.exprs(parts);
            }
            ExprKind::List(elements) => {
                self.push("list", span);
                self.exprs(elements);
            }
            ExprKind::Outcome { payload, .. } => {
                self.push("outcome", span);
                if let Some(payload) = payload {
                    self.expr(*payload);
                }
            }
            ExprKind::Instance { name, arguments } => {
                self.push("instance", span);
                self.name(name);
                self.types(arguments);
            }
            ExprKind::Record { name, fields } => {
                self.record(span, name, fields);
            }
            ExprKind::Call {
                callee, arguments, ..
            } => self.call(span, *callee, arguments),
            ExprKind::Field { value, .. } => {
                self.push("field", span);
                self.expr(*value);
            }
            ExprKind::Not { value, .. } => {
                self.push("not", span);
                self.expr(*value);
            }
            ExprKind::Binary {
                operator,
                left,
                right,
                ..
            } => self.operator("binary", *operator, &[*left, *right]),
            ExprKind::Pipe {
                operator,
                left,
                right,
                ..
            } => self.operator("pipe", *operator, &[*left, *right]),
            ExprKind::Bind {
                pattern,
                annotation,
                value,
                ..
            } => {
                self.push("binding", span);
                self.pattern(*pattern);
                if let Some(annotation) = annotation {
                    self.ty(*annotation);
                }
                self.expr(*value);
            }
            ExprKind::With { key, value, body } => {
                self.push("with", span);
                self.name(key);
                self.expr(*value);
                self.expr(*body);
            }
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.push("if", span);
                self.expr(*condition);
                self.expr(*then_body);
                if let Some(else_body) = else_body {
                    self.expr(*else_body);
                }
            }
            ExprKind::Block(lines) => {
                self.push("block", span);
                self.exprs(lines);
            }
            ExprKind::Arms(arms) => {
                self.push("arms", span);
                self.arms(arms);
            }
        }
    }

    /// A binary operator or a pipe, compared by its operator's span.
    fn operator(
        &mut self,
        kind: &'static str,
        operator: fipe::token::Span,
        operands: &[ExprId],
    ) {
        self.push(kind, operator);
        self.exprs(operands);
    }

    fn record(
        &mut self,
        span: fipe::token::Span,
        name: &Name,
        fields: &[fipe::ast::FieldValue],
    ) {
        self.push("record", span);
        self.name(name);
        for field in fields {
            self.push("field_value", field.span);
            self.name(&field.name);
            let punned = self
                .module
                .expr(field.value)
                .is_some_and(|value| value.span == field.name.span);
            if !punned {
                self.expr(field.value);
            }
        }
    }

    fn call(
        &mut self,
        span: fipe::token::Span,
        callee: ExprId,
        arguments: &[fipe::ast::Argument],
    ) {
        self.push("call", span);
        self.expr(callee);
        for argument in arguments {
            match &argument.kind {
                ArgumentKind::Positional(value) => self.expr(*value),
                ArgumentKind::Flag { value, .. } => {
                    self.push("flag", argument.span);
                    if let Some(value) = value {
                        self.expr(*value);
                    }
                }
            }
        }
    }

    fn pattern(&mut self, id: PatternId) {
        let pattern = self.module.pattern(id).expect("a pattern");
        let span = pattern.span;
        match &pattern.kind {
            PatternKind::Wildcard => self.push("name", span),
            PatternKind::Name(name) => self.name(name),
            PatternKind::Literal(literal) => self.literal(literal, span),
            PatternKind::Constructor { name, payload } => {
                self.push("constructor_pattern", span);
                self.name(name);
                self.pattern(*payload);
            }
            PatternKind::Default { name, value } => {
                self.push("default_pattern", span);
                self.name(name);
                self.expr(*value);
            }
            PatternKind::Group(inner) => {
                self.push("group_pattern", span);
                self.pattern(*inner);
            }
            PatternKind::Tuple(parts) => {
                self.push("tuple_pattern", span);
                self.patterns(parts);
            }
            PatternKind::List(elements) => {
                self.push("list_pattern", span);
                self.patterns(elements);
            }
            PatternKind::Outcome { payload, .. } => {
                self.push("outcome_pattern", span);
                if let Some(payload) = payload {
                    self.pattern(*payload);
                }
            }
            PatternKind::Record { name, fields, rest } => {
                self.push("record_pattern", span);
                self.name(name);
                for field in fields {
                    self.push("pattern_field", field.span);
                    self.name(&field.name);
                    if let Some(inner) = field.pattern {
                        self.pattern(inner);
                    }
                }
                if let Some(rest) = rest {
                    self.push("rest_pattern", *rest);
                }
            }
        }
    }

    fn ty(&mut self, id: TypeId) {
        let ty = self.module.ty(id).expect("a type");
        let span = ty.span;
        match &ty.kind {
            TypeKind::Unit => self.push("unit_type", span),
            TypeKind::Named { name, arguments } => {
                self.push("named_type", span);
                self.name(name);
                self.types(arguments);
            }
            TypeKind::Array { element, length } => {
                self.push("array_type", span);
                self.ty(*element);
                self.ty(*length);
            }
            TypeKind::Number(_) => self.push("integer", span),
            TypeKind::Sum(parts) => {
                self.push("number_sum", span);
                self.types(parts);
            }
            TypeKind::Tuple(parts) => {
                self.push("tuple_type", span);
                self.types(parts);
            }
            TypeKind::Both(parts) => {
                self.push("both_type", span);
                self.types(parts);
            }
            TypeKind::Either(parts) => {
                self.push("either_type", span);
                self.types(parts);
            }
            TypeKind::Not(inner) => {
                self.push("not_type", span);
                self.ty(*inner);
            }
            TypeKind::Function { parameter, result } => {
                self.push("function_type", span);
                self.ty(*parameter);
                self.ty(*result);
            }
            TypeKind::Qualified { constraints, ty } => {
                self.push("qualified_type", span);
                for constraint in constraints {
                    self.push("constraint", constraint.span);
                    match &constraint.kind {
                        fipe::ast::ConstraintKind::Named {
                            class,
                            variable,
                        } => {
                            self.ty(*class);
                            self.name(variable);
                        }
                        fipe::ast::ConstraintKind::Member { member, ty } => {
                            self.name(member);
                            self.ty(*ty);
                        }
                    }
                }
                self.ty(*ty);
            }
            TypeKind::Labeled { name, ty, default } => {
                self.push("labeled_type", span);
                self.name(name);
                self.ty(*ty);
                if let Some(default) = default {
                    self.expr(*default);
                }
            }
        }
    }

    fn field(&mut self, field: &fipe::ast::Field) {
        self.push("field_declaration", field.span);
        self.name(&field.name);
        self.ty(field.ty);
        if let Some(default) = field.default {
            self.expr(default);
        }
    }

    fn entry(&mut self, entry: &Entry) {
        let item = match entry {
            Entry::Line(line) => return self.expr(*line),
            Entry::Item(item) => item,
        };
        match &item.kind {
            ItemKind::Use { names, .. } => {
                self.push("import", item.span);
                self.names(names);
            }
            ItemKind::Type {
                resource: _,
                name,
                parameters,
                base,
                members,
                limits,
            } => {
                self.push("type_item", item.span);
                self.name(name);
                for parameter in parameters {
                    self.push("type_parameter", parameter.span);
                    self.name(&parameter.name);
                    if let Some(bound) = parameter.bound {
                        self.ty(bound);
                    }
                    if let Some(default) = parameter.default {
                        self.ty(default);
                    }
                }
                if let Some(base) = base {
                    self.ty(*base);
                }
                for member in members.iter().flatten() {
                    match member {
                        Member::Field(field) => self.field(field),
                        Member::Constructor {
                            name, fields, span, ..
                        } => {
                            self.push("constructor", *span);
                            self.name(name);
                            self.fields(fields);
                        }
                    }
                }
                for limit in limits {
                    match limit {
                        Limit::Member { name, ty, span } => {
                            self.push("member_limit", *span);
                            self.name(name);
                            self.ty(*ty);
                        }
                        Limit::Test(test) => self.expr(*test),
                    }
                }
            }
            ItemKind::Impl {
                target,
                claim,
                members,
            } => {
                self.push("impl_item", item.span);
                self.ty(*target);
                if let Some(claim) = claim {
                    self.ty(*claim);
                }
                for member in members {
                    match member {
                        ImplMember::Named {
                            name,
                            annotation,
                            value,
                            span,
                            ..
                        } => {
                            self.push("impl_member", *span);
                            self.name(name);
                            if let Some(annotation) = annotation {
                                self.ty(*annotation);
                            }
                            if let Some(value) = value {
                                self.expr(*value);
                            }
                        }
                        ImplMember::Constructor { arm, .. } => self.arm(arm),
                    }
                }
            }
            ItemKind::Declare { name, ty } => {
                self.push("declaration", item.span);
                self.name(name);
                self.ty(*ty);
            }
        }
    }
}

/// The crate's nodes, or its first diagnostic's span when it rejects the
/// source.
fn expected(text: &str) -> Result<Vec<Node>, (usize, usize)> {
    let module = parse(lex(text)).map_err(|errors| {
        let first = errors.first().expect("a rejection has an error");
        (first.span.start, first.span.end)
    })?;
    let mut walk = Crate {
        module: &module,
        nodes: Vec::new(),
    };
    for entry in &module.entries {
        walk.entry(entry);
    }
    let mut nodes = walk.nodes;
    nodes.sort_unstable();
    Ok(nodes)
}

// The grammar's side: named nodes mapped to the crate's kinds.

/// The oracle kind of a grammar node; `None` for a node not compared.
fn kind(node: &str) -> Option<&'static str> {
    Some(match node {
        "name" => "name",
        "upper_name" => "upper_name",
        "word" => "word",
        "path" => "path",
        "selector" => "selector",
        "integer" => "integer",
        "float" => "float",
        "string" => "string",
        "raw_string" => "raw_string",
        "unit" => "unit",
        "group" => "group",
        "tuple" => "tuple",
        "list" => "list",
        "outcome" => "outcome",
        "instance" => "instance",
        "record" => "record",
        "field_value" => "field_value",
        "call" | "application" => "call",
        "flag_argument" => "flag",
        "field" => "field",
        "not" => "not",
        "binary" => "binary",
        "pipe" => "pipe",
        "binding" => "binding",
        "with" => "with",
        "if" => "if",
        "block" => "block",
        "arms" => "arms",
        "arm" => "arm",
        "constructor_pattern" => "constructor_pattern",
        "default_pattern" => "default_pattern",
        "group_pattern" => "group_pattern",
        "tuple_pattern" => "tuple_pattern",
        "list_pattern" => "list_pattern",
        "outcome_pattern" => "outcome_pattern",
        "record_pattern" => "record_pattern",
        "pattern_field" => "pattern_field",
        "rest_pattern" => "rest_pattern",
        "unit_type" => "unit_type",
        "named_type" => "named_type",
        "array_type" => "array_type",
        "number_sum" => "number_sum",
        "tuple_type" => "tuple_type",
        "both_type" => "both_type",
        "either_type" => "either_type",
        "not_type" => "not_type",
        "function_type" => "function_type",
        "qualified_type" => "qualified_type",
        "constraint" => "constraint",
        "labeled_type" => "labeled_type",
        "import" => "import",
        "type_item" => "type_item",
        "type_parameter" => "type_parameter",
        "field_declaration" => "field_declaration",
        "constructor" => "constructor",
        "member_limit" => "member_limit",
        "impl_item" => "impl_item",
        "impl_member" => "impl_member",
        "declaration" => "declaration",
        _ => return None,
    })
}

fn tree(text: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_fipe::LANGUAGE.into())
        .expect("the Fipe grammar loads");
    parser.parse(text, None).expect("a tree")
}

/// Every node of the tree, depth first.
fn walk(tree: &tree_sitter::Tree, mut visit: impl FnMut(tree_sitter::Node)) {
    let mut cursor = tree.walk();
    loop {
        visit(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

/// The grammar's nodes, or the first error or missing node.
fn found(text: &str) -> Result<Vec<Node>, String> {
    let tree = tree(text);
    let mut nodes = Vec::new();
    let mut error = None;
    walk(&tree, |node| {
        if (node.is_error() || node.is_missing()) && error.is_none() {
            error = Some(format!(
                "{} at {}..{}",
                if node.is_missing() {
                    "missing"
                } else {
                    "error"
                },
                node.start_byte(),
                node.end_byte()
            ));
        }
        if !node.is_named() {
            return;
        }
        let Some(kind) = kind(node.kind()) else {
            return;
        };
        let span = match kind {
            "binary" | "pipe" => {
                node.child_by_field_name("operator").map_or_else(
                    || (node.start_byte(), node.end_byte()),
                    |operator| (operator.start_byte(), operator.end_byte()),
                )
            }
            _ => (node.start_byte(), node.end_byte()),
        };
        nodes.push((kind, span.0, span.1));
    });
    if let Some(error) = error {
        return Err(error);
    }
    nodes.sort_unstable();
    Ok(nodes)
}

/// Whether the tree has an error or missing node in the crate's
/// diagnostic's window: from the start of the line holding the last
/// token before the diagnostic (blank and comment-only lines hold none),
/// so that a recovery which flags the operator before a blank line still
/// counts, through its end.
fn error_at(text: &str, span: (usize, usize)) -> bool {
    let mut line = text[..span.0].rfind('\n').map_or(0, |at| at + 1);
    while line > 0 {
        let content = text[line..span.0.max(line)].trim();
        if !(content.is_empty() || content.starts_with('#')) {
            break;
        }
        line = text[..line - 1].rfind('\n').map_or(0, |at| at + 1);
    }
    let window = (line.min(span.0), span.1.max(span.0 + 1));
    let tree = tree(text);
    let mut hit = false;
    walk(&tree, |node| {
        if node.is_error() || node.is_missing() {
            let (start, end) = (node.start_byte(), node.end_byte());
            if start <= window.1 && window.0 <= end {
                hit = true;
            }
        }
    });
    hit
}

/// The differences between two sorted node lists, for a failure message.
fn difference(expected: &[Node], found: &[Node], text: &str) -> String {
    let show = |node: &Node| {
        let snippet = text.get(node.1..node.2).unwrap_or("?");
        let snippet: String = snippet.chars().take(40).collect();
        format!("{} {}..{} {:?}", node.0, node.1, node.2, snippet)
    };
    let mut out = String::new();
    for node in expected {
        if !found.contains(node) {
            let _ = writeln!(out, "  crate only: {}", show(node));
        }
    }
    for node in found {
        if !expected.contains(node) {
            let _ = writeln!(out, "  grammar only: {}", show(node));
        }
    }
    out
}

/// The outcome of comparing one source.
enum Verdict {
    Equal(usize),
    Rejected,
}

fn compare(label: &str, text: &str) -> Result<Verdict, String> {
    match expected(text) {
        Ok(expected) => {
            let found = found(text)
                .map_err(|error| format!("{label}: {error}\n{text}"))?;
            if expected == found {
                Ok(Verdict::Equal(expected.len()))
            } else {
                Err(format!(
                    "{label}: trees differ\n{}{text}",
                    difference(&expected, &found, text)
                ))
            }
        }
        Err(span) => {
            if error_at(text, span) {
                Ok(Verdict::Rejected)
            } else {
                Err(format!(
                    "{label}: the crate rejects it at {}..{}, the grammar \
                     has no error there\n{text}",
                    span.0, span.1
                ))
            }
        }
    }
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "fipe") {
            out.push(path);
        }
    }
}

/// The `fipe` blocks of the specification's chapters, with their labels.
fn blocks() -> Vec<(String, String)> {
    let dir = root().join("Documentation/spec");
    let mut chapters: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    chapters.sort();
    let mut out = Vec::new();
    for chapter in chapters {
        let text = std::fs::read_to_string(&chapter).unwrap();
        let mut block: Option<(usize, String)> = None;
        for (number, line) in text.lines().enumerate() {
            match &mut block {
                None if line.trim_start() == "```fipe" => {
                    block = Some((number + 1, String::new()));
                }
                Some((start, body)) if line.trim_start() == "```" => {
                    let name = chapter.file_name().unwrap().to_string_lossy();
                    out.push((format!("{name}:{start}"), body.clone()));
                    block = None;
                }
                Some((_, body)) => {
                    body.push_str(line);
                    body.push('\n');
                }
                None => {}
            }
        }
    }
    out
}

/// Run every source; panic with every failure at once.
fn run(sources: &[(String, String)]) -> (usize, usize, usize) {
    let (mut equal, mut rejected, mut nodes) = (0, 0, 0);
    let mut failures = Vec::new();
    for (label, text) in sources {
        match compare(label, text) {
            Ok(Verdict::Equal(count)) => {
                equal += 1;
                nodes += count;
            }
            Ok(Verdict::Rejected) => rejected += 1,
            Err(failure) => failures.push(failure),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} sources fail:\n\n{}",
        failures.len(),
        sources.len(),
        failures.join("\n")
    );
    (equal, rejected, nodes)
}

#[test]
fn the_corpus_files_parse_as_the_crate_does() {
    let root = root();
    let mut paths = Vec::new();
    for dir in ["Documentation", "prelude", "tests/fixtures"] {
        files(&root.join(dir), &mut paths);
    }
    assert!(
        paths.len() >= 15,
        "corpus not found under {}",
        root.display()
    );
    let sources: Vec<_> = paths
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            (path.display().to_string(), text)
        })
        .collect();
    let (equal, rejected, nodes) = run(&sources);
    assert_eq!(rejected, 0, "the crate rejects a corpus file");
    eprintln!("corpus: {equal} files, {nodes} nodes equal");
}

#[test]
fn the_chapter_blocks_parse_as_the_crate_does() {
    let sources = blocks();
    assert!(sources.len() >= 15, "chapter blocks not found");
    let (equal, rejected, nodes) = run(&sources);
    eprintln!("chapters: {equal} blocks equal ({nodes} nodes), {rejected} rejected with an error node");
}

/// Rows for every form the plan names, the continuation rules and the
/// malformed rows; each runs as its own source.
const ROWS: &[&str] = &[
    // The root path and division.
    "ls /",
    "ls / b",
    "cat (/) other",
    "a / b",
    // A blank line after a pipe ends the line.
    "a |\n\nb",
    // The `if` and `with` heads.
    "if a { b } else { c }",
    "if a { b } else if c { d } else { e }",
    "if f x { y }",
    "if (Point { x: 1 }) == p { b }",
    "if Point { x: 1 } == p { b }",
    "with cwd ./x { ls }",
    "x = if a { 1 } else { 2 }",
    // Continuation.
    "x = 1 +\n  2",
    "x = a\n  | f",
    "x = a\n\n| f",
    "x =\n  1",
    "x =\n\n  1",
    "f = a b if a > b => a,\n  _ b => b",
    "[a,\n  b,\n]",
    "[a\n  b]",
    "f(x,\n  y)",
    "f(x\n  , y)",
    "if a { b }\n\nelse { c }",
    "if a { b }\n# note\nelse { c }",
    "type T: Integer\nwhere self > 0",
    "type T: Integer where\n  self > 0,\n  self < 9",
    "x = a\n  + b\n  - c",
    "x = a\n  == b",
    "x = a\n  / b",
    "ls\n/",
    "x = a |\n  # note\n  b",
    "x = a\n  -1",
    "x = a\n  -b",
    "x = a\n  !b",
    "f: A -> B\n  = x => x",
    "f: A ->\n  B = x => x",
    // Bindings.
    "[x, y] = e",
    "Just x = e",
    "x: I64 = e",
    "id: Integer | String = 3",
    "cmd -c=80",
    "x => y = 1",
    "pub x = 1",
    // Arms.
    "max = a b if a > b => a, _ b => b",
    "f = Just e rest => e",
    "g = Just Just e => e",
    "find x | { [:e] => e.name, [!] => \"-\" }",
    "find x | ([:e] => e.name, [!] => \"-\")",
    "h = (n = 1) => n",
    "find x | {\n  [:e] if e.size > 0 => e.name,\n  [:_] => \"empty\",\n  [!] => \"-\",\n}",
    "x => y => z",
    "[x => 1]",
    // Applications, calls and flags.
    "ls -l -c=80 -c 80 /etc",
    "f(a,b)",
    "f()",
    "f (x)",
    "f (a, b)",
    "f (x,y)",
    "(e).name",
    "f().name",
    "Point(3, 4)",
    "Point -y 4 -x 3",
    "Point.line 2",
    "map .name",
    "add 2 -1",
    "add (2 - 1)",
    "(cat x) > y",
    "add 2 - 1",
    "cat x > y",
    "n * fact (n - 1)",
    // Instances.
    "read<I64> \"3\"",
    "x: Rd<fs> = Rd 4",
    "to<Point> p",
    "a<b",
    "a < b",
    // Outcomes.
    "[:e]",
    "[:]",
    "[!e]",
    "[!]",
    "[(!x)]",
    "[:e] = o",
    // Records, groups, blocks, lists, tuples.
    "Point { x: 1, y: 2 }",
    "Point { x, y }",
    "map { [:e] => e.name, [!] => \"-\" }",
    "(x = 1\n x + 1)",
    "{ a\n  b }",
    "{}",
    "()",
    "(a, b)",
    "(a, b,)",
    "[]",
    "[1, 2, 3]",
    "f (\n  g\n)",
    // Precedence.
    "!a",
    "!!a",
    "a * b + c",
    "a + b * c",
    "a == b",
    "a ++ b",
    "a | b |: c |! d || e && f",
    "a - b - c",
    "a < b < c",
    "a == b == c",
    "a < b == c",
    "!a * b",
    "x = -y",
    "if\n  a { b }",
    "type\nT",
    // Patterns.
    "(a, b) = t",
    "Pt { x, y: 1, .. } = p",
    "(x) = y",
    "\"a\" = x",
    "_ = x",
    "f = (a, b) [c, d] => a",
    // Items.
    "use fs",
    "use x.inc x.Point",
    "use text 2",
    "type Point { pub x: I64, y: I64 = 0 }",
    "type Shape { Circle(r: F64), Rect(w: F64, h: F64), Dot }",
    "type Rd<Api>",
    "type Box<T: Ord = I64> { v: T }",
    "type T: Integer where self > 0",
    "type V: Vec<U8> where self.len < 5",
    "type S { a: I64 } where self.a > 0",
    "impl Point { x (y = 0) => Self { x, y }, pub origin = Self 0 0, show: Self -> String }",
    "impl Point: Show { show = self => \"p\" }",
    "pub len: Vec<a> -> Natural",
    "pub show: Show a => a -> String",
    "pub age: b.age: a => b -> a",
    "ls: (path: Src = ./) -> (long: Bool = False) -> Vec<Entry>",
    "x: A * B -> C = f",
    "x: A & B | !C = y",
    "x: [U8; 4] = y",
    "x: [T; n + 1] = y",
    "x: () -> T = f",
    "x: (A -> B) -> C = f",
    "x: Bits<8> = y",
    "## d\nx = 1",
    "##! m\nx = 1",
    "type P {\n  ## x\n  pub x: I64,\n}",
];

#[test]
fn the_rows_parse_as_the_crate_does() {
    let sources: Vec<_> = ROWS
        .iter()
        .map(|row| ((*row).to_string(), (*row).to_string()))
        .collect();
    let (equal, rejected, nodes) = run(&sources);
    eprintln!("rows: {equal} equal ({nodes} nodes), {rejected} rejected with an error node");
}

/// The modifier is contextual, including its raw-token boundaries.
#[test]
fn resource_headers_match_core_flags_and_name_spans() {
    let rows = [
        "pub type resource Table",
        "type resource Rd<Api>",
        "type resource data",
        "# café\r\npub type resource Table\r\ntype Next",
        "type resource\tTable",
        "type resource\u{2003}Table",
        "type resource \\\nTable",
        "type \\\nresource Table",
        "type resource Table: Integer",
        "type resource Table { value: Integer }",
        "type resource Table where self > 0",
        "resource = 1",
        "f = resource => resource",
        "type Data { resource: Integer }",
        "type resource",
        "type resource<A>",
        "type resource: Integer",
        "type resource { value: Integer }",
        "type resource where self > 0",
        "type resource\nTable",
        "type resource # comment\nTable",
        "type resource; Table",
        "type resource\nwhere self > 0",
        "type\nresource Table",
        "type resource api.Table",
        "type resource ## doc\nTable",
    ];
    for text in rows {
        compare(text, text).unwrap();
        let Ok(module) = parse(lex(text)) else {
            continue;
        };
        let expected: Vec<_> = module
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(fipe::ast::Item {
                    kind: ItemKind::Type { resource, name, .. },
                    ..
                }) => Some((*resource, name.span.start, name.span.end)),
                _ => None,
            })
            .collect();
        let parsed = tree(text);
        let mut actual = Vec::new();
        walk(&parsed, |node| {
            if node.kind() == "type_item" {
                let name = node.child_by_field_name("name").unwrap();
                let modifier = node.child_by_field_name("resource");
                if let Some(modifier) = modifier {
                    assert_eq!(modifier.kind(), "resource_modifier");
                    assert_eq!(&text[modifier.byte_range()], "resource");
                }
                actual.push((
                    modifier.is_some(),
                    name.start_byte(),
                    name.end_byte(),
                ));
            }
        });
        assert_eq!(expected, actual, "{text}");
    }
}

/// The inputs of `test/corpus`, each with whether it is marked `:error`.
fn corpus_tests() -> Vec<(String, String, bool)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test/corpus");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    let rule = format!("{}\n", "=".repeat(80));
    let dashes = format!("\n{}\n", "-".repeat(80));
    let mut out = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut sections = text.split(rule.as_str());
        sections.next();
        while let (Some(header), Some(body)) =
            (sections.next(), sections.next())
        {
            let error = header.lines().any(|line| line == ":error");
            let title = header.lines().next().unwrap_or("").to_string();
            // The blank line before the dashes is no part of the input.
            let input = body.split(dashes.as_str()).next().unwrap_or("");
            let input = input.strip_suffix('\n').unwrap_or(input).to_string();
            let file = path.file_name().unwrap().to_string_lossy();
            out.push((format!("{file}: {title}"), input, error));
        }
    }
    out
}

#[test]
fn the_corpus_tests_parse_as_the_crate_does() {
    let tests = corpus_tests();
    assert!(tests.len() >= 40, "only {} corpus tests", tests.len());
    let mut failures = Vec::new();
    for (label, input, error) in &tests {
        match compare(label, input) {
            Ok(Verdict::Equal(_)) if !error => {}
            Ok(Verdict::Rejected) if *error => {}
            Ok(_) => failures.push(format!(
                "{label}: marked {}, the crate {}",
                if *error { "error" } else { "valid" },
                if *error { "accepts it" } else { "rejects it" },
            )),
            Err(failure) => failures.push(failure),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("corpus tests: {} agree with the crate", tests.len());
}
