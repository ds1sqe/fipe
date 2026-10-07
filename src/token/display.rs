use super::{fmt, Affixed, Kind, Written};
use alloc::string::String;

/// Canonical spelling for diagnostics; use `Source::slice` for exact spelling.
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kw(word) => word.fmt(f),
            Self::Symbol(symbol) => symbol.fmt(f),
            Self::Name(parts) => name(f, parts),
            Self::Selector(parts) => {
                f.write_str(".")?;
                name(f, parts)
            }
            Self::Word(text) | Self::Path(text) | Self::Float(text) => {
                f.write_str(text)
            }
            Self::Flag(text) => write!(f, "-{text}"),
            Self::Int {
                negative,
                magnitude,
                radix,
                digits,
            } => integer(f, *negative, magnitude, *radix, *digits),
            Self::Str { value, raw: true } => write!(f, "'{value}'"),
            Self::Str { value, raw: false } => quoted(f, value.as_bytes()),
            Self::Affixed(affixed) => affixed.fmt(f),
            Self::Doc(text) => write!(f, "## {text}"),
            Self::InnerDoc(text) => write!(f, "##! {text}"),
            Self::Newline => f.write_str("\n"),
            Self::Eof => f.write_str("<eof>"),
        }
    }
}

impl fmt::Display for Affixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = |negative: bool| if negative { "-" } else { "" };
        match &self.form {
            Written::Int {
                negative,
                magnitude,
                radix,
                digits,
            } => {
                integer(f, *negative, magnitude, *radix, *digits)?;
            }
            Written::Float(spelling) => f.write_str(spelling)?,
            Written::Digits { negative, digits } => {
                write!(f, "{}0{}{digits}", sign(*negative), self.prefix)?;
            }
            Written::Text { value, raw } => {
                f.write_str(&self.prefix)?;
                // A raw quote holds source text, so it is UTF-8.
                match core::str::from_utf8(value) {
                    Ok(text) if *raw => write!(f, "'{text}'")?,
                    _ => quoted(f, value)?,
                }
            }
        }
        f.write_str(&self.suffix)
    }
}

fn name(f: &mut fmt::Formatter<'_>, parts: &[String]) -> fmt::Result {
    for (index, part) in parts.iter().enumerate() {
        if index != 0 {
            f.write_str(".")?;
        }
        f.write_str(part)?;
    }
    Ok(())
}

fn quoted(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    f.write_str("\"")?;
    for byte in bytes {
        match byte {
            b'\n' => f.write_str("\\n")?,
            b'\t' => f.write_str("\\t")?,
            b'\r' => f.write_str("\\r")?,
            b'\\' => f.write_str("\\\\")?,
            b'"' => f.write_str("\\\"")?,
            0 => f.write_str("\\0")?,
            b' '..=b'~' => write!(f, "{}", char::from(*byte))?,
            _ => write!(f, "\\x{byte:02X}")?,
        }
    }
    f.write_str("\"")
}

fn integer(
    f: &mut fmt::Formatter<'_>,
    negative: bool,
    magnitude: &super::Magnitude,
    radix: u8,
    digits: usize,
) -> fmt::Result {
    use alloc::string::ToString;
    if negative {
        f.write_str("-")?;
    }
    let prefix = match radix {
        2 => "0b",
        8 => "0o",
        16 => "0x",
        _ => "",
    };
    f.write_str(prefix)?;
    let text = if prefix.is_empty() {
        magnitude.to_string()
    } else {
        let base = super::Magnitude::from(u64::from(radix));
        let mut value = magnitude.clone();
        let mut chars = alloc::vec::Vec::new();
        loop {
            let (next, rem) = value.div_rem(&base).expect("nonzero radix");
            chars.push(char::from(
                b"0123456789abcdef"
                    [usize::try_from(rem.to_u64().unwrap_or(0)).unwrap_or(0)],
            ));
            value = next;
            if value.is_zero() {
                break;
            }
        }
        chars.into_iter().rev().collect()
    };
    for _ in text.len()..digits {
        f.write_str("0")?;
    }
    f.write_str(&text)
}
