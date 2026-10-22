//! The std host's declarations load, and every documentation example
//! checks against them.
use fipe::{check, lex::lex, parse::parse};

const GEOMETRY: &str =
    include_str!("../../../Documentation/examples/geometry.fipe");

const EXAMPLES: &[(&str, &str)] = &[
    (
        "01-files",
        include_str!("../../../Documentation/examples/01-files.fipe"),
    ),
    (
        "02-outcomes",
        include_str!("../../../Documentation/examples/02-outcomes.fipe"),
    ),
    (
        "03-types",
        include_str!("../../../Documentation/examples/03-types.fipe"),
    ),
    (
        "04-text",
        include_str!("../../../Documentation/examples/04-text.fipe"),
    ),
    (
        "05-modules",
        include_str!("../../../Documentation/examples/05-modules.fipe"),
    ),
    ("geometry", GEOMETRY),
];

#[test]
fn every_example_checks_against_the_std_declarations() {
    let declarations = fipe_host::declarations();
    let files = [(String::from("geometry"), parse(lex(GEOMETRY)).unwrap())];
    for (name, source) in EXAMPLES {
        let module = parse(lex(source)).unwrap();
        let checked = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        );
        assert!(checked.is_ok(), "{name}: {:?}", checked.err());
    }
}

#[test]
fn the_cli_module_declares_args_and_env_not_constants() {
    let declarations = fipe_host::declarations();
    let cli = declarations.get_module("cli").unwrap();
    assert!(cli.value("args").is_some());
    assert!(cli.value("env").is_some());
    for gone in ["port", "seed", "primes", "rgb"] {
        assert!(cli.value(gone).is_none(), "cli.{gone}");
    }
}

#[test]
fn the_conformance_constants_do_not_check_against_the_std_host() {
    let declarations = fipe_host::declarations();
    let module = parse(lex("use cli\np = cli.port")).unwrap();
    let checked =
        check::check(&module, &declarations, &[], check::Options::default());
    assert!(checked.is_err());
}
