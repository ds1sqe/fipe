use fipe::{
    ast::{Entry, ExprKind, ImplMember, ItemKind, Module},
    decl::Declarations,
    infer::{infer, infer_partial, Error, Reason},
    lex::lex,
    parse::parse,
    resolve::resolve,
    resolve::Origin,
    token::Source,
    typed::Typed,
    types::{Obligation, Type},
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

fn module(text: &str) -> Module {
    parse(lex(text)).unwrap_or_else(|errors| panic!("{text:?}: {errors:?}"))
}

fn run(text: &str) -> (Module, Result<Typed, Vec<Error>>) {
    let parsed = module(text);
    let resolved = resolve(&parsed, &host(), &[])
        .unwrap_or_else(|errors| panic!("{text:?}: {errors:?}"));
    let typed = infer(&parsed, &resolved, &host(), &[]);
    (parsed, typed)
}

/// The type of the last line: its value, or its binding's type.
fn ty(text: &str) -> String {
    let (parsed, typed) = run(text);
    let typed = typed.unwrap_or_else(|errors| {
        panic!("{text:?}: {:?}", located(text, &errors))
    });
    let Some(Entry::Line(id)) = parsed.entries.last() else {
        panic!("{text:?}: no line")
    };
    let id = match parsed.expr(*id).unwrap().kind {
        ExprKind::Bind { .. } => {
            return typed.binding(*id).unwrap().to_string()
        }
        _ => *id,
    };
    typed.expr(id).unwrap().to_string()
}

fn located<'a>(text: &'a str, errors: &[Error]) -> Vec<(Reason, &'a str)> {
    errors
        .iter()
        .map(|e| {
            let slice = Source::new(text).slice(e.span).unwrap_or("?");
            (e.reason.clone(), slice)
        })
        .collect()
}

fn errors(text: &str) -> Vec<Error> {
    run(text).1.expect_err(text)
}

/// Each error's reason, without types, and its spelling.
fn kinds(text: &str) -> Vec<(&'static str, &str)> {
    errors(text)
        .iter()
        .map(|e| {
            let slice = Source::new(text).slice(e.span).unwrap_or("?");
            (kind(&e.reason), slice)
        })
        .collect()
}

const fn kind(reason: &Reason) -> &'static str {
    match reason {
        Reason::Mismatch { .. } => "Mismatch",
        Reason::Occurs => "Occurs",
        Reason::CallReading { .. } => "CallReading",
        Reason::TypeDepth => "TypeDepth",
        Reason::NothingPayload => "NothingPayload",
        Reason::OutcomeText { .. } => "OutcomeText",
        Reason::UnknownFamily => "UnknownFamily",
        Reason::ClaimBlock { .. } => "ClaimBlock",
        Reason::MissingClaim { .. } => "MissingClaim",
        Reason::MissingMember { .. } => "MissingMember",
        Reason::NoField { .. } => "NoField",
        Reason::NoCommonMeaning { .. } => "NoCommonMeaning",
        Reason::Conversion { .. } => "Conversion",
        Reason::ConflictingConversion => "ConflictingConversion",
        Reason::NotAFunction => "NotAFunction",
        Reason::Spots { .. } => "Spots",
        Reason::DeclarationValue { .. } => "DeclarationValue",
        Reason::Default { .. } => "Default",
        Reason::NonBoolChaining => "NonBoolChaining",
        Reason::ExtraArgument => "ExtraArgument",
        Reason::PositionalBool => "PositionalBool",
        Reason::LiteralApplied => "LiteralApplied",
        Reason::IfWithoutElse => "IfWithoutElse",
        Reason::UnknownSpot { .. } => "UnknownSpot",
        Reason::FlagOperand { .. } => "FlagOperand",
        Reason::DuplicateSupply { .. } => "DuplicateSupply",
        Reason::FilledSpot => "FilledSpot",
        Reason::OpenSpot => "OpenSpot",
        Reason::NeedsMaybeOrResult => "NeedsMaybeOrResult",
        Reason::NeedsOutcome => "NeedsOutcome",
        Reason::NotYetChecked { .. } => "NotYetChecked",
        Reason::Discarded => "Discarded",
        Reason::MissingField { .. } => "MissingField",
        Reason::Limit { .. } => "Limit",
        Reason::OutOfRange => "OutOfRange",
        Reason::NotExact { .. } => "NotExact",
        Reason::NoBracketForm { .. } => "NoBracketForm",
        Reason::FamilyClash { .. } => "FamilyClash",
        Reason::NoPayload { .. } => "NoPayload",
        Reason::MissingPayload { .. } => "MissingPayload",
        Reason::Ambiguous { .. } => "Ambiguous",
        Reason::ImportCycle => "ImportCycle",
        Reason::PrivateMember { .. } => "PrivateMember",
        Reason::PrivateField { .. } => "PrivateField",
        Reason::NoConstructor { .. } => "NoConstructor",
    }
}

