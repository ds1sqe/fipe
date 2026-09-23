use std::env;

use fipe::{benchmark::benchmark, bytecode_vm, treewalker};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        println!("Please choose run option");
        println!("mode=bytecode\t\t-> will launch bytecode-vm");
        println!("mode=treewalker\t\t-> will launch treewalker");
        println!("mode=benchmark\t\t-> will launch a benchmark");
        return;
    } else {
        if args[1] == "mode=bytecode" {
            bytecode_vm::repl::start()
        } else if args[1] == "mode=treewalker" {
            treewalker::repl::start()
        } else if args[1] == "mode=benchmark" {
            benchmark();
        } else {
            println!("Invalid option: {}", args[1]);
        }
    }
}
