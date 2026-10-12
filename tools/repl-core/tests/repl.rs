use fipe_repl_core::{Out, Repl};

const GEOMETRY: &str =
    include_str!("../../../Documentation/examples/geometry.fipe");
const HOST: [(&str, &str); 6] = [
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
    (
        "tests/fixtures/host/config.fipe",
        include_str!("../../../tests/fixtures/host/config.fipe"),
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
    assert_eq!(text, "1:1 x: binding at line 1\nfipe> ");
}

#[test]
fn a_diagnostic_underlines_its_span() {
    let text = written(Repl::new().feed(b"nope 1\n\n"));
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
        text.contains("1:1 Point: constructor geometry.Point.Point"),
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
    let text = written(repl.file("geometry.fipe", Some(GEOMETRY)));
    assert!(!text.contains("expected F64"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    let text = written(repl.feed(b":r geometry.Point 1\n"));
    assert!(text.contains("constructor"), "{text}");
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
    assert_eq!(
        written(repl.feed(b":t files.k\n")),
        "files.k: Integer\nexact 1\nknown at compile\nfipe> "
    );
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
        "usage: :load <stem or path>\nunknown command :bogus\n\
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
        "bad.fipe:2:9: no meaning reads all literal forms: Whole, Text\n\
         pub g = 1 + \"a\"\n        ^^^^^^^\nfipe> "
    );
}

#[test]
fn an_absent_file_is_an_unknown_module() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use missing\n\n");
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
    let outs = repl.feed(b"use shapes.P\n\n");
    assert_eq!(outs, [Out::File("shapes.fipe".into())]);
    let text = written(repl.file(
        "shapes.fipe",
        Some("pub type P { pub x: I64 }\nimpl P { hidden = self => 1 }\n"),
    ));
    assert_eq!(text, "fipe> ");
    let text = written(repl.feed(b"P.hidden\n\n"));
    assert!(text.contains("member `hidden` is private"), "{text}");
}

/// Answer `:host`'s six requests with the fixtures.
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
    assert!(text.contains("ls: value commands.ls"), "{text}");
    assert!(text.contains("x: binding at line 1"), "{text}");
}

#[test]
fn a_vertical_pipeline_is_one_entry() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":host\n");
    host(&mut repl, outs);
    let text = written(repl.feed(
        b"ls -l /var/log\n    | filter (e => e.size > 1)\n    | map .name\n:t\n",
    ));
    // The line runs and asks the host, which the REPL does not wire yet.
    assert_eq!(
        text,
        "fault at 1:7: host: no host for commands.read\nls -l /var/log\n      \
         ^^^^^^^^\nls -l /var/log | filter (e => e.size > 1) | map .name: \
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
    assert_eq!(text, "1 + 2: Integer\nexact 3\nknown at compile\nfipe> ");
}

#[test]
fn t_reports_a_type_mismatch() {
    let text = written(Repl::new().feed(b"n = 1\n:t n + \"a\"\n"));
    let line = text.lines().next().unwrap();
    assert!(line.contains("expected"), "{text}");
    assert!(line.contains("found"), "{text}");
}

#[test]
fn q_exits() {
    assert_eq!(Repl::new().feed(b":q\nx\n"), [Out::Exit]);
}

#[test]
fn a_redefinition_in_a_later_entry_shadows() {
    let text =
        written(Repl::new().feed(b"f = x => x\n\nf = x => x + 1\n:t f\n"));
    assert_eq!(
        text,
        "f: Add a, Literal a => a -> a\nknown at compile\nfipe> "
    );
}

#[test]
fn a_duplicate_in_one_entry_names_its_first_site() {
    let text = written(Repl::new().feed(b"type B { C,\n  C }\n\n"));
    assert_eq!(
        text,
        "2:3: already defined: first defined at 1:10\n  C }\n  ^\nfipe> "
    );
}

#[test]
fn an_ambiguity_names_its_class() {
    let text = written(Repl::new().feed(b"[] == []\n\n"));
    let line = text.lines().next().unwrap();
    assert_eq!(line, "1:4: ambiguous: `Eq` on an unknown type");
}

#[test]
fn a_syntax_error_names_what_it_expected_in_words() {
    let text = written(Repl::new().feed(b"q = (1 ]\n\n"));
    assert!(text.starts_with("1:8: expected )\n"), "{text}");
    assert!(!text.contains("Symbol"), "{text}");
}

#[test]
fn a_mismatch_prints_types_by_display() {
    let text = written(Repl::new().feed(b"n = 1\n\nn + \"a\"\n\n"));
    assert_eq!(
        text,
        "1:5: expected Integer, found String\nn + \"a\"\n    ^^^\nfipe> "
    );
}

const PRIMES: &str = "pub type Prime { pub n: U64 }
impl Prime { pub check: Prime -> Bool = self => self.n > 1 }
pub next: Prime -> Prime = p => Prime { n: p.n + 1 }
";

#[test]
fn two_types_of_one_name_are_printed_with_their_modules() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use primes.Prime primes.next\n\n");
    assert_eq!(outs, [Out::File("primes.fipe".into())]);
    written(repl.file("primes.fipe", Some(PRIMES)));
    let text = written(repl.feed(b"type Prime { s: String }\n\n"));
    assert_eq!(text, "fipe> ");
    let text = written(repl.feed(b"next (Prime \"x\")\n\n"));
    assert_eq!(
        text,
        "1:7: expected primes.Prime, found session[2].Prime\n\
         next (Prime \"x\")\n      ^^^^^^^^^\nfipe> "
    );
}

