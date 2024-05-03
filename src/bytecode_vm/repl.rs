use std::io::{self, BufRead, Write};

use crate::{lexer::Lexer, parser::Parser, token::Kind};

use super::{bytecode::compiler::Compiler, vm::VM};
const PROMPT: &str = "-> ";

pub fn start() {
    let mut buf = String::new();
    let mut stdin = io::stdin().lock(); // We get `Stdin` here.

    let debug_lexer = false;
    let debug_parser = false;
    let show_error = true;

    let show_stack = true;
    let show_instruction = true;
    let run_single_cycle = true;

    loop {
        io::stdout().lock().write_all(PROMPT.as_bytes()).unwrap();
        io::stdout().flush().unwrap();
        match stdin.read_line(&mut buf) {
            Ok(_) => {
                let lexer = Lexer::new(buf.clone());

                let mut cloned_lexer = lexer.clone();

                if debug_lexer {
                    loop {
                        let cur_token = cloned_lexer.next();

                        println!("Debug Output (Lexer) >> {:?}", cur_token);

                        if cur_token.kind == Kind::EOF {
                            break;
                        }
                    }
                }

                let mut parser = Parser::new(lexer);
                let program = parser.parse();

                if debug_parser {
                    println!("Debug Output (Parser) >> {:?}", program);
                }

                if program.is_ok() {
                    let program = program.unwrap();
                    let mut comp = Compiler::new();
                    comp.compile(program);
                    let mut vm = VM::new(comp.bytecode());
                    buf.clear();
                    println!("Intitial state:{}", vm.to_string());

                    println!("Commands:");
                    println!("\tpressing enter: excute next cycle.");
                    println!("\t          exit: terminate");

                    loop {
                        match stdin.read_line(&mut buf) {
                            Ok(_) => {
                                // HACK: clear screen
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                if buf == "exit" {
                                    break;
                                }
                                if !vm.is_runable() {
                                    break;
                                }
                                vm.run_single();

                                println!("{}", vm.to_string());
                            }
                            Err(err) => {
                                println!("Error occured during reading stdin");
                                println!("{:?}", err);
                            }
                        }
                    }

                    println!("VM Terminated.");
                    println!("Please give new input.");
                } else {
                    if show_error {
                        println!("!!!> ERROR OCCURED <!!!");
                        for errs in program.err().unwrap() {
                            println!(">> ERROR DETAIL ");
                            for err in errs {
                                println!("Pos>> {:?}", err.as_ref().position());
                                println!("Detail>> {} ", err.as_ref().detail());
                            }
                        }
                    }
                }
                buf.clear();
            }
            Err(err) => {
                println!("Error occured during reading stdin");
                println!("{:?}", err);
                return;
            }
        }
    }
}
