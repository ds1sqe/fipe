//! Finite public-boundary cases, isolated so a parser/checker hang is retained.
use fipe::{
    check, compile,
    decl::{self, Declarations, Kind, SpotProblem},
    lex::{self, lex},
    parse::{self, parse},
    token::{Source, Span},
};
use std::{
    process::Command,
    time::{Duration, Instant},
};

const CHILD: &str = "FIPE_SOURCE_FUZZ_CASE";
const SELECTOR: &str = "bounded_source_and_declaration_campaign";
const MAX_BYTES: usize = 8192;

#[derive(Clone, Copy)]
enum Api {
    Source,
    Commands,
    Module(&'static str),
}
#[derive(Clone, Copy)]
enum Lookup {
    Value,
    Type,
    Resource,
    Generic,
}
#[derive(Clone, Copy)]
enum Refusal {
    Duplicate,
    Constructor,
    Spot,
    Ambiguous,
    Resource,
    Line,
    Syntax,
    ModuleName,
    ModuleConflict,
}
#[derive(Clone, Copy)]
enum Expected {
    Observe,
    Compile,
    Parse,
    Nesting,
    Depth,
    Quote,
    Invisible,
    Registered(&'static str, Lookup),
    Refused(Refusal),
}
struct Case {
    id: String,
    source: String,
    api: Api,
    expected: Expected,
}
impl Case {
    fn new(
        id: impl Into<String>,
        source: impl Into<String>,
        api: Api,
        expected: Expected,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            api,
            expected,
        }
    }
}
#[derive(Default)]
struct Counts {
    lex_diagnostics: usize,
    parse_ok: usize,
    parse_refused: usize,
    check_ok: usize,
    check_refused: usize,
    compile_ok: usize,
    compile_budget_open: usize,
    declaration_registered: usize,
    declaration_refused: usize,
}

fn variants(source: &str) -> [(&'static str, String); 4] {
    let chars: Vec<_> = source.chars().collect();
    assert!(!chars.is_empty());
    let first = chars
        .iter()
        .position(|ch| "()[]{}<>".contains(*ch))
        .unwrap_or(0);
    [
        ("original", source.to_owned()),
        (
            "truncate-mid-scalar",
            chars[..chars.len() / 2].iter().collect(),
        ),
        (
            "delete-first-delimiter",
            chars
                .iter()
                .enumerate()
                .filter_map(|(index, ch)| (index != first).then_some(*ch))
                .collect(),
        ),
        ("prefix-invisible-open", format!("\n\u{200b}({source}")),
    ]
}

fn corpus_cases(cases: &mut Vec<Case>) {
    let sources = [
        include_str!("../Documentation/examples/hello.fipe"),
        include_str!("../Documentation/examples/01-files.fipe"),
        include_str!("../Documentation/examples/02-outcomes.fipe"),
        include_str!("../Documentation/examples/03-types.fipe"),
        include_str!("../Documentation/examples/04-text.fipe"),
        include_str!("../Documentation/examples/05-modules.fipe"),
        include_str!("../Documentation/examples/geometry.fipe"),
        include_str!("fixtures/host/commands.fipe"),
    ];
    for (index, source) in sources.iter().enumerate() {
        for (variant, text) in variants(source) {
            cases.push(Case::new(
                format!("source-corpus-{index:02}-{variant}"),
                text,
                Api::Source,
                Expected::Observe,
            ));
        }
    }
    let declarations = [
        include_str!("fixtures/host/commands.fipe"),
        include_str!("fixtures/host/fs.fipe"),
        include_str!("fixtures/host/sys.fipe"),
        include_str!("fixtures/host/cli.fipe"),
        include_str!("fixtures/host/text.fipe"),
        include_str!("fixtures/host/config.fipe"),
    ];
    for (index, source) in declarations.iter().enumerate() {
        for (variant_index, (variant, text)) in
            variants(source).into_iter().enumerate()
        {
            let api = if (index * 4 + variant_index) % 2 == 0 {
                Api::Commands
            } else {
                Api::Module("probe")
            };
            cases.push(Case::new(
                format!("declaration-corpus-{index:02}-{variant}"),
                text,
                api,
                Expected::Observe,
            ));
        }
    }
}

fn boundary_cases(cases: &mut Vec<Case>) {
    for (name, source, expected) in [
        (
            "nested-parens",
            format!("{}x{}", "(".repeat(1000), ")".repeat(1000)),
            Expected::Nesting,
        ),
        (
            "nested-type",
            format!("x: {}U8{} = y", "Vec<".repeat(1000), ">".repeat(1000)),
            Expected::Nesting,
        ),
        (
            "nested-constructor-pattern",
            format!("{}x = value", "Just ".repeat(1000)),
            Expected::Nesting,
        ),
        (
            "nested-function-type",
            format!("f: {}A{} = g", "(".repeat(1000), " -> B)".repeat(1000)),
            Expected::Nesting,
        ),
        (
            "flat-add-255",
            format!("0{}", " + 1".repeat(parse::MAX_DEPTH - 1)),
            Expected::Parse,
        ),
        (
            "flat-add-256",
            format!("0{}", " + 1".repeat(parse::MAX_DEPTH)),
            Expected::Depth,
        ),
        (
            "flat-pipe-255",
            format!("a{}", " | a".repeat(parse::MAX_DEPTH - 1)),
            Expected::Parse,
        ),
        (
            "flat-pipe-256",
            format!("a{}", " | a".repeat(parse::MAX_DEPTH)),
            Expected::Depth,
        ),
        (
            "wide-arguments-1024",
            format!("f{}", " a".repeat(1024)),
            Expected::Parse,
        ),
        (
            "wide-list-1024",
            format!("[{}]", vec!["1"; 1024].join(", ")),
            Expected::Parse,
        ),
        (
            "unterminated-quote",
            "\"unfinished\nnext".to_owned(),
            Expected::Quote,
        ),
        (
            "invisible-scalar",
            "\u{200b}ls".to_owned(),
            Expected::Invisible,
        ),
    ] {
        cases.push(Case::new(
            format!("boundary-{name}"),
            source,
            Api::Source,
            expected,
        ));
    }
}

fn case_catalog() -> Vec<Case> {
    let mut cases = control_cases();
    corpus_cases(&mut cases);
    boundary_cases(&mut cases);
    declaration_negatives(&mut cases);
    assert_eq!(cases.len(), 96);
    for case in &cases {
        assert!(case.source.len() <= MAX_BYTES);
    }
    assert_eq!(
        cases
            .iter()
            .filter(|case| matches!(case.api, Api::Source))
            .count(),
        52
    );
    cases
}
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
fn valid_span(source: &str, span: Span) {
    assert!(
        Source::new(source).slice(span).is_some(),
        "invalid diagnostic span {span:?}"
    );
}
fn valid_check_error(source: &str, error: &check::Error) {
    let span = match error {
        check::Error::Syntax(error) => error.span,
        check::Error::Resolve(error) => error.span,
        check::Error::Type(error) => error.span,
        check::Error::Compile { span, .. } => *span,
        check::Error::File { .. }
        | check::Error::FileResolve { .. }
        | check::Error::FileType { .. } => {
            panic!("no file source supplied: {error:?}")
        }
    };
    valid_span(source, span);
}
fn phase(case: &Case, name: &str) {
    eprintln!("FUZZ_PHASE id={} phase={name}", case.id);
}

fn source_case(case: &Case) -> Counts {
    let mut counts = Counts::default();
    phase(case, "Lex");
    let lexed = lex(&case.source);
    counts.lex_diagnostics = lexed.errors.len();
    for error in &lexed.errors {
        valid_span(&case.source, error.span);
    }
    match case.expected {
        Expected::Quote => assert!(lexed
            .errors
            .iter()
            .any(|error| error.reason == lex::Reason::UnterminatedQuote)),
        Expected::Invisible => assert!(lexed
            .errors
            .iter()
            .any(|error| error.reason == lex::Reason::Stray('\u{200b}'))),
        _ => {}
    }
    phase(case, "Parse");
    let module = match parse(lexed) {
        Ok(module) => {
            assert!(!matches!(
                case.expected,
                Expected::Nesting
                    | Expected::Depth
                    | Expected::Quote
                    | Expected::Invisible
            ));
            counts.parse_ok = 1;
            module
        }
        Err(errors) => {
            assert!(!matches!(
                case.expected,
                Expected::Compile | Expected::Parse
            ));
            assert!(!errors.is_empty());
            for error in &errors {
                valid_span(&case.source, error.span);
            }
            if matches!(case.expected, Expected::Nesting) {
                assert!(errors
                    .iter()
                    .any(|error| error.reason == parse::Reason::NestingLimit));
            }
            if matches!(case.expected, Expected::Depth) {
                assert!(errors
                    .iter()
                    .any(|error| error.reason == parse::Reason::DepthLimit));
            }
            eprintln!(
                "FUZZ_PARSE_REFUSED id={} first={:?}",
                case.id,
                errors.first()
            );
            counts.parse_refused = 1;
            return counts;
        }
    };
    phase(case, "Check");
    let unit = match check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options {
            strict: false,
            compile_budget: 256,
        },
    ) {
        Ok(unit) => {
            counts.check_ok = 1;
            unit
        }
        Err(errors) => {
            assert!(!matches!(case.expected, Expected::Compile));
            assert!(!errors.is_empty());
            for error in &errors {
                valid_check_error(&case.source, error);
            }
            eprintln!(
                "FUZZ_CHECK_REFUSED id={} first={:?}",
                case.id,
                errors.first()
            );
            counts.check_refused = 1;
            return counts;
        }
    };
    counts.compile_budget_open = unit
        .plan
        .iter()
        .filter(|entry| matches!(entry.reason, fipe::ir::Reason::CompileBudget))
        .count();
    phase(case, "Compile");
    let _program = compile::compile(&unit);
    counts.compile_ok = 1;
    phase(case, "Drop");
    counts
}

fn expected_refusal(reason: &decl::Reason, expected: Refusal) -> bool {
    match expected {
        Refusal::Duplicate => matches!(reason, decl::Reason::Duplicate { .. }),
        Refusal::Constructor => {
            matches!(reason, decl::Reason::ConstructorCollision { name, .. } if name == "Released")
        }
        Refusal::Spot => {
            matches!(reason, decl::Reason::Spots { problem: SpotProblem::Duplicate(name) } if name == "x")
        }
        Refusal::Ambiguous => matches!(
            reason,
            decl::Reason::Spots {
                problem: SpotProblem::AmbiguousFlag('l')
            }
        ),
        Refusal::Resource => matches!(reason, decl::Reason::ResourceForm),
        Refusal::Line => matches!(reason, decl::Reason::NotDeclaration),
        Refusal::Syntax => matches!(reason, decl::Reason::Syntax(_)),
        Refusal::ModuleName => {
            matches!(reason, decl::Reason::ModuleName { name } if name == "Bad.Name")
        }
        Refusal::ModuleConflict => {
            matches!(reason, decl::Reason::ModuleConflict { name } if name == "probe.child")
        }
    }
}
fn valid_declaration_error(source: &str, error: &decl::Error) {
    match &error.reason {
        decl::Reason::ModuleName { .. }
        | decl::Reason::ModuleConflict { .. } => {
            assert_eq!(error.span, Span::default());
        }
        decl::Reason::Duplicate { first }
        | decl::Reason::ConstructorCollision { first, .. } => {
            valid_span(source, *first);
        }
        _ => {}
    }
    valid_span(source, error.span);
}
fn registered(namespace: &decl::Namespace, name: &str, lookup: Lookup) {
    let declared = match lookup {
        Lookup::Value => namespace.value(name).unwrap(),
        _ => namespace.ty(name).unwrap(),
    };
    assert!(declared.public);
    if !matches!(lookup, Lookup::Value) {
        let Kind::Type {
            resource,
            parameters,
            ..
        } = declared.kind
        else {
            panic!("type lookup");
        };
        assert_eq!(resource, matches!(lookup, Lookup::Resource));
        if matches!(lookup, Lookup::Generic) {
            assert_eq!(parameters, 1);
        }
    }
}
fn declaration_case(case: &Case) -> Counts {
    phase(case, "Registration");
    let mut declarations = Declarations::new();
    declarations.commands("pub sentinel: Integer").unwrap();
    declarations
        .module("probe", "pub old: Integer\npub child: Integer")
        .unwrap();
    let before = declarations.clone();
    let result = match case.api {
        Api::Commands => declarations.commands(&case.source),
        Api::Module(name) => declarations.module(name, &case.source),
        Api::Source => unreachable!("source cases use source pipeline"),
    };
    let mut counts = Counts::default();
    match result {
        Ok(()) => {
            assert!(!matches!(case.expected, Expected::Refused(_)));
            if let Expected::Registered(name, lookup) = case.expected {
                let namespace = match case.api {
                    Api::Commands => declarations.commands.as_ref().unwrap(),
                    Api::Module(name) => declarations.get_module(name).unwrap(),
                    Api::Source => unreachable!("declaration API"),
                };
                registered(namespace, name, lookup);
            }
            counts.declaration_registered = 1;
        }
        Err(errors) => {
            assert!(!matches!(case.expected, Expected::Registered(_, _)));
            assert!(!errors.is_empty());
            for error in &errors {
                valid_declaration_error(&case.source, error);
            }
            if let Expected::Refused(expected) = case.expected {
                assert!(errors
                    .iter()
                    .any(|error| expected_refusal(&error.reason, expected)));
            }
            assert_eq!(declarations, before, "refusal must be atomic");
            assert!(declarations.get_module("new").is_none());
            assert!(declarations.get_module("probe.child").is_none());
            eprintln!(
                "FUZZ_DECL_REFUSED id={} first={:?}",
                case.id,
                errors.first()
            );
            counts.declaration_refused = 1;
        }
    }
    phase(case, "Drop");
    counts
}

fn child_case(index: usize, case: &Case) {
    eprintln!(
        "FUZZ_CASE ordinal={index} id={} bytes={} fnv={:016x}",
        case.id,
        case.source.len(),
        fnv(case.source.as_bytes())
    );
    let counts = match case.api {
        Api::Source => source_case(case),
        _ => declaration_case(case),
    };
    eprintln!("FUZZ_RESULT ordinal={index} id={} lex_diagnostics={} parse_ok={} parse_refused={} check_ok={} check_refused={} compile_ok={} compile_budget_open={} declaration_registered={} declaration_refused={}",
        case.id, counts.lex_diagnostics, counts.parse_ok, counts.parse_refused,
        counts.check_ok, counts.check_refused, counts.compile_ok, counts.compile_budget_open,
        counts.declaration_registered, counts.declaration_refused);
}
fn run_child(index: usize, case: &Case, overall: Instant) {
    assert!(
        Instant::now() < overall,
        "campaign deadline before {}",
        case.id
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", SELECTOR, "--nocapture", "--test-threads=1"])
        .env(CHILD, index.to_string())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline || Instant::now() >= overall {
            let killed = child.kill();
            let status = child.wait().unwrap();
            panic!(
                "case {} timeout; kill={killed:?} status={status} input={:?}",
                case.id, case.source
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "case {} failed: {status}; input={:?}",
        case.id,
        case.source
    );
}
#[test]
fn bounded_source_and_declaration_campaign() {
    let cases = case_catalog();
    match std::env::var(CHILD) {
        Ok(index) => {
            let index: usize = index.parse().expect("numeric frozen case ID");
            child_case(index, &cases[index]);
        }
        Err(std::env::VarError::NotPresent) => {
            let overall = Instant::now() + Duration::from_secs(240);
            for (index, case) in cases.iter().enumerate() {
                run_child(index, case, overall);
            }
            eprintln!("FUZZ_SUMMARY completed=96 source=52 declaration=44; child summaries are nonadditive");
        }
        Err(error) => panic!("invalid worker selection: {error}"),
    }
}

fn control_cases() -> Vec<Case> {
    vec![
        Case::new("source-control-00", "1", Api::Source, Expected::Compile),
        Case::new(
            "source-control-01",
            "x = 1\nx + 1",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-02",
            "id = x => x\nid 1",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-03",
            "if True { 1 } else { 2 }",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-04",
            "Just x = Just 3\nx + 1",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-05",
            "((Just (a, b))) = Just (3, 4)\na + b",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-06",
            "(1, \"a\")",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "source-control-07",
            "f = (w = 80) (c = (w / 8)) => c\nf 16",
            Api::Source,
            Expected::Compile,
        ),
        Case::new(
            "declaration-control-00",
            "pub echo: Integer -> Integer",
            Api::Commands,
            Expected::Registered("echo", Lookup::Value),
        ),
        Case::new(
            "declaration-control-01",
            "pub type Point { pub x: Integer, pub y: Integer }",
            Api::Commands,
            Expected::Registered("Point", Lookup::Type),
        ),
        Case::new(
            "declaration-control-02",
            "pub type resource Table",
            Api::Commands,
            Expected::Registered("Table", Lookup::Resource),
        ),
        Case::new(
            "declaration-control-03",
            "pub type Rd<T>",
            Api::Commands,
            Expected::Registered("Rd", Lookup::Generic),
        ),
        Case::new(
            "declaration-control-04",
            "pub type Packet",
            Api::Module("probe"),
            Expected::Registered("Packet", Lookup::Type),
        ),
        Case::new(
            "declaration-control-05",
            "pub choose: (left: Bool) -> (right: Bool) -> Bool",
            Api::Module("probe"),
            Expected::Registered("choose", Lookup::Value),
        ),
        Case::new(
            "declaration-control-06",
            "pub width: U8",
            Api::Module("probe"),
            Expected::Registered("width", Lookup::Value),
        ),
        Case::new(
            "declaration-control-07",
            "pub type Box<T> { pub value: T }",
            Api::Module("probe"),
            Expected::Registered("Box", Lookup::Generic),
        ),
    ]
}

fn declaration_negatives(cases: &mut Vec<Case>) {
    cases.extend([
        Case::new("declaration-negative-duplicate-value", "pub x: I64\ny = 1\npub x: U64", Api::Commands, Expected::Refused(Refusal::Duplicate)),
        Case::new("declaration-negative-constructor-collision", "pub type Released { pub keys: Vec<U64> }\npub type Outcome { Released(value: Released), Nothing }", Api::Module("new.child"), Expected::Refused(Refusal::Constructor)),
        Case::new("declaration-negative-duplicate-spot", "f: (x: Integer) -> (x: Integer) -> ()", Api::Commands, Expected::Refused(Refusal::Spot)),
        Case::new("declaration-negative-ambiguous-bool", "pub type Values {\n    pub linebreak: Bool,\n    pub list: Bool,\n}", Api::Module("probe"), Expected::Refused(Refusal::Ambiguous)),
        Case::new("declaration-negative-resource-base", "pub type resource Table: Integer", Api::Commands, Expected::Refused(Refusal::Resource)),
        Case::new("declaration-negative-resource-fields", "pub type resource Table {}", Api::Module("new.child"), Expected::Refused(Refusal::Resource)),
        Case::new("declaration-negative-resource-constructor", "pub type resource Table { Make(value: Integer) }", Api::Commands, Expected::Refused(Refusal::Resource)),
        Case::new("declaration-negative-resource-where", "pub type resource Table where self > 0", Api::Module("probe"), Expected::Refused(Refusal::Resource)),
        Case::new("declaration-negative-executable-line", "pub x: I64\nls -l\n", Api::Commands, Expected::Refused(Refusal::Line)),
        Case::new("declaration-negative-incomplete-signature", "x: ", Api::Module("probe"), Expected::Refused(Refusal::Syntax)),
        Case::new("declaration-negative-invalid-module-name", "pub value: Integer", Api::Module("Bad.Name"), Expected::Refused(Refusal::ModuleName)),
        Case::new("declaration-negative-submodule-value-collision", "pub value: Integer", Api::Module("probe.child"), Expected::Refused(Refusal::ModuleConflict)),
    ]);
}
