use super::{fmt, Kind};
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
            } => {
                if *negative {
                    f.write_str("-")?;
                }
                magnitude.fmt(f)
            }
            Self::Str { value, raw: true } => write!(f, "'{value}'"),
            Self::Str { value, raw: false } => quoted(f, value.as_bytes()),
            Self::Bytes(bytes) => {
                f.write_str("b")?;
                quoted(f, bytes)
            }
            Self::Rd(id) => write!(f, "rd:{id}"),
            Self::Newline => f.write_str("\n"),
            Self::Eof => f.write_str("<eof>"),
        }
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
