//! The `fipe` command: one script gives equal bytes and exit code under
//! `run` and as a built executable, the REPL shows its values and
//! performs its effects, `check` exits by the worst file, and the write
//! roots hold in each runner.
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
};

/// A fresh directory `name` under the tests' target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `libfipe_native_rt.a`, built once per test binary in its own target
/// directory, in the test's own profile.
fn runtime() -> &'static Path {
    static ARCHIVE: OnceLock<PathBuf> = OnceLock::new();
    ARCHIVE.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("runtime");
        let release = !cfg!(debug_assertions);
        let status = Command::new(env!("CARGO"))
            .args(["build", "--locked", "--offline", "--manifest-path"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../native/runtime/Cargo.toml"),
            )
            .arg("--target-dir")
            .arg(&target)
            .args(release.then_some("--release"))
            .status()
            .unwrap();
        assert!(status.success(), "the runtime does not build");
        let profile = if release { "release" } else { "debug" };
        target.join(profile).join("libfipe_native_rt.a")
    })
}

/// `fipe ARGS...` in `dir`, with `input` on stdin.
fn fipe(dir: &Path, args: &[&str], input: &[u8]) -> Output {
    run_exe(dir, Path::new(env!("CARGO_BIN_EXE_fipe")), args, input, &[])
}

/// Run `exe` in `dir` with arguments, input and environment.
fn run_exe(
    dir: &Path,
    exe: &Path,
    args: &[&str],
    input: &[u8],
    env: &[(&str, &str)],
) -> Output {
    let mut child = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

/// The executable `fipe build` makes of `script` in `dir`.
fn built(dir: &Path, script: &str) -> PathBuf {
    built_with(dir, script, &[])
}

/// As `built`, with further build options and a name of their own.
fn built_with(dir: &Path, script: &str, more: &[&str]) -> PathBuf {
    let exe = dir.join(format!("{script}{}.exe", more.concat()));
    let runtime = runtime().to_str().unwrap();
    let mut args = vec![
        "build",
        script,
        "-o",
        exe.to_str().unwrap(),
        "--runtime",
        runtime,
    ];
    args.extend(more);
    let out = fipe(dir, &args, b"");
    assert!(out.status.success(), "{out:?}");
    exe
}

/// (stdout, stderr, exit code).
fn shown(output: &Output) -> (String, String, i32) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap(),
    )
}

#[test]
fn run_and_the_built_executable_give_equal_bytes_and_exit_codes() {
    let dir = scratch("agree");
    let scripts: &[(&str, &str, &[&str], &[u8])] = &[
        (
            "ok.fipe",
            "\"hello\"\n\"written\\n\" | put ./out.txt\n[1, 2] | len\n",
            &[],
            b"",
        ),
        ("err.fipe", "Err \"bad\"\n", &[], b""),
        ("fault.fipe", "(1).div 0\n", &[], b""),
        ("input.fipe", "s => s | lines | len\n", &[], b"a\nb\n"),
        (
            "args.fipe",
            "use cli\ncli.args | join \" \"\n",
            &["a", "b"],
            b"",
        ),
        // The last line takes no String or Vec<U8>: refused before any
        // effect, so the file is not written.
        (
            "refused.fipe",
            "\"x\" | put ./effect.txt\nn => n + 1\n",
            &[],
            b"1\n",
        ),
    ];
    let expected = [
        ("hello\n2\n", "", 0),
        ("", "bad\n", 1),
        ("", "fault in main at 1:1: division by zero\n", 1),
        ("2\n", "", 0),
        ("a b\n", "", 0),
        (
            "",
            "fault in main at 2:1: host: host input enters only a String \
             or Vec<U8> spot\n",
            2,
        ),
    ];
    for ((script, text, args, input), expected) in scripts.iter().zip(expected)
    {
        fs::write(dir.join(script), text).unwrap();
        let expected = (expected.0.into(), expected.1.into(), expected.2);
        let _ = fs::remove_file(dir.join("out.txt"));
        let mut run_args = vec!["run", script, "--"];
        run_args.extend(args.iter());
        let ran = shown(&fipe(&dir, &run_args, input));
        assert_eq!(ran, expected, "{script} under run");
        if *script == "ok.fipe" {
            let written = fs::read_to_string(dir.join("out.txt")).unwrap();
            assert_eq!(written, "written\n");
        }
        for more in [&[][..], &["--llvm"]] {
            let _ = fs::remove_file(dir.join("out.txt"));
            let exe = built_with(&dir, script, more);
            let ran = shown(&run_exe(&dir, &exe, args, input, &[]));
            assert_eq!(ran, expected, "{script} as an executable {more:?}");
            if *script == "ok.fipe" {
                let written = fs::read_to_string(dir.join("out.txt")).unwrap();
                assert_eq!(written, "written\n");
            }
        }
    }
    assert!(!dir.join("effect.txt").exists());
    // A script that does not check: its diagnostics and exit 2 from
    // both `run` and `build`.
    fs::write(dir.join("bad.fipe"), "nope\n").unwrap();
    let ran = shown(&fipe(&dir, &["run", "bad.fipe"], b""));
    assert_eq!(
        ran,
        (String::new(), "bad.fipe:1:1: unknown name\n".into(), 2)
    );
    let built = fipe(&dir, &["build", "bad.fipe", "-o", "bad.exe"], b"");
    assert_eq!(shown(&built), ran);
}

