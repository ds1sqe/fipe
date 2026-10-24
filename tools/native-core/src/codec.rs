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
    ir,
    lex::lex,
    mir::Program,
    parse::parse,
    token::{Magnitude, Span},
    value::{BigInt, Closure, Packed, Reason, Record, Value},
    vm::Fault,
};
use std::{collections::BTreeMap, fmt, rc::Rc};

/// Why bytes do not decode, or a value does not cross the pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes end inside an item.
    Short,
    /// A tag no item has.
    Tag(u64),
    /// Malformed text, numbers, graph references or excessive Value depth.
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
            Self::Malformed => f.write_str("a malformed codec payload"),
            Self::Opaque => f.write_str(
                "a function or a witness cannot cross the host pipe",
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Reject a program whose complete Host declaration set contains resources.
///
/// # Errors
/// Resource declarations are not supported by native execution, even when unused.
pub fn ensure_resource_free(program: &Program) -> Result<(), String> {
    if program.tables.borrow().has_resources {
        return Err(String::from(
            "native execution does not support resource declarations",
        ));
    }
    Ok(())
}

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
    /// The diagnostics of a source that does not check, or unsupported resource declarations.
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
    /// The diagnostics of a source that does not check, or unsupported resource declarations.
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
        let program = fipe::compile::compile(&unit);
        ensure_resource_free(&program)?;
        Ok(program)
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
    fnv(format!(
        "{:?}{:?}{:?}",
        program.bodies, program.receiver_adapters, program.receiver_contexts
    )
    .as_bytes())
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

/// Borrowed traversal: shared immutable allocations are expanded once.
/// The private visit count makes the unique-edge work bound testable.
fn carrier_scan<'a>(
    roots: impl IntoIterator<Item = &'a Value>,
) -> (bool, usize) {
    let mut pending: Vec<_> = roots.into_iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut visited = 0;
    while let Some(value) = pending.pop() {
        visited += 1;
        if matches!(value, Value::Resource(_)) {
            return (true, visited);
        }
        let identity = match value {
            Value::Seq(parts) => {
                Some((0, Rc::as_ptr(parts).cast::<()>() as usize))
            }
            Value::Tuple(parts) => {
                Some((1, Rc::as_ptr(parts).cast::<()>() as usize))
            }
            Value::Record(record) => {
                Some((2, Rc::as_ptr(record).cast::<()>() as usize))
            }
            Value::Packed(packed) => {
                Some((3, Rc::as_ptr(packed).cast::<()>() as usize))
            }
            Value::Function(closure) => {
                Some((4, Rc::as_ptr(closure).cast::<()>() as usize))
            }
            _ => None,
        };
        if identity.is_some_and(|identity| !seen.insert(identity)) {
            continue;
        }
        match value {
            Value::Seq(parts) | Value::Tuple(parts) => {
                pending.extend(parts.iter());
            }
            Value::Record(record) => pending.extend(&record.fields),
            Value::Packed(packed) => pending.push(packed.value()),
            Value::Function(closure) => {
                pending.extend(closure.captures.iter().chain(&closure.filled));
            }
            _ => {}
        }
    }
    (false, visited)
}

fn frame_contains_resource(frame: &Frame) -> bool {
    match frame {
        Frame::Call { request, lines } => {
            let Request::Call {
                arguments,
                environment,
                ..
            } = request
            else {
                unreachable!("a program asks only for host calls")
            };
            carrier_scan(
                arguments
                    .iter()
                    .chain(environment.iter().map(|(_, value)| value))
                    .chain(lines.iter().map(|(_, value)| value)),
            )
            .0
        }
        Frame::Done { result, lines, .. } => {
            carrier_scan(
                result.iter().chain(lines.iter().map(|(_, value)| value)),
            )
            .0
        }
    }
}

