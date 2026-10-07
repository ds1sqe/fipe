//! Inference over a resolved module.
//!
//! The judgments of [types.md] with the arity half of spot binding from
//! [evaluation.md] §Parameter binding. It unifies and never casts,
//! evaluates no default and calls no host. Bindings generalize at their
//! boundary; an arm binding and the members of one impl are typed as a
//! group, monomorphic inside.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md
//! [evaluation.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/08-evaluation.md

mod arms;
mod bind;
mod claims;
mod conversions;
mod dot;
mod expr;
mod groups;
mod lower;
mod outcome;
mod spots;

use crate::{
    ast::{Arm, Entry, ExprId, ExprKind, ImplMember, ItemKind, Module, Name},
    decl::Declarations,
    frame::Frame,
    resolve::{Origin, Resolved, Target},
    token::Span,
    typed::Typed,
    types::{replace, Lit, Mismatch, Obligation, Scheme, Subst, Type, Var},
};
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

/// A located type error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: Span,
    pub reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// `actual` at `span` against `expected`, which `other` required.
    Mismatch {
        expected: Type,
        actual: Type,
        other: Span,
    },
    /// A type that would contain itself.
    Occurs,
    /// An outcome bracket or marked operator with no known family.
    UnknownFamily,
    /// No field or member on `ty`; `other` is the
    /// use that made a scheme's head `ty`, or the dot itself.
    NoField {
        ty: Type,
        other: Span,
    },
    ClaimBlock {
        claim: Type,
        problem: String,
    },
    MissingClaim {
        ty: Type,
        claim: Type,
    },
    MissingMember {
        ty: Type,
        name: String,
    },
    /// No default meaning accepts all the literal forms together.
    NoCommonMeaning {
        forms: Vec<String>,
    },
    NotAFunction,
    Conversion {
        target: Type,
        candidates: Vec<Type>,
    },
    ConflictingConversion,
    ExtraArgument,
    /// A flag naming no spot, or a valued flag with no operand.
    UnknownSpot {
        name: String,
    },
    /// A pipe into a stage with no open spot, or a spot supplied twice.
    FilledSpot,
    /// A stage on `Maybe`'s miss side with an open spot: `|!` on a
    /// `Maybe` supplies nothing, so its stage must be a value.
    OpenSpot,
    /// `||` or `&&` on `Either` or another non-`Bool` type.
    NeedsMaybeOrResult,
    /// A type-dependent form reserved for a later minor.
    NotYetChecked {
        form: crate::resolve::Unchecked,
    },
    /// A non-last line of a body whose value is not `()`.
    Discarded,
    /// A record construction without a field that has no default.
    MissingField {
        name: String,
    },
    /// An integer literal outside its type's range.
    OutOfRange,
    /// An obligation on a variable that left the binding's type.
    Ambiguous {
        class: String,
    },
    /// A value of a file module reached again through its own `use`
    /// lines, which has no type; resolve reports the cycle at its `use`.
    ImportCycle,
    /// `value.a`: a non-`pub` inherent member `a` of the value's type,
    /// declared in another module.
    PrivateMember {
        member: String,
    },
}

/// Type one resolved module. `files` are the file modules its `use`
/// lines may name, as given to [`resolve`](crate::resolve::resolve).
///
/// # Errors
/// Every located type error; the walk continues past each one.
pub fn infer(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    files: &[(String, Module)],
) -> Result<Typed, Vec<Error>> {
    let (typed, errors) = infer_partial(module, resolved, declarations, files);
    if errors.is_empty() {
        Ok(typed)
    } else {
        Err(errors)
    }
}

/// [`infer`] keeping what it learned beside its errors: a binding that
/// typed has its scheme even when another line failed.
#[must_use]
pub fn infer_partial(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    files: &[(String, Module)],
) -> (Typed, Vec<Error>) {
    run(module, resolved, declarations, files, None, &[], usize::MAX)
}