#[test]
fn the_repl_shows_values_and_performs_effects_through_the_std_host() {
    let dir = scratch("repl");
    let input = b"\"hello\"\n\n\"written\\n\" | put ./out.txt\n\n\
        [1, 2] | len\n\nuse text\n\ntext.trim \"  x \"\n\n:q\n";
    let (stdout, stderr, code) = shown(&fipe(&dir, &["repl"], input));
    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "host: tools/host/decl\nfipe> ....> \"hello\"\nfipe> ....> fipe> \
         ....> 2\nfipe> ....> fipe> ....> \"x\"\nfipe> "
    );
    assert_eq!(
        fs::read_to_string(dir.join("out.txt")).unwrap(),
        "written\n"
    );
}

#[test]
fn check_exits_by_the_worst_file() {
    let dir = scratch("check");
    fs::write(dir.join("ok.fipe"), "1 + 1\n").unwrap();
    fs::write(dir.join("bad.fipe"), "nope\n").unwrap();
    let ran = shown(&fipe(&dir, &["check", "ok.fipe"], b""));
    assert_eq!(ran, (String::new(), String::new(), 0));
    let ran = shown(&fipe(&dir, &["check", "ok.fipe", "bad.fipe"], b""));
    assert_eq!(
        ran,
        (String::new(), "bad.fipe:1:1: unknown name\n".into(), 1)
    );
    let both = fipe(&dir, &["check", "bad.fipe", "none.fipe"], b"");
    let (_, stderr, code) = shown(&both);
    assert_eq!(code, 2);
    assert!(
        stderr.starts_with(
            "bad.fipe:1:1: unknown name\nnone.fipe: cannot read: "
        ),
        "{stderr}"
    );
    let (_, stderr, code) = shown(&fipe(&dir, &["check"], b""));
    assert_eq!(code, 2);
    assert!(stderr.starts_with("usage: fipe run FILE"), "{stderr}");
    let (stdout, _, code) = shown(&fipe(&dir, &["help"], b""));
    assert_eq!(code, 0);
    assert!(stdout.starts_with("usage: fipe run FILE"), "{stdout}");
    let stray = fipe(&dir, &["run", "ok.fipe", "ERROR"], b"");
    let (_, stderr, code) = shown(&stray);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "fipe: unexpected operand ERROR; the script's arguments follow --\n"
    );
    let wrong = fipe(&dir, &["run", "ok.fipe", "--budget", "x"], b"");
    let (_, stderr, code) = shown(&wrong);
    assert_eq!(code, 2);
    assert!(stderr.starts_with("--budget takes a number\n"), "{stderr}");
}

