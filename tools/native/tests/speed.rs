//! The VM against the native backend on a deep recursion, a doubly
//! recursive `fib` and a `map`/`fold` loop: the median wall time of
//! several runs of each. A measurement, not a gate, so it is ignored;
//! run it by hand in release:
//! `cargo test --release --locked --offline --test speed -- --ignored
//! --nocapture`.
use fipe::{embed::Outcome, value::structural::show, vm::Machine};
use fipe_native::codec::Unit;
use std::{
    process::Command,
    time::{Duration, Instant},
};

mod common;

const RUNS: usize = 7;

/// Each program, by name; `startup` is the executable's fixed cost,
/// recompiling the unit it carries.
const PROGRAMS: &[(&str, &str)] = &[
    ("startup", "0"),
    (
        "deep recursion",
        "down: Integer -> Integer = \
         n => if n == 0 { 0 } else { n + (down (n - 1)) }\n\
         down 1000000",
    ),
    (
        "fib",
        "fib: Integer -> Integer = \
         n => if n < 2 { n } else { (fib (n - 1)) + (fib (n - 2)) }\n\
         fib 25",
    ),
    (
        "map/fold",
        "dbl: Vec<Integer> -> Vec<Integer> = xs => xs.concat xs\n\
         grow: Integer -> Vec<Integer> = \
         k => if k == 0 { [1] } else { dbl (grow (k - 1)) }\n\
         xs = grow 18\n\
         xs | map (n => n * 3) | fold 0 (acc n => acc + n)",
    ),
];

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
#[ignore = "a measurement, run by hand"]
fn the_native_backend_against_the_vm() {
    println!("| program | VM | native | VM / native |");
    println!("| --- | --- | --- | --- |");
    for (name, source) in PROGRAMS {
        let (unit, program) = Unit::new(source, Vec::new(), Vec::new())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let executable =
            common::executable(&name.replace([' ', '/'], "-"), &unit, &program);
        let mut shown = String::new();
        let vm = median(
            (0..RUNS)
                .map(|_| {
                    let start = Instant::now();
                    let Outcome::Done(Ok(value)) =
                        Machine::new(None).run(&program)
                    else {
                        panic!("{name}: the VM run does not end with a value")
                    };
                    let time = start.elapsed();
                    shown = show(&value, &program.tables.borrow());
                    time
                })
                .collect(),
        );
        let native = median(
            (0..RUNS)
                .map(|_| {
                    let start = Instant::now();
                    let output = Command::new(&executable)
                        .env_remove("FIPE_BUDGET")
                        .output()
                        .unwrap();
                    let time = start.elapsed();
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    assert_eq!(stdout, format!("{shown}\n"), "{name}");
                    time
                })
                .collect(),
        );
        let ratio = vm.as_secs_f64() / native.as_secs_f64();
        println!("| {name} | {vm:.2?} | {native:.2?} | {ratio:.2} |");
    }
}
