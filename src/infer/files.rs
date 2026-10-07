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
    pub files: RefCell<BTreeMap<String, CheckedFile>>,
    pub values:
        RefCell<BTreeMap<(crate::resolve::Origin, String), DeclaredValue>>,
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
}
