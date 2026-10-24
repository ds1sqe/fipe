//! Inference over a resolved module.
//!
//! The judgments of [types.md] with recorded spot binding from
//! [evaluation.md] §Parameter binding. It unifies meanings and records
//! the closed cast table at known places; it
//! evaluates no default and calls no host. Bindings generalize at their
//! boundary; an arm binding and the members of one impl are typed as a
//! group, monomorphic inside.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md
//! [evaluation.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/08-evaluation.md

mod arms;
mod bind;
mod casts;
mod claims;
mod containers;
mod conversions;
mod dispatch;
mod dot;
mod expr;
pub(crate) mod files;
mod groups;
mod hidden;
mod index;
mod lower;
mod outcome;
mod record;
mod spots;
mod test;
mod trial;
pub(crate) use spots::{SourceNodes, Supply};

use crate::{
    ast::{Arm, Entry, ExprId, ExprKind, ImplMember, ItemKind, Module, Name},
    decl::Declarations,
    frame::Frame,
    resolve::{Origin, Resolved, Target},
    token::Span,
    typed::Typed,
    types::{replace, Lit, Mismatch, Obligation, Scheme, Subst, Type, Var},
};
use alloc::{collections::BTreeMap, rc::Rc, string::String, vec, vec::Vec};

/// A located type error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: Span,
    pub reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Spots {
        problem: crate::decl::SpotProblem,
    },
    TypeDepth,
    CallReading {
        style: crate::ast::CallStyle,
    },
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
    NothingPayload,
    OutcomeText {
        family: Type,
    },
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
    /// Record syntax needs a record layout or a sum constructor with fields.
    NotRecord,
    /// A file cannot supply marked resource types, directly or through signatures.
    ResourceInFile {
        module: String,
    },
    Conversion {
        source: Option<Type>,
        target: Type,
        candidates: Vec<Type>,
    },
    ConflictingConversion,
    ExtraArgument,
    /// An `if` without `else` whose body has a value: such an `if` is
    /// `()`.
    IfWithoutElse,
    /// A literal written with operands, as `2 ^ 10`: a literal takes
    /// none; at the first operand.
    LiteralApplied,
    /// A positional operand while a Bool spot is open: positional
    /// operands fill non-Bool spots only.
    PositionalBool,
    NonBoolChaining,
    /// A spot's declared default does not meet its spot; `cause` is the
    /// first error of the default's own typing, none when its names did
    /// not resolve.
    Default {
        name: String,
        cause: Option<alloc::boxed::Box<Self>>,
    },
    DeclarationValue {
        name: String,
        origin: Origin,
        declaration: Span,
    },
    /// A flag naming no spot.
    UnknownSpot {
        name: String,
    },
    /// A valued flag with no operand after it.
    FlagOperand {
        name: String,
    },
    /// A spot supplied twice, by name.
    DuplicateSupply {
        name: String,
    },
    /// A pipe into a stage with no open spot.
    FilledSpot,
    /// A stage on `Maybe`'s miss side with an open spot: `|!` on a
    /// `Maybe` supplies nothing, so its stage must be a value.
    OpenSpot,
    /// `||` or `&&` on `Either` or another non-`Bool` type.
    NeedsMaybeOrResult,
    /// `|:` or `|!` on a type that is no outcome.
    NeedsOutcome,
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
    /// A literal `ty` cannot hold exactly: a fraction at an integer
    /// place that is no integer, or an integer past a float's precision.
    NotExact {
        ty: Type,
    },
    /// An outcome bracket at a place whose type has no bracket form.
    NoBracketForm {
        ty: Type,
    },
    /// `||` with a right side of another outcome family than the left.
    FamilyClash {
        left: Type,
        right: Type,
    },
    /// A payload pattern on a constructor without fields.
    NoPayload {
        name: String,
    },
    /// `[:]` or `[!]` with no payload where the side carries one.
    MissingPayload {
        good: bool,
        ty: Type,
    },
    /// A refuted limit, or an open entry in a strict session.
    Limit {
        message: String,
    },
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
    PrivateField {
        field: String,
    },
    /// A record constructed outside its module while a private field
    /// has no default: the derived constructor does not exist there.
    NoConstructor {
        field: String,
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
    run(
        module,
        resolved,
        declarations,
        files,
        None,
        (&[], &files::FileCache::default()),
        usize::MAX,
    )
}

