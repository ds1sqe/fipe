//! Trial readings. A place may read an expression twice, against its
//! type and then as a cast source, and a dispatch tries candidates that
//! must leave nothing behind. A trial begins at a [`Snapshot`] and ends by
//! being kept or restored; while one is open, every write to the state
//! that is not a push is journaled, so restoring costs what the trial
//! wrote rather than a copy of the state.

use super::{Infer, State, Status};
use crate::types::{Scheme, SubstMark, Type};
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};

/// One write to undo, newest last in the journal.
pub(super) enum Undo {
    Type(usize, Option<Type>),
    Cast(usize, Option<Box<crate::typed::Cast>>),
    Binding(usize, Option<Scheme>),
    Status(usize, Status),
    /// An obligation's `solved` and `assumed` flags.
    Flags(usize, bool, bool),
    Selected((usize, usize), Option<crate::typed::Selected>),
    MemberContract(
        crate::typed::MemberContractUse,
        Option<Box<crate::typed::MemberContract>>,
    ),
    ContractStage((usize, usize), Option<super::ContractBindingUse>),
    ContractRefinement(
        (
            crate::typed::MemberContractUse,
            crate::token::Span,
            crate::token::Span,
            usize,
            usize,
        ),
        Option<Type>,
    ),
    MeaningArguments((usize, usize), Option<Vec<Type>>),
    DefaultInstance(
        (usize, usize, usize),
        Option<crate::typed::DeclaredExpression>,
    ),
    Capsule(alloc::rc::Rc<super::LexicalContext>),
    Other(Box<dyn FnOnce(&mut State)>),
}

/// Where a trial began: the journal's length, the push-only lists'
/// lengths and the scalars.
pub(super) struct Snapshot {
    journal: usize,
    subst: SubstMark,
    errors: usize,
    warnings: usize,
    entries: usize,
    checks: usize,
    obligations: usize,
    tests: usize,
    families: usize,
    rebuilding: usize,
    defaults: usize,
    main_lifts: usize,
    claims: usize,
    uses: usize,
    deferred: usize,
    env: usize,
    lowering: usize,
    dots_start: usize,
    solving_dots: bool,
}

/// Which of the journaled fields a scoped restore undoes.
#[derive(Clone, Copy)]
pub(super) enum Field {
    Selection,
    DefaultInstances,
}

impl State {
    pub(super) const fn journaling(&self) -> bool {
        self.trials > 0
    }

    pub(super) fn journal(&mut self, undo: Undo) {
        if self.trials > 0 {
            self.journal.push(undo);
        }
    }

    fn undo(&mut self, undo: Undo) {
        match undo {
            Undo::Type(index, old) => self.types[index] = old,
            Undo::Cast(index, old) => self.casts[index] = old,
            Undo::Binding(index, old) => self.bindings[index] = old,
            Undo::Status(index, old) => self.status[index] = old,
            Undo::Flags(index, solved, assumed) => {
                let owed = &mut self.obligations[index];
                owed.solved = solved;
                owed.assumed = assumed;
            }
            Undo::Selected(key, old) => put(&mut self.selected, key, old),
            Undo::MemberContract(key, old) => {
                put(&mut self.member_contracts, key, old.map(|old| *old));
            }
            Undo::ContractStage(key, old) => {
                put(&mut self.contract_stages, key, old);
            }
            Undo::ContractRefinement(key, old) => {
                put(&mut self.contract_refinements, key, old);
            }
            Undo::MeaningArguments(key, old) => {
                put(&mut self.meaning_arguments, key, old);
            }
            Undo::DefaultInstance(key, old) => {
                put(&mut self.default_instances, key, old);
            }
            Undo::Capsule(old) => self.restore_capsule(&old, None),
            Undo::Other(undo) => undo(self),
        }
    }
}

fn put<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, old: Option<V>) {
    match old {
        Some(value) => {
            map.insert(key, value);
        }
        None => {
            map.remove(&key);
        }
    }
}