fn examples() -> Vec<(String, Module)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fipe"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
            (stem, module(&std::fs::read_to_string(&path).unwrap()))
        })
        .collect()
}

/// Every error of an example: (line, reason, spelling).
fn example(stem: &str) -> Vec<(usize, &'static str, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples")
        .join(format!("{stem}.fipe"));
    let text = std::fs::read_to_string(path).unwrap();
    let parsed = module(&text);
    let files = examples();
    let resolved = resolve(&parsed, &host(), &files).unwrap();
    let source = Source::new(&text);
    match infer(&parsed, &resolved, &host(), &files) {
        Ok(_) => Vec::new(),
        Err(errors) => errors
            .iter()
            .map(|e| {
                let line = source.line_col(e.span.start).0;
                let slice = source.slice(e.span).unwrap_or("?").to_string();
                (line, kind(&e.reason), slice)
            })
            .collect(),
    }
}

#[test]
fn literals_and_arithmetic() {
    assert_eq!(ty("1 + 2"), "Integer");
    assert_eq!(ty("x: U8 = 1 + 2"), "U8");
    let text = "1 + \"a\"";
    let [error] = errors(text).try_into().unwrap();
    assert_eq!(Source::new(text).slice(error.span), Some(text));
    assert_eq!(
        error.reason,
        Reason::NoCommonMeaning {
            forms: vec!["Whole".into(), "Text".into()]
        }
    );
    assert_eq!(kinds("\"a\" + \"b\""), [("MissingClaim", "+")]);
}

#[test]
fn arms_share_their_input_and_result_types() {
    let text = "max = a b if a > b => a, _ b => b";
    let (parsed, typed) = run(text);
    let typed = typed.unwrap();
    let Some(Entry::Line(id)) = parsed.entries.last() else {
        panic!()
    };
    let scheme = typed.binding(*id).unwrap();
    assert_eq!(scheme.to_string(), "Ord a => a -> a -> a");
    let Type::Function { parameter, result } = &scheme.ty else {
        panic!("{}", scheme.ty)
    };
    let a = (**parameter).clone();
    let Type::Var(var) = a else { panic!("{a}") };
    assert_eq!(scheme.variables, [var]);
    assert_eq!(**result, Type::function(a.clone(), a.clone()));
    assert_eq!(scheme.obligations.len(), 1);
    assert!(scheme.obligations.iter().any(|o| matches!(o, Obligation::Claim { class: Type::Named { name, .. }, head, .. } if name.name == "Ord" && head == &a)));
    assert_eq!(typed.unsolved, Vec::<Obligation>::new());
    // A dot on an unknown head is a member obligation in the scheme.
    assert_eq!(ty("f = x => x.size"), "a.size: b => a -> b");
}

#[test]
fn spots_bind_before_application() {
    assert_eq!(
        ty("ls -l /var/log | filter (e => e.size > 1) | map .name"),
        "Vec<String>"
    );
    assert_eq!(ty("add = a b => a + b\nadd 2"), "Integer -> Integer");
    assert_eq!(ty("ls | sort -by .size -desc"), "Vec<Entry>");
    assert_eq!(ty("ls -r -depth 2 /etc -name \"*.conf\""), "Vec<Entry>");
    let point = "type Point { x: I64, y: I64 = 0 }\n";
    assert_eq!(ty(&format!("{point}Point 3")), "Point");
    assert_eq!(ty(&format!("{point}Point -y 1 -x 2")), "Point");
    assert_eq!(ty(&format!("{point}Point")), "I64 -> Point");
    let text = format!("{point}Point 3 4 5");
    assert_eq!(kinds(&text), [("ExtraArgument", "5")]);
    assert_eq!(kinds("ls | cat x"), [("FilledSpot", "cat x")]);
    assert_eq!(kinds("ls -nope"), [("UnknownSpot", "-nope")]);
    assert_eq!(ty("ls -lr"), "Vec<Entry>");
    assert_eq!(ty("Just"), "a -> Maybe<a>");
    assert_eq!(ty("f = x => x\nf 1"), "Integer");
}

