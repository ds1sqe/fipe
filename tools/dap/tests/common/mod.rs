//! What the session tests compare with: an example's trace fixture.
use std::path::PathBuf;

/// The repository's root.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// What a session over an example sends, by its trace fixture: the
/// `console` lines (each call, then the value or Fault), the success
/// and the error output.
pub struct Expected {
    pub console: Vec<String>,
    pub success: String,
    pub error: String,
}

pub fn expected(example: &str) -> Expected {
    let path = root().join(format!("tests/fixtures/traces/{example}.txt"));
    let text = std::fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut console: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with("call "))
        .map(|line| format!("{line}\n"))
        .collect();
    console.push(format!("{}\n", lines[0]));
    let quoted = |label: &str| {
        let line = lines
            .iter()
            .find_map(|line| line.strip_prefix(label))
            .unwrap();
        unescape(line)
    };
    Expected {
        console,
        success: quoted("success "),
        error: quoted("error "),
    }
}

/// A Rust string literal's text, as the trace writes its outputs.
fn unescape(literal: &str) -> String {
    let inner = &literal[1..literal.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let code: String =
                    chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                out.push(
                    char::from_u32(u32::from_str_radix(&code, 16).unwrap())
                        .unwrap(),
                );
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