impl Infer<'_> {
    pub(super) fn journal_shared_context(&mut self) {
        self.journal_list(|s| &mut s.obligations);
        self.journal_list(|s| &mut s.families);
        self.journal_list(|s| &mut s.rebuilding);
        self.journal_list(|s| &mut s.status);
        self.journal_list(|s| &mut s.uses);
        self.journal_list(|s| &mut s.deferred);
        self.journal_list(|s| &mut s.pending_defaults);
        self.journal_list(|s| &mut s.errors);
        self.journal_list(|s| &mut s.warnings);
        self.journal_list(|s| &mut s.defaults);
        self.journal_list(|s| &mut s.lowering);
        self.journal_list(|s| &mut s.lexical_frames);
        if self.state.journaling() {
            let dots_start = self.state.dots_start;
            let solving_dots = self.state.solving_dots;
            let lowered = self.state.lowered.clone();
            let acyclic_types = self.state.acyclic_types.clone();
            self.state.journal.push(Undo::Other(Box::new(move |state| {
                state.dots_start = dots_start;
                state.solving_dots = solving_dots;
                state.lowered = lowered;
                state.acyclic_types = acyclic_types;
            })));
        }
    }

    /// Begin a trial reading.
    pub(super) fn snapshot(&mut self) -> Snapshot {
        let state = &mut self.state;
        state.trials += 1;
        Snapshot {
            journal: state.journal.len(),
            subst: state.subst.mark(),
            errors: state.errors.len(),
            warnings: state.warnings.len(),
            entries: state.entries.len(),
            checks: state.checks.len(),
            obligations: state.obligations.len(),
            tests: state.tests.len(),
            families: state.families.len(),
            rebuilding: state.rebuilding.len(),
            defaults: state.defaults.len(),
            main_lifts: state.main_lifts.len(),
            claims: state.claims.len(),
            uses: state.uses.len(),
            deferred: state.deferred.len(),
            env: state.env.len(),
            lowering: state.lowering.len(),
            dots_start: state.dots_start,
            solving_dots: state.solving_dots,
        }
    }

    /// Keep what the trial read.
    pub(super) fn keep(&mut self, snapshot: Snapshot) {
        let state = &mut self.state;
        state.subst.keep(snapshot.subst);
        state.trials -= 1;
        if state.trials == 0 {
            state.journal.clear();
        }
    }

    /// Undo everything since the snapshot.
    pub(super) fn restore(&mut self, snapshot: Snapshot) {
        let state = &mut self.state;
        while state.journal.len() > snapshot.journal {
            let undo = state.journal.pop().expect("journal entry");
            state.undo(undo);
        }
        state.subst.restore(snapshot.subst);
        state.errors.truncate(snapshot.errors);
        state.warnings.truncate(snapshot.warnings);
        state.entries.truncate(snapshot.entries);
        state.checks.truncate(snapshot.checks);
        state.obligations.truncate(snapshot.obligations);
        state.tests.truncate(snapshot.tests);
        state.families.truncate(snapshot.families);
        state.rebuilding.truncate(snapshot.rebuilding);
        state.defaults.truncate(snapshot.defaults);
        state.main_lifts.truncate(snapshot.main_lifts);
        state.claims.truncate(snapshot.claims);
        state.uses.truncate(snapshot.uses);
        state.deferred.truncate(snapshot.deferred);
        state.env.truncate(snapshot.env);
        state.lowering.truncate(snapshot.lowering);
        state.dots_start = snapshot.dots_start;
        state.solving_dots = snapshot.solving_dots;
        state.trials -= 1;
        if state.trials == 0 {
            state.journal.clear();
        }
    }

    /// Keep the trial but undo its writes to one field: a lookup whose
    /// selections or default instances must not stand.
    pub(super) fn keep_except(&mut self, snapshot: Snapshot, field: Field) {
        let state = &mut self.state;
        let mut rest = Vec::new();
        while state.journal.len() > snapshot.journal {
            let undo = state.journal.pop().expect("journal entry");
            if let Undo::Capsule(old) = &undo {
                state.restore_capsule(old, Some(field));
                rest.push(undo);
                continue;
            }
            let mine = matches!(
                (&undo, field),
                (
                    Undo::Selected(..)
                        | Undo::MeaningArguments(..)
                        | Undo::MemberContract(..)
                        | Undo::ContractStage(..)
                        | Undo::ContractRefinement(..),
                    Field::Selection,
                ) | (Undo::DefaultInstance(..), Field::DefaultInstances)
            );
            if mine {
                state.undo(undo);
            } else {
                rest.push(undo);
            }
        }
        state.journal.extend(rest.into_iter().rev());
        self.keep(snapshot);
    }

    pub(super) fn set_member_contract(
        &mut self,
        key: crate::typed::MemberContractUse,
        contract: crate::typed::MemberContract,
    ) {
        let old = self.state.member_contracts.insert(key.clone(), contract);
        if self.state.journaling() {
            self.state
                .journal(Undo::MemberContract(key, old.map(Box::new)));
        }
    }

    pub(super) fn set_contract_stage(
        &mut self,
        key: (usize, usize),
        binding: Option<super::ContractBindingUse>,
    ) {
        let old = if let Some(binding) = binding {
            self.state.contract_stages.insert(key, binding)
        } else {
            self.state.contract_stages.remove(&key)
        };
        self.state.journal(Undo::ContractStage(key, old));
    }

    pub(super) fn set_contract_refinement(
        &mut self,
        key: (
            crate::typed::MemberContractUse,
            crate::token::Span,
            crate::token::Span,
            usize,
            usize,
        ),
        ty: Type,
    ) {
        let old = self.state.contract_refinements.insert(key.clone(), ty);
        self.state.journal(Undo::ContractRefinement(key, old));
    }

    pub(super) fn set_type(&mut self, index: usize, ty: Type) {
        let old = self.state.types[index].replace(ty);
        self.state.journal(Undo::Type(index, old));
    }

    pub(super) fn set_cast(&mut self, index: usize, cast: crate::typed::Cast) {
        let old = self.state.casts[index].replace(Box::new(cast));
        self.state.journal(Undo::Cast(index, old));
    }

    pub(super) fn set_binding(&mut self, index: usize, scheme: Scheme) {
        let old = self.state.bindings[index].replace(scheme);
        self.state.journal(Undo::Binding(index, old));
    }

    pub(super) fn set_status(&mut self, index: usize, status: Status) {
        let old = core::mem::replace(&mut self.state.status[index], status);
        self.state.journal(Undo::Status(index, old));
    }

    /// Mark an obligation solved, in place.
    pub(super) fn solve_obligation(&mut self, index: usize) {
        let owed = &self.state.obligations[index];
        let flags = Undo::Flags(index, owed.solved, owed.assumed);
        self.state.journal(flags);
        self.state.obligations[index].solved = true;
    }

    pub(super) fn select(
        &mut self,
        key: (usize, usize),
        selected: crate::typed::Selected,
    ) {
        let old = self.state.selected.insert(key, selected);
        self.state.journal(Undo::Selected(key, old));
    }

    pub(super) fn set_meaning_arguments(
        &mut self,
        key: (usize, usize),
        arguments: Vec<Type>,
    ) {
        let old = self.state.meaning_arguments.insert(key, arguments);
        self.state.journal(Undo::MeaningArguments(key, old));
    }

    pub(super) fn set_default_instance(
        &mut self,
        key: (usize, usize, usize),
        instance: crate::typed::DeclaredExpression,
    ) {
        let old = self.state.default_instances.insert(key, instance);
        self.state.journal(Undo::DefaultInstance(key, old));
    }

    /// Insert into one of the state's span-keyed maps.
    pub(super) fn insert<K, V>(
        &mut self,
        map: fn(&mut State) -> &mut BTreeMap<K, V>,
        key: K,
        value: V,
    ) where
        K: Ord + Clone + 'static,
        V: 'static,
    {
        let old = map(&mut self.state).insert(key.clone(), value);
        if self.state.journaling() {
            self.state.journal.push(Undo::Other(Box::new(move |state| {
                put(map(state), key, old);
            })));
        }
    }

    /// Before one entry of a span-keyed map is changed in place.
    pub(super) fn journal_key<K, V>(
        &mut self,
        map: fn(&mut State) -> &mut BTreeMap<K, V>,
        key: K,
    ) where
        K: Ord + Clone + 'static,
        V: Clone + 'static,
    {
        if self.state.journaling() {
            let old = map(&mut self.state).get(&key).cloned();
            self.state.journal.push(Undo::Other(Box::new(move |state| {
                put(map(state), key, old);
            })));
        }
    }

    /// Before a list of the state is rebuilt rather than pushed to.
    pub(super) fn journal_list<T: Clone + 'static>(
        &mut self,
        list: fn(&mut State) -> &mut Vec<T>,
    ) {
        if self.state.journaling() {
            let old = list(&mut self.state).clone();
            self.state.journal.push(Undo::Other(Box::new(move |state| {
                *list(state) = old;
            })));
        }
    }
}

