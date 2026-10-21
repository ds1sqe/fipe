//! The line editor: plain state over key events, with no terminal.
//!
//! A driver decodes input bytes into [`Key`]s with [`Keys`], passes each
//! to [`Editor::key`] and acts on the [`Event`]: it redraws
//! [`Editor::view`] after [`Event::Draw`] and feeds a submitted line,
//! with its LF, to the [`Repl`](crate::Repl). One physical line is
//! edited at a time; the REPL's own continuation decides whether the
//! next line continues the entry, and its prompt says so.
//!
//! Tab yields [`Event::Complete`] with the text before the cursor; the
//! driver asks [`Repl::complete`](crate::Repl::complete) and passes the
//! answer to [`Editor::complete`].

use crate::highlight::{classes, Class};
use alloc::{string::String, vec::Vec};
use core::ops::Range;

/// One key press, decoded from terminal input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    /// Ctrl-K: delete to the end of the line.
    KillEnd,
    /// Ctrl-U: delete to the start of the line.
    KillStart,
    /// Ctrl-W: delete the word before the cursor.
    KillWord,
    Up,
    Down,
    Tab,
    /// Ctrl-C.
    Interrupt,
    /// Ctrl-D.
    Eof,
}

/// What the driver does after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The line or cursor changed: redraw it.
    Draw,
    /// Enter: this line, without its LF, is to run; the editor is empty.
    Line(String),
    /// Ctrl-C: the line is cleared and the entry is to be dropped.
    Interrupt,
    /// Ctrl-D on an empty line: input ends.
    End,
    /// Tab: complete the word that ends this text, the line before the
    /// cursor.
    Complete(String),
    /// A second Tab with several candidates: list them under the line,
    /// then redraw it.
    List(Vec<String>),
    /// Nothing changed.
    None,
}

/// What to draw: the prompt, the line, the cursor's column counted
/// from the prompt's start, and the line's token classes as char ranges
/// of `text`, in order and apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View<'a> {
    pub prompt: &'a str,
    pub text: String,
    pub cursor: usize,
    pub classes: Vec<(Range<usize>, Class)>,
}

impl<'a> View<'a> {
    #[must_use]
    pub fn new(prompt: &'a str, text: String, cursor: usize) -> Self {
        let classes = classes(&text);
        Self {
            prompt,
            text,
            cursor,
            classes,
        }
    }
}

/// What Tab offers: the text before the cursor that each candidate
/// replaces, and the candidates in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub partial: String,
    pub candidates: Vec<String>,
}

/// A line with a cursor, and the lines entered before it.
#[derive(Default)]
pub struct Editor {
    prompt: &'static str,
    line: Vec<char>,
    /// The cursor as an index into `line`, a char boundary by type.
    cursor: usize,
    history: Vec<String>,
    /// The history entry shown, and the line being typed before Up.
    recall: Option<(usize, Vec<char>)>,
    /// The last key was a Tab that left several candidates.
    tabbed: bool,
}