#[test]
fn words_take_a_read_spot_type() {
    assert_eq!(ty("cat foo"), "String");
    assert_eq!(ty("find ./x"), "Maybe<Entry>");
    assert_eq!(ty("port: U16 = \"8080\""), "U16");
}

#[test]
fn outcomes_follow_their_family() {
    assert_eq!(ty("find x |: cat"), "Maybe<String>");
    assert_eq!(ty("Just 1 |: (x => x + 1)"), "Maybe<Integer>");
    assert_eq!(ty("find a && rm a"), "Maybe<Result<(), Error>>");
    assert_eq!(ty("x: Result<I64, String> = Ok 1\nx || 0"), "I64");
    assert_eq!(ty("m: Maybe<I64> = [:1]"), "Maybe<I64>");
    assert_eq!(
        ty("g = p => {\n    [:x] = find p\n    x | cat\n}"),
        "Src -> Maybe<String>"
    );
    assert_eq!(kinds("m = [:1]"), [("UnknownFamily", "[:1]")]);
    assert_eq!(kinds("m: Maybe<I64> = [!1]"), [("NothingPayload", "[!1]")]);
    let either = "e: Either<(), I64> = Right 1\n";
    assert_eq!(
        ty(&format!("{either}e |: (x => x + 1)")),
        "Either<(), Integer>"
    );
    let text = format!("{either}e || 0");
    assert_eq!(kinds(&text), [("NeedsMaybeOrResult", "||")]);
    assert_eq!(
        kinds("[1, 2, 3] |: (x => Just x)"),
        [("NeedsOutcome", "|:")]
    );
    // `|!` on `Maybe` supplies nothing: its stage is a value, and a
    // plain one is discarded for `Nothing`.
    assert_eq!(ty("find x |: cat |! log \"no config\""), "Maybe<String>");
    assert_eq!(ty("find x |! find y"), "Maybe<Entry>");
    assert_eq!(kinds("find x |! log"), [("OpenSpot", "log")]);
    assert_eq!(ty("find x |! log \"x\" || \"d\""), "Entry");
    // On `Result`, `|!` still passes the error payload.
    let result = "r: Result<I64, String> = Ok 1\n";
    assert_eq!(ty(&format!("{result}r |! (e => e)")), "Result<I64, String>");
}

#[test]
fn dots_read_fields_then_self_functions() {
    assert_eq!(ty("e = ls | head\ne.name"), "String");
    let text = "e = ls | head\ne.nope";
    let [error] = errors(text).try_into().unwrap();
    let Reason::NoField { ty: head, .. } = error.reason else {
        panic!()
    };
    assert_eq!(head, Type::named(Origin::Commands, "Entry", Vec::new()));
    let point = "type Point { x: I64, y: I64 = 0 }\n\
        impl Point { dist: Point -> Point -> I64 = self q => self.x - q.x }\n";
    assert_eq!(ty(&format!("{point}p = Point 1\np.dist p")), "I64");
    assert_eq!(ty("\"3\" | read<I64>"), "Result<I64, ParseError>");
}

/// A line of main generalizes nothing: its variables leave there and
/// default if they can, and a trait obligation still on a variable is
/// ambiguous even when the variable is in the line's type.
#[test]
fn a_line_whose_target_is_unknown_is_ambiguous() {
    let text = "read \"8080\"";
    let [error] = errors(text).try_into().unwrap();
    assert_eq!(
        error.reason,
        Reason::Ambiguous {
            class: "Read".into()
        }
    );
    assert_eq!(Source::new(text).slice(error.span), Some("read"));
    assert_eq!(kinds("x => x == x"), [("Ambiguous", "==")]);
    // A binding generalizes instead; a known target resolves it.
    assert_eq!(
        ty("port = read \"8080\""),
        "Read a => Result<a, ParseError>"
    );
    assert_eq!(ty("read<U16> \"8080\""), "Result<U16, ParseError>");
    // `Add` alone defaults where the variable leaves the line.
    assert_eq!(kinds("add = a b => a + b\nadd"), [("Ambiguous", "+")]);
    assert_eq!(ty("Just"), "a -> Maybe<a>");
}

