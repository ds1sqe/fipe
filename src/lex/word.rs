use super::Reason;
use crate::token::{Keyword, Kind, Symbol};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

pub(super) fn classify(word: &str) -> Result<Kind, Reason> {
    if let Some(number) = number(word) {
        return number;
    }
    let symbol = match word {
        "+" => Some(Symbol::Plus),
        "-" => Some(Symbol::Minus),
        "*" => Some(Symbol::Star),
        "/" => Some(Symbol::Slash),
        "%" => Some(Symbol::Percent),
        _ => None,
    };
    if let Some(symbol) = symbol {
        return Ok(Kind::Symbol(symbol));
    }
    let flag = word.strip_prefix("--").or_else(|| word.strip_prefix('-'));
    if let Some(flag) =
        flag.filter(|s| s.starts_with(|ch: char| ch.is_ascii_alphabetic()))
    {
        return Ok(Kind::Flag(flag.to_string()));
    }
    if word.starts_with('/')
        || word.starts_with("./")
        || word.starts_with("../")
    {
        return Ok(Kind::Path(word.to_string()));
    }
    if let Some(selector) = word.strip_prefix('.').and_then(name) {
        return Ok(Kind::Selector(selector));
    }
    if let Some(keyword) = Keyword::from_word(word) {
        return Ok(Kind::Kw(keyword));
    }
    Ok(name(word).map_or_else(|| Kind::Word(word.to_string()), Kind::Name))
}

fn name(word: &str) -> Option<Vec<String>> {
    let valid = word.split('.').all(|part| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    });
    valid.then(|| word.split('.').map(ToString::to_string).collect())
}

fn number(word: &str) -> Option<Result<Kind, Reason>> {
    let (negative, body) =
        word.strip_prefix('-').map_or((false, word), |s| (true, s));
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = body.strip_prefix(prefix) {
            return digits_valid(digits, radix)
                .then(|| integer(digits, radix, negative));
        }
    }
    if digits_valid(body, 10) {
        return Some(integer(body, 10, negative));
    }
    is_float(body).then(|| Ok(Kind::Float(word.to_string())))
}

fn integer(text: &str, radix: u32, negative: bool) -> Result<Kind, Reason> {
    magnitude(text, radix)
        .map(|magnitude| Kind::Int {
            negative,
            magnitude,
        })
        .ok_or(Reason::IntegerRange)
}

fn magnitude(text: &str, radix: u32) -> Option<u64> {
    text.chars()
        .filter(|&ch| ch != '_')
        .try_fold(0_u64, |n, ch| {
            n.checked_mul(u64::from(radix))?
                .checked_add(u64::from(ch.to_digit(radix)?))
        })
}

fn digits_valid(text: &str, radix: u32) -> bool {
    let mut needs_digit = true;
    for ch in text.chars() {
        if ch.is_digit(radix) {
            needs_digit = false;
        } else if ch == '_' && !needs_digit {
            needs_digit = true;
        } else {
            return false;
        }
    }
    !needs_digit
}

fn is_float(body: &str) -> bool {
    let (mantissa, has_exponent) =
        if let Some((mantissa, exponent)) = body.split_once(['e', 'E']) {
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            if !digits_valid(digits, 10) {
                return false;
            }
            (mantissa, true)
        } else {
            (body, false)
        };
    if let Some((whole, fraction)) = mantissa.split_once('.') {
        digits_valid(whole, 10) && digits_valid(fraction, 10)
    } else {
        has_exponent && digits_valid(mantissa, 10)
    }
}
