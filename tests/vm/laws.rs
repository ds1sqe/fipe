//! The monad laws of the marked pipes: left identity, right identity
//! and associativity of `|:` on `Maybe` and `Result` and of `|!` on
//! `Result`'s error side, over generated values and functions. Each
//! case's outcome is computed here from the generated case, not only
//! compared between the two sides of its law.
use super::run::shown;
use core::fmt::Write;

/// Cases per law, checked in units of `BATCH`.
const CASES: usize = 1_000;
const BATCH: usize = 100;

/// A small seeded generator (xorshift64).
struct Gen(u64);

impl Gen {
    const fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> i64 {
        i64::try_from(self.next() % bound).unwrap()
    }
}

/// An outcome as the language shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Good(i64),
    Bad(i64),
    Miss,
}

impl Side {
    fn shown(self, maybe: bool) -> String {
        match (self, maybe) {
            (Self::Good(v), true) => format!("Just {v}"),
            (Self::Good(v), false) => format!("Ok {v}"),
            (Self::Bad(e), _) => format!("Err {e}"),
            (Self::Miss, _) => String::from("Nothing"),
        }
    }
}

/// What a generated function does with its argument: continue the
/// chain with a payload, or stop it, with a payload unless on `Maybe`.
#[derive(Clone, Copy, Debug)]
enum Step {
    Continue(i64),
    Stop(i64),
}

/// A generated function from an integer to an outcome. Every payload
/// it makes is non-negative.
#[derive(Clone, Copy, Debug)]
enum Fun {
    /// `v => continue (v * a + b)`
    Affine(i64, i64),
    /// `v => stop (v + k)`
    Fail(i64),
    /// `v => if v > c { continue (v - c) } else { stop (v + c) }`
    Above(i64),
    /// `v => if v < c { continue (c - v) } else { stop (v + c) }`
    Below(i64),
}

impl Fun {
    fn generate(gen: &mut Gen) -> Self {
        match gen.below(4) {
            0 => Self::Affine(gen.below(5), gen.below(10)),
            1 => Self::Fail(gen.below(10)),
            2 => Self::Above(gen.below(30)),
            _ => Self::Below(gen.below(30)),
        }
    }

    /// The source over parameter `v`, with `go` the constructor that
    /// continues and `stop` the one that stops (`None`: `Nothing`).
    fn source(self, v: &str, go: &str, stop: Option<&str>) -> String {
        let stop = |k: i64| {
            stop.map_or_else(
                || String::from("Nothing"),
                |stop| format!("{stop} ({v} + {k})"),
            )
        };
        match self {
            Self::Affine(a, b) => format!("{v} => {go} ({v} * {a} + {b})"),
            Self::Fail(k) => format!("{v} => {}", stop(k)),
            Self::Above(c) => format!(
                "{v} => if {v} > {c} {{ {go} ({v} - {c}) }} else {{ {} }}",
                stop(c)
            ),
            Self::Below(c) => format!(
                "{v} => if {v} < {c} {{ {go} ({c} - {v}) }} else {{ {} }}",
                stop(c)
            ),
        }
    }

    const fn apply(self, v: i64) -> Step {
        match self {
            Self::Affine(a, b) => Step::Continue(v * a + b),
            Self::Fail(k) => Step::Stop(v + k),
            Self::Above(c) if v > c => Step::Continue(v - c),
            Self::Below(c) if v < c => Step::Continue(c - v),
            Self::Above(c) | Self::Below(c) => Step::Stop(v + c),
        }
    }
}

/// Which pipe the law is about.
#[derive(Clone, Copy)]
enum Pipe {
    /// `|:` on `Maybe`.
    ThenMaybe,
    /// `|:` on `Result`.
    ThenResult,
    /// `|!` on `Result`: a function takes the error; `Err` continues
    /// the chain on the error side and `Ok` stops it.
    OrResult,
}

impl Pipe {
    const fn maybe(self) -> bool {
        matches!(self, Self::ThenMaybe)
    }

    const fn mark(self) -> &'static str {
        match self {
            Self::ThenMaybe | Self::ThenResult => "|:",
            Self::OrResult => "|!",
        }
    }

    /// The constructor that continues the chain, as source.
    const fn unit(self) -> &'static str {
        match self {
            Self::ThenMaybe => "Just",
            Self::ThenResult => "Ok",
            Self::OrResult => "Err",
        }
    }

    /// The constructor that stops the chain; `None` for `Nothing`.
    const fn stop(self) -> Option<&'static str> {
        match self {
            Self::ThenMaybe => None,
            Self::ThenResult => Some("Err"),
            Self::OrResult => Some("Ok"),
        }
    }

    /// The side a step lands on.
    const fn side(self, step: Step) -> Side {
        match (self, step) {
            (Self::ThenMaybe | Self::ThenResult, Step::Continue(v))
            | (Self::OrResult, Step::Stop(v)) => Side::Good(v),
            (Self::ThenMaybe, Step::Stop(_)) => Side::Miss,
            (Self::ThenResult, Step::Stop(e))
            | (Self::OrResult, Step::Continue(e)) => Side::Bad(e),
        }
    }

    /// A generated value of the family.
    fn value(self, gen: &mut Gen) -> Side {
        let v = gen.below(50);
        match (self, gen.below(3)) {
            (Self::ThenMaybe, 0) => Side::Miss,
            (Self::ThenResult | Self::OrResult, 0) => Side::Bad(v),
            _ => Side::Good(v),
        }
    }

    fn function(self, f: Fun, v: &str) -> String {
        f.source(v, self.unit(), self.stop())
    }

    const fn apply(self, f: Fun, v: i64) -> Side {
        self.side(f.apply(v))
    }

    /// `m <mark> f`: the pipe's meaning.
    fn bind(self, m: Side, f: impl Fn(i64) -> Side) -> Side {
        match (self, m) {
            (Self::ThenMaybe | Self::ThenResult, Side::Good(v))
            | (Self::OrResult, Side::Bad(v)) => f(v),
            (_, other) => other,
        }
    }
}