/// Every example checks without exemptions.
#[test]
fn the_examples_type() {
    for (stem, _) in examples() {
        assert_eq!(example(&stem), [], "{stem}");
    }
}

/// A comment `# name: type` directly above a binding or an impl member
/// states the type inference derives for it; members that failed to
/// type are skipped. An impl member is monomorphic: its value's type.
#[test]
fn the_example_comments_state_the_inferred_types() {
    let files = examples();
    let mut checked = Vec::new();
    for (stem, parsed) in &files {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Documentation/examples")
            .join(format!("{stem}.fipe"));
        let text = std::fs::read_to_string(path).unwrap();
        let source = Source::new(&text);
        let resolved = resolve(parsed, &host(), &files).unwrap();
        let (typed, errors) = infer_partial(parsed, &resolved, &host(), &files);
        let failed = |span: fipe::token::Span| {
            errors
                .iter()
                .any(|e| span.start <= e.span.start && e.span.end <= span.end)
        };
        // Line of each binding's name: its type, or `None` if it failed.
        let mut bindings = std::collections::BTreeMap::new();
        for entry in &parsed.entries {
            match entry {
                Entry::Line(id) => {
                    let expr = parsed.expr(*id).unwrap();
                    if !matches!(expr.kind, ExprKind::Bind { .. }) {
                        continue;
                    }
                    let scheme =
                        typed.binding(*id).filter(|_| !failed(expr.span));
                    let line = source.line_col(expr.span.start).0;
                    bindings.insert(line, scheme.map(ToString::to_string));
                }
                Entry::Item(item) => {
                    let ItemKind::Impl { members, .. } = &item.kind else {
                        continue;
                    };
                    for member in members {
                        let ImplMember::Named {
                            name,
                            value: Some(value),
                            span,
                            ..
                        } = member
                        else {
                            continue;
                        };
                        let ty = typed.expr(*value).filter(|_| !failed(*span));
                        let line = source.line_col(name.span.start).0;
                        bindings.insert(line, ty.map(ToString::to_string));
                    }
                }
            }
        }
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let Some((name, ty)) = line
                .trim()
                .strip_prefix("# ")
                .and_then(|c| c.split_once(": "))
            else {
                continue;
            };
            if !name.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
            }) {
                continue;
            }
            let Some(next) =
                (index + 1..lines.len()).find(|n| !lines[*n].trim().is_empty())
            else {
                continue;
            };
            if !lines[next].trim().starts_with(&format!("{name} =")) {
                continue;
            }
            let derived = bindings
                .get(&(next + 1))
                .unwrap_or_else(|| panic!("{stem}:{}: no binding", next + 1));
            match derived {
                Some(derived) => {
                    assert_eq!(derived, ty, "{stem}:{}", index + 1);
                    checked.push(name.to_string());
                }
                None => checked.push(format!("({name})")),
            }
        }
    }
    assert_eq!(
        checked,
        ["older", "load", "read_port", "scale", "greet", "check"]
    );
}

#[path = "infer/generalize.rs"]
mod generalize;

#[path = "infer/members.rs"]
mod members;

#[test]
fn long_marked_and_fallback_chains_type_without_deep_recursion() {
    // Flat chains nest to the left; each element must not take a frame.
    let marked = "|: f ".repeat(5_000);
    let text = format!("f = x => x + 1\nm = Just 1\nm {marked}");
    assert_eq!(ty(&text), "Maybe<Integer>");
    let fallback = "|| False ".repeat(20_000);
    assert_eq!(ty(&format!("True {fallback}")), "Bool");
    let and = "&& True ".repeat(20_000);
    assert_eq!(ty(&format!("True {and}")), "Bool");
}

