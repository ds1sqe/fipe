//! Document and workspace symbols: a module's top-level bindings,
//! declarations and types with their fields and constructors.
use fipe::ast::{Entry, ExprKind, ItemKind, Member, Module, PatternKind};
use fipe::typed::Typed;
use lsp_types::{DocumentSymbol, Range, SymbolKind};

/// One symbol of a module, its ranges still byte spans.
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub detail: Option<String>,
    /// The whole item, and its name.
    pub span: (usize, usize),
    pub selection: (usize, usize),
    pub children: Vec<Self>,
}

/// The symbols of a module's top-level entries, in source order.
///
/// A binding (a function when its scheme is one, when `typed` knows
/// it), a signature-only declaration, a type with its fields and
/// constructors.
#[must_use]
pub fn of_module(
    module: &Module,
    text: &str,
    typed: Option<&Typed>,
) -> Vec<Symbol> {
    let slice = |(start, end): (usize, usize)| {
        text.get(start..end).unwrap_or_default().trim().to_string()
    };
    let mut symbols = Vec::new();
    for entry in &module.entries {
        match entry {
            Entry::Line(id) => {
                let Some(expr) = module.expr(*id) else {
                    continue;
                };
                let ExprKind::Bind { pattern, value, .. } = &expr.kind else {
                    continue;
                };
                let Some(pattern) = module.pattern(*pattern) else {
                    continue;
                };
                let PatternKind::Name(name) = &pattern.kind else {
                    continue;
                };
                let scheme = typed
                    .and_then(|typed| typed.binding(*id))
                    .map(ToString::to_string);
                let function = scheme.as_ref().map_or_else(
                    || {
                        module.expr(*value).is_some_and(|value| {
                            matches!(value.kind, ExprKind::Arms(_))
                        })
                    },
                    |scheme| scheme.contains("->"),
                );
                symbols.push(Symbol {
                    name: name.segments.join("."),
                    kind: if function {
                        SymbolKind::FUNCTION
                    } else {
                        SymbolKind::VARIABLE
                    },
                    detail: scheme,
                    span: (expr.span.start, expr.span.end),
                    selection: (name.span.start, name.span.end),
                    children: Vec::new(),
                });
            }
            Entry::Item(item) => match &item.kind {
                ItemKind::Declare { name, ty } => {
                    let detail = module
                        .ty(*ty)
                        .map(|ty| slice((ty.span.start, ty.span.end)));
                    let function =
                        detail.as_ref().is_some_and(|ty| ty.contains("->"));
                    symbols.push(Symbol {
                        name: name.segments.join("."),
                        kind: if function {
                            SymbolKind::FUNCTION
                        } else {
                            SymbolKind::VARIABLE
                        },
                        detail,
                        span: (item.span.start, item.span.end),
                        selection: (name.span.start, name.span.end),
                        children: Vec::new(),
                    });
                }
                ItemKind::Type { name, members, .. } => {
                    symbols.push(type_symbol(
                        module,
                        &slice,
                        item,
                        name,
                        members.as_deref(),
                    ));
                }
                ItemKind::Impl { .. } | ItemKind::Use { .. } => {}
            },
        }
    }
    symbols
}

/// A `type` item's symbol: a struct, or an enum when it has
/// constructors, with its fields and constructors as children.
fn type_symbol(
    module: &Module,
    slice: &dyn Fn((usize, usize)) -> String,
    item: &fipe::ast::Item,
    name: &fipe::ast::Name,
    members: Option<&[Member]>,
) -> Symbol {
    let mut children = Vec::new();
    let mut sum = false;
    for member in members.into_iter().flatten() {
        match member {
            Member::Field(field) => children.push(Symbol {
                name: field.name.segments.join("."),
                kind: SymbolKind::FIELD,
                detail: module
                    .ty(field.ty)
                    .map(|ty| slice((ty.span.start, ty.span.end))),
                span: (field.span.start, field.span.end),
                selection: (field.name.span.start, field.name.span.end),
                children: Vec::new(),
            }),
            Member::Constructor {
                name, fields, span, ..
            } => {
                sum = true;
                children.push(Symbol {
                    name: name.segments.join("."),
                    kind: SymbolKind::ENUM_MEMBER,
                    detail: Some(format!(
                        "{} field{}",
                        fields.len(),
                        if fields.len() == 1 { "" } else { "s" }
                    )),
                    span: (span.start, span.end),
                    selection: (name.span.start, name.span.end),
                    children: Vec::new(),
                });
            }
        }
    }
    Symbol {
        name: name.segments.join("."),
        kind: if sum {
            SymbolKind::ENUM
        } else {
            SymbolKind::STRUCT
        },
        detail: None,
        span: (item.span.start, item.span.end),
        selection: (name.span.start, name.span.end),
        children,
    }
}

/// A symbol and its children as the protocol's hierarchical form,
/// ranges made by `range`.
#[must_use]
pub fn document_symbol(
    symbol: &Symbol,
    range: &dyn Fn((usize, usize)) -> Range,
) -> DocumentSymbol {
    #[allow(deprecated)] // `deprecated` is a field of the protocol's struct.
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: symbol.detail.clone(),
        kind: symbol.kind,
        tags: None,
        deprecated: None,
        range: range(symbol.span),
        selection_range: range(symbol.selection),
        children: (!symbol.children.is_empty()).then(|| {
            symbol
                .children
                .iter()
                .map(|child| document_symbol(child, range))
                .collect()
        }),
    }
}

/// Whether `name` matches a workspace symbol `query`: every character
/// of the query in order, ignoring case; an empty query matches all.
#[must_use]
pub fn matches(query: &str, name: &str) -> bool {
    let mut rest = name.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| rest.by_ref().any(|have| have == wanted))
}
