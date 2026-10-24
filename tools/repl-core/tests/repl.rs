use fipe_repl_core::{DriveError, Out, Repl, RequestId};
use std::collections::VecDeque;

/// Legacy transcript tests acknowledge only boundaries, never File/Call requests.
fn drain(repl: &mut Repl, outs: Vec<Out>) -> Vec<Out> {
    let mut pending = VecDeque::from(outs);
    let mut result = Vec::new();
    while let Some(out) = pending.pop_front() {
        match out {
            Out::Start { resume, .. }
            | Out::End { resume, .. }
            | Out::Reset { resume, .. } => {
                pending.extend(repl.resume(&resume).unwrap());
            }
            Out::Write(bytes) => match result.last_mut() {
                Some(Out::Write(prior)) => prior.extend(bytes),
                _ => result.push(Out::Write(bytes)),
            },
            other => result.push(other),
        }
    }
    result
}
fn test_feed(repl: &mut Repl, bytes: &[u8]) -> Vec<Out> {
    let outs = repl.feed(bytes);
    drain(repl, outs)
}
fn test_flush(repl: &mut Repl) -> Vec<Out> {
    let outs = repl.flush();
    drain(repl, outs)
}
fn test_file(
    repl: &mut Repl,
    id: &RequestId,
    path: &str,
    text: Option<&str>,
) -> Result<Vec<Out>, DriveError> {
    let outs = repl.file(id, path, text)?;
    Ok(drain(repl, outs))
}
fn test_answer(
    repl: &mut Repl,
    id: &RequestId,
    answer: impl FnOnce(
        &fipe::embed::Request,
        &fipe::ir::Tables,
    ) -> fipe::embed::Reply,
) -> Result<Vec<Out>, DriveError> {
    let outs = repl.answer(id, answer)?;
    Ok(drain(repl, outs))
}

fn test_host_sources(
    repl: &mut Repl,
    dir: &str,
    sources: &[(&str, &str)],
) -> Result<Vec<Out>, DriveError> {
    let outs = repl.host_sources(dir, sources)?;
    Ok(drain(repl, outs))
}

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

/// Extract a real request token while checking the output kind and exact path.
fn file_request(outs: &[Out], expected: &str) -> RequestId {
    let [Out::File { id, path }] = outs else {
        panic!("expected file {expected}: {outs:?}")
    };
    assert_eq!(path, expected);
    id.clone()
}

fn call_request(outs: &[Out]) -> RequestId {
    let [Out::Call { id }] = outs else {
        panic!("expected call: {outs:?}")
    };
    id.clone()
}

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
fn help_lists_every_command_on_one_line() {
    let text = written(test_feed(&mut Repl::new(), b":help\n"));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 11, "{text}");
    for command in [":ast", ":r", ":t", ":ir", ":mir", ":load", ":checks"] {
        assert!(lines.iter().any(|line| line.starts_with(command)), "{text}");
    }
    assert!(lines[8].starts_with(":help [name]"), "{text}");
    assert!(lines[9].starts_with(":q "), "{text}");
    let text = written(test_feed(&mut Repl::new(), b":help :t\n:help :zz\n"));
    assert!(text.starts_with(":t [line]"), "{text}");
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("unknown command :zz\n"), "{text}");
}

#[test]
fn help_on_a_name_prints_its_doc_and_type_as_t_does() {
    let mut repl = Repl::new();
    let help = written(test_feed(&mut repl, b":help len\n"));
    let types = written(test_feed(&mut repl, b":t len\n"));
    assert_eq!(help, types);
    assert!(help.contains("len: "), "{help}");
    let text =
        written(test_feed(&mut repl, b"## The answer.\nx = 42\n\n:help x\n"));
    assert!(text.contains("The answer.\nx: Integer\n"), "{text}");
    let text = written(test_feed(&mut repl, b":help nosuch\n:help 1 + 2\n"));
    assert_eq!(
        text,
        "no name `nosuch` in scope\n\
         usage: :help [name or :command]\nfipe> "
    );
}

#[test]
fn the_first_feed_prompts() {
    assert_eq!(
        test_feed(&mut Repl::new(), &[]),
        [Out::Write(b"fipe> ".to_vec())]
    );
}

#[test]
fn a_local_resolves_to_its_site() {
    let text = written(test_feed(&mut Repl::new(), b"x = 1\n:r x\n"));
    assert_eq!(text, "1:1 x: binding at line 1\nfipe> ");
}

#[test]
fn a_diagnostic_underlines_its_span() {
    let text = written(test_feed(&mut Repl::new(), b"nope 1\n\n"));
    assert_eq!(text, "1:1: unknown name\nnope 1\n^^^^\nfipe> ");
}

#[test]
fn a_partial_line_waits_for_its_end_or_a_flush() {
    let mut repl = Repl::new();
    assert_eq!(test_feed(&mut repl, b"nope"), []);
    assert_eq!(
        written(test_flush(&mut repl)),
        "1:1: unknown name\nnope\n^^^^\n"
    );
    assert_eq!(test_flush(&mut repl), []);
}

#[test]
fn use_asks_for_the_file_and_resumes() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"use geometry.Point\nPoint 1\n:r Point\n");
    let id = file_request(&outs, "geometry.fipe");
    let text = written(
        test_file(&mut repl, &id, "geometry.fipe", Some(GEOMETRY)).unwrap(),
    );
    assert!(
        text.contains("1:1 Point: constructor geometry.Point.Point"),
        "{text}"
    );
}

#[test]
fn load_asks_for_the_stem() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load geometry\n");
    let id = file_request(&outs, "geometry.fipe");
    let text = written(
        test_file(&mut repl, &id, "geometry.fipe", Some(GEOMETRY)).unwrap(),
    );
    assert!(!text.contains("expected F64"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    let text = written(test_feed(&mut repl, b":r geometry.Point 1\n"));
    assert!(text.contains("constructor"), "{text}");
}

#[test]
fn load_takes_a_stem_or_a_path() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load 01-files\n");
    let id = {
        let [Out::Write(bytes), Out::File { id, path }] = outs.as_slice()
        else {
            panic!("{outs:?}")
        };
        assert_eq!(bytes, b"01-files as files\n");
        assert_eq!(path, "01-files.fipe");
        id.clone()
    };
    let text = written(
        test_file(&mut repl, &id, "01-files.fipe", Some("pub k = 1\n"))
            .unwrap(),
    );
    assert_eq!(text, "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b":t files.k\n")),
        "files.k: Integer\nexact 1\nknown at compile\nfipe> "
    );
    let outs = test_feed(&mut repl, b":load ./x/y.fipe\n");
    let id = file_request(&outs, "./x/y.fipe");
    let text = written(test_file(&mut repl, &id, "./x/y.fipe", None).unwrap());
    assert!(text.starts_with("1:5: unknown module\nuse y\n"), "{text}");
    let outs = test_feed(&mut repl, b":load shapes.fipe\n");
    file_request(&outs, "shapes.fipe");
}

#[test]
fn load_without_a_stem_and_unknown_commands_say_so() {
    let text =
        written(test_feed(&mut Repl::new(), b":load\n:bogus 1\n:ast x\n"));
    assert_eq!(
        text,
        "usage: :load <stem or path>\nunknown command :bogus\n\
         usage: :ast (the last line that parsed)\nfipe> "
    );
}

#[test]
fn a_file_diagnostic_is_located_in_its_file() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load bad\n");
    let id = file_request(&outs, "bad.fipe");
    let text = written(
        test_file(
            &mut repl,
            &id,
            "bad.fipe",
            Some("x = 1\npub g = 1 + \"a\"\n"),
        )
        .unwrap(),
    );
    assert_eq!(
        text,
        "bad.fipe:2:9: no meaning reads all literal forms: Whole, Text\n\
         pub g = 1 + \"a\"\n        ^^^^^^^\nfipe> "
    );
}

#[test]
fn an_absent_file_is_an_unknown_module() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"use missing\n\n");
    let id = file_request(&outs, "missing.fipe");
    let text =
        written(test_file(&mut repl, &id, "missing.fipe", None).unwrap());
    assert_eq!(
        text,
        "1:5: unknown module\nuse missing\n    ^^^^^^^\nfipe> "
    );
}

#[test]
fn a_private_inherent_member_is_named() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"use shapes.P\n\n");
    let id = file_request(&outs, "shapes.fipe");
    let text = written(
        test_file(
            &mut repl,
            &id,
            "shapes.fipe",
            Some("pub type P { pub x: I64 }\nimpl P { hidden = self => 1 }\n"),
        )
        .unwrap(),
    );
    assert_eq!(text, "fipe> ");
    let text = written(test_feed(&mut repl, b"P.hidden\n\n"));
    assert!(text.contains("member `hidden` is private"), "{text}");
}

/// Answer `:host`'s six requests with the fixtures.
fn host(repl: &mut Repl, mut outs: Vec<Out>) -> String {
    for (path, text) in &HOST {
        let id = file_request(&outs, path);
        outs = test_file(repl, &id, path, Some(text)).unwrap();
    }
    written(outs)
}

#[test]
fn host_loads_the_fixtures_and_keeps_accepted_lines() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"x = 1\n:host\n");
    assert_eq!(host(&mut repl, outs), "host: tests/fixtures/host\nfipe> ");
    let text = written(test_feed(&mut repl, b"ls -l /etc\n:r ls\n:r x\n"));
    assert!(text.contains("ls: value commands.ls"), "{text}");
    assert!(text.contains("x: binding at line 1"), "{text}");
}

#[test]
fn a_vertical_pipeline_is_one_entry() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":host\n");
    host(&mut repl, outs);
    let text = written(test_feed(&mut repl,
        b"ls -l /var/log\n    | filter (e => e.size > 1)\n    | map .name\n:t\n",
    ));
    // The line runs and asks the host, which the REPL does not wire yet;
    // its run Faults, so the line is withdrawn.
    assert_eq!(
        text,
        "fault at 1:7: host: no host for commands.read\nls -l /var/log\n      \
         ^^^^^^^^\nno accepted line\nfipe> "
    );
    let text = written(test_feed(&mut repl,
        b"names = _ => ls -l /var/log\n    | filter (e => e.size > 1)\n    | map .name\n:t\n",
    ));
    assert!(text.starts_with("names: a -> Vec<String>\n"), "{text}");
}

#[test]
fn ast_shows_the_last_accepted_line_as_a_tree() {
    let text = written(test_feed(&mut Repl::new(), b"f = x => x + 1\n:ast\n"));
    assert_eq!(
        text,
        "Bind\n  Name f\n  Arms\n    Arm\n      Name x\n      Binary +\n\
         \x20       Name x\n        Int 1\nfipe> "
    );
}

#[test]
fn t_shows_the_last_binding_s_scheme() {
    let text =
        written(test_feed(&mut Repl::new(), b"add = a b => a + b\n:t\n"));
    let line = text.lines().next().unwrap();
    assert!(line.starts_with("add: "), "{text}");
    assert!(line.contains("->"), "{text}");
}

#[test]
fn t_queries_an_expression_s_type() {
    let text = written(test_feed(&mut Repl::new(), b":t 1 + 2\n"));
    assert_eq!(text, "1 + 2: Integer\nexact 3\nknown at compile\nfipe> ");
}

#[test]
fn t_reports_a_type_mismatch() {
    let text = written(test_feed(&mut Repl::new(), b"n = 1\n:t n + \"a\"\n"));
    let line = text.lines().next().unwrap();
    assert!(line.contains("expected"), "{text}");
    assert!(line.contains("found"), "{text}");
}

#[test]
fn q_exits() {
    assert_eq!(test_feed(&mut Repl::new(), b":q\nx\n"), [Out::Exit]);
}

#[test]
fn a_redefinition_in_a_later_entry_shadows() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"f = x => x\n\nf = x => x + 1\n:t f\n",
    ));
    assert_eq!(
        text,
        "f: Add a, Literal a => a -> a\nknown at compile\nfipe> "
    );
}

#[test]
fn a_duplicate_in_one_entry_names_its_first_site() {
    let text = written(test_feed(&mut Repl::new(), b"type B { C,\n  C }\n\n"));
    assert_eq!(
        text,
        "2:3: already defined: first defined at 1:10\n  C }\n  ^\nfipe> "
    );
}

#[test]
fn an_ambiguity_names_its_class() {
    let text = written(test_feed(&mut Repl::new(), b"[] == []\n\n"));
    let line = text.lines().next().unwrap();
    assert_eq!(line, "1:4: ambiguous: `Eq` on an unknown type");
}

#[test]
fn a_syntax_error_names_what_it_expected_in_words() {
    let text = written(test_feed(&mut Repl::new(), b"q = (1 ]\n\n"));
    assert!(text.starts_with("1:8: expected )\n"), "{text}");
    assert!(!text.contains("Symbol"), "{text}");
}

#[test]
fn a_mismatch_prints_types_by_display() {
    let text = written(test_feed(&mut Repl::new(), b"n = 1\n\nn + \"a\"\n\n"));
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
    let outs = test_feed(&mut repl, b"use primes.Prime primes.next\n\n");
    let id = file_request(&outs, "primes.fipe");
    written(test_file(&mut repl, &id, "primes.fipe", Some(PRIMES)).unwrap());
    let text = written(test_feed(&mut repl, b"type Prime { s: String }\n\n"));
    assert_eq!(text, "fipe> ");
    let text = written(test_feed(&mut repl, b"next (Prime \"x\")\n\n"));
    assert_eq!(
        text,
        "1:7: expected primes.Prime, found session[2].Prime\n\
         next (Prime \"x\")\n      ^^^^^^^^^\nfipe> "
    );
}

#[test]
fn the_identity_and_literal_smokes() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"use primes.Prime primes.next\n\n");
    let id = file_request(&outs, "primes.fipe");
    written(test_file(&mut repl, &id, "primes.fipe", Some(PRIMES)).unwrap());
    let text = written(test_feed(&mut repl, b":t Prime.check\n:t 2 + 2.5\n"));
    assert_eq!(text, "Prime.check: Prime -> Bool\nknown at compile\n2 + 2.5: F64\nknown at compile\nheld Size<8>\nfipe> ");
    let text =
        written(test_feed(&mut repl, b"p = Prime 7\n\nnext (p.n + 2)\n\n"));
    assert_eq!(
        text,
        "1:7: expected Prime, found Integer\n\
         next (p.n + 2)\n      ^^^^^^^\nfipe> "
    );
}

