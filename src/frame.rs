//! What a session's accepted lines defined. A later line is resolved and
//! typed against this frame, never against their text, so an accepted
//! line is frozen: its names keep the targets and schemes they had.
//!
//! An accepted line's own declarations take `Origin::Line(index)`; its
//! bindings stay lexical, `Target::Local` at their sites in the session's
//! text, with their frozen schemes.

use crate::{
    ast::{Entry, ImplMember, ItemKind, Module},
    infer::Local,
    resolve::{Definition, Origin, Resolved, Target},
    token::Span,
    typed::Typed,
    types::{Subst, Type},
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

/// A name's span in the session's text: the key of a frozen binding.
pub type Site = (usize, usize);

/// A type an accepted line declared: its line and its name.
pub type Owner = (usize, String);

/// The definitions of every accepted line, the latest winning.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub lines: Vec<Line>,
    pub values: BTreeMap<String, Target>,
    pub types: BTreeMap<String, Target>,
    pub locals: BTreeMap<Site, Local>,
    pub facts: BTreeMap<Site, crate::limits::known::Known>,
    /// The lines holding impls of a type, in order.
    pub impls: BTreeMap<Owner, Vec<usize>>,
    /// An inherent member of a type, by name: its binding's site.
    pub members: BTreeMap<(Owner, String), Site>,
    /// A type's written constructor: its binding's site.
    pub constructors: BTreeMap<Owner, Site>,
    /// The weak variables: a monomorphic binding's variables, which a
    /// later line may solve. Every line's inference starts from them.
    pub weak: Subst,
}

/// One accepted line: its parse, its resolution, its typing.
#[derive(Clone, Debug)]
pub struct Line {
    pub module: Module,
    pub resolved: Resolved,
    pub typed: Typed,
}

#[derive(Clone, Debug, Default)]
struct Delta {
    values: Vec<(String, Target)>,
    types: Vec<(String, Target)>,
    locals: Vec<(Site, Local)>,
    facts: Vec<(Site, crate::limits::known::Known)>,
    impls: Vec<Owner>,
    members: Vec<((Owner, String), Site)>,
    constructors: Vec<(Owner, Site)>,
    /// The weak variables after the line.
    weak: Subst,
}

impl Frame {
    /// The module of an accepted line's declaration.
    pub fn module(&self, origin: &Origin) -> Option<&Module> {
        match origin {
            Origin::Line(index) => Some(&self.lines.get(*index)?.module),
            _ => None,
        }
    }

    /// Freeze a checked line as the next accepted one. What it replaced
    /// is returned: given back to `withdraw` while it is the last line,
    /// it takes the line back.
    pub fn accept(
        &mut self,
        module: Module,
        resolved: Resolved,
        typed: &Typed,
    ) -> Undo {
        let index = self.lines.len();
        let mut delta = delta(index, &module, &resolved, typed);
        delta.weak = rebase_weak(&typed.weak, index);
        delta.facts.clone_from(&typed.facts);
        for (_, known) in &mut delta.facts {
            known.rehome(&Origin::Line(index));
        }
        let undo = self.add(index, delta);
        let mut typed = typed.clone();
        for (_, known) in &mut typed.facts {
            known.rehome(&Origin::Line(index));
        }
        // The weak variables live in the frame, once, not in every line.
        typed.weak = Subst::default();
        self.lines.push(Line {
            module,
            resolved,
            typed,
        });
        undo
    }

    /// Take back the last accepted line, given what its `accept`
    /// returned.
    pub fn withdraw(&mut self, undo: Undo) {
        self.lines.pop();
        restore(&mut self.values, undo.values);
        restore(&mut self.types, undo.types);
        restore(&mut self.locals, undo.locals);
        restore(&mut self.facts, undo.facts);
        restore(&mut self.members, undo.members);
        restore(&mut self.constructors, undo.constructors);
        for owner in undo.impls.iter().rev() {
            if let Some(lines) = self.impls.get_mut(owner) {
                lines.pop();
                if lines.is_empty() {
                    self.impls.remove(owner);
                }
            }
        }
        let mut bindings = self.weak.bindings().to_vec();
        bindings.resize(undo.weak.count, None);
        for (index, old) in undo.weak.changed {
            bindings[index] = old;
        }
        self.weak = Subst::from_bindings(bindings);
    }

    fn add(&mut self, index: usize, delta: Delta) -> Undo {
        for owner in &delta.impls {
            self.impls.entry(owner.clone()).or_default().push(index);
        }
        Undo {
            values: insert(&mut self.values, delta.values),
            types: insert(&mut self.types, delta.types),
            locals: insert(&mut self.locals, delta.locals),
            facts: insert(&mut self.facts, delta.facts),
            impls: delta.impls,
            members: insert(&mut self.members, delta.members),
            constructors: insert(&mut self.constructors, delta.constructors),
            weak: weak_undo(
                &core::mem::replace(&mut self.weak, delta.weak),
                &self.weak,
            ),
        }
    }
}

/// What an accepted line replaced in the frame: each key it wrote with
/// the value before, in writing order.
#[derive(Clone, Debug)]
pub struct Undo {
    values: Vec<(String, Option<Target>)>,
    types: Vec<(String, Option<Target>)>,
    locals: Vec<(Site, Option<Local>)>,
    facts: Vec<(Site, Option<crate::limits::known::Known>)>,
    impls: Vec<Owner>,
    members: Vec<((Owner, String), Option<Site>)>,
    constructors: Vec<(Owner, Option<Site>)>,
    weak: WeakUndo,
}