/// Inference with the file modules already being typed, which a cycle
/// of `use` lines must not enter again; lines from byte `query` on are
/// queried.
pub(crate) fn run(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: Option<&Frame>,
    (visiting, cache): (&[String], &files::FileCache),
    query: usize,
) -> (Typed, Vec<Error>) {
    let mut infer = Infer::new(
        module,
        resolved,
        declarations,
        files,
        frame,
        visiting,
        cache,
    );
    for error in crate::decl::spots::check(module) {
        if let crate::decl::Reason::Spots { problem } = error.reason {
            infer.error(error.span, Reason::Spots { problem });
        }
    }
    infer.declare();
    let mut impls = 0;
    for entry in &module.entries {
        match entry {
            Entry::Line(id) => {
                if module.expressions[id.0].span.start >= query {
                    infer.query_line(*id);
                } else {
                    infer.line(*id);
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
            Entry::Item(item) => {
                if let ItemKind::Type { name, members, .. } = &item.kind {
                    // A test reads the bindings above its type, so it is
                    // typed after them, at the type's own entry.
                    infer.check_tests(item);
                    infer.field_defaults(name, members.as_deref());
                }
            }
        }
        // A depth refusal ends this unit: later bindings can otherwise
        // rebuild the refused chain repeatedly during error recovery.
        if infer
            .state
            .errors
            .iter()
            .any(|error| matches!(error.reason, Reason::TypeDepth))
        {
            break;
        }
    }
    infer.check_claim_blocks();
    infer.finish()
}

/// One actual invocation, independent of the source expression's identity.
#[derive(Clone)]
struct DefaultOwner {
    key: (Origin, Span),
}

type LexicalOwner = Option<Rc<DefaultOwner>>;

fn same_owner(a: &LexicalOwner, b: &LexicalOwner) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

/// Immutable lexical facts. Shared solver queues and substitution never live here.
#[derive(Clone)]
struct LexicalFacts {
    constructor_fields: BTreeMap<(usize, usize), Vec<usize>>,
    derived_defaults: BTreeMap<(usize, usize), crate::typed::DefaultValue>,
    conversions: BTreeMap<(usize, usize), crate::typed::Conversion>,
    pattern_equals: BTreeMap<usize, crate::typed::Comparison>,
    body_parameters: BTreeMap<(usize, usize), Vec<Type>>,
    tests: Vec<crate::typed::Test>,
    default_instances:
        BTreeMap<(usize, usize, usize), crate::typed::DeclaredExpression>,
    member_contracts:
        BTreeMap<crate::typed::MemberContractUse, crate::typed::MemberContract>,
    member_contract_reads:
        BTreeMap<(usize, usize), Vec<Vec<crate::typed::MemberContractRead>>>,
    contract_stages: BTreeMap<(usize, usize), ContractBindingUse>,
    contract_refinements: BTreeMap<
        (crate::typed::MemberContractUse, Span, Span, usize, usize),
        Type,
    >,
    accesses: BTreeMap<(usize, usize), Vec<crate::typed::Access>>,
    roots: BTreeMap<(usize, usize), SavedRoot>,
    pattern_types: BTreeMap<usize, Type>,
    selected: BTreeMap<(usize, usize), crate::typed::Selected>,
    meaning_arguments: BTreeMap<(usize, usize), Vec<Type>>,
    main_lifts: Vec<bind::Lift>,
    callee_types: BTreeMap<(usize, usize), Type>,
    rebuild_types: BTreeMap<(usize, usize), Type>,
    rebuilds: BTreeMap<(usize, usize), crate::typed::Rebuild>,
    modes: BTreeMap<(usize, usize), crate::typed::Mode>,
    types: Vec<Option<Type>>,
    entries: Vec<crate::limits::EntryInput>,
    checks: Vec<crate::typed::Check>,
    casts: Vec<Option<alloc::boxed::Box<crate::typed::Cast>>>,
    supplied: BTreeMap<(usize, usize), Vec<(String, Supply)>>,
    bound: BTreeMap<(usize, usize), crate::typed::Bound>,
    bindings: Vec<Option<Scheme>>,
    locals: BTreeMap<(usize, usize), Local>,
    claims: Vec<(Type, bool, Vec<Type>)>,
    env: Vec<Type>,
}

impl LexicalFacts {
    fn capture(state: &State) -> Self {
        Self {
            constructor_fields: state.constructor_fields.clone(),
            derived_defaults: state.derived_defaults.clone(),
            conversions: state.conversions.clone(),
            pattern_equals: state.pattern_equals.clone(),
            body_parameters: state.body_parameters.clone(),
            tests: state.tests.clone(),
            default_instances: state.default_instances.clone(),
            member_contracts: state.member_contracts.clone(),
            member_contract_reads: state.member_contract_reads.clone(),
            contract_stages: state.contract_stages.clone(),
            contract_refinements: state.contract_refinements.clone(),
            accesses: state.accesses.clone(),
            roots: state.roots.clone(),
            pattern_types: state.pattern_types.clone(),
            selected: state.selected.clone(),
            meaning_arguments: state.meaning_arguments.clone(),
            main_lifts: state.main_lifts.clone(),
            callee_types: state.callee_types.clone(),
            rebuild_types: state.rebuild_types.clone(),
            rebuilds: state.rebuilds.clone(),
            modes: state.modes.clone(),
            types: state.types.clone(),
            entries: state.entries.clone(),
            checks: state.checks.clone(),
            casts: state.casts.clone(),
            supplied: state.supplied.clone(),
            bound: state.bound.clone(),
            bindings: state.bindings.clone(),
            locals: state.locals.clone(),
            claims: state.claims.clone(),
            env: state.env.clone(),
        }
    }

    fn install(&self, state: &mut State) {
        state.constructor_fields = self.constructor_fields.clone();
        state.derived_defaults = self.derived_defaults.clone();
        state.conversions = self.conversions.clone();
        state.pattern_equals = self.pattern_equals.clone();
        state.body_parameters = self.body_parameters.clone();
        state.tests.clone_from(&self.tests);
        state.default_instances = self.default_instances.clone();
        state.member_contracts = self.member_contracts.clone();
        state.member_contract_reads = self.member_contract_reads.clone();
        state.contract_stages = self.contract_stages.clone();
        state.contract_refinements = self.contract_refinements.clone();
        state.accesses = self.accesses.clone();
        state.roots = self.roots.clone();
        state.pattern_types = self.pattern_types.clone();
        state.selected = self.selected.clone();
        state.meaning_arguments = self.meaning_arguments.clone();
        state.main_lifts.clone_from(&self.main_lifts);
        state.callee_types = self.callee_types.clone();
        state.rebuild_types = self.rebuild_types.clone();
        state.rebuilds = self.rebuilds.clone();
        state.modes = self.modes.clone();
        state.types.clone_from(&self.types);
        state.entries.clone_from(&self.entries);
        state.checks.clone_from(&self.checks);
        state.casts.clone_from(&self.casts);
        state.supplied = self.supplied.clone();
        state.bound = self.bound.clone();
        state.bindings.clone_from(&self.bindings);
        state.locals = self.locals.clone();
        state.claims.clone_from(&self.claims);
        state.env.clone_from(&self.env);
    }

    fn restore_field(&self, current: &mut Self, field: trial::Field) {
        match field {
            trial::Field::Selection => {
                current.selected.clone_from(&self.selected);
                current.member_contracts.clone_from(&self.member_contracts);
                current.contract_stages.clone_from(&self.contract_stages);
                current
                    .contract_refinements
                    .clone_from(&self.contract_refinements);
                current
                    .meaning_arguments
                    .clone_from(&self.meaning_arguments);
            }
            trial::Field::DefaultInstances => {
                current
                    .default_instances
                    .clone_from(&self.default_instances);
            }
        }
    }
}

#[derive(Clone)]
struct LexicalContext {
    owner: LexicalOwner,
    resolved: Rc<Resolved>,
    self_type: Option<Type>,
    home: Option<Origin>,
    facts: LexicalFacts,
}

/// Everything inference changes. A trial reading ([`trial`]) journals
/// its writes and is kept or restored.
struct State {
    lexical_owner: LexicalOwner,
    lexical_resolved: Option<Rc<Resolved>>,
    lexical_frames: Vec<Rc<LexicalContext>>,
    constructor_fields: BTreeMap<(usize, usize), Vec<usize>>,
    derived_defaults: BTreeMap<(usize, usize), crate::typed::DefaultValue>,
    conversions: BTreeMap<(usize, usize), crate::typed::Conversion>,
    pattern_equals: BTreeMap<usize, crate::typed::Comparison>,
    body_parameters: BTreeMap<(usize, usize), Vec<Type>>,
    tests: Vec<crate::typed::Test>,
    pending_defaults: Vec<PendingDefault>,
    default_instances:
        BTreeMap<(usize, usize, usize), crate::typed::DeclaredExpression>,
    member_contracts:
        BTreeMap<crate::typed::MemberContractUse, crate::typed::MemberContract>,
    member_contract_reads:
        BTreeMap<(usize, usize), Vec<Vec<crate::typed::MemberContractRead>>>,
    contract_stages: BTreeMap<(usize, usize), ContractBindingUse>,
    contract_refinements: BTreeMap<
        (crate::typed::MemberContractUse, Span, Span, usize, usize),
        Type,
    >,
    accesses: BTreeMap<(usize, usize), Vec<crate::typed::Access>>,
    roots: BTreeMap<(usize, usize), SavedRoot>,
    pattern_types: BTreeMap<usize, Type>,
    selected: BTreeMap<(usize, usize), crate::typed::Selected>,
    meaning_arguments: BTreeMap<(usize, usize), Vec<Type>>,
    main_lifts: Vec<bind::Lift>,
    callee_types: BTreeMap<(usize, usize), Type>,
    rebuild_types: BTreeMap<(usize, usize), Type>,
    rebuilds: BTreeMap<(usize, usize), crate::typed::Rebuild>,
    modes: BTreeMap<(usize, usize), crate::typed::Mode>,
    subst: Subst,
    types: Vec<Option<Type>>,
    /// Each value entering a place: its place type, span and kind, and
    /// whether the place was a type variable when entered.
    entries: Vec<crate::limits::EntryInput>,
    checks: Vec<crate::typed::Check>,
    casts: Vec<Option<alloc::boxed::Box<crate::typed::Cast>>>,
    families: Vec<outcome::Pending>,
    /// Marked-pipe rebuilds whose stage result is not known yet.
    rebuilding: Vec<outcome::Rebuilding>,
    supplied: BTreeMap<(usize, usize), Vec<(String, Supply)>>,
    bound: BTreeMap<(usize, usize), crate::typed::Bound>,
    defaults: Vec<(Origin, Span)>,
    bindings: Vec<Option<Scheme>>,
    /// Lexical bindings by the span of the name that bound them.
    locals: BTreeMap<(usize, usize), Local>,
    obligations: Vec<Owed>,
    solving_dots: bool,
    dots_start: usize,
    claims: Vec<(Type, bool, Vec<Type>)>,
    lowering: Vec<crate::types::TypeName>,
    lowered: BTreeMap<crate::types::TypeName, Type>,
    acyclic_types: alloc::collections::BTreeSet<String>,
    errors: Vec<Error>,
    warnings: Vec<crate::typed::Warning>,
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
    /// Writes to undo for the open trials, newest last; empty when none.
    journal: Vec<trial::Undo>,
    /// How many trials are open.
    trials: usize,
}

impl State {
    /// A fresh state over `subst` for a module of `size` expressions.
    fn new(subst: Subst, size: usize) -> Self {
        Self {
            lexical_owner: None,
            lexical_resolved: None,
            lexical_frames: Vec::new(),
            conversions: BTreeMap::new(),
            pattern_equals: BTreeMap::new(),
            body_parameters: BTreeMap::new(),
            tests: Vec::new(),
            subst,
            pending_defaults: Vec::new(),
            default_instances: BTreeMap::new(),
            derived_defaults: BTreeMap::new(),
            constructor_fields: BTreeMap::new(),
            member_contracts: BTreeMap::new(),
            member_contract_reads: BTreeMap::new(),
            contract_stages: BTreeMap::new(),
            contract_refinements: BTreeMap::new(),
            accesses: BTreeMap::new(),
            roots: BTreeMap::new(),
            pattern_types: BTreeMap::new(),
            selected: BTreeMap::new(),
            meaning_arguments: BTreeMap::new(),
            main_lifts: Vec::new(),
            callee_types: BTreeMap::new(),
            rebuild_types: BTreeMap::new(),
            rebuilds: BTreeMap::new(),
            modes: BTreeMap::new(),
            types: vec![None; size],
            entries: Vec::new(),
            checks: Vec::new(),
            casts: vec![None; size],
            families: Vec::new(),
            rebuilding: Vec::new(),
            supplied: BTreeMap::new(),
            bound: BTreeMap::new(),
            defaults: Vec::new(),
            bindings: vec![None; size],
            locals: BTreeMap::new(),
            obligations: Vec::new(),
            solving_dots: false,
            dots_start: 0,
            claims: Vec::new(),
            lowering: Vec::new(),
            lowered: BTreeMap::new(),
            acyclic_types: alloc::collections::BTreeSet::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            env: Vec::new(),
            status: Vec::new(),
            uses: Vec::new(),
            deferred: Vec::new(),
            journal: Vec::new(),
            trials: 0,
        }
    }
}

impl State {
    const fn move_common(&mut self, target: &mut Self) {
        core::mem::swap(&mut self.subst, &mut target.subst);
        core::mem::swap(&mut self.obligations, &mut target.obligations);
        core::mem::swap(&mut self.families, &mut target.families);
        core::mem::swap(&mut self.rebuilding, &mut target.rebuilding);
        core::mem::swap(&mut self.status, &mut target.status);
        core::mem::swap(&mut self.uses, &mut target.uses);
        core::mem::swap(&mut self.deferred, &mut target.deferred);
        core::mem::swap(
            &mut self.pending_defaults,
            &mut target.pending_defaults,
        );
        core::mem::swap(&mut self.dots_start, &mut target.dots_start);
        core::mem::swap(&mut self.solving_dots, &mut target.solving_dots);
        core::mem::swap(&mut self.errors, &mut target.errors);
        core::mem::swap(&mut self.warnings, &mut target.warnings);
        core::mem::swap(&mut self.defaults, &mut target.defaults);
        core::mem::swap(&mut self.lowering, &mut target.lowering);
        core::mem::swap(&mut self.lowered, &mut target.lowered);
        core::mem::swap(&mut self.acyclic_types, &mut target.acyclic_types);
        core::mem::swap(&mut self.lexical_frames, &mut target.lexical_frames);
    }

    /// Restore one exact context, wherever it is currently suspended.
    fn restore_capsule(
        &mut self,
        old: &LexicalContext,
        field: Option<trial::Field>,
    ) {
        if same_owner(&self.lexical_owner, &old.owner) {
            if let Some(field) = field {
                let mut facts = LexicalFacts::capture(self);
                old.facts.restore_field(&mut facts, field);
                facts.install(self);
            } else {
                old.facts.install(self);
            }
            return;
        }
        if let Some(frame) = self
            .lexical_frames
            .iter_mut()
            .rev()
            .find(|frame| same_owner(&frame.owner, &old.owner))
        {
            let mut updated = (**frame).clone();
            if let Some(field) = field {
                old.facts.restore_field(&mut updated.facts, field);
            } else {
                updated = old.clone();
            }
            *frame = Rc::new(updated);
            return;
        }
        let pending = self
            .pending_defaults
            .iter_mut()
            .find(|pending| same_owner(&pending.context.owner, &old.owner))
            .expect("a queued default has its lexical context");
        let mut updated = (*pending.context).clone();
        if let Some(field) = field {
            old.facts.restore_field(&mut updated.facts, field);
        } else {
            updated = old.clone();
        }
        pending.context = Rc::new(updated);
    }
}

impl Infer<'_> {
    fn lexical_context(&self) -> Rc<LexicalContext> {
        Rc::new(LexicalContext {
            owner: self.state.lexical_owner.clone(),
            resolved: self
                .state
                .lexical_resolved
                .clone()
                .unwrap_or_else(|| Rc::new(self.resolved.clone())),
            self_type: self.self_type.clone(),
            home: self.declared_home.clone(),
            facts: LexicalFacts::capture(&self.state),
        })
    }

    fn push_owed(&mut self, mut owed: Owed) {
        owed.lexical_owner.clone_from(&self.state.lexical_owner);
        self.state.obligations.push(owed);
    }

    fn queued_lexical_context(
        &self,
        owner: &LexicalOwner,
    ) -> Rc<LexicalContext> {
        self.state
            .lexical_frames
            .iter()
            .rev()
            .find(|frame| same_owner(&frame.owner, owner))
            .cloned()
            .or_else(|| {
                self.state
                    .pending_defaults
                    .iter()
                    .find(|pending| same_owner(&pending.context.owner, owner))
                    .map(|pending| Rc::clone(&pending.context))
            })
            .expect("a queued lexical owner is active, suspended, or pending")
    }

    fn with_lexical_owner<R>(
        &mut self,
        owner: &LexicalOwner,
        action: impl FnOnce(&mut Infer<'_>) -> R,
    ) -> R {
        if same_owner(&self.state.lexical_owner, owner) {
            return action(self);
        }
        let target = self.queued_lexical_context(owner);
        let previous = self.lexical_context();
        let before: Vec<_> = self
            .state
            .pending_defaults
            .iter()
            .map(|p| Rc::clone(&p.context))
            .chain(self.state.lexical_frames.iter().cloned())
            .chain(core::iter::once(Rc::clone(&previous)))
            .collect();
        self.journal_shared_context();
        self.state.lexical_frames.push(Rc::clone(&previous));
        let mut inner = Infer {
            module: self.module,
            resolved: &target.resolved,
            declarations: self.declarations,
            files: self.files,
            frame: self.frame,
            visiting: self.visiting.clone(),
            file_cache: self.file_cache,
            indexes: self.indexes.clone(),
            expression_ids: self.expression_ids.clone(),
            operators: self.operators.clone(),
            weak: self.weak,
            state: State::new(Subst::default(), self.module.expressions.len()),
            items: Vec::new(),
            groups: Vec::new(),
            self_type: None,
            declared_home: None,
            checked_file_scope: self.checked_file_scope.clone(),
        };
        target.facts.install(&mut inner.state);
        self.state.move_common(&mut inner.state);
        inner.state.lexical_owner.clone_from(&target.owner);
        inner.state.lexical_resolved = Some(Rc::clone(&target.resolved));
        inner.items = core::mem::take(&mut self.items);
        inner.groups = core::mem::take(&mut self.groups);
        inner.weak = self.weak;
        inner.self_type.clone_from(&target.self_type);
        inner.declared_home.clone_from(&target.home);
        let guard = owner.as_ref().map(|owner| owner.key.clone());
        let pushed = guard
            .as_ref()
            .is_some_and(|key| !inner.state.defaults.contains(key));
        if pushed {
            inner
                .state
                .defaults
                .push(guard.clone().expect("owner guard"));
        }
        let result = action(&mut inner);
        if pushed {
            assert_eq!(inner.state.defaults.pop(), guard);
        }
        assert_eq!(
            inner.state.trials, 0,
            "lexical callbacks close their trials"
        );
        let updated = inner.lexical_context();
        if let Some(frame) = inner
            .state
            .lexical_frames
            .iter_mut()
            .rev()
            .find(|frame| same_owner(&frame.owner, owner))
        {
            *frame = updated;
        } else {
            inner
                .state
                .pending_defaults
                .iter_mut()
                .find(|p| same_owner(&p.context.owner, owner))
                .expect("pending owner")
                .context = updated;
        }
        let caller =
            inner.state.lexical_frames.pop().expect("suspended caller");
        inner.state.move_common(&mut self.state);
        caller.facts.install(&mut self.state);
        self.state.lexical_owner.clone_from(&caller.owner);
        self.state.lexical_resolved = Some(Rc::clone(&caller.resolved));
        self.items = inner.items;
        self.groups = inner.groups;
        self.self_type.clone_from(&caller.self_type);
        self.declared_home.clone_from(&caller.home);
        for old in before {
            self.state.journal(trial::Undo::Capsule(old));
        }
        result
    }
}

/// An obligation, where it arose, and the literal that raised it.
#[derive(Clone)]
struct Owed {
    contract_reads: Vec<crate::typed::MemberContractRead>,
    contract_use: Option<crate::typed::MemberContractUse>,
    contract_ordinal: Option<usize>,
    lexical_owner: LexicalOwner,
    obligation: Obligation,
    span: Span,
    literal: Option<Lit>,
    literal_argument: bool,
    /// A member obligation already solved, left in place so that marks
    /// into the list stay valid.
    solved: bool,
    assumed: bool,
}

impl Owed {
    const fn new(obligation: Obligation, span: Span) -> Self {
        Self {
            contract_reads: Vec::new(),
            contract_use: None,
            contract_ordinal: None,
            lexical_owner: None,
            obligation,
            span,
            literal: None,
            literal_argument: false,
            solved: false,
            assumed: false,
        }
    }
}

#[derive(Clone)]
enum Status {
    Pending,
    Typing,
    Done(alloc::boxed::Box<Generalized>),
}

#[derive(Clone)]
struct Use {
    lexical_owner: LexicalOwner,
    owner: Option<Type>,
    /// This use's monomorphic check ran; errors still invalidate completion.
    checked: bool,
    item: usize,
    ty: Type,
    span: Span,
}

/// One per-invocation default decision snapshot, private until source completion.
#[derive(Clone)]
struct PendingDefault {
    context: Rc<LexicalContext>,
    earlier: Vec<Slot>,
    instance: crate::typed::DeclaredExpression,
    uses: Vec<(usize, Use)>,
    invocation: core::ops::Range<usize>,
    roots: BTreeMap<(usize, usize), SavedRoot>,
}

/// A root snapshot and the exact paired callee it came from.
#[derive(Clone)]
struct SavedRoot {
    root: crate::typed::Root,
    callee: Type,
    owner: Option<Type>,
}

/// A member scheme and its owner share one set of quantified variables.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Generalized {
    scheme: Scheme,
    owner: Option<Type>,
}

/// A lexical binding's scheme; a session's frame keeps its accepted
/// lines' bindings in this form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Local {
    pub(crate) owner: Option<Type>,
    pub(crate) scheme: Scheme,
    /// The spot list of a binding whose value is an arm list.
    pub(crate) heads: Option<Vec<Head>>,
}