impl Frame {
    /// The frame's payload, without its length.
    ///
    /// # Errors
    ///
    /// [`Error::Opaque`] for a witness in it, or a function in a
    /// request.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if frame_contains_resource(self) {
            return Err(Error::Opaque);
        }
        let mut out = Vec::new();
        let mut writer = ValueWriter::new(&mut out);
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
                number(writer.out, 0);
                number(writer.out, *declaration as u64);
                writer.values(arguments, false)?;
                writer.keyed(environment, false)?;
                writer.keyed(lines, true)?;
            }
            Self::Done {
                result,
                lines,
                charged,
            } => {
                number(writer.out, 1);
                match result {
                    Ok(value) => {
                        number(writer.out, 0);
                        writer.value(value, true)?;
                    }
                    Err(fault) => {
                        number(writer.out, 1);
                        fault_into(writer.out, fault);
                    }
                }
                writer.keyed(lines, true)?;
                number(writer.out, *charged);
            }
        }
        Ok(out)
    }

    /// # Errors
    ///
    /// A payload that is not a frame.
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let mut raw = Reader(input);
        let mut input = ValueReader::new(&mut raw);
        match input.input.number()? {
            0 => Ok(Self::Call {
                request: Request::Call {
                    declaration: input.input.index()?,
                    arguments: input.values(false)?,
                    environment: input.keyed(false)?,
                },
                lines: input.keyed(true)?,
            }),
            1 => {
                let result = match input.input.number()? {
                    0 => Ok(input.value(true)?),
                    1 => Err(input.input.fault()?),
                    tag => return Err(Error::Tag(tag)),
                };
                Ok(Self::Done {
                    result,
                    lines: input.keyed(true)?,
                    charged: input.input.number()?,
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
    if carrier_scan(reply.iter()).0 {
        return Err(Error::Opaque);
    }
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
    let mut raw = Reader(input);
    let mut input = ValueReader::new(&mut raw);
    match input.input.number()? {
        0 => Ok(Ok(input.value(false)?)),
        1 => Ok(Err(input.input.text()?)),
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

// Value containers are bounded independently of their descriptive Type roots.
const MAX_VALUE_DEPTH: usize = 256;
const VALUE_REFERENCE: u64 = 12;
type Allocation = (u8, usize);

#[derive(Clone, Copy)]
struct ValueProperties {
    height: usize,
    opaque_safe: bool,
}
impl ValueProperties {
    fn allowed(self, depth: usize, shown: bool) -> Result<(), Error> {
        if depth
            .checked_add(self.height)
            .is_none_or(|n| n > MAX_VALUE_DEPTH)
        {
            return Err(Error::Malformed);
        }
        if !shown && !self.opaque_safe {
            return Err(Error::Opaque);
        }
        Ok(())
    }
}
fn container_properties(
    parts: impl IntoIterator<Item = ValueProperties>,
) -> Result<ValueProperties, Error> {
    let mut height = 0;
    let mut opaque_safe = true;
    for part in parts {
        height = height.max(part.height);
        opaque_safe &= part.opaque_safe;
    }
    height += 1;
    if height > MAX_VALUE_DEPTH {
        return Err(Error::Malformed);
    }
    Ok(ValueProperties {
        height,
        opaque_safe,
    })
}
fn value_allocation(value: &Value) -> Option<Allocation> {
    match value {
        Value::Seq(p) => Some((0, Rc::as_ptr(p) as usize)),
        Value::Tuple(p) => Some((1, Rc::as_ptr(p) as usize)),
        Value::Record(p) => Some((2, Rc::as_ptr(p) as usize)),
        Value::Packed(p) => Some((3, Rc::as_ptr(p) as usize)),
        _ => None,
    }
}
enum WrittenValue {
    Visiting,
    Complete {
        index: usize,
        properties: ValueProperties,
    },
}
struct ValueWriter<'a> {
    out: &'a mut Vec<u8>,
    memo: BTreeMap<Allocation, WrittenValue>,
    completed: usize,
}
impl<'a> ValueWriter<'a> {
    const fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            memo: BTreeMap::new(),
            completed: 0,
        }
    }
    fn values(&mut self, values: &[Value], shown: bool) -> Result<(), Error> {
        number(self.out, values.len() as u64);
        for value in values {
            self.value(value, shown)?;
        }
        Ok(())
    }
    fn keyed(
        &mut self,
        values: &[(usize, Value)],
        shown: bool,
    ) -> Result<(), Error> {
        number(self.out, values.len() as u64);
        for (key, value) in values {
            number(self.out, *key as u64);
            self.value(value, shown)?;
        }
        Ok(())
    }
    fn scalar(
        &mut self,
        value: &Value,
        shown: bool,
    ) -> Result<ValueProperties, Error> {
        let opaque_safe = match value {
            Value::Unit => {
                number(self.out, 0);
                true
            }
            Value::Int(int) => {
                number(self.out, 1);
                self.out.extend(int.to_le_bytes());
                true
            }
            Value::Big(big) => {
                number(self.out, 2);
                number(self.out, u64::from(big.negative));
                bytes(self.out, big.magnitude.to_string().as_bytes());
                true
            }
            Value::F32(float) => {
                number(self.out, 3);
                number(self.out, u64::from(float.to_bits()));
                true
            }
            Value::F64(float) => {
                number(self.out, 4);
                number(self.out, float.to_bits());
                true
            }
            Value::Text(text) => {
                number(self.out, 5);
                bytes(self.out, text.as_bytes());
                true
            }
            Value::Function(_) if shown => {
                number(self.out, 10);
                false
            }
            Value::Resource(_) | Value::Function(_) | Value::Meaning(_) => {
                return Err(Error::Opaque)
            }
            _ => unreachable!("noncontainer value"),
        };
        Ok(ValueProperties {
            height: 0,
            opaque_safe,
        })
    }

    fn value(&mut self, value: &Value, shown: bool) -> Result<(), Error> {
        enum Job<'a> {
            Read(&'a Value, usize),
            Finish(Allocation, usize),
        }
        let mut jobs = vec![Job::Read(value, 0)];
        let mut done: Vec<ValueProperties> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Finish(identity, count) => {
                    let start = done
                        .len()
                        .checked_sub(count)
                        .ok_or(Error::Malformed)?;
                    let properties =
                        container_properties(done[start..].iter().copied())?;
                    done.truncate(start);
                    let index = self.completed;
                    self.completed = self
                        .completed
                        .checked_add(1)
                        .ok_or(Error::Malformed)?;
                    self.memo.insert(
                        identity,
                        WrittenValue::Complete { index, properties },
                    );
                    done.push(properties);
                }
                Job::Read(value, depth) => {
                    if depth > MAX_VALUE_DEPTH {
                        return Err(Error::Malformed);
                    }
                    if let Some(identity) = value_allocation(value) {
                        if let Some(known) = self.memo.get(&identity) {
                            let WrittenValue::Complete { index, properties } =
                                known
                            else {
                                return Err(Error::Malformed);
                            };
                            properties.allowed(depth, shown)?;
                            number(self.out, VALUE_REFERENCE);
                            number(self.out, *index as u64);
                            done.push(*properties);
                            continue;
                        }
                        if depth == MAX_VALUE_DEPTH {
                            return Err(Error::Malformed);
                        }
                        self.memo.insert(identity, WrittenValue::Visiting);
                        let children: &[Value] = match value {
                            Value::Seq(parts) | Value::Tuple(parts) => {
                                number(
                                    self.out,
                                    if matches!(value, Value::Seq(_)) {
                                        6
                                    } else {
                                        7
                                    },
                                );
                                number(self.out, parts.len() as u64);
                                parts
                            }
                            Value::Record(record) => {
                                number(self.out, 8);
                                number(self.out, record.meaning as u64);
                                number(self.out, record.constructor as u64);
                                number(self.out, record.fields.len() as u64);
                                &record.fields
                            }
                            Value::Packed(packed) => {
                                number(self.out, 11);
                                type_into(self.out, packed.source_type());
                                type_into(self.out, packed.target_type());
                                std::slice::from_ref(packed.value())
                            }
                            _ => unreachable!("container identity"),
                        };
                        jobs.push(Job::Finish(identity, children.len()));
                        jobs.extend(
                            children
                                .iter()
                                .rev()
                                .map(|child| Job::Read(child, depth + 1)),
                        );
                    } else {
                        done.push(self.scalar(value, shown)?);
                    }
                }
            }
        }
        debug_assert_eq!(done.len(), 1);
        Ok(())
    }
}