#[test]
fn the_identity_and_literal_smokes() {
    let mut repl = Repl::new();
    repl.feed(b"use primes.Prime primes.next\n\n");
    written(repl.file("primes.fipe", Some(PRIMES)));
    let text = written(repl.feed(b":t Prime.check\n:t 2 + 2.5\n"));
    assert_eq!(text, "Prime.check: Prime -> Bool\nknown at compile\n2 + 2.5: F64\nknown at compile\nheld Size<8>\nfipe> ");
    let text = written(repl.feed(b"p = Prime 7\n\nnext (p.n + 2)\n\n"));
    assert_eq!(
        text,
        "1:7: expected Prime, found Integer\n\
         next (p.n + 2)\n      ^^^^^^^\nfipe> "
    );
}

#[test]
fn an_entry_is_held_until_a_blank_line_or_a_command() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"y = (1 +\n")), "....> ");
    assert_eq!(written(repl.feed(b"2\n")), "....> ");
    assert_eq!(written(repl.feed(b")\n")), "....> ");
    assert_eq!(
        written(repl.feed(b":t\n")),
        "y: Integer\nexact 3\nknown at compile\nfipe> "
    );
    // A command's own bracket is closed first; then it runs at once.
    assert_eq!(written(repl.feed(b":t (1 +\n")), "....> ");
    assert_eq!(
        written(repl.feed(b"  2)\n")),
        "(1 + 2): Integer\nexact 3\nknown at compile\nfipe> "
    );
    assert_eq!(written(repl.feed(b"type K: Integer &\n")), "....> ");
    assert_eq!(written(repl.feed(b"  Natural\n")), "....> ");
    assert_eq!(written(repl.feed(b"\n")), "fipe> ");
    assert!(written(repl.feed(b":t K\n")).starts_with("K: "));
}

#[test]
fn a_leading_operator_continues_a_line_given_with_it() {
    let text = written(Repl::new().feed(b"z = 1\n  + 2\n:t\n"));
    assert_eq!(text, "z: Integer\nexact 3\nknown at compile\nfipe> ");
}

#[test]
fn a_blank_line_inside_a_bracket_stays_in_the_entry() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"w = (1\n")), "....> ");
    assert_eq!(written(repl.feed(b"\n")), "....> ");
    let text = written(repl.feed(b"]\n\n"));
    assert!(text.starts_with("3:1: expected "), "{text}");
    assert!(text.contains("\n]\n^\n"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    // End of input submits an entry whatever it has open.
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"w = (1\n")), "....> ");
    let text = written(repl.flush());
    assert!(text.contains("expected )"), "{text}");
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
fn a_pipeline_typed_line_by_line_is_one_entry() {
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
        assert_eq!(written(repl.feed(line.as_bytes())), "....> ", "{line}");
    }
    let text = written(repl.feed(b":t\n"));
    assert!(text.ends_with(": Vec<String>\nfipe> "), "{text}");
    assert_eq!(repl.feed(b"x = 1\n"), [Out::Write(b"....> ".to_vec())]);
    let text = written(repl.feed(b":r ls\n"));
    assert_eq!(text.matches("ls: value").count(), 1, "{text}");
}

#[test]
fn a_rejected_entry_defines_nothing() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"y = 1\n")), "....> ");
    assert_eq!(written(repl.feed(b"  + \"a\"\n")), "....> ");
    let text = written(repl.feed(b"\n"));
    assert!(
        text.starts_with(
            "1:5: no meaning reads all literal forms: Whole, Text\n"
        ),
        "{text}"
    );
    assert!(written(repl.feed(b":t y\n")).contains("unknown name"));
}

/// All written text with the prompts taken out.
fn unprompted(outs: Vec<Out>) -> String {
    written(outs).replace("fipe> ", "").replace("....> ", "")
}

#[test]
fn entries_are_independent_of_read_boundaries() {
    let input = "x = 1\n+ \"a\"\n:t x\n";
    let whole = unprompted(Repl::new().feed(input.as_bytes()));
    let mut repl = Repl::new();
    let mut lines = String::new();
    for line in input.split_inclusive('\n') {
        lines += &unprompted(repl.feed(line.as_bytes()));
    }
    assert_eq!(whole, lines);
    assert!(
        whole.starts_with(
            "1:5: no meaning reads all literal forms: Whole, Text\n"
        ),
        "{whole}"
    );
    assert!(whole.ends_with("unknown name\nx\n^\n"), "{whole}");
}

#[test]
fn a_complete_query_finishes_before_a_later_leading_operator() {
    let text = written(Repl::new().feed(b":t 1\n  + 2\n\n"));
    assert!(text.starts_with("1: Integer\n"), "{text}");
    assert!(text.contains("expected an expression"), "{text}");
}

#[test]
fn a_command_or_a_blank_line_ends_the_entry_a_comment_does_not() {
    for (input, at) in [
        ("x = 1\n:t\n+ 2\n\n", "1:1: expected "),
        ("x = 1\n\n\n+ 2\n\n", "1:1: expected "),
        // A comment line is the entry's text, blank to the grammar.
        ("x = 1\n# note\n+ 2\n\n", "3:1: expected "),
    ] {
        let text = written(Repl::new().feed(input.as_bytes()));
        assert!(text.contains(at), "{input:?}: {text}");
        assert!(text.contains("\n+ 2\n^"), "{input:?}: {text}");
    }
    let mut repl = Repl::new();
    let outs = repl.feed(b":load k\n");
    assert_eq!(outs, [Out::File("k.fipe".into())]);
    written(repl.file("k.fipe", Some("pub k = 1\n")));
    let text = written(repl.feed(b"+ 2\n\n"));
    assert!(text.starts_with("1:1: expected "), "{text}");
}

