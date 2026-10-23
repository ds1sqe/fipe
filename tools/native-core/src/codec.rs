//! The byte formats the backend shares with its runtime: the unit an
//! executable carries, and the frames of the host pipe.
//!
//! An integer is eight bytes little-endian; text and a list are their
//! length, then their contents.
//!
//! A function crosses only where the host shows it, in a main line or
//! the run's result, as the placeholder [`function`], which shows as
//! `<function>`; in a request or a reply it is [`Error::Opaque`].
//!
//! Both backends and the runtime use this crate, so each format has
//! one implementation.
use fipe::{
    check,
    decl::Declarations,
    embed::Request,
    lex::lex,
    mir::Program,
    parse::parse,
    token::{Magnitude, Span},
    value::{BigInt, Closure, Packed, Reason, Record, Value},
    vm::Fault,
};
use std::{fmt, rc::Rc};

/// Why bytes do not decode, or a value does not cross the pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes end inside an item.
    Short,
    /// A tag no item has.
    Tag(u64),
    /// Text that is not UTF-8, or an integer that is not decimal.
    Malformed,
    /// A closure or a witness the host would read, which only the
    /// program can.
    Opaque,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short => f.write_str("the frame ends early"),
            Self::Tag(tag) => write!(f, "no item has tag {tag}"),
            Self::Malformed => f.write_str("a malformed text or integer"),
            Self::Opaque => f.write_str(
                "a function or a witness cannot cross the host pipe",
            ),
        }
    }
}

impl std::error::Error for Error {}

/// The sources an executable carries and the fingerprint of the
/// program they compile to.
///
/// `sources` are `main` first, then the host's declarations,
/// `commands` before its modules; `files` the file modules `main` may
/// `use`, each by its stem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub sources: Vec<(String, String)>,
    pub files: Vec<(String, String)>,
    pub fingerprint: u64,
}

impl Unit {
    /// The unit of a main source under a host's declaration files,
    /// with the file modules it may `use`, and its program.
    ///
    /// # Errors
    ///
    /// The diagnostics of a source that does not check.
    pub fn new(
        main: &str,
        host: Vec<(String, String)>,
        files: Vec<(String, String)>,
    ) -> Result<(Self, Program), String> {
        let mut sources = vec![(String::from("main"), String::from(main))];
        sources.extend(host);
        let mut unit = Self {
            sources,
            files,
            fingerprint: 0,
        };
        let program = unit.compile()?;
        unit.fingerprint = fingerprint(&program);
        Ok((unit, program))
    }

    /// The source of a namespace: a host's, a file module's or the
    /// main's.
    #[must_use]
    pub fn source(&self, namespace: &str) -> Option<&str> {
        let mut all = self.sources.iter().chain(&self.files);
        let found = all.find(|(name, _)| name == namespace);
        found.map(|(_, text)| text.as_str())
    }

    /// The program of the sources, checked and lowered.
    ///
    /// # Errors
    ///
    /// The diagnostics of a source that does not check.
    pub fn compile(&self) -> Result<Program, String> {
        let mut declarations = Declarations::new();
        let mut main = "";
        for (name, text) in &self.sources {
            let declared = match name.as_str() {
                "main" => {
                    main = text;
                    Ok(())
                }
                "commands" => declarations.commands(text),
                _ => declarations.module(name, text),
            };
            declared.map_err(|errors| format!("{name}: {errors:?}"))?;
        }
        let module =
            parse(lex(main)).map_err(|errors| format!("main: {errors:?}"))?;
        let files = self
            .files
            .iter()
            .map(|(stem, text)| {
                let file = parse(lex(text))
                    .map_err(|errors| format!("{stem}: {errors:?}"))?;
                Ok((stem.clone(), file))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let unit = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .map_err(|errors| format!("main: {errors:?}"))?;
        Ok(fipe::compile::compile(&unit))
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        number(&mut out, self.fingerprint);
        for list in [&self.sources, &self.files] {
            number(&mut out, list.len() as u64);
            for (name, text) in list {
                bytes(&mut out, name.as_bytes());
                bytes(&mut out, text.as_bytes());
            }
        }
        out
    }

    /// # Errors
    ///
    /// Bytes that are not a unit.
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let mut input = Reader(input);
        let fingerprint = input.number()?;
        let mut list = || {
            (0..input.number()?)
                .map(|_| Ok((input.text()?, input.text()?)))
                .collect::<Result<_, Error>>()
        };
        let sources = list()?;
        let files = list()?;
        Ok(Self {
            sources,
            files,
            fingerprint,
        })
    }
}

/// The fingerprint of a program: FNV-1a over its bodies' `Debug`, so a
/// change of the compiler or of a source changes it.
#[must_use]
pub fn fingerprint(program: &Program) -> u64 {
    fnv(format!("{:?}", program.bodies).as_bytes())
}

/// FNV-1a, 64 bits.
#[must_use]
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The value a function in a main line or a result decodes to: a
/// closure of no body, which only shows, as `<function>`.
#[must_use]
pub fn function() -> Value {
    Value::Function(Rc::new(Closure {
        body: usize::MAX,
        captures: Vec::new(),
        meanings: Vec::new(),
        filled: Vec::new(),
        guards: Vec::new(),
    }))
}

/// A frame the program writes on fd 3.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    /// A host call, with the main lines kept so far; a reply follows
    /// on fd 4.
    Call {
        request: Request,
        lines: Vec<(usize, Value)>,
    },
    /// The run ended: its result, every main line kept, and the units
    /// it charged.
    Done {
        result: Result<Value, Fault>,
        lines: Vec<(usize, Value)>,
        charged: u64,
    },
}

