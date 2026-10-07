use crate::token::{Affixed, Keyword, Kind, Magnitude, Symbol, Written};
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

pub(super) fn classify(word: &str) -> Kind {
    if let Some(number) = number(word) {
        return number;
    }
    let symbol = match word {
        "+" => Some(Symbol::Plus),
        "-" => Some(Symbol::Minus),
        "*" => Some(Symbol::Star),
        "/" => Some(Symbol::Slash),
        "%" => Some(Symbol::Percent),
        "++" => Some(Symbol::Concat),
        _ => None,
    };
    if let Some(symbol) = symbol {
        return Kind::Symbol(symbol);
    }
    let flag = word.strip_prefix("--").or_else(|| word.strip_prefix('-'));
    if let Some(flag) =
        flag.filter(|s| s.starts_with(|ch: char| ch.is_ascii_alphabetic()))
    {
        return Kind::Flag(flag.to_string());
    }
    if word.starts_with('/')
        || word.starts_with("./")
        || word.starts_with("../")
    {
        return Kind::Path(word.to_string());
    }
    if let Some(selector) = word.strip_prefix('.').and_then(name) {
        return Kind::Selector(selector);
    }
    if let Some(keyword) = Keyword::from_word(word) {
        return Kind::Kw(keyword);
    }
    name(word).map_or_else(|| Kind::Word(word.to_string()), Kind::Name)
}

/// A quote's prefix: a name with no uppercase letter, not a keyword.
pub(super) fn prefix(word: &str) -> bool {
    word.starts_with(|ch: char| ch.is_ascii_lowercase() || ch == '_')
        && word.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'
        })
        && Keyword::from_word(word).is_none()
}

/// A quote's suffix: a one-segment name, not a keyword.
pub(super) fn suffix(word: &str) -> bool {
    identifier(word) && Keyword::from_word(word).is_none()
}

fn identifier(part: &str) -> bool {
    let mut chars = part.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn name(word: &str) -> Option<Vec<String>> {
    word.split('.')
        .all(identifier)
        .then(|| word.split('.').map(ToString::to_string).collect())
}

/// A number, read by the order of lexical.md §Numbers: a radix word, the
/// longest decimal number, a program's prefix after a lone `0`, else a
/// suffix. `None` is a word.
fn number(word: &str) -> Option<Kind> {
    let (negative, body) =
        word.strip_prefix('-').map_or((false, word), |s| (true, s));
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = body.strip_prefix(prefix) {
            let end = digits_end(rest, radix);
            if end == 0 {
                return None;
            }
            let (digits, suffix) = rest.split_at(end);
            let form = Written::Int {
                negative,
                magnitude: Magnitude::parse(digits, radix)?,
                radix: u8::try_from(radix).ok()?,
                digits: digits.bytes().filter(|b| *b != b'_').count(),
            };
            return affixed(form, suffix);
        }
    }
    let (end, float) = decimal_end(body)?;
    let (spelling, rest) = word.split_at(word.len() - body.len() + end);
    let form = if float {
        Written::Float(spelling.to_string())
    } else {
        Written::Int {
            negative,
            magnitude: Magnitude::parse(&body[..end], 10)?,
            radix: 10,
            digits: body[..end].bytes().filter(|b| *b != b'_').count(),
        }
    };
    if &body[..end] == "0" {
        if let Some(digits) = rest.strip_prefix(program_prefix) {
            if !digits.is_empty()
                && digits.chars().all(|ch| ch.is_ascii_alphanumeric())
            {
                return Some(Kind::Affixed(Box::new(Affixed {
                    prefix: rest[..1].to_string(),
                    form: Written::Digits {
                        negative,
                        digits: digits.to_string(),
                    },
                    suffix: String::new(),
                })));
            }
        }
    }
    affixed(form, rest)
}

/// The letter of a program's number prefix; `x`, `o` and `b` are the
/// lexer's own.
const fn program_prefix(ch: char) -> bool {
    ch.is_ascii_lowercase() && !matches!(ch, 'x' | 'o' | 'b')
}

/// A number with no suffix, or with one that is a letter and then name
/// characters.
fn affixed(form: Written, suffix: &str) -> Option<Kind> {
    if suffix.is_empty() {
        return Some(match form {
            Written::Int {
                negative,
                magnitude,
                radix,
                digits,
            } => Kind::Int {
                negative,
                magnitude,
                radix,
                digits,
            },
            Written::Float(spelling) => Kind::Float(spelling),
            Written::Digits { .. } | Written::Text { .. } => return None,
        });
    }
    let valid = suffix.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && identifier(suffix);
    valid.then(|| {
        Kind::Affixed(Box::new(Affixed {
            prefix: String::new(),
            form,
            suffix: suffix.to_string(),
        }))
    })
}

/// The end of the longest run of `radix` digits with single underscores
/// between them.
fn digits_end(text: &str, radix: u32) -> usize {
    let bytes = text.as_bytes();
    let digit = |at: usize| {
        bytes
            .get(at)
            .is_some_and(|&byte| char::from(byte).is_digit(radix))
    };
    let mut end = 0;
    while digit(end) {
        end += 1;
        if bytes.get(end) == Some(&b'_') && digit(end + 1) {
            end += 1;
        }
    }
    end
}

/// The end of the longest decimal integer or float at the start, and
/// whether it is a float; `None` when no digit starts it.
fn decimal_end(body: &str) -> Option<(usize, bool)> {
    let mut end = digits_end(body, 10);
    if end == 0 {
        return None;
    }
    let mut float = false;
    if body[end..].starts_with('.') {
        let fraction = digits_end(&body[end + 1..], 10);
        if fraction > 0 {
            end += 1 + fraction;
            float = true;
        }
    }
    if body[end..].starts_with(['e', 'E']) {
        let sign = usize::from(body[end + 1..].starts_with(['+', '-']));
        let exponent = digits_end(&body[end + 1 + sign..], 10);
        if exponent > 0 {
            end += 1 + sign + exponent;
            float = true;
        }
    }
    Some((end, float))
}
