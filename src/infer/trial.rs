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
    MeaningArguments((usize, usize), Option<Vec<Type>>),
    DefaultInstance(
        (usize, usize, usize),
        Option<crate::typed::DeclaredExpression>,
    ),
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
            Undo::Cast(index, old) => self.casts[index] = old.map(|cast| *cast),
            Undo::Binding(index, old) => self.bindings[index] = old,
            Undo::Status(index, old) => self.status[index] = old,
            Undo::Flags(index, solved, assumed) => {
                let owed = &mut self.obligations[index];
                owed.solved = solved;
                owed.assumed = assumed;
            }
            Undo::Selected(key, old) => put(&mut self.selected, key, old),
            Undo::MeaningArguments(key, old) => {
                put(&mut self.meaning_arguments, key, old);
            }
            Undo::DefaultInstance(key, old) => {
                put(&mut self.default_instances, key, old);
            }
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
            let mine = matches!(
                (&undo, field),
                (
                    Undo::Selected(..) | Undo::MeaningArguments(..),
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

    pub(super) fn set_type(&mut self, index: usize, ty: Type) {
        let old = self.state.types[index].replace(ty);
        self.state.journal(Undo::Type(index, old));
    }

    pub(super) fn set_cast(&mut self, index: usize, cast: crate::typed::Cast) {
        let old = self.state.casts[index].replace(cast).map(Box::new);
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