/// What a line changed in the weak variables, which every line carries
/// in full: how many there were, and each of those it solved with its
/// value before.
#[derive(Clone, Debug)]
struct WeakUndo {
    count: usize,
    changed: Vec<(usize, Option<Type>)>,
}

fn weak_undo(before: &Subst, after: &Subst) -> WeakUndo {
    let (before, after) = (before.bindings(), after.bindings());
    WeakUndo {
        count: before.len(),
        changed: before
            .iter()
            .enumerate()
            .filter(|(index, old)| after.get(*index) != Some(*old))
            .map(|(index, old)| (index, old.clone()))
            .collect(),
    }
}

/// Write each entry, returning what each key held before.
fn insert<K: Ord + Clone, V>(
    map: &mut BTreeMap<K, V>,
    entries: Vec<(K, V)>,
) -> Vec<(K, Option<V>)> {
    entries
        .into_iter()
        .map(|(key, value)| {
            let old = map.insert(key.clone(), value);
            (key, old)
        })
        .collect()
}

/// Put back what `insert` replaced, the last write first.
fn restore<K: Ord, V>(map: &mut BTreeMap<K, V>, entries: Vec<(K, Option<V>)>) {
    for (key, old) in entries.into_iter().rev() {
        match old {
            Some(old) => {
                map.insert(key, old);
            }
            None => {
                map.remove(&key);
            }
        }
    }
}

/// What line `index` defines, with its declarations moved from
/// `Origin::This` to `Origin::Line(index)`.
fn delta(
    index: usize,
    module: &Module,
    resolved: &Resolved,
    typed: &Typed,
) -> Delta {
    let frozen: BTreeMap<Site, &Local> = typed
        .frozen
        .iter()
        .map(|(site, local)| (*site, local))
        .collect();
    let mut delta = Delta::default();
    let keep = |site: Span, delta: &mut Delta| -> bool {
        let key = (site.start, site.end);
        let Some(local) = frozen.get(&key) else {
            return false;
        };
        delta.locals.push((key, rebase_local(local, index)));
        true
    };
    for Definition {
        name,
        ty,
        target,
        site,
    } in &resolved.definitions
    {
        let target = match target {
            Target::Local { site } => {
                if !keep(*site, &mut delta) {
                    continue;
                }
                target.clone()
            }
            // A binding or an inherent member: its frozen scheme.
            Target::Value {
                origin: Origin::This,
                name,
            } => {
                if keep(*site, &mut delta) {
                    Target::Local { site: *site }
                } else {
                    Target::Value {
                        origin: Origin::Line(index),
                        name: name.clone(),
                    }
                }
            }
            other => rebase_target(other, index),
        };
        let list = if *ty {
            &mut delta.types
        } else {
            &mut delta.values
        };
        list.push((name.clone(), target));
    }
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Impl {
            target, members, ..
        } = &item.kind
        else {
            continue;
        };
        let owner = match resolved.ty(*target) {
            Some(Target::Type {
                origin: Origin::This,
                name,
            }) => (index, name.clone()),
            Some(Target::Type {
                origin: Origin::Line(line),
                name,
            }) => (*line, name.clone()),
            _ => continue,
        };
        delta.impls.push(owner.clone());
        let mut constructor = None;
        for member in members {
            match member {
                ImplMember::Named { name, .. } => {
                    if keep(name.span, &mut delta) {
                        let member = name.segments.last().cloned();
                        let key = (owner.clone(), member.unwrap_or_default());
                        delta
                            .members
                            .push((key, (name.span.start, name.span.end)));
                    }
                }
                ImplMember::Constructor { arm, .. }
                    if constructor.is_none() =>
                {
                    constructor = Some(arm.span);
                }
                ImplMember::Constructor { .. } => {}
            }
        }
        if let Some(site) = constructor {
            if keep(site, &mut delta) {
                let site = (site.start, site.end);
                delta.constructors.push((owner, site));
            }
        }
    }
    delta
}

fn rebase_weak(weak: &Subst, index: usize) -> Subst {
    let bindings = weak.bindings().iter();
    Subst::from_bindings(
        bindings
            .map(|ty| ty.as_ref().map(|ty| ty.rehome(&Origin::Line(index))))
            .collect(),
    )
}

fn rebase_local(local: &Local, index: usize) -> Local {
    Local {
        scheme: local.scheme.rehome(&Origin::Line(index)),
        heads: local.heads.clone(),
    }
}

fn rebase_target(target: &Target, index: usize) -> Target {
    let line = || Origin::Line(index);
    match target {
        Target::Type {
            origin: Origin::This,
            name,
        } => Target::Type {
            origin: line(),
            name: name.clone(),
        },
        Target::Constructor {
            origin: Origin::This,
            ty,
            name,
        } => Target::Constructor {
            origin: line(),
            ty: ty.clone(),
            name: name.clone(),
        },
        Target::Value {
            origin: Origin::This,
            name,
        } => Target::Value {
            origin: line(),
            name: name.clone(),
        },
        other => other.clone(),
    }
}
