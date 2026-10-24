//! The replay host: a run's host calls answered from a trace fixture.
//!
//! The fixtures are the crate's conformance host's
//! (`tests/fixtures/traces/*.txt`): the Nth call gets the Nth recorded
//! reply. A call that is not the one recorded is answered with a
//! failure naming both, so a diverging run Faults at that call instead
//! of reading the wrong reply.
//!
//! A trace line is `call ns.label (arguments) [with key value]… ->
//! reply`, the reply a value in the language's spelling or `failure
//! "message"`; other lines are ignored. Values are read back with the
//! crate's lexer against the run's tables: constructors by their label.
use fipe::{
    ir::Tables,
    lex::lex,
    token::{Kind, Symbol, Token},
    value::{Record, Value},
};
use std::rc::Rc;

/// One recorded call: who was called, and the reply's spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// `ns.label`.
    pub callee: String,
    /// The whole line, as the trace wrote it.
    pub line: String,
    /// The reply after ` -> `.
    pub reply: String,
}

/// The recorded calls of a trace, in order.
#[derive(Clone, Debug, Default)]
pub struct Replay {
    calls: Vec<Recorded>,
    next: usize,
}

impl Replay {
    /// The calls of a trace's text.
    ///
    /// # Errors
    /// A `call` line without ` -> `.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut calls = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let Some(rest) = line.strip_prefix("call ") else {
                continue;
            };
            let (_, reply) = rest.split_once(" -> ").ok_or_else(|| {
                format!("trace line {}: no reply", number + 1)
            })?;
            let callee = rest.split(' ').next().unwrap_or_default();
            calls.push(Recorded {
                callee: callee.to_string(),
                line: line.to_string(),
                reply: reply.to_string(),
            });
        }
        Ok(Self { calls, next: 0 })
    }

    /// The recorded calls.
    #[must_use]
    pub fn calls(&self) -> &[Recorded] {
        &self.calls
    }

    /// The reply to the next call, which names `callee`: the recorded
    /// value.
    ///
    /// # Errors
    /// A failure message when the trace is exhausted, records another
    /// callee or a failure, or its reply cannot be read.
    pub fn answer(
        &mut self,
        callee: &str,
        tables: &Tables,
    ) -> Result<Value, String> {
        let Some(recorded) = self.calls.get(self.next) else {
            return Err(format!("replay: no recorded call for {callee}"));
        };
        self.next += 1;
        if recorded.callee != callee {
            return Err(format!(
                "replay: {callee} called where the trace recorded {}",
                recorded.callee
            ));
        }
        if let Some(message) = recorded.reply.strip_prefix("failure ") {
            return Err(
                read_text(message).unwrap_or_else(|| message.to_string())
            );
        }
        read(&recorded.reply, tables)
    }
}

fn read_text(spelling: &str) -> Option<String> {
    match lex(spelling).tokens.first()?.kind.clone() {
        Kind::Str { value, .. } => Some(value),
        _ => None,
    }
}

/// A value in the language's spelling, as `show` writes it, read
/// against `tables`.
///
/// # Errors
/// A spelling the reader does not know, or a constructor no meaning
/// has.
pub fn read(spelling: &str, tables: &Tables) -> Result<Value, String> {
    let lexed = lex(spelling);
    if let Some(error) = lexed.errors.first() {
        return Err(format!("cannot read {spelling:?}: {:?}", error.reason));
    }
    let tokens: Vec<Token> = lexed
        .tokens
        .into_iter()
        .filter(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
        .collect();
    let mut reader = Reader {
        tokens: &tokens,
        at: 0,
        tables,
        spelling,
    };
    let value = reader.value()?;
    if reader.at != tokens.len() {
        return Err(reader.fail());
    }
    Ok(value)
}

struct Reader<'a> {
    tokens: &'a [Token],
    at: usize,
    tables: &'a Tables,
    spelling: &'a str,
}

impl Reader<'_> {
    fn fail(&self) -> String {
        format!("cannot read {:?}", self.spelling)
    }

    fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.at).map(|token| &token.kind)
    }

    fn symbol(&self, symbol: Symbol) -> bool {
        matches!(self.peek(), Some(Kind::Symbol(found)) if *found == symbol)
    }

    fn expect(&mut self, symbol: Symbol) -> Result<(), String> {
        if self.symbol(symbol) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.fail())
        }
    }

    /// A value: a constructor with its arguments, or an atom.
    fn value(&mut self) -> Result<Value, String> {
        if let Some(Kind::Name(segments)) = self.peek() {
            let label = segments.join(".");
            self.at += 1;
            let mut fields = Vec::new();
            while self.atom_starts() {
                fields.push(self.atom()?);
            }
            return self.construct(&label, fields);
        }
        self.atom()
    }

    fn atom_starts(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Kind::Name(_)
                    | Kind::Int { .. }
                    | Kind::Float(_)
                    | Kind::Str { .. }
                    | Kind::Symbol(Symbol::LParen | Symbol::LBracket)
            )
        )
    }

    fn atom(&mut self) -> Result<Value, String> {
        let kind = self.peek().cloned().ok_or_else(|| self.fail())?;
        self.at += 1;
        match kind {
            Kind::Name(segments) => {
                self.construct(&segments.join("."), Vec::new())
            }
            Kind::Int {
                negative,
                magnitude,
                ..
            } => Ok(Value::integer(negative, magnitude)),
            Kind::Float(text) => text
                .replace('_', "")
                .parse::<f64>()
                .map(Value::F64)
                .map_err(|_| self.fail()),
            Kind::Str { value, .. } => Ok(Value::text(&value)),
            Kind::Symbol(Symbol::LParen) => {
                if self.symbol(Symbol::RParen) {
                    self.at += 1;
                    return Ok(Value::Unit);
                }
                let mut parts = vec![self.value()?];
                while self.symbol(Symbol::Comma) {
                    self.at += 1;
                    parts.push(self.value()?);
                }
                self.expect(Symbol::RParen)?;
                Ok(if parts.len() == 1 {
                    parts.remove(0)
                } else {
                    Value::Tuple(Rc::new(parts))
                })
            }
            Kind::Symbol(Symbol::LBracket) => {
                let mut items = Vec::new();
                if !self.symbol(Symbol::RBracket) {
                    items.push(self.value()?);
                    while self.symbol(Symbol::Comma) {
                        self.at += 1;
                        items.push(self.value()?);
                    }
                }
                self.expect(Symbol::RBracket)?;
                Ok(Value::Seq(Rc::new(items)))
            }
            _ => Err(self.fail()),
        }
    }

    /// The meaning and constructor whose label is `label` and arity is
    /// the fields', the first in the tables.
    fn construct(
        &self,
        label: &str,
        fields: Vec<Value>,
    ) -> Result<Value, String> {
        for (meaning, declared) in self.tables.meanings.iter().enumerate() {
            if let Some(constructor) = declared
                .constructors
                .iter()
                .position(|constructor| constructor.label == label)
            {
                if declared.constructors[constructor].fields.len()
                    != fields.len()
                {
                    continue;
                }
                return Ok(Value::Record(Rc::new(Record {
                    meaning,
                    constructor,
                    fields,
                })));
            }
        }
        // An opaque type has no constructors; `show` writes its label.
        if let Some(meaning) = self.tables.meanings.iter().position(|m| {
            !m.resource && m.label == label && m.constructors.is_empty()
        }) {
            return Ok(Value::Record(Rc::new(Record {
                meaning,
                constructor: 0,
                fields,
            })));
        }
        Err(format!(
            "replay: no constructor {label} of {} fields",
            fields.len()
        ))
    }
}