#[cfg(test)]
mod lexical_tests {
    use super::*;
    use crate::{
        ast::ExprId, decl::Declarations, resolve::Origin, token::Span,
    };
    use alloc::rc::Rc;

    fn selection() -> crate::typed::Selected {
        crate::typed::Selected::Derived {
            ty: Type::Unit,
            kind: crate::ir::Derived::Default,
        }
    }

    #[test]
    fn lexical_capsule_selection_keep_preserves_other_facts_for_outer_restore()
    {
        let module = crate::parse::parse(crate::lex::lex("1")).expect("module");
        let declarations = Declarations::new();
        let resolved = crate::resolve::resolve(&module, &declarations, &[])
            .expect("resolved");
        let cache = super::super::files::FileCache::new();
        let mut infer = Infer::new(
            &module,
            &resolved,
            &declarations,
            &[],
            None,
            &[],
            &cache,
        );
        let outer = infer.snapshot();
        let selected_trial = infer.snapshot();
        let before = infer.lexical_context();
        infer.state.journal(Undo::Capsule(before));
        infer.state.selected.insert((0, 1), selection());
        infer
            .state
            .meaning_arguments
            .insert((0, 1), alloc::vec![Type::Unit]);
        infer.state.types[0] = Some(Type::Unit);
        infer.keep_except(selected_trial, Field::Selection);
        assert!(infer.state.selected.is_empty());
        assert!(infer.state.meaning_arguments.is_empty());
        assert_eq!(infer.state.types[0], Some(Type::Unit));
        infer.restore(outer);
        assert_eq!(infer.state.types[0], None);
    }

