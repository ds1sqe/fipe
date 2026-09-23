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

    loop {
        io::stdout().lock().write_all(PROMPT.as_bytes()).unwrap();
        io::stdout().flush().unwrap();
        match stdin.read_line(&mut buf) {
            Ok(0) => return,
            Ok(_) => {
                let lexer = Lexer::new(buf.clone());

                let mut cloned_lexer = lexer.clone();

                if debug_lexer {
                    loop {
                        let cur_token = cloned_lexer.next_token();

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

                if let Ok(program) = program {
                    let comp_rst = Compiler::create();
                    if comp_rst.is_err() {
                        println!(
                            "Error while compiler creation {:?}",
                            comp_rst
                        );
                        continue;
                    }
                    let mut compiler = comp_rst.unwrap();

                    let comp = compiler.compile(program);

                    if comp.is_err() {
                        println!("Error while compiler creation {:?}", comp);
                        continue;
                    }

                    let bytecode_rst = compiler.bytecode();
                    match bytecode_rst {
                        Ok(_) => (),
                        Err(err) => {
                            println!("Error while taking bytecode {:?}", err);
                            continue;
                        }
                    }

                    let mut vm =
                        unsafe { VM::new(bytecode_rst.unwrap_unchecked()) };
                    buf.clear();
                    println!("Intitial state:{}", vm);

                    println!("Commands:");
                    println!("\tpressing enter: excute next cycle.");
                    println!("\t          exit: terminate");

                    loop {
                        buf.clear();
                        match stdin.read_line(&mut buf) {
                            Ok(0) => return,
                            Ok(_) => {
                                // HACK: clear screen
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                println!("\n\n\n\n\n\n\n\n\n");
                                if buf.trim_end() == "exit" {
                                    break;
                                }
                                if !vm.is_runable() {
                                    break;
                                }
                                if let Err(e) = vm.run_single() {
                                    eprintln!("{:?}", e);
                                    break;
                                }

                                println!("{}", vm);
                            }
                            Err(err) => {
                                println!("Error occured during reading stdin");
                                println!("{:?}", err);
                            }
                        }
                    }

                    println!("VM Terminated.");
                    println!("Please give new input.");
                } else if show_error {
                    println!("!!!> ERROR OCCURED <!!!");
                    for errs in program.err().unwrap() {
                        println!(">> ERROR DETAIL ");
                        for err in errs {
                            println!("Pos>> {:?}", err.as_ref().position());
                            println!("Detail>> {} ", err.as_ref().detail());
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
