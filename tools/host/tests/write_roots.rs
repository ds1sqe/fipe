//! Real scripts run by the std host in fresh temporary directories:
//! acquired write-root effects and their public error channels.
use fipe_host::{load, Host, Policy};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

/// A fresh directory under the system's temporary directory, removed
/// when dropped.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fipe-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(fs::canonicalize(dir).unwrap())
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn file(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What a run wrote and how it exited.
struct Ran {
    success: String,
    error: String,
    exit: u8,
}

/// Run `script` written to `cwd/script.fipe`, `cwd` the working
/// directory, with further write roots, arguments and input.
fn run_with(
    cwd: &Path,
    script: &str,
    allow: &[PathBuf],
    args: &[&str],
    input: &[u8],
) -> Ran {
    let path = cwd.join("script.fipe");
    fs::write(&path, script).unwrap();
    let script = load(&path).unwrap_or_else(|error| panic!("{error}"));
    let policy = Policy::new(cwd, allow).unwrap();
    let mut host = Host::new(policy)
        .with_args(args.iter().map(|arg| String::from(*arg)).collect());
    let (observation, exit) = host.run(&script, &mut &input[..], None);
    Ran {
        success: String::from_utf8(observation.success).unwrap(),
        error: String::from_utf8(observation.error).unwrap(),
        exit,
    }
}

fn run(cwd: &Path, script: &str) -> Ran {
    run_with(cwd, script, &[], &[], b"")
}

#[test]
fn put_and_append_write_inside_the_roots() {
    let temp = Temp::new();
    fs::create_dir(temp.path("work")).unwrap();
    fs::create_dir(temp.path("allowed")).unwrap();
    let allowed = temp.path("allowed/log");
    let ran = run_with(
        &temp.path("work"),
        &format!(
            "put ./out.txt \"ab\"\nappend ./out.txt \"cd\"\ncat ./out.txt\n\
             put {0} \"one\"\nappend {0} \"two\"\nput out \"done\\n\"\n\
             rm ./out.txt\n",
            allowed.display()
        ),
        &[temp.path("allowed")],
        &[],
        b"",
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.exit, 0);
    assert_eq!(ran.success, "abcd\ndone\n");
    assert!(!temp.path("work/out.txt").exists());
    assert_eq!(fs::read(&allowed).unwrap(), b"onetwo");
    // Without the allowance the same write is refused.
    let ran = run(
        &temp.path("work"),
        &format!("put {} \"x\"", allowed.display()),
    );
    assert_eq!(ran.exit, 1);
    assert_eq!(fs::read(&allowed).unwrap(), b"onetwo");
}

#[test]
fn a_with_cwd_moves_where_paths_resolve() {
    let temp = Temp::new();
    temp.file("sub/a", "hello");
    let ran = run(&temp.0, "with cwd ./sub { put ./made \"a\"\nappend ./made \"b\"\nput ./gone \"x\"\nrm ./gone }\n");
    assert_eq!(ran.error, "");
    assert_eq!(ran.exit, 0);
    assert_eq!(fs::read(temp.path("sub/made")).unwrap(), b"ab");
    assert!(!temp.path("sub/gone").exists());
    assert!(!temp.path("made").exists());
    assert!(!temp.path("gone").exists());
}

#[test]
fn an_absolute_path_outside_is_refused() {
    let temp = Temp::new();
    let victim = temp.file("elsewhere/victim", "keep");
    fs::create_dir(temp.path("work")).unwrap();
    let work = temp.path("work");
    let ran = run(&work, &format!("rm {}", victim.display()));
    assert_eq!(ran.exit, 1);
    assert!(
        ran.error.contains("outside the write roots"),
        "{}",
        ran.error
    );
    let ran = run(&work, &format!("put {} \"x\"", victim.display()));
    assert_eq!(ran.exit, 1);
    assert!(
        ran.error.contains("outside the write roots"),
        "{}",
        ran.error
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    // The working directory itself is never removed.
    let ran = run(&work, &format!("rm {}", work.display()));
    assert_eq!(ran.exit, 1);
    assert!(work.exists());
}

#[test]
fn public_cross_grant_write_and_link_removal_preserve_target() {
    let temp = Temp::new();
    fs::create_dir(temp.path("work")).unwrap();
    fs::create_dir(temp.path("allowed")).unwrap();
    temp.file("allowed/file", "keep");
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path("allowed"), temp.path("work/link"))
        .unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(
        temp.path("allowed"),
        temp.path("work/link"),
    )
    .expect("real directory-link privilege required");
    let ran = run_with(
        &temp.path("work"),
        "put ./link/file \"new\"\nappend ./link/file \"!\"\nrm ./link\n",
        &[temp.path("allowed")],
        &[],
        b"",
    );
    assert_eq!(ran.exit, 0, "{}", ran.error);
    assert_eq!(ran.error, "");
    assert_eq!(fs::read(temp.path("allowed/file")).unwrap(), b"new!");
    assert!(!temp.path("work/link").exists());
    assert!(temp.path("allowed").is_dir());
}

#[cfg(windows)]
fn powershell_spelling(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
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
fn windows_public_junction_case_effects() {
    let temp = Temp::new();
    fs::create_dir(temp.path("work")).unwrap();
    fs::create_dir(temp.path("allowed")).unwrap();
    temp.file("allowed/sentinel", "keep");
    junction(&temp.path("allowed"), &temp.path("work/junction"));
    let ran = run_with(&temp.path("work"),
        "put ./JUNCTION/log \"a\"\nappend ./junction/log \"b\"\ncat ./junction/log\nrm ./junction\n",
        &[temp.path("allowed")], &[], b"");
    assert_eq!(ran.exit, 0, "{}", ran.error);
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "ab\n");
    assert_eq!(fs::read(temp.path("allowed/log")).unwrap(), b"ab");
    assert_eq!(fs::read(temp.path("allowed/sentinel")).unwrap(), b"keep");
    assert!(temp.path("allowed").is_dir());
    assert!(fs::symlink_metadata(temp.path("work/junction")).is_err());
}