/// [`infer_partial`] after the definitions of a session's accepted
/// lines, with the lines from byte `query` on queried rather than run: a
/// queried expression line is generalized like a binding, not defaulted
/// where it leaves, and its scheme is [`Typed::binding`] of the line.
pub(crate) fn infer_in(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: &Frame,
    query: usize,
) -> (Typed, Vec<Error>) {
    run(
        module,
        resolved,
        declarations,
        files,
        Some(frame),
        &[],
        query,
    )
}

/// Inference with the file modules already being typed, which a cycle
/// of `use` lines must not enter again; lines from byte `query` on are
/// queried.
fn run(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: Option<&Frame>,
    visiting: &[String],
    query: usize,
) -> (Typed, Vec<Error>) {
    let subst = frame.map_or_else(Subst::default, |f| f.weak.clone());
    let mut infer = Infer {
        module,
        resolved,
        declarations,
        files,
        frame,
        visiting: visiting.to_vec(),
        weak: subst.count(),
        state: State::new(subst, module.expressions.len()),
        items: Vec::new(),
        groups: Vec::new(),
        self_type: None,
    };
    infer.declare();
    infer.field_defaults_all();
    let mut impls = 0;
    for entry in &module.entries {
        match entry {
            Entry::Line(id) => {
                if module.expressions[id.0].span.start >= query {
                    infer.query_line(*id);
                } else {
                    infer.line(*id, true);
                }
            }
            // An impl is order-free: typed here unless a use typed it
            // first (`groups.rs`).
            Entry::Item(item) if matches!(item.kind, ItemKind::Impl { .. }) => {
                if infer.groups[impls]
                    .members
                    .iter()
                    .any(|m| matches!(infer.state.status[*m], Status::Pending))
                {
                    infer.type_group(impls);
                }
                impls += 1;
            }
            Entry::Item(_) => {}
        }
    }
    infer.check_claim_blocks();
    infer.finish()
}

/// Everything inference changes, cloned to try one reading of `||`.
#[derive(Clone)]
struct State {
    subst: Subst,
    types: Vec<Option<Type>>,
    bindings: Vec<Option<Scheme>>,
    /// Lexical bindings by the span of the name that bound them.
    locals: BTreeMap<(usize, usize), Local>,
    obligations: Vec<Owed>,
    solving_dots: bool,
    claims: Vec<(Type, bool, Vec<Type>)>,
    checked_claims: Vec<(Type, Type)>,
    lowering: Vec<crate::types::TypeName>,
    errors: Vec<Error>,
    /// Monomorphic types in scope: parameters and destructured values.
    env: Vec<Type>,
    /// Per item, how far its typing is.
    status: Vec<Status>,
    /// Uses of impl members inside their own group, unified with
    /// the binding's type at the group's end.
    uses: Vec<Use>,
    /// Uses of impl members before their group could be typed, held
    /// monomorphic until it is and then unified with its scheme.
    deferred: Vec<Use>,
}

impl State {
    /// A fresh state over `subst` for a module of `size` expressions.
    fn new(subst: Subst, size: usize) -> Self {
        Self {
            subst,
            types: vec![None; size],
            bindings: vec![None; size],
            locals: BTreeMap::new(),
            obligations: Vec::new(),
            solving_dots: false,
            claims: Vec::new(),
            checked_claims: Vec::new(),
            lowering: Vec::new(),
            errors: Vec::new(),
            env: Vec::new(),
            status: Vec::new(),
            uses: Vec::new(),
            deferred: Vec::new(),
        }
    }
}

/// An obligation, where it arose, and the literal that raised it.
#[derive(Clone)]
struct Owed {
    obligation: Obligation,
    span: Span,
    literal: Option<Lit>,
    /// A member obligation already solved, left in place so that marks
    /// into the list stay valid.
    solved: bool,
    assumed: bool,
}

impl Owed {
    const fn new(obligation: Obligation, span: Span) -> Self {
        Self {
            obligation,
            span,
            literal: None,
            solved: false,
            assumed: false,
        }
    }
}