/// One head position of an arm list: its name, and whether it defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    name: Option<String>,
    default: bool,
    value: Option<ExprId>,
    site: Span,
}

/// A value of this module known before its line: an impl member, or
/// a written constructor.
#[derive(Clone)]
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
#[derive(Clone)]
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
    place_kind: crate::typed::Place,
    name: Option<String>,
    ty: Type,
    default: bool,
    /// A declared default and the spot's name, in the callee's home.
    value: Option<ExprId>,
    site: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContractBindingUse {
    origin: Origin,
    site: Span,
    at: Span,
}

/// A value about to be applied.
#[derive(Clone, Debug)]
struct Callee {
    contract_binding: Option<ContractBindingUse>,
    owner: Option<Type>,
    selected: Option<crate::typed::Selected>,
    meanings: Vec<Type>,
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
    /// A private field with no default of a record declared elsewhere:
    /// its derived constructor does not exist here.
    sealed: Option<String>,
}

impl Callee {
    const fn plain(ty: Type) -> Self {
        Self {
            contract_binding: None,
            owner: None,
            selected: None,
            meanings: Vec::new(),
            ty,
            spots: None,
            self_slot: None,
            supplied: None,
            parameters: Vec::new(),
            home: None,
            conversion: None,
            sealed: None,
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
    place_kind: crate::typed::Place,
    ty: Type,
    span: Span,
    /// At a spot, where a word takes the spot's type.
    spot: bool,
    cast: bool,
}

impl Expected {
    fn at(mut self, place_kind: crate::typed::Place) -> Self {
        self.place_kind = place_kind;
        self.cast = true;
        self
    }

    const fn new(ty: Type, span: Span) -> Self {
        Self {
            place_kind: crate::typed::Place::Result,
            ty,
            span,
            spot: false,
            cast: false,
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
    file_cache: &'a files::FileCache,
    indexes: Vec<(&'a Module, index::Index)>,
    expression_ids: BTreeMap<Span, usize>,
    /// Each operator's operands, by the operator's span.
    operators: BTreeMap<Span, (crate::ast::ExprId, Option<crate::ast::ExprId>)>,
    /// The session's weak variables, `Var(0)` up to this: free in every
    /// line after the one that left them.
    weak: u32,
    state: State,
    items: Vec<Item>,
    /// The item groups, one per impl in order.
    groups: Vec<Group>,
    /// `Self` inside an impl body.
    self_type: Option<Type>,
    declared_home: Option<Origin>,
    /// This expression's module is a completed File, with names local to it.
    checked_file_scope: Option<String>,
}

impl Infer<'_> {
    /// Classification follows the nominal owner, never its display label.
    fn resource(&self, ty: &Type) -> bool {
        let Type::Named { name, .. } = meaning(ty) else {
            return false;
        };
        self.owner(&name.origin).is_some_and(|module| module.entries.iter().any(|entry| {
            matches!(entry, Entry::Item(crate::ast::Item { kind: ItemKind::Type { resource: true, name: declared, .. }, .. }) if last(declared) == name.name)
        }))
    }

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
                Mismatch::DepthLimit => Reason::TypeDepth,
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
        self.push_owed(Owed::new(obligation, span));
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
                let contract_mark = self.state.obligations.len();
                let (ty, meanings, owner) = self.instantiate_member(
                    &local.scheme,
                    local.owner.as_ref(),
                    span,
                );
                let mut callee = self.with_heads(ty, local.heads.as_deref());
                callee.meanings = meanings;
                callee.owner = owner;
                if !self.state.locals.contains_key(&key) {
                    callee.home = self
                        .frame
                        .and_then(|frame| {
                            frame.lines.iter().position(|line| {
                                line.typed
                                    .frozen
                                    .iter()
                                    .any(|(at, _)| *at == key)
                            })
                        })
                        .map(Origin::Line);
                }
                callee.selected = Some(crate::typed::Selected::Value {
                    origin: callee.home.clone().unwrap_or(Origin::This),
                    site,
                });
                let binding = ContractBindingUse {
                    origin: callee.home.clone().unwrap_or(Origin::This),
                    site,
                    at: span,
                };
                self.attach_contract_binding(contract_mark, &binding);
                callee.contract_binding = Some(binding);
                if self.state.locals.contains_key(&key) {
                    self.local_contract_view(site, &mut callee);
                }
                callee
            }
            None => Callee::plain(self.fresh()),
        }
    }

    fn bind_local(&mut self, name: &Name, ty: Type) {
        let local = Local {
            owner: None,
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
                    target,
                    claim,
                    members,
                } = &item.kind
                {
                    self.declare_impl(*target, *claim, members);
                    let names = self.outside_names(item.span);
                    if let Some(group) = self.groups.last_mut() {
                        group.names = names;
                    }
                }
            }
        }
    }

    fn declare_source_bindings(&mut self) {
        for entry in &self.module.entries {
            let Entry::Line(id) = entry else { continue };
            let ExprKind::Bind {
                pattern,
                annotation,
                value,
                ..
            } = &self.module.expressions[id.0].kind
            else {
                continue;
            };
            let crate::ast::PatternKind::Name(name) =
                &self.module.patterns[pattern.0].kind
            else {
                continue;
            };
            let arms = match &self.module.expressions[value.0].kind {
                ExprKind::Arms(arms) => Some(arms.as_slice()),
                _ => None,
            };
            let ty = match (annotation, arms) {
                (Some(annotation), _) => self.lower_here(*annotation),
                (None, Some(arms)) => self.skeleton(arms),
                (None, None) => self.fresh(),
            };
            let mut item = Item::new(
                Key::Binding(last(name)),
                ty,
                arms.map(|a| self.heads(a)),
                name.span,
            );
            item.body = Some(Body::Value(*value));
            item.annotated = annotation.is_some();
            let group = self.groups.len();
            let first = self.items.len();
            self.push_item(item, group);
            self.groups.push(Group {
                members: vec![first],
                self_type: None,
                names: Vec::new(),
            });
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
        claim: Option<crate::ast::TypeId>,
        members: &[ImplMember],
    ) {
        let module = self.module;
        let group = self.groups.len();
        let ty = lower::named(module, target).unwrap_or_default();
        let self_type = self.lower_here(target);
        self.self_type = Some(self_type.clone());
        let class = claim.map(|id| self.lower_here(id));
        let required = class
            .as_ref()
            .map(|class| {
                self.member_limits(
                    class,
                    &self_type,
                    module.types[target.0].span,
                    &mut Vec::new(),
                )
            })
            .unwrap_or_default();
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
                    if let Some(class) = &class {
                        self.constrain_claim_member(
                            class,
                            &self_type,
                            &required,
                            &item,
                            module.types[target.0].span,
                        );
                    }
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

    /// Obligations settle, members resolve or fail, and
    /// every type is applied through the final substitution.
    fn limit_reads_line(&self, target: &Target, self_site: Span) -> bool {
        let site = match target {
            Target::Local { site } => *site,
            Target::Members { head, .. } => {
                return self.limit_reads_line(head, self_site)
            }
            _ => return false,
        };
        if site == self_site {
            return false;
        }
        core::iter::once(self.module)
            .chain(
                self.frame
                    .into_iter()
                    .flat_map(|f| f.lines.iter().map(|l| &l.module)),
            )
            .any(|module| {
                module.expressions.iter().any(|e| {
                    let ExprKind::Bind { pattern, value, .. } = e.kind else {
                        return false;
                    };
                    module.patterns[pattern.0].span == site
                        && !matches!(
                            module.expressions[value.0].kind,
                            ExprKind::Arms(_)
                        )
                })
            })
    }

    fn assume_type_parameters(
        &mut self,
        name: &Name,
        parameters: &[crate::ast::TypeParameter],
    ) -> (Type, usize) {
        let named = Type::named(
            Origin::This,
            &last(name),
            parameters
                .iter()
                .map(|p| Type::Rigid(last(&p.name)))
                .collect(),
        );
        let mut vars = lower::Vars::new(lower::SelfKind::Type(named.clone()));
        self.parameter_claims(&named, &mut vars, name.span);
        let mark = self.state.obligations.len();
        for obligation in vars.obligations {
            let mut owed = Owed::new(obligation, name.span);
            owed.solved = true;
            owed.assumed = true;
            self.push_owed(owed);
        }
        (named, mark)
    }

    /// A type's tests, each typed as a `Bool` with `self` bound.
    fn check_tests(&mut self, item: &crate::ast::Item) {
        let module = self.module;
        let resolved = self.resolved;
        let ItemKind::Type {
            resource: _,
            name,
            base,
            members,
            limits,
            parameters,
        } = &item.kind
        else {
            return;
        };
        let (named, mark) = self.assume_type_parameters(name, parameters);
        let owner = named.clone();
        let ty = self.limited(named);
        let ty = meaning(&ty).clone();
        self.bind_local(
            &Name {
                segments: vec![String::from("self")],
                span: name.span,
            },
            ty.clone(),
        );
        self.self_type = Some(ty);
        for limit in limits {
            if let crate::ast::Limit::Test(id) = limit {
                let span = module.expressions[id.0].span;
                if base.is_none() && members.is_none() {
                    self.error(
                        span,
                        Reason::Limit {
                            message: String::from("a test needs a base"),
                        },
                    );
                    continue;
                }
                let enters_self =
                    resolved.expressions().any(|(expr, target)| {
                        let expression = &module.expressions[expr.0];
                        span.start <= expression.span.start
                            && expression.span.end <= span.end
                            && test_enters_self(
                                module,
                                resolved,
                                expr,
                                target,
                                &last(name),
                            )
                    });
                let line_binding =
                    resolved.expressions().any(|(expr, target)| {
                        let expression = &module.expressions[expr.0];
                        span.start <= expression.span.start
                            && expression.span.end <= span.end
                            && self.limit_reads_line(target, name.span)
                    });
                if line_binding {
                    self.error(
                        span,
                        Reason::Limit {
                            message: String::from(
                                "a limit cannot read a line binding",
                            ),
                        },
                    );
                    continue;
                }
                if enters_self {
                    self.error(
                        span,
                        Reason::Limit {
                            message: String::from("a test enters its own type"),
                        },
                    );
                    continue;
                }
                self.record_local_test(&owner, name.span, *id);
                self.expr(
                    *id,
                    Some(&Expected::new(prelude("Bool"), name.span)),
                );
            }
        }
        self.dots();
        for owed in &mut self.state.obligations[mark..] {
            owed.assumed = false;
        }
        self.self_type = None;
    }

    fn record_decisions(&self) -> Typed {
        let mut decisions = self.state.decisions();
        let subst = &self.state.subst;
        decisions.casts = self
            .state
            .casts
            .iter()
            .map(|cast| {
                cast.as_ref().map(|cast| {
                    alloc::boxed::Box::new(crate::typed::Cast {
                        operation: cast
                            .operation
                            .as_ref()
                            .map(|operation| operation.applied(subst)),
                        row: cast.row,
                        from: subst.apply(&cast.from),
                        to: subst.apply(&cast.to),
                    })
                })
            })
            .collect();
        decisions.constructors = self.file_cache.used_constructors();
        decisions.schemes.extend(
            self.items.iter().zip(&self.state.status).filter_map(
                |(item, status)| {
                    let Status::Done(scheme) = status else {
                        return None;
                    };
                    Some((
                        (item.site.start, item.site.end),
                        bind::applied(&scheme.scheme, &self.state.subst),
                    ))
                },
            ),
        );
        if self.state.errors.is_empty() {
            self.complete_owned_references(&mut decisions);
            decisions.complete_references(&self.state.subst);
        }
        decisions.hidden = self.record_hidden(&decisions.schemes, &decisions);
        decisions
    }

    fn owned_arguments(
        &self,
        member: &Generalized,
        ty: &Type,
        actual_owner: &Type,
    ) -> Vec<Type> {
        let mut subst = self.state.subst.clone();
        let fresh: Vec<_> = member
            .scheme
            .variables
            .iter()
            .map(|v| (*v, Type::Var(subst.fresh())))
            .collect();
        subst
            .unify(&crate::types::replace(&member.scheme.ty, &fresh), ty)
            .expect("checked member reference");
        subst
            .unify(
                &crate::types::replace(
                    member.owner.as_ref().expect("member owner"),
                    &fresh,
                ),
                actual_owner,
            )
            .expect("checked member owner");
        fresh.iter().map(|(_, ty)| subst.apply(ty)).collect()
    }

    fn local_member(
        &self,
        selected: &crate::typed::Selected,
    ) -> Option<&Generalized> {
        let crate::typed::Selected::Value {
            origin: Origin::This,
            site,
        } = selected
        else {
            return None;
        };
        let index = self.items.iter().position(|item| item.site == *site)?;
        let Status::Done(member) = &self.state.status[index] else {
            return None;
        };
        member.owner.as_ref()?;
        Some(member)
    }

    fn complete_owned_references(&self, decisions: &mut Typed) {
        self.complete_saved_references(
            decisions,
            &self.state.uses,
            &self.state.roots,
        );
    }
    fn complete_saved_references(
        &self,
        decisions: &mut Typed,
        uses: &[Use],
        roots: &BTreeMap<(usize, usize), SavedRoot>,
    ) {
        for used in uses.iter().filter(|u| u.checked) {
            let Status::Done(member) = &self.state.status[used.item] else {
                continue;
            };
            let (Some(_), Some(owner)) = (&member.owner, &used.owner) else {
                continue;
            };
            decisions.meaning_arguments.insert(
                (used.span.start, used.span.end),
                self.owned_arguments(member, &used.ty, owner),
            );
        }
        for access in decisions.accesses.values_mut().flatten() {
            let Some(member) = self.local_member(&access.selected) else {
                continue;
            };
            access.meanings =
                self.owned_arguments(member, &access.callee, &access.head);
        }
        for (site, root) in &mut decisions.roots {
            let saved = &roots[site];
            let (Some(member), Some(owner)) =
                (self.local_member(&root.selected), &saved.owner)
            else {
                continue;
            };
            root.meanings = self.owned_arguments(member, &saved.callee, owner);
        }
        for row in decisions
            .dispatch
            .iter_mut()
            .flat_map(|dispatch| &mut dispatch.rows)
        {
            if let Some(member) = self.local_member(&row.selected) {
                row.meanings =
                    self.owned_arguments(member, &row.ty, &row.receiver);
            }
        }
    }

    fn finish(mut self) -> (Typed, Vec<Error>) {
        self.check_conversions();
        self.dots();
        self.default_literals(0, &[], &[]);
        self.finish_dots();
        self.solve_families(true);
        self.finish_rebuilds();
        let main = self.main_type();
        self.finish_rebuilds();
        self.left_members();
        self.record_supplied_tests();
        self.record_operations();
        let shapes = self.record_shapes();
        let unsolved = self.settle();
        let mut dispatch = self.record_dispatch(&shapes);
        loop {
            let requests: alloc::collections::BTreeSet<_> = dispatch
                .iter()
                .map(|dispatch| (dispatch.name.clone(), dispatch.receiverless))
                .collect();
            self.record_dispatch_input_tests(&dispatch);
            if !self.state.errors.is_empty() {
                break;
            }
            if self
                .dispatch_requests()
                .iter()
                .all(|request| requests.contains(request))
            {
                break;
            }
            // New checked row-input tests can call generic helpers. Rebuild
            // rows only when their finite semantic request set grows.
            dispatch = self.record_dispatch(&shapes);
        }
        let (frozen, weak, session_variables) = self.frozen();
        let decisions = self.record_decisions();
        let inputs = self.limit_inputs();
        let subst = &self.state.subst;
        let mut typed = Typed {
            dispatch,
            shapes,
            declared: self.file_cache.used(),
            constructors: self.file_cache.used_constructors(),
            main: Some(subst.apply(&main)),
            rebuilds: core::mem::take(&mut self.state.rebuilds),
            modes: core::mem::take(&mut self.state.modes),
            warnings: core::mem::take(&mut self.state.warnings),
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
            supplied: core::mem::take(&mut self.state.supplied),
            checks: core::mem::take(&mut self.state.checks)
                .into_iter()
                .map(|mut check| {
                    check.target = check.target.map(|ty| subst.apply(&ty));
                    check
                })
                .collect(),
            known: Vec::new(),
            facts: Vec::new(),
            frozen,
            weak,
            session_variables,
            weak_before: self.weak,
            ..decisions
        };
        let entries: Vec<_> = self
            .state
            .entries
            .iter()
            .map(|(id, ty, span, kind, variable, receiving_call)| {
                (
                    *id,
                    subst.apply(ty),
                    *span,
                    kind.clone(),
                    *variable,
                    *receiving_call,
                )
            })
            .collect();
        let mut errors = core::mem::take(&mut self.state.errors);
        errors
            .extend(crate::limits::check(&inputs, &entries, None, &mut typed));
        typed.hidden = self.record_hidden(&typed.schemes, &typed);
        import_checks(self.file_cache, &mut typed);
        errors.sort_by_key(|error| (error.span.start, error.span.end));
        (typed, errors)
    }
}

/// Every binding and item of a module by its site, frozen.
type Frozen = Vec<((usize, usize), Local)>;

/// Give a declaration's own stored facts the same canonical binders as its scheme.
pub(crate) fn canonical_body_type(
    ty: &Type,
    binders: &[(String, Var)],
) -> Type {
    bind::unrigid(ty, binders)
}

/// The first variable a frozen scheme quantifies: its variables are
/// numbered apart from the weak ones, which later lines extend.
const QUANTIFIED: u32 = 1 << 31;

impl Infer<'_> {
    fn freeze_member(
        scheme: &Scheme,
        owner: Option<&Type>,
        subst: &Subst,
        env: &[Var],
        weak: &[(Var, Type)],
    ) -> Generalized {
        let scheme = bind::applied(scheme, subst);
        let owner = owner.map(|owner| subst.apply(owner));
        let root = owner.as_ref().map_or_else(
            || scheme.ty.clone(),
            |owner| Type::Tuple(alloc::vec![scheme.ty.clone(), owner.clone()]),
        );
        let general = Scheme::generalize(
            &root,
            &scheme.obligations,
            env,
            &Subst::default(),
        );
        let mut quantified = if owner.is_some() {
            scheme.variables.clone()
        } else {
            Vec::new()
        };
        for var in general.variables {
            if !quantified.contains(&var) {
                quantified.push(var);
            }
        }
        quantified.retain(|var| !env.contains(var));
        let mut names = weak.to_vec();
        let mut variables = Vec::new();
        for (index, var) in quantified.iter().enumerate() {
            let name = Var(QUANTIFIED.saturating_add(count(index)));
            names.push((*var, Type::Var(name)));
            variables.push(name);
        }
        Generalized {
            owner: owner.map(|owner| replace(&owner, &names)),
            scheme: Scheme {
                variables,
                obligations: scheme
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
                ty: replace(&scheme.ty, &names),
            },
        }
    }

    /// Every binding and item by its site, applied and generalized
    /// against the file-level environment, and the weak variables after
    /// this module. A pattern-bound binding is monomorphic in a file, so
    /// it freezes monomorphic: its variables stay weak, shared with the
    /// lines after it, which may solve them as the file's later lines
    /// would. Every other variable is quantified.
    fn frozen(&self) -> (Frozen, Subst, BTreeMap<u32, usize>) {
        let subst = &self.state.subst;
        // The earlier lines' weak variables and the file-level
        // monomorphic types: parameters and blocks have left `env`.
        // Gathered through a set: the weak variables alone number the
        // session's, and `variables` scans its output for each one.
        let mut env = Vec::new();
        let mut seen = alloc::collections::BTreeSet::new();
        for var in 0..self.weak {
            gather(&subst.apply(&Type::Var(Var(var))), &mut seen, &mut env);
        }
        for ty in &self.state.env {
            gather(&subst.apply(ty), &mut seen, &mut env);
        }
        self.close(&mut env);
        // New weak variables follow the earlier ones.
        let mut weak: Vec<(Var, Type)> = Vec::new();
        for var in env.iter().filter(|var| var.0 >= self.weak) {
            let next = self.weak.saturating_add(count(weak.len()));
            weak.push((*var, Type::Var(Var(next))));
        }
        let freeze = |scheme: &Scheme, owner: Option<&Type>| {
            Self::freeze_member(scheme, owner, subst, &env, &weak)
        };
        let mut frozen: Vec<_> = self
            .state
            .locals
            .iter()
            .map(|(site, local)| {
                let pair = freeze(&local.scheme, local.owner.as_ref());
                (
                    *site,
                    Local {
                        scheme: pair.scheme,
                        owner: pair.owner,
                        heads: local.heads.clone(),
                    },
                )
            })
            .collect();
        for (item, status) in self.items.iter().zip(&self.state.status) {
            let pair = match status {
                Status::Done(member) => {
                    freeze(&member.scheme, member.owner.as_ref())
                }
                _ => freeze(&Scheme::mono(item.ty.clone()), None),
            };
            frozen.push((
                (item.site.start, item.site.end),
                Local {
                    scheme: pair.scheme,
                    owner: pair.owner,
                    heads: item.heads.clone(),
                },
            ));
        }
        let earlier = (0..self.weak).map(|var| {
            let ty = subst.apply(&Type::Var(Var(var)));
            (ty != Type::Var(Var(var))).then(|| replace(&ty, &weak))
        });
        let later = weak.iter().map(|_| None);
        let session_variables = weak
            .iter()
            .map(|(var, target)| {
                let Type::Var(target) = target else {
                    unreachable!()
                };
                (var.0, target.0 as usize)
            })
            .collect();
        (
            frozen,
            Subst::from_bindings(earlier.chain(later).collect()),
            session_variables,
        )
    }
}