impl Frame {
    /// The frame's payload, without its length.
    ///
    /// # Errors
    ///
    /// [`Error::Opaque`] for a witness in it, or a function in a
    /// request.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        match self {
            Self::Call { request, lines } => {
                let Request::Call {
                    declaration,
                    arguments,
                    environment,
                } = request
                else {
                    unreachable!("a program asks only for host calls")
                };
                number(&mut out, 0);
                number(&mut out, *declaration as u64);
                values(&mut out, arguments, false)?;
                keyed(&mut out, environment, false)?;
                keyed(&mut out, lines, true)?;
            }
            Self::Done {
                result,
                lines,
                charged,
            } => {
                number(&mut out, 1);
                match result {
                    Ok(value) => {
                        number(&mut out, 0);
                        value_into(&mut out, value, true)?;
                    }
                    Err(fault) => {
                        number(&mut out, 1);
                        fault_into(&mut out, fault);
                    }
                }
                keyed(&mut out, lines, true)?;
                number(&mut out, *charged);
            }
        }
        Ok(out)
    }

    /// # Errors
    ///
    /// A payload that is not a frame.
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let mut input = Reader(input);
        match input.number()? {
            0 => Ok(Self::Call {
                request: Request::Call {
                    declaration: input.index()?,
                    arguments: input.values(false)?,
                    environment: input.keyed(false)?,
                },
                lines: input.keyed(true)?,
            }),
            1 => {
                let result = match input.number()? {
                    0 => Ok(input.value(true)?),
                    1 => Err(input.fault()?),
                    tag => return Err(Error::Tag(tag)),
                };
                Ok(Self::Done {
                    result,
                    lines: input.keyed(true)?,
                    charged: input.number()?,
                })
            }
            tag => Err(Error::Tag(tag)),
        }
    }
}

/// A reply's payload: the value, or the failure's message.
///
/// # Errors
///
/// [`Error::Opaque`] for a function or a witness in the value.
pub fn encode_reply(reply: &Result<Value, String>) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    match reply {
        Ok(value) => {
            number(&mut out, 0);
            value_into(&mut out, value, false)?;
        }
        Err(message) => {
            number(&mut out, 1);
            bytes(&mut out, message.as_bytes());
        }
    }
    Ok(out)
}

/// # Errors
///
/// A payload that is not a reply.
pub fn decode_reply(input: &[u8]) -> Result<Result<Value, String>, Error> {
    let mut input = Reader(input);
    match input.number()? {
        0 => Ok(Ok(input.value(false)?)),
        1 => Ok(Err(input.text()?)),
        tag => Err(Error::Tag(tag)),
    }
}

fn number(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_le_bytes());
}

fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    number(out, value.len() as u64);
    out.extend(value);
}

fn values(
    out: &mut Vec<u8>,
    values: &[Value],
    shown: bool,
) -> Result<(), Error> {
    number(out, values.len() as u64);
    values
        .iter()
        .try_for_each(|value| value_into(out, value, shown))
}

/// A list of values each with an index: the `with` keys, the lines'
/// nodes.
fn keyed(
    out: &mut Vec<u8>,
    items: &[(usize, Value)],
    shown: bool,
) -> Result<(), Error> {
    number(out, items.len() as u64);
    items.iter().try_for_each(|(index, value)| {
        number(out, *index as u64);
        value_into(out, value, shown)
    })
}