#[derive(Clone)]
enum Status {
    Pending,
    Typing,
    Done(Scheme),
}

#[derive(Clone)]
struct Use {
    item: usize,
    ty: Type,
    span: Span,
}

/// A lexical binding's scheme; a session's frame keeps its accepted
/// lines' bindings in this form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Local {
    pub(crate) scheme: Scheme,
    /// The spot list of a binding whose value is an arm list.
    pub(crate) heads: Option<Vec<Head>>,
}

/// One head position of an arm list: its name, and whether it defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    name: Option<String>,
    default: bool,
}

/// A value of this module known before its line: an impl member, or
/// a written constructor.
struct Item {
    key: Key,
    ty: Type,
    heads: Option<Vec<Head>>,
    /// Its impl's group, and the value it is checked with; a member's
    /// second key has none.
    group: usize,
    body: Option<Body>,
    /// Typed against a written signature, whose names are rigid.
    annotated: bool,
    /// The name, and whether it is `pub`.
    site: Span,
    public: bool,
}

/// What an impl item's type is checked against.
#[derive(Clone)]
enum Body {
    Value(ExprId),
    Arms(Vec<Arm>),
}

/// The items of one impl, typed together, and its `Self`.
struct Group {
    members: Vec<usize>,
    self_type: Option<Type>,
    /// The sites of the file-level bindings its members name.
    names: Vec<Span>,
}

