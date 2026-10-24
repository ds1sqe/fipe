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
fn assemble(mut completed: Build) -> Value {
    let shape = core::mem::replace(&mut completed.shape, Shape::Top);
    let built = core::mem::take(&mut completed.built);
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
fn written(
    ctx: &Ctx<'_>,
    ty: &ir::Type,
) -> Result<Option<super::calls::Invoke>, Reason> {
    let Some(member) = ctx.natives.members.default else {
        return Ok(None);
    };
    let Some(selected) = super::dispatch::select_for(
        ctx.tables,
        ctx.prelude,
        member,
        Some(ty),
        None,
        ctx.work,
    )?
    else {
        return Ok(None);
    };
    super::pending::supplied_target(
        ctx,
        &ctx.tables.dispatch[member].rows[selected.row].target,
        &selected.bindings,
        Vec::new(),
        Some(ty),
    )
}

fn copy_parts<'a>(
    parts: impl Iterator<Item = &'a ir::Type>,
    parameters: &[usize],
    arguments: &[ir::Type],
    ctx: &Ctx<'_>,
) -> Result<Vec<ir::Type>, Reason> {
    let mut copied = crate::value::resource::Forest::default();
    for ty in parts {
        let ordinary = super::resource::Work::new(None);
        let paid = if ctx.tables.has_resources {
            ctx.work
        } else {
            &ordinary
        };
        let mut result = super::resource::materialize(
            ty, parameters, arguments, ctx.tables, paid, &mut Ok,
        )?;
        let ty = result.0.pop().expect("default type");
        copied.0.push(ty);
    }
    Ok(core::mem::take(&mut copied.0))
}

fn meaning_default(
    work: &mut Vec<Build>,
    ctx: &Ctx<'_>,
    ty: &ir::Type,
) -> Option<Step> {
    let ir::Type::Meaning { index, arguments } = ty else {
        unreachable!("nominal default")
    };
    let meaning = &ctx.tables.meanings[*index];
    let written = match written(ctx, ty) {
        Ok(written) => written,
        Err(reason) => return Some(Step::Fault(reason)),
    };
    if let Some(invoke) = written {
        return Some(Step::Call(invoke));
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
        match structural::default(ty, ctx.tables, prelude) {
            Ok(value) => {
                work.last_mut().expect("current build").built.push(value);
            }
            Err(reason) => return Some(Step::Fault(reason)),
        }
        return None;
    }
    if meaning.resource {
        return Some(Step::Fault(Reason::Message(
            "resource has no intrinsic default".into(),
        )));
    }
    let parts = match copy_parts(
        meaning.constructors[0].fields.iter().map(|field| &field.ty),
        &meaning.parameters,
        arguments,
        ctx,
    ) {
        Ok(parts) => parts,
        Err(reason) => return Some(Step::Fault(reason)),
    };
    work.push(Build {
        shape: Shape::Record {
            meaning: *index,
            constructor: 0,
        },
        parts,
        built: Vec::new(),
    });
    None
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
        let ordinary = super::resource::Work::new(None);
        let paid = if ctx.tables.has_resources {
            ctx.work
        } else {
            &ordinary
        };
        let copied = match super::resource::materialize(
            &build.parts[build.built.len()],
            &[],
            &[],
            ctx.tables,
            paid,
            &mut Ok,
        ) {
            Ok(copied) => copied,
            Err(reason) => return Step::Fault(reason),
        };
        let ty = &copied.0[0];
        match ty {
            ir::Type::Tuple(parts) => work.push(Build {
                shape: Shape::Tuple,
                parts: match copy_parts(parts.iter(), &[], &[], ctx) {
                    Ok(parts) => parts,
                    Err(reason) => return Step::Fault(reason),
                },
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
                    parts: match copy_parts(
                        core::iter::once(element.as_ref()),
                        &[],
                        &[],
                        ctx,
                    ) {
                        Ok(parts) => parts,
                        Err(reason) => return Step::Fault(reason),
                    },
                    built: Vec::new(),
                });
            }
            ir::Type::Meaning { .. } => {
                if let Some(step) = meaning_default(work, ctx, ty) {
                    return step;
                }
            }
            _ => match structural::default(ty, ctx.tables, ctx.prelude) {
                Ok(value) => build.built.push(value),
                Err(reason) => return Step::Fault(reason),
            },
        }
    }
}