    #[test]
    fn lexical_capsule_default_listing_keeps_new_token_and_embedded_instance() {
        let module = crate::parse::parse(crate::lex::lex("1")).expect("module");
        let declarations = Declarations::new();
        let resolved = crate::resolve::resolve(&module, &declarations, &[])
            .expect("resolved");
        let cache = super::super::files::FileCache::new();
        let mut infer = Infer::new(
            &module,
            &resolved,
            &declarations,
            &[],
            None,
            &[],
            &cache,
        );
        let outer = infer.snapshot();
        let listing = infer.snapshot();
        let token = Rc::new(super::super::DefaultOwner {
            key: (Origin::This, Span { start: 0, end: 1 }),
        });
        let mut context = (*infer.lexical_context()).clone();
        context.owner = Some(Rc::clone(&token));
        let context = Rc::new(context);
        let instance = crate::typed::DeclaredExpression {
            origin: Origin::This,
            value: ExprId(0),
            resolved: Rc::new(resolved.clone()),
            typed: Rc::new(crate::typed::Typed::default()),
        };
        infer.journal_list(|state| &mut state.pending_defaults);
        infer
            .state
            .pending_defaults
            .push(super::super::PendingDefault {
                context: Rc::clone(&context),
                earlier: Vec::new(),
                instance: instance.clone(),
                uses: Vec::new(),
                invocation: 0..0,
                roots: BTreeMap::new(),
            });
        infer.state.journal(Undo::Capsule(context));
        infer.state.pending_defaults[0].context = {
            let mut changed =
                (*infer.state.pending_defaults[0].context).clone();
            changed
                .facts
                .default_instances
                .insert((0, 1, 0), instance.clone());
            changed.facts.types[0] = Some(Type::Unit);
            Rc::new(changed)
        };
        infer.keep_except(listing, Field::DefaultInstances);
        assert_eq!(infer.state.pending_defaults.len(), 1);
        assert!(infer.state.pending_defaults[0]
            .context
            .facts
            .default_instances
            .is_empty());
        assert_eq!(
            infer.state.pending_defaults[0].context.facts.types[0],
            Some(Type::Unit)
        );
        assert!(Rc::ptr_eq(
            &instance.typed,
            &infer.state.pending_defaults[0].instance.typed
        ));
        infer.restore(outer);
        assert!(infer.state.pending_defaults.is_empty());
    }
}