impl Item {
    const fn new(
        key: Key,
        ty: Type,
        heads: Option<Vec<Head>>,
        site: Span,
    ) -> Self {
        Self {
            key,
            ty,
            heads,
            group: 0,
            body: None,
            annotated: false,
            site,
            public: false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Key {
    Binding(String),
    /// A member of a type's namespace: (type, member).
    Member(String, String),
    Constructor(String),
}

/// One spot of a callee.
#[derive(Clone, Debug)]
struct Slot {
    name: Option<String>,
    ty: Type,
    default: bool,
    /// A declared default and the spot's name, in the callee's home.
    value: Option<ExprId>,
    site: Span,
}

/// A value about to be applied.
#[derive(Clone, Debug)]
struct Callee {
    /// The whole type without a spot list; the result with one.
    ty: Type,
    spots: Option<Vec<Slot>>,
    /// The slot a dot fills with its head.
    self_slot: Option<usize>,
    /// A slot filled before application, by a dot.
    supplied: Option<(usize, Type, Span)>,
    /// Declared type parameters in order, for `name<τ…>`.
    parameters: Vec<Type>,
    /// The declaring module of a callee declared elsewhere.
    home: Option<Origin>,
    conversion: Option<(Type, String)>,
}

impl Callee {
    const fn plain(ty: Type) -> Self {
        Self {
            ty,
            spots: None,
            self_slot: None,
            supplied: None,
            parameters: Vec::new(),
            home: None,
            conversion: None,
        }
    }

    /// The function type of every spot, then the result.
    fn whole(&self) -> Type {
        self.spots
            .iter()
            .flatten()
            .rev()
            .fold(self.ty.clone(), |ty, slot| {
                Type::function(slot.ty.clone(), ty)
            })
    }
}

/// A type an expression is checked against, and where it came from.
#[derive(Clone)]
struct Expected {
    ty: Type,
    span: Span,
    /// At a spot, where a word takes the spot's type.
    spot: bool,
}

impl Expected {
    const fn new(ty: Type, span: Span) -> Self {
        Self {
            ty,
            span,
            spot: false,
        }
    }
}

struct Infer<'a> {
    module: &'a Module,
    resolved: &'a Resolved,
    declarations: &'a Declarations,
    files: &'a [(String, Module)],
    /// A session's accepted lines.
    frame: Option<&'a Frame>,
    visiting: Vec<String>,
    /// The session's weak variables, `Var(0)` up to this: free in every
    /// line after the one that left them.
    weak: u32,
    state: State,
    items: Vec<Item>,
    /// The item groups, one per impl in order.
    groups: Vec<Group>,
    /// `Self` inside an impl body.
    self_type: Option<Type>,
}

impl Infer<'_> {
    fn fresh(&mut self) -> Type {
        Type::Var(self.state.subst.fresh())
    }

    fn apply(&self, ty: &Type) -> Type {
        self.state.subst.apply(ty)
    }

    fn error(&mut self, span: Span, reason: Reason) {
        self.state.errors.push(Error { span, reason });
    }

    fn span(&self, id: ExprId) -> Span {
        self.module.expressions[id.0].span
    }

    /// Unify, reporting a failure at `span` against `other`.
    fn unify(
        &mut self,
        expected: &Type,
        actual: &Type,
        span: Span,
        other: Span,
    ) {
        if let Err(mismatch) = self.state.subst.unify(expected, actual) {
            let reason = match *mismatch {
                Mismatch::Different { .. } => Reason::Mismatch {
                    expected: self.apply(expected),
                    actual: self.apply(actual),
                    other,
                },
                Mismatch::Occurs { .. } => Reason::Occurs,
            };
            self.error(span, reason);
        }
    }

    /// Unify where a later check reports the failure.
    fn hint(&mut self, expected: &Type, actual: &Type) {
        let _ = self.state.subst.unify(expected, actual);
    }

    fn require_member(&mut self, head: Type, name: &str, ty: Type, span: Span) {
        let obligation = Obligation::Member {
            head,
            name: String::from(name),
            ty,
            dot: span,
        };
        self.state.obligations.push(Owed::new(obligation, span));
    }

    fn item_index(&self, key: &Key) -> Option<usize> {
        self.items.iter().position(|item| item.key == *key)
    }

    fn local(&mut self, site: Span, span: Span) -> Callee {
        let key = (site.start, site.end);
        let local =
            self.state.locals.get(&key).or_else(|| {
                self.frame.and_then(|frame| frame.locals.get(&key))
            });
        match local.cloned() {
            Some(mut local) => {
                if !self.state.locals.contains_key(&key) {
                    for obligation in &mut local.scheme.obligations {
                        let (Obligation::Member { dot, .. }
                        | Obligation::Claim { dot, .. }) = obligation;
                        *dot = span;
                    }
                    for (_, _, dot) in &mut local.scheme.literals {
                        *dot = span;
                    }
                }
                let ty = self.instantiate(&local.scheme, span);
                self.with_heads(ty, local.heads.as_deref())
            }
            None => Callee::plain(self.fresh()),
        }
    }

    fn bind_local(&mut self, name: &Name, ty: Type) {
        let local = Local {
            scheme: Scheme::mono(ty),
            heads: None,
        };

        self.state
            .locals
            .insert((name.span.start, name.span.end), local);
    }

    /// A type skeleton for an arm list: one fresh type per head
    /// position, then the result.
    fn skeleton(&mut self, arms: &[Arm]) -> Type {
        let count = arms.first().map_or(0, |arm| arm.patterns.len());
        let mut ty = self.fresh();
        for _ in 0..count {
            ty = Type::function(self.fresh(), ty);
        }
        ty
    }

    /// The impl members of this module, known before their lines: one
    /// group per impl, in order.
    fn declare(&mut self) {
        let module = self.module;
        for entry in &module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Impl {
                    target, members, ..
                } = &item.kind
                {
                    self.declare_impl(*target, members);
                    let names = self.outside_names(item.span);
                    if let Some(group) = self.groups.last_mut() {
                        group.names = names;
                    }
                }
            }
        }
    }

    /// The sites of the bindings named inside `span` but bound outside.
    fn outside_names(&self, span: Span) -> Vec<Span> {
        let inside = |s: Span| span.start <= s.start && s.end <= span.end;
        let mut names = Vec::new();
        for (id, target) in self.resolved.expressions() {
            let mut target = target;
            while let Target::Members { head, .. } = target {
                target = head;
            }
            if let Target::Local { site } = target {
                if inside(self.span(id)) && !inside(*site) {
                    names.push(*site);
                }
            }
        }
        names
    }

    fn declare_impl(
        &mut self,
        target: crate::ast::TypeId,
        members: &[ImplMember],
    ) {
        let module = self.module;
        let group = self.groups.len();
        let ty = lower::named(module, target).unwrap_or_default();
        let self_type = self.lower_here(target);
        self.self_type = Some(self_type.clone());
        let first = self.items.len();
        let mut constructor = Vec::new();
        for member in members {
            match member {
                // A program member with no value, which resolve reports.
                ImplMember::Named { value: None, .. } => {}
                ImplMember::Named {
                    public,
                    name,
                    annotation,
                    value: Some(value),
                    ..
                } => {
                    let arms = match &module.expressions[value.0].kind {
                        ExprKind::Arms(arms) => Some(arms.as_slice()),
                        _ => None,
                    };
                    let member_ty = match (annotation, arms) {
                        (Some(annotation), _) => self.lower_here(*annotation),
                        (None, Some(arms)) => self.skeleton(arms),
                        (None, None) => self.fresh(),
                    };
                    let heads = arms.map(|arms| self.heads(arms));
                    if let Some(heads) = &heads {
                        self.self_parameter(&member_ty, heads, name.span);
                    }
                    let key = Key::Member(ty.clone(), last(name));
                    let mut item = Item::new(key, member_ty, heads, name.span);
                    item.public = *public;
                    item.annotated = annotation.is_some();
                    item.body = Some(Body::Value(*value));
                    self.push_item(item, group);
                }
                ImplMember::Constructor { arm, .. } => {
                    constructor.push(arm.clone());
                }
            }
        }
        if !constructor.is_empty() {
            let skeleton = self.skeleton(&constructor);
            let heads = Some(self.heads(&constructor));
            let site = item_span(&constructor);
            let mut item =
                Item::new(Key::Constructor(ty), skeleton, heads, site);
            item.body = Some(Body::Arms(constructor));
            self.push_item(item, group);
        }
        self.groups.push(Group {
            members: (first..self.items.len()).collect(),
            self_type: Some(self_type),
            names: Vec::new(),
        });
        self.self_type = None;
    }

    fn push_item(&mut self, mut item: Item, group: usize) {
        item.group = group;
        self.items.push(item);
        self.state.status.push(Status::Pending);
    }

    /// In an impl member the parameter `self` has type `Self`.
    fn self_parameter(&mut self, ty: &Type, heads: &[Head], span: Span) {
        let Some(self_type) = self.self_type.clone() else {
            return;
        };
        let Some((parameters, _)) = self.peel(ty, heads.len()) else {
            return;
        };
        for (head, parameter) in heads.iter().zip(parameters) {
            if head.name.as_deref() == Some("self") {
                self.unify(&parameter, &self_type, span, span);
            }
        }
    }

    /// Record field defaults, checked against their field's types.
    fn field_defaults_all(&mut self) {
        let module = self.module;
        for entry in &module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Type { name, members, .. } = &item.kind {
                    self.field_defaults(name, members.as_deref());
                }
            }
        }
    }

    /// Obligations settle, members resolve or fail, and
    /// every type is applied through the final substitution.
    fn finish(mut self) -> (Typed, Vec<Error>) {
        self.check_conversions();
        self.dots();
        self.default_literals(0, &[], &[]);
        self.left_members();
        let unsolved = self.settle();
        let (frozen, weak) = self.frozen();
        let subst = &self.state.subst;
        let typed = Typed {
            expressions: self
                .state
                .types
                .iter()
                .map(|ty| ty.as_ref().map(|ty| subst.apply(ty)))
                .collect(),
            bindings: self
                .state
                .bindings
                .iter()
                .map(|scheme| scheme.as_ref().map(|s| bind::applied(s, subst)))
                .collect(),

            unsolved,
            frozen,
            weak,
        };
        let mut errors = self.state.errors;
        errors.sort_by_key(|error| (error.span.start, error.span.end));
        (typed, errors)
    }
}

