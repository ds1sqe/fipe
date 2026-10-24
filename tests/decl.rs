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
        "show", "read", "to", "try", "map", "filter", "len", "join", "fault",
        "log", "sort", "count", "hypot", "utf8",
    ] {
        assert!(prelude.value(name).is_some(), "{name}");
        assert!(prelude.value(name).unwrap().public, "{name}");
    }
    for name in [
        "Maybe", "Result", "Either", "String", "Bool", "I64", "Vec", "Literal",
        "TryFrom", "Count",
    ] {
        assert!(prelude.ty(name).is_some(), "{name}");
    }
    assert!(matches!(
        &prelude.value("map").unwrap().kind,
        Kind::Function { .. }
    ));
    assert!(matches!(
        &prelude.ty("I64").unwrap().kind,
        Kind::Type { resource: false, opaque: false, parameters: 0, constructors } if constructors.is_empty()
    ));
    assert!(matches!(
        &prelude.ty("Maybe").unwrap().kind,
        Kind::Type { resource: false, opaque: false, parameters: 1, constructors }
            if constructors == &["Just", "Nothing"]
    ));
    assert!(matches!(
        &prelude.ty("String").unwrap().kind,
        Kind::Type { constructors, .. } if constructors == &["String"]
    ));
    let sort = &prelude.value("sort").unwrap().kind;
    assert_eq!(spot_names(sort), [Some("by"), Some("desc"), None]);
    let Kind::Function {
        constraints: 2,
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
    assert_eq!(source.slice(spots[0].span), Some("(path: Src = ./)"));
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

/// A declaration file's form the checker cannot check yet is one
/// error that names it.
#[test]
fn declaration_files_report_each_unchecked_form_once() {
    use Unchecked as U;
    {
        let (input, form, spelling) = ("x = 10ms", U::Affix, "10ms");
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
    for input in [
        "pub f: Bits<n> -> C",
        "pub f: [A; n] -> C",
        "pub f: Bits<n>",
        "pub f: A * B -> C",
        "x = (1, 2)",
        "pub f: A & B -> C",
        "pub f: !A -> C",
        "pub f: !A -> A",
        "pub type U8: Natural where self <= 255",
    ] {
        Namespace::parse("x", input).unwrap();
    }
    Namespace::parse(
        "x",
        "x = a ++ b\npub f: A | B -> C\npub type Key: Eq & Show",
    )
    .unwrap();
    // An array's integer length and `b"…"` are checked forms.
    Namespace::parse("x", "pub f: [A; 3] -> C\nx = b\"\\xFF\"").unwrap();
}

#[test]
fn public_prelude_and_host_declarations_have_docs() {
    use fipe::ast::{Entry, ExprKind, ImplMember, ItemKind, Member};
    let declarations = host();
    for namespace in std::iter::once(&declarations.prelude)
        .chain(declarations.commands.iter())
        .chain(&declarations.modules)
    {
        for entry in &namespace.module.entries {
            match entry {
                Entry::Item(item) => {
                    if item.public {
                        assert!(
                            item.doc.is_some(),
                            "{}: {item:?}",
                            namespace.name
                        );
                    }
                    match &item.kind {
                        ItemKind::Type {
                            members: Some(members),
                            ..
                        } => {
                            for member in members {
                                if let Member::Field(field) = member {
                                    if field.public {
                                        assert!(field.doc.is_some());
                                    }
                                }
                            }
                        }
                        ItemKind::Impl { members, .. } => {
                            for member in members {
                                if let ImplMember::Named {
                                    public: true,
                                    doc,
                                    ..
                                } = member
                                {
                                    assert!(doc.is_some());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Entry::Line(id) => {
                    if let ExprKind::Bind {
                        public: true, doc, ..
                    } = &namespace.module.expr(*id).unwrap().kind
                    {
                        assert!(doc.is_some());
                    }
                }
            }
        }
    }
}

#[test]
fn doc_lists_check_only_declared_spot_names() {
    let source = "## List.\n## - depth: how far down to look\n## * missing: opaque\n## - NotASpot: opaque\n## - two words: opaque\n## -  depth: opaque\npub ls: (depth: U64 = 1) -> String\ntype P\nimpl P {\n## Read.\n## - self: the receiver\nf: (self: Self) -> Self,\n}";
    Namespace::parse("commands", source).unwrap();
    for namespace in ["prelude", "commands", "fs"] {
        let errors = Namespace::parse(
            namespace,
            "## - missing: not a spot\nf: (depth: U64) -> String",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].reason,
            Reason::UnknownDocSpot {
                name: "missing".into()
            }
        );
    }
    let errors = Namespace::parse("host", "type P\nimpl P {\n## - absent: not a spot\nf: (self: Self) -> Self,\n}").unwrap_err();
    assert_eq!(
        errors[0].reason,
        Reason::UnknownDocSpot {
            name: "absent".into()
        }
    );
    // An implemented member is program text, not a supplied signature.
    Namespace::parse("host", "type P { x: Integer }\nimpl P {\n## - opaque: text\nf = self => self,\n}").unwrap();
}

#[test]
fn supplied_claim_blocks_are_validated_at_load() {
    for text in [
        "type V\nimpl V: Unknown {}",
        "type V\nimpl V: Zero {}",
        "type V\nimpl V: Zero { zero: Self, extra: Self }",
        "type V\nimpl V: Zero { zero: Self }\nimpl V: Zero { zero: Self }",
        "type V\nimpl V { zero: Self }\nimpl V: Zero { zero: Self }",
    ] {
        assert!(
            fipe::decl::Namespace::parse("host", text)
                .unwrap_err()
                .iter()
                .any(|e| matches!(e.reason, Reason::ClaimBlock { .. })),
            "{text}"
        );
    }
    assert!(fipe::decl::Namespace::parse(
        "host",
        "type V\nimpl V: Zero { zero: Self }"
    )
    .is_ok());
}

#[test]
fn spot_colon_needs_space_or_end() {
    Namespace::parse("host", "## - https://example.com/manual\n## - missing:text\nf: Integer -> String").unwrap();
    assert!(
        Namespace::parse("host", "## - missing:\nf: Integer -> String")
            .is_err()
    );
}

#[test]
fn bad_supplied_claim_type_is_rejected_at_load() {
    let mut declarations = Declarations::new();
    assert!(declarations
        .module(
            "text",
            "pub type Tok\nimpl Tok: Show { show: (self: Self) -> Integer }"
        )
        .is_err());
}

const COLLIDING_TYPES: &str = "pub type Released { pub keys: Vec<U64> }\npub type Outcome { Released(value: Released), Nothing }";

#[test]
fn declared_constructor_collisions_identify_both_owners_and_spans() {
    for (text, first_type, second_type) in [
        (COLLIDING_TYPES.to_owned(), "Released", "Outcome"),
        ("pub type Outcome { Released(value: Released), Nothing }\npub type Released { pub keys: Vec<U64> }".to_owned(), "Outcome", "Released"),
        (format!("{COLLIDING_TYPES}\npub resolve: U64 -> Outcome = key => Released (Released {{ keys: [key] }})"), "Released", "Outcome"),
    ] {
        let errors = Namespace::parse("fixture", &text).unwrap_err();
        let [error] = errors.as_slice() else { panic!("{errors:?}") };
        let Reason::ConstructorCollision { name, first_type: first_owner, second_type: second_owner, first } = &error.reason else { panic!("{error:?}") };
        assert_eq!(name, "Released");
        assert_eq!(first_owner, first_type);
        assert_eq!(second_owner, second_type);
        assert_eq!(Source::new(&text).slice(*first), Some("Released"));
        assert_eq!(Source::new(&text).slice(error.span), Some("Released"));
        let record = text.find("type Released").unwrap() + "type ".len();
        let variant = text.find("Released(value").unwrap();
        let (first_start, second_start) = if first_type == "Released" {
            (record, variant)
        } else {
            (variant, record)
        };
        assert_eq!((first.start, first.end), (first_start, first_start + "Released".len()));
        assert_eq!((error.span.start, error.span.end), (second_start, second_start + "Released".len()));
        assert!(first.start < error.span.start);
        assert_eq!(error.reason.to_string(), format!("constructor `Released` is declared by both type `{first_type}` and type `{second_type}`"));
    }
    for text in [
        "pub type First { Shared }\npub type Second { Shared }",
        "type First { Shared }\npub type Second { Shared }",
        "pub type First { Shared }\ntype Second { Shared }",
    ] {
        let errors = Namespace::parse("fixture", text).unwrap_err();
        assert!(
            matches!(&errors[0].reason, Reason::ConstructorCollision { name, first_type, second_type, .. } if name == "Shared" && first_type == "First" && second_type == "Second")
        );
    }
}

#[test]
fn constructor_collision_registration_is_atomic() {
    let mut declarations = Declarations::new();
    declarations.module("fixture", "pub old: Integer").unwrap();
    declarations.commands("old: Integer").unwrap();
    let before = declarations.clone();
    for name in ["fixture", "new.child"] {
        let errors = declarations.module(name, COLLIDING_TYPES).unwrap_err();
        assert!(matches!(
            errors[0].reason,
            Reason::ConstructorCollision { .. }
        ));
        assert_eq!(declarations, before);
    }
    let errors = declarations.commands(COLLIDING_TYPES).unwrap_err();
    assert!(matches!(
        errors[0].reason,
        Reason::ConstructorCollision { .. }
    ));
    assert_eq!(declarations, before);
    assert!(declarations.get_module("new").is_none());
}

#[test]
fn constructor_validation_preserves_existing_namespace_policies() {
    let mut declarations = Declarations::new();
    for name in ["one", "two"] {
        declarations
            .module(name, "pub type Item { pub n: Integer }")
            .unwrap();
    }
    Namespace::parse("fixture", "type Rd<Api>\nRd: U64 -> Rd<api>").unwrap();
    // Explicit value precedence is separate existing policy.
    Namespace::parse("fixture", "type Item { n: Integer }\nItem: Integer")
        .unwrap();
    let errors = Namespace::parse(
        "fixture",
        "type Item { n: Integer }\ntype Item { n: Integer }",
    )
    .unwrap_err();
    assert!(errors
        .iter()
        .all(|error| matches!(error.reason, Reason::Duplicate { .. })));
    let source = COLLIDING_TYPES.replace("Released(value", "Release(value");
    declarations.module("fixture", &source).unwrap();
    let client = fipe::parse::parse(fipe::lex::lex("use fixture.Released fixture.Outcome fixture.Release\npub resolve: U64 -> Outcome = key => Release (Released { keys: [key] })")).unwrap();
    fipe::check::check(
        &client,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
}

#[test]
fn resource_registration_is_a_trusted_host_api_and_is_atomic() {
    for name in ["csv", "main", "commands"] {
        let namespace =
            Namespace::parse(name, "pub type resource Table").unwrap();
        assert!(
            matches!(namespace.ty("Table").unwrap().kind, Kind::Type { resource: true, opaque: true, ref constructors, .. } if constructors.is_empty())
        );
        assert!(namespace.value("Table").is_none());
    }
    let mut declarations = Declarations::new();
    declarations.commands("pub type resource Table").unwrap();
    declarations
        .module(
            "text.open",
            "type resource Private\npub type resource Rd<Api>",
        )
        .unwrap();
    assert!(matches!(
        declarations
            .get_module("text.open")
            .unwrap()
            .ty("Rd")
            .unwrap()
            .kind,
        Kind::Type {
            resource: true,
            parameters: 1,
            ..
        }
    ));
    let before = declarations.clone();
    for source in [
        "pub type resource Table: Integer",
        "pub type resource Table {}",
        "pub type resource Table { Make(value: Integer) }",
        "pub type resource Table where self > 0",
    ] {
        let errors = declarations.module("text.open", source).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.reason, Reason::ResourceForm)),
            "{errors:?}"
        );
        assert_eq!(
            &source[errors
                .iter()
                .find(|e| matches!(e.reason, Reason::ResourceForm))
                .unwrap()
                .span
                .start..][..5],
            "Table"
        );
        assert_eq!(declarations, before);
    }
    assert!(Namespace::parse("prelude", "pub type resource Table")
        .unwrap_err()
        .iter()
        .any(|e| matches!(e.reason, Reason::ResourceForm)));
    let namespace = Namespace::parse("plain", "pub type Packet").unwrap();
    assert!(matches!(
        namespace.ty("Packet").unwrap().kind,
        Kind::Type {
            resource: false,
            opaque: true,
            ..
        }
    ));
}

#[test]
fn resource_opaque_claims_need_supplied_operations_and_add_no_constructor() {
    for claim in ["Read", "Default", "Show", "Eq", "Ord"] {
        let source =
            format!("pub type resource Table\nimpl Table: {claim} {{}}");
        assert!(Namespace::parse("csv", &source).is_err(), "{source}");
    }
    Namespace::parse("csv", "pub type resource Table\nimpl Table: Read { read: String -> Result<Self, ParseError> }\nimpl Table: Default { default: Self }\nimpl Table: Show { show: Self -> String }").unwrap();
    // A no-constructor resource cannot collide with this ordinary sum constructor.
    Namespace::parse(
        "csv",
        "pub type resource Table\npub type Other { Table }",
    )
    .unwrap();
}

#[test]
fn claim_qualifiers_cannot_strengthen_a_supplied_contract() {
    for resource in [false, true] {
        for commands in [false, true] {
            for (bound, qualifier, valid) in [
                ("", "Eq T => ", false),
                (": Eq", "Eq T => ", true),
                ("", "", true),
            ] {
                let marker = if resource { "resource " } else { "" };
                let source = format!(
                    "pub type Key where self.inspect: Integer\npub type {marker}Rd<T{bound}>\nimpl Rd<T>: Key {{ inspect: {qualifier}(self: Self) -> Integer }}"
                );
                let mut host = Declarations::new();
                let result = if commands {
                    host.commands(&source)
                } else {
                    host.module("csv", &source)
                };
                assert_eq!(result.is_ok(), valid, "{source}: {result:?}");
                if let Err(errors) = result {
                    assert!(
                        errors.iter().any(|e| matches!(
                            e.reason,
                            Reason::ClaimBlock { .. }
                        )),
                        "{errors:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn claim_qualifiers_required_and_inherited_contracts_are_assumptions() {
    for declaration in [
        "pub type Key<T: Eq> where self.inspect: Integer\npub type Rd<T>\nimpl Rd<T>: Key<T> { inspect: Eq T => (self: Self) -> Integer }",
        "pub type Base<T: Eq> where self.inspect: Integer\npub type Key<T: Eq>: Base<T>\npub type Rd<T>\nimpl Rd<T>: Key<T> { inspect: Eq T => (self: Self) -> Integer }",
        "pub type Key where self.inspect: Integer\npub type Rd<T: Eq & Show>\nimpl Rd<T>: Key { inspect: Eq T, Show T => (self: Self) -> Integer }",
    ] {
        let mut host = Declarations::new();
        host.module("csv", declaration)
            .unwrap_or_else(|e| panic!("{declaration}: {e:?}"));
    }
    let declaration = "pub type Key where self.inspect: Integer\npub type Rd<T: Eq>\nimpl Rd<T>: Key { inspect: Eq T, Show T => (self: Self) -> Integer }";
    assert!(Declarations::new().module("csv", declaration).is_err());
}

#[test]
fn claim_qualifiers_cannot_choose_a_concrete_universal_receiver() {
    let source = "pub type Key where self.inspect: Integer\npub type Rd<T>\nimpl Rd<T>: Key { inspect: (self: Rd<Integer>) -> Integer }";
    assert!(Declarations::new().module("csv", source).is_err());
}

#[test]
fn generic_claim_owner_original_bodyless_block_loads() {
    let source = "pub type Probe where self.inspect: Integer\npub type Box<T>: Probe { pub value: T }\nimpl Box<T>: Probe { inspect: (self: Self) -> Integer }";
    Namespace::parse("probe", source).unwrap();
}

const GENERIC_COPY_PROBE: &str = "pub type Probe where self.copy: Self\npub type Local\npub type Box<T>: Probe { pub value: T }";

fn generic_claim_owner_refused(source: &str, expected: &str) {
    let errors = Namespace::parse("probe", source).unwrap_err();
    assert!(errors.iter().any(|error|matches!(&error.reason,Reason::ClaimBlock{problem}if problem==expected)),"{source}: {errors:?}");
}

#[test]
fn generic_claim_owner_direct_binders_follow_existing_scope_rules() {
    for name in ["T", "a", "U", "Integer", "Local"] {
        let source = format!(
            "{GENERIC_COPY_PROBE}\nimpl Box<{name}>: Probe {{ copy: (self: Self) -> Self }}"
        );
        Namespace::parse("probe", &source)
            .unwrap_or_else(|errors| panic!("{name}: {errors:?}"));
    }
    Namespace::parse("probe",&format!("{GENERIC_COPY_PROBE}\nimpl Box: Probe {{ copy: (self: Self) -> Self }}")).unwrap();
}

#[test]
fn generic_claim_owner_duplicates_and_plain_overlap_are_contextual() {
    for second in ["Box<b>", "Box"] {
        generic_claim_owner_refused(
            &format!(
                "{GENERIC_COPY_PROBE}\nimpl Box<a>: Probe {{ copy: (self: Self) -> Self }}\nimpl {second}: Probe {{ copy: (self: Self) -> Self }}"
            ),
            "second block for this claim",
        );
    }
    generic_claim_owner_refused(
        &format!(
            "{GENERIC_COPY_PROBE}\nimpl Box<a> {{ copy: (self: Self) -> Self }}\nimpl Box<b>: Probe {{ copy: (self: Self) -> Self }}"
        ),
        "member occurs in a plain impl and a claim",
    );
}

#[test]
fn generic_claim_owner_positions_keep_swapped_and_repeated_constraints_distinct(
) {
    let prefix = "pub type Probe<A> where self.inspect: A\npub type Pair<T,U>: Probe<T> { pub left: T, pub right: U }";
    for (owner, result) in [("Pair<a,b>", "a"), ("Pair<b,a>", "b")] {
        let source = format!(
            "{prefix}\nimpl {owner}: Probe<{result}> {{ inspect: (self: Self) -> {result} }}"
        );
        Namespace::parse("probe", &source)
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    }
    generic_claim_owner_refused(
        &format!(
            "{prefix}\nimpl Pair<b,a>: Probe<a> {{ inspect: (self: Self) -> a }}"
        ),
        "claim is not listed on the type",
    );
    generic_claim_owner_refused(
        &format!(
            "{prefix}\nimpl Pair<a,a>: Probe<a> {{ inspect: (self: Self) -> a }}"
        ),
        "listed claim has no block or derived member",
    );
}

#[test]
fn generic_claim_owner_real_specialization_does_not_cover_a_blanket_promise() {
    for target in ["Box<Vec<Integer>>", "Box<[Integer;2]>", "Box<()>"] {
        generic_claim_owner_refused(
            &format!(
                "{GENERIC_COPY_PROBE}\nimpl {target}: Probe {{ copy: (self: Self) -> Self }}"
            ),
            "listed claim has no block or derived member",
        );
    }
    let mut declarations = Declarations::new();
    declarations.module("values", "pub type Concrete").unwrap();
    let source = format!(
        "{GENERIC_COPY_PROBE}\nimpl Box<values.Concrete>: Probe {{ copy: (self: Self) -> Self }}"
    );
    let errors = declarations.module("probe", &source).unwrap_err();
    assert!(errors.iter().any(|error|matches!(&error.reason,Reason::ClaimBlock{problem}if problem=="listed claim has no block or derived member")),"{errors:?}");
    generic_claim_owner_refused(
        "pub type Probe where self.copy: Self\npub type Size<n>: Probe { pub value: Integer }\nimpl Size<2>: Probe { copy: (self: Self) -> Self }",
        "listed claim has no block or derived member",
    );
}

#[test]
fn generic_claim_owner_class_arguments_and_inherited_formals_do_not_capture() {
    let prefix = "pub type Probe<A> where self.inspect: A\npub type Derived<T>: Probe<T>\npub type Box<T>: Derived<Integer> { pub value: T }";
    Namespace::parse("probe",&format!("{prefix}\nimpl Box<a>: Probe<Integer> {{ inspect: (self: Self) -> Integer }}")).unwrap();
    generic_claim_owner_refused(
        &format!(
            "{prefix}\nimpl Box<a>: Probe<a> {{ inspect: (self: Self) -> a }}"
        ),
        "claim is not listed on the type",
    );
    Namespace::parse("probe","pub type Box<T>\nimpl Box<a>: From<Integer> { from: Integer -> Self }\nimpl Box<b>: From<String> { from: String -> Self }").unwrap();
}

#[test]
fn generic_claim_owner_expanding_invalid_class_cycles_stay_finite() {
    assert!(Namespace::parse("probe","pub type Loop<T>: Loop<Vec<T>>\npub type Box<T>: Loop<T> { pub value: T }\nimpl Box<a>: Loop<a> {}").is_err());
}

#[test]
fn generic_claim_owner_structural_specializations_keep_their_arguments() {
    let prefix = "pub type Probe<A> where self.inspect: A\npub type Box<T>: Probe<T> { pub value: T }\nimpl Box<a>: Probe<a> { inspect: (self: Self) -> a }";
    for argument in [
        "(Integer * String)",
        "((Integer -> String) * Integer)",
        "(Integer | String)",
    ] {
        let lawful = format!(
            "{prefix}\nimpl Box<{argument}>: Probe<{argument}> {{ inspect: (self: Self) -> {argument} }}"
        );
        Namespace::parse("probe", &lawful)
            .unwrap_or_else(|errors| panic!("{lawful}: {errors:?}"));
        generic_claim_owner_refused(
            &format!(
                "{prefix}\nimpl Box<{argument}>: Probe<Integer> {{ inspect: (self: Self) -> Integer }}"
            ),
            "claim is not listed on the type",
        );
    }
}
