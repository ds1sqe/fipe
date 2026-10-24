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
            Rvalue::Use(operand) => self.operand(program, *operand)?,
            Rvalue::Admit { values, spot } => {
                return self
                    .admit_assignment(program, dest, values, *spot, node);
            }
            Rvalue::Construct {
                meaning,
                constructor,
                fields,
            } => Value::Record(Rc::new(Record {
                meaning: *meaning,
                constructor: *constructor,
                fields: self.operands(program, fields)?,
            })),
            Rvalue::Tuple(parts) => {
                Value::Tuple(Rc::new(self.operands(program, parts)?))
            }
            Rvalue::Array(parts) => Value::seq(self.operands(program, parts)?),
            Rvalue::Repeat { value, length } => {
                self.repeat(program, *value, *length)?
            }
            Rvalue::Field(value, index) => {
                self.field_value(program, *value, *index)?
            }
            Rvalue::Length(value) => match self.operand(program, *value)? {
                Value::Seq(parts) => Value::Int(
                    i64::try_from(parts.len()).expect("a length fits"),
                ),
                other => unreachable!("a sequence, not {other:?}"),
            },
            Rvalue::Closure {
                body,
                captures,
                meanings,
            } => self.closure(program, *body, captures, meanings)?,
            Rvalue::Wrap {
                meaning,
                constructor,
                value,
            } => Value::Record(Rc::new(Record {
                meaning: *meaning,
                constructor: *constructor,
                fields: alloc::vec![self.operand(program, *value)?],
            })),
            Rvalue::Pack { tag, value } => {
                self.pack(program, dest, tag, *value)?
            }
            Rvalue::Cast { row, value, .. } => {
                // The target is the local's type, witnesses in place.
                let target = crate::value::resource::Forest(alloc::vec![
                    self.local_type(program, dest)?
                ]);
                let value = self.operand(program, *value)?;
                super::casts::cast(*row, &target.0[0], value, &self.prelude())?
            }
            Rvalue::Convert {
                mode,
                predicate,
                value,
            } => {
                let value = self.operand(program, *value)?;
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

    fn field_value(
        &mut self,
        program: &mir::Program,
        operand: mir::Operand,
        index: usize,
    ) -> Result<Value, Reason> {
        let value = self.operand(program, operand)?;
        if let Value::Packed(packed) = &value {
            let budgeted = program.tables.borrow().has_resources;
            let work = if budgeted {
                self.resource_work()
            } else {
                super::resource::Work::new(None)
            };
            let valid = (|| {
                let tables = program.tables.borrow();
                super::resource::validate_current(&value, &tables, &work)?;
                let source = super::resource::packed_type(
                    packed, false, &tables, &work,
                )?;
                super::fits::source_receiver(
                    packed.value(),
                    &source.0[0],
                    &tables,
                    &self.prelude(),
                    &work,
                )
            })();
            if budgeted {
                self.pay(work.spent())?;
            }
            if !valid? {
                return Err(Reason::Host(
                    "packed source has no valid receiver".into(),
                ));
            }
        }
        Ok(super::step::field(&value, index))
    }

    /// Admit a supplied argument before publishing its private callee tuple.
    fn admit_assignment(
        &mut self,
        program: &mir::Program,
        dest: usize,
        values: &[mir::Operand; 2],
        spot: usize,
        node: usize,
    ) -> Result<Flow, Reason> {
        let callee = self.operand(program, values[0])?;
        let argument = self.operand(program, values[1])?;
        let (tuple, guards) =
            self.admitted_binding(program, callee, spot, argument, node)?;
        if guards.is_empty() {
            self.top().locals[dest] = tuple;
            Ok(Flow::Continue)
        } else {
            let frame = self.top();
            let (block, statement) = (frame.block, frame.statement);
            let Value::Tuple(values) = tuple else {
                unreachable!("admitted binding tuple")
            };
            let values = Rc::try_unwrap(values).map_err(|_| {
                Reason::Host("shared admitted binding tuple".into())
            })?;
            self.frames.push(Frame::Native(NativeFrame {
                node,
                pending: Pending::Arguments(super::pending::Arguments::new(
                    values, guards,
                )),
                back: Return::Into {
                    dest,
                    block,
                    statement,
                },
                result: None,
            }));
            self.resume(program, None).map_err(|fault| fault.reason)
        }
    }

    /// A repeated element, its length written or read from a witness.
    fn repeat(
        &mut self,
        program: &mir::Program,
        value: mir::Operand,
        length: mir::Length,
    ) -> Result<Value, Reason> {
        let value = self.operand(program, value)?;
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
        Ok(Value::seq(alloc::vec![value; length]))
    }

    /// A closure of a body with its captures copied now and its
    /// meanings substituted by this frame.
    fn closure(
        &mut self,
        program: &mir::Program,
        body: usize,
        captures: &[mir::Operand],
        meanings: &[ir::Type],
    ) -> Result<Value, Reason> {
        let captures = self.operands(program, captures)?;
        let mut copied = crate::value::resource::Forest::default();
        let callable = program
            .receiver_contexts
            .get(body)
            .and_then(Option::as_ref)
            .and_then(|context| context.protocol_result.as_ref())
            .map(|result| result.meaning);
        for (position, ty) in meanings.iter().enumerate() {
            copied.0.push(self.substitute_charged(
                program,
                ty,
                callable == Some(position),
            )?);
        }
        let meanings = core::mem::take(&mut copied.0);
        self.receiver_closure(
            program,
            Closure {
                body,
                captures,
                meanings,
                filled: Vec::new(),
                guards: Vec::new(),
            },
        )
    }

    /// Capture the actual source and destination contract in one frame.
    fn pack(
        &mut self,
        program: &mir::Program,
        dest: usize,
        tag: &ir::Type,
        value: mir::Operand,
    ) -> Result<Value, Reason> {
        let mut source = crate::value::resource::Forest(alloc::vec![
            self.substitute(program, tag)?
        ]);
        let mut target = crate::value::resource::Forest(alloc::vec![
            self.local_type(program, dest)?
        ]);
        let value = self.operand(program, value)?;
        let budgeted = program.tables.borrow().has_resources;
        let work = if budgeted {
            self.resource_work()
        } else {
            super::resource::Work::new(None)
        };
        let result = (|| {
            let tables = program.tables.borrow();
            let bindings = super::resource::capture_bindings(
                &[&source.0[0], &target.0[0]],
                &tables,
                &work,
            )?;
            super::resource::validate_current(&value, &tables, &work)?;
            if let Value::Packed(inner) = &value {
                if !super::resource::source_link(
                    &source.0[0],
                    &bindings,
                    inner,
                    &work,
                )? {
                    return Err(Reason::Host(
                        "packed layers do not agree".into(),
                    ));
                }
            }
            work.pay(3)?;
            Ok(Value::Packed(Rc::new(Packed::checked(
                source.0.pop().expect("source"),
                target.0.pop().expect("target"),
                value,
                bindings,
            ))))
        })();
        if budgeted {
            self.pay(work.spent())?;
        }
        result
    }

    /// A native as an rvalue, which computes at once.
    fn native_rvalue(
        &mut self,
        program: &mir::Program,
        dest: usize,
        op: usize,
        args: &[mir::Operand],
    ) -> Result<Value, Reason> {
        let args = self.operands(program, args)?;
        let result = crate::value::resource::Forest(alloc::vec![
            self.local_type(program, dest)?
        ]);
        let natives = self.natives();
        self.pay_native(natives.of(op), &args)?;
        let prelude = self.prelude();
        let tables = program.tables.borrow();
        let work = self.resource_work();
        let ctx = Ctx {
            program,
            tables: &tables,
            prelude: &prelude,
            natives: &natives,
            globals: &self.globals,
            result: result.0.first(),
            work: &work,
        };
        let step = natives.call(op, args, &ctx);
        drop(tables);
        self.pay(work.spent())?;
        match step? {
            Step::Done(value) => Ok(value),
            Step::Fault(reason) => Err(reason),
            Step::Call(_) | Step::Start(_) | Step::AdmitArgument { .. } => {
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
                let ty = crate::value::resource::Forest(alloc::vec![
                    self.local_type(program, dest)?
                ]);
                match &ty.0[0] {
                    ir::Type::Meaning { index, .. }
                        if Some(*index) == prelude.maybe =>
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
                let witnesses =
                    self.predicate_witnesses(program, predicate, false)?;
                let frame = self.top();
                let (block, statement) = (frame.block, frame.statement);
                let snapshot = self.snapshot_predicate(program, predicate)?;
                let run = self.owned_predicate_run(
                    program,
                    snapshot,
                    alloc::vec![value],
                    super::predicates::After::Convert(mode),
                    witnesses,
                )?;
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