/// Every binding and item of a module by its site, frozen.
type Frozen = Vec<((usize, usize), Local)>;

/// The first variable a frozen scheme quantifies: its variables are
/// numbered apart from the weak ones, which later lines extend.
const QUANTIFIED: u32 = 1 << 31;

impl Infer<'_> {
    /// Every binding and item by its site, applied and generalized
    /// against the file-level environment, and the weak variables after
    /// this module. A pattern-bound binding is monomorphic in a file, so
    /// it freezes monomorphic: its variables stay weak, shared with the
    /// lines after it, which may solve them as the file's later lines
    /// would. Every other variable is quantified.
    fn frozen(&self) -> (Frozen, Subst) {
        let subst = &self.state.subst;
        // The earlier lines' weak variables and the file-level
        // monomorphic types: parameters and blocks have left `env`.
        let mut env = Vec::new();
        for var in 0..self.weak {
            subst.apply(&Type::Var(Var(var))).variables(&mut env);
        }
        for ty in &self.state.env {
            subst.apply(ty).variables(&mut env);
        }
        self.close(&mut env);
        // New weak variables follow the earlier ones.
        let mut weak: Vec<(Var, Type)> = Vec::new();
        for var in env.iter().filter(|var| var.0 >= self.weak) {
            let next = self.weak.saturating_add(count(weak.len()));
            weak.push((*var, Type::Var(Var(next))));
        }
        let freeze = |scheme: &Scheme| {
            let scheme = bind::applied(scheme, subst);
            let general = Scheme::generalize(
                &scheme.ty,
                &scheme.obligations,
                &env,
                &Subst::default(),
            );
            let mut names = weak.clone();
            let mut variables = Vec::new();
            for (index, var) in general.variables.iter().enumerate() {
                let name = Var(QUANTIFIED.saturating_add(count(index)));
                names.push((*var, Type::Var(name)));
                variables.push(name);
            }
            Scheme {
                variables,
                obligations: general
                    .obligations
                    .iter()
                    .map(|o| o.map(|ty| replace(ty, &names)))
                    .collect(),
                literals: scheme
                    .literals
                    .iter()
                    .map(|(ty, form, span)| {
                        (replace(ty, &names), form.clone(), *span)
                    })
                    .collect(),
                ty: replace(&general.ty, &names),
            }
        };
        let mut frozen: Vec<_> = self
            .state
            .locals
            .iter()
            .map(|(site, local)| {
                let local = Local {
                    scheme: freeze(&local.scheme),
                    heads: local.heads.clone(),
                };
                (*site, local)
            })
            .collect();
        for (item, status) in self.items.iter().zip(&self.state.status) {
            let scheme = match status {
                Status::Done(scheme) => freeze(scheme),
                _ => freeze(&Scheme::mono(item.ty.clone())),
            };
            let local = Local {
                scheme,
                heads: item.heads.clone(),
            };
            frozen.push(((item.site.start, item.site.end), local));
        }
        let earlier = (0..self.weak).map(|var| {
            let ty = subst.apply(&Type::Var(Var(var)));
            (ty != Type::Var(Var(var))).then(|| replace(&ty, &weak))
        });
        let later = weak.iter().map(|_| None);
        (frozen, Subst::from_bindings(earlier.chain(later).collect()))
    }
}

fn count(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn meaning(ty: &Type) -> &Type {
    match ty {
        Type::Limited { meaning, .. } => meaning,
        _ => ty,
    }
}

fn prelude(name: &str) -> Type {
    Type::named(Origin::Prelude, name, Vec::new())
}

fn prelude_of(name: &str, arguments: Vec<Type>) -> Type {
    Type::named(Origin::Prelude, name, arguments)
}

fn is_prelude(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Named { name: n, .. }
        if n.origin == Origin::Prelude && n.name == name)
}

fn last(name: &Name) -> String {
    name.segments.last().cloned().unwrap_or_default()
}

fn item_span(arms: &[Arm]) -> Span {
    arms.first().map_or_else(Span::default, |arm| arm.span)
}
