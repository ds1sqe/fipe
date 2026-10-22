//! Real scripts run by the std host in fresh temporary directories:
//! the listing, reading and writing it performs, the write roots it
//! keeps, its input and its exit codes.
use fipe_host::{load, Host, LoadError, Policy};
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
fn a_listing_filtered_and_counted_over_a_temp_tree() {
    let temp = Temp::new();
    temp.file("tree/big.txt", &"x".repeat(100));
    temp.file("tree/small.txt", "x");
    temp.file("tree/huge.log", &"x".repeat(300));
    temp.file("tree/sub/deep.txt", &"x".repeat(500));
    let ran = run(
        &temp.0,
        "ls ./tree | filter (e => e.size > 50) | len\n\
         ls -r -depth 2 ./tree -name \"*.txt\" | map .path\n",
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.exit, 0);
    // The directory `sub` is an entry too, sized by the file system, so
    // the count is of files over 50 bytes, plus `sub` when it is.
    let sub = fs::metadata(temp.path("tree/sub")).unwrap().len();
    let expected = 2 + usize::from(sub > 50);
    assert_eq!(
        ran.success,
        format!(
            "{expected}\n[\"./tree/big.txt\", \"./tree/small.txt\", \
             \"./tree/sub/deep.txt\"]\n"
        )
    );
}

#[test]
fn a_file_is_found_read_and_grepped() {
    let temp = Temp::new();
    temp.file("notes.md", "buy milk\ncall home\nbuy bread\n");
    let ran = run(
        &temp.0,
        "cat ./notes.md | lines | len\n\
         cat ./notes.md | grep \"buy\"\n\
         find ./absent |: (e => e.name) || \"none\"\n\
         [:e] = find ./notes.md\n\
         e.name\n",
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "3\nbuy milk\nbuy bread\n\nnone\nnotes.md\n");
    let ran = run(&temp.0, "cat ./absent");
    assert_eq!(ran.exit, 1);
    assert_eq!(ran.error, "fault in main at 1:5: host: no entry ./absent\n");
}

#[test]
fn removing_a_parent_entry_is_refused_and_it_survives() {
    let temp = Temp::new();
    let x = temp.file("x", "keep");
    fs::create_dir(temp.path("work")).unwrap();
    let ran = run(&temp.path("work"), "rm ../x");
    assert_eq!(ran.exit, 1, "{}", ran.error);
    assert_eq!(ran.error, "../x is outside the write roots\n");
    assert_eq!(fs::read_to_string(x).unwrap(), "keep");
}

#[test]
fn a_link_leaving_the_write_roots_is_refused() {
    let temp = Temp::new();
    let outside = temp.file("outside/target", "keep");
    fs::create_dir(temp.path("work")).unwrap();
    std::os::unix::fs::symlink(temp.path("outside"), temp.path("work/dir"))
        .unwrap();
    std::os::unix::fs::symlink(&outside, temp.path("work/file")).unwrap();
    let work = temp.path("work");
    // Through a linked directory: rm is an `Err`, put a Fault.
    let ran = run(&work, "rm ./dir/target");
    assert_eq!(ran.exit, 1);
    assert!(
        ran.error.contains("outside the write roots"),
        "{}",
        ran.error
    );
    let ran = run(&work, "put ./dir/new \"x\"");
    assert_eq!(ran.exit, 1);
    assert!(
        ran.error.starts_with("fault in main at 1:1:"),
        "{}",
        ran.error
    );
    assert!(!temp.path("outside/new").exists());
    // Through a linked file: the write would land outside.
    let ran = run(&work, "append ./file \"x\"");
    assert_eq!(ran.exit, 1);
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    // Removing the link removes the link, never its target.
    let ran = run(&work, "rm ./file");
    assert_eq!(ran.exit, 0, "{}", ran.error);
    assert!(!temp.path("work/file").exists());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
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
    let ran = run(&temp.0, "with cwd ./sub { cat ./a }\nput ./sub/b \"x\"\n");
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "hello\n");
    assert!(temp.path("sub/b").exists());
}