#[test]
fn an_entry_is_held_until_a_blank_line_or_a_command() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"y = (1 +\n")), "....> ");
    assert_eq!(written(test_feed(&mut repl, b"2\n")), "....> ");
    assert_eq!(written(test_feed(&mut repl, b")\n")), "....> ");
    assert_eq!(
        written(test_feed(&mut repl, b":t\n")),
        "y: Integer\nexact 3\nknown at compile\nfipe> "
    );
    // A command's own bracket is closed first; then it runs at once.
    assert_eq!(written(test_feed(&mut repl, b":t (1 +\n")), "....> ");
    assert_eq!(
        written(test_feed(&mut repl, b"  2)\n")),
        "(1 + 2): Integer\nexact 3\nknown at compile\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"type K: Integer &\n")),
        "....> "
    );
    assert_eq!(written(test_feed(&mut repl, b"  Natural\n")), "....> ");
    assert_eq!(written(test_feed(&mut repl, b"\n")), "fipe> ");
    assert!(written(test_feed(&mut repl, b":t K\n")).starts_with("K: "));
}

#[test]
fn a_leading_operator_continues_a_line_given_with_it() {
    let text = written(test_feed(&mut Repl::new(), b"z = 1\n  + 2\n:t\n"));
    assert_eq!(text, "z: Integer\nexact 3\nknown at compile\nfipe> ");
}

#[test]
fn a_blank_line_inside_a_bracket_stays_in_the_entry() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"w = (1\n")), "....> ");
    assert_eq!(written(test_feed(&mut repl, b"\n")), "....> ");
    let text = written(test_feed(&mut repl, b"]\n\n"));
    assert!(text.starts_with("3:1: expected "), "{text}");
    assert!(text.contains("\n]\n^\n"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    // End of input submits an entry whatever it has open.
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"w = (1\n")), "....> ");
    let text = written(test_flush(&mut repl));
    assert!(text.contains("expected )"), "{text}");
}

#[test]
fn type_arguments_do_not_continue() {
    let text = written(test_feed(&mut Repl::new(), b"r = read<I64>\n:t\n"));
    assert!(text.starts_with("r: "), "{text}");
}

#[test]
fn end_of_input_at_a_prompt_ends_its_line() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, &[])), "fipe> ");
    assert_eq!(written(test_flush(&mut repl)), "\n");
}

#[test]
fn a_pipeline_typed_line_by_line_is_one_entry() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":host\n");
    host(&mut repl, outs);
    // Inside a function, so the line stands although the REPL wires no
    // host for `ls`.
    for line in [
        "f = _ => ls -l /var/log\n",
        "    | filter (e => e.size > 1_000_000)\n",
        "    | sort -by .size -desc\n",
        "    | take 10\n",
        "    | map .name\n",
    ] {
        assert_eq!(
            written(test_feed(&mut repl, line.as_bytes())),
            "....> ",
            "{line}"
        );
    }
    let text = written(test_feed(&mut repl, b":t\n"));
    assert!(text.starts_with("f: a -> Vec<String>\n"), "{text}");
    assert_eq!(
        test_feed(&mut repl, b"x = 1\n"),
        [Out::Write(b"....> ".to_vec())]
    );
    let text = written(test_feed(&mut repl, b":r ls\n"));
    assert_eq!(text.matches("ls: value").count(), 1, "{text}");
}

#[test]
fn a_rejected_entry_defines_nothing() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"y = 1\n")), "....> ");
    assert_eq!(written(test_feed(&mut repl, b"  + \"a\"\n")), "....> ");
    let text = written(test_feed(&mut repl, b"\n"));
    assert!(
        text.starts_with(
            "1:5: no meaning reads all literal forms: Whole, Text\n"
        ),
        "{text}"
    );
    assert!(written(test_feed(&mut repl, b":t y\n")).contains("unknown name"));
}

/// All written text with the prompts taken out.
fn unprompted(outs: Vec<Out>) -> String {
    written(outs).replace("fipe> ", "").replace("....> ", "")
}

#[test]
fn entries_are_independent_of_read_boundaries() {
    let input = "x = 1\n+ \"a\"\n:t x\n";
    let whole = unprompted(test_feed(&mut Repl::new(), input.as_bytes()));
    let mut repl = Repl::new();
    let mut lines = String::new();
    for line in input.split_inclusive('\n') {
        lines += &unprompted(test_feed(&mut repl, line.as_bytes()));
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
    let text = written(test_feed(&mut Repl::new(), b":t 1\n  + 2\n\n"));
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
        let text = written(test_feed(&mut Repl::new(), input.as_bytes()));
        assert!(text.contains(at), "{input:?}: {text}");
        assert!(text.contains("\n+ 2\n^"), "{input:?}: {text}");
    }
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load k\n");
    let id = file_request(&outs, "k.fipe");
    written(test_file(&mut repl, &id, "k.fipe", Some("pub k = 1\n")).unwrap());
    let text = written(test_feed(&mut repl, b"+ 2\n\n"));
    assert!(text.starts_with("1:1: expected "), "{text}");
}

#[test]
fn load_of_a_second_file_for_a_loaded_module_changes_nothing() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load 01-files\n");
    assert_eq!(outs[0], Out::Write(b"01-files as files\n".to_vec()));
    let id = file_request(&outs[1..], "01-files.fipe");
    written(
        test_file(&mut repl, &id, "01-files.fipe", Some("pub k = 1\n"))
            .unwrap(),
    );
    let text = written(test_feed(&mut repl, b":load 02-files\n"));
    assert_eq!(text, "02-files: already loaded as `files`\nfipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b":t files.k\n")),
        "files.k: Integer\nexact 1\nknown at compile\nfipe> "
    );
    let outs = test_feed(&mut repl, b":load 01-files\n");
    assert_eq!(
        outs,
        [Out::Write(b"01-files as files\nfipe> ".to_vec())],
        "reloading the same file is still allowed"
    );
}

#[test]
fn a_diagnostic_past_column_65535_underlines_without_a_panic() {
    let line = format!("{}nope\n\n", " ".repeat(70_000));
    let text = written(test_feed(&mut Repl::new(), line.as_bytes()));
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[0], "1:70001: unknown name");
    assert_eq!(rows[2], format!("{}^^^^", " ".repeat(70_000)));
}

#[test]
fn a_leading_assign_continues_the_line_before() {
    // `  = 2` joins `x = 1` rather than standing alone, where it would
    // be rejected at 1:3.
    let text = written(test_feed(&mut Repl::new(), b"x = 1\n  = 2\n:t x\n"));
    assert!(!text.starts_with("1:"), "{text}");
    assert!(text.ends_with("fipe> "), "{text}");
    // A signature and its `= value` line are one entry: either the pair
    // is accepted, or it is rejected at its second line, never the
    // signature alone.
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"f: I64 -> I64\n")), "....> ");
    let text = written(test_feed(&mut repl, b"  = x => x + 1\n:t f\n"));
    assert!(!text.contains("declaration form"), "{text}");
    assert!(
        text.starts_with("2:3: ") || text.starts_with("f: I64 -> I64\n"),
        "{text}"
    );
    // An empty line submits it alone.
    let text = written(test_feed(&mut Repl::new(), b"pub f: I64\n\n"));
    assert!(text.contains("declaration form"), "{text}");
}

#[test]
fn a_lowercase_numeric_type_earns_its_hint() {
    let text = written(test_feed(&mut Repl::new(), b"x: i64 = 1\n\n"));
    assert_eq!(
        text,
        "1:4: numeric types are uppercase: write `I64`\n\
         x: i64 = 1\n   ^^^\nfipe> "
    );
}

#[test]
fn ast_shows_a_checked_tuple() {
    let text = written(test_feed(&mut Repl::new(), b"pair = (1, 2)\n:ast\n"));
    assert_eq!(
        text,
        "Bind\n  Name pair\n  Tuple\n    Int 1\n    Int 2\nfipe> "
    );
}

#[test]
fn a_where_line_continues_an_accepted_type_item() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"type P { x: I64 }\nwhere 0 <= self.x\n:ast\n:t P 1\n",
    ));
    assert_eq!(
        text,
        "Type P\n  Field x\n    Type I64\n  Where\n    Binary <=\n      Int 0\n      Name self.x\n\
         P 1: P\nnames P\nparts x: 1\nknown at compile\nfipe> "
    );
}

#[test]
fn member_schemes_and_type_parameter_defaults_are_visible() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"scale = v k => v * k\n:t scale\n",
    ));
    assert_eq!(
        text,
        "scale: Mul<b> a => a -> b -> a\nknown at compile\nfipe> "
    );
    let text = written(test_feed(
        &mut Repl::new(),
        b"type Op<R = Self, O = Self> where self.op: R -> O\n:ast\n",
    ));
    assert_eq!(text.matches("Default").count(), 2, "{text}");
    let text = written(test_feed(&mut Repl::new(), b"type V: Mul<F64> { x: F64 }\nimpl V: Mul<F64> { mul = k self => Self (self.x * k) }\nv = V 1.0\n:t v * 2.0\n"));
    assert!(
        text.contains("V does not have a ready claim for Literal"),
        "{text}"
    );
    let text = written(test_feed(&mut Repl::new(), b"n = 1\n:t n.nope\n"));
    assert!(
        text.contains("no such field or member on Integer"),
        "{text}"
    );
    let text = written(test_feed(
        &mut Repl::new(),
        b"c = True\n:t if c { 2 } else { \"a\" }\n",
    ));
    assert!(
        text.contains("no meaning reads all literal forms: Whole, Text"),
        "{text}"
    );
}

#[test]
fn docs_are_displayed_above_types_and_under_tree_owners() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"## A value\n")), "....> ");
    assert_eq!(
        written(test_feed(&mut repl, b"\nx = 1\n:t x\n")),
        "A value\nx: Integer\nexact 1\nknown at compile\nfipe> "
    );
    let text = written(test_feed(
        &mut repl,
        b"## A record\ntype P {\n## A field\nx: I64,\n}\n:ast\n",
    ));
    assert!(
        text.contains(
            "Type P\n  Doc \"A record\"\n  Field x\n    Doc \"A field\""
        ),
        "{text}"
    );
    let mut outs = test_feed(&mut repl, b":host\n:t ls\n");
    let mut text = String::new();
    for (index, (path, source)) in HOST.iter().enumerate() {
        let id = file_request(&outs, path);
        outs = test_file(&mut repl, &id, path, Some(source)).unwrap();
        if index + 1 == HOST.len() {
            text = written(outs.clone());
        } else {
            file_request(&outs, HOST[index + 1].0);
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
    assert_eq!(
        written(test_feed(&mut repl, b"banner = 'first\n")),
        "....> "
    );
    assert_eq!(written(test_feed(&mut repl, b"\n# text\n:q\n")), "....> ");
    let text = written(test_feed(&mut repl, b"last'\n:ast\n"));
    assert!(
        text.contains("Str \"first\\n\\n# text\\n:q\\nlast\""),
        "{text}"
    );
    assert_eq!(
        written(test_feed(&mut repl, b"msg = \"the file \\\n")),
        "....> "
    );
    let text = written(test_feed(&mut repl, b"   was not found\"\n:ast\n"));
    assert!(text.contains("Str \"the file was not found\""), "{text}");
    let text =
        written(test_feed(&mut repl, b"x = \"unterminated\n\ny = 2\n:t y\n"));
    assert!(text.contains("unterminated quote"), "{text}");
    assert!(text.contains("y: Integer"), "{text}");
}

#[test]
fn ast_shows_impl_claim() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"impl Vec2: Mul<F64> { mul = k self => self }\n:ast\n",
    ));
    assert!(text.contains("Claim"), "{text}");
    assert!(text.contains("Mul"), "{text}");
    assert!(text.contains("F64"), "{text}");
}

#[test]
fn host_replays_whole_entries() {
    let mut repl = Repl::new();
    let outs =
        test_feed(&mut repl, b"x = 'a\n\nb'\n\n## Doc.\n\nv = 7\n:host\n");
    let text = host(&mut repl, outs);
    assert!(!text.contains("quote") && !text.contains("owner"), "{text}");
    let text = written(test_feed(&mut repl, b":t x\n:t v\n"));
    assert!(
        text.contains("x: String") && text.contains("Doc.\nv: Integer"),
        "{text}"
    );
}

#[test]
fn documented_heads_continue() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"## Doubles.\nf: I64 -> I64\n  = x => x * 2\n:t f\n",
    ));
    assert!(text.contains("Doubles.\nf: I64 -> I64"), "{text}");
    for source in [
        "## Positive.\ntype Pos: I64\nwhere self > 0\n:ast\n",
        "## Positive.\ntype Pos: I64 where\nself > 0\n:ast\n",
    ] {
        let text = written(test_feed(&mut Repl::new(), source.as_bytes()));
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
        let text = written(test_feed(&mut Repl::new(), source.as_bytes()));
        assert!(
            text.contains("unexpected character")
                && text.contains("y: Integer"),
            "{text}"
        );
    }
}

#[test]
fn inline_doc_does_not_hold_next_line() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"x = 1 ## note\n\ny = 2\n:t y\n",
    ));
    assert!(
        text.contains("expected the end") && text.contains("y: Integer"),
        "{text}"
    );
    let text = written(test_feed(
        &mut Repl::new(),
        b"x = [## d\n\ny = 1\n]\n:ast\n",
    ));
    assert!(text.contains("Doc \"d\""), "{text}");
}

#[test]
fn raw_quote_scans_new_lines_only() {
    let mut repl = Repl::new();
    let start = std::time::Instant::now();
    test_feed(&mut repl, b"x = '\n");
    for _ in 0..2_000 {
        test_feed(&mut repl, b"0123456789012345678901234567890123456789\n");
    }
    let text = written(test_feed(&mut repl, b"'\n:t x\n"));
    assert!(text.contains("x: String"), "{text}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn trailing_join_continues() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"f = a b => a\ng = f 1 \\\n  2\n:t g\n",
    ));
    assert!(text.contains("g: Integer"), "{text}");
}

#[test]
fn type_echo_preserves_quote_bytes() {
    let text = written(test_feed(&mut Repl::new(), b":t 'a\n  b'\n"));
    assert!(text.contains("'a\n  b': String"), "{text}");
}

#[test]
fn empty_module_doc_has_a_tree() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"##! only a module doc\n\n:ast\n",
    ));
    assert!(text.contains("Module doc \"only a module doc\""), "{text}");
}

#[test]
fn the_lines_before_a_blank_line_are_one_unit() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"x = 1\ny = x\n")), "....> ");
    let text = written(test_feed(&mut repl, b"\n:ast\n:t y\n"));
    assert!(text.contains("Bind\n  Name x\n"), "{text}");
    assert!(text.contains("Bind\n  Name y\n"), "{text}");
    assert!(text.contains("y: Integer"), "{text}");
    let mut repl = Repl::new();
    test_feed(&mut repl, b"x = 1\n");
    assert_eq!(written(test_flush(&mut repl)), "\n");
    assert!(written(test_feed(&mut repl, b":t x\n")).contains("x: Integer"));
}