const PRIMES: &str = "pub type Prime { pub n: U64 }
impl Prime {
    pub check: Prime -> Bool = self => self.n > 1,
    hidden: Prime -> Bool = self => True,
}
pub next: Prime -> Prime = p => Prime { n: p.n + 1 }
";

/// `text` typed with the file module `primes` beside it.
fn with_primes(text: &str) -> Result<Typed, Vec<Error>> {
    let parsed = module(text);
    let files = [("primes".to_string(), module(PRIMES))];
    let resolved = resolve(&parsed, &host(), &files)
        .unwrap_or_else(|errors| panic!("{text:?}: {errors:?}"));
    infer(&parsed, &resolved, &host(), &files)
}

/// The type of the last line of `text` with `primes`, as [`ty`].
fn primes_ty(text: &str) -> String {
    let parsed = module(text);
    let typed = with_primes(text).unwrap_or_else(|errors| {
        panic!("{text:?}: {:?}", located(text, &errors))
    });
    let Some(Entry::Line(id)) = parsed.entries.last() else {
        panic!("{text:?}: no line")
    };
    match parsed.expr(*id).unwrap().kind {
        ExprKind::Bind { .. } => typed.binding(*id).unwrap().to_string(),
        _ => typed.expr(*id).unwrap().to_string(),
    }
}

#[test]
fn a_file_module_s_types_keep_their_identity_in_its_importer() {
    // The module's own function on the module's own type.
    let twice = "use primes.Prime primes.next
pub twice: Prime -> Prime = p => next (next p)";
    assert_eq!(primes_ty(twice), "Prime -> Prime");
    // An importer's type of the same name is another type.
    let text = "use primes.next\ntype Prime { s: String }\nnext (Prime \"x\")";
    let [error] = with_primes(text).expect_err(text).try_into().unwrap();
    assert_eq!(Source::new(text).slice(error.span), Some("Prime \"x\""));
    let Reason::Mismatch {
        expected, actual, ..
    } = error.reason
    else {
        panic!("{:?}", error.reason)
    };
    let origin = |ty: &Type| match ty {
        Type::Named { name, .. } => name.origin.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(origin(&expected), Origin::File("primes".into()));
    assert_eq!(origin(&actual), Origin::This);
    // The message says which module each is from.
    assert_eq!(expected.beside(&actual).to_string(), "primes.Prime");
    assert_eq!(actual.beside(&expected).to_string(), "Prime");
    assert_eq!(expected.to_string(), "Prime");
}

#[test]
fn an_imported_type_s_members_resolve_through_its_module() {
    assert_eq!(primes_ty("use primes.Prime\nPrime.check"), "Prime -> Bool");
    assert_eq!(primes_ty("use primes\nprimes.Prime.check"), "Prime -> Bool");
    assert_eq!(primes_ty("use primes.Prime\n(Prime 7).check"), "Bool");
    assert_eq!(primes_ty("use primes.Prime\nPrime 7 | .check"), "Bool");
}

#[test]
fn a_private_member_reached_as_a_value_method_is_named() {
    // `T.a`/`m.T.a` are caught by resolve; `value.a` reaches infer.
    let text = "use primes.Prime\n(Prime 7).hidden";
    let [error] = with_primes(text).expect_err(text).try_into().unwrap();
    assert_eq!(
        Source::new(text).slice(error.span),
        Some("(Prime 7).hidden")
    );
    assert_eq!(
        error.reason,
        Reason::PrivateMember {
            member: "hidden".into()
        }
    );
    let text = "use primes.Prime\nPrime 7 | .hidden";
    let [error] = with_primes(text).expect_err(text).try_into().unwrap();
    assert_eq!(
        error.reason,
        Reason::PrivateMember {
            member: "hidden".into()
        }
    );
    // A member that does not exist at all is still `NoField`.
    let text = "use primes.Prime\n(Prime 7).nope";
    let [error] = with_primes(text).expect_err(text).try_into().unwrap();
    assert!(matches!(error.reason, Reason::NoField { .. }), "{error:?}");
}

#[test]
fn one_mismatch_is_reported_once() {
    let text = "use primes.Prime primes.next\np = Prime 7\nnext (p.n + 2)";
    let errors = with_primes(text).expect_err(text);
    let located: Vec<_> = errors
        .iter()
        .map(|e| (kind(&e.reason), Source::new(text).slice(e.span)))
        .collect();
    assert_eq!(located, [("Mismatch", Some("p.n + 2"))]);
    let text = "f: String -> String = s => s\nx: U64 = 1\nf (x + 2 * 3)";
    assert_eq!(ty(text), "String");
}

#[test]
fn literal_operands_take_their_type_in_any_order() {
    for text in ["2 + 2.5", "2.5 + 2", "2 + 2 + 2.5", "x = 2.5\n2 + x"] {
        assert_eq!(ty(text), "F64", "{text}");
    }
    assert_eq!(ty("x = 2 * 2.5"), "F64");
    assert_eq!(ty("y: U8 = 3\n2 + y"), "Integer");
    assert_eq!(ty("2 < 2.5"), "Bool");
    assert_eq!(ty("1 + 1"), "Integer");
    assert_eq!(ty("y: U8 = 3\n300 + y"), "Integer");
    // A fraction at an integer operand's type must be an integer.
    assert_eq!(kinds("y: U8 = 3\n2.5 + y"), [("NotExact", "2.5")]);
    assert_eq!(ty("y: U8 = 3\n2.0 + y"), "Integer");
}

#[test]
fn the_neutral_conversion_example_converts_a_point() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples/03-types.fipe");
    let text = std::fs::read_to_string(path).unwrap();
    let parsed = module(&text);
    let files = examples();
    let resolved = resolve(&parsed, &host(), &files).unwrap();
    let (typed, _) = infer_partial(&parsed, &resolved, &host(), &files);
    let source = Source::new(&text);
    let conversion = parsed.entries.iter().find_map(|entry| {
        let Entry::Line(id) = entry else { return None };
        let expr = parsed.expr(*id).unwrap();
        let spelled = source.slice(expr.span)?;
        spelled.ends_with("to<Polar>").then_some(expr)
    });
    let ExprKind::Pipe { left, .. } = conversion.unwrap().kind else {
        panic!("not a pipe")
    };
    // From `Point`, through `Polar`'s `From<Point>`, not `Polar` to itself.
    assert_eq!(typed.expr(left).unwrap().to_string(), "Point");
}