#[cfg(test)]
mod lexical_guard_tests {
    use super::*;
    use crate::{decl::Declarations, resolve::Origin, token::Span};
    use alloc::rc::Rc;

    #[test]
    fn inactive_default_owner_reinstates_its_signature_guard_across_unit_work()
    {
        let module = crate::parse::parse(crate::lex::lex("1")).expect("module");
        let declarations = Declarations::new();
        let resolved = crate::resolve::resolve(&module, &declarations, &[])
            .expect("resolved");
        let cache = super::super::files::FileCache::new();
        let mut infer = Infer::new(
            &module,
            &resolved,
            &declarations,
            &[],
            None,
            &[],
            &cache,
        );
        let key = (Origin::This, Span { start: 0, end: 1 });
        let token = Rc::new(super::super::DefaultOwner { key: key.clone() });
        let owner = Some(Rc::clone(&token));
        let mut context = (*infer.lexical_context()).clone();
        context.owner = owner.clone();
        let instance = crate::typed::DeclaredExpression {
            origin: Origin::This,
            value: crate::ast::ExprId(0),
            resolved: Rc::new(resolved.clone()),
            typed: Rc::new(crate::typed::Typed::default()),
        };
        infer
            .state
            .pending_defaults
            .push(super::super::PendingDefault {
                context: Rc::new(context),
                earlier: Vec::new(),
                instance,
                uses: Vec::new(),
                invocation: 0..0,
                roots: BTreeMap::new(),
            });
        infer.with_lexical_owner(&owner, |inner| {
            assert_eq!(inner.state.defaults, alloc::vec![key.clone()]);
            inner.with_lexical_owner(&None, |unit| {
                assert_eq!(unit.state.defaults, alloc::vec![key.clone()]);
                unit.with_lexical_owner(&owner, |nested| {
                    assert_eq!(nested.state.defaults, alloc::vec![key.clone()]);
                });
            });
        });
        assert!(infer.state.defaults.is_empty());
    }