#[test]
fn one_unit_checks_mutual_claims() {
    let source = b"type A: From<B> {}\ntype B: From<A> {}\nimpl A: From<B> { from: B -> Self = b => Self {} }\nimpl B: From<A> { from: A -> Self = a => Self {} }\n\na = A {}\nb = a | to<B>\n:t b\n";
    assert_eq!(
        written(test_feed(&mut Repl::new(), source)),
        "b: B\nknown at compile\nfipe> "
    );
}

#[test]
fn leading_division_continues() {
    let text = written(test_feed(&mut Repl::new(), b"8\n/ 2\n:t\n"));
    assert!(text.contains("8 / 2: Integer"), "{text}");
}

#[test]
fn queries_are_independent_of_read_boundaries() {
    for source in [":t 1\n+ 2\n", ":r 1\n+ 2\n"] {
        let together = written(test_feed(&mut Repl::new(), source.as_bytes()));
        let mut repl = Repl::new();
        let mut separate = String::new();
        for line in source.split_inclusive('\n') {
            separate.push_str(&written(test_feed(&mut repl, line.as_bytes())));
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
    assert_eq!(written(test_feed(&mut repl, b"type Shape\n")), "....> ");
    let text = written(test_feed(
        &mut repl,
        b"where self.area: F64\nx: Shape -> F64 = s => s.area\n:t x\n",
    ));
    assert_eq!(text, "x: Shape -> F64\nknown at compile\nfipe> ");
}

#[test]
fn signature_does_not_swallow_next_binding() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"sig: Integer -> Integer\n\ny = 1\n:t y\nz =\n5\n:t z\n",
    ));
    assert!(
        text.contains("y: Integer") && text.contains("z: Integer"),
        "{text}"
    );
}

#[test]
fn unknown_impl_has_one_accurate_error() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"impl Unknown { f = self => self }\n\n",
    ));
    assert!(
        text.contains("unknown type") && !text.contains("another module"),
        "{text}"
    );
}

#[test]
fn commands_use_words() {
    let text =
        written(test_feed(&mut Repl::new(), b"x = 1\n:r x\n:load\nSelf\n\n"));
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
    let text = written(test_feed(&mut repl, b"a: U8 = 200\nb: U16 = 1000\n:t a + b\nsum2: U8 -> U8 -> U8 = x y => x + y\n:checks sum2\n"));
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
            let outs = test_feed(&mut repl, b":host\n");
            host(&mut repl, outs);
        }
        let text = written(test_feed(
            &mut repl,
            b"sum2: U8 -> U8 -> U8 = x y => x + y\n\ng = x => U8 x\n:t g\n",
        ));
        assert!(text.contains("open entry"), "{text}");
        assert!(text.contains("g: Integer -> Integer"), "{text}");
        repl.strict(false);
        let text = written(test_feed(
            &mut repl,
            b"sum2: U8 -> U8 -> U8 = x y => x + y\n:t sum2\n",
        ));
        assert!(!text.contains("open entry"), "{text}");
        assert!(text.contains("sum2: U8 -> U8 -> U8"), "{text}");
    }
}

#[test]
fn checks_name_places_and_disambiguate_identical_results() {
    let text = written(test_feed(&mut Repl::new(), b"is_prime = n => n > 1\n\ntype Prime: Integer where is_prime self\n\nn: Prime = 7\n\nf: U8 -> U8 = x => x + 1\n\nh: U8 -> U8 = x => x + 1\n\ntake: U8 -> Integer = amount => amount\n\nuse_it = x => take x\n\ntype Box { value: U8 }\n\nbox_it = x => Box x\n\nwrap: Integer -> Maybe<U8> = x => Just x\n\nitems: Vec<Integer> -> Vec<U8> = xs => xs\n:checks\n"));
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
    let named = written(test_feed(
        &mut Repl::new(),
        b"f: U8 -> U8 = x => x + 1\n:checks f\n",
    ));
    assert!(!named.contains("the result at "), "{named}");
}

#[test]
fn timing_and_holding_facts_are_visible_on_both_driver_paths() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":host\n");
    host(&mut repl, outs);
    // A line reading the host Faults, since the REPL wires none, and is
    // withdrawn: `:t` on the line itself shows its facts.
    let text = written(test_feed(&mut repl, b"use cli config\ntype Port: Integer where 0 < self, self <= config.max_port\n:t port: Port = cli.port\nport: Port = ([8080] | head)\n:checks port\nx = Compile (2 + 3)\n:t x\n:t rgb = cli.rgb\n"));
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
    let text = written(test_feed(&mut repl, b"type Flag: Bool & Bits<1>\ntype Mode: Size<1> { read: Flag, write: Flag, exec: Flag }\nm = Mode -read -write=False -exec\n:t m\n"));
    assert!(text.contains("held Size<1>, 3 of 8 bits"), "{text}");
    assert!(
        text.contains("parts read: True, write: False, exec: True"),
        "{text}"
    );
}

#[test]
fn written_tests_keep_sources_and_include_proved_siblings() {
    let mut repl = Repl::new();
    let text = written(test_feed(&mut repl, b"type Band: Integer where 0 <= self, self < 10\nf: U8 -> Band = n => n\n:checks f\n"));
    assert_eq!(text.matches("against Band:").count(), 1, "{text}");
    assert!(text.contains("against Band: self < 10"), "{text}");
    assert!(!text.contains("the result at "), "{text}");

    let outs = test_feed(&mut repl, b"use bands\n\n");
    let id = file_request(&outs, "bands.fipe");
    written(test_file(&mut repl, &id, "bands.fipe", Some("pub type Band: Integer where 0 <= self, self < 10\npub f: Integer -> Band = n => n\n")).unwrap());
    let text = written(test_feed(&mut repl, b":checks\n"));
    assert!(text.contains("against Band: 0 <= self"), "{text}");
    assert!(text.contains("against Band: self < 10"), "{text}");

    let mut repl = Repl::new();
    let mut outs = test_feed(&mut repl, b":host\n");
    for (path, source) in HOST {
        let id = file_request(&outs, path);
        let source = if path.ends_with("config.fipe") {
            format!("{source}\n## A host limit.\npub type HostBand: Integer where 0 <= self, self < 10\n")
        } else {
            source.into()
        };
        outs = test_file(&mut repl, &id, path, Some(&source)).unwrap();
    }
    written(outs);
    let text = written(test_feed(
        &mut repl,
        b"use config\nf: U8 -> config.HostBand = n => n\n:checks f\n",
    ));
    assert!(text.contains("against HostBand: self < 10"), "{text}");
    assert!(!text.contains("the result at "), "{text}");
}

#[test]
fn container_types_warnings_and_conversion_hint_are_visible() {
    let text = written(test_feed(&mut Repl::new(), b"xs = [1, 2, 3]\n:t xs ++ [4, 5]\ng = x (y = 0) => x\n\ng (1, 2)\n\ntotal: Vec<Integer> -> Integer = xs => xs | sum\n\ntotal -xs xs\n\n"));
    assert!(text.contains("[Integer; 5]"), "{text}");
    assert!(
        text.contains("warning: written as one tuple argument"),
        "{text}"
    );
    assert!(text.contains("to<Vec<Integer>>"), "{text}");
}

#[test]
fn outcome_errors_explain_the_family_and_exit() {
    let text = written(test_feed(
        &mut Repl::new(),
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
    let text = written(test_feed(&mut Repl::new(), b"type I8b: Integer & Bits<8>\nwhere -128 <= self, self <= 127\nx: I8b = 1\n:t x\n"));
    assert!(!text.contains("does not fit"), "{text}");
    assert!(text.contains("x: I8b"), "{text}");
}

#[test]
fn global_checks_name_their_functions_with_local_positions() {
    let text = written(test_feed(&mut Repl::new(),
        b"x = 1\n\nf: U8 -> U8 = x => x + 1\n\ng: U8 -> U8 = x => x + 1\n:checks\n",
    ));
    assert!(text.contains("f: eval, per call:"), "{text}");
    assert!(text.contains("g: eval, per call:"), "{text}");
    assert!(text.contains("at 1:10"), "{text}");
    assert!(!text.contains("at 3:10"), "{text}");
    // A member's open entry names its type and member.
    let text = written(test_feed(&mut Repl::new(), b"type L: Len { n: Natural }\nimpl L: Len { len: Self -> Natural = self => self.n - 1 }\n:checks\n"));
    assert!(text.contains("L.len: eval, per call:"), "{text}");
    // One proved by its body has none.
    let text = written(test_feed(&mut Repl::new(), b"type L: Len { n: Natural }\nimpl L: Len { len = self => self.n }\n:checks\n"));
    assert!(!text.contains("L.len:"), "{text}");
}

#[test]
fn timing_diagnostic_names_the_impure_input() {
    let text = written(test_feed(&mut Repl::new(), b":t Compile (sin 1.0)\n"));
    assert!(
        text.contains("1:10: timing requires compile; input `sin` is not pure"),
        "{text}"
    );
    assert!(text.contains("         ^^^"), "{text}");
}

#[test]
fn a_type_without_a_test_explains_how_to_enter_it() {
    let text = written(test_feed(
        &mut Repl::new(),
        b"type Flag: Bool & Bits<1>\n\nFlag True\n\n",
    ));
    assert!(
        text.contains("`Flag` has no test, so it is no gate"),
        "{text}"
    );
    assert!(text.contains("to<Flag>"), "{text}");
}

#[test]
fn ir_shows_collected_supplies_modes_and_rebuilds() {
    let mut repl = Repl::new();
    let text = written(test_feed(
        &mut repl,
        b"g: Vec<Integer> -> Integer = xs => len xs\n\n:ir g 1 2 3\n",
    ));
    assert!(text.contains("collected into spot 0"), "{text}");
    let text = written(test_feed(&mut repl, b":ir Nothing || 4\n"));
    assert!(text.contains("or values"), "{text}");
    assert!(text.contains("by payload"), "{text}");
    let text = written(test_feed(&mut repl, b":ir Just 1 && Just 2\n"));
    assert!(text.contains("by family"), "{text}");
    let text = written(test_feed(
        &mut repl,
        b"m: Maybe<Integer> = Just 1\n\n:ir m |: (x => x + 1)\n",
    ));
    assert!(text.contains("good side of value"), "{text}");
    assert!(text.contains("rebuilt by wrapping"), "{text}");
}

#[test]
fn ir_queries_show_checked_operations_without_accepting() {
    let mut repl = Repl::new();
    let text = written(test_feed(&mut repl, b"x = 1\n:ir x + 2\n:ir\n"));
    assert!(text.contains("checked unit:"), "{text}");
    assert!(text.contains("call value"), "{text}");
    assert!(!text.contains("Span"), "{text}");
    assert!(!text.contains("start:"), "{text}");
    let text = written(test_feed(&mut repl, b":t x\n"));
    assert!(text.contains("Integer"), "{text}");
}

#[test]
fn commands_accept_tabs_and_reject_extra_arguments() {
    let mut repl = Repl::new();
    assert!(written(test_feed(&mut repl, b":t\t1\n")).contains("1: Integer"));
    assert!(written(test_feed(&mut repl, b":q extra\n"))
        .contains("unknown command"));
    assert!(
        written(test_feed(&mut repl, b":ast extra\n")).contains("usage: :ast")
    );
    let outs = test_feed(&mut repl, b":host\tdirectory\n");
    file_request(&outs, "directory/commands.fipe");
}

#[test]
fn host_directory_is_a_path_not_repl_input() {
    let mut repl = Repl::new();
    let outs = repl.host_directory("dir\n:q").unwrap();
    file_request(&outs, "dir\n:q/commands.fipe");
    assert!(written(Repl::new().host_directory("dir\0suffix").unwrap())
        .contains("NUL"));
}

#[test]
fn source_controls_are_visible_and_carets_follow_rendered_columns() {
    let text = written(test_feed(&mut Repl::new(), b"\tmissing\n\n"));
    assert!(text.contains("\\tmissing\n  ^^^^^^^"), "{text}");
    let text = written(test_feed(&mut Repl::new(), b":t 'a\x1bb'\n"));
    assert!(!text.contains('\x1b'), "{text}");
    assert!(text.contains("\\u{1b}"), "{text}");
}

#[test]
fn ast_shows_visibility_versions_and_member_annotations() {
    let mut repl = Repl::new();
    written(test_feed(&mut repl, b"pub type P { pub x: Integer }\n"));
    let text = written(test_feed(&mut repl, b":ast\n"));
    assert!(
        text.contains("Type P pub") && text.contains("Field pub x"),
        "{text}"
    );
    written(test_feed(
        &mut repl,
        b"impl P { pub f: Integer -> Integer = n => n }\n",
    ));
    let text = written(test_feed(&mut repl, b":ast\n"));
    assert!(
        text.contains("Member pub f") && text.contains("Function"),
        "{text}"
    );
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"use text 2\n\n");
    let id = file_request(&outs, "text.fipe");
    let text = written(
        test_file(&mut repl, &id, "text.fipe", Some("pub x = 1\n")).unwrap(),
    );
    assert_ne!(text, "");
    assert!(
        written(test_feed(&mut repl, b":ast\n")).contains("Use text version 2")
    );
}

#[test]
fn a_conversion_without_candidates_ends_its_message() {
    let text = written(test_feed(&mut Repl::new(), b":t to<F64> 3\n"));
    assert!(
        text.contains("no conversion or member of F64 accepts Integer\n"),
        "{text}"
    );
}

#[test]
fn mir_shows_bodies_locals_blocks_and_charges_in_words() {
    let mut repl = Repl::new();
    let text = written(test_feed(
        &mut repl,
        b"f = (w = 80) (c = (w / 8)) => c\n\n:mir f 16\n",
    ));
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
    let text =
        written(test_feed(&mut repl, b":mir if 1 == 1 { 2 } else { 3 }\n"));
    assert!(
        text.contains("switch _3: True to block 2, False to block 3"),
        "{text}"
    );
    assert!(text.contains("call native prelude.eq"), "{text}");
    let text = written(test_feed(&mut repl, b":mir with cwd /tmp { 1 }\n"));
    assert!(text.contains("push cwd = constant 0"), "{text}");
    assert!(text.contains("    pop\n"), "{text}");
}

#[test]
fn mir_without_an_argument_shows_the_last_accepted_line() {
    let mut repl = Repl::new();
    assert_eq!(
        written(test_feed(&mut repl, b":mir\n")),
        "no accepted line\nfipe> "
    );
    let text = written(test_feed(&mut repl, b"x = 2.5\n:mir\n"));
    assert!(text.contains("constant 0 = 2.5"), "{text}");
    assert!(text.contains("global x = constant 0"), "{text}");
    let text = written(test_feed(&mut repl, b":mir (x, \"a\")\n"));
    assert!(text.contains("constant 0 = \"a\""), "{text}");
    assert!(text.contains("tuple (global x, constant 0)"), "{text}");
}

#[test]
fn an_accepted_line_prints_its_value() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"1 + 2\n\n")), "3\nfipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"[1, 2] | map (n => n + 1)\n\n")),
        "[2, 3]\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"\"a\" ++ \"b\"\n\n")),
        "\"ab\"\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"Just 1 |: (n => Just (n + 1))\n\n")),
        "Just 2\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"x => x\n\n")),
        "<function>\nfipe> "
    );
}

