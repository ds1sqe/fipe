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
    types::Subst,
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

    /// Freeze a checked line as the next accepted one.
    pub fn accept(
        &mut self,
        module: Module,
        resolved: Resolved,
        typed: &Typed,
    ) {
        let index = self.lines.len();
        let mut delta = delta(index, &module, &resolved, typed);
        delta.weak = rebase_weak(&typed.weak, index);
        delta.facts.clone_from(&typed.facts);
        for (_, known) in &mut delta.facts {
            known.rehome(&Origin::Line(index));
        }
        self.add(index, &delta);
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
    }

    fn add(&mut self, index: usize, delta: &Delta) {
        self.values.extend(delta.values.iter().cloned());
        self.types.extend(delta.types.iter().cloned());
        self.locals.extend(delta.locals.iter().cloned());
        self.facts.extend(delta.facts.iter().cloned());
        for owner in &delta.impls {
            self.impls.entry(owner.clone()).or_default().push(index);
        }
        self.members.extend(delta.members.iter().cloned());
        self.constructors.extend(delta.constructors.iter().cloned());
        self.weak = delta.weak.clone();
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