#[test]
fn load_of_a_second_file_for_a_loaded_module_changes_nothing() {
    let mut repl = Repl::new();
    repl.feed(b":load 01-files\n");
    written(repl.file("01-files.fipe", Some("pub k = 1\n")));
    let text = written(repl.feed(b":load 02-files\n"));
    assert_eq!(text, "02-files: already loaded as `files`\nfipe> ");
    assert_eq!(
        written(repl.feed(b":t files.k\n")),
        "files.k: Integer\nexact 1\nknown at compile\nfipe> "
    );
    let outs = repl.feed(b":load 01-files\n");
    assert_eq!(
        outs,
        [Out::Write(b"01-files as files\nfipe> ".to_vec())],
        "reloading the same file is still allowed"
    );
}

#[test]
fn a_diagnostic_past_column_65535_underlines_without_a_panic() {
    let line = format!("{}nope\n\n", " ".repeat(70_000));
    let text = written(Repl::new().feed(line.as_bytes()));
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[0], "1:70001: unknown name");
    assert_eq!(rows[2], format!("{}^^^^", " ".repeat(70_000)));
}

#[test]
fn a_leading_assign_continues_the_line_before() {
    // `  = 2` joins `x = 1` rather than standing alone, where it would
    // be rejected at 1:3.
    let text = written(Repl::new().feed(b"x = 1\n  = 2\n:t x\n"));
    assert!(!text.starts_with("1:"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    // A signature and its `= value` line are one entry: either the pair
    // is accepted, or it is rejected at its second line, never the
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
    let text = written(Repl::new().feed(b"x: i64 = 1\n\n"));
    assert_eq!(
        text,
        "1:4: numeric types are uppercase: write `I64`\n\
         x: i64 = 1\n   ^^^\nfipe> "
    );
}

#[test]
fn ast_shows_a_checked_tuple() {
    let text = written(Repl::new().feed(b"pair = (1, 2)\n:ast\n"));
    assert_eq!(
        text,
        "Bind\n  Name pair\n  Tuple\n    Int 1\n    Int 2\nfipe> "
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
        "Type P\n  Field x\n    Type I64\n  Where\n    Binary <=\n      Int 0\n      Name self.x\n\
         P 1: P\nnames P\nparts x: 1\nknown at compile\nfipe> "
    );
}

#[test]
fn member_schemes_and_type_parameter_defaults_are_visible() {
    let text = written(Repl::new().feed(b"scale = v k => v * k\n:t scale\n"));
    assert_eq!(
        text,
        "scale: Mul<b> a => a -> b -> a\nknown at compile\nfipe> "
    );
    let text = written(
        Repl::new()
            .feed(b"type Op<R = Self, O = Self> where self.op: R -> O\n:ast\n"),
    );
    assert_eq!(text.matches("Default").count(), 2, "{text}");
    let text = written(Repl::new().feed(b"type V: Mul<F64> { x: F64 }\nimpl V: Mul<F64> { mul = k self => Self (self.x * k) }\nv = V 1.0\n:t v * 2.0\n"));
    assert!(
        text.contains("V does not have a ready claim for Literal"),
        "{text}"
    );
    let text = written(Repl::new().feed(b"n = 1\n:t n.nope\n"));
    assert!(
        text.contains("no such field or member on Integer"),
        "{text}"
    );
    let text =
        written(Repl::new().feed(b"c = True\n:t if c { 2 } else { \"a\" }\n"));
    assert!(
        text.contains("no meaning reads all literal forms: Whole, Text"),
        "{text}"
    );
}

#[test]
fn docs_are_displayed_above_types_and_under_tree_owners() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"## A value\n")), "....> ");
    assert_eq!(
        written(repl.feed(b"\nx = 1\n:t x\n")),
        "A value\nx: Integer\nexact 1\nknown at compile\nfipe> "
    );
    let text = written(
        repl.feed(b"## A record\ntype P {\n## A field\nx: I64,\n}\n:ast\n"),
    );
    assert!(
        text.contains(
            "Type P\n  Doc \"A record\"\n  Field x\n    Doc \"A field\""
        ),
        "{text}"
    );
    let outs = repl.feed(b":host\n:t ls\n");
    assert!(matches!(outs.as_slice(), [Out::File(_)]));
    let mut text = String::new();
    for (index, (path, source)) in HOST.iter().enumerate() {
        let outs = repl.file(path, Some(source));
        if index + 1 == HOST.len() {
            text = written(outs);
        } else {
            assert!(matches!(outs.as_slice(), [Out::File(_)]));
        }
    }
    assert!(text.contains("List directory entries.\n- path:"), "{text}");
    assert!(text.contains("- depth: how far down to look"), "{text}");
    assert!(
        text.find("List directory entries.").unwrap()
            < text.find("ls:").unwrap()
    );
}

#[test]
fn quotes_continue_across_physical_input_lines() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"banner = 'first\n")), "....> ");
    assert_eq!(written(repl.feed(b"\n# text\n:q\n")), "....> ");
    let text = written(repl.feed(b"last'\n:ast\n"));
    assert!(
        text.contains("Str \"first\\n\\n# text\\n:q\\nlast\""),
        "{text}"
    );
    assert_eq!(written(repl.feed(b"msg = \"the file \\\n")), "....> ");
    let text = written(repl.feed(b"   was not found\"\n:ast\n"));
    assert!(text.contains("Str \"the file was not found\""), "{text}");
    let text = written(repl.feed(b"x = \"unterminated\n\ny = 2\n:t y\n"));
    assert!(text.contains("unterminated quote"), "{text}");
    assert!(text.contains("y: Integer"), "{text}");
}

