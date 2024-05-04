use std::time::Instant;

use crate::{
    ast::Nodetrait,
    bytecode_vm::{bytecode::compiler::Compiler, vm::VM},
    heap::Heap,
    lexer::Lexer,
    object::environment::Environment,
    parser::Parser,
    treewalker::eval::evaluate,
};

pub fn benchmark() {
    println!("start benchmark\n");

    let fibo = "
let fibonacci = fn(x) {
    if (x == 0) {
        0
    } else {
        if (x == 1) {
            return 1;
        } else {
            fibonacci(x - 1) + fibonacci(x - 2)
        }
    }
};
fibonacci(25);
"
    .to_string();
    println!("parsing: {}", fibo);

    let start_time = Instant::now();

    let lex = Lexer::new(fibo);
    let mut parser = Parser::new(lex);
    let program = parser.parse().unwrap();

    let parsed_time = start_time.elapsed();
    println!("parse_time : {:?}", parsed_time);

    let mut comp = Compiler::new();

    let compile_start = Instant::now();

    comp.compile(program.clone());

    let compile_time = compile_start.elapsed();

    println!("compile_time : {:?}", compile_time);

    let bytecode = comp.bytecode();

    let mut vm = VM::new(bytecode);
    let vm_start = Instant::now();
    println!("\n\nLaunching VM...");
    while vm.is_runable() {
        vm.run_single()
    }
    let vm_time = vm_start.elapsed();

    println!("vm_time : {:?}\nvm_result : {:?}", vm_time, vm.last_pop());

    let mut env = Environment::new();
    let mut heap = Heap::new();
    let eval_start = Instant::now();
    println!("\n\nLaunching Evaluator...");
    let result = evaluate(program.to_node(), &mut heap, &mut env);
    let eval_time = eval_start.elapsed();

    println!("eval_time : {:?}\neval_result : {:?}", eval_time, result);

    println!(
        "eval_time/vm_time : {:?}",
        eval_time.as_secs_f64() / vm_time.as_secs_f64()
    );
}

pub fn benchmark_long() {
    println!("start benchmark_long\n");
    let fibo = "
let fibonacci = fn(x) {
    if (x == 0) {
        0
    } else {
        if (x == 1) {
            return 1;
        } else {
            fibonacci(x - 1) + fibonacci(x - 2)
        }
    }
};
fibonacci(30);
"
    .to_string();
    println!("parsing: {}", fibo);

    let start_time = Instant::now();

    let lex = Lexer::new(fibo);
    let mut parser = Parser::new(lex);
    let program = parser.parse().unwrap();

    let parsed_time = start_time.elapsed();
    println!("parse_time : {:?}", parsed_time);

    let mut comp = Compiler::new();

    let compile_start = Instant::now();

    comp.compile(program.clone());

    let compile_time = compile_start.elapsed();

    println!("compile_time : {:?}", compile_time);

    let bytecode = comp.bytecode();

    let mut vm = VM::new(bytecode);
    let vm_start = Instant::now();
    println!("\n\nLaunching VM...");
    while vm.is_runable() {
        vm.run_single()
    }
    let vm_time = vm_start.elapsed();

    println!("vm_time : {:?}\nvm_result : {:?}", vm_time, vm.last_pop());

    let mut env = Environment::new();
    let mut heap = Heap::new();
    let eval_start = Instant::now();
    println!("\n\nLaunching Evaluator...");
    let result = evaluate(program.to_node(), &mut heap, &mut env);
    let eval_time = eval_start.elapsed();

    println!("eval_time : {:?}\neval_result : {:?}", eval_time, result);

    println!(
        "eval_time/vm_time : {:?}",
        eval_time.as_secs_f64() / vm_time.as_secs_f64()
    );
}