impl Editor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The prompt drawn before the line, as the REPL last wrote it.
    pub const fn prompt(&mut self, prompt: &'static str) {
        self.prompt = prompt;
    }

    #[must_use]
    pub fn view(&self) -> View<'_> {
        // ponytail: one column per char; wide and combining chars
        // misplace the cursor until a width table is wanted.
        View::new(
            self.prompt,
            self.line.iter().collect(),
            self.prompt.chars().count() + self.cursor,
        )
    }

    /// Apply Tab's completion: one candidate replaces the partial word;
    /// several insert their common prefix, and a second Tab in a row
    /// lists them.
    pub fn complete(&mut self, completion: Completion) -> Event {
        let Completion {
            partial,
            candidates,
        } = completion;
        let Some(first) = candidates.first() else {
            return Event::None;
        };
        if candidates.len() > 1 {
            if self.tabbed {
                return Event::List(candidates);
            }
            self.tabbed = true;
        }
        let common = candidates[1..]
            .iter()
            .fold(first.as_str(), |common, next| shared(common, next));
        let length = partial.chars().count();
        let Some(start) = self.cursor.checked_sub(length) else {
            return Event::None;
        };
        if common == partial
            || !common.starts_with(partial.as_str())
            || !self.line[start..self.cursor]
                .iter()
                .copied()
                .eq(partial.chars())
        {
            return Event::None;
        }
        self.line.splice(start..self.cursor, common.chars());
        self.cursor = start + common.chars().count();
        Event::Draw
    }

    pub fn key(&mut self, key: Key) -> Event {
        if key != Key::Tab {
            self.tabbed = false;
        }
        let before = (self.line.len(), self.cursor);
        match key {
            Key::Char(c) => {
                self.line.insert(self.cursor, c);
                self.cursor += 1;
            }
            Key::Enter => return Event::Line(self.submit()),
            Key::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.line.remove(self.cursor);
            }
            Key::Delete if self.cursor < self.line.len() => {
                self.line.remove(self.cursor);
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.line.len()),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.line.len(),
            Key::KillEnd => self.line.truncate(self.cursor),
            Key::KillStart => {
                self.line.drain(..self.cursor);
                self.cursor = 0;
            }
            Key::KillWord => {
                let mut start = self.cursor;
                while start > 0 && self.line[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !self.line[start - 1].is_whitespace() {
                    start -= 1;
                }
                self.line.drain(start..self.cursor);
                self.cursor = start;
            }
            Key::Tab => {
                let before = self.line[..self.cursor].iter().collect();
                return Event::Complete(before);
            }
            Key::Up => return self.up(),
            Key::Down => return self.down(),
            Key::Interrupt => {
                self.clear();
                return Event::Interrupt;
            }
            Key::Eof if self.line.is_empty() => return Event::End,
            Key::Eof => return self.key(Key::Delete),
            Key::Backspace | Key::Delete => return Event::None,
        }
        // A kill that removed nothing still redraws nothing new.
        if before == (self.line.len(), self.cursor)
            && !matches!(key, Key::Char(_))
        {
            return Event::None;
        }
        Event::Draw
    }

    fn submit(&mut self) -> String {
        let line: String = self.line.iter().collect();
        if !line.trim().is_empty()
            && self.history.last().is_none_or(|last| *last != line)
        {
            self.history.push(line.clone());
        }
        self.clear();
        line
    }

    fn clear(&mut self) {
        self.line.clear();
        self.cursor = 0;
        self.recall = None;
    }

    fn show(&mut self, line: Vec<char>) -> Event {
        self.line = line;
        self.cursor = self.line.len();
        Event::Draw
    }

    fn up(&mut self) -> Event {
        let index = match &self.recall {
            Some((0, _)) => return Event::None,
            Some((index, _)) => index - 1,
            None if self.history.is_empty() => return Event::None,
            None => self.history.len() - 1,
        };
        let draft = match self.recall.take() {
            Some((_, draft)) => draft,
            None => core::mem::take(&mut self.line),
        };
        self.recall = Some((index, draft));
        self.show(self.history[index].chars().collect())
    }

    fn down(&mut self) -> Event {
        match self.recall.take() {
            None => Event::None,
            Some((index, draft)) if index + 1 == self.history.len() => {
                self.show(draft)
            }
            Some((index, draft)) => {
                self.recall = Some((index + 1, draft));
                self.show(self.history[index + 1].chars().collect())
            }
        }
    }
}

/// The longest common prefix of `a` and `b`, at a char boundary.
fn shared<'a>(a: &'a str, b: &str) -> &'a str {
    let length = a
        .char_indices()
        .zip(b.chars())
        .take_while(|((_, x), y)| x == y)
        .last()
        .map_or(0, |((at, x), _)| at + x.len_utf8());
    &a[..length]
}

/// Terminal input bytes to keys: UTF-8 text, control bytes and the
/// escape sequences of arrows, Home, End and Delete. A partial sequence
/// or char waits for the next bytes; unknown sequences are dropped.
#[derive(Default)]
pub struct Keys {
    pending: Vec<u8>,
}

impl Keys {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(bytes);
        let mut keys = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            let Some((key, length)) = decode(&self.pending[at..]) else {
                break;
            };
            keys.extend(key);
            at += length;
        }
        self.pending.drain(..at);
        keys
    }
}

/// The key at the start of `bytes` and the bytes it takes; `None` when
/// more bytes are needed.
fn decode(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    let key = match bytes[0] {
        0x1b => return escape(bytes),
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7f | 0x08 => Key::Backspace,
        0x01 => Key::Home,
        0x02 => Key::Left,
        0x03 => Key::Interrupt,
        0x04 => Key::Eof,
        0x05 => Key::End,
        0x06 => Key::Right,
        0x0b => Key::KillEnd,
        0x0e => Key::Down,
        0x10 => Key::Up,
        0x15 => Key::KillStart,
        0x17 => Key::KillWord,
        byte if byte < 0x20 => return Some((None, 1)),
        byte => {
            let length = match byte {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => 1,
            };
            let text = bytes.get(..length)?;
            return Some((
                core::str::from_utf8(text)
                    .ok()
                    .and_then(|text| text.chars().next())
                    .map(Key::Char),
                length,
            ));
        }
    };
    Some((Some(key), 1))
}

/// `ESC [ params final` or `ESC O final`; any other byte after ESC
/// drops the ESC alone.
fn escape(bytes: &[u8]) -> Option<(Option<Key>, usize)> {
    match bytes.get(1)? {
        b'[' | b'O' => {}
        _ => return Some((None, 1)),
    }
    let end = 2 + bytes[2..].iter().position(|b| (0x40..=0x7e).contains(b))?;
    let key = match &bytes[2..=end] {
        b"A" => Key::Up,
        b"B" => Key::Down,
        b"C" => Key::Right,
        b"D" => Key::Left,
        b"H" | b"1~" | b"7~" => Key::Home,
        b"F" | b"4~" | b"8~" => Key::End,
        b"3~" => Key::Delete,
        _ => return Some((None, end + 1)),
    };
    Some((Some(key), end + 1))
}
