use fipe_repl_core::{Out, Repl};

const GEOMETRY: &str =
    include_str!("../../../Documentation/examples/geometry.fipe");
const HOST: [(&str, &str); 5] = [
    (
        "tests/fixtures/host/commands.fipe",
        include_str!("../../../tests/fixtures/host/commands.fipe"),
    ),
    (
        "tests/fixtures/host/fs.fipe",
        include_str!("../../../tests/fixtures/host/fs.fipe"),
    ),
    (
        "tests/fixtures/host/sys.fipe",
        include_str!("../../../tests/fixtures/host/sys.fipe"),
    ),
    (
        "tests/fixtures/host/cli.fipe",
        include_str!("../../../tests/fixtures/host/cli.fipe"),
    ),
    (
        "tests/fixtures/host/text.fipe",
        include_str!("../../../tests/fixtures/host/text.fipe"),
    ),
];

/// The written bytes, with any other output a test failure.
fn written(outs: Vec<Out>) -> String {
    let mut bytes = Vec::new();
    for out in outs {
        let Out::Write(chunk) = out else {
            panic!("unexpected {out:?}")
        };
        bytes.extend(chunk);
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn the_first_feed_prompts() {
    assert_eq!(Repl::new().feed(&[]), [Out::Write(b"fipe> ".to_vec())]);
}

#[test]
fn a_local_resolves_to_its_site() {
    let text = written(Repl::new().feed(b"x = 1\n:r x\n"));
    assert_eq!(
        text,
        "1:1 0..1 x: Local { site: Span { start: 0, end: 1 } }\nfipe> "
    );
}

#[test]
fn a_diagnostic_underlines_its_span() {
    let text = written(Repl::new().feed(b"nope 1\n"));
    assert_eq!(text, "1:1: unknown name\nnope 1\n^^^^\nfipe> ");
}

#[test]
fn a_partial_line_waits_for_its_end_or_a_flush() {
    let mut repl = Repl::new();
    assert_eq!(repl.feed(b"nope"), []);
    assert_eq!(written(repl.flush()), "1:1: unknown name\nnope\n^^^^\n");
    assert_eq!(repl.flush(), []);
}

#[test]
fn use_asks_for_the_file_and_resumes() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use geometry.Point\nPoint 1\n:r Point\n");
    assert_eq!(outs, [Out::File("geometry.fipe".into())]);
    let text = written(repl.file("geometry.fipe", Some(GEOMETRY)));
    assert!(
        text.contains(
            "1:1 0..5 Point: Constructor { origin: File(\"geometry\")"
        ),
        "{text}"
    );
}

#[test]
fn load_asks_for_the_stem() {
    let mut repl = Repl::new();
    assert_eq!(
        repl.feed(b":load geometry\n"),
        [Out::File("geometry.fipe".into())]
    );
    // Its four `hypot`/`atan2` rows are 07's casts; the line is accepted.
    let text = written(repl.file("geometry.fipe", Some(GEOMETRY)));
    assert_eq!(text.matches("expected F64, found I64").count(), 4, "{text}");
    assert!(text.starts_with("geometry.fipe:12:55: "), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    let text = written(repl.feed(b":r geometry.Point 1\n"));
    assert!(text.contains("Constructor"), "{text}");
}

#[test]
fn load_takes_a_stem_or_a_path() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":load 01-files\n");
    assert_eq!(
        outs,
        [
            Out::Write(b"01-files as files\n".to_vec()),
            Out::File("01-files.fipe".into())
        ]
    );
    let text = written(repl.file("01-files.fipe", Some("pub k = 1\n")));
    assert_eq!(text, "fipe> ");
    assert_eq!(written(repl.feed(b":t files.k\n")), "files.k: I64\nfipe> ");
    let outs = repl.feed(b":load ./x/y.fipe\n");
    assert_eq!(outs, [Out::File("./x/y.fipe".into())]);
    let text = written(repl.file("./x/y.fipe", None));
    assert!(text.starts_with("1:5: unknown module\nuse y\n"), "{text}");
    let outs = repl.feed(b":load shapes.fipe\n");
    assert_eq!(outs, [Out::File("shapes.fipe".into())]);
}

#[test]
fn load_without_a_stem_and_unknown_commands_say_so() {
    let text = written(Repl::new().feed(b":load\n:bogus 1\n:ast x\n"));
    assert_eq!(
        text,
        "usage: :load <stem>\nunknown command :bogus\n\
         usage: :ast (the last line that parsed)\nfipe> "
    );
}

#[test]
fn a_file_diagnostic_is_located_in_its_file() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":load bad\n");
    assert_eq!(outs, [Out::File("bad.fipe".into())]);
    let text =
        written(repl.file("bad.fipe", Some("x = 1\npub g = 1 + \"a\"\n")));
    assert_eq!(
        text,
        "bad.fipe:2:13: expected I64, found String\n\
         pub g = 1 + \"a\"\n            ^^^\nfipe> "
    );
}