#[test]
fn ast_shows_impl_claim() {
    let text = written(
        Repl::new()
            .feed(b"impl Vec2: Mul<F64> { mul = k self => self }\n:ast\n"),
    );
    assert!(text.contains("Claim"), "{text}");
    assert!(text.contains("Mul"), "{text}");
    assert!(text.contains("F64"), "{text}");
}

#[test]
fn host_replays_whole_entries() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"x = 'a\n\nb'\n\n## Doc.\n\nv = 7\n:host\n");
    let text = host(&mut repl, outs);
    assert!(!text.contains("quote") && !text.contains("owner"), "{text}");
    let text = written(repl.feed(b":t x\n:t v\n"));
    assert!(
        text.contains("x: String") && text.contains("Doc.\nv: Integer"),
        "{text}"
    );
}

#[test]
fn documented_heads_continue() {
    let text = written(
        Repl::new().feed(b"## Doubles.\nf: I64 -> I64\n  = x => x * 2\n:t f\n"),
    );
    assert!(text.contains("Doubles.\nf: I64 -> I64"), "{text}");
    for source in [
        "## Positive.\ntype Pos: I64\nwhere self > 0\n:ast\n",
        "## Positive.\ntype Pos: I64 where\nself > 0\n:ast\n",
    ] {
        let text = written(Repl::new().feed(source.as_bytes()));
        assert!(text.contains("Binary >"), "{text}");
        assert!(
            !text.contains("expected") && !text.contains("group an"),
            "{text}"
        );
    }
}

#[test]
fn bad_raw_quote_keeps_its_closing_line() {
    for bad in ['\u{200b}', '\r'] {
        let source = format!("x = 'a{bad}c\nb'\n\ny = 2\n:t y\n");
        let text = written(Repl::new().feed(source.as_bytes()));
        assert!(
            text.contains("unexpected character")
                && text.contains("y: Integer"),
            "{text}"
        );
    }
}

#[test]
fn inline_doc_does_not_hold_next_line() {
    let text = written(Repl::new().feed(b"x = 1 ## note\n\ny = 2\n:t y\n"));
    assert!(
        text.contains("expected the end") && text.contains("y: Integer"),
        "{text}"
    );
    let text = written(Repl::new().feed(b"x = [## d\n\ny = 1\n]\n:ast\n"));
    assert!(text.contains("Doc \"d\""), "{text}");
}

#[test]
fn raw_quote_scans_new_lines_only() {
    let mut repl = Repl::new();
    let start = std::time::Instant::now();
    repl.feed(b"x = '\n");
    for _ in 0..2_000 {
        repl.feed(b"0123456789012345678901234567890123456789\n");
    }
    let text = written(repl.feed(b"'\n:t x\n"));
    assert!(text.contains("x: String"), "{text}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn trailing_join_continues() {
    let text =
        written(Repl::new().feed(b"f = a b => a\ng = f 1 \\\n  2\n:t g\n"));
    assert!(text.contains("g: Integer"), "{text}");
}

#[test]
fn type_echo_preserves_quote_bytes() {
    let text = written(Repl::new().feed(b":t 'a\n  b'\n"));
    assert!(text.contains("'a\n  b': String"), "{text}");
}

#[test]
fn empty_module_doc_has_a_tree() {
    let text = written(Repl::new().feed(b"##! only a module doc\n\n:ast\n"));
    assert!(text.contains("Module doc \"only a module doc\""), "{text}");
}

#[test]
fn the_lines_before_a_blank_line_are_one_unit() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"x = 1\ny = x\n")), "....> ");
    let text = written(repl.feed(b"\n:ast\n:t y\n"));
    assert!(text.contains("Bind\n  Name x\n"), "{text}");
    assert!(text.contains("Bind\n  Name y\n"), "{text}");
    assert!(text.contains("y: Integer"), "{text}");
    let mut repl = Repl::new();
    repl.feed(b"x = 1\n");
    assert_eq!(written(repl.flush()), "\n");
    assert!(written(repl.feed(b":t x\n")).contains("x: Integer"));
}

#[test]
fn one_unit_checks_mutual_claims() {
    let source = b"type A: From<B> {}\ntype B: From<A> {}\nimpl A: From<B> { from: B -> Self = b => Self {} }\nimpl B: From<A> { from: A -> Self = a => Self {} }\n\na = A {}\nb = a | to<B>\n:t b\n";
    assert_eq!(
        written(Repl::new().feed(source)),
        "b: B\nknown at compile\nfipe> "
    );
}

#[test]
fn leading_division_continues() {
    let text = written(Repl::new().feed(b"8\n/ 2\n:t\n"));
    assert!(text.contains("8 / 2: Integer"), "{text}");
}

#[test]
fn queries_are_independent_of_read_boundaries() {
    for source in [":t 1\n+ 2\n", ":r 1\n+ 2\n"] {
        let together = written(Repl::new().feed(source.as_bytes()));
        let mut repl = Repl::new();
        let mut separate = String::new();
        for line in source.split_inclusive('\n') {
            separate.push_str(&written(repl.feed(line.as_bytes())));
        }
        assert_eq!(
            together.replace("fipe> ", ""),
            separate.replace("fipe> ", "")
        );
    }
}

#[test]
fn opaque_head_waits_for_where() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"type Shape\n")), "....> ");
    let text =
        written(repl.feed(
            b"where self.area: F64\nx: Shape -> F64 = s => s.area\n:t x\n",
        ));
    assert_eq!(text, "x: Shape -> F64\nknown at compile\nfipe> ");
}

#[test]
fn signature_does_not_swallow_next_binding() {
    let text = written(
        Repl::new()
            .feed(b"sig: Integer -> Integer\n\ny = 1\n:t y\nz =\n5\n:t z\n"),
    );
    assert!(
        text.contains("y: Integer") && text.contains("z: Integer"),
        "{text}"
    );
}