#[path = "infer/conversions.rs"]
mod conversions;
#[path = "infer/literals.rs"]
mod literals;

#[path = "infer/claims.rs"]
mod claims;

#[path = "infer/claim_cases.rs"]
mod claim_cases;

#[path = "infer/claim_rules.rs"]
mod claim_rules;

#[test]
fn operands_past_the_spot_list_apply_to_a_function_result() {
    assert_eq!(ty("g = a => { b => a + b }\ng 1 2"), "Integer");
    assert_eq!(
        ty("h: Integer -> Integer -> Integer = a => { b => a + b }\nh 1 2"),
        "Integer"
    );
    let compose = "compose = f g => { x => f (g x) }\ninc = n => n + 1\n";
    assert_eq!(ty(&format!("{compose}compose inc inc 1")), "Integer");
    assert_eq!(
        ty("twice = f => { x => f (f x) }\ntwice (n => n + 1) 3"),
        "Integer"
    );
    // A result that is no function, or a spot still open, keeps the error.
    assert_eq!(kinds("inc = n => n + 1\ninc 1 2"), [("ExtraArgument", "2")]);
    assert_eq!(
        kinds("t = { True => \"y\", False => \"n\" }\nt True"),
        [("PositionalBool", "True")]
    );
    assert_eq!(kinds("2 ^ 10"), [("LiteralApplied", "^")]);
    assert_eq!(kinds("if True { 1 }"), [("IfWithoutElse", "{ 1 }")]);
    assert_eq!(
        kinds("app = f x => f x\ninc = n => n + 1\napp inc 1 2"),
        [("ExtraArgument", "2")]
    );
    let flags = kinds("type M { a: Bool, b: Bool }\nM True False");
    assert_eq!(flags.len(), 2, "{flags:?}");
    assert!(flags.iter().all(|(kind, _)| *kind == "ExtraArgument"));
}