#[test]
fn an_absent_file_is_an_unknown_module() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use missing\n");
    assert_eq!(outs, [Out::File("missing.fipe".into())]);
    let text = written(repl.file("missing.fipe", None));
    assert_eq!(
        text,
        "1:5: unknown module\nuse missing\n    ^^^^^^^\nfipe> "
    );
}

#[test]
fn a_private_inherent_member_is_named() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use shapes.P\n");
    assert_eq!(outs, [Out::File("shapes.fipe".into())]);
    let text = written(repl.file(
        "shapes.fipe",
        Some("pub type P { pub x: I64 }\nimpl P { hidden = self => 1 }\n"),
    ));
    assert_eq!(text, "fipe> ");
    let text = written(repl.feed(b"P.hidden\n"));
    assert!(text.contains("member `hidden` is private"), "{text}");
}

/// Answer `:host`'s five requests with the fixtures.
fn host(repl: &mut Repl, mut outs: Vec<Out>) -> String {
    for (index, (path, text)) in HOST.iter().enumerate() {
        assert_eq!(outs, [Out::File((*path).into())], "{index}");
        outs = repl.file(path, Some(text));
    }
    written(outs)
}

#[test]
fn host_loads_the_fixtures_and_keeps_accepted_lines() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"x = 1\n:host\n");
    assert_eq!(host(&mut repl, outs), "host: tests/fixtures/host\nfipe> ");
    let text = written(repl.feed(b"ls -l /etc\n:r ls\n:r x\n"));
    assert!(text.contains("ls: Value { origin: Commands"), "{text}");
    assert!(text.contains("x: Local"), "{text}");
}

#[test]
fn a_vertical_pipeline_is_one_entry() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":host\n");
    host(&mut repl, outs);
    let text = written(repl.feed(
        b"ls -l /var/log\n    | filter (e => e.size > 1)\n    | map .name\n:t\n",
    ));
    assert_eq!(
        text,
        "ls -l /var/log | filter (e => e.size > 1) | map .name: \
         Vec<String>\nfipe> "
    );
}

#[test]
fn ast_shows_the_last_accepted_line_as_a_tree() {
    let text = written(Repl::new().feed(b"f = x => x + 1\n:ast\n"));
    assert_eq!(
        text,
        "Bind\n  Name f\n  Arms\n    Arm\n      Name x\n      Binary +\n\
         \x20       Name x\n        Int 1\nfipe> "
    );
}

#[test]
fn t_shows_the_last_binding_s_scheme() {
    let text = written(Repl::new().feed(b"add = a b => a + b\n:t\n"));
    let line = text.lines().next().unwrap();
    assert!(line.starts_with("add: "), "{text}");
    assert!(line.contains("->"), "{text}");
}

#[test]
fn t_queries_an_expression_s_type() {
    let text = written(Repl::new().feed(b":t 1 + 2\n"));
    assert_eq!(text, "1 + 2: I64\nfipe> ");
}

#[test]
fn t_reports_a_type_mismatch() {
    let text = written(Repl::new().feed(b":t 1 + \"a\"\n"));
    let line = text.lines().next().unwrap();
    assert!(line.contains("expected"), "{text}");
    assert!(line.contains("found"), "{text}");
}

#[test]
fn q_exits() {
    assert_eq!(Repl::new().feed(b":q\nx\n"), [Out::Exit]);
}

#[test]
fn a_redefinition_in_a_later_line_shadows() {
    let text = written(Repl::new().feed(b"f = x => x\nf = x => x + 1\n:t f\n"));
    assert_eq!(text, "f: Num a => a -> a\nfipe> ");
}

#[test]
fn a_duplicate_in_one_entry_names_its_first_site() {
    let text = written(Repl::new().feed(b"type B { C,\n  C }\n"));
    assert_eq!(
        text,
        "2:3: already defined: first defined at 1:10\n  C }\n  ^\nfipe> "
    );
}

#[test]
fn an_ambiguity_names_its_class() {
    let text = written(Repl::new().feed(b"[] == []\n"));
    let line = text.lines().next().unwrap();
    assert_eq!(line, "1:1: ambiguous: `Eq` on an unknown type");
}

#[test]
fn a_syntax_error_names_what_it_expected_in_words() {
    let text = written(Repl::new().feed(b"q = (1 ]\n"));
    assert!(text.starts_with("1:8: expected )\n"), "{text}");
    assert!(!text.contains("Symbol"), "{text}");
}

#[test]
fn a_mismatch_prints_types_by_display() {
    let text = written(Repl::new().feed(b"1 + \"a\"\n"));
    assert_eq!(
        text,
        "1:5: expected I64, found String\n1 + \"a\"\n    ^^^\nfipe> "
    );
}