/// A standalone value starts a fresh message memo.
fn value_into(
    out: &mut Vec<u8>,
    value: &Value,
    shown: bool,
) -> Result<(), Error> {
    ValueWriter::new(out).value(value, shown)
}

#[derive(Clone)]
struct ReadValue {
    value: Value,
    properties: ValueProperties,
}
enum ValueShape {
    Seq,
    Tuple,
    Record(usize, usize),
    Packed(TypeForest),
}
struct ValueReader<'r, 'a> {
    input: &'r mut Reader<'a>,
    memo: Vec<ReadValue>,
}
impl<'r, 'a> ValueReader<'r, 'a> {
    const fn new(input: &'r mut Reader<'a>) -> Self {
        Self {
            input,
            memo: Vec::new(),
        }
    }
    fn count(&mut self, minimum: usize) -> Result<usize, Error> {
        let count = self.input.index()?;
        if count > self.input.0.len() / minimum {
            return Err(Error::Short);
        }
        Ok(count)
    }
    fn values(&mut self, shown: bool) -> Result<Vec<Value>, Error> {
        let count = self.count(8)?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(self.value(shown)?);
        }
        Ok(values)
    }
    fn keyed(&mut self, shown: bool) -> Result<Vec<(usize, Value)>, Error> {
        let count = self.count(16)?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push((self.input.index()?, self.value(shown)?));
        }
        Ok(values)
    }
    fn finish(
        &mut self,
        shape: ValueShape,
        count: usize,
        done: &mut Vec<ReadValue>,
    ) -> Result<(), Error> {
        let start = done.len().checked_sub(count).ok_or(Error::Malformed)?;
        let parts = done.split_off(start);
        let properties =
            container_properties(parts.iter().map(|p| p.properties))?;
        let mut values: Vec<_> = parts.into_iter().map(|p| p.value).collect();
        let value = match shape {
            ValueShape::Seq => Value::seq(values),
            ValueShape::Tuple => Value::Tuple(Rc::new(values)),
            ValueShape::Record(meaning, constructor) => {
                Value::Record(Rc::new(Record {
                    meaning,
                    constructor,
                    fields: values,
                }))
            }
            ValueShape::Packed(mut roots) => {
                let target = roots.0.pop().expect("target");
                let source = roots.0.pop().expect("source");
                Value::Packed(Rc::new(
                    Packed::from_data(
                        source,
                        target,
                        &values.pop().expect("payload"),
                    )
                    .ok_or(Error::Opaque)?,
                ))
            }
        };
        let entry = ReadValue { value, properties };
        self.memo.push(entry.clone());
        done.push(entry);
        Ok(())
    }

    fn scalar(&mut self, tag: u64, shown: bool) -> Result<ReadValue, Error> {
        let value = match tag {
            0 => Value::Unit,
            1 => Value::Int(self.input.number()?.cast_signed()),
            2 => {
                let negative = self.input.number()? != 0;
                let magnitude = Magnitude::parse(&self.input.text()?, 10)
                    .ok_or(Error::Malformed)?;
                Value::Big(Rc::new(BigInt {
                    negative,
                    magnitude,
                }))
            }
            3 => Value::F32(f32::from_bits(
                u32::try_from(self.input.number()?)
                    .map_err(|_| Error::Malformed)?,
            )),
            4 => Value::F64(f64::from_bits(self.input.number()?)),
            5 => Value::text(&self.input.text()?),
            10 if shown => function(),
            _ => return Err(Error::Tag(tag)),
        };
        Ok(ReadValue {
            value,
            properties: ValueProperties {
                height: 0,
                opaque_safe: tag != 10,
            },
        })
    }

    fn value(&mut self, shown: bool) -> Result<Value, Error> {
        enum Job {
            Read(usize),
            Finish(ValueShape, usize),
        }
        let mut jobs = vec![Job::Read(0)];
        let mut reads = 1_usize;
        let mut done: Vec<ReadValue> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Finish(shape, count) => {
                    self.finish(shape, count, &mut done)?;
                }
                Job::Read(depth) => {
                    reads -= 1;
                    if depth > MAX_VALUE_DEPTH {
                        return Err(Error::Malformed);
                    }
                    let tag = self.input.number()?;
                    if matches!(tag, 6 | 7 | 8 | 11) && depth == MAX_VALUE_DEPTH
                    {
                        return Err(Error::Malformed);
                    }
                    if tag == VALUE_REFERENCE {
                        let index = self.input.index()?;
                        let entry =
                            self.memo.get(index).ok_or(Error::Malformed)?;
                        entry.properties.allowed(depth, shown)?;
                        done.push(entry.clone());
                        continue;
                    }
                    let (shape, count) = match tag {
                        6 | 7 => (
                            if tag == 6 {
                                ValueShape::Seq
                            } else {
                                ValueShape::Tuple
                            },
                            self.count(8)?,
                        ),
                        8 => {
                            let meaning = self.input.index()?;
                            let constructor = self.input.index()?;
                            (
                                ValueShape::Record(meaning, constructor),
                                self.count(8)?,
                            )
                        }
                        11 => {
                            let mut roots =
                                TypeForest(vec![self.input.type_root()?]);
                            roots.0.push(self.input.type_root()?);
                            (ValueShape::Packed(roots), 1)
                        }
                        _ => {
                            done.push(self.scalar(tag, shown)?);
                            continue;
                        }
                    };
                    reads = reads.checked_add(count).ok_or(Error::Malformed)?;
                    if reads > self.input.0.len() / 8 {
                        return Err(Error::Short);
                    }
                    jobs.push(Job::Finish(shape, count));
                    jobs.extend((0..count).map(|_| Job::Read(depth + 1)));
                }
            }
        }
        let result = done.pop().ok_or(Error::Malformed)?;
        debug_assert!(done.is_empty());
        Ok(result.value)
    }
}