#[test]
fn a_limited_type_over_a_claimed_base_is_a_place_of_that_meaning() {
    let shape = "type Shape where self.area: F64\ntype C: Shape { r: F64 }\nimpl C: Shape { area = self => self.r }\ntype Named: Shape where 0.0 < self.area\n";
    assert_eq!(ty(&format!("{shape}nn: Named = C 1.0")), "Named");
    assert_eq!(ty("type IS: Integer | String\nq: IS = 3"), "IS");
    assert_eq!(
        ty("type Pos: Integer where self > 0\ntype PosS: Pos | String\npp: PosS = 5"),
        "PosS"
    );
    // Without a test the base names a combination the type claims.
    assert_eq!(ty("type RS: Read & Show\nu: RS = 5"), "RS");
}

#[test]
fn a_declared_default_s_literal_chooses_with_the_body() {
    assert_eq!(ty("g = (n = 1) => n + 1.5\ng"), "F64");
    assert_eq!(ty("g = (n = 1) => n + 1.5\ng 2"), "F64");
    assert_eq!(ty("f = (a = 1) (b = 2.5) => a + b\nf"), "F64");
    assert_eq!(ty("f = (a = 1) (b = 2.5) => a + b\nf -b 2"), "F64");
    assert_eq!(ty("h = (n = 1) => n + 1.5 + n\nh"), "F64");
    assert_eq!(ty("k = (n = 1) => n\nk"), "Integer");
    assert_eq!(ty("w = (n = 1) => n + 2\nw"), "Integer");
}

#[test]
fn a_flag_without_its_operand_and_a_duplicate_flag_are_named() {
    assert_eq!(kinds("ls -depth"), [("FlagOperand", "-depth")]);
    assert_eq!(kinds("ls -depth -l"), [("FlagOperand", "-depth")]);
    assert_eq!(kinds("ls -l -l"), [("DuplicateSupply", "-l")]);
    assert_eq!(kinds("ls -ll"), [("DuplicateSupply", "-ll")]);
    assert_eq!(kinds("ls -nope"), [("UnknownSpot", "-nope")]);
    assert_eq!(ty("ls -depth 2"), "Vec<Entry>");
}

#[test]
fn an_outcome_form_at_the_wrong_place_names_its_own_form() {
    assert_eq!(kinds("v: String = [:1]"), [("NoBracketForm", "[:1]")]);
    assert_eq!(
        kinds("r: Either<Integer, String> = [:\"x\"]"),
        [("NoBracketForm", "[:\"x\"]")]
    );
    // The left of the second `||` is a literal's variable: no outcome.
    assert_eq!(kinds("Just 3 || 4 || 5"), [("NeedsMaybeOrResult", "||")]);
    assert_eq!(kinds("m = [:1]"), [("UnknownFamily", "[:1]")]);
    assert_eq!(kinds("Just 1 || Ok 1"), [("FamilyClash", "||")]);
    assert_eq!(ty("Just 1 || Just 2"), "Maybe<Integer>");
    assert_eq!(
        kinds("ok: Result<Integer, String> = [!]"),
        [("MissingPayload", "[!]")]
    );
    assert_eq!(ty("ok: Result<Integer, ()> = [!]"), "Result<Integer, ()>");
    assert_eq!(ty("m: Maybe<Integer> = [!]"), "Maybe<Integer>");
    assert_eq!(
        ty("f: Result<Integer, String> -> Integer = { [:x] => x, [!] => 0 }"),
        "Result<Integer, String> -> Integer"
    );
}

#[test]
fn a_payload_on_a_constructor_without_fields_is_named() {
    assert_eq!(
        kinds("c = { True True => 1, _ => 0 }"),
        [("NoPayload", "True")]
    );
    assert_eq!(
        ty("c = { Just x => x, Nothing => 0 }"),
        "Literal a => Maybe<a> -> a"
    );
}

#[test]
fn an_integer_a_float_cannot_hold_is_not_exact_or_out_of_range() {
    assert_eq!(
        kinds("n: F64 = 9007199254740993"),
        [("NotExact", "9007199254740993")]
    );
    assert_eq!(ty("n: F64 = 9007199254740992"), "F64");
    let huge = "340282366920938463463374607431768211456";
    assert_eq!(kinds(&format!("n: F32 = {huge}")), [("OutOfRange", huge)]);
}