/// One law's cases, each as its two sides' source and the outcome
/// both must show.
type Case = (String, String, Side);

/// Run cases in units of `BATCH`: each unit binds its cases' functions
/// and shows both sides of every case in one vector.
fn check(pipe: Pipe, fs: &[(Fun, Fun)], cases: &[Case]) {
    for (batch, chunk) in cases.chunks(BATCH).enumerate() {
        let mut source = String::new();
        let mut sides = Vec::new();
        let mut expected = Vec::new();
        for (offset, (left, right, outcome)) in chunk.iter().enumerate() {
            let index = batch * BATCH + offset;
            let (f, g) = fs[index];
            let _ = writeln!(source, "f{index} = {}", pipe.function(f, "x"));
            let _ = writeln!(source, "g{index} = {}", pipe.function(g, "y"));
            sides.extend([left.as_str(), right.as_str()]);
            let shown = outcome.shown(pipe.maybe());
            expected.extend([shown.clone(), shown]);
        }
        let _ = writeln!(source, "[{}]", sides.join(", "));
        let expected = format!("[{}]", expected.join(", "));
        assert_eq!(shown(&source), expected, "{source}");
    }
}

fn generate(pipe: Pipe, seed: u64) -> (Vec<Side>, Vec<i64>, Vec<(Fun, Fun)>) {
    let mut gen = Gen(seed);
    let mut values = Vec::new();
    let mut payloads = Vec::new();
    let mut fs = Vec::new();
    for _ in 0..CASES {
        values.push(pipe.value(&mut gen));
        payloads.push(gen.below(50));
        fs.push((Fun::generate(&mut gen), Fun::generate(&mut gen)));
    }
    (values, payloads, fs)
}

/// `unit a <mark> f` shows as `f a`.
fn left_identity(pipe: Pipe, seed: u64) {
    let (_, payloads, fs) = generate(pipe, seed);
    let (mark, unit) = (pipe.mark(), pipe.unit());
    let cases: Vec<Case> = payloads
        .iter()
        .zip(&fs)
        .enumerate()
        .map(|(i, (&a, &(f, _)))| {
            (
                format!("({unit} {a} {mark} f{i})"),
                format!("(f{i} {a})"),
                pipe.apply(f, a),
            )
        })
        .collect();
    check(pipe, &fs, &cases);
}

/// `m <mark> (v => unit v)` shows as `m`.
fn right_identity(pipe: Pipe, seed: u64) {
    let (values, _, fs) = generate(pipe, seed);
    let (mark, unit) = (pipe.mark(), pipe.unit());
    let cases: Vec<Case> = values
        .iter()
        .map(|&m| {
            let m_source = m.shown(pipe.maybe());
            (
                format!("(({m_source}) {mark} (v => {unit} v))"),
                format!("({m_source})"),
                pipe.bind(m, |v| pipe.side(Step::Continue(v))),
            )
        })
        .collect();
    check(pipe, &fs, &cases);
}

/// `(m <mark> f) <mark> g` shows as `m <mark> (v => f v <mark> g)`.
fn associativity(pipe: Pipe, seed: u64) {
    let (values, _, fs) = generate(pipe, seed);
    let mark = pipe.mark();
    let cases: Vec<Case> = values
        .iter()
        .zip(&fs)
        .enumerate()
        .map(|(i, (&m, &(f, g)))| {
            let m_source = m.shown(pipe.maybe());
            let outcome = pipe
                .bind(pipe.bind(m, |v| pipe.apply(f, v)), |v| pipe.apply(g, v));
            (
                format!("((({m_source}) {mark} f{i}) {mark} g{i})"),
                format!("(({m_source}) {mark} (v => (f{i} v) {mark} g{i}))"),
                outcome,
            )
        })
        .collect();
    check(pipe, &fs, &cases);
}

#[test]
fn then_on_maybe_has_a_left_identity() {
    left_identity(Pipe::ThenMaybe, 0x9e37_79b9_7f4a_7c15);
}

#[test]
fn then_on_maybe_has_a_right_identity() {
    right_identity(Pipe::ThenMaybe, 0xbf58_476d_1ce4_e5b9);
}

#[test]
fn then_on_maybe_is_associative() {
    associativity(Pipe::ThenMaybe, 0x94d0_49bb_1331_11eb);
}

#[test]
fn then_on_result_has_a_left_identity() {
    left_identity(Pipe::ThenResult, 0x2545_f491_4f6c_dd1d);
}

#[test]
fn then_on_result_has_a_right_identity() {
    right_identity(Pipe::ThenResult, 0x1405_7b7e_f767_814f);
}

#[test]
fn then_on_result_is_associative() {
    associativity(Pipe::ThenResult, 0xd6e8_feb8_6659_fd93);
}

#[test]
fn or_on_result_has_a_left_identity() {
    left_identity(Pipe::OrResult, 0xa076_1d64_78bd_642f);
}

#[test]
fn or_on_result_has_a_right_identity() {
    right_identity(Pipe::OrResult, 0xe703_7ed1_a0b4_28db);
}

#[test]
fn or_on_result_is_associative() {
    associativity(Pipe::OrResult, 0x8ebc_6af0_9c88_c6e3);
}
