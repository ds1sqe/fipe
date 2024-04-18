use std::env;

use dlang::{bytecode_vm, treewalker};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        println!("Please choose run option");
        println!("mode=bytecode\t\t-> will launch bytecode-vm");
        println!("mode=treewalker\t\t-> will launch treewalker");
        return;
    } else {
        if args[1] == "mode=bytecode" {
            bytecode_vm::repl::start()
        } else if args[1] == "mode=treewalker" {
            treewalker::repl::start()
        } else {
            println!("Invalid option: {}", args[1]);
        }
    }
}
