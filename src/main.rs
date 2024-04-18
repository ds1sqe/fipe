use std::env;

use dlang::treewalker::repl::start;

fn main() {
    let args: Vec<String> = env::args().collect();
    dbg!(args);
    start()
}
