//! File checking is shared by every use in one compilation unit.
use super::{Error, Infer};
use crate::{
    ast::Module,
    decl::Declarations,
    resolve::{self, Resolved},
    typed::Typed,
};
use alloc::{collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;

#[derive(Default)]
pub struct FileCache {
    pub expressions: RefCell<
        BTreeMap<
            (crate::resolve::Origin, usize),
            crate::typed::DeclaredExpression,
        >,
    >,
    pub files: RefCell<BTreeMap<String, CheckedFile>>,
    pub values:
        RefCell<BTreeMap<(crate::resolve::Origin, String), DeclaredValue>>,
    /// The declared expressions this unit read or recorded: what its
    /// typing lists and its lowering emits.
    used:
        RefCell<alloc::collections::BTreeSet<(crate::resolve::Origin, usize)>>,
}

impl core::fmt::Debug for FileCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileCache")
            .field("expressions", &self.expressions.borrow().len())
            .field("files", &self.files.borrow().len())
            .field("values", &self.values.borrow().len())
            .field("used", &self.used.borrow().len())
            .finish()
    }
}

impl FileCache {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            expressions: RefCell::new(BTreeMap::new()),
            files: RefCell::new(BTreeMap::new()),
            values: RefCell::new(BTreeMap::new()),
            used: RefCell::new(alloc::collections::BTreeSet::new()),
        }
    }

    /// A declared expression recorded before, in this unit or an
    /// earlier one of the session, now used by this unit.
    pub fn expression(
        &self,
        key: &(crate::resolve::Origin, usize),
    ) -> Option<crate::typed::DeclaredExpression> {
        let expression = self.expressions.borrow().get(key).cloned()?;
        self.used.borrow_mut().insert(key.clone());
        Some(expression)
    }

    /// Record a declared expression this unit typed.
    pub fn record(
        &self,
        key: (crate::resolve::Origin, usize),
        expression: crate::typed::DeclaredExpression,
    ) {
        self.used.borrow_mut().insert(key.clone());
        self.expressions.borrow_mut().insert(key, expression);
    }

    /// The declared expressions this unit used, in key order.
    #[must_use]
    pub fn used(&self) -> Vec<crate::typed::DeclaredExpression> {
        let expressions = self.expressions.borrow();
        self.used
            .borrow()
            .iter()
            .filter_map(|key| expressions.get(key).cloned())
            .collect()
    }

    /// Forget what one unit owns: its files, checked afresh per unit,
    /// its own declared expressions and the values still being checked.
    /// What the other modules declared stays, typed once per session.
    pub fn end_unit(&self) {
        let keep = |origin: &crate::resolve::Origin| {
            *origin != crate::resolve::Origin::This
        };
        self.files.borrow_mut().clear();
        self.used.borrow_mut().clear();
        self.expressions
            .borrow_mut()
            .retain(|(origin, _), _| keep(origin));
        self.values.borrow_mut().retain(|(origin, _), value| {
            keep(origin) && !matches!(value, DeclaredValue::Checking)
        });
    }

    /// A copy of what `end_unit` keeps.
    #[must_use]
    pub fn inherit(&self) -> Self {
        let keep = |origin: &crate::resolve::Origin| {
            *origin != crate::resolve::Origin::This
        };
        Self {
            expressions: RefCell::new(
                self.expressions
                    .borrow()
                    .iter()
                    .filter(|((origin, _), _)| keep(origin))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            files: RefCell::default(),
            used: RefCell::default(),
            values: RefCell::new(
                self.values
                    .borrow()
                    .iter()
                    .filter(|((origin, _), value)| {
                        keep(origin)
                            && !matches!(value, DeclaredValue::Checking)
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
        }
    }
}

#[derive(Clone)]
pub enum DeclaredValue {
    Checking,
    Failed,
    Typed(crate::types::Scheme),
}

#[derive(Clone)]
pub struct CheckedFile {
    pub resolved: Rc<Resolved>,
    pub typed: Rc<Typed>,
    pub resolve_errors: Vec<resolve::Error>,
    pub type_errors: Vec<Error>,
}

pub fn check_file(
    stem: &str,
    declarations: &Declarations,
    files: &[(String, Module)],
    cache: &FileCache,
    visiting: &[String],
) -> Option<CheckedFile> {
    if let Some(checked) = cache.files.borrow().get(stem) {
        return Some(checked.clone());
    }
    let (_, module) = files.iter().find(|(name, _)| name == stem)?;
    let (resolved, resolve_errors) =
        resolve::resolve_used(module, declarations, files, stem);
    let mut visiting = visiting.to_vec();
    visiting.push(String::from(stem));
    let (typed, type_errors) = super::run(
        module,
        &resolved,
        declarations,
        files,
        None,
        (&visiting, cache),
        usize::MAX,
    );
    let checked = CheckedFile {
        resolved: Rc::new(resolved),
        typed: Rc::new(typed),
        resolve_errors,
        type_errors,
    };
    cache
        .files
        .borrow_mut()
        .insert(String::from(stem), checked.clone());
    Some(checked)
}

impl Infer<'_> {
    pub(super) fn cached_file(&self, stem: &str) -> Option<CheckedFile> {
        self.file_cache.files.borrow().get(stem).cloned()
    }

    /// A file checked in this unit, or checked now; none inside its
    /// own import cycle.
    pub(super) fn checked_file(&self, stem: &str) -> Option<CheckedFile> {
        if self.visiting.iter().any(|s| s == stem) {
            return None;
        }
        check_file(
            stem,
            self.declarations,
            self.files,
            self.file_cache,
            &self.visiting,
        )
    }

    /// A checked file's top-level bindings as locals, by site, for an
    /// expression of the file typed on its own.
    pub(super) fn file_locals(
        &self,
        stem: &str,
    ) -> Vec<((usize, usize), super::Local)> {
        let Some((checked, (_, module))) = self
            .checked_file(stem)
            .zip(self.files.iter().find(|(name, _)| name == stem))
        else {
            return Vec::new();
        };
        module
            .entries
            .iter()
            .filter_map(|entry| {
                let crate::ast::Entry::Line(id) = entry else {
                    return None;
                };
                let crate::ast::ExprKind::Bind { pattern, value, .. } =
                    &module.expressions[id.0].kind
                else {
                    return None;
                };
                let crate::ast::PatternKind::Name(name) =
                    &module.patterns[pattern.0].kind
                else {
                    return None;
                };
                let scheme = checked.typed.binding(*id)?.clone();
                let heads = match &module.expressions[value.0].kind {
                    crate::ast::ExprKind::Arms(arms) => {
                        Some(super::arms::heads_in(module, arms))
                    }
                    _ => None,
                };
                Some((
                    (name.span.start, name.span.end),
                    super::Local { scheme, heads },
                ))
            })
            .collect()
    }
}