    #[test]
    fn member_contract_trial_restore_and_selection_keep_preserve_source_ownership(
    ) {
        let module = crate::parse::parse(crate::lex::lex("1")).unwrap();
        let declarations = Declarations::new();
        let resolved =
            crate::resolve::resolve(&module, &declarations, &[]).unwrap();
        let cache = super::super::files::FileCache::new();
        let mut infer = Infer::new(
            &module,
            &resolved,
            &declarations,
            &[],
            None,
            &[],
            &cache,
        );
        let site = Span { start: 0, end: 1 };
        let read = crate::typed::MemberContractRead {
            span: site,
            step: 0,
        };
        let mut owed = super::super::Owed::new(
            crate::types::Obligation::Member {
                head: Type::Unit,
                name: alloc::string::String::from("op"),
                ty: Type::Unit,
                dot: site,
            },
            site,
        );
        owed.contract_reads.push(read.clone());
        infer.push_owed(owed);
        let key = crate::typed::MemberContractUse {
            origin: Origin::This,
            binding: site,
            at: site,
            obligation: 0,
        };
        let contract = crate::typed::MemberContract {
            selected: crate::typed::Selected::Derived {
                ty: Type::Unit,
                kind: crate::ir::Derived::Default,
            },
            home: Origin::This,
            meanings: Vec::new(),
            receiver: Type::Unit,
            callable: Type::function(Type::Unit, Type::Unit),
            reads: alloc::vec![read.clone()],
        };
        let binding = super::super::ContractBindingUse {
            origin: Origin::This,
            site,
            at: site,
        };
        let refinement = (key.clone(), site, site, 0, 0);
        let trial = infer.snapshot();
        infer.journal_list(|s| &mut s.obligations);
        infer.state.obligations[0].contract_reads.push(
            crate::typed::MemberContractRead {
                span: site,
                step: 1,
            },
        );
        infer.insert(
            |s| &mut s.member_contract_reads,
            (0, 1),
            alloc::vec![alloc::vec![read.clone()]],
        );
        infer.set_member_contract(key.clone(), contract.clone());
        infer.set_contract_stage((0, 1), Some(binding.clone()));
        infer.set_contract_refinement(refinement.clone(), Type::Unit);
        let facts = super::super::LexicalFacts::capture(&infer.state);
        infer.state.member_contract_reads.clear();
        infer.state.member_contracts.clear();
        infer.state.contract_stages.clear();
        infer.state.contract_refinements.clear();
        facts.install(&mut infer.state);
        assert_eq!(infer.state.member_contracts[&key], contract);
        assert_eq!(
            infer.state.member_contract_reads[&(0, 1)],
            alloc::vec![alloc::vec![read.clone()]]
        );
        assert_eq!(infer.state.contract_stages[&(0, 1)], binding);
        assert_eq!(infer.state.contract_refinements[&refinement], Type::Unit);
        infer.restore(trial);
        assert_eq!(
            infer.state.obligations[0].contract_reads,
            alloc::vec![read.clone()]
        );
        assert!(infer.state.member_contract_reads.is_empty());
        assert!(infer.state.member_contracts.is_empty());
        assert!(infer.state.contract_stages.is_empty());
        assert!(infer.state.contract_refinements.is_empty());

        selection_keeps_contract_source_reads(
            &mut infer, key, contract, binding, refinement, read,
        );
    }
    fn selection_keeps_contract_source_reads(
        infer: &mut Infer<'_>,
        key: crate::typed::MemberContractUse,
        contract: crate::typed::MemberContract,
        binding: super::super::ContractBindingUse,
        refinement: (crate::typed::MemberContractUse, Span, Span, usize, usize),
        read: crate::typed::MemberContractRead,
    ) {
        let outer = infer.snapshot();
        let selected = infer.snapshot();
        infer.insert(
            |s| &mut s.member_contract_reads,
            (0, 1),
            alloc::vec![alloc::vec![read]],
        );
        infer.set_member_contract(key, contract);
        infer.set_contract_stage((0, 1), Some(binding));
        infer.set_contract_refinement(refinement, Type::Unit);
        infer.keep_except(selected, Field::Selection);
        assert!(infer.state.member_contracts.is_empty());
        assert!(infer.state.contract_stages.is_empty());
        assert!(infer.state.contract_refinements.is_empty());
        assert!(!infer.state.member_contract_reads.is_empty());
        infer.restore(outer);
        assert!(infer.state.member_contract_reads.is_empty());
    }
}
