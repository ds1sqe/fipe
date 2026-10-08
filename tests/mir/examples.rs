use fipe::{check, compile, decl::Declarations, lex::lex, parse::parse};

/// Every documentation example and every host fixture lowers to a
/// program the walker accepts; every entered entry is an `Eval` or
/// `Init` one, so no body holds a `Compile` entry (the checker's
/// compile phase owns those). An entry attached to a definition nothing reaches has
/// no statement, as the definition has no body.
#[test]
fn documentation_examples_lower_and_walk() {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    for (name, source) in [
        ("fs", include_str!("../fixtures/host/fs.fipe")),
        ("sys", include_str!("../fixtures/host/sys.fipe")),
        ("cli", include_str!("../fixtures/host/cli.fipe")),
        ("text", include_str!("../fixtures/host/text.fipe")),
    ] {
        declarations.module(name, source).unwrap();
    }
    let geometry = parse(lex(include_str!(
        "../../Documentation/examples/geometry.fipe"
    )))
    .unwrap();
    let files = [(String::from("geometry"), geometry)];
    for (name, source) in [
        (
            "01-files",
            include_str!("../../Documentation/examples/01-files.fipe"),
        ),
        (
            "02-outcomes",
            include_str!("../../Documentation/examples/02-outcomes.fipe"),
        ),
        (
            "03-types",
            include_str!("../../Documentation/examples/03-types.fipe"),
        ),
        (
            "04-text",
            include_str!("../../Documentation/examples/04-text.fipe"),
        ),
        (
            "05-modules",
            include_str!("../../Documentation/examples/05-modules.fipe"),
        ),
        (
            "geometry",
            include_str!("../../Documentation/examples/geometry.fipe"),
        ),
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .unwrap_or_else(|errors| panic!("{name}: {errors:?}"));
        let program = compile::compile(&unit);
        super::walk::check_program(&program);
        let tables = unit.tables();
        let entered: Vec<usize> = program
            .bodies
            .iter()
            .flat_map(|body| &body.blocks)
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement {
                fipe::mir::Statement::Enter { entry, .. } => Some(*entry),
                _ => None,
            })
            .collect();
        for entry in &entered {
            let entry = &tables.entries[*entry];
            assert_ne!(
                entry.phase,
                fipe::ir::Phase::Compile,
                "{name}: {entry:?}"
            );
        }
        assert!(
            program.bodies.len() > 1 || name == "geometry",
            "{name} reaches a body beyond its main"
        );
    }
}