#[test]
fn a_binding_prints_nothing_and_its_value_serves_later_lines() {
    let mut repl = Repl::new();
    assert_eq!(written(test_feed(&mut repl, b"x = 1\n\n")), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"add2 = n => n + 2\n\n")),
        "fipe> "
    );
    assert_eq!(written(test_feed(&mut repl, b"add2 x\n\n")), "3\nfipe> ");
}

#[test]
fn a_fault_names_its_line_and_column() {
    let mut repl = Repl::new();
    assert_eq!(
        written(test_feed(&mut repl, b"1 / ([0] | head)\n\n")),
        "fault at 1:5: division by zero: divisor is 0\n\
         1 / ([0] | head)\n    ^^^^^^^^^^^^\nfipe> "
    );
    let text = written(test_feed(&mut repl, b"f = n => 10 / n\n\nf 0\n\n"));
    assert!(
        // The faulted first line was withdrawn, so `f` stands on line 1.
        text.contains("fault at line 1, 1:15: division by zero: divisor is 0"),
        "{text}"
    );
}

#[test]
fn the_budget_reaches_the_machine() {
    let mut repl = Repl::new();
    repl.budget(5);
    assert_eq!(written(test_feed(&mut repl, b"1 + 2\n\n")), "3\nfipe> ");
    // `1 + 2` costs five: its operator, two literals, the call, one limb.
    repl.budget(4);
    let text = written(test_feed(&mut repl, b"1 + 2\n\n"));
    assert!(
        text.starts_with("fault at 1:1: the budget is exhausted"),
        "{text}"
    );
}

#[test]
fn a_refuted_entry_names_its_limit_and_value_at_its_place() {
    let mut repl = Repl::new();
    let text = written(test_feed(
        &mut repl,
        b"big = _ => 300\n\nx: U8 = big ()\n\n",
    ));
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
    assert_eq!(written(test_feed(&mut repl, PRIME)), "fipe> ");
    assert_eq!(written(test_feed(&mut repl, b"p: Prime = 7\n\n")), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b":checks p\n")),
        "p: Prime\nexact 7\nnames Prime\nknown at compile\n\
         compile, once: binding p, exact 7, against Prime: \
         proved at compile time on 7\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"q: Prime = 8\n\n")),
        "1:4: refuted entry against Prime: 8\nq: Prime = 8\n   ^^^^^\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"n: U8 & Prime = 13\n\n")),
        "fipe> "
    );
    let text = written(test_feed(&mut repl, b":checks n\n"));
    assert!(
        text.contains("against Prime: proved at compile time on 13"),
        "{text}"
    );
}

#[test]
fn the_compile_budget_leaves_an_entry_open() {
    let mut repl = Repl::new();
    repl.compile_budget(10);
    assert_eq!(written(test_feed(&mut repl, PRIME)), "fipe> ");
    assert_eq!(written(test_feed(&mut repl, b"p: Prime = 7\n\n")), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b":checks p\n")),
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
    assert_eq!(written(test_feed(&mut repl, mask)), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"m: Mask = 0b0000_1111\n\nm\n\n")),
        "Mask 15\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"k: Mask = 15\n\n")),
        "1:11: the literal Faults reading Mask: a Mask is written as eight \
         binary digits\nk: Mask = 15\n          ^^\nfipe> "
    );
}

#[test]
fn a_compile_demand_is_shown_exact() {
    let mut repl = Repl::new();
    assert_eq!(
        written(test_feed(&mut repl, b"f = n => n * 6\n\n")),
        "fipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"x = Compile (f 7)\n\n")),
        "fipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b":t x\n")),
        "x: Integer\nexact 42\nknown at compile\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"z = Compile (1 / ([0] | head))\n\n")),
        "1:18: the value demanded at compile time Faults: division by zero\n\
         z = Compile (1 / ([0] | head))\n                 ^^^^^^^^^^^^\nfipe> "
    );
}

#[test]
fn a_sequence_that_faults_on_its_bracket_is_a_check_error() {
    let mut repl = Repl::new();
    let pos = b"type Pos: Sequence<Integer> { first: Integer }\n\n\
impl Pos: Sequence<Integer> { sequence = xs => if (xs | head) > 0 { Self (xs | head) } else { fault \"not positive\" } }\n\n";
    assert_eq!(written(test_feed(&mut repl, pos)), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"p: Pos = [0]\n\n")),
        "1:10: the sequence of Pos Faults on this bracket: not positive\n\
         p: Pos = [0]\n         ^^^\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"q: Pos = [3]\n\nq\n\n")),
        "Pos 3\nfipe> "
    );
}

#[test]
fn an_empty_sum_is_its_meaning_s_zero() {
    let mut repl = Repl::new();
    let vec2 = b"type Vec2: Add & Zero { x: F64, y: F64 }\n\n\
impl Vec2: Add { add = o self => Self (self.x + o.x) (self.y + o.y) }\n\n\
impl Vec2: Zero { zero = Self 0.0 0.0 }\n\n";
    assert_eq!(written(test_feed(&mut repl, vec2)), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"t: Vec2 = [] | sum\n\nt\n\n")),
        "Vec2 0.0 0.0\nfipe> "
    );
}

#[test]
fn a_load_whose_module_faults_is_withdrawn_and_asks_again() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load m\n");
    let id = file_request(&outs, "m.fipe");
    let text = written(
        test_file(&mut repl, &id, "m.fipe", Some("pub a = (1.0 / 0.0)\n"))
            .unwrap(),
    );
    assert!(text.contains("the result is not finite"), "{text}");
    let text = written(test_feed(&mut repl, b"m.a\n\n"));
    assert!(text.starts_with("1:1: unknown name"), "{text}");
    let outs = test_feed(&mut repl, b":load m\n");
    let id = file_request(&outs, "m.fipe");
    assert_eq!(
        written(
            test_file(&mut repl, &id, "m.fipe", Some("pub a = 2.5\n")).unwrap()
        ),
        "fipe> "
    );
    assert_eq!(written(test_feed(&mut repl, b"m.a\n\n")), "2.5\nfipe> ");
}

#[test]
fn a_withdrawn_load_leaves_its_path_free() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load a/m\n");
    let id = file_request(&outs, "a/m.fipe");
    let text = written(
        test_file(&mut repl, &id, "a/m.fipe", Some("pub a = (1.0 / 0.0)\n"))
            .unwrap(),
    );
    assert!(text.contains("the result is not finite"), "{text}");
    let outs = test_feed(&mut repl, b":load b/m\n");
    let id = file_request(&outs, "b/m.fipe");
    let text = written(
        test_file(&mut repl, &id, "b/m.fipe", Some("pub a = 2.5\n")).unwrap(),
    );
    assert_eq!(text, "fipe> ");
    assert_eq!(written(test_feed(&mut repl, b"m.a\n\n")), "2.5\nfipe> ");
}

#[test]
fn a_line_whose_run_faults_is_withdrawn() {
    let mut repl = Repl::new();
    let text = written(test_feed(
        &mut repl,
        b"stop = n => fault \"stop\"\n\nx: Integer = stop 1\n\n",
    ));
    assert!(text.contains("stop"), "{text}");
    assert_eq!(
        written(test_feed(&mut repl, b"x + 1\n\n")),
        "1:1: unknown name: the line that bound `x` was withdrawn; give it again\nx + 1\n^\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"x = 1\n\nx + 1\n\n")),
        "2\nfipe> "
    );
    // `:t` reads the last line that stood.
    let text = written(test_feed(&mut repl, b"y = (1.0 / 0.0)\n\n:t\n\n"));
    assert!(text.contains("x + 1: Integer\n"), "{text}");
}

#[test]
fn a_name_of_a_withdrawn_line_says_why_it_is_unknown() {
    let mut repl = Repl::new();
    let text = written(test_feed(&mut repl, b"x = 1.0 / 0.0\n\n"));
    assert!(text.contains("the result is not finite"), "{text}");
    let text = written(test_feed(&mut repl, b"x + 1.0\n\n"));
    assert!(
        text.starts_with(
            "1:1: unknown name: the line that bound `x` was withdrawn; give it again\n"
        ),
        "{text}"
    );
    let text = written(test_feed(&mut repl, b"y + 1.0\n\n"));
    assert!(text.starts_with("1:1: unknown name\n"), "{text}");
}

#[test]
fn a_use_after_a_withdrawn_load_says_how_to_load_it_again() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":load a/m\n");
    let id = file_request(&outs, "a/m.fipe");
    let text = written(
        test_file(&mut repl, &id, "a/m.fipe", Some("pub a = (1.0 / 0.0)\n"))
            .unwrap(),
    );
    assert!(text.contains("the result is not finite"), "{text}");
    let outs = test_feed(&mut repl, b"use m\n\n");
    let id = file_request(&outs, "m.fipe");
    let text = written(test_file(&mut repl, &id, "m.fipe", None).unwrap());
    assert!(
        text.starts_with(
            "`m`: its `:load` of a/m.fipe was withdrawn; load it again with `:load a/m.fipe`\n"
        ),
        "{text}"
    );
}

#[test]
fn a_fault_in_a_loaded_module_is_located_in_that_file() {
    let mut repl = Repl::new();
    written(test_feed(&mut repl, b"a = 1\n\nb = 2\n\n"));
    let outs = test_feed(&mut repl, b"use geo\n\n");
    let id = file_request(&outs, "geo.fipe");
    written(
        test_file(
            &mut repl,
            &id,
            "geo.fipe",
            Some("pub reveal = _ => 7\npub boom = n => n / 0.0\n"),
        )
        .unwrap(),
    );
    let text = written(test_feed(&mut repl, b"geo.boom 1.0\n\n"));
    assert!(text.contains("fault at geo.fipe:2:"), "{text}");
    assert!(text.contains("pub boom = n => n / 0.0\n"), "{text}");
    assert!(!text.contains("line 1"), "{text}");
}

#[test]
fn a_fault_in_a_host_default_is_located_in_the_declaration_file() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b"a = 1\n\n:host\n");
    host(&mut repl, outs);
    let text = written(test_feed(&mut repl, b"ls\n\n"));
    // The line of `ls`'s declaration, whose `path` default Faults.
    let line = include_str!("../../../tests/fixtures/host/commands.fipe")
        .lines()
        .position(|line| line.starts_with("pub ls:"))
        .unwrap()
        + 1;
    let at = format!("fault at tests/fixtures/host/commands.fipe:{line}:");
    assert!(text.contains(&at), "{text}");
}

#[test]
fn ir_shows_literals_and_resolution_says_when_nothing_is_named() {
    let mut repl = Repl::new();
    let text = written(test_feed(&mut repl, b":ir 7 - 2\n"));
    assert!(text.contains("= literal 7 prepared by"), "{text}");
    assert!(text.contains("= literal 2 prepared by"), "{text}");
    let text = written(test_feed(&mut repl, b":r 1 + 2\n"));
    assert!(text.starts_with("no names to resolve"), "{text}");
}

#[test]
fn an_unbound_word_at_a_typed_spot_says_it_is_text() {
    let mut repl = Repl::new();
    let outs = test_feed(&mut repl, b":host\n");
    host(&mut repl, outs);
    let text = written(test_feed(&mut repl, b"use fs\n\nfs.list dir /etc\n\n"));
    assert!(
        text.contains(
            "found String; `dir` is text here: no name `dir` is bound"
        ),
        "{text}"
    );
}

#[test]
fn conversion_candidates_name_the_line_of_a_shadowed_type() {
    let mut repl = Repl::new();
    written(test_feed(
        &mut repl,
        b"type A { n: I64 }\n\na1 = A 1\n\ntype A { n: I64 }\n\n",
    ));
    written(test_feed(&mut repl,
        b"type B: From<A> { m: I64 }\nimpl B: From<A> { from: A -> Self = a => Self a.n }\n\n",
    ));
    let text = written(test_feed(&mut repl, b"a1 | B.from\n\n"));
    assert!(text.contains("candidates: "), "{text}");
    let candidates = text.split("candidates: ").nth(1).unwrap();
    let source = text.split(" accepts ").nth(1).unwrap();
    assert_ne!(
        candidates.split(['\n', ';']).next(),
        source.split(['\n', ';']).next(),
        "{text}"
    );
}

mod line_editor {
    use super::test_feed;
    use fipe_repl_core::edit::{Completion, Editor, Event, Key, Keys};
    use fipe_repl_core::Repl;

    /// An editor after typing `text`.
    fn typed(text: &str) -> Editor {
        let mut editor = Editor::new();
        editor.prompt("fipe> ");
        for c in text.chars() {
            assert_eq!(editor.key(Key::Char(c)), Event::Draw);
        }
        editor
    }

    /// The line and the cursor's column after the prompt.
    fn state(editor: &Editor) -> (String, usize) {
        let view = editor.view();
        (view.text, view.cursor - view.prompt.len())
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut editor = typed("ac");
        editor.key(Key::Left);
        editor.key(Key::Char('b'));
        assert_eq!(state(&editor), ("abc".into(), 2));
        assert_eq!(editor.view().cursor, 8);
    }

    #[test]
    fn enter_submits_and_empties_the_line() {
        let mut editor = typed("1 + 1");
        editor.key(Key::Home);
        assert_eq!(editor.key(Key::Enter), Event::Line("1 + 1".into()));
        assert_eq!(state(&editor), (String::new(), 0));
        assert_eq!(Editor::new().key(Key::Enter), Event::Line(String::new()));
    }

    #[test]
    fn backspace_and_delete_remove_around_the_cursor() {
        let mut editor = typed("abcd");
        editor.key(Key::Left);
        assert_eq!(editor.key(Key::Backspace), Event::Draw);
        assert_eq!(state(&editor), ("abd".into(), 2));
        assert_eq!(editor.key(Key::Delete), Event::Draw);
        assert_eq!(state(&editor), ("ab".into(), 2));
        assert_eq!(editor.key(Key::Delete), Event::None);
        editor.key(Key::Home);
        assert_eq!(editor.key(Key::Backspace), Event::None);
    }