const PRIMES: &str = "pub type Prime { pub n: U64 }
impl Prime { pub check: Prime -> Bool = self => self.n > 1 }
pub next: Prime -> Prime = p => Prime { n: p.n + 1 }
";

#[test]
fn two_types_of_one_name_are_printed_with_their_modules() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use primes.Prime primes.next\n");
    assert_eq!(outs, [Out::File("primes.fipe".into())]);
    written(repl.file("primes.fipe", Some(PRIMES)));
    let text = written(repl.feed(b"type Prime { s: String }\n"));
    assert_eq!(text, "fipe> ");
    let text = written(repl.feed(b"next (Prime \"x\")\n"));
    assert_eq!(
        text,
        "1:7: expected primes.Prime, found Prime\n\
         next (Prime \"x\")\n      ^^^^^^^^^\nfipe> "
    );
}

#[test]
fn the_identity_and_literal_smokes() {
    let mut repl = Repl::new();
    repl.feed(b"use primes.Prime primes.next\n");
    written(repl.file("primes.fipe", Some(PRIMES)));
    let text = written(repl.feed(b":t Prime.check\n:t 2 + 2.5\n"));
    assert_eq!(text, "Prime.check: Prime -> Bool\n2 + 2.5: F64\nfipe> ");
    let text = written(repl.feed(b"p = Prime 7\nnext (p.n + 2)\n"));
    assert_eq!(
        text,
        "1:7: expected Prime, found U64\n\
         next (p.n + 2)\n      ^^^\nfipe> "
    );
}

#[test]
fn an_open_bracket_or_trailing_operator_continues_the_entry() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"y = (1 +\n")), "....> ");
    assert_eq!(written(repl.feed(b"2\n")), "....> ");
    assert_eq!(written(repl.feed(b")\n")), "fipe> ");
    assert_eq!(written(repl.feed(b":t\n")), "y: I64\nfipe> ");
    assert_eq!(written(repl.feed(b":t 1 +\n")), "....> ");
    assert_eq!(written(repl.feed(b"  2\n")), "1 + 2: I64\nfipe> ");
}

#[test]
fn a_leading_operator_continues_a_line_given_with_it() {
    let text = written(Repl::new().feed(b"z = 1\n  + 2\n:t\n"));
    assert_eq!(text, "z: I64\nfipe> ");
}

#[test]
fn an_empty_line_submits_an_open_entry() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"w = (1\n")), "....> ");
    let text = written(repl.feed(b"\n"));
    assert!(text.starts_with("1:7: expected )\nw = (1\n"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
}

#[test]
fn type_arguments_do_not_continue() {
    let text = written(Repl::new().feed(b"r = read<I64>\n:t\n"));
    assert!(text.starts_with("r: "), "{text}");
}

#[test]
fn end_of_input_at_a_prompt_ends_its_line() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(&[])), "fipe> ");
    assert_eq!(written(repl.flush()), "\n");
}

#[test]
fn a_leading_operator_typed_later_continues_the_accepted_line() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":host\n");
    host(&mut repl, outs);
    for line in [
        "ls -l /var/log\n",
        "    | filter (e => e.size > 1_000_000)\n",
        "    | sort -by .size -desc\n",
        "    | take 10\n",
        "    | map .name\n",
    ] {
        assert_eq!(written(repl.feed(line.as_bytes())), "fipe> ", "{line}");
    }
    let text = written(repl.feed(b":t\n"));
    assert!(text.ends_with(": Vec<String>\nfipe> "), "{text}");
    assert_eq!(repl.feed(b"x = 1\n").len(), 1);
    let text = written(repl.feed(b":r ls\n"));
    assert_eq!(text.matches("ls: Value").count(), 1, "{text}");
}

#[test]
fn a_rejected_continuation_keeps_the_line_before() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"y = 1\n")), "fipe> ");
    let text = written(repl.feed(b"  + \"a\"\n"));
    assert!(
        text.starts_with("2:5: expected I64, found String\n"),
        "{text}"
    );
    assert_eq!(written(repl.feed(b":t y\n")), "y: I64\nfipe> ");
}

/// All written text with the prompts taken out.
fn unprompted(outs: Vec<Out>) -> String {
    written(outs).replace("fipe> ", "").replace("....> ", "")
}

#[test]
fn a_rejected_join_in_one_read_keeps_the_line_before() {
    let input = "x = 1\n+ \"a\"\n:t x\n";
    let whole = unprompted(Repl::new().feed(input.as_bytes()));
    let mut repl = Repl::new();
    let mut lines = String::new();
    for line in input.split_inclusive('\n') {
        lines += &unprompted(repl.feed(line.as_bytes()));
    }
    assert_eq!(whole, lines);
    assert!(
        whole.starts_with("2:3: expected I64, found String\n"),
        "{whole}"
    );
    assert!(whole.ends_with("x: I64\n"), "{whole}");
}

