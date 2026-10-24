use super::{ir, Index, Origin, Rc, RefCell, Shape, TypeName, Vec};
use alloc::collections::BTreeMap;
use core::cell::{Ref, RefMut};

impl Index {
    /// Queries own a copy of the tables and cannot update accepted units.
    pub fn fork(&self) -> Self {
        Self {
            store: self.store.fork(),
            ..self.clone()
        }
    }

    pub fn session(&mut self, line: usize, typed: &crate::typed::Typed) {
        let home = Origin::Line(line);
        self.home = Some(home.clone());
        while self.weak_slots.len() < typed.weak.bindings().len() {
            let slot = self.store.push_session_meaning(None);
            self.weak_slots.push(slot);
        }
        self.weak_before
            .insert(home.clone(), typed.weak_before as usize);
        for (variable, weak) in &typed.session_variables {
            self.session_variables
                .insert((home.clone(), *variable), self.weak_slots[*weak]);
        }
        // Frozen weak-variable indices differ from inference's raw
        // indices: in the frozen numbering every slot is its number.
        let origin = Origin::This;
        self.weak_before
            .insert(origin.clone(), self.weak_slots.len());
        for (index, binding) in typed.weak.bindings().iter().enumerate() {
            let value = binding.as_ref().map(|ty| {
                let ty = ty.rehome(&home);
                self.ty(&origin, &typed.weak.apply(&ty))
            });
            *self.store.session_meaning_mut(self.weak_slots[index]) = value;
        }
        self.weak_before.remove(&origin);
    }
}

/// The lengths of the append-only tables and of the index's own
/// sequences before a session line is emitted.
#[derive(Clone, Debug)]
pub struct Mark {
    namespaces: usize,
    keys: usize,
    meanings: usize,
    dispatch: usize,
    definitions: usize,
    declarations: usize,
    modules: usize,
    tests: usize,
    entries: usize,
    nodes: usize,
    slots: usize,
    session_meanings: usize,
    parameters: usize,
    constant_defaults: usize,
    weak_slots: usize,
    emitted: usize,
    home: Option<Origin>,
}

/// A session line's mark and the writes it made to what existed before
/// it: what undoes the line.
#[derive(Clone, Debug)]
pub struct Journal {
    mark: Mark,
    undos: Vec<Undo>,
}

impl Journal {
    /// How many value definitions and modules stood before the line.
    pub const fn before(&self) -> (usize, usize) {
        (self.mark.definitions, self.mark.modules)
    }

    /// How many limit tests stood before the line.
    pub const fn tests_before(&self) -> usize {
        self.mark.tests
    }
}

/// A write a session line made to something that existed before it,
/// with what was there.
#[derive(Clone, Debug)]
enum Undo {
    Meaning(usize, ir::Meaning),
    SessionMeaning(usize, Option<ir::Type>),
    SlotEntry(usize, ir::Slot),
    Node(usize, ir::Node),
    Rows(usize, usize),
    Slot((Origin, crate::token::Span), Option<usize>),
    Shape(TypeName, Option<(Origin, Shape)>),
    Visible(crate::token::Span, Option<usize>),
    SourceModule(Origin, Option<Rc<crate::ast::Module>>),
}

impl Index {
    /// Start a session line that may still be refused: later writes to
    /// what exists are journaled, so `rollback` restores the index
    /// without copying it.
    pub fn begin(&mut self) {
        let tables = self.store.tables();
        let mark = Mark {
            namespaces: tables.namespaces.len(),
            keys: tables.keys.len(),
            meanings: tables.meanings.len(),
            dispatch: tables.dispatch.len(),
            definitions: tables.definitions.len(),
            declarations: tables.declarations.len(),
            modules: tables.modules.len(),
            tests: tables.tests.len(),
            entries: tables.entries.len(),
            nodes: tables.nodes.len(),
            slots: tables.slots.len(),
            session_meanings: tables.session_meanings.len(),
            parameters: self.parameters.len(),
            constant_defaults: self.constant_defaults.len(),
            weak_slots: self.weak_slots.len(),
            emitted: self.emitted.len(),
            home: self.home.clone(),
        };
        drop(tables);
        self.store.journal = Some(Journal {
            mark,
            undos: Vec::new(),
        });
    }

    /// Keep the line. Its journal is returned: given back to `withdraw`
    /// before another line is kept, it undoes the line.
    ///
    /// # Panics
    /// If no line was begun.
    pub const fn commit(&mut self) -> Journal {
        self.store.journal.take().expect("a line was begun")
    }

    /// Undo a kept line, given its journal, while no line kept after it
    /// stands.
    pub fn withdraw(&mut self, journal: Journal) {
        self.undo(journal);
    }

    /// Undo the line begun last.
    pub fn rollback(&mut self) {
        if let Some(journal) = self.store.journal.take() {
            self.undo(journal);
        }
    }

