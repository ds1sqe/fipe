use super::{Index, Origin, Rc, RefCell};

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
            self.tables.borrow_mut().session_meanings[self.weak_slots[index]] =
                value;
        }
        self.weak_before.remove(&origin);
    }
}