    #[test]
    fn movement_stays_within_the_line() {
        let mut editor = typed("ab");
        assert_eq!(editor.key(Key::Right), Event::None);
        editor.key(Key::Home);
        assert_eq!(state(&editor), ("ab".into(), 0));
        assert_eq!(editor.key(Key::Left), Event::None);
        editor.key(Key::Right);
        assert_eq!(state(&editor), ("ab".into(), 1));
        editor.key(Key::End);
        assert_eq!(state(&editor), ("ab".into(), 2));
    }

    #[test]
    fn kills_delete_to_either_end_or_a_word() {
        let mut editor = typed("one two  three");
        editor.key(Key::KillWord);
        assert_eq!(state(&editor), ("one two  ".into(), 9));
        editor.key(Key::KillWord);
        assert_eq!(state(&editor), ("one ".into(), 4));
        editor.key(Key::Left);
        editor.key(Key::Left);
        editor.key(Key::KillEnd);
        assert_eq!(state(&editor), ("on".into(), 2));
        editor.key(Key::Left);
        editor.key(Key::KillStart);
        assert_eq!(state(&editor), ("n".into(), 0));
    }

    #[test]
    fn multibyte_text_edits_by_char() {
        let mut editor = typed("é→x");
        editor.key(Key::Left);
        editor.key(Key::Backspace);
        assert_eq!(state(&editor), ("éx".into(), 1));
    }

    #[test]
    fn up_and_down_walk_the_history_and_keep_the_draft() {
        let mut editor = typed("first");
        editor.key(Key::Enter);
        for c in "second".chars() {
            editor.key(Key::Char(c));
        }
        editor.key(Key::Enter);
        editor.key(Key::Enter);
        editor.key(Key::Char('d'));
        editor.key(Key::Up);
        assert_eq!(state(&editor), ("second".into(), 6));
        editor.key(Key::Up);
        assert_eq!(state(&editor).0, "first");
        assert_eq!(editor.key(Key::Up), Event::None);
        editor.key(Key::Down);
        assert_eq!(state(&editor).0, "second");
        editor.key(Key::Down);
        assert_eq!(state(&editor), ("d".into(), 1));
        assert_eq!(editor.key(Key::Down), Event::None);
    }

    #[test]
    fn a_repeated_line_is_kept_once() {
        let mut editor = typed("x");
        editor.key(Key::Enter);
        editor.key(Key::Char('x'));
        editor.key(Key::Enter);
        editor.key(Key::Up);
        editor.key(Key::Up);
        assert_eq!(state(&editor).0, "x");
        assert_eq!(editor.key(Key::Up), Event::None);
    }

    #[test]
    fn ctrl_c_clears_and_ctrl_d_ends_only_an_empty_line() {
        let mut editor = typed("ab");
        editor.key(Key::Home);
        assert_eq!(editor.key(Key::Eof), Event::Draw);
        assert_eq!(state(&editor), ("b".into(), 0));
        assert_eq!(editor.key(Key::Interrupt), Event::Interrupt);
        assert_eq!(state(&editor), (String::new(), 0));
        assert_eq!(editor.key(Key::Eof), Event::End);
    }

    #[test]
    fn bytes_decode_to_keys() {
        let mut keys = Keys::new();
        assert_eq!(
            keys.push(b"a\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1bOH\x1b[3~"),
            [
                Key::Char('a'),
                Key::Up,
                Key::Down,
                Key::Right,
                Key::Left,
                Key::Home,
                Key::End,
                Key::Home,
                Key::Delete,
            ]
        );
        assert_eq!(
            keys.push(b"\x01\x05\x0b\x15\x17\x03\x04\x7f\r\n\x09\x1b[1;5C"),
            [
                Key::Home,
                Key::End,
                Key::KillEnd,
                Key::KillStart,
                Key::KillWord,
                Key::Interrupt,
                Key::Eof,
                Key::Backspace,
                Key::Enter,
                Key::Tab,
            ]
        );
    }

    #[test]
    fn cr_lf_is_one_enter_even_across_reads() {
        let mut keys = Keys::new();
        assert_eq!(
            keys.push(b"a\r\nb\n\n"),
            [
                Key::Char('a'),
                Key::Enter,
                Key::Char('b'),
                Key::Enter,
                Key::Enter,
            ]
        );
        assert_eq!(keys.push(b"\r"), [Key::Enter]);
        assert_eq!(
            keys.push(b"\nc\r\r"),
            [Key::Char('c'), Key::Enter, Key::Enter,]
        );
    }

    #[test]
    fn an_invalid_utf8_byte_drops_only_itself() {
        let mut keys = Keys::new();
        assert_eq!(keys.push(b"\xc3A"), [Key::Char('A')]);
        assert_eq!(keys.push(b"\xe2B\xffC"), [Key::Char('B'), Key::Char('C')]);
        assert_eq!(keys.push(b"\xc0\x80D"), [Key::Char('D')]);
    }

    #[test]
    fn a_split_sequence_or_char_waits_for_its_rest() {
        let mut keys = Keys::new();
        assert_eq!(keys.push(b"\x1b["), []);
        assert_eq!(keys.push(b"D\xc3"), [Key::Left]);
        assert_eq!(keys.push(b"\xa9"), [Key::Char('é')]);
    }

    #[test]
    fn the_prompt_follows_the_entry_and_interrupt_drops_it() {
        let mut repl = Repl::new();
        test_feed(&mut repl, &[]);
        assert_eq!(repl.prompt(), "fipe> ");
        assert_eq!(super::written(test_feed(&mut repl, b"(1 +\n")), "....> ");
        assert_eq!(repl.prompt(), "....> ");
        assert_eq!(super::written(repl.interrupt()), "fipe> ");
        assert_eq!(super::written(test_feed(&mut repl, b"2\n\n")), "2\nfipe> ");
    }

    /// The candidates for the line typed so far.
    fn candidates(repl: &Repl, before: &str) -> (String, Vec<String>) {
        let completion = repl.complete(before);
        (completion.partial, completion.candidates)
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| String::from(*name)).collect()
    }

    #[test]
    fn tab_completes_a_host_module_member() {
        let mut repl = Repl::new();
        let outs = test_feed(&mut repl, b":host\n");
        super::host(&mut repl, outs);
        let found = candidates(&repl, "fs.li");
        assert_eq!(found, ("li".into(), names(&["list"])));
        assert!(candidates(&repl, "f").1.contains(&"fs".into()));
    }

    #[test]
    fn tab_completes_commands_at_a_line_s_start() {
        let repl = Repl::new();
        assert_eq!(
            candidates(&repl, " :ho"),
            (":ho".into(), names(&[":host"]))
        );
        assert_eq!(candidates(&repl, ":").1.len(), 10);
        assert_eq!(candidates(&repl, ":he").1, names(&[":help"]));
    }

    #[test]
    fn a_dot_after_nothing_or_a_dot_offers_no_member() {
        let mut repl = Repl::new();
        test_feed(&mut repl, b"x = \"a\"\n\n");
        assert_eq!(candidates(&repl, "..").1, names(&[]));
        assert_eq!(candidates(&repl, "x..").1, names(&[]));
        assert_eq!(candidates(&repl, "1 + .").1, names(&[]));
    }

    #[test]
    fn tab_completes_session_names_and_prelude_names() {
        let mut repl = Repl::new();
        test_feed(
            &mut repl,
            b"zeta_one = 1\nzeta_two = 2\n\ntype Zed { Zork }\n\n",
        );
        let found = candidates(&repl, "1 + zeta_");
        assert_eq!(found.1, names(&["zeta_one", "zeta_two"]));
        assert_eq!(candidates(&repl, "Zo").1, names(&["Zork"]));
        assert!(candidates(&repl, "le").1.contains(&"len".into()));
        assert_eq!(candidates(&repl, "\"zeta_").1, names(&[]));
        assert_eq!(candidates(&repl, "1 + ").1, names(&[]));
    }

    #[test]
    fn tab_completes_a_value_s_fields_and_impl_members() {
        let mut repl = Repl::new();
        test_feed(
            &mut repl,
            b"type Pt { px: Integer, py: Integer }\n\
              impl Pt { pnorm = self => self.px + self.py }\n\
              \n\
              here = Pt { px: 1, py: 2 }\n\n",
        );
        let found = candidates(&repl, "here.p");
        assert_eq!(found, ("p".into(), names(&["pnorm", "px", "py"])));
    }

    #[test]
    fn one_candidate_inserts_several_insert_their_prefix_then_list() {
        let mut editor = typed("x = fs.li");
        assert_eq!(editor.key(Key::Tab), Event::Complete("x = fs.li".into()));
        let one = Completion {
            partial: "li".into(),
            candidates: names(&["list"]),
        };
        assert_eq!(editor.complete(one), Event::Draw);
        assert_eq!(state(&editor), ("x = fs.list".into(), 11));
        let mut editor = typed("ze");
        let several = Completion {
            partial: "ze".into(),
            candidates: names(&["zeta_one", "zeta_two"]),
        };
        editor.key(Key::Tab);
        assert_eq!(editor.complete(several.clone()), Event::Draw);
        assert_eq!(state(&editor), ("zeta_".into(), 5));
        editor.key(Key::Tab);
        let rest = Completion {
            partial: "zeta_".into(),
            ..several.clone()
        };
        assert_eq!(
            editor.complete(rest.clone()),
            Event::List(several.candidates)
        );
        editor.key(Key::Char('o'));
        editor.key(Key::Backspace);
        editor.key(Key::Tab);
        assert_eq!(editor.complete(rest), Event::None);
        assert_eq!(editor.complete(Completion::default()), Event::None);
    }

    #[test]
    fn the_view_classes_the_line_by_token() {
        use fipe_repl_core::highlight::Class;
        let editor = typed("if é | Just 1.5 ++ \"a\" # note");
        let view = editor.view();
        let spelled: Vec<(String, Class)> = view
            .classes
            .iter()
            .map(|(range, class)| {
                (
                    view.text
                        .chars()
                        .skip(range.start)
                        .take(range.len())
                        .collect(),
                    *class,
                )
            })
            .collect();
        assert_eq!(
            spelled,
            [
                ("if".into(), Class::Keyword),
                ("é".into(), Class::String),
                ("|".into(), Class::Operator),
                ("Just".into(), Class::Constructor),
                ("1.5".into(), Class::Number),
                ("++".into(), Class::Operator),
                ("\"a\"".into(), Class::String),
                ("# note".into(), Class::Comment),
            ]
        );
        let open = typed("x = 'abc");
        let last = open.view().classes.last().cloned();
        assert_eq!(last, Some((4..8, Class::String)));
    }
}

#[test]
fn a_hosted_session_hands_each_call_to_the_driver_and_goes_on() {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let mut repl = Repl::new();
    repl.hosted(true);
    let outs = test_feed(&mut repl, b":host\n");
    host(&mut repl, outs);
    assert_eq!(written(test_feed(&mut repl, b"use text\n\n")), "fipe> ");
    let outs = test_feed(&mut repl, b"x = text.trim \"  a  \"\n\n");
    let id = call_request(&outs);
    let outs = test_answer(&mut repl, &id, |request, tables| {
        let Request::Call {
            declaration,
            arguments,
            ..
        } = request
        else {
            panic!("a run asks only for host calls")
        };
        assert_eq!(tables.declarations[*declaration].label, "trim");
        assert_eq!(arguments, &[Value::text("  a  ")]);
        Reply::Value(Ok(Value::text("a")))
    })
    .unwrap();
    assert_eq!(written(outs), "fipe> ");
    assert_eq!(written(test_feed(&mut repl, b"x\n\n")), "\"a\"\nfipe> ");
    // A failed answer Faults the line, which is withdrawn.
    let outs = test_feed(&mut repl, b"y = text.trim \"b\"\n\n");
    let id = call_request(&outs);
    let text = written(
        test_answer(&mut repl, &id, |_, _| {
            Reply::Value(Err(String::from("refused")))
        })
        .unwrap(),
    );
    assert!(text.contains("host: refused"), "{text}");
    assert!(written(test_feed(&mut repl, b"y\n\n")).contains("unknown name"));
    // Nothing waits otherwise, and a file answer is not a call's.
    assert_eq!(
        test_answer(&mut repl, &id, |_, _| panic!("stale callback")),
        Err(DriveError::WrongRequest)
    );
    let outs = test_feed(&mut repl, b"text.trim \"c\"\n\n");
    let id = call_request(&outs);
    assert_eq!(
        test_file(&mut repl, &id, "x.fipe", None),
        Err(DriveError::WrongRequest)
    );
    let outs =
        test_answer(&mut repl, &id, |_, _| Reply::Value(Ok(Value::text("c"))))
            .unwrap();
    assert_eq!(written(outs), "\"c\"\nfipe> ");
}

#[test]
fn host_sources_load_declarations_without_file_requests() {
    let mut repl = Repl::new();
    let sources: Vec<(&str, &str)> = HOST
        .iter()
        .map(|(path, text)| {
            (
                path.rsplit('/').next().unwrap().trim_end_matches(".fipe"),
                *text,
            )
        })
        .collect();
    let text = written(
        test_host_sources(&mut repl, "tests/fixtures/host", &sources).unwrap(),
    );
    assert_eq!(text, "host: tests/fixtures/host\nfipe> ");
    let text = written(test_feed(&mut repl, b":r ls\n"));
    assert!(text.contains("ls: value commands.ls"), "{text}");
}

#[test]
fn supplied_nested_host_modules_keep_their_names_and_complete() {
    let mut repl = Repl::new();
    written(
        test_host_sources(
            &mut repl,
            "host",
            &[
                ("text.open", "## Open text.\npub new = 7"),
                ("client.open", "## Show a region.\npub new = 9"),
            ],
        )
        .unwrap(),
    );
    let result = written(test_feed(
        &mut repl,
        b"use text.open\nopen.new + client.open.new\n\n",
    ));
    assert!(result.contains("16"), "{result}");
    assert_eq!(repl.complete("text.").candidates, ["open"]);
    assert_eq!(repl.complete("text.open.").candidates, ["new"]);
    assert_eq!(repl.complete("open.").candidates, ["new"]);
}

