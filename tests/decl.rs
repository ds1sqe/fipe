use fipe::{
    decl::{Declarations, Kind, Namespace, Reason},
    resolve::Unchecked,
    token::Source,
};

fn host() -> Declarations {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("fixtures/host/commands.fipe"))
        .unwrap();
    for (name, text) in [
        ("fs", include_str!("fixtures/host/fs.fipe")),
        ("sys", include_str!("fixtures/host/sys.fipe")),
        ("cli", include_str!("fixtures/host/cli.fipe")),
        ("text", include_str!("fixtures/host/text.fipe")),
    ] {
        declarations.module(name, text).unwrap();
    }
    declarations
}

fn spot_names(kind: &Kind) -> Vec<Option<&str>> {
    let Kind::Function { spots, .. } = kind else {
        panic!("{kind:?}");
    };
    spots.iter().map(|spot| spot.name.as_deref()).collect()
}

#[test]
fn prelude_declares_the_language_names() {
    let prelude = Declarations::new().prelude;
    for name in [
        "show", "read", "to", "try", "into", "from", "map", "filter", "len",
        "join", "fault", "log", "sort", "count", "hypot", "utf8",
    ] {
        assert!(prelude.value(name).is_some(), "{name}");
        assert!(prelude.value(name).unwrap().public, "{name}");
    }
    for name in [
        "Maybe", "Result", "Either", "String", "Bool", "I64", "Vec", "Functor",
        "TryFrom", "Count",
    ] {
        assert!(prelude.ty(name).is_some(), "{name}");
    }
    assert!(matches!(
        &prelude.value("map").unwrap().kind,
        Kind::Method { class, .. } if class == "Functor"
    ));
    assert!(matches!(
        &prelude.ty("I64").unwrap().kind,
        Kind::Type { opaque: true, parameters: 0, constructors } if constructors.is_empty()
    ));
    assert!(matches!(
        &prelude.ty("Maybe").unwrap().kind,
        Kind::Type { opaque: false, parameters: 1, constructors }
            if constructors == &["Just", "Nothing"]
    ));
    assert!(matches!(
        &prelude.ty("String").unwrap().kind,
        Kind::Type { constructors, .. } if constructors == &["String"]
    ));
    let sort = &prelude.value("sort").unwrap().kind;
    assert_eq!(spot_names(sort), [Some("by"), Some("desc"), None]);
    let Kind::Function {
        constraints: 1,
        spots,
        result,
    } = sort
    else {
        panic!("{sort:?}");
    };
    assert!(spots[0].default.is_some() && spots[1].default.is_some());
    assert!(spots[2].default.is_none());
    let text = include_str!("../prelude/prelude.fipe");
    assert_eq!(
        Source::new(text).slice(prelude.module.ty(*result).unwrap().span),
        Some("Vec<a>")
    );
    assert!(matches!(
        prelude.value("show").unwrap().kind,
        Kind::Function { constraints: 1, .. }
    ));
}

#[test]
fn host_declarations_carry_spots_defaults_and_opaque_types() {
    let declarations = host();
    let commands = declarations.commands.as_ref().unwrap();
    let ls = &commands.value("ls").unwrap().kind;
    assert_eq!(
        spot_names(ls),
        [
            Some("path"),
            Some("long"),
            Some("recursive"),
            Some("depth"),
            Some("name")
        ]
    );
    let Kind::Function { spots, result, .. } = ls else {
        panic!()
    };
    assert!(spots.iter().all(|spot| spot.default.is_some()));
    let text = include_str!("fixtures/host/commands.fipe");
    let source = Source::new(text);
    assert_eq!(
        source.slice(commands.module.ty(*result).unwrap().span),
        Some("Vec<Entry>")
    );
    assert_eq!(source.slice(spots[0].span), Some("(path: Src = \".\")"));
    assert!(matches!(
        &commands.ty("Rd").unwrap().kind,
        Kind::Type {
            opaque: true,
            parameters: 1,
            ..
        }
    ));
    assert_eq!(spot_names(&commands.value("Rd").unwrap().kind), [None]);
    assert!(matches!(
        commands.value("out").unwrap().kind,
        Kind::Value { .. }
    ));
    assert!(matches!(
        &commands.ty("Entry").unwrap().kind,
        Kind::Type { constructors, .. } if constructors == &["Entry"]
    ));
    let fs = declarations.get_module("fs").unwrap();
    assert_eq!(
        spot_names(&fs.value("list").unwrap().kind),
        [Some("dir"), Some("path"), Some("long")]
    );
    assert!(fs.ty("Error").unwrap().public);
    let text = declarations.get_module("text").unwrap();
    assert_eq!(
        spot_names(&text.value("wrap").unwrap().kind),
        [Some("width"), None]
    );
    assert!(declarations.get_module("net").is_none());
}

