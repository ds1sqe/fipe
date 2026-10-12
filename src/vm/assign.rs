//! An assignment: an rvalue computed into a local of the top frame,
//! and a conversion, which may start a predicate run.
use super::{
    frames::{Frame, NativeFrame},
    natives::{Ctx, Step},
    pending::Pending,
    Flow, Machine, Return,
};
use crate::{
    ir, mir,
    mir::Rvalue,
    value::{Closure, Packed, Reason, Record, Value},
};
use alloc::{rc::Rc, vec::Vec};

impl Machine {
    pub(super) fn assign(
        &mut self,
        program: &mir::Program,
        dest: usize,
        rvalue: &Rvalue,
        node: usize,
    ) -> Result<Flow, Reason> {
        let value = match rvalue {
            Rvalue::Use(operand) => self.operand(program, *operand),
            Rvalue::Construct {
                meaning,
                constructor,
                fields,
            } => Value::Record(Rc::new(Record {
                meaning: *meaning,
                constructor: *constructor,
                fields: self.operands(program, fields),
            })),
            Rvalue::Tuple(parts) => {
                Value::Tuple(Rc::new(self.operands(program, parts)))
            }
            Rvalue::Array(parts) => Value::seq(self.operands(program, parts)),
            Rvalue::Repeat { value, length } => {
                self.repeat(program, *value, *length)
            }
            Rvalue::Field(value, index) => {
                super::step::field(&self.operand(program, *value), *index)
            }
            Rvalue::Length(value) => match self.operand(program, *value) {
                Value::Seq(parts) => Value::Int(
                    i64::try_from(parts.len()).expect("a length fits"),
                ),
                other => unreachable!("a sequence, not {other:?}"),
            },
            Rvalue::Closure {
                body,
                captures,
                meanings,
            } => self.closure(program, *body, captures, meanings),
            Rvalue::Wrap {
                meaning,
                constructor,
                value,
            } => Value::Record(Rc::new(Record {
                meaning: *meaning,
                constructor: *constructor,
                fields: alloc::vec![self.operand(program, *value)],
            })),
            Rvalue::Pack { tag, value } => self.pack(program, tag, *value),
            Rvalue::Cast { row, value, .. } => {
                // The target is the local's type, witnesses in place.
                let target = self.local_type(program, dest);
                let value = self.operand(program, *value);
                super::casts::cast(*row, &target, value, &self.prelude())?
            }
            Rvalue::Convert {
                mode,
                predicate,
                value,
            } => {
                let value = self.operand(program, *value);
                return self
                    .convert(program, dest, *mode, predicate, value, node);
            }
            Rvalue::Native { op, args } => {
                self.native_rvalue(program, dest, *op, args)?
            }
        };
        self.top().locals[dest] = value;
        Ok(Flow::Continue)
    }

    /// A repeated element, its length written or read from a witness.
    fn repeat(
        &self,
        program: &mir::Program,
        value: mir::Operand,
        length: mir::Length,
    ) -> Value {
        let value = self.operand(program, value);
        let length = match length {
            mir::Length::Fixed(length) => length,
            mir::Length::Hidden(index) => {
                let Some(Frame::Body(frame)) = self.frames.last() else {
                    unreachable!()
                };
                match &frame.meanings[index] {
                    ir::Type::Number(number) => {
                        number.to_u64().expect("a length fits")
                    }
                    other => unreachable!("a length, not {other:?}"),
                }
            }
        };
        let length = usize::try_from(length).expect("a length fits");
        Value::seq(alloc::vec![value; length])
    }

    /// A closure of a body with its captures copied now and its
    /// meanings substituted by this frame.
    fn closure(
        &self,
        program: &mir::Program,
        body: usize,
        captures: &[mir::Operand],
        meanings: &[ir::Type],
    ) -> Value {
        let captures = self.operands(program, captures);
        let Some(Frame::Body(frame)) = self.frames.last() else {
            unreachable!()
        };
        let meanings = meanings
            .iter()
            .map(|ty| Self::substitute(program, frame, ty))
            .collect();
        Value::Function(Rc::new(Closure {
            body,
            captures,
            meanings,
            filled: Vec::new(),
        }))
    }

    /// A value packed under its tag's meaning.
    fn pack(
        &self,
        program: &mir::Program,
        tag: &ir::Type,
        value: mir::Operand,
    ) -> Value {
        let Some(Frame::Body(frame)) = self.frames.last() else {
            unreachable!()
        };
        let ir::Type::Meaning { index, .. } =
            Self::substitute(program, frame, tag)
        else {
            unreachable!("a tag is a meaning")
        };
        Value::Packed(Rc::new(Packed {
            meaning: index,
            value: self.operand(program, value),
        }))
    }

    /// A native as an rvalue, which computes at once.
    fn native_rvalue(
        &mut self,
        program: &mir::Program,
        dest: usize,
        op: usize,
        args: &[mir::Operand],
    ) -> Result<Value, Reason> {
        let args = self.operands(program, args);
        let result = self.local_type(program, dest);
        let natives = self.natives();
        self.pay(super::budget::size(natives.of(op), &args))?;
        let prelude = self.prelude();
        let tables = program.tables.borrow();
        let ctx = Ctx {
            program,
            tables: &tables,
            prelude: &prelude,
            natives: &natives,
            globals: &self.globals,
            result: Some(result),
        };
        match natives.call(op, args, &ctx)? {
            Step::Done(value) => Ok(value),
            Step::Fault(reason) => Err(reason),
            Step::Call(_) | Step::Start(_) => {
                unreachable!("a native rvalue computes at once")
            }
        }
    }

    /// A conversion: direct, wrapped, unwrapped, or checked against its
    /// predicate by the predicate's tests.
    fn convert(
        &mut self,
        program: &mir::Program,
        dest: usize,
        mode: ir::ConversionMode,
        predicate: &ir::Predicate,
        value: Value,
        node: usize,
    ) -> Result<Flow, Reason> {
        let prelude = self.prelude();
        let value = match mode {
            ir::ConversionMode::Direct => value,
            ir::ConversionMode::Wrap => {
                let ty = self.local_type(program, dest);
                match ty {
                    ir::Type::Meaning { index, .. }
                        if Some(index) == prelude.maybe =>
                    {
                        prelude.just(value)
                    }
                    _ => prelude.ok(value),
                }
            }
            ir::ConversionMode::Unwrap => match &value {
                Value::Record(record) if record.constructor == 0 => {
                    record.fields[0].clone()
                }
                _ => return Err(Reason::Cast),
            },
            ir::ConversionMode::Check | ir::ConversionMode::TryCheck => {
                let frame = self.top();
                let (block, statement) = (frame.block, frame.statement);
                let run = super::predicates::Run::new(
                    predicate.clone(),
                    value,
                    super::predicates::After::Convert(mode),
                );
                self.frames.push(Frame::Native(NativeFrame {
                    node,
                    pending: Pending::Predicate(run),
                    back: Return::Into {
                        dest,
                        block,
                        statement,
                    },
                    result: None,
                }));
                return self
                    .resume(program, None)
                    .map_err(|fault| fault.reason);
            }
        };
        self.top().locals[dest] = value;
        Ok(Flow::Continue)
    }
}
