//! The write roots: where a write, an append or a removal may land.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// The working directory and the write roots, canonicalized once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    cwd: PathBuf,
    roots: Vec<PathBuf>,
}

impl Policy {
    /// The policy of a working directory and the further directories a
    /// script may write under; the working directory is a write root.
    ///
    /// # Errors
    /// When a directory does not exist or cannot be resolved.
    pub fn new(cwd: &Path, allow_write: &[PathBuf]) -> io::Result<Self> {
        let cwd = fs::canonicalize(cwd)?;
        let mut roots = vec![cwd.clone()];
        for dir in allow_write {
            roots.push(fs::canonicalize(dir)?);
        }
        Ok(Self { cwd, roots })
    }

    /// The canonical working directory paths resolve against.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Where a destination lands, when it is inside a write root: its
    /// parent resolved, links and `..` included, then its own name.
    /// `follow` also resolves an existing link at the destination, as a
    /// write through it would; a removal does not follow it. `written`
    /// is resolved against `base` and names the destination in a
    /// refusal.
    pub(crate) fn writable(
        &self,
        base: &Path,
        written: &str,
        follow: bool,
    ) -> Result<PathBuf, String> {
        let path = base.join(written);
        let outside = || format!("{written} is outside the write roots");
        let Some(Component::Normal(name)) = path.components().next_back()
        else {
            return Err(outside());
        };
        let parent = path.parent().ok_or_else(outside)?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| format!("cannot resolve {written}: {error}"))?;
        let target = parent.join(name);
        if !self.inside(&target) {
            return Err(outside());
        }
        // ponytail: checked then used, so a link swapped in between is not
        // caught; opening relative to a held directory handle needs unsafe
        // or a crate.
        if follow && fs::symlink_metadata(&target).is_ok() {
            match fs::canonicalize(&target) {
                Ok(resolved) if self.inside(&resolved) => {}
                _ => return Err(outside()),
            }
        }
        Ok(target)
    }

    /// Whether a resolved path is strictly below a write root.
    fn inside(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| path != root && path.starts_with(root))
    }
}
