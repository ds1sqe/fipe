use super::{invisible, spacing, word, Reason, Scanner};
use crate::token::{Affixed, Kind, Written};
use alloc::{boxed::Box, string::String, vec::Vec};

impl Scanner<'_> {
    /// A quote after its `prefix`, which is empty when not written, then
    /// a suffix that touches it. With either the bytes need not be UTF-8.
    pub(super) fn quoted(&mut self, prefix: String) -> Result<Kind, Reason> {
        let raw = self.peek() == Some('\'');
        let value = self.quote();
        // A failed quote still owns its suffix, so the span covers it; an
        // unterminated one stopped at LF or EOF, where no word follows.
        let suffix = self.suffix();
        let value = value?;
        if prefix.is_empty() && suffix.is_empty() {
            return String::from_utf8(value)
                .map(|value| Kind::Str { value, raw })
                .map_err(|_| Reason::InvalidUtf8);
        }
        Ok(Kind::Affixed(Box::new(Affixed {
            prefix,
            form: Written::Text { value, raw },
            suffix,
        })))
    }

    /// A name touching the closing quote, when the whole word is one.
    fn suffix(&mut self) -> String {
        let start = self.offset;
        let end = self.word_end(start);
        let word = &self.text[start..end];
        if !word::suffix(word) {
            return String::new();
        }
        while self.offset < end {
            self.bump();
        }
        String::from(word)
    }

    fn quote(&mut self) -> Result<Vec<u8>, Reason> {
        let quote = self.bump().expect("quoted starts at a quote");
        let raw = quote == '\'';
        let mut value = Vec::new();
        let mut error = None;
        loop {
            match self.peek() {
                None => {
                    return Err(error.unwrap_or(Reason::UnterminatedQuote));
                }
                Some('\n') if !raw => {
                    return Err(error.unwrap_or(Reason::UnterminatedQuote));
                }
                Some(ch) if ch == quote => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    if let Err(reason) = self.quoted_char(raw, &mut value) {
                        error.get_or_insert(reason);
                    }
                }
            }
        }
        error.map_or(Ok(value), Err)
    }

    fn quoted_char(
        &mut self,
        raw: bool,
        out: &mut Vec<u8>,
    ) -> Result<(), Reason> {
        let ch = self.bump().expect("quoted_char is called before EOF");
        if ch == '\\' && !raw {
            if self.peek() == Some('\n') {
                self.bump();
                while self.peek().is_some_and(spacing) {
                    self.bump();
                }
            } else {
                out.push(self.escape()?);
            }
        } else if invisible(ch)
            || (ch.is_control() && ch != '\t' && !(raw && ch == '\n'))
        {
            return Err(Reason::Stray(ch));
        } else {
            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
        }
        Ok(())
    }

    fn escape(&mut self) -> Result<u8, Reason> {
        let ch = self
            .peek()
            .filter(|&ch| ch != '\n')
            .ok_or(Reason::BadEscape)?;
        self.bump();
        match ch {
            'n' => Ok(b'\n'),
            't' => Ok(b'\t'),
            'r' => Ok(b'\r'),
            '\\' => Ok(b'\\'),
            '"' => Ok(b'"'),
            '\'' => Ok(b'\''),
            '0' => Ok(0),
            'x' => Ok(self.hex_digit()? * 16 + self.hex_digit()?),
            _ => Err(Reason::BadEscape),
        }
    }

    fn hex_digit(&mut self) -> Result<u8, Reason> {
        // Leave a closing quote or LF for recovery, even after an incomplete
        // escape; it must not consume a token on the next physical line.
        let ch = self.peek().ok_or(Reason::BadEscape)?;
        let value = match ch {
            '0'..='9' => ch as u8 - b'0',
            'a'..='f' => ch as u8 - b'a' + 10,
            'A'..='F' => ch as u8 - b'A' + 10,
            _ => return Err(Reason::BadEscape),
        };
        self.bump();
        Ok(value)
    }
}