#[test]
fn unknown_impl_has_one_accurate_error() {
    let text =
        written(Repl::new().feed(b"impl Unknown { f = self => self }\n\n"));
    assert!(
        text.contains("unknown type") && !text.contains("another module"),
        "{text}"
    );
}

#[test]
fn commands_use_words() {
    let text = written(Repl::new().feed(b"x = 1\n:r x\n:load\nSelf\n\n"));
    assert!(
        !text.contains("Local {") && !text.contains("Span {"),
        "{text}"
    );
    assert!(
        text.contains("<stem or path>") && text.contains("`Self` outside"),
        "{text}"
    );
}

#[test]
fn checks_print_the_named_scheme_and_the_same_facts_as_type_queries() {
    let mut repl = Repl::new();
    let text = written(repl.feed(b"a: U8 = 200\nb: U16 = 1000\n:t a + b\nsum2: U8 -> U8 -> U8 = x y => x + y\n:checks sum2\n"));
    assert!(text.contains("a + b: Integer\nexact 1200\n"), "{text}");
    assert!(text.contains("sum2: U8 -> U8 -> U8\n"), "{text}");
    assert_eq!(text.matches("eval, per call").count(), 1, "{text}");
    assert!(
        text.contains("eval, per call: the result, range 0..510, against U8 (unknown input)\n"),
        "{text}"
    );
}

#[test]
fn strict_mode_rejects_implicit_checks_and_survives_host_reload() {
    for reload in [false, true] {
        let mut repl = Repl::new();
        repl.strict(true);
        if reload {
            let outs = repl.feed(b":host\n");
            host(&mut repl, outs);
        }
        let text = written(repl.feed(
            b"sum2: U8 -> U8 -> U8 = x y => x + y\n\ng = x => U8 x\n:t g\n",
        ));
        assert!(text.contains("open entry"), "{text}");
        assert!(text.contains("g: Integer -> Integer"), "{text}");
        repl.strict(false);
        let text = written(
            repl.feed(b"sum2: U8 -> U8 -> U8 = x y => x + y\n:t sum2\n"),
        );
        assert!(!text.contains("open entry"), "{text}");
        assert!(text.contains("sum2: U8 -> U8 -> U8"), "{text}");
    }
}

