#![cfg(not(miri))]

use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn exits_on_eof(mode: &str) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe"))
        .arg(mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{mode} kept running after EOF");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn bytecode_repl_exits_on_eof() {
    exits_on_eof("mode=bytecode");
}

#[test]
fn treewalker_repl_exits_on_eof() {
    exits_on_eof("mode=treewalker");
}