#[test]
fn arguments_and_the_environment_are_the_processs() {
    let temp = Temp::new();
    let ran = run_with(
        &temp.0,
        "use cli\ncli.args\ncli.args | len\ncli.env \"FIPE_HOST_UNSET_VARIABLE\" || \"unset\"\n\
         cli.env \"PATH\" |: (p => (len p) > 0) || False\n",
        &[],
        &["a b", "c"],
        b"",
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "[\"a b\", \"c\"]\n2\nunset\nTrue\n");
}

#[test]
fn resources_are_looked_up_and_listed() {
    let temp = Temp::new();
    temp.file("data/one", "1");
    temp.file("data/two", "22");
    let ran = run(
        &temp.0,
        &format!(
            "use fs\ndir = sys.lookup root {} | Rd<fs>\n\
             fs.list dir ./data | map .name\nfs.list dir ./data | map fs.size\n",
            temp.0.display()
        ),
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "[\"one\", \"two\"]\n[1, 2]\n");
}

#[test]
fn text_is_wrapped_and_trimmed() {
    let temp = Temp::new();
    let ran = run(
        &temp.0,
        "use text\ntext.wrap -width 5 \"aa bb cc\"\ntext.trim \"  x  \"\n",
    );
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "aa bb\ncc\nx\n");
}

#[test]
fn standard_input_enters_a_string_or_bytes_spot() {
    let temp = Temp::new();
    let script =
        "count_lines: String -> Integer = s => s | lines | len\ncount_lines\n";
    let ran = run_with(&temp.0, script, &[], &[], b"a\nb\nc\n");
    assert_eq!((ran.success.as_str(), ran.exit), ("3\n", 0));
    let script = "size: Vec<U8> -> Integer = b => len b\nsize\n";
    let ran = run_with(&temp.0, script, &[], &[], b"\xff\x00");
    assert_eq!((ran.success.as_str(), ran.exit), ("2\n", 0));
    // Text that is not UTF-8, and a spot of another type, are refused
    // before any effect.
    let script = "put ./touched \"x\"\ncount_lines: String -> Integer = s => s | lines | len\ncount_lines\n";
    let ran = run_with(&temp.0, script, &[], &[], b"\xff");
    assert_eq!(ran.exit, 2);
    assert!(
        ran.error.contains("the input is not UTF-8 text"),
        "{}",
        ran.error
    );
    assert!(!temp.path("touched").exists());
    let script = "twice: Integer -> Integer = n => n * 2\ntwice\n";
    let ran = run_with(&temp.0, script, &[], &[], b"4");
    assert_eq!(ran.exit, 2);
    assert!(ran.error.contains("String or Vec<U8>"), "{}", ran.error);
}

#[test]
fn the_exit_code_follows_the_runs_end() {
    let temp = Temp::new();
    let ran = run(&temp.0, "x: Result<Integer, String> = Ok 1\nx\n");
    assert_eq!((ran.success.as_str(), ran.exit), ("1\n", 0));
    let ran = run(&temp.0, "x: Result<Integer, String> = Err \"bad\"\nx\n");
    assert_eq!((ran.error.as_str(), ran.exit), ("bad\n", 1));
    let ran = run(&temp.0, "a = 1\nfault \"stop\"\n");
    assert_eq!(ran.exit, 1);
    assert_eq!(ran.error, "fault in main at 2:1: stop\n");
}

#[test]
fn a_script_loads_the_modules_it_uses_beside_it() {
    let temp = Temp::new();
    temp.file(
        "shapes.fipe",
        "## Twice a number.\npub twice: Integer -> Integer = n => n * 2\n",
    );
    let ran = run(&temp.0, "use shapes.twice\ntwice 21\n");
    assert_eq!(ran.error, "");
    assert_eq!(ran.success, "42\n");
}

#[test]
fn a_script_that_does_not_check_is_located() {
    let temp = Temp::new();
    let path = temp.file("bad.fipe", "a = 1\nb = nope + 1\n");
    let Err(LoadError::Check(diagnostics)) = load(&path) else {
        panic!("the script checks")
    };
    assert_eq!(diagnostics[0].path, path);
    assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 5));
    let shown = diagnostics[0].to_string();
    assert!(shown.ends_with("bad.fipe:2:5: unknown name"), "{shown}");
    assert!(matches!(
        load(&temp.path("absent.fipe")),
        Err(LoadError::Read { .. })
    ));
}