#[test]
fn checks_name_places_and_disambiguate_identical_results() {
    let text = written(Repl::new().feed(b"is_prime = n => n > 1\n\ntype Prime: Integer where is_prime self\n\nn: Prime = 7\n\nf: U8 -> U8 = x => x + 1\n\nh: U8 -> U8 = x => x + 1\n\ntake: U8 -> Integer = amount => amount\n\nuse_it = x => take x\n\ntype Box { value: U8 }\n\nbox_it = x => Box x\n\nwrap: Integer -> Maybe<U8> = x => Just x\n\nitems: Vec<Integer> -> Vec<U8> = xs => xs\n:checks\n"));
    for expected in [
        "binding n, exact 7, against Prime",
        "parameter amount",
        "field Box.value",
        "a payload",
        "an element",
        "the result at ",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(text.matches("the result at ").count(), 2, "{text}");
    assert!(text.contains("the result at 1:10,"), "{text}");
    assert!(text.contains("the result at 1:10,"), "{text}");
    assert!(!text.contains("entry at"), "{text}");
    let named =
        written(Repl::new().feed(b"f: U8 -> U8 = x => x + 1\n:checks f\n"));
    assert!(!named.contains("the result at "), "{named}");
}

#[test]
fn timing_and_holding_facts_are_visible_on_both_driver_paths() {
    let mut repl = Repl::new();
    let outs = repl.feed(b":host\n");
    host(&mut repl, outs);
    let text = written(repl.feed(b"use cli config\ntype Port: Integer where 0 < self, self <= config.max_port\nport: Port = cli.port\n:t port\n:checks port\nx = Compile (2 + 3)\n:t x\nrgb = cli.rgb\n:t rgb\n"));
    assert!(text.contains("known at init"), "{text}");
    assert!(text.contains("init, once:"), "{text}");
    assert!(text.contains("against Port: 0 < self"), "{text}");
    assert!(
        text.contains("against Port: self <= config.max_port"),
        "{text}"
    );
    assert!(!text.contains("binding port at "), "{text}");
    assert!(text.contains("known at compile"), "{text}");
    assert!(text.contains("held Size<3>"), "{text}");
    let text = written(repl.feed(b"type Flag: Bool & Bits<1>\ntype Mode: Size<1> { read: Flag, write: Flag, exec: Flag }\nm = Mode -read -write=False -exec\n:t m\n"));
    assert!(text.contains("held Size<1>, 3 of 8 bits"), "{text}");
    assert!(
        text.contains("parts read: True, write: False, exec: True"),
        "{text}"
    );
}

#[test]
fn written_tests_keep_sources_and_include_proved_siblings() {
    let mut repl = Repl::new();
    let text = written(repl.feed(b"type Band: Integer where 0 <= self, self < 10\nf: U8 -> Band = n => n\n:checks f\n"));
    assert_eq!(text.matches("against Band:").count(), 1, "{text}");
    assert!(text.contains("against Band: self < 10"), "{text}");
    assert!(!text.contains("the result at "), "{text}");

    assert_eq!(
        repl.feed(b"use bands\n\n"),
        [Out::File("bands.fipe".into())]
    );
    written(repl.file("bands.fipe", Some("pub type Band: Integer where 0 <= self, self < 10\npub f: Integer -> Band = n => n\n")));
    let text = written(repl.feed(b":checks\n"));
    assert!(text.contains("against Band: 0 <= self"), "{text}");
    assert!(text.contains("against Band: self < 10"), "{text}");

    let mut repl = Repl::new();
    let mut outs = repl.feed(b":host\n");
    for (path, source) in HOST {
        assert_eq!(outs, [Out::File(path.into())]);
        let source = if path.ends_with("config.fipe") {
            format!("{source}\n## A host limit.\npub type HostBand: Integer where 0 <= self, self < 10\n")
        } else {
            source.into()
        };
        outs = repl.file(path, Some(&source));
    }
    written(outs);
    let text =
        written(repl.feed(
            b"use config\nf: U8 -> config.HostBand = n => n\n:checks f\n",
        ));
    assert!(text.contains("against HostBand: self < 10"), "{text}");
    assert!(!text.contains("the result at "), "{text}");
}

#[test]
fn container_types_warnings_and_conversion_hint_are_visible() {
    let text = written(Repl::new().feed(b"xs = [1, 2, 3]\n:t xs ++ [4, 5]\ng = x (y = 0) => x\n\ng (1, 2)\n\ntotal: Vec<Integer> -> Integer = xs => xs | sum\n\ntotal -xs xs\n\n"));
    assert!(text.contains("[Integer; 5]"), "{text}");
    assert!(
        text.contains("warning: written as one tuple argument"),
        "{text}"
    );
    assert!(text.contains("to<Vec<Integer>>"), "{text}");
}

#[test]
fn outcome_errors_explain_the_family_and_exit() {
    let text = written(Repl::new().feed(
        b"s: String = Just 1\n\nm: Maybe<Integer> = [!1]\n\nf = [:1]\n\n",
    ));
    assert!(
        text.contains("Maybe<Integer> cannot enter a text place; use || or |:"),
        "{text}"
    );
    assert!(text.contains("Nothing has no payload"), "{text}");
    assert!(text.contains("no outcome family is known here"), "{text}");
}

#[test]
fn holding_type_continues_with_where() {
    let text = written(Repl::new().feed(b"type I8b: Integer & Bits<8>\nwhere -128 <= self, self <= 127\nx: I8b = 1\n:t x\n"));
    assert!(!text.contains("does not fit"), "{text}");
    assert!(text.contains("x: I8b"), "{text}");
}

#[test]
fn global_checks_name_their_functions_with_local_positions() {
    let text = written(Repl::new().feed(
        b"x = 1\n\nf: U8 -> U8 = x => x + 1\n\ng: U8 -> U8 = x => x + 1\n:checks\n",
    ));
    assert!(text.contains("f: eval, per call:"), "{text}");
    assert!(text.contains("g: eval, per call:"), "{text}");
    assert!(text.contains("at 1:10"), "{text}");
    assert!(!text.contains("at 3:10"), "{text}");
    let text = written(Repl::new().feed(b"type L: Len { n: Natural }\nimpl L: Len { len = self => self.n }\n:checks\n"));
    assert!(text.contains("L.len: eval, per call:"), "{text}");
}

#[test]
fn timing_diagnostic_names_the_impure_input() {
    let text = written(Repl::new().feed(b":t Compile (sin 1.0)\n"));
    assert!(
        text.contains("1:10: timing requires compile; input `sin` is not pure"),
        "{text}"
    );
    assert!(text.contains("         ^^^"), "{text}");
}

#[test]
fn a_type_without_a_test_explains_how_to_enter_it() {
    let text = written(
        Repl::new().feed(b"type Flag: Bool & Bits<1>\n\nFlag True\n\n"),
    );
    assert!(
        text.contains("`Flag` has no test, so it is no gate"),
        "{text}"
    );
    assert!(text.contains("to<Flag>"), "{text}");
}

#[test]
fn ir_shows_collected_supplies_modes_and_rebuilds() {
    let mut repl = Repl::new();
    let text =
        written(repl.feed(
            b"g: Vec<Integer> -> Integer = xs => len xs\n\n:ir g 1 2 3\n",
        ));
    assert!(text.contains("collected into spot 0"), "{text}");
    let text = written(repl.feed(b":ir Nothing || 4\n"));
    assert!(text.contains("or values"), "{text}");
    assert!(text.contains("by payload"), "{text}");
    let text = written(repl.feed(b":ir Just 1 && Just 2\n"));
    assert!(text.contains("by family"), "{text}");
    let text = written(
        repl.feed(b"m: Maybe<Integer> = Just 1\n\n:ir m |: (x => x + 1)\n"),
    );
    assert!(text.contains("good side of value"), "{text}");
    assert!(text.contains("rebuilt by wrapping"), "{text}");
}

#[test]
fn ir_queries_show_checked_operations_without_accepting() {
    let mut repl = Repl::new();
    let text = written(repl.feed(b"x = 1\n:ir x + 2\n:ir\n"));
    assert!(text.contains("checked unit:"), "{text}");
    assert!(text.contains("call value"), "{text}");
    assert!(!text.contains("Span"), "{text}");
    assert!(!text.contains("start:"), "{text}");
    let text = written(repl.feed(b":t x\n"));
    assert!(text.contains("Integer"), "{text}");
}

#[test]
fn commands_accept_tabs_and_reject_extra_arguments() {
    let mut repl = Repl::new();
    assert!(written(repl.feed(b":t\t1\n")).contains("1: Integer"));
    assert!(written(repl.feed(b":q extra\n")).contains("unknown command"));
    assert!(written(repl.feed(b":ast extra\n")).contains("usage: :ast"));
    assert_eq!(
        repl.feed(b":host\tdirectory\n"),
        [Out::File("directory/commands.fipe".into())]
    );
}

#[test]
fn host_directory_is_a_path_not_repl_input() {
    let mut repl = Repl::new();
    assert_eq!(
        repl.host_directory("dir\n:q"),
        [Out::File("dir\n:q/commands.fipe".into())]
    );
    assert!(written(Repl::new().host_directory("dir\0suffix")).contains("NUL"));
}

#[test]
fn source_controls_are_visible_and_carets_follow_rendered_columns() {
    let text = written(Repl::new().feed(b"\tmissing\n\n"));
    assert!(text.contains("\\tmissing\n  ^^^^^^^"), "{text}");
    let text = written(Repl::new().feed(b":t 'a\x1bb'\n"));
    assert!(!text.contains('\x1b'), "{text}");
    assert!(text.contains("\\u{1b}"), "{text}");
}

#[test]
fn ast_shows_visibility_versions_and_member_annotations() {
    let mut repl = Repl::new();
    written(repl.feed(b"pub type P { pub x: Integer }\n"));
    let text = written(repl.feed(b":ast\n"));
    assert!(
        text.contains("Type P pub") && text.contains("Field pub x"),
        "{text}"
    );
    written(repl.feed(b"impl P { pub f: Integer -> Integer = n => n }\n"));
    let text = written(repl.feed(b":ast\n"));
    assert!(
        text.contains("Member pub f") && text.contains("Function"),
        "{text}"
    );
    let mut repl = Repl::new();
    repl.feed(b"use text 2\n\n");
    let text = written(repl.file("text.fipe", Some("pub x = 1\n")));
    assert!(!text.is_empty());
    assert!(written(repl.feed(b":ast\n")).contains("Use text version 2"));
}

#[test]
fn a_conversion_without_candidates_ends_its_message() {
    let text = written(Repl::new().feed(b":t to<F64> 3\n"));
    assert!(
        text.contains("no conversion or member of F64 accepts Integer\n"),
        "{text}"
    );
}

#[test]
fn mir_shows_bodies_locals_blocks_and_charges_in_words() {
    let mut repl = Repl::new();
    let text =
        written(repl.feed(b"f = (w = 80) (c = (w / 8)) => c\n\n:mir f 16\n"));
    assert!(text.contains("body 0: definition f"), "{text}");
    assert!(text.contains("constant 0 = 16"), "{text}");
    assert!(text.contains("_1 = constant 0"), "{text}");
    assert!(text.contains("call native prelude.div"), "{text}");
    assert!(
        text.contains("call body 0 (_1, _2) with meanings (Integer) into _0"),
        "{text}"
    );
    assert!(text.contains("block 0 charges"), "{text}");
    assert!(!text.contains("Span"), "{text}");
    let text = written(repl.feed(b":mir if 1 == 1 { 2 } else { 3 }\n"));
    assert!(
        text.contains("switch _3: True to block 2, False to block 3"),
        "{text}"
    );
    assert!(text.contains("call native prelude.eq"), "{text}");
    let text = written(repl.feed(b":mir with cwd /tmp { 1 }\n"));
    assert!(text.contains("push cwd = constant 0"), "{text}");
    assert!(text.contains("    pop\n"), "{text}");
}

#[test]
fn mir_without_an_argument_shows_the_last_accepted_line() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b":mir\n")), "no accepted line\nfipe> ");
    let text = written(repl.feed(b"x = 2.5\n:mir\n"));
    assert!(text.contains("constant 0 = 2.5"), "{text}");
    assert!(text.contains("global x = constant 0"), "{text}");
    let text = written(repl.feed(b":mir (x, \"a\")\n"));
    assert!(text.contains("constant 0 = \"a\""), "{text}");
    assert!(text.contains("tuple (global x, constant 0)"), "{text}");
}

