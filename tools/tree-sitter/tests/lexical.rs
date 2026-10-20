//! The grammar's leaves against `fipe::lex`: for every source the crate
//! lexes without a diagnostic, the grammar's flat token rule yields the
//! same tokens, as (kind, byte span), and the same comment spans.
//! Suppressed newlines and line joins are hidden in the tree, as they
//! yield no token from the crate.

use fipe::lex::lex;
use fipe::token::{Kind, Written};
use std::path::{Path, PathBuf};

type Tokens = Vec<(String, usize, usize)>;
type Spans = Vec<(usize, usize)>;

/// The language repository holding the corpus: `FIPE_ROOT`, else the
/// checkout this package sits in.
fn root() -> PathBuf {
    std::env::var_os("FIPE_ROOT").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    )
}

/// The grammar's node kind for a crate token; `None` for EOF.
fn kind(kind: &Kind) -> Option<String> {
    Some(match kind {
        Kind::Kw(keyword) => keyword.spelling().to_string(),
        Kind::Symbol(symbol) => symbol.spelling().to_string(),
        Kind::Name(_) => "name".into(),
        Kind::Word(_) => "word".into(),
        Kind::Flag(_) => "flag".into(),
        Kind::Path(_) => "path".into(),
        Kind::Selector(_) => "selector".into(),
        Kind::Int { .. } => "integer".into(),
        Kind::Float(_) => "float".into(),
        Kind::Str { raw: false, .. } => "string".into(),
        Kind::Str { raw: true, .. } => "raw_string".into(),
        Kind::Affixed(affixed) => match &affixed.form {
            Written::Int { .. } | Written::Digits { .. } => "integer".into(),
            Written::Float(_) => "float".into(),
            Written::Text { raw: false, .. } => "string".into(),
            Written::Text { raw: true, .. } => "raw_string".into(),
        },
        Kind::Doc(_) => "doc".into(),
        Kind::InnerDoc(_) => "inner_doc".into(),
        Kind::Newline => "newline".into(),
        Kind::Eof => return None,
    })
}

/// The crate's tokens and comments, or `None` when it reports an error.
fn expected(text: &str) -> Option<(Tokens, Spans)> {
    let lexed = lex(text);
    if !lexed.errors.is_empty() {
        return None;
    }
    let tokens = lexed
        .tokens
        .iter()
        .filter_map(|t| kind(&t.kind).map(|k| (k, t.span.start, t.span.end)))
        .collect();
    let comments = lexed.comments.iter().map(|s| (s.start, s.end)).collect();
    Some((tokens, comments))
}

/// The grammar's leaves and comments; `Err` names an error or missing
/// node.
fn grammar(text: &str) -> Result<(Tokens, Spans), String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_fipe::LANGUAGE.into())
        .expect("the Fipe grammar loads");
    let tree = parser.parse(text, None).expect("a tree");
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            return Err(format!(
                "{} at {}..{}",
                node.kind(),
                node.start_byte(),
                node.end_byte()
            ));
        }
        if node.child_count() == 0 && node.kind() != "file" {
            let span = (node.start_byte(), node.end_byte());
            if node.kind() == "comment" {
                comments.push(span);
            } else {
                tokens.push((node.kind().to_string(), span.0, span.1));
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Ok((tokens, comments));
            }
        }
    }
}

/// Compare one source; `false` when the crate rejects it.
fn compare(label: &str, text: &str) -> bool {
    let Some(expected) = expected(text) else {
        return false;
    };
    let found = grammar(text)
        .unwrap_or_else(|error| panic!("{label}: {error}\n{text}"));
    assert_eq!(found.0, expected.0, "{label}: tokens\n{text}");
    assert_eq!(found.1, expected.1, "{label}: comments\n{text}");
    true
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "fipe") {
            out.push(path);
        }
    }
}

#[test]
fn the_corpus_files_lex_as_the_crate_does() {
    let root = root();
    let mut paths = Vec::new();
    for dir in ["Documentation", "prelude", "tests/fixtures"] {
        files(&root.join(dir), &mut paths);
    }
    assert!(
        paths.len() >= 15,
        "corpus not found under {}",
        root.display()
    );
    for path in &paths {
        let text = std::fs::read_to_string(path).unwrap();
        let label = path.display().to_string();
        assert!(compare(&label, &text), "{label}: the crate rejects it");
    }
}

/// Every `fipe` code block of the specification and API chapters the crate
/// lexes without a diagnostic.
#[test]
fn the_chapter_blocks_lex_as_the_crate_does() {
    let root = root();
    let mut compared = 0;
    for dir in ["Documentation/spec", "Documentation/api"] {
        let mut entries: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        entries.sort();
        for path in entries {
            let text = std::fs::read_to_string(&path).unwrap();
            let mut block: Option<String> = None;
            for (number, line) in text.lines().enumerate() {
                match &mut block {
                    None if line.trim_start() == "```fipe" => {
                        block = Some(String::new());
                    }
                    Some(body) if line.trim_start() == "```" => {
                        let label = format!("{}:{}", path.display(), number);
                        if compare(&label, body) {
                            compared += 1;
                        }
                        block = None;
                    }
                    Some(body) => {
                        body.push_str(line);
                        body.push('\n');
                    }
                    None => {}
                }
            }
        }
    }
    assert!(compared >= 10, "only {compared} blocks compared");
}

