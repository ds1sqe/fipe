//! Byte offsets, which the crate speaks, and LSP positions, whose
//! character unit the client and server negotiate.

use lsp_types::{Position, Range};

/// The unit of a position's `character`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

/// The position of byte `offset` in `text`; an offset inside a scalar
/// or past the end is clamped to the scalar or the end.
#[must_use]
pub fn position(text: &str, offset: usize, encoding: Encoding) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let line = before.matches('\n').count();
    let column = &before[line_start..];
    let character = match encoding {
        Encoding::Utf8 => column.len(),
        Encoding::Utf16 => column.encode_utf16().count(),
    };
    Position::new(
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(character).unwrap_or(u32::MAX),
    )
}

/// The byte offset of `position` in `text`, clamped to its line's end.
#[must_use]
pub fn offset(text: &str, position: Position, encoding: Encoding) -> usize {
    let mut start = 0;
    for _ in 0..position.line {
        match text[start..].find('\n') {
            Some(at) => start += at + 1,
            None => return text.len(),
        }
    }
    let line = text[start..].split('\n').next().unwrap_or_default();
    let wanted = position.character as usize;
    let mut units = 0;
    for (at, scalar) in line.char_indices() {
        if units >= wanted {
            return start + at;
        }
        units += match encoding {
            Encoding::Utf8 => scalar.len_utf8(),
            Encoding::Utf16 => scalar.len_utf16(),
        };
    }
    start + line.len()
}

/// The range of the byte span `start..end` in `text`.
#[must_use]
pub fn range(
    text: &str,
    (start, end): (usize, usize),
    encoding: Encoding,
) -> Range {
    Range::new(
        position(text, start, encoding),
        position(text, end, encoding),
    )
}