#[test]
fn an_accepted_line_prints_its_value() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"1 + 2\n\n")), "3\nfipe> ");
    assert_eq!(
        written(repl.feed(b"[1, 2] | map (n => n + 1)\n\n")),
        "[2, 3]\nfipe> "
    );
    assert_eq!(written(repl.feed(b"\"a\" ++ \"b\"\n\n")), "\"ab\"\nfipe> ");
    assert_eq!(
        written(repl.feed(b"Just 1 |: (n => Just (n + 1))\n\n")),
        "Just 2\nfipe> "
    );
    assert_eq!(written(repl.feed(b"x => x\n\n")), "<function>\nfipe> ");
}

#[test]
fn a_binding_prints_nothing_and_its_value_serves_later_lines() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"x = 1\n\n")), "fipe> ");
    assert_eq!(written(repl.feed(b"add2 = n => n + 2\n\n")), "fipe> ");
    assert_eq!(written(repl.feed(b"add2 x\n\n")), "3\nfipe> ");
}

#[test]
fn a_fault_names_its_line_and_column() {
    let mut repl = Repl::new();
    assert_eq!(
        written(repl.feed(b"1 / ([0] | head)\n\n")),
        "fault at 1:5: division by zero: divisor is 0\n\
         1 / ([0] | head)\n    ^^^^^^^^^^^^\nfipe> "
    );
    let text = written(repl.feed(b"f = n => 10 / n\n\nf 0\n\n"));
    assert!(
        text.contains("fault at line 2, 1:15: division by zero: divisor is 0"),
        "{text}"
    );
}

#[test]
fn the_budget_reaches_the_machine() {
    let mut repl = Repl::new();
    repl.budget(5);
    assert_eq!(written(repl.feed(b"1 + 2\n\n")), "3\nfipe> ");
    // `1 + 2` costs five: its operator, two literals, the call, one limb.
    repl.budget(4);
    let text = written(repl.feed(b"1 + 2\n\n"));
    assert!(
        text.starts_with("fault at 1:1: the budget is exhausted"),
        "{text}"
    );
}