/// The rows of 01-lexical.md, one source each, and the edges of each
/// rule the grammar or the scanner implements.
const CASES: &[&str] = &[
    // Boundaries and classification.
    "a+b x*2 my-file 2024-01-01 fs.Entry",
    "a.b a. a..b .size .a.b .5 ./x ../x / // /tmp/a+b",
    "x * 2 a - b a + b a % b a ++ b a / b",
    "+x ++x +++ -- --- ---x -x --x -x-y -n=-5 -c=80 --name=a",
    "if else with type impl use pub iff if-x _ __ True self Self",
    "-> ->x a->b --> => == != <= >= < > ! & && | || |: |!",
    "( ) { } [ ] , ; : = Vec<Vec<U8>> a<b f(x) (e).name",
    // Numbers.
    "0 -0 7 -7 1_000 1__0 1_ 0_1 007 0x1F 0x1Fe 0x1Fg 0x 0xZZ 0xg 0x1_g",
    "0o17 0o78 0o 0b0111 0b2 0b -0x1F 00x5",
    "1.5 1. .5 1.5.3 1.e5 1e5 1E5 1e+5 1e-5 1e 1e+ 1e_5 1ea 1e5x 1e5e5",
    "1.5e5 1.5e 1.5ex 1.5e5_ 1.5_ 1e5- 1e5-z 1e5_",
    "10ms 2i 1st 0ms 0r7Z 0r_1 0m 0e 0e5 0X1 0B1 0a 0xa 0x1Fe_",
    "-1e -.5 --5 -0x 0x1Fg_h 12ab_3 1x_ 1_a",
    // Words with backslashes, and line joins.
    "a\\b \\\\x a\\\\b",
    "cat x \\\n    y\n",
    "cat x\\\n    y\n",
    "cat x\\ \t\n    y\n",
    "cat x\\\\\ny\n",
    "a\\ b\n",
    "f(a\\) b\n",
    "a\\\"x\"\n",
    "\\ x\n",
    "x 12\\\n-n\\\n/tmp\\\n.sel\\\nif\\\n+\\\nfs.x\\\n1.5\\\nw+w\\\n",
    "end\\",
    "end \\",
    "a |\\\n\nb\n",
    // Comments, docs and pipe continuation.
    "a | # comment\n  b\n",
    "a |\n\nb\n",
    "a ||\n  b &&\n c |: d |!\n e\n",
    "a | ## doc\n  b\n",
    "a | b\nc\n",
    "a |b\nc\n",
    "# c\n## doc\n##! inner\n### banner\n#x\nx# y\n",
    "##\n## \n##!\n",
    "a | # c\n# d\nb\n",
    "a |   \n   b\n",
    // Quoted literals.
    "\"abc\" 'raw' \"a\\nb\\t\\\\\\\"\\'\\0\\x41\" ''",
    "re\"a+b\" \"a+b\"i re\"x\"i b\"\\xFF\" f \"abc\" f\"abc\"",
    "Re\"x\" if\"x\" \"x\"if \"x\"ifx \"x\"x+y \"x\"x.y _\"x\" a1\"x\" a-b\"x\"",
    "banner = '\n  Fipe\n'\n",
    "msg = \"the file \\\n       was not found\"\n",
    "msg = \"line one\\n\\\n       line two\"\n",
    "sql = 'where name = \"a\" # text\n       and size > 1'\n",
    "'a'\"b\" \"a\"'b' x'y' 'x'y",
    "a | 'x\ny' | b\n",
    // Unicode, CR, BOM.
    "\u{feff}x = 1\n",
    "ä é 名前 a\u{a0}b c\r\nd\r\n",
    "x\u{3000}y\u{2028}z\n",
];

#[test]
fn the_lexical_rows_lex_as_the_crate_does() {
    for (index, case) in CASES.iter().enumerate() {
        let label = format!("case {index}");
        assert!(compare(&label, case), "{label}: the crate rejects {case:?}");
    }
}

/// The one known difference: a quote's suffix that a line join ends.
/// The crate sees the suffix; the scanner would have to look past the
/// backslash to tell a join from a word character, so it ends the quote
/// before the suffix, which then lexes as a name.
#[test]
fn a_suffix_ended_by_a_line_join_is_a_name_of_its_own() {
    let text = "\"a\"i\\\nx\n";
    let (crate_tokens, _) = expected(text).expect("the crate accepts it");
    assert_eq!(crate_tokens[0], ("string".to_string(), 0, 4));
    let (tokens, _) = grammar(text).expect("no error node");
    assert_eq!(tokens[0], ("string".to_string(), 0, 3));
    assert_eq!(tokens[1], ("name".to_string(), 3, 4));
}