fn type_into(out: &mut Vec<u8>, ty: &ir::Type) {
    let mut work = vec![ty];
    while let Some(ty) = work.pop() {
        match ty {
            ir::Type::Unit => number(out, 0),
            ir::Type::Parameter(i) => {
                number(out, 1);
                number(out, *i as u64);
            }
            ir::Type::Session(i) => {
                number(out, 2);
                number(out, *i as u64);
            }
            ir::Type::Number(n) => {
                number(out, 3);
                bytes(out, n.to_string().as_bytes());
            }
            ir::Type::Meaning { index, arguments } => {
                number(out, 4);
                number(out, *index as u64);
                number(out, arguments.len() as u64);
                work.extend(arguments.iter().rev());
            }
            ir::Type::Tuple(p)
            | ir::Type::Packed(p)
            | ir::Type::All(p)
            | ir::Type::Sum(p) => {
                number(
                    out,
                    match ty {
                        ir::Type::Tuple(_) => 5,
                        ir::Type::Packed(_) => 8,
                        ir::Type::All(_) => 9,
                        _ => 11,
                    },
                );
                number(out, p.len() as u64);
                work.extend(p.iter().rev());
            }
            ir::Type::Array { element, length } => {
                number(out, 6);
                work.push(length);
                work.push(element);
            }
            ir::Type::Function { parameter, result } => {
                number(out, 7);
                work.push(result);
                work.push(parameter);
            }
            ir::Type::Not(inner) => {
                number(out, 10);
                work.push(inner);
            }
        }
    }
}

