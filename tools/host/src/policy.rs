//! Acquired directory grants for writes, appends and removals.
use cap_fs_ext::DirExt as _;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    fmt, fs,
    io::{self, Write as _},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// The working directory and acquired write roots.
#[derive(Clone)]
pub struct Policy {
    cwd: PathBuf,
    roots: Vec<Grant>,
}
#[derive(Clone)]
struct Grant {
    label: PathBuf,
    dir: Arc<Dir>,
}
impl fmt::Debug for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Policy")
            .field("cwd", &self.cwd)
            .field(
                "roots",
                &self
                    .roots
                    .iter()
                    .map(|root| &root.label)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}
impl PartialEq for Policy {
    fn eq(&self, other: &Self) -> bool {
        self.cwd == other.cwd
            && self
                .roots
                .iter()
                .map(|root| &root.label)
                .eq(other.roots.iter().map(|root| &root.label))
    }
}
impl Eq for Policy {}

pub struct Destination<'a> {
    dir: &'a Dir,
    suffix: PathBuf,
}
impl Destination<'_> {
    fn open(&self, replace: bool) -> io::Result<cap_std::fs::File> {
        let mut options = OpenOptions::new();
        options
            .create(true)
            .write(true)
            .truncate(replace)
            .append(!replace);
        self.dir.open_with(&self.suffix, &options)
    }
    pub(crate) fn write(&self, bytes: &[u8], replace: bool) -> io::Result<()> {
        self.open(replace)?.write_all(bytes)
    }
    pub(crate) fn remove(&self) -> io::Result<()> {
        if self.dir.symlink_metadata(&self.suffix)?.is_dir() {
            self.dir.remove_dir(&self.suffix)
        } else {
            self.dir.remove_file_or_symlink(&self.suffix)
        }
    }
}
impl Policy {
    /// Acquire the working directory and further write roots once.
    ///
    /// # Errors
    /// When a directory does not exist, cannot be resolved or opened.
    pub fn new(cwd: &Path, allow_write: &[PathBuf]) -> io::Result<Self> {
        let cwd = fs::canonicalize(cwd)?;
        let mut roots = Vec::with_capacity(allow_write.len() + 1);
        for path in std::iter::once(cwd.as_path())
            .chain(allow_write.iter().map(PathBuf::as_path))
        {
            let label = fs::canonicalize(path)?;
            let dir =
                Arc::new(Dir::open_ambient_dir(&label, ambient_authority())?);
            roots.push(Grant { label, dir });
        }
        Ok(Self { cwd, roots })
    }
    /// The canonical working directory paths resolve against.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    // Canonicalization prepares an address; only the acquired Dir authorizes effects.
    pub(crate) fn writable(
        &self,
        base: &Path,
        written: &str,
        follow: bool,
    ) -> Result<Destination<'_>, String> {
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
        // Preserve the existing rule: both the written entry and followed target must be granted.
        if !self.inside(&target) {
            return Err(outside());
        }
        let target = if follow && fs::symlink_metadata(&target).is_ok() {
            fs::canonicalize(&target).map_err(|_| outside())?
        } else {
            target
        };
        for root in &self.roots {
            if let Ok(suffix) = target.strip_prefix(&root.label) {
                if !suffix.as_os_str().is_empty() {
                    return Ok(Destination {
                        dir: &root.dir,
                        suffix: suffix.into(),
                    });
                }
            }
        }
        Err(outside())
    }
    fn inside(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| path != root.label && path.starts_with(&root.label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "fipe-policy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            for name in ["a", "b", "outside"] {
                fs::create_dir(path.join(name)).unwrap();
            }
            Self(fs::canonicalize(path).unwrap())
        }
        fn at(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
        fn policy(&self) -> Policy {
            Policy::new(&self.at("a"), &[self.at("b")]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[cfg(unix)]
    fn link(target: &Path, path: &Path) {
        std::os::unix::fs::symlink(target, path).unwrap();
    }
    #[cfg(windows)]
    fn link(target: &Path, path: &Path) {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, path).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, path).unwrap();
        }
    }
    #[test]
    fn effects_and_removal_boundaries() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Policy>();
        let f = Fixture::new();
        let p = f.policy();
        let clone = p.clone();
        assert_eq!(p, clone);
        let d = p.writable(p.cwd(), "file", true).unwrap();
        d.write(b"a", true).unwrap();
        d.write(b"b", false).unwrap();
        assert_eq!(fs::read(f.at("a/file")).unwrap(), b"ab");
        clone
            .writable(p.cwd(), "file", false)
            .unwrap()
            .remove()
            .unwrap();
        fs::create_dir(f.at("a/empty")).unwrap();
        p.writable(p.cwd(), "empty", false)
            .unwrap()
            .remove()
            .unwrap();
        fs::create_dir(f.at("a/nonempty")).unwrap();
        fs::write(f.at("a/nonempty/x"), b"keep").unwrap();
        assert!(p
            .writable(p.cwd(), "nonempty", false)
            .unwrap()
            .remove()
            .is_err());
        assert!(p.writable(p.cwd(), ".", false).is_err());
        assert!(p.writable(p.cwd(), "missing/x", true).is_err());
    }
    #[test]
    fn stable_links_use_union_and_unlink_only_entry() {
        let f = Fixture::new();
        let p = f.policy();
        fs::write(f.at("b/file"), b"b").unwrap();
        link(&f.at("b"), &f.at("a/cross"));
        p.writable(p.cwd(), "cross/file", true)
            .unwrap()
            .write(b"new", true)
            .unwrap();
        assert_eq!(fs::read(f.at("b/file")).unwrap(), b"new");
        link(&f.at("outside"), &f.at("a/escape"));
        assert!(p.writable(p.cwd(), "escape/new", true).is_err());
        p.writable(p.cwd(), "escape", false)
            .unwrap()
            .remove()
            .unwrap();
        assert!(f.at("outside").is_dir());
        let overlap =
            Policy::new(&f.at("a"), &[f.0.clone(), f.at("a")]).unwrap();
        overlap
            .writable(overlap.cwd(), "../b/file", true)
            .unwrap()
            .write(b"overlap", true)
            .unwrap();
        assert_eq!(fs::read(f.at("b/file")).unwrap(), b"overlap");
    }
    #[test]
    fn prepared_parent_swap_cannot_touch_outside() {
        let f = Fixture::new();
        let p = f.policy();
        fs::create_dir(f.at("a/parent")).unwrap();
        fs::write(f.at("outside/file"), b"keep").unwrap();
        let destination = p.writable(p.cwd(), "parent/file", true).unwrap();
        let removal = p.writable(p.cwd(), "parent/file", false).unwrap();
        fs::rename(f.at("a/parent"), f.at("a/original")).unwrap();
        link(&f.at("outside"), &f.at("a/parent"));
        assert!(destination.write(b"bad", true).is_err());
        assert!(destination.write(b"bad", false).is_err());
        assert!(removal.remove().is_err());
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"keep");
        // The old ambient effect really would follow the replacement.
        fs::write(f.at("a/parent/file"), b"ambient").unwrap();
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"ambient");
    }
    #[test]
    fn prepared_final_swap_cannot_append_or_remove_target() {
        let f = Fixture::new();
        let p = f.policy();
        fs::write(f.at("a/file"), b"inside").unwrap();
        fs::write(f.at("outside/file"), b"keep").unwrap();
        let destination = p.writable(p.cwd(), "file", true).unwrap();
        let removal = p.writable(p.cwd(), "file", false).unwrap();
        fs::remove_file(f.at("a/file")).unwrap();
        link(&f.at("outside/file"), &f.at("a/file"));
        assert!(destination.write(b"bad", false).is_err());
        assert!(destination.write(b"bad", true).is_err());
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"keep");
        removal.remove().unwrap();
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"keep");
    }
    #[cfg(unix)]
    #[test]
    fn root_replacement_and_clone_keep_original_authority() {
        let f = Fixture::new();
        let p = f.policy();
        let clone = p.clone();
        let destination = p.writable(p.cwd(), "file", true).unwrap();
        fs::rename(f.at("a"), f.at("old-a")).unwrap();
        fs::create_dir(f.at("a")).unwrap();
        destination.write(b"old", true).unwrap();
        clone
            .writable(clone.cwd(), "second", true)
            .unwrap()
            .write(b"old", true)
            .unwrap();
        assert_eq!(fs::read(f.at("old-a/file")).unwrap(), b"old");
        assert!(!f.at("a/file").exists());
        assert!(!f.at("a/second").exists());
    }
    #[cfg(windows)]
    #[test]
    fn windows_root_lock_lasts_until_final_clone_drop() {
        let f = Fixture::new();
        let p = f.policy();
        let clone = p.clone();
        assert_eq!(
            fs::rename(f.at("a"), f.at("old-a"))
                .unwrap_err()
                .raw_os_error(),
            Some(32)
        );
        drop(p);
        assert_eq!(
            fs::remove_dir(f.at("a")).unwrap_err().raw_os_error(),
            Some(32)
        );
        drop(clone);
        fs::rename(f.at("a"), f.at("old-a")).unwrap();
        fs::remove_dir(f.at("old-a")).unwrap();
    }
    #[test]
    fn hard_links_are_not_inode_isolation() {
        let f = Fixture::new();
        let p = f.policy();
        fs::write(f.at("outside/file"), b"old").unwrap();
        fs::hard_link(f.at("outside/file"), f.at("a/alias")).unwrap();
        p.writable(p.cwd(), "alias", true)
            .unwrap()
            .write(b"shared", true)
            .unwrap();
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"shared");
    }

    #[cfg(windows)]
    fn powershell_spelling(path: &Path) -> PathBuf {
        use std::path::Prefix;
        assert!(
            matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::VerbatimDisk(_)))
        );
        let text = path.to_str().unwrap();
        let ordinary = PathBuf::from(text.strip_prefix(r"\\?\").unwrap());
        assert!(
            ordinary
                .as_os_str()
                .to_str()
                .unwrap()
                .encode_utf16()
                .count()
                < 260
        );
        assert!(
            matches!(ordinary.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_)))
        );
        if path.exists() {
            assert_eq!(
                fs::canonicalize(&ordinary).unwrap(),
                fs::canonicalize(path).unwrap()
            );
        } else {
            assert_eq!(ordinary.file_name(), path.file_name());
            assert_eq!(
                fs::canonicalize(ordinary.parent().unwrap()).unwrap(),
                fs::canonicalize(path.parent().unwrap()).unwrap()
            );
        }
        ordinary
    }

    #[cfg(windows)]
    fn junction(target: &Path, path: &Path) {
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "New-Item -ItemType Junction -Path $env:FIPE_TEST_LINK -Target $env:FIPE_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("FIPE_TEST_LINK", powershell_spelling(path))
            .env("FIPE_TEST_TARGET", powershell_spelling(target))
            .status().unwrap();
        assert!(status.success(), "actual junction creation required");
        eprintln!(
            "JUNCTION_KIND={:?}",
            fs::symlink_metadata(path).unwrap().file_type()
        );
        assert_eq!(
            fs::canonicalize(path).unwrap(),
            fs::canonicalize(target).unwrap()
        );
        assert!(fs::read_link(path).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_names_refuse_before_payload() {
        let f = Fixture::new();
        let p = f.policy();
        p.writable(p.cwd(), "healthy", true)
            .unwrap()
            .write(b"before", true)
            .unwrap();
        p.writable(p.cwd(), "healthy", false)
            .unwrap()
            .remove()
            .unwrap();
        for name in ["CON", "NUL", "COM1", "LPT1", "COM¹", "COM²", "LPT¹"] {
            if let Ok(destination) = p.writable(p.cwd(), name, true) {
                for replace in [true, false] {
                    assert!(
                        destination.open(replace).is_err(),
                        "device {name} must refuse at open, before any payload"
                    );
                }
            }
        }
        assert_eq!(fs::read_dir(f.at("a")).unwrap().count(), 0);
        p.writable(p.cwd(), "healthy", true)
            .unwrap()
            .write(b"after", true)
            .unwrap();
        assert_eq!(fs::read(f.at("a/healthy")).unwrap(), b"after");
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_and_drive_preserve_grants() {
        let f = Fixture::new();
        let p = f.policy();
        let original = f.at("a/mixed-case");
        p.writable(p.cwd(), original.to_str().unwrap(), true)
            .unwrap()
            .write(b"absolute drive", true)
            .unwrap();
        let upper = PathBuf::from(original.to_str().unwrap().to_uppercase());
        p.writable(p.cwd(), upper.to_str().unwrap(), true)
            .unwrap()
            .write(b"same file", true)
            .unwrap();
        assert_eq!(fs::read(original).unwrap(), b"same file");
        // Lexical policy check only: no access to another drive or SMB server.
        for outside in [
            Path::new(r"Z:\ungranted\file"),
            Path::new(r"\\ungranted.invalid\share\file"),
            Path::new(r"\\?\GLOBALROOT\Device\ungranted"),
        ] {
            assert!(!p.inside(outside));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_effects_and_prepared_replacement() {
        let f = Fixture::new();
        let p = f.policy();
        fs::write(f.at("b/sentinel"), b"keep target").unwrap();
        junction(&f.at("b"), &f.at("a/junction"));
        p.writable(p.cwd(), "junction/healthy", true)
            .unwrap()
            .write(b"a", true)
            .unwrap();
        p.writable(p.cwd(), "junction/healthy", true)
            .unwrap()
            .write(b"b", false)
            .unwrap();
        assert_eq!(fs::read(f.at("b/healthy")).unwrap(), b"ab");
        let unlink = p.writable(p.cwd(), "junction", false).unwrap();
        unlink.remove().unwrap();
        assert!(fs::symlink_metadata(f.at("a/junction")).is_err());
        assert_eq!(fs::read(f.at("b/sentinel")).unwrap(), b"keep target");

        fs::create_dir(f.at("a/swap-target")).unwrap();
        junction(&f.at("a/swap-target"), &f.at("a/swap-target/parent"));
        fs::write(f.at("outside/file"), b"outside").unwrap();
        let write = p
            .writable(p.cwd(), "swap-target/parent/file", true)
            .unwrap();
        let remove = p
            .writable(p.cwd(), "swap-target/parent/file", false)
            .unwrap();
        write.write(b"healthy", true).unwrap();
        write.write(b"!", false).unwrap();
        assert_eq!(fs::read(f.at("a/swap-target/file")).unwrap(), b"healthy!");
        fs::write(f.at("a/swap-target/removable"), b"owned").unwrap();
        p.writable(p.cwd(), "swap-target/parent/removable", false)
            .unwrap()
            .remove()
            .unwrap();
        assert!(!f.at("a/swap-target/removable").exists());
        fs::rename(f.at("a/swap-target"), f.at("a/original-swap-target"))
            .unwrap();
        junction(&f.at("outside"), &f.at("a/swap-target"));
        assert!(write.write(b"bad", true).is_err());
        assert!(write.write(b"bad", false).is_err());
        assert!(remove.remove().is_err());
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"outside");

        junction(&f.at("b"), &f.at("a/final"));
        let final_remove = p.writable(p.cwd(), "final", false).unwrap();
        final_remove.remove().unwrap();
        junction(&f.at("outside"), &f.at("a/final"));
        final_remove.remove().unwrap();
        assert!(fs::symlink_metadata(f.at("a/final")).is_err());
        assert_eq!(fs::read(f.at("outside/file")).unwrap(), b"outside");
        assert_eq!(fs::read(f.at("b/sentinel")).unwrap(), b"keep target");
    }
}