#[test]
fn a_refuted_entry_names_its_limit_and_value_at_its_place() {
    let mut repl = Repl::new();
    let text = written(repl.feed(b"big = _ => 300\n\nx: U8 = big ()\n\n"));
    assert!(
        text.contains("fault at 1:9: refuted entry against U8: 300"),
        "{text}"
    );
}

/// A primality test written in the language.
const PRIME: &[u8] = b"trial = n d => if d * d > n { True } else { if n % d == 0 { False } else { trial n (d + 1) } }\n\n\
is_prime = n => n > 1 && trial n 2\n\n\
type Prime: Integer where is_prime self\n\n";

#[test]
fn a_named_test_on_an_exact_value_is_proved_at_compile_time() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(PRIME)), "fipe> ");
    assert_eq!(written(repl.feed(b"p: Prime = 7\n\n")), "fipe> ");
    assert_eq!(
        written(repl.feed(b":checks p\n")),
        "p: Prime\nexact 7\nnames Prime\nknown at compile\n\
         compile, once: binding p, exact 7, against Prime: \
         proved at compile time on 7\nfipe> "
    );
    assert_eq!(
        written(repl.feed(b"q: Prime = 8\n\n")),
        "1:4: refuted entry against Prime: 8\nq: Prime = 8\n   ^^^^^\nfipe> "
    );
    assert_eq!(written(repl.feed(b"n: U8 & Prime = 13\n\n")), "fipe> ");
    let text = written(repl.feed(b":checks n\n"));
    assert!(
        text.contains("against Prime: proved at compile time on 13"),
        "{text}"
    );
}

#[test]
fn the_compile_budget_leaves_an_entry_open() {
    let mut repl = Repl::new();
    repl.compile_budget(10);
    assert_eq!(written(repl.feed(PRIME)), "fipe> ");
    assert_eq!(written(repl.feed(b"p: Prime = 7\n\n")), "fipe> ");
    assert_eq!(
        written(repl.feed(b":checks p\n")),
        "p: Prime\nexact 7\nnames Prime\nknown at compile\n\
         compile, once: binding p, exact 7, against Prime \
         (compile budget exhausted)\nfipe> "
    );
}

#[test]
fn a_program_literal_that_faults_is_a_check_error() {
    let mut repl = Repl::new();
    let mask = b"type Mask: Literal { bits: U8 }\n\n\
impl Mask: Literal {\n    literal = {\n        Whole { value, radix: 2, digits: 8 } => Self value,\n        _ => fault \"a Mask is written as eight binary digits\",\n    },\n}\n\n";
    assert_eq!(written(repl.feed(mask)), "fipe> ");
    assert_eq!(
        written(repl.feed(b"m: Mask = 0b0000_1111\n\nm\n\n")),
        "Mask 15\nfipe> "
    );
    assert_eq!(
        written(repl.feed(b"k: Mask = 15\n\n")),
        "1:11: the literal Faults reading Mask: a Mask is written as eight \
         binary digits\nk: Mask = 15\n          ^^\nfipe> "
    );
}

#[test]
fn a_compile_demand_is_shown_exact() {
    let mut repl = Repl::new();
    assert_eq!(written(repl.feed(b"f = n => n * 6\n\n")), "fipe> ");
    assert_eq!(written(repl.feed(b"x = Compile (f 7)\n\n")), "fipe> ");
    assert_eq!(
        written(repl.feed(b":t x\n")),
        "x: Integer\nexact 42\nknown at compile\nfipe> "
    );
    assert_eq!(
        written(repl.feed(b"z = Compile (1 / ([0] | head))\n\n")),
        "1:18: the value demanded at compile time Faults: division by zero\n\
         z = Compile (1 / ([0] | head))\n                 ^^^^^^^^^^^^\nfipe> "
    );
}

#[test]
fn a_sequence_that_faults_on_its_bracket_is_a_check_error() {
    let mut repl = Repl::new();
    let pos = b"type Pos: Sequence<Integer> { first: Integer }\n\n\
impl Pos: Sequence<Integer> { sequence = xs => if (xs | head) > 0 { Self (xs | head) } else { fault \"not positive\" } }\n\n";
    assert_eq!(written(repl.feed(pos)), "fipe> ");
    assert_eq!(
        written(repl.feed(b"p: Pos = [0]\n\n")),
        "1:10: the sequence of Pos Faults on this bracket: not positive\n\
         p: Pos = [0]\n         ^^^\nfipe> "
    );
    assert_eq!(
        written(repl.feed(b"q: Pos = [3]\n\nq\n\n")),
        "Pos 3\nfipe> "
    );
}

#[test]
fn an_empty_sum_is_its_meaning_s_zero() {
    let mut repl = Repl::new();
    let vec2 = b"type Vec2: Add & Zero { x: F64, y: F64 }\n\n\
impl Vec2: Add { add = o self => Self (self.x + o.x) (self.y + o.y) }\n\n\
impl Vec2: Zero { zero = Self 0.0 0.0 }\n\n";
    assert_eq!(written(repl.feed(vec2)), "fipe> ");
    assert_eq!(
        written(repl.feed(b"t: Vec2 = [] | sum\n\nt\n\n")),
        "Vec2 0.0 0.0\nfipe> "
    );
}

#[test]
fn a_name_whose_line_faulted_first_has_no_value_when_read() {
    let mut repl = Repl::new();
    let text = written(
        repl.feed(b"stop = n => fault \"stop\"\n\nx: Integer = stop 1\n\n"),
    );
    assert!(text.contains("stop"), "{text}");
    let text = written(repl.feed(b"x + 1\n\n"));
    assert!(
        text.contains("x has no value: its line ended before binding it"),
        "{text}"
    );
}