// Partial decode/copy roots have the same iterative last-owner behavior as Meaning.
#[derive(Default)]
struct TypeForest(Vec<ir::Type>);
impl Drop for TypeForest {
    fn drop(&mut self) {
        while let Some(ty) = self.0.pop() {
            match ty {
                ir::Type::Meaning { arguments, .. }
                | ir::Type::Tuple(arguments)
                | ir::Type::Packed(arguments)
                | ir::Type::All(arguments)
                | ir::Type::Sum(arguments) => self.0.extend(arguments),
                ir::Type::Array { element, length } => {
                    self.0.push(*element);
                    self.0.push(*length);
                }
                ir::Type::Function { parameter, result } => {
                    self.0.push(*parameter);
                    self.0.push(*result);
                }
                ir::Type::Not(inner) => self.0.push(*inner),
                ir::Type::Unit
                | ir::Type::Parameter(_)
                | ir::Type::Session(_)
                | ir::Type::Number(_) => {}
            }
        }
    }
}
#[derive(Clone, Copy)]
enum TypeShape {
    Meaning(usize),
    Tuple,
    Array,
    Function,
    Packed,
    All,
    Not,
    Sum,
}
fn finish_type(shape: TypeShape, n: usize, done: &mut TypeForest) {
    let mut p = TypeForest(done.0.split_off(done.0.len() - n));
    let ty = match shape {
        TypeShape::Meaning(index) => ir::Type::Meaning {
            index,
            arguments: std::mem::take(&mut p.0),
        },
        TypeShape::Tuple => ir::Type::Tuple(std::mem::take(&mut p.0)),
        TypeShape::Packed => ir::Type::Packed(std::mem::take(&mut p.0)),
        TypeShape::All => ir::Type::All(std::mem::take(&mut p.0)),
        TypeShape::Sum => ir::Type::Sum(std::mem::take(&mut p.0)),
        TypeShape::Array => {
            let length = Box::new(p.0.pop().unwrap());
            let element = Box::new(p.0.pop().unwrap());
            ir::Type::Array { element, length }
        }
        TypeShape::Function => {
            let result = Box::new(p.0.pop().unwrap());
            let parameter = Box::new(p.0.pop().unwrap());
            ir::Type::Function { parameter, result }
        }
        TypeShape::Not => ir::Type::Not(Box::new(p.0.pop().unwrap())),
    };
    done.0.push(ty);
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

    #[cfg(test)]
    fn value(&mut self, shown: bool) -> Result<Value, Error> {
        ValueReader::new(self).value(shown)
    }

    fn type_root(&mut self) -> Result<ir::Type, Error> {
        enum Job {
            Read,
            Finish(TypeShape, usize),
        }
        let mut work = vec![Job::Read];
        let mut reads = 1_usize;
        let mut done = TypeForest::default();
        while let Some(job) = work.pop() {
            match job {
                Job::Finish(shape, n) => finish_type(shape, n, &mut done),
                Job::Read => {
                    reads -= 1;
                    let (shape, n) = match self.number()? {
                        0 => {
                            done.0.push(ir::Type::Unit);
                            continue;
                        }
                        1 => {
                            done.0.push(ir::Type::Parameter(self.index()?));
                            continue;
                        }
                        2 => {
                            done.0.push(ir::Type::Session(self.index()?));
                            continue;
                        }
                        3 => {
                            let n = Magnitude::parse(&self.text()?, 10)
                                .ok_or(Error::Malformed)?;
                            done.0.push(ir::Type::Number(n));
                            continue;
                        }
                        4 => {
                            let i = self.index()?;
                            (TypeShape::Meaning(i), self.index()?)
                        }
                        5 => (TypeShape::Tuple, self.index()?),
                        6 => (TypeShape::Array, 2),
                        7 => (TypeShape::Function, 2),
                        8 => (TypeShape::Packed, self.index()?),
                        9 => (TypeShape::All, self.index()?),
                        10 => (TypeShape::Not, 1),
                        11 => (TypeShape::Sum, self.index()?),
                        tag => return Err(Error::Tag(tag)),
                    };
                    // Each outstanding child needs at least one tag; include siblings.
                    reads = reads
                        .checked_add(n)
                        .filter(|total| *total <= self.0.len() / 8)
                        .ok_or(Error::Short)?;
                    work.push(Job::Finish(shape, n));
                    work.extend((0..n).map(|_| Job::Read));
                }
            }
        }
        Ok(done.0.pop().expect("decoded root"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_value_boundary_memo_counts_stored_edges_without_owned_copies() {
        let mut shared = Value::Tuple(Rc::new(vec![Value::Int(7)]));
        for _ in 0..12 {
            shared = Value::Tuple(Rc::new(vec![shared.clone(), shared]));
        }
        let (found, visits) = carrier_scan([&shared]);
        assert!(!found);
        assert!(visits <= 32, "shared graph revisited {visits} nodes");
        let Value::Tuple(parts) = &shared else {
            unreachable!()
        };
        let owners = Rc::strong_count(parts);
        assert!(!carrier_scan([&shared, &shared]).0);
        assert_eq!(Rc::strong_count(parts), owners);
        let mut declarations = Declarations::new();
        declarations
            .module("csv", "pub type resource Table\npub get: Table")
            .unwrap();
        let module = parse(lex("use csv\nf = _ => csv.get")).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let ty = fipe::value::resource::ResourceType::registered(
            &unit.tables(),
            "csv",
            "Table",
        )
        .unwrap();
        let resource = fipe::value::resource::ResourceDomain::new()
            .issue(&ty, Rc::from(&b"memo-private-entry"[..]));
        let hidden = Value::Function(Rc::new(Closure {
            body: usize::MAX,
            captures: vec![resource],
            meanings: vec![],
            filled: vec![shared],
            guards: vec![],
        }));
        let aliases =
            Value::Tuple(Rc::new(vec![hidden.clone(), hidden.clone()]));
        assert_eq!(encode_reply(&Ok(aliases.clone())), Err(Error::Opaque));
        assert_eq!(
            Frame::Done {
                result: Ok(aliases.clone()),
                lines: vec![(0, aliases)],
                charged: 0
            }
            .encode(),
            Err(Error::Opaque)
        );
        let (found, visits) = carrier_scan([&hidden]);
        assert!(found);
        assert!(visits <= 36);
        assert_eq!(
            Frame::Done {
                result: Ok(Value::Unit),
                lines: vec![(0, hidden)],
                charged: 0
            }
            .encode(),
            Err(Error::Opaque)
        );
    }
}

#[cfg(test)]
mod packed_type_wire {
    use super::*;

    #[test]
    fn every_type_form_round_trips_without_tables_or_evidence() {
        let source = ir::Type::Tuple(vec![
            ir::Type::Unit,
            ir::Type::Parameter(3),
            ir::Type::Session(4),
            ir::Type::Number(Magnitude::parse("19", 10).unwrap()),
            ir::Type::Meaning {
                index: 5,
                arguments: vec![ir::Type::Unit],
            },
            ir::Type::Array {
                element: Box::new(ir::Type::Parameter(2)),
                length: Box::new(ir::Type::Number(
                    Magnitude::parse("0", 10).unwrap(),
                )),
            },
            ir::Type::Function {
                parameter: Box::new(ir::Type::Unit),
                result: Box::new(ir::Type::Session(1)),
            },
            ir::Type::Packed(vec![ir::Type::Unit]),
            ir::Type::All(vec![ir::Type::Unit]),
            ir::Type::Not(Box::new(ir::Type::Unit)),
            ir::Type::Sum(vec![ir::Type::Unit]),
        ]);
        let value = Value::Packed(Rc::new(
            Packed::from_data(source, ir::Type::Parameter(9), &Value::Int(7))
                .unwrap(),
        ));
        let mut encoded = Vec::new();
        value_into(&mut encoded, &value, false).unwrap();
        assert_eq!(&encoded[..8], &11_u64.to_le_bytes());
        let decoded = Reader(&encoded).value(false).unwrap();
        assert_eq!(decoded, value);
        let (Value::Packed(original), Value::Packed(decoded)) =
            (&value, &decoded)
        else {
            unreachable!()
        };
        assert_eq!(original.source_type(), decoded.source_type());
        assert_eq!(original.target_type(), decoded.target_type());
        assert_eq!(decoded.value(), &Value::Int(7));
    }

    #[test]
    fn legacy_packed_tag_is_rejected_and_function_policy_is_unchanged() {
        assert_eq!(
            Reader(&9_u64.to_le_bytes()).value(false),
            Err(Error::Tag(9))
        );
        assert_eq!(
            Reader(&10_u64.to_le_bytes()).value(false),
            Err(Error::Tag(10))
        );
        assert_eq!(Reader(&10_u64.to_le_bytes()).value(true), Ok(function()));
    }

    #[test]
    fn nested_wide_headers_cannot_reuse_their_siblings_byte_quota() {
        let mut malformed = Vec::new();
        for _ in 0..2 {
            number(&mut malformed, 5);
            number(&mut malformed, 64);
        }
        for _ in 0..64 {
            number(&mut malformed, 0);
        }
        let mut reader = Reader(&malformed);
        assert!(matches!(reader.type_root(), Err(Error::Short)));
        assert_eq!(
            reader.0.len(),
            64 * 8,
            "reject at the second header before allocating its children"
        );
        let mut valid = Vec::new();
        number(&mut valid, 5);
        number(&mut valid, 64);
        for _ in 0..64 {
            number(&mut valid, 0);
        }
        let root =
            fipe::value::Meaning::new(Reader(&valid).type_root().unwrap());
        assert!(
            matches!(root.as_type(),ir::Type::Tuple(parts) if parts.len()==64)
        );
    }

    #[test]
    fn deep_complete_and_partial_type_roots_drop_iteratively() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut root = ir::Type::Unit;
                for _ in 0..4096 {
                    root = ir::Type::Not(Box::new(root));
                }
                let root = fipe::value::Meaning::new(root);
                let mut encoded = Vec::new();
                type_into(&mut encoded, root.as_type());
                let decoded = fipe::value::Meaning::new(
                    Reader(&encoded).type_root().unwrap(),
                );
                let mut work = vec![decoded.as_type()];
                let mut nodes = 0;
                while let Some(ty) = work.pop() {
                    nodes += 1;
                    if let ir::Type::Not(inner) = ty {
                        work.push(inner);
                    }
                }
                assert_eq!(nodes, 4097);
                // A complete deep first child must be cleaned when its sibling is short.
                let mut partial = Vec::new();
                number(&mut partial, 5);
                number(&mut partial, 2);
                partial.extend(encoded);
                number(&mut partial, 4);
                assert!(matches!(
                    Reader(&partial).type_root(),
                    Err(Error::Short)
                ));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

#[cfg(test)]
mod value_graph_wire {
    use super::*;

    fn nested(mut value: Value, depth: usize) -> Value {
        for _ in 0..depth {
            value = Value::Tuple(Rc::new(vec![value]));
        }
        value
    }
    fn pair(value: Value) -> Value {
        Value::Tuple(Rc::new(vec![value.clone(), value]))
    }
    fn children(value: &Value) -> &[Value] {
        let Value::Tuple(parts) = value else {
            panic!("tuple")
        };
        parts
    }
    fn same_allocation(a: &Value, b: &Value) -> bool {
        value_allocation(a).is_some()
            && value_allocation(a) == value_allocation(b)
    }

    #[test]
    fn every_data_container_preserves_its_shared_identity() {
        for child in [
            Value::seq(vec![Value::Int(7)]),
            Value::Tuple(Rc::new(vec![Value::Int(7)])),
            Value::Record(Rc::new(Record {
                meaning: 4,
                constructor: 2,
                fields: vec![Value::Int(7)],
            })),
            Value::Packed(Rc::new(
                Packed::from_data(
                    ir::Type::Unit,
                    ir::Type::Parameter(3),
                    &Value::Int(7),
                )
                .unwrap(),
            )),
        ] {
            let original = pair(child);
            let bytes = encode_reply(&Ok(original.clone())).unwrap();
            let decoded = decode_reply(&bytes).unwrap().unwrap();
            assert_eq!(decoded, original);
            let parts = children(&decoded);
            assert!(same_allocation(&parts[0], &parts[1]));
        }
    }

    #[test]
    fn shared_vector_identity_keeps_sequence_and_tuple_kinds_distinct() {
        let parts = Rc::new(vec![Value::Int(7)]);
        let sequence = Value::Seq(parts.clone());
        let tuple = Value::Tuple(parts);
        let original = Value::Tuple(Rc::new(vec![
            sequence.clone(),
            tuple.clone(),
            sequence,
            tuple,
        ]));
        let decoded = decode_reply(&encode_reply(&Ok(original)).unwrap())
            .unwrap()
            .unwrap();
        let parts = children(&decoded);
        let (
            Value::Seq(first_sequence),
            Value::Tuple(first_tuple),
            Value::Seq(second_sequence),
            Value::Tuple(second_tuple),
        ) = (&parts[0], &parts[1], &parts[2], &parts[3])
        else {
            panic!("distinct kinds")
        };
        assert!(Rc::ptr_eq(first_sequence, second_sequence));
        assert!(Rc::ptr_eq(first_tuple, second_tuple));
        assert!(!Rc::ptr_eq(first_sequence, first_tuple));
    }

    #[test]
    fn message_memo_spans_arguments_environment_lines_and_result() {
        let child = Value::Tuple(Rc::new(vec![Value::Int(7)]));
        let call = Frame::Call {
            request: Request::Call {
                declaration: 2,
                arguments: vec![child.clone()],
                environment: vec![(3, child.clone())],
            },
            lines: vec![(5, child.clone())],
        };
        let Frame::Call {
            request:
                Request::Call {
                    arguments,
                    environment,
                    ..
                },
            lines,
        } = Frame::decode(&call.encode().unwrap()).unwrap()
        else {
            panic!("call")
        };
        assert!(same_allocation(&arguments[0], &environment[0].1));
        assert!(same_allocation(&arguments[0], &lines[0].1));
        let done = Frame::Done {
            result: Ok(child.clone()),
            lines: vec![(5, child)],
            charged: 9,
        };
        let Frame::Done {
            result: Ok(value),
            lines,
            ..
        } = Frame::decode(&done.encode().unwrap()).unwrap()
        else {
            panic!("done")
        };
        assert!(same_allocation(&value, &lines[0].1));
    }

    #[test]
    fn unshared_bytes_are_unchanged_and_old_tree_reader_rejects_references() {
        // Frozen old tag-dispatch behavior, for the fixture's Unit/Int/Tuple subset.
        fn legacy(input: &mut Reader<'_>) -> Result<(), Error> {
            match input.number()? {
                0 => Ok(()),
                1 => {
                    input.number()?;
                    Ok(())
                }
                7 => {
                    for _ in 0..input.number()? {
                        legacy(input)?;
                    }
                    Ok(())
                }
                tag => Err(Error::Tag(tag)),
            }
        }

        let plain = Value::Tuple(Rc::new(vec![Value::Int(7)]));
        let expected: Vec<_> = [0_u64, 7, 1, 1, 7]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        assert_eq!(encode_reply(&Ok(plain.clone())).unwrap(), expected);
        let bytes = encode_reply(&Ok(pair(plain))).unwrap();
        let mut old = Reader(&bytes);
        assert_eq!(old.number().unwrap(), 0);
        assert_eq!(legacy(&mut old), Err(Error::Tag(VALUE_REFERENCE)));
    }

    #[test]
    fn forward_cyclic_unknown_and_cross_message_references_refuse() {
        for values in [
            vec![0_u64, VALUE_REFERENCE, 0],
            vec![0, VALUE_REFERENCE, u64::MAX],
            vec![0, 7, 1, VALUE_REFERENCE, 0],
        ] {
            let bytes: Vec<_> =
                values.into_iter().flat_map(u64::to_le_bytes).collect();
            assert_eq!(decode_reply(&bytes), Err(Error::Malformed));
        }
        let complete =
            encode_reply(&Ok(Value::Tuple(Rc::new(vec![])))).unwrap();
        assert!(decode_reply(&complete).is_ok());
        let fresh_ref: Vec<_> = [0_u64, VALUE_REFERENCE, 0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        assert_eq!(decode_reply(&fresh_ref), Err(Error::Malformed));
    }

    #[test]
    fn shown_function_containers_cannot_be_referenced_in_opaque_mode() {
        let value = Value::Tuple(Rc::new(vec![function()]));
        let mut bytes = Vec::new();
        let mut writer = ValueWriter::new(&mut bytes);
        writer.value(&value, true).unwrap();
        assert_eq!(writer.value(&value, false), Err(Error::Opaque));
        // A hostile stream can still spell the forbidden reference itself.
        number(&mut bytes, VALUE_REFERENCE);
        number(&mut bytes, 0);
        let mut raw = Reader(&bytes);
        let mut reader = ValueReader::new(&mut raw);
        assert!(reader.value(true).is_ok());
        assert_eq!(reader.value(false), Err(Error::Opaque));
        let mut bytes = Vec::new();
        let mut writer = ValueWriter::new(&mut bytes);
        writer.value(&value, true).unwrap();
        writer.value(&value, true).unwrap();
        let mut raw = Reader(&bytes);
        let mut reader = ValueReader::new(&mut raw);
        let first = reader.value(true).unwrap();
        let second = reader.value(true).unwrap();
        assert!(same_allocation(&first, &second));
    }

    #[test]
    fn depth_bound_and_partial_value_type_cleanup_are_stack_bounded() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let value = nested(Value::Unit, MAX_VALUE_DEPTH);
                let bytes = encode_reply(&Ok(value.clone())).unwrap();
                assert_eq!(decode_reply(&bytes).unwrap(), Ok(value));
                let over = nested(Value::Unit, MAX_VALUE_DEPTH + 1);
                assert_eq!(encode_reply(&Ok(over)), Err(Error::Malformed));
                let mut over_bytes = Vec::new();
                number(&mut over_bytes, 0);
                for _ in 0..=MAX_VALUE_DEPTH {
                    number(&mut over_bytes, 7);
                    number(&mut over_bytes, 1);
                }
                number(&mut over_bytes, 0);
                assert_eq!(decode_reply(&over_bytes), Err(Error::Malformed));
                let mut partial = Vec::new();
                number(&mut partial, 0);
                number(&mut partial, 7);
                number(&mut partial, 2);
                value_into(
                    &mut partial,
                    &nested(Value::Unit, MAX_VALUE_DEPTH - 1),
                    false,
                )
                .unwrap();
                number(&mut partial, 11);
                let mut ty = ir::Type::Unit;
                for _ in 0..4096 {
                    ty = ir::Type::Not(Box::new(ty));
                }
                let ty = fipe::value::Meaning::new(ty);
                type_into(&mut partial, ty.as_type());
                assert!(matches!(decode_reply(&partial), Err(Error::Short)));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn references_cannot_insert_a_completed_subtree_beyond_the_depth_bound() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let target = nested(Value::Unit, 128);
                let mut allowed = Vec::new();
                let mut writer = ValueWriter::new(&mut allowed);
                writer.value(&target, false).unwrap();
                writer.value(&nested(target.clone(), 128), false).unwrap();
                let mut raw = Reader(&allowed);
                let mut reader = ValueReader::new(&mut raw);
                assert!(reader.value(false).is_ok());
                assert!(reader.value(false).is_ok());
                let mut denied = Vec::new();
                let mut writer = ValueWriter::new(&mut denied);
                writer.value(&target, false).unwrap();
                assert_eq!(
                    writer.value(&nested(target.clone(), 129), false),
                    Err(Error::Malformed)
                );
                let mut hostile = Vec::new();
                value_into(&mut hostile, &target, false).unwrap();
                for _ in 0..129 {
                    number(&mut hostile, 7);
                    number(&mut hostile, 1);
                }
                number(&mut hostile, VALUE_REFERENCE);
                number(&mut hostile, 127);
                let mut raw = Reader(&hostile);
                let mut reader = ValueReader::new(&mut raw);
                assert!(reader.value(false).is_ok());
                assert_eq!(reader.value(false), Err(Error::Malformed));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn shared_dags_encode_compactly_and_malformed_widths_do_not_reserve() {
        let mut value = Value::Int(7);
        for _ in 0..48 {
            value = pair(value);
        }
        let encoded = encode_reply(&Ok(value)).unwrap();
        assert!(
            encoded.len() < 2048,
            "one definition and one ref per shared level"
        );
        let decoded = decode_reply(&encoded).unwrap().unwrap();
        let mut next = &decoded;
        for _ in 0..48 {
            let parts = children(next);
            assert!(
                same_allocation(&parts[0], &parts[1])
                    || matches!(
                        (&parts[0], &parts[1]),
                        (Value::Int(7), Value::Int(7))
                    )
            );
            next = &parts[0];
        }
        assert_eq!(next, &Value::Int(7));
        let mut malformed = Vec::new();
        number(&mut malformed, 0);
        for _ in 0..2 {
            number(&mut malformed, 7);
            number(&mut malformed, 64);
        }
        for _ in 0..64 {
            number(&mut malformed, 0);
        }
        let mut raw = Reader(&malformed);
        assert_eq!(raw.number().unwrap(), 0);
        let mut reader = ValueReader::new(&mut raw);
        assert_eq!(reader.value(false), Err(Error::Short));
        assert_eq!(reader.input.0.len(), 64 * 8);
        let huge: Vec<_> = [0_u64, 7, u64::MAX]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        assert_eq!(decode_reply(&huge), Err(Error::Short));
        for end in 0..encoded.len() {
            assert!(decode_reply(&encoded[..end]).is_err());
        }
    }
}
