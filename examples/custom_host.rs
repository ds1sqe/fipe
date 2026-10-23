//! A host declares a capability and answers the VM's requests.
use fipe::{
    check, compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    lex::lex,
    parse::parse,
    value::Value,
    vm::Machine,
};

fn main() {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "app.greetings",
            "## Greet a name.\npub greet: String -> String",
        )
        .expect("valid host declaration");
    let module = parse(lex("use app.greetings\ngreetings.greet \"Fipe\""))
        .expect("valid syntax");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("valid script");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(&program);
    loop {
        match outcome {
            Outcome::Done(result) => {
                let value = result.expect("successful host call");
                assert_eq!(value, Value::text("Hello, Fipe!"));
                println!("{value:?}");
                break;
            }
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) => {
                let tables = program.tables.borrow();
                let answer = match (
                    tables.declarations[declaration].label.as_str(),
                    arguments.as_slice(),
                ) {
                    ("greet", [Value::Text(name)]) => {
                        Ok(Value::text(&format!("Hello, {name}!")))
                    }
                    _ => Err(String::from("unsupported host call")),
                };
                drop(tables);
                outcome = machine.reply(&program, Reply::Value(answer));
            }
            Outcome::Request(Request::File { .. }) => {
                outcome = machine.reply(&program, Reply::File(None));
            }
        }
    }
}