#[test]
fn stale_call_ids_never_invoke_callbacks_or_answer_later_calls() {
    use fipe::{embed::Reply, value::Value};
    let mut repl = Repl::new();
    repl.hosted(true);
    written(
        test_host_sources(
            &mut repl,
            "host",
            &[("commands", "pub ping: Integer -> Integer")],
        )
        .unwrap(),
    );
    let first = call_request(&test_feed(&mut repl, b"(ping 1, ping 2)\n\n"));
    let second = call_request(
        &test_answer(&mut repl, &first, |_, _| Reply::Value(Ok(Value::Int(1))))
            .unwrap(),
    );
    assert_ne!(first, second);
    assert_eq!(first, first.clone());
    assert_eq!(format!("{first:?}"), "RequestId(..)");
    let mut callbacks = 0;
    assert_eq!(
        test_answer(&mut repl, &first, |_, _| {
            callbacks += 1;
            Reply::Value(Ok(Value::Int(99)))
        }),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(callbacks, 0);
    let mut foreign = Repl::new();
    foreign.hosted(true);
    written(
        test_host_sources(
            &mut foreign,
            "host",
            &[("commands", "pub ping: Integer -> Integer")],
        )
        .unwrap(),
    );
    let other = call_request(&test_feed(&mut foreign, b"ping 9\n\n"));
    assert_eq!(
        test_answer(&mut repl, &other, |_, _| panic!("foreign callback")),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(
        test_host_sources(&mut repl, "replacement", &[]),
        Err(DriveError::Busy)
    );
    assert_eq!(repl.host_directory("replacement"), Err(DriveError::Busy));
    assert!(test_feed(&mut repl, b"buffered = 3\n\n").is_empty());
    assert_eq!(
        written(
            test_answer(&mut repl, &second, |_, _| Reply::Value(Ok(
                Value::Int(2)
            )))
            .unwrap()
        ),
        "(1, 2)\nfipe> "
    );
    assert_eq!(
        test_answer(&mut repl, &second, |_, _| panic!("duplicate callback")),
        Err(DriveError::WrongRequest)
    );
    assert!(written(test_feed(&mut repl, b":t buffered\n"))
        .contains("buffered: Integer"));
}

#[test]
fn file_ids_check_kind_path_and_host_replacement_before_mutation() {
    use fipe::embed::Reply;
    let mut repl = Repl::new();
    let id = file_request(
        &test_feed(&mut repl, b"use shape\nshape.answer\n\n"),
        "shape.fipe",
    );
    assert_eq!(
        test_answer(&mut repl, &id, |_, _| panic!("file is not a call")),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(
        test_file(&mut repl, &id, "other.fipe", Some("invalid source")),
        Err(DriveError::WrongRequest)
    );
    let mut other = Repl::new();
    let foreign =
        file_request(&test_feed(&mut other, b"use shape\n\n"), "shape.fipe");
    assert_eq!(
        test_file(&mut repl, &foreign, "shape.fipe", None),
        Err(DriveError::WrongRequest)
    );
    // A token of another kind is refused even when its supplied path is current.
    let mut caller = Repl::new();
    caller.hosted(true);
    written(
        test_host_sources(
            &mut caller,
            "host",
            &[("commands", "pub ping: () -> ()")],
        )
        .unwrap(),
    );
    let call = call_request(&test_feed(&mut caller, b"ping ()\n\n"));
    assert_eq!(
        test_file(&mut repl, &call, "shape.fipe", None),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(
        test_file(&mut caller, &call, "shape.fipe", None),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(
        test_host_sources(&mut repl, "replacement", &[]),
        Err(DriveError::Busy)
    );
    assert_eq!(repl.host_directory("replacement"), Err(DriveError::Busy));
    assert_eq!(
        written(
            test_file(&mut repl, &id, "shape.fipe", Some("pub answer = 7"))
                .unwrap()
        ),
        "7\nfipe> "
    );
    assert_eq!(
        test_file(&mut repl, &id, "shape.fipe", None),
        Err(DriveError::WrongRequest)
    );
    written(
        test_answer(&mut caller, &call, |_, _| {
            Reply::Value(Ok(fipe::value::Value::Unit))
        })
        .unwrap(),
    );
}

#[test]
fn repeated_host_file_paths_require_fresh_request_ids() {
    let mut repl = Repl::new();
    let outs = repl.host_directory("tests/fixtures/host").unwrap();
    let first = file_request(&outs, HOST[0].0);
    let next =
        test_file(&mut repl, &first, HOST[0].0, Some(HOST[0].1)).unwrap();
    let second = file_request(&next, HOST[1].0);
    assert_ne!(first, second);
    assert_eq!(
        test_file(&mut repl, &first, HOST[1].0, Some("invalid")),
        Err(DriveError::WrongRequest)
    );
    let mut outs = next;
    for (path, text) in &HOST[1..] {
        let id = file_request(&outs, path);
        outs = test_file(&mut repl, &id, path, Some(text)).unwrap();
    }
    written(outs);
    let outs = repl.host_directory("tests/fixtures/host").unwrap();
    let current = file_request(&outs, HOST[0].0);
    assert_ne!(first, current);
    assert_eq!(
        test_file(&mut repl, &first, HOST[0].0, Some("invalid")),
        Err(DriveError::WrongRequest)
    );
    host(&mut repl, outs);
}

// These protocol checks deliberately bypass the legacy transcript normalizer.
fn start_gate(
    outs: &[Out],
) -> (fipe_repl_core::RunId, fipe_repl_core::ResumeId) {
    let [Out::Start { run, resume }] = outs else {
        panic!("expected Start: {outs:?}")
    };
    (run.clone(), resume.clone())
}
fn end_gate(
    outs: &[Out],
) -> (
    fipe_repl_core::RunId,
    fipe_repl_core::RunEnd,
    fipe_repl_core::ResumeId,
) {
    let [Out::End {
        run,
        result,
        resume,
    }] = outs
    else {
        panic!("expected End: {outs:?}")
    };
    (run.clone(), result.clone(), resume.clone())
}
fn hosted_ping() -> Repl {
    let mut repl = Repl::new();
    repl.hosted(true);
    written(
        test_host_sources(
            &mut repl,
            "host",
            &[("commands", "pub ping: Integer -> Integer")],
        )
        .unwrap(),
    );
    repl
}

#[test]
fn buffered_runs_yield_before_effects_and_before_the_next_start() {
    use fipe::{embed::Reply, value::Value};
    use fipe_repl_core::RunEnd;
    let mut repl = hosted_ping();
    let mut callbacks = 0;
    let mut committed = false;
    let (first_run, first_start) =
        start_gate(&repl.feed(b"x = ping 1\n\nping x\n\n"));
    assert_eq!(format!("{first_run:?}"), "RunId(..)");
    assert_eq!(format!("{first_start:?}"), "ResumeId(..)");
    assert_eq!(first_run, first_run.clone());
    assert_eq!(callbacks, 0);
    let call = call_request(&repl.resume(&first_start).unwrap());
    assert_eq!(callbacks, 0);
    let (end_run, result, first_end) = end_gate(
        &repl
            .answer(&call, |_, _| {
                callbacks += 1;
                assert!(!committed);
                Reply::Value(Ok(Value::Int(1)))
            })
            .unwrap(),
    );
    assert_eq!(first_run, end_run);
    assert_eq!(result, RunEnd::Success);
    assert_eq!(
        repl.resume(&first_start),
        Err(DriveError::WrongContinuation)
    );
    assert_eq!(
        repl.answer(&call, |_, _| panic!("terminal is not a request")),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(callbacks, 1);
    committed = true;
    let (second_run, second_start) =
        start_gate(&repl.resume(&first_end).unwrap());
    assert_ne!(first_run, second_run);
    assert_eq!(repl.resume(&first_end), Err(DriveError::WrongContinuation));
    assert_eq!(callbacks, 1);
    let call = call_request(&repl.resume(&second_start).unwrap());
    assert_eq!(callbacks, 1);
    assert_eq!(
        repl.resume(&second_start),
        Err(DriveError::WrongContinuation)
    );
    let (end_run, result, end) = end_gate(
        &repl
            .answer(&call, |request, _| {
                callbacks += 1;
                assert!(committed);
                let fipe::embed::Request::Call { arguments, .. } = request
                else {
                    panic!("not a call")
                };
                assert_eq!(arguments, &[Value::Int(1)]);
                Reply::Value(Ok(Value::Int(7)))
            })
            .unwrap(),
    );
    assert_eq!(callbacks, 2);
    assert_eq!(second_run, end_run);
    assert_eq!(result, RunEnd::Success);
    assert_eq!(written(repl.resume(&end).unwrap()), "7\nfipe> ");
    assert_eq!(repl.resume(&end), Err(DriveError::WrongContinuation));
    assert_eq!(callbacks, 2);
}

#[test]
fn boundary_tokens_are_foreign_safe_and_busy_operations_preserve_both_gates() {
    let mut repl = Repl::new();
    let (_, start) = start_gate(&repl.feed(b"42\n\n"));
    let mut foreign = Repl::new();
    let (_, other) = start_gate(&foreign.feed(b"1\n\n"));
    assert_eq!(repl.resume(&other), Err(DriveError::WrongContinuation));
    for token in [&start, &other] {
        assert_eq!(token, &token.clone());
    }
    assert!(repl.feed(b"99\n\n").is_empty());
    assert!(repl.flush().is_empty());
    assert_eq!(
        test_host_sources(&mut repl, "other", &[]),
        Err(DriveError::Busy)
    );
    assert_eq!(repl.host_directory("other"), Err(DriveError::Busy));
    let (_, result, end) = end_gate(&repl.resume(&start).unwrap());
    assert_eq!(result, fipe_repl_core::RunEnd::Success);
    assert_eq!(repl.resume(&other), Err(DriveError::WrongContinuation));
    assert!(repl.feed(b"99\n\n").is_empty());
    assert!(repl.flush().is_empty());
    assert!(repl.interrupt().is_empty());
    assert_eq!(
        test_host_sources(&mut repl, "other", &[]),
        Err(DriveError::Busy)
    );
    assert_eq!(repl.host_directory("other"), Err(DriveError::Busy));
    assert_eq!(written(repl.resume(&end).unwrap()), "42\n");
}

#[test]
fn pure_values_and_language_errors_are_successful_terminals_before_presentation(
) {
    for (source, expected) in [
        ("42\n\n", "42\nfipe> "),
        ("()\n\n", "fipe> "),
        ("Err \"data\"\n\n", "Err \"data\"\nfipe> "),
    ] {
        let mut repl = Repl::new();
        let (run, start) = start_gate(&repl.feed(source.as_bytes()));
        let (terminal_run, result, end) =
            end_gate(&repl.resume(&start).unwrap());
        assert_eq!(run, terminal_run);
        assert_eq!(result, fipe_repl_core::RunEnd::Success);
        assert_eq!(written(repl.resume(&end).unwrap()), expected);
    }
}

#[test]
fn faults_yield_typed_terminals_and_withdraw_only_after_acknowledgement() {
    let mut repl = Repl::new();
    written(test_feed(&mut repl, b"earlier = 11\n\n"));
    let (_, start) =
        start_gate(&repl.feed(b"failed: Integer = 1 / ([0] | head)\n\n"));
    let (_, result, end) = end_gate(&repl.resume(&start).unwrap());
    let fipe_repl_core::RunEnd::Fault(fault) = result else {
        panic!("not a fault")
    };
    assert_eq!(fault.span, fipe::token::Span { start: 36, end: 48 });
    assert!(repl
        .complete("fail")
        .candidates
        .contains(&"failed".to_owned()));
    let text = written(repl.resume(&end).unwrap());
    assert!(text.contains("division by zero"), "{text}");
    assert!(!repl
        .complete("fail")
        .candidates
        .contains(&"failed".to_owned()));
    assert_eq!(written(test_feed(&mut repl, b"earlier\n\n")), "11\nfipe> ");
}

#[test]
fn denied_and_unhosted_calls_report_fault_end_before_diagnostics() {
    use fipe::embed::Reply;
    for hosted in [false, true] {
        let mut repl = hosted_ping();
        repl.hosted(hosted);
        let (_, start) = start_gate(&repl.feed(b"ping 1\n\n"));
        let outs = repl.resume(&start).unwrap();
        let outs = if hosted {
            let call = call_request(&outs);
            repl.answer(&call, |_, _| Reply::Value(Err("denied".to_owned())))
                .unwrap()
        } else {
            outs
        };
        let (_, result, end) = end_gate(&outs);
        assert!(matches!(result, fipe_repl_core::RunEnd::Fault(_)));
        assert!(written(repl.resume(&end).unwrap()).contains(if hosted {
            "denied"
        } else {
            "no host for commands.ping"
        }));
    }
}

#[test]
fn checking_queries_and_file_waits_do_not_fake_runtime_boundaries() {
    let mut repl = Repl::new();
    for command in [
        ":help\n",
        ":t 1\n",
        ":r len\n",
        ":ir 1\n",
        ":mir 1\n",
        "unknown_name\n\n",
    ] {
        written(repl.feed(command.as_bytes()));
    }
    let query_file = file_request(&repl.feed(b":t use query\n"), "query.fipe");
    written(
        repl.file(&query_file, "query.fipe", Some("pub answer = 9"))
            .unwrap(),
    );
    let file = file_request(&repl.feed(b":load data\n"), "data.fipe");
    let mut other = Repl::new();
    let (_, continuation) = start_gate(&other.feed(b"1\n\n"));
    assert_eq!(
        repl.resume(&continuation),
        Err(DriveError::WrongContinuation)
    );
    let outs = repl
        .file(&file, "data.fipe", Some("pub answer = 8"))
        .unwrap();
    let (_, start) = start_gate(&outs);
    let (_, _, end) = end_gate(&repl.resume(&start).unwrap());
    written(repl.resume(&end).unwrap());
}

#[test]
fn executable_module_initialization_waits_for_start_acknowledgement() {
    use fipe::{embed::Reply, value::Value};
    let mut repl = Repl::new();
    repl.hosted(true);
    written(
        test_host_sources(&mut repl, "host", &[("cli", "pub port: Integer")])
            .unwrap(),
    );
    let file = file_request(&repl.feed(b":load data\n"), "data.fipe");
    let (_, start) = start_gate(
        &repl
            .file(&file, "data.fipe", Some("use cli\npub answer = cli.port"))
            .unwrap(),
    );
    assert_eq!(
        repl.answer(&file, |_, _| panic!("start is not a call")),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(
        repl.file(&file, "data.fipe", None),
        Err(DriveError::WrongRequest)
    );
    let call = call_request(&repl.resume(&start).unwrap());
    let (_, _, end) = end_gate(
        &repl
            .answer(&call, |_, _| Reply::Value(Ok(Value::Int(8))))
            .unwrap(),
    );
    written(repl.resume(&end).unwrap());
}

fn reset_gate(
    outs: &[Out],
    expected: fipe_repl_core::ResetReason,
) -> fipe_repl_core::ResumeId {
    let [Out::Reset { reason, resume }] = outs else {
        panic!("expected Reset: {outs:?}")
    };
    assert_eq!(*reason, expected);
    resume.clone()
}
fn exit_gate(repl: &mut Repl, outs: &[Out]) {
    let reset = reset_gate(outs, fipe_repl_core::ResetReason::Exit);
    assert_eq!(repl.resume(&reset).unwrap(), [Out::Exit]);
}

#[test]
fn interrupted_start_is_abandoned_once_before_withdrawal() {
    use fipe_repl_core::RunEnd;
    let mut repl = hosted_ping();
    written(test_feed(&mut repl, b"earlier = 7\n\n"));
    let (run, start) = start_gate(&repl.feed(b"pending = ping 1\n\n"));
    let (terminal_run, result, end) = end_gate(&repl.interrupt());
    assert_eq!(run, terminal_run);
    assert_eq!(result, RunEnd::Abandoned);
    assert_eq!(repl.resume(&start), Err(DriveError::WrongContinuation));
    assert!(repl.interrupt().is_empty());
    assert_eq!(repl.close(), Err(DriveError::Busy));
    assert!(repl
        .complete("pend")
        .candidates
        .contains(&"pending".to_owned()));
    written(repl.resume(&end).unwrap());
    assert!(!repl
        .complete("pend")
        .candidates
        .contains(&"pending".to_owned()));
    assert_eq!(written(test_feed(&mut repl, b"earlier\n\n")), "7\nfipe> ");
    let text = written(repl.feed(b"pending\n\n"));
    assert!(text.contains("was withdrawn;"), "{text}");
    assert!(!text.contains("Faulted"), "{text}");
}

#[test]
fn interrupted_call_rejects_late_answers_and_preserves_prior_globals() {
    use fipe::{embed::Reply, value::Value};
    let mut repl = hosted_ping();
    written(test_feed(&mut repl, b"earlier = 4\n\n"));
    let (run, start) = start_gate(&repl.feed(b"pending = ping 1\n\n"));
    let old = call_request(&repl.resume(&start).unwrap());
    let (terminal, result, end) = end_gate(&repl.interrupt());
    assert_eq!(run, terminal);
    assert_eq!(result, fipe_repl_core::RunEnd::Abandoned);
    let mut callbacks = 0;
    assert_eq!(
        repl.answer(&old, |_, _| {
            callbacks += 1;
            Reply::Value(Ok(Value::Int(9)))
        }),
        Err(DriveError::WrongRequest)
    );
    written(repl.resume(&end).unwrap());
    let (_, start) = start_gate(&repl.feed(b"ping earlier\n\n"));
    let current = call_request(&repl.resume(&start).unwrap());
    assert_ne!(old, current);
    assert_eq!(
        repl.answer(&old, |_, _| {
            callbacks += 1;
            Reply::Value(Ok(Value::Int(9)))
        }),
        Err(DriveError::WrongRequest)
    );
    assert_eq!(callbacks, 0);
    let (_, _, end) = end_gate(
        &repl
            .answer(&current, |request, _| {
                callbacks += 1;
                let fipe::embed::Request::Call { arguments, .. } = request
                else {
                    panic!("call")
                };
                assert_eq!(arguments, &[Value::Int(4)]);
                Reply::Value(Ok(Value::Int(4)))
            })
            .unwrap(),
    );
    written(repl.resume(&end).unwrap());
    assert_eq!(callbacks, 1);
}

#[test]
fn interrupt_and_close_preserve_actual_success_and_fault_terminals() {
    for source in ["1\n\n", "1 / ([0] | head)\n\n"] {
        let mut repl = Repl::new();
        let (_, start) = start_gate(&repl.feed(source.as_bytes()));
        let (_, terminal, end) = end_gate(&repl.resume(&start).unwrap());
        assert_ne!(terminal, fipe_repl_core::RunEnd::Abandoned);
        assert_eq!(repl.close(), Err(DriveError::Busy));
        assert!(repl.interrupt().is_empty());
        assert_eq!(repl.close(), Err(DriveError::Busy));
        let text = written(repl.resume(&end).unwrap());
        assert!(text.contains(if source.starts_with("1 /") {
            "division by zero"
        } else {
            "1\n"
        }));
        let outs = repl.close().unwrap();
        exit_gate(&mut repl, &outs);
    }
}

#[test]
fn check_and_candidate_file_cancellation_reject_old_same_path_ids() {
    let mut repl = Repl::new();
    let old = file_request(&repl.feed(b":t use data\n"), "data.fipe");
    written(repl.interrupt());
    assert_eq!(
        repl.file(&old, "data.fipe", Some("pub bad = 8")),
        Err(DriveError::WrongRequest)
    );
    let current = file_request(&repl.feed(b":t use data\n"), "data.fipe");
    assert_ne!(old, current);
    written(
        repl.file(&current, "data.fipe", Some("pub answer = 9"))
            .unwrap(),
    );
    let old = file_request(
        &repl.host_directory("candidate").unwrap(),
        "candidate/commands.fipe",
    );
    written(repl.interrupt());
    let current = file_request(
        &repl.host_directory("candidate").unwrap(),
        "candidate/commands.fipe",
    );
    assert_eq!(
        repl.file(&old, "candidate/commands.fipe", None),
        Err(DriveError::WrongRequest)
    );
    let outs = repl.close().unwrap();
    assert_eq!(
        repl.file(&current, "candidate/commands.fipe", None),
        Err(DriveError::WrongRequest)
    );
    exit_gate(&mut repl, &outs);
    assert_eq!(
        repl.file(&current, "candidate/commands.fipe", None),
        Err(DriveError::WrongRequest)
    );
}

#[test]
fn close_at_start_or_call_yields_abandoned_then_exit_reset_and_stays_closed() {
    for call in [false, true] {
        let mut repl = hosted_ping();
        let (_, start) = start_gate(&repl.feed(b"x = ping 1\n\n99\n\n"));
        let pending = call.then(|| call_request(&repl.resume(&start).unwrap()));
        let (_, result, end) = end_gate(&repl.close().unwrap());
        assert_eq!(result, fipe_repl_core::RunEnd::Abandoned);
        assert_eq!(repl.close(), Err(DriveError::Busy));
        assert!(repl.feed(b"100\n\n").is_empty());
        assert!(repl.interrupt().is_empty());
        let outs = repl.resume(&end).unwrap();
        let reset = reset_gate(&outs, fipe_repl_core::ResetReason::Exit);
        assert_eq!(repl.close(), Err(DriveError::Busy));
        assert!(repl.interrupt().is_empty());
        assert_eq!(repl.resume(&end), Err(DriveError::WrongContinuation));
        if let Some(id) = &pending {
            assert_eq!(
                repl.answer(id, |_, _| panic!("closed call")),
                Err(DriveError::WrongRequest)
            );
        }
        let disposed = true;
        assert_eq!(repl.resume(&reset).unwrap(), [Out::Exit]);
        assert!(disposed);
        if let Some(id) = &pending {
            assert_eq!(
                repl.answer(id, |_, _| panic!("exited call")),
                Err(DriveError::WrongRequest)
            );
        }
        assert_eq!(repl.close(), Ok(vec![]));
        assert!(repl.feed(b"x = 8\n\n").is_empty());
        assert!(repl.flush().is_empty());
        assert!(repl.interrupt().is_empty());
        assert_eq!(repl.host_sources("host", &[]), Err(DriveError::Closed));
        assert_eq!(repl.host_directory("host"), Err(DriveError::Closed));
        assert_eq!(repl.resume(&reset), Err(DriveError::WrongContinuation));
        assert!(!repl.complete("x").candidates.contains(&"x".to_owned()));
    }
}

#[test]
fn replacement_is_atomic_and_reset_tokens_preserve_the_candidate() {
    use fipe_repl_core::ResetReason;
    let mut repl = hosted_ping();
    assert!(written(
        repl.host_sources("invalid", &[("commands", "pub broken: ???")])
            .unwrap()
    )
    .contains("invalid"));
    assert!(repl.complete("pi").candidates.contains(&"ping".to_owned()));
    assert!(written(repl.host_directory("bad\0path").unwrap()).contains("NUL"));
    let file = file_request(
        &repl.host_directory("absent").unwrap(),
        "absent/commands.fipe",
    );
    assert!(
        written(repl.file(&file, "absent/commands.fipe", None).unwrap())
            .contains("not found")
    );
    let reset = reset_gate(
        &repl
            .host_sources(
                "fresh",
                &[("commands", "pub pong: Integer -> Integer")],
            )
            .unwrap(),
        ResetReason::HostReplacement,
    );
    assert!(repl.complete("pi").candidates.contains(&"ping".to_owned()));
    assert!(!repl.complete("po").candidates.contains(&"pong".to_owned()));
    let mut other = Repl::new();
    let foreign = reset_gate(&other.close().unwrap(), ResetReason::Exit);
    assert_eq!(repl.resume(&foreign), Err(DriveError::WrongContinuation));
    assert_eq!(repl.close(), Err(DriveError::Busy));
    assert!(repl.interrupt().is_empty());
    assert_eq!(repl.host_sources("another", &[]), Err(DriveError::Busy));
    assert_eq!(written(repl.resume(&reset).unwrap()), "host: fresh\nfipe> ");
    assert!(!repl.complete("pi").candidates.contains(&"ping".to_owned()));
    assert!(repl.complete("po").candidates.contains(&"pong".to_owned()));
    assert_eq!(repl.resume(&reset), Err(DriveError::WrongContinuation));
}

#[test]
fn replacement_replay_is_gated_cached_and_host_calls_are_refused() {
    use fipe::{embed::Reply, value::Value};
    let mut repl = hosted_ping();
    let file =
        file_request(&test_feed(&mut repl, b"use data\n\n"), "data.fipe");
    written(
        test_file(&mut repl, &file, "data.fipe", Some("pub answer = 3"))
            .unwrap(),
    );
    let call = call_request(&test_feed(&mut repl, b"x = ping data.answer\n\n"));
    written(
        test_answer(&mut repl, &call, |_, _| Reply::Value(Ok(Value::Int(3))))
            .unwrap(),
    );
    written(test_feed(&mut repl, b"y = 4\n\n"));
    let reset = reset_gate(
        &repl
            .host_sources(
                "replacement",
                &[("commands", "pub ping: Integer -> Integer")],
            )
            .unwrap(),
        fipe_repl_core::ResetReason::HostReplacement,
    );
    let mut outs = repl.resume(&reset).unwrap();
    let mut runs = Vec::new();
    for fault in [false, true, false] {
        let (run, start) = start_gate(&outs);
        assert!(!runs.contains(&run));
        runs.push(run.clone());
        let (terminal_run, result, end) =
            end_gate(&repl.resume(&start).unwrap());
        assert_eq!(run, terminal_run);
        assert_eq!(matches!(result, fipe_repl_core::RunEnd::Fault(_)), fault);
        // Any accidental provider Call or external File fails these exact shapes.
        outs = repl.resume(&end).unwrap();
    }
    assert_eq!(written(outs), "host: replacement\nfipe> ");
    assert!(!repl.complete("x").candidates.contains(&"x".to_owned()));
    assert_eq!(
        written(test_feed(&mut repl, b"y + data.answer\n\n")),
        "7\nfipe> "
    );
}

#[test]
fn interrupted_replay_drops_tail_but_computed_terminal_keeps_cursor() {
    use fipe_repl_core::{ResetReason, RunEnd};
    for terminal in [false, true] {
        let mut repl = Repl::new();
        written(test_feed(&mut repl, b"a = 1\n\nb = 2\n\n"));
        let reset = reset_gate(
            &repl.host_sources("fresh", &[]).unwrap(),
            ResetReason::HostReplacement,
        );
        assert!(repl.interrupt().is_empty()); // Reset candidate is preserved.
        let (_, start) = start_gate(&repl.resume(&reset).unwrap());
        let end = if terminal {
            let (_, result, end) = end_gate(&repl.resume(&start).unwrap());
            assert_eq!(result, RunEnd::Success);
            assert!(repl.interrupt().is_empty());
            end
        } else {
            let (_, result, end) = end_gate(&repl.interrupt());
            assert_eq!(result, RunEnd::Abandoned);
            end
        };
        let outs = repl.resume(&end).unwrap();
        if terminal {
            let (_, start) = start_gate(&outs);
            let (_, _, end) = end_gate(&repl.resume(&start).unwrap());
            assert_eq!(
                written(repl.resume(&end).unwrap()),
                "host: fresh\nfipe> "
            );
            assert_eq!(
                written(test_feed(&mut repl, b"a + b\n\n")),
                "3\nfipe> "
            );
        } else {
            assert_eq!(written(outs), "host: fresh\nfipe> ");
            assert!(!repl.complete("a").candidates.contains(&"a".to_owned()));
            assert!(!repl.complete("b").candidates.contains(&"b".to_owned()));
        }
    }
}

#[test]
fn replay_interrupt_retains_completed_prefix_and_close_preserves_evidence() {
    let mut repl = Repl::new();
    written(test_feed(&mut repl, b"a = 1\n\nb = 2\n\nc = 3\n\n"));
    let reset = reset_gate(
        &repl.host_sources("fresh", &[]).unwrap(),
        fipe_repl_core::ResetReason::HostReplacement,
    );
    let (_, start) = start_gate(&repl.resume(&reset).unwrap());
    let (_, _, end) = end_gate(&repl.resume(&start).unwrap());
    assert_eq!(repl.close(), Err(DriveError::Busy));
    let (_, _) = start_gate(&repl.resume(&end).unwrap());
    let (_, result, end) = end_gate(&repl.interrupt());
    assert_eq!(result, fipe_repl_core::RunEnd::Abandoned);
    written(repl.resume(&end).unwrap());
    assert_eq!(written(test_feed(&mut repl, b"a\n\n")), "1\nfipe> ");
    assert!(!repl.complete("b").candidates.contains(&"b".to_owned()));
    assert!(!repl.complete("c").candidates.contains(&"c".to_owned()));
    let reset = reset_gate(
        &repl.host_sources("again", &[]).unwrap(),
        fipe_repl_core::ResetReason::HostReplacement,
    );
    let (_, _) = start_gate(&repl.resume(&reset).unwrap());
    let (_, result, end) = end_gate(&repl.close().unwrap());
    assert_eq!(result, fipe_repl_core::RunEnd::Abandoned);
    let outs = repl.resume(&end).unwrap();
    exit_gate(&mut repl, &outs);
}

#[test]
fn buffered_quit_and_drained_eof_require_explicit_exit_reset() {
    use fipe_repl_core::ResetReason;
    let mut repl = Repl::new();
    let (_, start) = start_gate(&repl.feed(b"1\n\n:q\n99\n\n"));
    let (_, _, end) = end_gate(&repl.resume(&start).unwrap());
    let outs = repl.resume(&end).unwrap();
    let [Out::Write(text), Out::Reset { reason, resume }] = outs.as_slice()
    else {
        panic!("{outs:?}")
    };
    assert_eq!(text, b"1\n");
    assert_eq!(*reason, ResetReason::Exit);
    assert_eq!(repl.resume(resume).unwrap(), [Out::Exit]);
    let mut repl = Repl::new();
    assert!(repl.feed(b"42").is_empty());
    let (_, start) = start_gate(&repl.flush());
    assert!(repl.flush().is_empty());
    let (_, _, end) = end_gate(&repl.resume(&start).unwrap());
    assert!(repl.flush().is_empty());
    assert_eq!(written(repl.resume(&end).unwrap()), "42\n");
    let outs = repl.close().unwrap();
    exit_gate(&mut repl, &outs);
}

#[test]
fn driver_can_sequence_abandon_cleanup_replacement_replay_and_eof_disposal() {
    use fipe_repl_core::{ResetReason, RunEnd};
    let mut repl = hosted_ping();
    written(test_feed(&mut repl, b"kept = 3\n\n"));
    let (_, start) = start_gate(&repl.feed(b"new = ping kept\n\n"));
    let call = call_request(&repl.resume(&start).unwrap());
    let (_, result, end) = end_gate(&repl.interrupt());
    assert_eq!(result, RunEnd::Abandoned);
    let mut actions = vec!["run-revoked"];
    assert_eq!(
        repl.answer(&call, |_, _| panic!("late effect")),
        Err(DriveError::WrongRequest)
    );
    written(repl.resume(&end).unwrap());
    let reset = reset_gate(
        &repl.host_sources("replacement", &[]).unwrap(),
        ResetReason::HostReplacement,
    );
    actions.push("old-session-disposed");
    let (_, start) = start_gate(&repl.resume(&reset).unwrap());
    actions.push("replay-run-started");
    let (_, result, end) = end_gate(&repl.resume(&start).unwrap());
    assert_eq!(result, RunEnd::Success);
    actions.push("replay-run-committed");
    written(repl.resume(&end).unwrap());
    assert!(written(repl.flush()).ends_with('\n'));
    let reset = reset_gate(&repl.close().unwrap(), ResetReason::Exit);
    actions.push("new-session-disposed");
    assert_eq!(repl.resume(&reset).unwrap(), [Out::Exit]);
    assert_eq!(
        actions,
        [
            "run-revoked",
            "old-session-disposed",
            "replay-run-started",
            "replay-run-committed",
            "new-session-disposed"
        ]
    );
}

#[test]
fn replay_check_errors_are_diagnostics_without_execution_boundaries() {
    use fipe::{embed::Reply, value::Value};
    let mut repl = hosted_ping();
    let call = call_request(&test_feed(&mut repl, b"x = ping 1\n\n"));
    written(
        test_answer(&mut repl, &call, |_, _| Reply::Value(Ok(Value::Int(1))))
            .unwrap(),
    );
    let reset = reset_gate(
        &repl.host_sources("without-ping", &[]).unwrap(),
        fipe_repl_core::ResetReason::HostReplacement,
    );
    let text = written(repl.resume(&reset).unwrap());
    assert!(text.contains("unknown name"), "{text}");
    assert!(text.ends_with("host: without-ping\nfipe> "), "{text}");
    assert!(!repl.complete("x").candidates.contains(&"x".to_owned()));
}

fn resource_tool_repl() -> Repl {
    let mut repl = Repl::new();
    repl.hosted(true);
    written(
        test_host_sources(
            &mut repl,
            "resource-tool",
            &[(
                "commands",
                "pub type resource Table\npub acquire: () -> Table",
            )],
        )
        .unwrap(),
    );
    repl
}

fn resource_tool_answer(repl: &mut Repl, id: &RequestId) -> Vec<Out> {
    use fipe::{
        embed::{Reply, Request},
        value::resource::{ResourceDomain, ResourceType},
    };
    test_answer(repl, id, |request, tables| {
        let Request::Call { declaration, .. } = request else {
            panic!("resource acquire call")
        };
        assert_eq!(tables.declarations[*declaration].label, "acquire");
        let ty = ResourceType::registered(tables, "commands", "Table").unwrap();
        Reply::Value(Ok(ResourceDomain::new()
            .issue(&ty, std::rc::Rc::from(&b"SECRET-resource-tool-key"[..]))))
    })
    .unwrap()
}

#[test]
fn resource_tool_display_is_redacted_and_not_executable() {
    let mut repl = resource_tool_repl();
    let id = call_request(&test_feed(&mut repl, b"t = acquire ()\n\n"));
    assert_eq!(written(resource_tool_answer(&mut repl, &id)), "fipe> ");
    assert_eq!(
        written(test_feed(&mut repl, b"t\n\n")),
        "<resource>\nfipe> "
    );
    assert_eq!(
        written(test_feed(&mut repl, b"(7, t)\n\n")),
        "(7, <resource>)\nfipe> "
    );
    written(test_feed(
        &mut repl,
        b"type ResourceBox { value: Table }\n\n",
    ));
    assert_eq!(
        written(test_feed(&mut repl, b"ResourceBox t\n\n")),
        "ResourceBox <resource>\nfipe> "
    );
    let pasted = test_feed(&mut repl, b"<resource>\n\n");
    assert!(
        pasted.iter().all(|out| !matches!(out, Out::Call { .. })),
        "{pasted:?}"
    );
    let diagnostic = written(pasted);
    assert!(!diagnostic.contains("SECRET"));
    let errors = fipe::parse::parse(fipe::lex::lex("<resource>")).unwrap_err();
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|error| diagnostic.contains(&error.reason.to_string())),
        "{diagnostic}"
    );
    assert_eq!(
        written(test_feed(&mut repl, b"\"<resource>\"\n\n")),
        "\"<resource>\"\nfipe> "
    );
}

#[test]
fn resource_tool_replacement_replay_does_not_reacquire() {
    let mut repl = resource_tool_repl();
    let id = call_request(&test_feed(&mut repl, b"t = acquire ()\n\n"));
    written(resource_tool_answer(&mut repl, &id));
    let reset = reset_gate(
        &repl
            .host_sources(
                "replacement",
                &[(
                    "commands",
                    "pub type resource Table\npub acquire: () -> Table",
                )],
            )
            .unwrap(),
        fipe_repl_core::ResetReason::HostReplacement,
    );
    let (_, start) = start_gate(&repl.resume(&reset).unwrap());
    // Exact End shape rejects an unexpected Call/File without invoking any callback.
    let (_, result, end) = end_gate(&repl.resume(&start).unwrap());
    assert!(
        matches!(result, fipe_repl_core::RunEnd::Fault(_)),
        "{result:?}"
    );
    let output = written(repl.resume(&end).unwrap());
    assert!(!output.contains("<resource>"), "{output}");
    assert!(!output.contains("SECRET"), "{output}");
    assert!(!repl.complete("t").candidates.contains(&"t".to_owned()));
    assert!(written(test_feed(&mut repl, b"t\n\n")).contains("unknown name"));
}

// This small boundary fixture deliberately does not duplicate the Core grant registry.
struct ResourceOwnerRow {
    value: fipe::value::Value,
    payload: Option<String>,
}
struct ResourceOwnerPump {
    domain: fipe::value::resource::ResourceDomain,
    rows: Vec<ResourceOwnerRow>,
    new: Option<Vec<usize>>,
    events: Vec<&'static str>,
    reads: Vec<String>,
    released: usize,
    exited: bool,
}
impl ResourceOwnerPump {
    fn new() -> Self {
        Self {
            domain: fipe::value::resource::ResourceDomain::new(),
            rows: Vec::new(),
            new: None,
            events: Vec::new(),
            reads: Vec::new(),
            released: 0,
            exited: false,
        }
    }
    fn release(&mut self, index: usize) {
        if self.rows[index].payload.take().is_some() {
            self.released += 1;
            self.events.push("release");
        }
    }
    fn terminal(&mut self, result: &fipe_repl_core::RunEnd) {
        let created = self.new.take().expect("Start precedes End");
        match result {
            fipe_repl_core::RunEnd::Success => {
                self.events.push("end-success");
                for _ in created {
                    self.events.push("commit");
                }
            }
            fipe_repl_core::RunEnd::Abandoned => {
                self.events.push("end-abandon");
                for index in created {
                    self.events.push("revoke");
                    self.release(index);
                }
            }
            other @ fipe_repl_core::RunEnd::Fault(_) => {
                panic!("unexpected {other:?}")
            }
        }
    }
    fn answer(
        &mut self,
        request: &fipe::embed::Request,
        tables: &fipe::ir::Tables,
    ) -> fipe::embed::Reply {
        use fipe::{
            embed::{Reply, Request},
            value::{resource::ResourceType, Value},
        };
        assert!(self.new.is_some(), "owner opens before effects");
        let Request::Call {
            declaration,
            arguments,
            ..
        } = request
        else {
            panic!("no files")
        };
        match tables.declarations[*declaration].label.as_str() {
            "acquire" => {
                let ty = ResourceType::registered(tables, "commands", "Table")
                    .unwrap();
                let index = self.rows.len();
                let key = u64::try_from(index).unwrap().to_le_bytes();
                let value = self.domain.issue(&ty, std::rc::Rc::from(&key[..]));
                self.rows.push(ResourceOwnerRow {
                    value: value.clone(),
                    payload: Some("repl private actual row".into()),
                });
                self.new.as_mut().unwrap().push(index);
                self.events.push("create");
                Reply::Value(Ok(value))
            }
            "read_table" => {
                let row = self
                    .rows
                    .iter()
                    .find(|row| row.value == arguments[0])
                    .expect("owned alias");
                let payload = row.payload.as_ref().expect("live payload");
                self.events.push("borrow");
                self.reads.push(payload.clone());
                Reply::Value(Ok(Value::text(payload)))
            }
            other => panic!("unexpected {other}"),
        }
    }
    fn acknowledge(
        &mut self,
        repl: &mut Repl,
        resume: &fipe_repl_core::ResumeId,
        event: &'static str,
    ) -> Vec<Out> {
        let outs = repl.resume(resume).unwrap();
        self.events.push(event);
        assert_eq!(repl.resume(resume), Err(DriveError::WrongContinuation));
        outs
    }
    fn interrupt(&mut self, repl: &mut Repl, stale: &RequestId) {
        self.events.push("invalidate");
        let abandoned = repl.interrupt();
        assert!(self.pump(repl, abandoned).is_none());
        let mut callbacks = 0;
        assert_eq!(
            repl.answer(stale, |_, _| {
                callbacks += 1;
                panic!("stale callback")
            }),
            Err(DriveError::WrongRequest)
        );
        assert_eq!(callbacks, 0);
        assert_eq!(self.released, 1);
        assert!(self.rows[0].payload.is_some());
        assert!(self.rows[1].payload.is_none());
        assert!(self.events.ends_with(&[
            "pause",
            "invalidate",
            "end-abandon",
            "revoke",
            "release",
            "resume-end"
        ]));
    }
    fn pump(&mut self, repl: &mut Repl, outs: Vec<Out>) -> Option<RequestId> {
        let mut queue = VecDeque::from(outs);
        while let Some(out) = queue.pop_front() {
            match out {
                Out::Write(_) => {}
                Out::Start { resume, .. } => {
                    assert!(self.new.is_none());
                    self.events.push("start");
                    self.new = Some(Vec::new());
                    queue.extend(self.acknowledge(
                        repl,
                        &resume,
                        "resume-start",
                    ));
                }
                Out::End { result, resume, .. } => {
                    self.terminal(&result);
                    queue.extend(self.acknowledge(repl, &resume, "resume-end"));
                }
                Out::Call { id } => {
                    // Out::Call exposes only a token; this fixed source pauses after read and acquire.
                    if self.new.as_ref().unwrap().len() == 1
                        && self.rows.len() == 2
                    {
                        assert!(queue.is_empty());
                        self.events.push("pause");
                        return Some(id);
                    }
                    queue.extend(
                        repl.answer(&id, |request, tables| {
                            self.answer(request, tables)
                        })
                        .unwrap(),
                    );
                }
                Out::Reset { resume, .. } => {
                    assert!(self.new.is_none());
                    self.events.push("dispose");
                    for index in 0..self.rows.len() {
                        self.release(index);
                    }
                    queue.extend(self.acknowledge(
                        repl,
                        &resume,
                        "resume-reset",
                    ));
                }
                Out::Exit => {
                    self.events.push("exit");
                    self.exited = true;
                }
                other @ Out::File { .. } => panic!("unexpected {other:?}"),
            }
        }
        None
    }
}

#[test]
fn resource_owner_boundaries_commit_abandon_and_dispose_real_tables() {
    let mut repl = Repl::new();
    repl.hosted(true);
    let mut owner = ResourceOwnerPump::new();
    let setup = repl.host_sources("resource-owner", &[("commands", "pub type resource Table\npub acquire: () -> Table\npub read_table: Table -> String\npub pause_table: Table -> ()")]).unwrap();
    assert!(owner.pump(&mut repl, setup).is_none());
    owner.events.clear();
    let first = repl.feed(b"t = acquire ()\n\n");
    assert!(owner.pump(&mut repl, first).is_none());
    assert_eq!(
        owner.events,
        [
            "start",
            "resume-start",
            "create",
            "end-success",
            "commit",
            "resume-end"
        ]
    );
    assert_eq!(owner.released, 0);
    let next = repl.feed(
        b"{\nseen = read_table t\nfresh = acquire ()\npause_table fresh\n}\n\n",
    );
    let stale = owner.pump(&mut repl, next).expect("real pending pause");
    assert_eq!(owner.rows.len(), 2);
    assert_eq!(owner.released, 0);
    assert_eq!(owner.reads, ["repl private actual row"]);
    owner.interrupt(&mut repl, &stale);
    let read = repl.feed(b"read_table t\n\n");
    assert!(owner.pump(&mut repl, read).is_none());
    assert_eq!(owner.released, 1);
    assert_eq!(
        owner.reads,
        ["repl private actual row", "repl private actual row"]
    );
    let close = repl.close().unwrap();
    assert!(owner.pump(&mut repl, close).is_none());
    assert!(owner.exited);
    assert_eq!(owner.released, 2);
    assert!(owner.events.ends_with(&[
        "dispose",
        "release",
        "resume-reset",
        "exit"
    ]));
    assert!(repl.close().unwrap().is_empty());
    assert_eq!(owner.released, 2);
}