fn count(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// `ty`'s variables appended to `out` in first-occurrence order, with
/// `seen` holding what `out` holds, so a long list costs no scan.
pub(crate) fn gather(
    ty: &Type,
    seen: &mut alloc::collections::BTreeSet<Var>,
    out: &mut Vec<Var>,
) {
    let mut found = Vec::new();
    ty.variables(&mut found);
    for var in found {
        if seen.insert(var) {
            out.push(var);
        }
    }
}

pub(crate) fn meaning(ty: &Type) -> &Type {
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

impl<'a> Infer<'a> {
    fn new(
        module: &'a Module,
        resolved: &'a Resolved,
        declarations: &'a Declarations,
        files: &'a [(String, Module)],
        frame: Option<&'a Frame>,
        visiting: &[String],
        cache: &'a files::FileCache,
    ) -> Self {
        let indexes = core::iter::once(module)
            .chain(core::iter::once(&declarations.prelude.module))
            .chain(declarations.commands.iter().map(|n| &n.module))
            .chain(declarations.modules.iter().map(|n| &n.module))
            .chain(files.iter().map(|(_, m)| m))
            .map(|m| (m, index::index(m)))
            .collect();
        let subst = frame.map_or_else(Subst::default, |f| f.weak.clone());
        Self {
            module,
            resolved,
            declarations,
            files,
            frame,
            visiting: visiting.to_vec(),
            file_cache: cache,
            indexes,
            expression_ids: expression_ids(module),
            operators: operators(module),
            weak: subst.count(),
            state: State::new(subst, module.expressions.len()),
            items: Vec::new(),
            groups: Vec::new(),
            self_type: None,
            declared_home: None,
            checked_file_scope: None,
        }
    }
}

/// Each binary or `!` operator's operands, by the operator's span.
fn operators(
    module: &Module,
) -> BTreeMap<Span, (crate::ast::ExprId, Option<crate::ast::ExprId>)> {
    let mut operators = BTreeMap::new();
    for expr in &module.expressions {
        let (operator, operands) = match expr.kind {
            crate::ast::ExprKind::Binary {
                operator,
                left,
                right,
                ..
            } => (operator, (left, Some(right))),
            crate::ast::ExprKind::Not { operator, value } => {
                (operator, (value, None))
            }
            _ => continue,
        };
        operators.entry(operator).or_insert(operands);
    }
    operators
}

fn expression_ids(module: &Module) -> BTreeMap<Span, usize> {
    let mut ids = BTreeMap::new();
    for (i, expression) in module.expressions.iter().enumerate() {
        ids.entry(expression.span).or_insert(i);
    }
    ids
}

pub(crate) fn check_declarations(
    declarations: &Declarations,
    origin: Origin,
) -> Vec<Error> {
    let namespace = match &origin {
        Origin::Prelude => &declarations.prelude,
        Origin::Commands => declarations.commands.as_ref().unwrap(),
        Origin::Host(name) => declarations.get_module(name).unwrap(),
        _ => unreachable!(),
    };
    let module = &namespace.module;
    let (resolved, _) =
        crate::resolve::resolve_partial(&Module::default(), declarations, &[]);
    let cache = files::FileCache::default();
    let mut infer =
        Infer::new(module, &resolved, declarations, &[], None, &[], &cache);
    let scope = lower::Scope::Declared { module, origin };
    for entry in &module.entries {
        if let Entry::Item(crate::ast::Item {
            kind:
                ItemKind::Impl {
                    target,
                    claim: Some(claim),
                    ..
                },
            span,
            ..
        }) = entry
        {
            let mut vars = lower::Vars::new(lower::SelfKind::None);
            let mut head = infer.lower(&scope, *target, &mut vars);
            // Explicit impl parameters are universal. An omitted target's
            // arguments still come from its member signature (e.g. Vec Literal).
            if matches!(&module.types[target.0].kind, crate::ast::TypeKind::Named { arguments, .. } if !arguments.is_empty())
            {
                let mut variables = Vec::new();
                head.variables(&mut variables);
                let rigid: Vec<_> = variables
                    .into_iter()
                    .map(|v| (v, Type::Rigid(alloc::format!("claim{}", v.0))))
                    .collect();
                head = crate::types::replace(&head, &rigid);
                for (_, ty) in &mut vars.names {
                    *ty = crate::types::replace(ty, &rigid);
                }
            }
            if let Type::Named { name, arguments } = &mut head {
                if let Some((parameters, _)) =
                    infer.type_item(module, &name.name)
                {
                    while arguments.len() < parameters.len() {
                        arguments.push(infer.fresh());
                    }
                }
            }
            vars.self_kind = lower::SelfKind::Type(head.clone());
            let class = infer.lower(&scope, *claim, &mut vars);
            infer.check_supplied_claim(&head, &class, *span);
        }
    }
    infer.state.errors
}

fn test_enters_self(
    module: &Module,
    resolved: &Resolved,
    expr: ExprId,
    target: &Target,
    name: &str,
) -> bool {
    match target {
        Target::Type {
            origin: Origin::This,
            name: target,
        } => target == name,
        Target::Constructor {
            origin: Origin::This,
            ty,
            ..
        } => ty == name,
        Target::Value {
            origin: Origin::Prelude,
            name: target,
        } if matches!(target.as_str(), "to" | "try" | "read") => {
            matches!(&module.expressions[expr.0].kind, ExprKind::Instance { arguments, .. } if arguments.iter().any(|arg| matches!(resolved.ty(*arg), Some(Target::Type { origin: Origin::This, name: target }) if target == name)))
        }
        _ => false,
    }
}

impl State {
    fn applied_access_decisions(
        &self,
        pending: &[Rc<Typed>],
    ) -> BTreeMap<(usize, usize), Vec<crate::typed::Access>> {
        self.accesses
            .iter()
            .map(|(site, steps)| {
                (
                    *site,
                    steps
                        .iter()
                        .map(|step| {
                            step.applied_preserving(&self.subst, pending)
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn applied_member_contracts(
        &self,
    ) -> BTreeMap<crate::typed::MemberContractUse, crate::typed::MemberContract>
    {
        let subst = &self.subst;
        self.member_contracts
            .iter()
            .map(|(key, contract)| {
                let mut contract = contract.clone();
                contract.selected = contract.selected.applied(subst);
                contract.meanings = contract
                    .meanings
                    .iter()
                    .map(|ty| subst.apply(ty))
                    .collect();
                contract.receiver = subst.apply(&contract.receiver);
                contract.callable = subst.apply(&contract.callable);
                (key.clone(), contract)
            })
            .collect()
    }

    fn applied_derived_defaults(
        &self,
        pending: &[Rc<Typed>],
    ) -> BTreeMap<(usize, usize), crate::typed::DefaultValue> {
        self.derived_defaults
            .iter()
            .map(|(site, value)| {
                let mut value = value.clone();
                record::apply_default_preserving(
                    &mut value,
                    &self.subst,
                    pending,
                );
                (*site, value)
            })
            .collect()
    }

    fn decisions(&self) -> Typed {
        let subst = &self.subst;
        let pending: Vec<_> = self
            .pending_defaults
            .iter()
            .map(|plan| alloc::rc::Rc::clone(&plan.instance.typed))
            .collect();
        let mut bound = self.bound.clone();
        for binding in bound.values_mut() {
            record::apply_bound_preserving(binding, subst, &pending);
        }
        Typed {
            member_contract_reads: self.member_contract_reads.clone(),
            member_contracts: self.applied_member_contracts(),
            constructor_fields: self.constructor_fields.clone(),
            derived_defaults: self.applied_derived_defaults(&pending),
            body_parameters: self.body_parameters.clone(),
            tests: self
                .tests
                .iter()
                .map(|test| crate::typed::Test {
                    owner: subst.apply(&test.owner),
                    ..test.clone()
                })
                .collect(),
            default_instances: self
                .default_instances
                .iter()
                .map(|(site, instance)| {
                    (
                        *site,
                        record::apply_recorded_preserving(
                            instance, subst, &pending,
                        ),
                    )
                })
                .collect(),
            rebuild_types: self
                .rebuild_types
                .iter()
                .map(|(site, ty)| (*site, subst.apply(ty)))
                .collect(),
            accesses: self.applied_access_decisions(&pending),
            roots: self
                .roots
                .iter()
                .map(|(site, saved)| (*site, saved.root.applied(subst)))
                .collect(),
            schemes: self
                .locals
                .iter()
                .map(|(site, local)| {
                    (*site, bind::applied(&local.scheme, subst))
                })
                .collect(),
            bound,
            pattern_types: self
                .pattern_types
                .iter()
                .map(|(i, t)| (*i, subst.apply(t)))
                .collect(),
            meaning_arguments: self
                .meaning_arguments
                .iter()
                .map(|(s, ts)| {
                    (*s, ts.iter().map(|t| subst.apply(t)).collect())
                })
                .collect(),
            conversions: self
                .conversions
                .iter()
                .map(|(site, conversion)| (*site, conversion.applied(subst)))
                .collect(),
            pattern_equals: self
                .pattern_equals
                .iter()
                .map(|(id, comparison)| (*id, comparison.applied(subst)))
                .collect(),
            selected: self
                .selected
                .iter()
                .map(|(site, selected)| {
                    let selected = selected.applied(subst);
                    (*site, selected)
                })
                .collect(),
            callee_types: self
                .callee_types
                .iter()
                .map(|(s, t)| (*s, subst.apply(t)))
                .collect(),
            ..Typed::default()
        }
    }
}

fn import_checks(cache: &files::FileCache, typed: &mut Typed) {
    for (stem, checked) in cache.files.borrow().iter() {
        for check in &checked.typed.checks {
            if check.place_origin != Origin::This {
                continue;
            }
            let mut check = check.clone();
            check.place_origin = Origin::File(stem.clone());
            if check.limit_origin == Origin::This {
                check.limit_origin = Origin::File(stem.clone());
            }
            check.target = check
                .target
                .map(|ty| ty.rehome(&Origin::File(stem.clone())));
            check.known.rehome(&Origin::File(stem.clone()));
            typed.checks.push(check);
        }
    }
}

impl<'a> Infer<'a> {
    pub(crate) const fn limit_inputs(&self) -> crate::limits::Inputs<'a> {
        crate::limits::Inputs {
            module: self.module,
            resolved: self.resolved,
            declarations: self.declarations,
            files: self.files,
            cache: self.file_cache,
            frame: self.frame,
        }
    }
}