    /// The store back to its mark, then the index's own collections.
    fn undo(&mut self, journal: Journal) {
        let line = self.home.clone();
        let mark = self.store.undo(journal);
        self.namespaces.retain(|_, index| *index < mark.namespaces);
        self.meanings.retain(|_, index| *index < mark.meanings);
        self.parameters.retain(|_, index| *index < mark.parameters);
        self.definitions
            .retain(|_, index| *index < mark.definitions);
        self.declarations
            .retain(|_, index| *index < mark.declarations);
        self.dispatch.retain(|_, index| *index < mark.dispatch);
        for tests in self.gates.values_mut() {
            tests.retain(|test| *test < mark.tests);
        }
        self.gates.retain(|_, tests| !tests.is_empty());
        self.constant_defaults.truncate(mark.constant_defaults);
        self.weak_slots.truncate(mark.weak_slots);
        self.emitted.truncate(mark.emitted);
        if line != mark.home {
            if let Some(line) = &line {
                self.session_variables.retain(|(home, _), _| home != line);
                self.weak_before.remove(line);
            }
        }
        self.home = mark.home;
    }
}

/// The tables a session's units share, with the index's maps that a
/// later line may overwrite. Its fields are private to this module: an
/// append is free, and a write to an entry that stood before the line
/// goes through a method here that journals it, so no writer can leave
/// a write out of `rollback` and `withdraw`.
#[derive(Clone, Debug, Default)]
pub struct Store {
    tables: Rc<RefCell<ir::Tables>>,
    slots: BTreeMap<(Origin, crate::token::Span), usize>,
    shapes: BTreeMap<TypeName, (Origin, Shape)>,
    visible: BTreeMap<crate::token::Span, usize>,
    source_modules: BTreeMap<Origin, Rc<crate::ast::Module>>,
    /// While a session line may still be refused: its mark and the
    /// writes it made to what existed before it.
    journal: Option<Journal>,
}

/// An append to one of the tables, returning the new entry's index.
macro_rules! append {
    ($($name:ident: $field:ident, $ty:ty;)*) => {
        $(
            pub fn $name(&self, value: $ty) -> usize {
                let mut tables = self.tables.borrow_mut();
                tables.$field.push(value);
                tables.$field.len() - 1
            }
        )*
    };
}

impl Store {
    /// A fixed declaration-environment fact, not line-local runtime identity.
    pub fn resources(&self, present: bool) {
        self.tables.borrow_mut().has_resources = present;
    }

