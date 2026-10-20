use super::{ir, Index, Origin, Rc, RefCell, Shape, TypeName, Vec};

impl Index {
    /// Queries own a copy of the tables and cannot update accepted units.
    pub fn fork(&self) -> Self {
        Self {
            tables: Rc::new(RefCell::new(self.tables.borrow().clone())),
            ..self.clone()
        }
    }

    pub fn session(&mut self, line: usize, typed: &crate::typed::Typed) {
        let home = Origin::Line(line);
        self.home = Some(home.clone());
        while self.weak_slots.len() < typed.weak.bindings().len() {
            let slot = self.tables.borrow().session_meanings.len();
            self.tables.borrow_mut().session_meanings.push(None);
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
            let slot = self.weak_slots[index];
            self.note(|index, mark| {
                (slot < mark.session_meanings).then(|| {
                    Undo::SessionMeaning(
                        slot,
                        index.tables.borrow().session_meanings[slot].clone(),
                    )
                })
            });
            self.tables.borrow_mut().session_meanings[self.weak_slots[index]] =
                value;
        }
        self.weak_before.remove(&origin);
    }
}

/// The lengths of the append-only tables and of the index's own
/// sequences before a session line is emitted.
#[derive(Clone, Debug)]
pub struct Mark {
    pub(super) namespaces: usize,
    pub(super) keys: usize,
    pub(super) meanings: usize,
    pub(super) dispatch: usize,
    pub(super) definitions: usize,
    pub(super) declarations: usize,
    pub(super) modules: usize,
    pub(super) tests: usize,
    pub(super) entries: usize,
    pub(super) nodes: usize,
    pub(super) slots: usize,
    pub(super) session_meanings: usize,
    pub(super) parameters: usize,
    pub(super) constant_defaults: usize,
    pub(super) weak_slots: usize,
    pub(super) emitted: usize,
    pub(super) home: Option<Origin>,
}

/// A session line's mark and the writes it made to what existed before
/// it: what undoes the line.
#[derive(Clone, Debug)]
pub struct Journal {
    pub(super) mark: Mark,
    pub(super) undos: Vec<Undo>,
}

impl Journal {
    /// How many value definitions and modules stood before the line.
    pub const fn before(&self) -> (usize, usize) {
        (self.mark.definitions, self.mark.modules)
    }
}

/// A write a session line made to something that existed before it,
/// with what was there.
#[derive(Clone, Debug)]
pub enum Undo {
    Meaning(usize, Vec<usize>, Vec<ir::Constructor>),
    SessionMeaning(usize, Option<ir::Type>),
    SlotDefinition(usize, Option<usize>),
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
        let tables = self.tables.borrow();
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
        self.journal = Some(Journal {
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
        self.journal.take().expect("a line was begun")
    }

    /// Undo a kept line, given its journal, while no line kept after it
    /// stands.
    pub fn withdraw(&mut self, journal: Journal) {
        self.undo(journal);
    }

    /// Journal a write, when a line may be refused.
    pub(super) fn note(
        &mut self,
        undo: impl FnOnce(&Self, &Mark) -> Option<Undo>,
    ) {
        let Some(Journal { mark, .. }) = &self.journal else {
            return;
        };
        if let Some(undo) = undo(self, mark) {
            if let Some(Journal { undos, .. }) = &mut self.journal {
                undos.push(undo);
            }
        }
    }

    /// Journal a node the line is about to rewrite, when it existed.
    pub(super) fn keep_node(&mut self, node: usize) {
        self.note(|index, mark| {
            (node < mark.nodes).then(|| {
                Undo::Node(node, index.tables.borrow().nodes[node].clone())
            })
        });
    }

    /// Undo the line begun last.
    pub fn rollback(&mut self) {
        if let Some(journal) = self.journal.take() {
            self.undo(journal);
        }
    }

    /// Every journaled write in reverse, then every table and collection
    /// back to its mark.
    fn undo(&mut self, Journal { mark, undos }: Journal) {
        let line = self.home.clone();
        {
            let mut tables = self.tables.borrow_mut();
            for undo in undos.into_iter().rev() {
                match undo {
                    Undo::Meaning(index, parameters, constructors) => {
                        tables.meanings[index].parameters = parameters;
                        tables.meanings[index].constructors = constructors;
                    }
                    Undo::SessionMeaning(slot, value) => {
                        tables.session_meanings[slot] = value;
                    }
                    Undo::SlotDefinition(slot, definition) => {
                        tables.slots[slot].definition = definition;
                    }
                    Undo::Node(index, node) => tables.nodes[index] = node,
                    Undo::Rows(index, rows) => {
                        tables.dispatch[index].rows.truncate(rows);
                    }
                    Undo::Slot(key, slot) => {
                        restore(&mut self.slots, key, slot);
                    }
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
        }
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

/// Put a map's entry back as it was: the old value, or no entry.
fn restore<K: Ord, V>(
    map: &mut alloc::collections::BTreeMap<K, V>,
    key: K,
    value: Option<V>,
) {
    match value {
        Some(value) => {
            map.insert(key, value);
        }
        None => {
            map.remove(&key);
        }
    }
}
