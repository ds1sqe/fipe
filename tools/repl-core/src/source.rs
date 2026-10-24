//! Source-text continuation facts, independent of an evaluator or REPL instance.
use alloc::{string::String, vec::Vec};
use fipe::{
    lex::{lex, Lexed, Reason},
    parse,
    token::{Kind, Symbol, Token},
};

/// Whether ordinary source can be submitted, or contains an unfinished construct.
/// Neither variant promises that the source is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceContinuation {
    /// Submit for the language's ordinary diagnostics.
    Complete,
    /// More physical text may be appropriate. A valid completion is not promised.
    Open(ContinuationReason),
}

/// The unfinished construct, without exposing mutable editor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuationReason {
    /// A pipe or explicit backslash joins another physical line.
    LineContinuation,
    /// An opening delimiter has not been closed.
    Delimiter,
    /// A raw quote or an escaped physical line remains open.
    Quote,
    /// A standalone documentation comment awaits its item.
    Documentation,
}

/// Inspect exact UTF-8 source without a Session, lookup, evaluation or I/O.
///
/// `Complete` permits submission for diagnostics; `Open` is not a guarantee that
/// a valid suffix exists. A caller may always force submission. Final LF/CRLF
/// is significant. Colon commands and a terminal REPL's blank-line policy are
/// not part of this query.
#[must_use]
pub fn source_continuation(text: &str) -> SourceContinuation {
    use ContinuationReason::{
        Delimiter, Documentation, LineContinuation, Quote,
    };
    use SourceContinuation::{Complete, Open};
    let lexed = lex(text);
    let Some(open_delimiter) = delimiters(&lexed.tokens) else {
        return Complete;
    };
    if !lexed.errors.is_empty() {
        return if continuing_quote(text, &lexed) {
            Open(Quote)
        } else {
            Complete
        };
    }
    let doc = pending_doc(&lexed.tokens);
    let line = continuing_line(text, &lexed);
    // A real contradictory token is ready for diagnostics. EOF alone does not
    // create continuation: only the lexical/structural facts above can do that.
    if parse::parse(lexed).is_err_and(|errors| {
        errors.iter().any(|error| {
            error.span.start < text.len()
                || matches!(
                    error.reason,
                    parse::Reason::NestingLimit
                        | parse::Reason::DepthLimit
                        | parse::Reason::InvalidTokenStream
                )
        })
    }) {
        return Complete;
    }
    if open_delimiter {
        Open(Delimiter)
    } else if doc {
        Open(Documentation)
    } else if line {
        Open(LineContinuation)
    } else {
        Complete
    }
}

fn delimiters(tokens: &[Token]) -> Option<bool> {
    let mut expected = Vec::new();
    for token in tokens {
        let Kind::Symbol(symbol) = &token.kind else {
            continue;
        };
        match symbol {
            Symbol::LParen => expected.push(Symbol::RParen),
            Symbol::LBracket => expected.push(Symbol::RBracket),
            Symbol::LBrace => expected.push(Symbol::RBrace),
            Symbol::RParen | Symbol::RBracket | Symbol::RBrace
                if expected.pop() != Some(*symbol) =>
            {
                return None;
            }
            _ => {}
        }
    }
    Some(!expected.is_empty())
}

fn continuing_quote(text: &str, lexed: &Lexed) -> bool {
    let [error] = lexed.errors.as_slice() else {
        return false;
    };
    if error.span.end != text.len() {
        return false;
    }
    let written = &text[error.span.start..error.span.end];
    let Some(at) = written.find(['\'', '"']) else {
        return false;
    };
    match written.as_bytes()[at] {
        b'\'' => error.reason == Reason::UnterminatedQuote,
        b'"' if matches!(
            error.reason,
            Reason::UnterminatedQuote | Reason::BadEscape
        ) =>
        {
            let mut next = String::from(text);
            next.push('\n');
            let lookahead = lex(&next);
            let [after] = lookahead.errors.as_slice() else {
                return false;
            };
            if after.span.start == error.span.start
                && after.span.end == next.len()
                && after.reason == Reason::UnterminatedQuote
            {
                return true;
            }
            if error.reason != Reason::UnterminatedQuote {
                return false;
            }
            let Some(newline) = written.rfind('\n') else {
                return false;
            };
            indentation(&written[newline + 1..])
        }
        _ => false,
    }
}

fn indentation(text: &str) -> bool {
    let lexed = lex(text);
    lexed.errors.is_empty()
        && lexed.comments.is_empty()
        && lexed
            .tokens
            .iter()
            .all(|token| matches!(token.kind, Kind::Eof))
}

fn continuing_line(text: &str, lexed: &Lexed) -> bool {
    let last = lexed
        .tokens
        .iter()
        .rev()
        .find(|token| !matches!(token.kind, Kind::Eof));
    let implicit = last.is_some_and(|token| matches!(token.kind, Kind::Symbol(symbol) if symbol.continues_line()));
    let mut cursor = last.map_or(0, |token| token.span.end);
    let mut explicit = false;
    for comment in &lexed.comments {
        if comment.start < cursor {
            continue;
        }
        join_gap(&text[cursor..comment.start], &mut explicit);
        cursor = comment.end;
    }
    join_gap(&text[cursor..], &mut explicit);
    implicit || explicit
}

fn join_gap(text: &str, pending: &mut bool) {
    let lexed = lex(text);
    if !lexed.errors.is_empty() {
        return;
    }
    if lexed
        .tokens
        .iter()
        .any(|token| matches!(token.kind, Kind::Newline))
    {
        *pending = false;
    }
    let end = lexed
        .tokens
        .iter()
        .rev()
        .find(|token| !matches!(token.kind, Kind::Eof))
        .map_or(0, |token| token.span.end);
    // The lexer has already removed joins. Only its uncovered, comment-free
    // trivia can contain a backslash here; quote/comment bytes never enter it.
    if text[end..].contains('\\') && indentation(&text[end..]) {
        *pending = true;
    }
}

/// A lexer failure at EOF that another physical line can complete.
pub fn open_quote(code: &str, lexed: &Lexed) -> Option<char> {
    lexed.errors.iter().find_map(|error| {
        if error.span.end != code.len() {
            return None;
        }
        let written = &code[error.span.start..error.span.end];
        let at = written.find(['\'', '"'])?;
        let quote = written.as_bytes()[at] as char;
        ((quote == '\'' && !written[at + 1..].contains('\''))
            || (quote == '"' && written.ends_with('\\')))
        .then_some(quote)
    })
}

pub fn pending_doc(tokens: &[Token]) -> bool {
    let Some(last) = tokens
        .iter()
        .rposition(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
    else {
        return false;
    };
    if !matches!(tokens[last].kind, Kind::Doc(_) | Kind::InnerDoc(_)) {
        return false;
    }
    last == 0 || matches!(tokens[last - 1].kind, Kind::Newline)
}
