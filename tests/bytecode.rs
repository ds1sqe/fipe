use dlang::test::Tests;

#[test]
fn test_bytecode_compiler_integer() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("5 + 5 + 5 + 5 - 10", 10));

    todo!()
}