#[test]
fn declaration_files_reject_lines_and_duplicates() {
    let input = "pub x: I64\nls -l\n";
    let errors = Namespace::parse("x", input).unwrap_err();
    assert_eq!(errors[0].reason, Reason::NotDeclaration);
    assert_eq!(Source::new(input).slice(errors[0].span), Some("ls -l"));

    let input = "[a, b] = 1";
    let errors = Namespace::parse("x", input).unwrap_err();
    assert_eq!(errors[0].reason, Reason::NotDeclaration);
    assert_eq!(Source::new(input).slice(errors[0].span), Some("[a, b]"));

    let input = "pub x: I64\ny = 1\npub x: U64";
    let errors = Namespace::parse("x", input).unwrap_err();
    let Reason::Duplicate { first } = errors[0].reason else {
        panic!("{errors:?}");
    };
    assert_eq!(Source::new(input).slice(errors[0].span), Some("x"));
    assert_eq!(first.start, 4);

    // Types and values are separate namespaces: an opaque type's
    // constructor shares its name.
    let namespace =
        Namespace::parse("x", "type Rd<Api>\nRd: U64 -> Rd<api>").unwrap();
    assert!(namespace.ty("Rd").is_some() && namespace.value("Rd").is_some());
    assert!(!namespace.ty("Rd").unwrap().public);

    let errors = Namespace::parse("x", "x: ").unwrap_err();
    assert!(matches!(errors[0].reason, Reason::Syntax(_)));
}

/// 06.1: a declaration file's form the checker cannot check yet is one
/// error that names it.
#[test]
fn declaration_files_report_each_unchecked_form_once() {
    use Unchecked as U;
    for (input, form, spelling) in [
        ("pub f: A & B -> C", U::Both, "A & B"),
        ("pub f: A | B -> C", U::Either, "A | B"),
        ("pub f: !A -> C", U::Not, "!A"),
        ("pub f: A * B -> C", U::Tuple, "A * B"),
        ("pub f: Bits<8> -> C", U::Number, "8"),
        ("pub f: [A; n] -> C", U::Number, "n"),
        ("pub f: b.age: a => a -> b", U::MemberConstraint, "b.age: a"),
        (
            "pub type U8: Natural where self <= 255",
            U::LimitedType,
            "U8",
        ),
        ("type P { x: I64 }\nimpl P { n: I64 }", U::Supplied, "n"),
        ("x = (1, 2)", U::Tuple, "(1, 2)"),
        ("x = 10ms", U::Affix, "10ms"),
        ("x = a ++ b", U::Concat, "++"),
    ] {
        let errors = Namespace::parse("x", input).unwrap_err();
        let found: Vec<_> = errors
            .iter()
            .map(|e| (e.reason.clone(), Source::new(input).slice(e.span)))
            .collect();
        assert_eq!(
            found,
            [(Reason::NotYetChecked { form }, Some(spelling))],
            "{input}"
        );
    }
    // An array's integer length and `b"…"` are checked forms.
    Namespace::parse("x", "pub f: [A; 3] -> C\nx = b\"\\xFF\"").unwrap();
}