#[test]
fn a_query_continues_onto_a_leading_line_in_the_same_read() {
    let text = written(Repl::new().feed(b":t 1\n  + 2\n"));
    assert_eq!(text, "1 + 2: I64\nfipe> ");
}

#[test]
fn a_command_or_an_empty_line_ends_the_continuation() {
    for input in [
        "x = 1\n:t\n+ 2\n",
        "x = 1\n\n\n+ 2\n",
        "x = 1\n# note\n+ 2\n",
    ] {
        let text = written(Repl::new().feed(input.as_bytes()));
        assert!(text.contains("1:1: expected "), "{input:?}: {text}");
        assert!(text.contains("\n+ 2\n^"), "{input:?}: {text}");
    }
    let mut repl = Repl::new();
    let outs = repl.feed(b":load k\n");
    assert_eq!(outs, [Out::File("k.fipe".into())]);
    written(repl.file("k.fipe", Some("pub k = 1\n")));
    let text = written(repl.feed(b"+ 2\n"));
    assert!(text.starts_with("1:1: expected "), "{text}");
}

#[test]
fn load_of_a_second_file_for_a_loaded_module_changes_nothing() {
    let mut repl = Repl::new();
    repl.feed(b":load 01-files\n");
    written(repl.file("01-files.fipe", Some("pub k = 1\n")));
    let text = written(repl.feed(b":load 02-files\n"));
    assert_eq!(text, "02-files: already loaded as `files`\nfipe> ");
    assert_eq!(written(repl.feed(b":t files.k\n")), "files.k: I64\nfipe> ");
    let outs = repl.feed(b":load 01-files\n");
    assert_eq!(
        outs,
        [Out::Write(b"01-files as files\nfipe> ".to_vec())],
        "reloading the same file is still allowed"
    );
}

#[test]
fn a_diagnostic_past_column_65535_underlines_without_a_panic() {
    let line = format!("{}nope\n", " ".repeat(70_000));
    let text = written(Repl::new().feed(line.as_bytes()));
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[0], "1:70001: unknown name");
    assert_eq!(rows[2], format!("{}^^^^", " ".repeat(70_000)));
}

#[test]
fn a_leading_assign_continues_the_line_before() {
    // Accepted: `  = 2` joins `x = 1` rather than standing alone,
    // where it would be rejected at 1:3.
    let text = written(Repl::new().feed(b"x = 1\n  = 2\n:t x\n"));
    assert!(!text.starts_with("1:"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    // Held: a query's signature takes its `= value` line.
    let text = written(Repl::new().feed(b":t f: I64 -> I64\n  = x => x + 1\n"));
    assert!(!text.starts_with("1:"), "{text}");
    // A bare signature waits for its `= value` and joins it: either the
    // pair is accepted, or it is rejected at its second line, never the
    // signature alone.
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"f: I64 -> I64\n")), "....> ");
    let text = written(repl.feed(b"  = x => x + 1\n:t f\n"));
    assert!(!text.contains("declaration form"), "{text}");
    assert!(
        text.starts_with("2:3: ") || text.starts_with("f: I64 -> I64\n"),
        "{text}"
    );
    // An empty line submits it alone.
    let text = written(Repl::new().feed(b"pub f: I64\n\n"));
    assert!(text.contains("declaration form"), "{text}");
}

#[test]
fn a_lowercase_numeric_type_earns_its_hint() {
    let text = written(Repl::new().feed(b"x: i64 = 1\n"));
    assert_eq!(
        text,
        "1:4: numeric types are uppercase: write `I64`\n\
         x: i64 = 1\n   ^^^\nfipe> "
    );
}

#[test]
fn ast_shows_a_parsed_line_the_checker_rejects() {
    let text = written(Repl::new().feed(b"pair = (1, 2)\n:ast\n"));
    assert_eq!(
        text,
        "1:8: a tuple is not yet checked\npair = (1, 2)\n       ^^^^^^\n\
         Bind\n  Name pair\n  Tuple\n    Int 1\n    Int 2\nfipe> "
    );
}

#[test]
fn a_where_line_continues_an_accepted_type_item() {
    let text = written(
        Repl::new()
            .feed(b"type P { x: I64 }\nwhere 0 <= self.x\n:ast\n:t P 1\n"),
    );
    assert_eq!(
        text,
        "1:6: a limited type is not yet checked\n\
         type P { x: I64 }\n     ^\n\
         Type P\n  Where\n    Binary <=\n      Int 0\n      Name self.x\n\
         P 1: P\nfipe> "
    );
}
