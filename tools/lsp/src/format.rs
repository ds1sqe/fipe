//! Formatting from the tokens and the comment spans, keeping the
//! author's line breaks.
//!
//! Each line is indented four spaces per bracket open at its start, and
//! a line that starts with a pipe continues the line above, one level
//! deeper. Within a line, tokens that touch keep touching, since
//! touching changes the reading (`f(x)`, `x.name`, `-c=1`), and any
//! other gap becomes one space; the gap before a comment is kept, so
//! aligned comments stay aligned. A doc keeps its text whole, trailing
//! spaces included, since in Markdown they break the line. A line that starts inside a token,
//! such as a quote across lines, or after a line join, is kept as
//! written. Trailing spaces go, and the file ends with one newline.
//!
//! A file with a lexical or syntax error is refused, and so is any
//! result whose tokens or touching differ from the input's.

use fipe::lex::{lex, Lexed};
use fipe::parse::parse;
use fipe::token::{Kind, Symbol};

/// What one item of a line is, for indentation and spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Open,
    Close,
    Pipe,
    Comment,
    Doc,
    Other,
}

/// A token or comment with its byte span.
#[derive(Clone, Copy, Debug)]
struct Item {
    start: usize,
    end: usize,
    class: Class,
}

/// The formatted text of `text`, or why it is refused.
///
/// # Errors
/// The text has a lexical or syntax error, or formatting would change
/// what it reads as.
pub fn format(text: &str) -> Result<String, &'static str> {
    let lexed = lex(text);
    if !lexed.errors.is_empty() || parse(lex(text)).is_err() {
        return Err("the file has syntax errors");
    }
    let formatted = lines(text, &items(&lexed));
    if shape(&lex(&formatted)) != shape(&lexed)
        || parse(lex(&formatted)).is_err()
    {
        return Err("formatting would change what the file reads as");
    }
    Ok(formatted)
}

/// The tokens and comments of `lexed`, in source order.
fn items(lexed: &Lexed) -> Vec<Item> {
    let mut items: Vec<Item> = lexed
        .tokens
        .iter()
        .filter(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
        .map(|token| Item {
            start: token.span.start,
            end: token.span.end,
            class: class(&token.kind),
        })
        .collect();
    items.extend(lexed.comments.iter().map(|span| Item {
        start: span.start,
        end: span.end,
        class: Class::Comment,
    }));
    items.sort_by_key(|item| item.start);
    items
}

const fn class(kind: &Kind) -> Class {
    match kind {
        Kind::Symbol(Symbol::LParen | Symbol::LBrace | Symbol::LBracket) => {
            Class::Open
        }
        Kind::Symbol(Symbol::RParen | Symbol::RBrace | Symbol::RBracket) => {
            Class::Close
        }
        Kind::Symbol(
            Symbol::Pipe
            | Symbol::PipeGood
            | Symbol::PipeBad
            | Symbol::Or
            | Symbol::And,
        ) => Class::Pipe,
        Kind::Doc(_) | Kind::InnerDoc(_) => Class::Doc,
        _ => Class::Other,
    }
}

/// The text of `text` laid out line by line from its `items`.
fn lines(text: &str, items: &[Item]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth: usize = 0;
    let mut next = 0;
    let mut start = 0;
    let mut joined = false;
    while start <= text.len() {
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let line = &text[start..end];
        let inside = items
            .iter()
            .any(|item| item.start < start && start < item.end);
        let first = next;
        while next < items.len() && items[next].start < end {
            next += 1;
        }
        let on_line = &items[first..next];
        if inside || joined {
            out.push_str(line.trim_end());
        } else if let Some(head) = on_line.first() {
            if line[..head.start - start].trim().is_empty() {
                let mut level = depth;
                if head.class == Class::Close {
                    level = level.saturating_sub(1);
                }
                if head.class == Class::Pipe {
                    level += 1;
                }
                out.push_str(&"    ".repeat(level));
                out.push_str(&body(text, on_line, end));
            } else {
                out.push_str(line.trim_end());
            }
        }
        for item in on_line {
            match item.class {
                Class::Open => depth += 1,
                Class::Close => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        joined =
            !inside_comment(on_line, end) && line.trim_end().ends_with('\\');
        if end == text.len() {
            break;
        }
        out.push('\n');
        start = end + 1;
    }
    let trimmed = out.trim_end_matches('\n').len();
    out.truncate(trimmed);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Whether the line's last item is a comment reaching its end, so a
/// final backslash is the comment's and joins nothing.
fn inside_comment(on_line: &[Item], end: usize) -> bool {
    on_line
        .last()
        .is_some_and(|item| item.class == Class::Comment && item.end >= end)
}

/// The items of one line with their gaps laid out, ending at `end`.
fn body(text: &str, on_line: &[Item], end: usize) -> String {
    let mut body = String::new();
    for (index, item) in on_line.iter().enumerate() {
        if index > 0 {
            let gap = &text[on_line[index - 1].end..item.start];
            let comment = matches!(item.class, Class::Comment | Class::Doc);
            if comment || !gap.trim().is_empty() {
                body.push_str(gap);
            } else if !gap.is_empty() {
                body.push(' ');
            }
        }
        let piece = &text[item.start..item.end.min(end)];
        if item.class == Class::Comment {
            body.push_str(piece.trim_end());
        } else {
            body.push_str(piece);
        }
    }
    let last = on_line.last().map_or(end, |item| item.end.min(end));
    let tail = &text[last..end];
    if !tail.trim().is_empty() {
        body.push_str(tail.trim_end());
    }
    body
}

/// What formatting must keep: each token's kind, and whether it
/// touches the token before it on its line. Newlines at the end of the
/// file are not kept: they end nothing.
fn shape(lexed: &Lexed) -> Vec<(Kind, bool)> {
    let mut previous: Option<usize> = None;
    let mut shape: Vec<(Kind, bool)> = lexed
        .tokens
        .iter()
        .filter(|token| token.kind != Kind::Eof)
        .map(|token| {
            if token.kind == Kind::Newline {
                return (Kind::Newline, false);
            }
            let touches = previous == Some(token.span.start);
            previous = Some(token.span.end);
            (token.kind.clone(), touches)
        })
        .collect();
    while shape.last().is_some_and(|(kind, _)| *kind == Kind::Newline) {
        shape.pop();
    }
    shape
}