#[test]
fn writes_outside_the_roots_are_refused_in_each_runner_unless_allowed() {
    let root = scratch("roots");
    let dir = root.join("work");
    let other = root.join("other");
    fs::create_dir_all(&dir).unwrap();
    fs::create_dir_all(&other).unwrap();
    let script = "\"x\" | put ../other/esc.txt\n";
    fs::write(dir.join("esc.fipe"), script).unwrap();
    let refused = (
        String::new(),
        String::from(
            "fault in main at 1:7: host: ../other/esc.txt is outside the \
             write roots\n",
        ),
        1,
    );
    assert_eq!(shown(&fipe(&dir, &["run", "esc.fipe"], b"")), refused);
    assert!(!other.join("esc.txt").exists());
    let allowed = ["run", "esc.fipe", "--allow-write", "../other"];
    let ran = fipe(&dir, &allowed, b"");
    assert_eq!(shown(&ran), (String::new(), String::new(), 0));
    assert_eq!(fs::read_to_string(other.join("esc.txt")).unwrap(), "x");
    fs::remove_file(other.join("esc.txt")).unwrap();
    let exe = built(&dir, "esc.fipe");
    assert_eq!(shown(&run_exe(&dir, &exe, &[], b"", &[])), refused);
    assert!(!other.join("esc.txt").exists());
    let allowed = [("FIPE_ALLOW_WRITE", "../other")];
    let ran = run_exe(&dir, &exe, &[], b"", &allowed);
    assert_eq!(shown(&ran), (String::new(), String::new(), 0));
    assert_eq!(fs::read_to_string(other.join("esc.txt")).unwrap(), "x");
}

#[test]
fn math_modules_load_from_copied_sources_in_all_runners() {
    let dir = scratch("math-library");
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for name in [
        "math.fipe",
        "stats.fipe",
        "summary.fipe",
        "interpolate.fipe",
    ] {
        fs::copy(library.join(name), dir.join(name)).unwrap();
    }
    fs::write(
        dir.join("overflow.fipe"),
        "use math\nmath.dot [1e308] [2.0]\n",
    )
    .unwrap();
    let scripts = [
        ("summary.fipe", "(Just 5.0, Just 4.0, Just 2.0)\n", "", 0),
        ("interpolate.fipe", "(12.5, Just 11.0)\n", "", 0),
        (
            "overflow.fipe",
            "",
            "fault in math at 14:51: the result is not finite\n",
            1,
        ),
    ];
    let mut executables = Vec::new();
    for (script, stdout, stderr, code) in scripts {
        assert_eq!(
            shown(&fipe(&dir, &["check", script], b"")),
            (String::new(), String::new(), 0),
            "{script} checks"
        );
        let expected = (stdout.to_owned(), stderr.to_owned(), code);
        assert_eq!(
            shown(&fipe(&dir, &["run", script], b"")),
            expected,
            "{script} under VM"
        );
        for more in [&[][..], &["--llvm"]] {
            executables
                .push((built_with(&dir, script, more), expected.clone()));
        }
    }
    fs::remove_file(dir.join("stats.fipe")).unwrap();
    let (stdout, stderr, code) =
        shown(&fipe(&dir, &["check", "summary.fipe"], b""));
    assert_eq!(stdout, "");
    assert_eq!(code, 1);
    assert!(stderr.starts_with("summary.fipe:1:5: unknown module\n"));
    for name in [
        "math.fipe",
        "summary.fipe",
        "interpolate.fipe",
        "overflow.fipe",
    ] {
        fs::remove_file(dir.join(name)).unwrap();
    }
    let empty = dir.join("empty-cwd");
    fs::create_dir(&empty).unwrap();
    for (exe, expected) in executables {
        assert_eq!(
            shown(&run_exe(&empty, &exe, &[], b"", &[])),
            expected,
            "{} without source files",
            exe.display()
        );
    }
}