/// `shown`: the host only shows the value, so a function crosses as
/// the placeholder.
fn value_into(
    out: &mut Vec<u8>,
    value: &Value,
    shown: bool,
) -> Result<(), Error> {
    match value {
        Value::Unit => number(out, 0),
        Value::Int(int) => {
            number(out, 1);
            out.extend(int.to_le_bytes());
        }
        Value::Big(big) => {
            number(out, 2);
            number(out, u64::from(big.negative));
            bytes(out, big.magnitude.to_string().as_bytes());
        }
        Value::F32(float) => {
            number(out, 3);
            number(out, u64::from(float.to_bits()));
        }
        Value::F64(float) => {
            number(out, 4);
            number(out, float.to_bits());
        }
        Value::Text(text) => {
            number(out, 5);
            bytes(out, text.as_bytes());
        }
        Value::Seq(parts) => {
            number(out, 6);
            values(out, parts, shown)?;
        }
        Value::Tuple(parts) => {
            number(out, 7);
            values(out, parts, shown)?;
        }
        Value::Record(record) => {
            number(out, 8);
            number(out, record.meaning as u64);
            number(out, record.constructor as u64);
            values(out, &record.fields, shown)?;
        }
        Value::Packed(packed) => {
            number(out, 9);
            number(out, packed.meaning as u64);
            value_into(out, &packed.value, shown)?;
        }
        Value::Function(_) if shown => number(out, 10),
        Value::Function(_) | Value::Meaning(_) => return Err(Error::Opaque),
    }
    Ok(())
}

fn fault_into(out: &mut Vec<u8>, fault: &Fault) {
    number(out, fault.span.start as u64);
    number(out, fault.span.end as u64);
    number(out, fault.namespace as u64);
    let text = |out: &mut Vec<u8>, tag, message: &str| {
        number(out, tag);
        bytes(out, message.as_bytes());
    };
    match &fault.reason {
        Reason::Budget => number(out, 0),
        Reason::DivideByZero => number(out, 1),
        Reason::NotFinite => number(out, 2),
        Reason::NotExact { value, target } => {
            text(out, 3, value);
            bytes(out, target.as_bytes());
        }
        Reason::Failed { entry, value } => {
            text(out, 4, value);
            number(out, *entry as u64);
        }
        Reason::Cast => number(out, 5),
        Reason::Unreachable => number(out, 6),
        Reason::Host(message) => text(out, 7, message),
        Reason::Message(message) => text(out, 8, message),
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    const fn take(&mut self, count: usize) -> Result<&[u8], Error> {
        if self.0.len() < count {
            return Err(Error::Short);
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn number(&mut self) -> Result<u64, Error> {
        let taken = self.take(8)?.try_into().expect("eight bytes");
        Ok(u64::from_le_bytes(taken))
    }

    fn index(&mut self) -> Result<usize, Error> {
        usize::try_from(self.number()?).map_err(|_| Error::Malformed)
    }

    fn text(&mut self) -> Result<String, Error> {
        let count = self.index()?;
        let taken = self.take(count)?.to_vec();
        String::from_utf8(taken).map_err(|_| Error::Malformed)
    }

    fn values(&mut self, shown: bool) -> Result<Vec<Value>, Error> {
        (0..self.number()?).map(|_| self.value(shown)).collect()
    }

    fn keyed(&mut self, shown: bool) -> Result<Vec<(usize, Value)>, Error> {
        (0..self.number()?)
            .map(|_| Ok((self.index()?, self.value(shown)?)))
            .collect()
    }

    fn value(&mut self, shown: bool) -> Result<Value, Error> {
        Ok(match self.number()? {
            0 => Value::Unit,
            1 => Value::Int(self.number()?.cast_signed()),
            2 => {
                let negative = self.number()? != 0;
                let magnitude = Magnitude::parse(&self.text()?, 10)
                    .ok_or(Error::Malformed)?;
                Value::Big(Rc::new(BigInt {
                    negative,
                    magnitude,
                }))
            }
            3 => {
                let bits = u32::try_from(self.number()?)
                    .map_err(|_| Error::Malformed)?;
                Value::F32(f32::from_bits(bits))
            }
            4 => Value::F64(f64::from_bits(self.number()?)),
            5 => Value::text(&self.text()?),
            6 => Value::seq(self.values(shown)?),
            7 => Value::Tuple(Rc::new(self.values(shown)?)),
            8 => Value::Record(Rc::new(Record {
                meaning: self.index()?,
                constructor: self.index()?,
                fields: self.values(shown)?,
            })),
            9 => Value::Packed(Rc::new(Packed {
                meaning: self.index()?,
                value: self.value(shown)?,
            })),
            10 if shown => function(),
            tag => return Err(Error::Tag(tag)),
        })
    }

    fn fault(&mut self) -> Result<Fault, Error> {
        let span = Span {
            start: self.index()?,
            end: self.index()?,
        };
        let namespace = self.index()?;
        let reason = match self.number()? {
            0 => Reason::Budget,
            1 => Reason::DivideByZero,
            2 => Reason::NotFinite,
            3 => Reason::NotExact {
                value: self.text()?,
                target: self.text()?,
            },
            4 => {
                let value = self.text()?;
                Reason::Failed {
                    entry: self.index()?,
                    value,
                }
            }
            5 => Reason::Cast,
            6 => Reason::Unreachable,
            7 => Reason::Host(self.text()?),
            8 => Reason::Message(self.text()?),
            tag => return Err(Error::Tag(tag)),
        };
        Ok(Fault {
            span,
            namespace,
            reason,
        })
    }
}
