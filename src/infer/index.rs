//! Read-only declaration and impl lookup for the modules in one unit.
use super::Infer;
use crate::ast::{Entry, ItemKind, Module, TypeKind};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

pub(super) type Index = BTreeMap<String, Vec<usize>>;

pub(super) fn index(module: &Module) -> Index {
    let mut index = Index::new();
    for (i, entry) in module.entries.iter().enumerate() {
        if let Some(name) = owner(module, entry) {
            index.entry(String::from(name)).or_default().push(i);
        }
    }
    index
}

fn owner<'a>(module: &'a Module, entry: &'a Entry) -> Option<&'a str> {
    let Entry::Item(item) = entry else {
        return None;
    };
    let name = match &item.kind {
        ItemKind::Type { name, .. } => name,
        ItemKind::Impl { target, .. } => {
            let TypeKind::Named { name, .. } = &module.types[target.0].kind
            else {
                return None;
            };
            name
        }
        _ => return None,
    };
    name.segments.last().map(String::as_str)
}

impl Infer<'_> {
    pub(super) fn type_entries<'m>(
        &self,
        module: &'m Module,
        name: &str,
    ) -> Vec<&'m Entry> {
        if let Some((_, index)) =
            self.indexes.iter().find(|(m, _)| core::ptr::eq(*m, module))
        {
            return index
                .get(name)
                .into_iter()
                .flatten()
                .map(|i| &module.entries[*i])
                .collect();
        }
        // Accepted session units already have an owner index in Frame; each
        // selected unit is normally one declaration or impl.
        module
            .entries
            .iter()
            .filter(|e| owner(module, e) == Some(name))
            .collect()
    }
}
