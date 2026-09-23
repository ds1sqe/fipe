use fipe::{
    ast::Nodetrait,
    bytecode_vm::{
        bytecode::compiler::Compiler,
        vm::{errors::VmError, VM},
    },
    heap::Heap,
    lexer::Lexer,
    object::{environment::Environment, Object},
    parser::Parser,
    token::{Kind, Token},
    treewalker::eval::evaluate,
};

#[test]
fn lexer_preserves_unicode_strings_and_identifiers() {
    let mut lexer = Lexer::new("let 이름 = \"café 한글\"; 이름".to_owned());
    for expected in [
        Token::new(Kind::Let),
        Token::with(Kind::Ident, "이름"),
        Token::new(Kind::Assign),
        Token::with(Kind::String, "café 한글"),
        Token::new(Kind::Semicolon),
        Token::with(Kind::Ident, "이름"),
        Token::new(Kind::EOF),
        Token::new(Kind::EOF),
    ] {
        assert_eq!(lexer.next_token(), expected);
    }
}

fn run_vm(source: &str) -> Result<Option<Object>, VmError> {
    let program = Parser::new(Lexer::new(source.to_owned())).parse().unwrap();
    let mut compiler = Compiler::create().unwrap();
    compiler.compile(program).unwrap();
    let mut vm = VM::new(compiler.bytecode().unwrap());
    while vm.is_runable() {
        vm.run_single()?;
    }
    Ok(vm.last_pop().clone())
}

#[test]
fn vm_rejects_out_of_bounds_array_indices() {
    for source in ["[1][1]", "[1][-1]", "[][0]"] {
        assert!(run_vm(source).is_err(), "{source}");
    }
}

#[test]
fn evaluator_rejects_index_equal_to_array_length() {
    for source in ["[1][1]", "[][0]"] {
        let program =
            Parser::new(Lexer::new(source.to_owned())).parse().unwrap();
        assert!(evaluate(
            program.to_node(),
            &mut Heap::new(),
            &mut Environment::new()
        )
        .is_err());
    }
}

#[test]
fn garbage_collection_keeps_reachable_values_after_block_rollover() {
    use fipe::object::Int;
    let mut heap = Heap::new();
    let mut env = Environment::new();
    for value in 0..300 {
        heap.enlist(&mut env, "last".to_owned(), Object::Int(Int { value }))
            .unwrap();
    }
    heap.run_gc(&mut env);
    let program = Parser::new(Lexer::new("last".to_owned())).parse().unwrap();
    assert_eq!(
        evaluate(program.to_node(), &mut heap, &mut env).unwrap(),
        Some(Object::Int(Int { value: 299 }))
    );
}
