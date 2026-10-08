//! The default of a type, built part by part with an explicit stack;
//! a meaning with a written `default` row is called for its value.
use super::natives::{Ctx, Step};
use super::walk::{Build, Shape};
use crate::{
    ir,
    value::{structural, Reason, Record, Value},
};
use alloc::{rc::Rc, string::String, vec::Vec};

/// Build the default of a type part by part; a meaning with a written
/// `default` row is called for its value.
/// A finished build's value from its built parts.
fn assemble(build: Build) -> Value {
    let Build { shape, built, .. } = build;
    match shape {
        Shape::Top => built.into_iter().next().expect("one"),
        Shape::Tuple => Value::Tuple(Rc::new(built)),
        Shape::Array(length) => {
            let element = built.into_iter().next().expect("the element");
            Value::seq(alloc::vec![element; length])
        }
        Shape::Record {
            meaning,
            constructor,
        } => Value::Record(Rc::new(Record {
            meaning,
            constructor,
            fields: built,
        })),
    }
}

/// The written `default` row of a meaning: its definition and meanings.
fn written(ctx: &Ctx<'_>, meaning: usize) -> Option<(usize, Vec<ir::Type>)> {
    let member = ctx.natives.members.default?;
    let selected =
        crate::compile::dispatch::select_by_head(ctx.tables, member, meaning)?;
    match &ctx.tables.dispatch[member].rows[selected.row].target {
        ir::Implementation::Definition { index, meanings } => {
            Some((*index, meanings.clone()))
        }
        _ => None,
    }
}

pub(super) fn default_step(
    work: &mut Vec<Build>,
    ctx: &Ctx<'_>,
    returned: Option<Value>,
) -> Step {
    if let Some(value) = returned {
        work.last_mut()
            .expect("a build awaits the value")
            .built
            .push(value);
    }
    loop {
        let Some(build) = work.last_mut() else {
            unreachable!("the top build finishes the walk")
        };
        if build.built.len() == build.parts.len() {
            let finished = work.pop().expect("the build");
            let top = matches!(finished.shape, Shape::Top);
            let value = assemble(finished);
            match work.last_mut() {
                Some(parent) if !top => parent.built.push(value),
                _ => return Step::Done(value),
            }
            continue;
        }
        let ty = build.parts[build.built.len()].clone();
        match &ty {
            ir::Type::Tuple(parts) => work.push(Build {
                shape: Shape::Tuple,
                parts: parts.clone(),
                built: Vec::new(),
            }),
            ir::Type::Array { element, length } => {
                let ir::Type::Number(length) = &**length else {
                    return Step::Fault(Reason::Message(String::from(
                        "an array length is a number",
                    )));
                };
                let length =
                    usize::try_from(length.to_u64().unwrap_or(0)).unwrap_or(0);
                work.push(Build {
                    shape: Shape::Array(length),
                    parts: alloc::vec![(**element).clone()],
                    built: Vec::new(),
                });
            }
            ir::Type::Meaning { index, arguments } => {
                let meaning = &ctx.tables.meanings[*index];
                if let Some((definition, meanings)) = written(ctx, *index) {
                    return Step::Call(super::calls::definition_invoke(
                        ctx.program,
                        ctx.globals,
                        definition,
                        Vec::new(),
                        meanings,
                    ));
                }
                let prelude = ctx.prelude;
                let known = [
                    prelude.integer,
                    prelude.f32,
                    prelude.f64,
                    prelude.string,
                    prelude.vec,
                    prelude.deque,
                    prelude.maybe,
                    prelude.bool,
                ]
                .contains(&Some(*index));
                if known || meaning.constructors.len() != 1 {
                    match structural::default(&ty, ctx.tables, prelude) {
                        Ok(value) => build.built.push(value),
                        Err(reason) => return Step::Fault(reason),
                    }
                    continue;
                }
                let parts = meaning.constructors[0]
                    .fields
                    .iter()
                    .map(|field| {
                        field.ty.substituted(&meaning.parameters, arguments)
                    })
                    .collect();
                work.push(Build {
                    shape: Shape::Record {
                        meaning: *index,
                        constructor: 0,
                    },
                    parts,
                    built: Vec::new(),
                });
            }
            _ => match structural::default(&ty, ctx.tables, ctx.prelude) {
                Ok(value) => build.built.push(value),
                Err(reason) => return Step::Fault(reason),
            },
        }
    }
}
