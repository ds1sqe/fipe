//! The dogfood scripts of `Documentation/dogfood/` run to their golden
//! output: each with the arguments and input its header names, from
//! that directory, under the std host.
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

fn dogfood() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../Documentation/dogfood")
        .canonicalize()
        .unwrap()
}

/// Each script: its name, arguments, input file and exit code.
const SCRIPTS: &[(&str, &[&str], Option<&str>, i32)] = &[
    ("wc", &[], Some("input/poem.txt"), 0),
    ("grep-log", &["ERROR"], Some("input/app.log"), 0),
    ("du", &["input/tree"], None, 0),
    ("csv-sum", &["units"], Some("input/sales.csv"), 0),
    ("config-lint", &[], Some("input/app.conf"), 1),
    ("dedupe", &[], Some("input/fruit.txt"), 0),
    ("todos", &["input/src"], None, 0),
    ("rename-plan", &["input/names"], None, 0),
    ("rm-old", &["input/stale", "315360000"], None, 0),
    ("calc", &[], Some("input/sums.txt"), 0),
];

/// `rm-old` reads each entry's age: the stale files are dated before
/// the cutoff and the kept ones after, as the script's header says.
fn date_stale(dir: &Path) {
    let year = Duration::from_hours(365 * 24);
    let old = SystemTime::UNIX_EPOCH + 31 * year;
    let new = SystemTime::now() + 20 * year;
    for (file, time) in [
        ("old.log", old),
        ("cache/older.tmp", old),
        ("keep.txt", new),
        ("cache/newer.tmp", new),
    ] {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(dir.join("input/stale").join(file))
            .unwrap();
        file.set_modified(time).unwrap();
    }
}

#[test]
fn every_dogfood_script_runs_to_its_golden_output() {
    let dir = dogfood();
    date_stale(&dir);
    for (name, args, input, exit) in SCRIPTS {
        let mut child = Command::new(env!("CARGO_BIN_EXE_fipe"))
            .arg("run")
            .arg(format!("{name}.fipe"))
            .arg("--")
            .args(*args)
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let bytes = input
            .map_or_else(Vec::new, |input| fs::read(dir.join(input)).unwrap());
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        let golden = fs::read(dir.join(format!("{name}.out"))).unwrap();
        let error =
            fs::read(dir.join(format!("{name}.err"))).unwrap_or_default();
        assert_eq!(output.status.code(), Some(*exit), "{name}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&golden),
            "{name}: stdout"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&error),
            "{name}: stderr"
        );
    }
}