    /// The tables, for reading.
    pub fn tables(&self) -> Ref<'_, ir::Tables> {
        self.tables.borrow()
    }

    /// The first node of the line being emitted, or 0 outside a session
    /// line: every node from it on is the line's own.
    pub fn first_node(&self) -> usize {
        self.journal
            .as_ref()
            .map_or(0, |journal| journal.mark.nodes)
    }

    /// The tables an emitted unit shares with later stages.
    pub fn unit(&self) -> Rc<RefCell<ir::Tables>> {
        Rc::clone(&self.tables)
    }

    /// An own copy of the tables and maps, for a query; no journal.
    fn fork(&self) -> Self {
        Self {
            tables: Rc::new(RefCell::new(self.tables.borrow().clone())),
            journal: None,
            ..self.clone()
        }
    }

    append! {
        push_namespace: namespaces, alloc::string::String;
        push_key: keys, alloc::string::String;
        push_meaning: meanings, ir::Meaning;
        push_dispatch: dispatch, ir::Dispatch;
        push_definition: definitions, ir::Definition;
        push_declaration: declarations, ir::Declaration;
        push_module: modules, ir::Module;
        push_test: tests, ir::Test;
        push_entry: entries, ir::Entry;
        push_node: nodes, ir::Node;
        push_slot: slots, ir::Slot;
        push_session_meaning: session_meanings, Option<ir::Type>;
    }

    /// Journal a write, when a line may be refused and what it changes
    /// stood before the line.
    fn note(&mut self, undo: impl FnOnce(&ir::Tables, &Mark) -> Option<Undo>) {
        if let Some(Journal { mark, undos }) = &mut self.journal {
            if let Some(undo) = undo(&self.tables.borrow(), mark) {
                undos.push(undo);
            }
        }
    }

    /// A node to rewrite.
    pub fn node_mut(&mut self, index: usize) -> RefMut<'_, ir::Node> {
        self.note(|tables, mark| {
            (index < mark.nodes)
                .then(|| Undo::Node(index, tables.nodes[index].clone()))
        });
        RefMut::map(self.tables.borrow_mut(), |tables| &mut tables.nodes[index])
    }

    /// A meaning to fill.
    pub fn meaning_mut(&mut self, index: usize) -> RefMut<'_, ir::Meaning> {
        self.note(|tables, mark| {
            (index < mark.meanings)
                .then(|| Undo::Meaning(index, tables.meanings[index].clone()))
        });
        RefMut::map(self.tables.borrow_mut(), |tables| {
            &mut tables.meanings[index]
        })
    }

    /// A slot to complete.
    pub fn slot_mut(&mut self, index: usize) -> RefMut<'_, ir::Slot> {
        self.note(|tables, mark| {
            (index < mark.slots)
                .then(|| Undo::SlotEntry(index, tables.slots[index].clone()))
        });
        RefMut::map(self.tables.borrow_mut(), |tables| &mut tables.slots[index])
    }

    /// A session meaning to fix.
    pub fn session_meaning_mut(
        &mut self,
        slot: usize,
    ) -> RefMut<'_, Option<ir::Type>> {
        self.note(|tables, mark| {
            (slot < mark.session_meanings).then(|| {
                Undo::SessionMeaning(
                    slot,
                    tables.session_meanings[slot].clone(),
                )
            })
        });
        RefMut::map(self.tables.borrow_mut(), |tables| {
            &mut tables.session_meanings[slot]
        })
    }

    /// Add a row to a dispatch table.
    pub fn push_row(&mut self, dispatch: usize, row: ir::DispatchRow) {
        self.note(|tables, mark| {
            (dispatch < mark.dispatch).then(|| {
                Undo::Rows(dispatch, tables.dispatch[dispatch].rows.len())
            })
        });
        self.tables.borrow_mut().dispatch[dispatch].rows.push(row);
    }

    pub const fn slots(
        &self,
    ) -> &BTreeMap<(Origin, crate::token::Span), usize> {
        &self.slots
    }

    pub const fn shapes(&self) -> &BTreeMap<TypeName, (Origin, Shape)> {
        &self.shapes
    }

    pub const fn visible(&self) -> &BTreeMap<crate::token::Span, usize> {
        &self.visible
    }

    pub const fn source_modules(
        &self,
    ) -> &BTreeMap<Origin, Rc<crate::ast::Module>> {
        &self.source_modules
    }

    pub fn insert_slot(
        &mut self,
        key: (Origin, crate::token::Span),
        slot: usize,
    ) {
        let old = self.slots.insert(key.clone(), slot);
        self.note(|_, _| Some(Undo::Slot(key, old)));
    }

    pub fn insert_shape(&mut self, key: TypeName, shape: (Origin, Shape)) {
        let old = self.shapes.insert(key.clone(), shape);
        self.note(|_, _| Some(Undo::Shape(key, old)));
    }

    pub fn insert_visible(&mut self, site: crate::token::Span, slot: usize) {
        let old = self.visible.insert(site, slot);
        self.note(|_, _| Some(Undo::Visible(site, old)));
    }

    pub fn insert_source_module(
        &mut self,
        home: Origin,
        module: Rc<crate::ast::Module>,
    ) {
        let old = self.source_modules.insert(home.clone(), module);
        self.note(|_, _| Some(Undo::SourceModule(home, old)));
    }

    /// Every journaled write in reverse, then every table back to its
    /// mark; the mark, for the index's own collections.
    fn undo(&mut self, Journal { mark, undos }: Journal) -> Mark {
        let mut tables = self.tables.borrow_mut();
        for undo in undos.into_iter().rev() {
            match undo {
                Undo::Meaning(index, meaning) => {
                    tables.meanings[index] = meaning;
                }
                Undo::SessionMeaning(slot, value) => {
                    tables.session_meanings[slot] = value;
                }
                Undo::SlotEntry(index, slot) => tables.slots[index] = slot,
                Undo::Node(index, node) => tables.nodes[index] = node,
                Undo::Rows(index, rows) => {
                    tables.dispatch[index].rows.truncate(rows);
                }
                Undo::Slot(key, slot) => restore(&mut self.slots, key, slot),
                Undo::Shape(key, shape) => {
                    restore(&mut self.shapes, key, shape);
                }
                Undo::Visible(key, slot) => {
                    restore(&mut self.visible, key, slot);
                }
                Undo::SourceModule(key, module) => {
                    restore(&mut self.source_modules, key, module);
                }
            }
        }
        tables.namespaces.truncate(mark.namespaces);
        tables.keys.truncate(mark.keys);
        tables.meanings.truncate(mark.meanings);
        tables.resource_cache.truncate(mark.meanings);
        tables.dispatch.truncate(mark.dispatch);
        tables.definitions.truncate(mark.definitions);
        tables.declarations.truncate(mark.declarations);
        tables.modules.truncate(mark.modules);
        tables.tests.truncate(mark.tests);
        tables.entries.truncate(mark.entries);
        tables.nodes.truncate(mark.nodes);
        tables.slots.truncate(mark.slots);
        tables.session_meanings.truncate(mark.session_meanings);
        tables.prepared.retain(|node, _| *node < mark.nodes);
        drop(tables);
        mark
    }
}

/// Put a map's entry back as it was: the old value, or no entry.
fn restore<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, value: Option<V>) {
    match value {
        Some(value) => {
            map.insert(key, value);
        }
        None => {
            map.remove(&key);
        }
    }
}
