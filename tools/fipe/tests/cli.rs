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
fn repl_eof_drains_trailing_input_and_effects_without_quit() {
    let dir = scratch("repl-eof");
    let input = b"\"written\" | put ./out.txt\n\n40 + 2";
    let (stdout, stderr, code) = shown(&fipe(&dir, &["repl"], input));
    assert_eq!(code, 0);
    assert_eq!(stderr, "");
    assert!(stdout.ends_with("42\n"), "{stdout}");
    assert_eq!(fs::read_to_string(dir.join("out.txt")).unwrap(), "written");
}

#[cfg(unix)]
#[test]
fn repl_read_error_closes_before_retaining_exit_two() {
    let dir = scratch("repl-read-error");
    let output = Command::new(env!("CARGO_BIN_EXE_fipe"))
        .arg("repl")
        .current_dir(&dir)
        .stdin(Stdio::from(fs::File::open(&dir).unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let (stdout, stderr, code) = shown(&output);
    assert_eq!(code, 2);
    assert_eq!(stderr, "");
    assert!(stdout.ends_with("cannot read standard input\n"), "{stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn output_failure_stops_direct_native_and_llvm_before_the_next_effect() {
    let dir = scratch("output-failure");
    let script = "output.fipe";
    fs::write(dir.join(script), "\"before\"\nput ./after \"no\"\n").unwrap();
    let native = built(&dir, script);
    let llvm = built_with(&dir, script, &["--llvm"]);
    for (exe, args) in [
        (Path::new(env!("CARGO_BIN_EXE_fipe")), vec!["run", script]),
        (native.as_path(), vec![]),
        (llvm.as_path(), vec![]),
    ] {
        let failed = Command::new(exe)
            .args(&args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            ))
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1), "{failed:?}");
        assert!(!dir.join("after").exists());
        let healthy = run_exe(&dir, exe, &args, b"", &[]);
        assert_eq!(healthy.status.code(), Some(0), "{healthy:?}");
        assert_eq!(healthy.stdout, b"before\n");
        assert_eq!(fs::read(dir.join("after")).unwrap(), b"no");
        fs::remove_file(dir.join("after")).unwrap();
    }
    let terminal = "terminal.fipe";
    fs::write(dir.join(terminal), "42\n").unwrap();
    let native = built(&dir, terminal);
    let llvm = built_with(&dir, terminal, &["--llvm"]);
    for (exe, args) in [
        (Path::new(env!("CARGO_BIN_EXE_fipe")), vec!["run", terminal]),
        (native.as_path(), vec![]),
        (llvm.as_path(), vec![]),
    ] {
        let failed = Command::new(exe)
            .args(args)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            ))
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    }
}

#[test]
fn command_discovery_reports_version_help_and_usage_without_running() {
    let dir = scratch("command-discovery");
    let version = fipe(&dir, &["--version"], b"");
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        version.stdout,
        format!("fipe {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(version.stderr.is_empty());
    for args in [
        &["--help"][..],
        &["-h"],
        &["help"],
        &["help", "ignored"],
        &["--help", "ignored"],
    ] {
        let output = fipe(&dir, args, b"");
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert!(output.stdout.starts_with(b"usage: fipe run FILE"));
    }
    for args in [
        &["--version", "extra"][..],
        &["--unknown"],
        &["run", "--version"],
    ] {
        let output = fipe(&dir, args, b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

fn fmt_command(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fipe"))
        .arg("fmt")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn fmt_formats_checks_and_keeps_failed_inputs_unchanged() {
    let dir = scratch("fmt-files");
    let good = dir.join("good.fipe");
    let bad = dir.join("bad.fipe");
    fs::write(&good, "x  =   missing\n").unwrap();
    fs::write(&bad, "x = = 1\n").unwrap();
    let output = fmt_command(&["--check".as_ref(), good.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_to_string(&good).unwrap(), "x  =   missing\n");
    assert_eq!(
        fmt_command(&[good.as_os_str(), bad.as_os_str()])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fs::read_to_string(&good).unwrap(), "x = missing\n");
    assert_eq!(fs::read_to_string(&bad).unwrap(), "x = = 1\n");
    assert_eq!(
        fmt_command(&["--check".as_ref(), good.as_os_str()])
            .status
            .code(),
        Some(0)
    );
    fs::write(dir.join("bytes.fipe"), [255]).unwrap();
    assert_eq!(
        fmt_command(&[bad.as_os_str(), dir.join("bytes.fipe").as_os_str()])
            .status
            .code(),
        Some(2)
    );
    for args in [
        vec![],
        vec!["--unknown".as_ref()],
        vec!["-".as_ref()],
        vec!["--check".as_ref(), "--check".as_ref(), good.as_os_str()],
    ] {
        assert_eq!(fmt_command(&args).status.code(), Some(2));
    }
}

#[test]
fn fmt_readonly_clean_and_check_do_not_require_writing() {
    let dir = scratch("fmt-readonly");
    let path = dir.join("a.fipe");
    fs::write(&path, "x = 1\n").unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    assert_eq!(fmt_command(&[path.as_os_str()]).status.code(), Some(0));
    assert_eq!(
        fmt_command(&["--check".as_ref(), path.as_os_str()])
            .status
            .code(),
        Some(0)
    );
    fs::set_permissions(&path, permissions.clone()).unwrap();
    fs::write(&path, "x  =  1\n").unwrap();
    let mut readonly = fs::metadata(&path).unwrap().permissions();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    assert_eq!(
        fmt_command(&["--check".as_ref(), path.as_os_str()])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(fmt_command(&[path.as_os_str()]).status.code(), Some(2));
    assert_eq!(fs::read_to_string(&path).unwrap(), "x  =  1\n");
    fs::set_permissions(&path, permissions).unwrap();
}

#[cfg(unix)]
#[test]
fn fmt_exact_os_paths_permissions_and_symlinks() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{symlink, PermissionsExt},
    };
    let dir = scratch("fmt-os-paths");
    let name = std::ffi::OsString::from_vec(b"bad-\xff.fipe".to_vec());
    let path = dir.join(&name);
    let neighbor = dir.join(name.to_string_lossy().as_ref());
    fs::write(&path, "x  =  1\n").unwrap();
    fs::write(&neighbor, "neighbor bytes").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(fmt_command(&[path.as_os_str()]).status.code(), Some(0));
    assert_eq!(fs::read_to_string(&path).unwrap(), "x = 1\n");
    assert_eq!(fs::read_to_string(&neighbor).unwrap(), "neighbor bytes");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let link = dir.join("link");
    symlink(&path, &link).unwrap();
    assert_eq!(fmt_command(&[link.as_os_str()]).status.code(), Some(2));
    assert_eq!(fmt_command(&[dir.as_os_str()]).status.code(), Some(2));
    let dash = dir.join("-dash.fipe");
    fs::write(&dash, "x  =  1\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_fipe"))
        .current_dir(&dir)
        .args(["fmt", "--", "-dash.fipe"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(fs::read_dir(&dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".fipe-fmt-")));
}

#[test]
fn fmt_refuses_unsafe_mixed_literal_without_writing() {
    let path = scratch("fmt-mixed-literal").join("mixed.fipe");
    let original = "x = 1\r\ns = re'one\ntwo'\r\n";
    fs::write(&path, original).unwrap();
    assert_eq!(fmt_command(&[path.as_os_str()]).status.code(), Some(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn fmt_unsupported_option_keeps_quotes_and_escaped_control_characters() {
    let output = fmt_command(&[std::ffi::OsStr::new("--bad\n\"")]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"fipe fmt: unsupported option \"--bad\\n\\\"\"\n"
    );
}
