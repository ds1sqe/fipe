//! The deepest expression the parser admits checks, compiles and runs
//! on a 2 MiB stack: every stage after the parser walks the tree.
use fipe::{
    check, compile, decl::Declarations, embed::Outcome, lex::lex, parse::parse,
    parse::MAX_DEPTH, value::structural, vm::Machine,
};

/// The value `source` runs to, on a thread with a 2 MiB stack.
fn run_on_small_stack(source: String) -> String {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let module = parse(lex(&source)).unwrap();
            let unit = check::check(
                &module,
                &Declarations::new(),
                &[],
                check::Options::default(),
            )
            .unwrap();
            let program = compile::compile(&unit);
            let mut machine = Machine::new(None);
            let Outcome::Done(Ok(value)) = machine.run(&program) else {
                panic!("the run ends with a value")
            };
            let shown = structural::show(&value, &program.tables.borrow());
            shown
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn the_deepest_operator_chain_runs_on_a_small_stack() {
    let chain = format!("0{}\n", " + 1".repeat(MAX_DEPTH - 1));
    assert_eq!(run_on_small_stack(chain), (MAX_DEPTH - 1).to_string());
}

#[test]
fn the_deepest_pipeline_runs_on_a_small_stack() {
    let pipeline =
        format!("f = x => x + 1\n0{}\n", " | f".repeat(MAX_DEPTH - 1));
    assert_eq!(run_on_small_stack(pipeline), (MAX_DEPTH - 1).to_string());
}

#[test]
fn the_deepest_lift_chain_runs_on_a_small_stack() {
    let stages = MAX_DEPTH - 8;
    let chain = format!("Just 0{}\n", " |: (x => Just (x + 1))".repeat(stages));
    assert_eq!(run_on_small_stack(chain), format!("Just {stages}"));
}

#[test]
fn the_deepest_condition_chain_runs_on_a_small_stack() {
    // Each `&&` sits one above the last, over `1 < 2`, two deep.
    let chain = format!("True{}\n", " && 1 < 2".repeat(MAX_DEPTH - 2));
    assert_eq!(run_on_small_stack(chain), "True");
}
