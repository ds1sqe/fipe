use super::{Index, Origin, Rc, RefCell};

impl Index {
    /// Queries own a copy of the tables and cannot update accepted units.
    pub fn fork(&self) -> Self {
        Self {
            tables: Rc::new(RefCell::new(self.tables.borrow().clone())),
            ..self.clone()
        }
    }

    pub fn checkpoint(&self) -> Self {
        Self {
            dispatch_lengths: self
                .tables
                .borrow()
                .dispatch
                .iter()
                .map(|dispatch| dispatch.rows.len())
                .collect(),
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
        for (variable, weak) in &typed.session_variables {
            self.session_variables
                .insert((home.clone(), *variable), self.weak_slots[*weak]);
        }
        // Frozen weak-variable indices differ from inference's raw indices.
        let origin = Origin::This;
        let saved = self.session_variables.clone();
        for (variable, slot) in self.weak_slots.iter().enumerate() {
            self.session_variables.insert(
                (
                    origin.clone(),
                    u32::try_from(variable).expect("weak variable count"),
                ),
                *slot,
            );
        }
        for (index, binding) in typed.weak.bindings().iter().enumerate() {
            let value = binding.as_ref().map(|ty| {
                let ty = ty.rehome(&home);
                self.ty(&origin, &typed.weak.apply(&ty))
            });
            self.tables.borrow_mut().session_meanings[self.weak_slots[index]] =
                value;
        }
        self.session_variables = saved;
    }

    pub fn restore_meanings(&self, saved: &[Option<crate::ir::Type>]) {
        for (index, dispatch) in
            self.tables.borrow_mut().dispatch.iter_mut().enumerate()
        {
            dispatch.rows.truncate(
                self.dispatch_lengths.get(index).copied().unwrap_or(0),
            );
        }
        for (index, meaning) in self
            .tables
            .borrow_mut()
            .session_meanings
            .iter_mut()
            .enumerate()
        {
            meaning.clone_from(&saved.get(index).cloned().flatten());
        }
    }
}
