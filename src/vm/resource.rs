//! Frozen Host admission and charged resource type work.
use crate::{
    ir::{self, Type},
    value::{
        resource::{
            Descriptor, Forest, ResourceError, ResourceStep, ResourceType,
        },
        Reason,
    },
};
use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
use core::cell::Cell;

pub(super) struct Work {
    left: Cell<Option<u64>>,
    spent: Cell<u64>,
}
impl Work {
    pub(super) const fn new(left: Option<u64>) -> Self {
        Self {
            left: Cell::new(left),
            spent: Cell::new(0),
        }
    }
    pub(super) fn pay(&self, units: u64) -> Result<(), Reason> {
        if let Some(left) = self.left.get() {
            if left < units {
                self.spent.set(self.spent.get().saturating_add(left));
                self.left.set(Some(0));
                return Err(Reason::Budget);
            }
            self.left.set(Some(left - units));
        }
        self.spent.set(self.spent.get().saturating_add(units));
        Ok(())
    }
    pub(super) const fn spent(&self) -> u64 {
        self.spent.get()
    }
}
#[derive(Clone, Copy)]
enum Shape {
    Meaning(usize),
    Tuple,
    Packed,
    All,
    Sum,
    Array,
    Function,
    Not,
}
enum Copy<'a> {
    Read(&'a Type, bool),
    Finish(Shape, usize),
}

/// Copy/substitute without recursive expansion, keeping every partial tree guarded.
/// The mapper translates nominal indices, permitting frozen local stamp indices.
pub(super) fn materialize(
    ty: &Type,
    parameters: &[usize],
    arguments: &[Type],
    tables: &ir::Tables,
    work: &Work,
    map: &mut impl FnMut(usize) -> Result<usize, Reason>,
) -> Result<Forest, Reason> {
    work.pay(u64::try_from(parameters.len()).unwrap_or(u64::MAX))?;
    let bindings: Vec<_> =
        parameters.iter().copied().zip(arguments.iter()).collect();
    materialize_bound(ty, &bindings, tables, work, true, map)
}
fn materialize_bound<'a>(
    ty: &'a Type,
    bindings: &[(usize, &'a Type)],
    tables: &'a ir::Tables,
    work: &Work,
    sessions: bool,
    map: &mut impl FnMut(usize) -> Result<usize, Reason>,
) -> Result<Forest, Reason> {
    let mut jobs = vec![Copy::Read(ty, true)];
    let mut done = Forest::default();
    while let Some(job) = jobs.pop() {
        work.pay(1)?;
        match job {
            Copy::Read(ty, substitute) => {
                read_type(
                    ty,
                    substitute,
                    (bindings, tables),
                    sessions,
                    work,
                    map,
                    (&mut jobs, &mut done),
                )?;
            }
            Copy::Finish(shape, count) => {
                finish_type(shape, count, &mut done);
            }
        }
    }
    Ok(done)
}

/// The creator's exact parameter associations, retained without owning predicate trees.
pub(super) type Witnesses = Rc<[(usize, Rc<crate::value::Meaning>)]>;

pub(super) fn capture_predicate(
    predicate: &ir::Predicate,
    entry: bool,
    hidden: &[usize],
    meanings: &[Type],
    tables: &ir::Tables,
    work: &Work,
) -> Result<Witnesses, Reason> {
    enum Job<'a> {
        Predicate(&'a ir::Predicate, bool),
        Type(&'a Type),
    }
    work.pay(1)?;
    let mut jobs = vec![Job::Predicate(predicate, entry)];
    let mut captured: Vec<(usize, Rc<crate::value::Meaning>)> = Vec::new();
    while let Some(job) = jobs.pop() {
        work.pay(1)?;
        match job {
            Job::Predicate(predicate, true) => {
                if let ir::Predicate::Function { parameter, result } = predicate
                {
                    work.pay(2)?;
                    jobs.push(Job::Predicate(result, true));
                    jobs.push(Job::Predicate(
                        parameter,
                        matches!(
                            parameter.as_ref(),
                            ir::Predicate::Function { .. }
                        ),
                    ));
                }
            }
            Job::Predicate(predicate, false) => match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    work.pay(u64::try_from(parts.len()).unwrap_or(u64::MAX))?;
                    jobs.extend(
                        parts
                            .iter()
                            .rev()
                            .map(|part| Job::Predicate(part, false)),
                    );
                }
                ir::Predicate::Not(inner) => {
                    work.pay(1)?;
                    jobs.push(Job::Predicate(inner, false));
                }
                ir::Predicate::Test { meanings, .. } => {
                    work.pay(
                        u64::try_from(meanings.len()).unwrap_or(u64::MAX),
                    )?;
                    jobs.extend(meanings.iter().rev().map(Job::Type));
                }
                ir::Predicate::Function { .. }
                | ir::Predicate::Nonzero { .. } => {}
            },
            Job::Type(ty) => match ty {
                Type::Parameter(parameter) => {
                    capture_parameter(
                        *parameter,
                        hidden,
                        meanings,
                        tables,
                        work,
                        &mut captured,
                    )?;
                }
                Type::Meaning { arguments, .. }
                | Type::Tuple(arguments)
                | Type::Packed(arguments)
                | Type::All(arguments)
                | Type::Sum(arguments) => {
                    work.pay(
                        u64::try_from(arguments.len()).unwrap_or(u64::MAX),
                    )?;
                    jobs.extend(arguments.iter().rev().map(Job::Type));
                }
                Type::Array { element, length } => {
                    work.pay(2)?;
                    jobs.push(Job::Type(length));
                    jobs.push(Job::Type(element));
                }
                Type::Function { parameter, result } => {
                    work.pay(2)?;
                    jobs.push(Job::Type(result));
                    jobs.push(Job::Type(parameter));
                }
                Type::Not(inner) => {
                    work.pay(1)?;
                    jobs.push(Job::Type(inner));
                }
                Type::Unit | Type::Session(_) | Type::Number(_) => {}
            },
        }
    }
    work.pay(
        u64::try_from(captured.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    )?;
    Ok(Rc::from(captured))
}

fn capture_parameter(
    parameter: usize,
    hidden: &[usize],
    meanings: &[Type],
    tables: &ir::Tables,
    work: &Work,
    captured: &mut Vec<(usize, Rc<crate::value::Meaning>)>,
) -> Result<(), Reason> {
    work.pay(u64::try_from(captured.len()).unwrap_or(u64::MAX))?;
    if captured.iter().any(|(id, _)| *id == parameter) {
        return Ok(());
    }
    work.pay(u64::try_from(hidden.len()).unwrap_or(u64::MAX))?;
    let actual = hidden
        .iter()
        .position(|id| *id == parameter)
        .and_then(|index| meanings.get(index))
        .ok_or_else(|| Reason::Host("missing predicate witness".into()))?;
    work.pay(1)?;
    let mut copied = materialize(actual, &[], &[], tables, work, &mut Ok)?;
    work.pay(1)?;
    let actual = copied.0.pop().expect("one captured witness");
    captured.push((parameter, Rc::new(crate::value::Meaning::new(actual))));
    Ok(())
}

pub(super) fn predicate_meanings(
    arguments: &[Type],
    witnesses: &Witnesses,
    tables: &ir::Tables,
    work: &Work,
) -> Result<Forest, Reason> {
    work.pay(u64::try_from(arguments.len()).unwrap_or(u64::MAX))?;
    let mut result = Forest::default();
    for argument in arguments {
        work.pay(u64::try_from(witnesses.len()).unwrap_or(u64::MAX))?;
        let bindings: Vec<_> = witnesses
            .iter()
            .map(|(id, meaning)| (*id, meaning.as_type()))
            .collect();
        let mut copied = materialize_bound(
            argument, &bindings, tables, work, true, &mut Ok,
        )?;
        result.0.append(&mut copied.0);
    }
    Ok(result)
}

fn fixed_session<'a>(
    ty: &'a Type,
    tables: &'a ir::Tables,
    work: &Work,
) -> Result<&'a Type, Reason> {
    let mut next = ty;
    let mut seen = Vec::new();
    while let Type::Session(index) = next {
        work.pay(
            u64::try_from(seen.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )?;
        if seen.contains(index) {
            return Err(Reason::Host("nonconcrete resource identity".into()));
        }
        seen.push(*index);
        if let Some(Some(fixed)) = tables.session_meanings.get(*index) {
            next = fixed;
        } else {
            break;
        }
    }
    Ok(next)
}
fn read_type<'a>(
    ty: &'a Type,
    substitute: bool,
    environment: (&[(usize, &'a Type)], &'a ir::Tables),
    sessions: bool,
    work: &Work,
    map: &mut impl FnMut(usize) -> Result<usize, Reason>,
    builders: (&mut Vec<Copy<'a>>, &mut Forest),
) -> Result<(), Reason> {
    let (bindings, tables) = environment;
    let (jobs, done) = builders;
    if substitute {
        if let Type::Parameter(parameter) = ty {
            work.pay(u64::try_from(bindings.len()).unwrap_or(u64::MAX))?;
            if let Some((_, arg)) =
                bindings.iter().find(|(known, _)| known == parameter)
            {
                jobs.push(Copy::Read(arg, false));
                return Ok(());
            }
        }
    }
    if let Type::Session(slot) = ty {
        if !sessions {
            done.0.push(Type::Session(*slot));
            return Ok(());
        }
        let next = fixed_session(ty, tables, work)?;
        if matches!(next, Type::Session(_)) {
            done.0.push(Type::Session(*slot));
        } else {
            jobs.push(Copy::Read(next, substitute));
        }
        return Ok(());
    }
    let arity = match ty {
        Type::Meaning { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Packed(arguments)
        | Type::All(arguments)
        | Type::Sum(arguments) => arguments.len(),
        Type::Array { .. } | Type::Function { .. } => 2,
        Type::Not(_) => 1,
        _ => 0,
    };
    work.pay(u64::try_from(arity).unwrap_or(u64::MAX))?;
    let (shape, parts): (_, Vec<&Type>) = match ty {
        Type::Unit => {
            done.0.push(Type::Unit);
            return Ok(());
        }
        Type::Parameter(p) => {
            done.0.push(Type::Parameter(*p));
            return Ok(());
        }
        Type::Number(n) => {
            work.pay(
                u64::try_from(n.bit_length().div_ceil(64)).unwrap_or(u64::MAX),
            )?;
            done.0.push(Type::Number(n.clone()));
            return Ok(());
        }
        Type::Meaning { index, arguments } => {
            (Shape::Meaning(map(*index)?), arguments.iter().collect())
        }
        Type::Tuple(parts) => (Shape::Tuple, parts.iter().collect()),
        Type::Packed(parts) => (Shape::Packed, parts.iter().collect()),
        Type::All(parts) => (Shape::All, parts.iter().collect()),
        Type::Sum(parts) => (Shape::Sum, parts.iter().collect()),
        Type::Array { element, length } => {
            (Shape::Array, vec![element.as_ref(), length.as_ref()])
        }
        Type::Function { parameter, result } => {
            (Shape::Function, vec![parameter.as_ref(), result.as_ref()])
        }
        Type::Not(inner) => (Shape::Not, vec![inner.as_ref()]),
        Type::Session(_) => unreachable!(),
    };
    jobs.push(Copy::Finish(shape, parts.len()));
    jobs.extend(
        parts
            .into_iter()
            .rev()
            .map(|part| Copy::Read(part, substitute)),
    );
    Ok(())
}
fn finish_type(shape: Shape, count: usize, done: &mut Forest) {
    let start = done.0.len() - count;
    // No fallible operation occurs after children leave their guard.
    let mut parts = done.0.split_off(start);
    let ty = match shape {
        Shape::Meaning(index) => Type::Meaning {
            index,
            arguments: parts,
        },
        Shape::Tuple => Type::Tuple(parts),
        Shape::Packed => Type::Packed(parts),
        Shape::All => Type::All(parts),
        Shape::Sum => Type::Sum(parts),
        Shape::Array => {
            let length = parts.pop().expect("length");
            let element = parts.pop().expect("element");
            Type::Array {
                element: Box::new(element),
                length: Box::new(length),
            }
        }
        Shape::Function => {
            let result = parts.pop().expect("result");
            let parameter = parts.pop().expect("parameter");
            Type::Function {
                parameter: Box::new(parameter),
                result: Box::new(result),
            }
        }
        Shape::Not => Type::Not(Box::new(parts.pop().expect("inner"))),
    };
    done.0.push(ty);
}

use crate::value::{Prelude, Value};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Free {
    Parameter(usize),
    Session(usize),
}
#[derive(Default, Clone, PartialEq, Eq)]
struct Summary {
    resource: bool,
    exposed: BTreeSet<usize>,
    demanded: BTreeSet<usize>,
    unresolved: BTreeSet<Free>,
}
impl Summary {
    fn merge(&mut self, other: &Self, work: &Work) -> Result<(), Reason> {
        work.pay(1)?;
        self.resource |= other.resource;
        for p in &other.exposed {
            work.pay(1)?;
            self.exposed.insert(*p);
        }
        for p in &other.demanded {
            work.pay(1)?;
            self.demanded.insert(*p);
        }
        for p in &other.unresolved {
            work.pay(1)?;
            self.unresolved.insert(*p);
        }
        Ok(())
    }
    fn demand(
        &mut self,
        ty: &Type,
        formals: &[usize],
        work: &Work,
    ) -> Result<(), Reason> {
        let mut nodes = vec![ty];
        while let Some(node) = nodes.pop() {
            work.pay(1)?;
            match node {
                Type::Parameter(p) => {
                    work.pay(u64::try_from(formals.len()).unwrap_or(u64::MAX))?;
                    if formals.contains(p) {
                        self.demanded.insert(*p);
                    } else {
                        self.unresolved.insert(Free::Parameter(*p));
                    }
                }
                Type::Session(p) => {
                    self.unresolved.insert(Free::Session(*p));
                }
                _ => children(node, &mut nodes, work)?,
            }
        }
        Ok(())
    }
}
struct Template {
    parameters: Vec<usize>,
    resource: bool,
    constructors: Vec<Vec<Forest>>,
    stamp: Option<Rc<()>>,
    summary: Summary,
}
pub(super) struct Frozen {
    root: Forest,
    templates: BTreeMap<usize, Template>,
    prelude: Rc<Prelude>,
    label: String,
    direct: Option<ResourceType>,
    ordinary: bool,
    exposes_resources: bool,
    association: Vec<(usize, Rc<()>)>,
    issued: Vec<(usize, Rc<()>)>,
}
impl core::fmt::Debug for Frozen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FrozenExpectation")
    }
}
fn children<'a>(
    ty: &'a Type,
    nodes: &mut Vec<&'a Type>,
    work: &Work,
) -> Result<(), Reason> {
    match ty {
        Type::Meaning { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Packed(arguments)
        | Type::All(arguments)
        | Type::Sum(arguments) => {
            work.pay(u64::try_from(arguments.len()).unwrap_or(u64::MAX))?;
            nodes.extend(arguments);
        }
        Type::Array { element, length } => {
            work.pay(2)?;
            nodes.push(element);
            nodes.push(length);
        }
        Type::Function { parameter, result } => {
            work.pay(2)?;
            nodes.push(parameter);
            nodes.push(result);
        }
        Type::Not(inner) => {
            work.pay(1)?;
            nodes.push(inner);
        }
        Type::Unit
        | Type::Number(_)
        | Type::Parameter(_)
        | Type::Session(_) => {}
    }
    Ok(())
}
fn capture_templates(
    root: &Type,
    tables: &ir::Tables,
    work: &Work,
) -> Result<BTreeMap<usize, Template>, Reason> {
    let mut pending = Vec::new();
    nominals(root, &mut pending, work)?;
    let mut templates = BTreeMap::new();
    while let Some(index) = pending.pop() {
        work.pay(1)?;
        if templates.contains_key(&index) {
            continue;
        }
        let Some(meaning) = tables.meanings.get(index) else {
            return Err(Reason::Host("invalid resource type".into()));
        };
        work.pay(u64::try_from(meaning.parameters.len()).unwrap_or(u64::MAX))?;
        let mut template = Template {
            parameters: meaning.parameters.clone(),
            resource: meaning.resource,
            constructors: Vec::new(),
            stamp: None,
            summary: Summary::default(),
        };
        // Resource payload fields do not describe script-visible values.
        if !meaning.resource {
            for constructor in &meaning.constructors {
                work.pay(1)?;
                let mut fields = Vec::new();
                for field in &constructor.fields {
                    work.pay(1)?;
                    let copied = materialize(
                        &field.ty,
                        &[],
                        &[],
                        tables,
                        work,
                        &mut Ok,
                    )?;
                    nominals(&copied.0[0], &mut pending, work)?;
                    fields.push(copied);
                }
                template.constructors.push(fields);
            }
        }
        templates.insert(index, template);
    }
    Ok(templates)
}

impl Frozen {
    pub(super) fn capture(
        ty: &Type,
        tables: &ir::Tables,
        prelude: Rc<Prelude>,
        work: &Work,
    ) -> Result<Self, Reason> {
        if !tables.has_resources {
            return Ok(Self {
                root: materialize(
                    ty,
                    &[],
                    &[],
                    tables,
                    &Work::new(None),
                    &mut Ok,
                )?,
                templates: BTreeMap::new(),
                prelude,
                label: super::fits::named(ty, tables),
                direct: None,
                ordinary: true,
                exposes_resources: false,
                association: Vec::new(),
                issued: Vec::new(),
            });
        }
        let root = materialize(ty, &[], &[], tables, work, &mut Ok)?;
        let templates = capture_templates(&root.0[0], tables, work)?;
        let label = match &root.0[0] {
            Type::Meaning { index, .. } => tables
                .meanings
                .get(*index)
                .map_or("checked value", |m| m.label.as_str()),
            _ => "checked value",
        };
        work.pay(u64::try_from(label.len()).unwrap_or(u64::MAX))?;
        let mut frozen = Self {
            root,
            templates,
            prelude,
            label: label.into(),
            direct: None,
            ordinary: false,
            exposes_resources: false,
            association: Vec::new(),
            issued: Vec::new(),
        };
        loop {
            let mut changed = false;
            work.pay(
                u64::try_from(frozen.templates.len()).unwrap_or(u64::MAX),
            )?;
            let keys: Vec<_> = frozen.templates.keys().copied().collect();
            for index in keys {
                work.pay(1)?;
                let template = &frozen.templates[&index];
                let mut summary = Summary::default();
                for constructor in &template.constructors {
                    for field in constructor {
                        summary.merge(
                            &frozen.summary(
                                &field.0[0],
                                &template.parameters,
                                work,
                            )?,
                            work,
                        )?;
                    }
                }
                work.pay(
                    u64::try_from(
                        summary.exposed.len()
                            + summary.demanded.len()
                            + summary.unresolved.len()
                            + template.summary.exposed.len()
                            + template.summary.demanded.len()
                            + template.summary.unresolved.len(),
                    )
                    .unwrap_or(u64::MAX),
                )?;
                if summary != template.summary {
                    frozen
                        .templates
                        .get_mut(&index)
                        .expect("template")
                        .summary = summary;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        frozen.finish_capture(tables, work)?;
        Ok(frozen)
    }
    fn finish_capture(
        &mut self,
        tables: &ir::Tables,
        work: &Work,
    ) -> Result<(), Reason> {
        let summary = self.summary(&self.root.0[0], &[], work)?;
        if !summary.unresolved.is_empty() {
            return Err(Reason::Host("nonconcrete resource identity".into()));
        }
        self.exposes_resources = summary.resource;
        let required = self.required_stamps(work)?;
        for index in required {
            work.pay(
                u64::try_from(tables.resource_cache.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            )?;
            self.templates.get_mut(&index).expect("template").stamp =
                Some(tables.resource_cache.stamp(index));
        }
        if let Type::Meaning { index, .. } = &self.root.0[0] {
            if self.templates[index].resource {
                self.direct = Some(self.witness(&self.root.0[0], work)?);
            }
        }
        let association = {
            work.pay(1)?;
            let mut roots = vec![&self.root.0[0]];
            for template in self.templates.values() {
                work.pay(1)?;
                for constructor in &template.constructors {
                    work.pay(1)?;
                    for field in constructor {
                        work.pay(1)?;
                        roots.push(&field.0[0]);
                    }
                }
            }
            capture_bindings(&roots, tables, work)?
        };
        self.association = association;
        let entries = tables.resource_cache.entries();
        work.pay(u64::try_from(entries.len()).unwrap_or(u64::MAX))?;
        self.issued.reserve(entries.len());
        for (index, stamp) in entries.iter() {
            work.pay(1)?;
            self.issued.push((*index, Rc::clone(stamp)));
        }
        drop(entries);
        Ok(())
    }
    fn summary(
        &self,
        ty: &Type,
        formals: &[usize],
        work: &Work,
    ) -> Result<Summary, Reason> {
        enum Job<'a> {
            Read(&'a Type),
            Join(&'a Type, usize),
        }
        let mut jobs = vec![Job::Read(ty)];
        let mut done: Vec<Summary> = Vec::new();
        while let Some(job) = jobs.pop() {
            work.pay(1)?;
            match job {
                Job::Read(node) => {
                    let mut immediate = Summary::default();
                    let descend = match node {
                        Type::Parameter(p) => {
                            work.pay(
                                u64::try_from(formals.len())
                                    .unwrap_or(u64::MAX),
                            )?;
                            if formals.contains(p) {
                                immediate.exposed.insert(*p);
                            }
                            false
                        }
                        Type::Meaning { index, arguments } => {
                            let template = &self.templates[index];
                            if template.resource {
                                immediate.resource = true;
                                for argument in arguments {
                                    immediate
                                        .demand(argument, formals, work)?;
                                }
                                false
                            } else if Some(*index) == self.prelude.vec
                                || Some(*index) == self.prelude.deque
                            {
                                true
                            } else {
                                !template.constructors.is_empty()
                            }
                        }
                        Type::Tuple(_) | Type::Packed(_) => true,
                        Type::Array { length, .. } => {
                            !matches!(length.as_ref(),Type::Number(n) if n.to_u64()==Some(0))
                        }
                        _ => false,
                    };
                    if !descend {
                        done.push(immediate);
                        continue;
                    }
                    let mut parts = Vec::new();
                    match node {
                        Type::Array { element, .. } => {
                            work.pay(1)?;
                            parts.push(element.as_ref());
                        }
                        _ => children(node, &mut parts, work)?,
                    }
                    jobs.push(Job::Join(node, parts.len()));
                    jobs.extend(parts.into_iter().rev().map(Job::Read));
                }
                Job::Join(node, count) => {
                    let values = done.split_off(done.len() - count);
                    let result =
                        self.joined_summary(node, &values, formals, work)?;
                    done.push(result);
                }
            }
        }
        Ok(done.pop().expect("summary"))
    }
    fn joined_summary(
        &self,
        node: &Type,
        values: &[Summary],
        formals: &[usize],
        work: &Work,
    ) -> Result<Summary, Reason> {
        let mut result = Summary::default();
        if let Type::Meaning { index, arguments } = node {
            if Some(*index) == self.prelude.vec
                || Some(*index) == self.prelude.deque
            {
                if let Some(element) = values.first() {
                    result.merge(element, work)?;
                }
            } else {
                let template = &self.templates[index];
                result.resource = template.summary.resource;
                for free in &template.summary.unresolved {
                    work.pay(1)?;
                    result.unresolved.insert(*free);
                }
                for (position, parameter) in
                    template.parameters.iter().enumerate()
                {
                    work.pay(1)?;
                    if template.summary.exposed.contains(parameter) {
                        if let Some(value) = values.get(position) {
                            result.merge(value, work)?;
                        }
                    }
                    if template.summary.demanded.contains(parameter) {
                        if let Some(argument) = arguments.get(position) {
                            result.demand(argument, formals, work)?;
                        }
                    }
                }
            }
        } else {
            for value in values {
                result.merge(value, work)?;
            }
        }
        Ok(result)
    }

    pub(super) fn direct(
        &self,
    ) -> Result<ResourceType, crate::value::resource::ResourceError> {
        self.direct
            .clone()
            .ok_or(crate::value::resource::ResourceError::NotResourceResult)
    }
    fn required_stamps(&self, work: &Work) -> Result<BTreeSet<usize>, Reason> {
        let mut required = BTreeSet::new();
        let mut expanded = BTreeSet::new();
        let mut nodes = vec![(&self.root.0[0], false)];
        while let Some((node, identity)) = nodes.pop() {
            work.pay(1)?;
            if let Type::Meaning { index, arguments } = node {
                let template = &self.templates[index];
                if identity || template.resource {
                    work.pay(
                        u64::try_from(required.len())
                            .unwrap_or(u64::MAX)
                            .saturating_add(1),
                    )?;
                    required.insert(*index);
                    work.pay(
                        u64::try_from(arguments.len()).unwrap_or(u64::MAX),
                    )?;
                    nodes.extend(arguments.iter().map(|arg| (arg, true)));
                } else if Some(*index) == self.prelude.vec
                    || Some(*index) == self.prelude.deque
                {
                    if let Some(element) = arguments.first() {
                        nodes.push((element, false));
                    }
                } else if !template.constructors.is_empty() {
                    for (position, parameter) in
                        template.parameters.iter().enumerate()
                    {
                        work.pay(1)?;
                        if let Some(argument) = arguments.get(position) {
                            if template.summary.demanded.contains(parameter) {
                                nodes.push((argument, true));
                            }
                            if template.summary.exposed.contains(parameter) {
                                nodes.push((argument, false));
                            }
                        }
                    }
                    work.pay(
                        u64::try_from(expanded.len())
                            .unwrap_or(u64::MAX)
                            .saturating_add(1),
                    )?;
                    if expanded.insert(*index) {
                        for constructor in &template.constructors {
                            for field in constructor {
                                work.pay(1)?;
                                nodes.push((&field.0[0], false));
                            }
                        }
                    }
                }
            } else if identity {
                let mut parts = Vec::new();
                children(node, &mut parts, work)?;
                nodes.extend(parts.into_iter().map(|part| (part, true)));
            } else {
                match node {
                    Type::Tuple(parts) | Type::Packed(parts) => {
                        work.pay(
                            u64::try_from(parts.len()).unwrap_or(u64::MAX),
                        )?;
                        nodes.extend(parts.iter().map(|part| (part, false)));
                    }
                    Type::Array { element, length } if !matches!(length.as_ref(),Type::Number(n) if n.to_u64()==Some(0)) =>
                    {
                        nodes.push((element, false));
                    }
                    _ => {}
                }
            }
        }
        Ok(required)
    }
    fn witness(&self, ty: &Type, work: &Work) -> Result<ResourceType, Reason> {
        // Template indices remain frozen; only stamp identity crosses contexts.
        let mut stamps = Vec::new();
        let root = materialize(
            ty,
            &[],
            &[],
            &ir::Tables::default(),
            work,
            &mut |index| {
                let stamp = self
                    .templates
                    .get(&index)
                    .and_then(|template| template.stamp.as_ref())
                    .ok_or_else(|| {
                        Reason::Host("nonconcrete resource identity".into())
                    })?;
                for (local, known) in stamps.iter().enumerate() {
                    work.pay(1)?;
                    if Rc::ptr_eq(known, stamp) {
                        return Ok(local);
                    }
                }
                work.pay(1)?;
                stamps.push(Rc::clone(stamp));
                Ok(stamps.len() - 1)
            },
        )?;
        Ok(ResourceType(Rc::new(Descriptor { root, stamps })))
    }
}
const fn projection_error(reason: &Reason) -> ResourceError {
    if matches!(reason, Reason::Budget) {
        ResourceError::Budget
    } else {
        ResourceError::NonConcrete
    }
}

impl Frozen {
    pub(super) fn project(
        &self,
        path: &[ResourceStep],
        work: &Work,
    ) -> Result<ResourceType, ResourceError> {
        if path.is_empty() {
            return self.direct();
        }
        // Unknown/data-only values cannot expose a resource through these paths.
        if !self.exposes_resources {
            return Err(ResourceError::NotResourceResult);
        }
        let mut selected: Option<Forest> = None;
        for (position, step) in path.iter().enumerate() {
            work.pay(1).map_err(|reason| projection_error(&reason))?;
            let current = selected
                .as_ref()
                .map_or(&self.root.0[0], |forest| &forest.0[0]);
            let next = self.project_step(current, *step, position, work)?;
            selected = Some(next);
        }
        work.pay(1).map_err(|reason| projection_error(&reason))?;
        let selected =
            &selected.as_ref().expect("nonempty path selected a type").0[0];
        let Type::Meaning { index, .. } = selected else {
            return Err(ResourceError::NotResourceResult);
        };
        let template = self
            .templates
            .get(index)
            .ok_or(ResourceError::NonConcrete)?;
        if !template.resource {
            return Err(ResourceError::NotResourceResult);
        }
        if !concrete(selected, work)
            .map_err(|reason| projection_error(&reason))?
        {
            return Err(ResourceError::NonConcrete);
        }
        self.witness(selected, work)
            .map_err(|reason| projection_error(&reason))
    }

    fn project_step(
        &self,
        current: &Type,
        step: ResourceStep,
        position: usize,
        work: &Work,
    ) -> Result<Forest, ResourceError> {
        let invalid = ResourceError::InvalidPath { step: position };
        let empty = ir::Tables::default();
        let (part, parameters, arguments): (&Type, &[usize], &[Type]) = match (
            current, step,
        ) {
            (Type::Tuple(parts), ResourceStep::Tuple(index))
            | (Type::Packed(parts), ResourceStep::Packed(index)) => {
                (parts.get(index).ok_or(invalid)?, &[], &[])
            }
            (Type::Array { element, length }, ResourceStep::Element) if !matches!(length.as_ref(), Type::Number(n) if n.to_u64() == Some(0)) => {
                (element, &[], &[])
            }
            (Type::Meaning { index, arguments }, ResourceStep::Element)
                if Some(*index) == self.prelude.vec
                    || Some(*index) == self.prelude.deque =>
            {
                (arguments.first().ok_or(invalid)?, &[], &[])
            }
            (
                Type::Meaning { index, arguments },
                ResourceStep::Field { constructor, field },
            ) => {
                let template = self
                    .templates
                    .get(index)
                    .ok_or(ResourceError::NonConcrete)?;
                if template.resource {
                    return Err(invalid);
                }
                let field = template
                    .constructors
                    .get(constructor)
                    .and_then(|fields| fields.get(field))
                    .ok_or(invalid)?;
                (&field.0[0], &template.parameters, arguments)
            }
            _ => return Err(invalid),
        };
        materialize(part, parameters, arguments, &empty, work, &mut Ok)
            .map_err(|reason| projection_error(&reason))
    }
}

fn nominals(
    ty: &Type,
    pending: &mut Vec<usize>,
    work: &Work,
) -> Result<(), Reason> {
    let mut nodes = vec![ty];
    while let Some(node) = nodes.pop() {
        work.pay(1)?;
        if let Type::Meaning { index, .. } = node {
            pending.push(*index);
        }
        children(node, &mut nodes, work)?;
    }
    Ok(())
}

/// Hidden references are forbidden in data-only admission. Shared immutable DAG
/// allocations are visited once, including captures and partial applications.
pub(super) fn data_only(value: &Value, work: &Work) -> Result<bool, Reason> {
    crate::value::resource::data_only(value, &mut |units| work.pay(units))
}

pub(super) fn allocation(value: &Value) -> Option<(u8, usize)> {
    Some(match value {
        Value::Seq(x) => (0, Rc::as_ptr(x) as usize),
        Value::Tuple(x) => (1, Rc::as_ptr(x) as usize),
        Value::Record(x) => (2, Rc::as_ptr(x) as usize),
        Value::Packed(x) => (3, Rc::as_ptr(x) as usize),
        Value::Function(x) => (4, Rc::as_ptr(x) as usize),
        _ => return None,
    })
}
fn equal(
    a: &Type,
    b: &Type,
    work: &Work,
    nominal: &mut impl FnMut(usize, usize) -> bool,
) -> Result<bool, Reason> {
    let mut nodes = vec![(a, b)];
    while let Some((a, b)) = nodes.pop() {
        work.pay(1)?;
        match (a, b) {
            (
                Type::Meaning {
                    index: i,
                    arguments: a,
                },
                Type::Meaning {
                    index: j,
                    arguments: b,
                },
            ) => {
                if !nominal(*i, *j) || a.len() != b.len() {
                    return Ok(false);
                }
                work.pay(u64::try_from(a.len()).unwrap_or(u64::MAX))?;
                nodes.extend(a.iter().zip(b));
            }
            (Type::Tuple(a), Type::Tuple(b))
            | (Type::Packed(a), Type::Packed(b))
            | (Type::All(a), Type::All(b))
            | (Type::Sum(a), Type::Sum(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                work.pay(u64::try_from(a.len()).unwrap_or(u64::MAX))?;
                nodes.extend(a.iter().zip(b));
            }
            (
                Type::Array {
                    element: a,
                    length: n,
                },
                Type::Array {
                    element: b,
                    length: m,
                },
            )
            | (
                Type::Function {
                    parameter: a,
                    result: n,
                },
                Type::Function {
                    parameter: b,
                    result: m,
                },
            ) => {
                nodes.push((a, b));
                nodes.push((n, m));
            }
            (Type::Not(a), Type::Not(b)) => nodes.push((a, b)),
            (Type::Unit, Type::Unit) => {}
            (Type::Parameter(a), Type::Parameter(b))
            | (Type::Session(a), Type::Session(b))
                if a == b => {}
            (Type::Number(a), Type::Number(b)) => {
                work.pay(
                    u64::try_from(
                        a.bit_length().max(b.bit_length()).div_ceil(64),
                    )
                    .unwrap_or(u64::MAX),
                )?;
                if a != b {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}
impl Frozen {
    pub(super) fn mismatch(&self) -> Reason {
        self.misfit("reply")
    }
    pub(super) fn input_mismatch(&self) -> Reason {
        self.misfit("input")
    }
    fn misfit(&self, subject: &str) -> Reason {
        if self.ordinary {
            return Reason::Host(alloc::format!(
                "the {subject} is not {}",
                self.label
            ));
        }
        let article = if self
            .label
            .chars()
            .next()
            .is_some_and(|c| "AEIOUaeiou".contains(c))
        {
            "an"
        } else {
            "a"
        };
        Reason::Host(alloc::format!(
            "the {subject} is not {article} {}",
            self.label
        ))
    }
    fn fits_nominal<'v>(
        &self,
        ty: &Type,
        value: &'v Value,
        pending: &mut Vec<(&'v Value, Forest)>,
        work: &Work,
        empty: &ir::Tables,
    ) -> Result<bool, Reason> {
        let Type::Meaning { index, arguments } = ty else {
            unreachable!("nominal admission")
        };
        let template = &self.templates[index];
        if template.resource {
            let Value::Resource(reference) = value else {
                return Ok(false);
            };
            let nested;
            let expected = if let Some(direct) = &self.direct {
                direct
            } else {
                nested = self.witness(ty, work)?;
                &nested
            };
            let a = &expected.0;
            let b = &reference.resource_type().0;
            if !equal(&a.root.0[0], &b.root.0[0], work, &mut |i, j| {
                Rc::ptr_eq(&a.stamps[i], &b.stamps[j])
            })? {
                return Ok(false);
            }
        } else if Some(*index) == self.prelude.integer {
            if !matches!(value, Value::Int(_) | Value::Big(_)) {
                return Ok(false);
            }
        } else if Some(*index) == self.prelude.f32 {
            if !matches!(value, Value::F32(_)) {
                return Ok(false);
            }
        } else if Some(*index) == self.prelude.f64 {
            if !matches!(value, Value::F64(_)) {
                return Ok(false);
            }
        } else if Some(*index) == self.prelude.string {
            if !matches!(value, Value::Text(_)) {
                return Ok(false);
            }
        } else if Some(*index) == self.prelude.vec
            || Some(*index) == self.prelude.deque
        {
            let Value::Seq(values) = value else {
                return Ok(false);
            };
            if let Some(element) = arguments.first() {
                for value in values.iter() {
                    pending.push((
                        value,
                        materialize(element, &[], &[], empty, work, &mut Ok)?,
                    ));
                }
            } else if !data_only(value, work)? {
                return Ok(false);
            }
        } else if template.constructors.is_empty() {
            if !data_only(value, work)? {
                let Value::Packed(packed) = value else {
                    return Ok(false);
                };
                return self.associated(ty, packed, work);
            }
        } else {
            let Value::Record(record) = value else {
                return Ok(false);
            };
            if record.meaning != *index {
                return Ok(false);
            }
            let Some(fields) = template.constructors.get(record.constructor)
            else {
                return Ok(false);
            };
            if fields.len() != record.fields.len() {
                return Ok(false);
            }
            for (field, value) in fields.iter().zip(&record.fields) {
                pending.push((
                    value,
                    materialize(
                        &field.0[0],
                        &template.parameters,
                        arguments,
                        empty,
                        work,
                        &mut Ok,
                    )?,
                ));
            }
        }
        Ok(true)
    }

    pub(super) fn fits(
        &self,
        value: &Value,
        tables: &ir::Tables,
        work: &Work,
    ) -> Result<bool, Reason> {
        if self.ordinary {
            return super::fits::fits(
                value,
                &self.root.0[0],
                tables,
                &self.prelude,
                work,
            );
        }
        let empty = ir::Tables::default();
        let root =
            materialize(&self.root.0[0], &[], &[], &empty, work, &mut Ok)?;
        let mut pending = vec![(value, root)];
        let mut seen: Vec<((u8, usize), Forest)> = Vec::new();
        while let Some((value, ty)) = pending.pop() {
            work.pay(1)?;
            let ty = &ty.0[0];
            if let Some(identity) = allocation(value) {
                let mut known = false;
                for (prior, expected) in &seen {
                    work.pay(1)?;
                    if *prior == identity
                        && equal(ty, &expected.0[0], work, &mut |a, b| a == b)?
                    {
                        known = true;
                        break;
                    }
                }
                if known {
                    continue;
                }
                seen.push((
                    identity,
                    materialize(ty, &[], &[], &empty, work, &mut Ok)?,
                ));
            }
            let mut queue = |value, part: &Type| -> Result<(), Reason> {
                pending.push((
                    value,
                    materialize(part, &[], &[], &empty, work, &mut Ok)?,
                ));
                Ok(())
            };
            match (ty, value) {
                (Type::Meaning { .. }, value) => {
                    if !self.fits_nominal(
                        ty,
                        value,
                        &mut pending,
                        work,
                        &empty,
                    )? {
                        return Ok(false);
                    }
                }
                (Type::Unit, Value::Unit)
                | (Type::Function { .. }, Value::Function(_)) => {}
                (Type::Tuple(parts), Value::Tuple(values)) => {
                    if parts.len() != values.len() {
                        return Ok(false);
                    }
                    for (part, value) in parts.iter().zip(values.iter()) {
                        queue(value, part)?;
                    }
                }
                (Type::Array { element, length }, Value::Seq(values)) => {
                    if let Type::Number(length) = length.as_ref() {
                        if length.to_u64() != u64::try_from(values.len()).ok() {
                            return Ok(false);
                        }
                    }
                    for value in values.iter() {
                        queue(value, element)?;
                    }
                }
                (Type::Packed(parts), Value::Packed(packed)) => {
                    let Some(part) =
                        self.packed_part(parts, packed, value, work)?
                    else {
                        return Ok(false);
                    };
                    queue(packed.value(), part)?;
                }
                (
                    Type::Parameter(_)
                    | Type::Session(_)
                    | Type::Number(_)
                    | Type::All(_)
                    | Type::Not(_)
                    | Type::Sum(_),
                    _,
                ) => {
                    if !data_only(value, work)? {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }
}
impl Frozen {
    fn packed_part<'a>(
        &self,
        parts: &'a [Type],
        packed: &crate::value::Packed,
        value: &Value,
        work: &Work,
    ) -> Result<Option<&'a Type>, Reason> {
        let mut found = None;
        for part in parts {
            let same = if let Some(proof) = packed.association() {
                equal_bindings(
                    part,
                    &self.association,
                    packed.source_type(),
                    &proof.bindings,
                    work,
                )?
            } else {
                equal_current(part, packed.source_type(), work)?
            };
            if same {
                found = Some(part);
                break;
            }
        }
        let Some(part) = found else { return Ok(None) };
        if !data_only(value, work)? && !self.issued_value(value, work)? {
            return Ok(None);
        }
        Ok(Some(part))
    }
}

pub(super) fn equal_current(
    a: &Type,
    b: &Type,
    work: &Work,
) -> Result<bool, Reason> {
    equal(a, b, work, &mut |a, b| a == b)
}

pub(super) fn concrete(ty: &Type, work: &Work) -> Result<bool, Reason> {
    let mut nodes = vec![ty];
    while let Some(node) = nodes.pop() {
        work.pay(1)?;
        if matches!(node, Type::Parameter(_) | Type::Session(_)) {
            return Ok(false);
        }
        children(node, &mut nodes, work)?;
    }
    Ok(true)
}

/// A runtime selection owns guarded copied bindings until invocation takes them.
pub(super) struct Selection(pub(super) crate::compile::dispatch::Selected);
impl core::ops::Deref for Selection {
    type Target = crate::compile::dispatch::Selected;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for Selection {
    fn drop(&mut self) {
        let mut forest = Forest::default();
        for (_, ty) in self.0.bindings.drain(..) {
            forest.0.push(ty);
        }
    }
}

fn matches<'a>(
    pattern: &Type,
    actual: &'a Type,
    bindings: &mut Vec<(usize, &'a Type)>,
    work: &Work,
) -> Result<bool, Reason> {
    let mut nodes = vec![(pattern, actual)];
    while let Some((pattern, actual)) = nodes.pop() {
        work.pay(1)?;
        match (pattern, actual) {
            (Type::Parameter(parameter), actual) => {
                let mut bound = None;
                for (known, value) in bindings.iter() {
                    work.pay(1)?;
                    if known == parameter {
                        bound = Some(*value);
                        break;
                    }
                }
                if let Some(bound) = bound {
                    if !equal(bound, actual, work, &mut |a, b| a == b)? {
                        return Ok(false);
                    }
                } else {
                    work.pay(1)?;
                    bindings.push((*parameter, actual));
                }
            }
            (
                Type::Meaning {
                    index: a,
                    arguments: x,
                },
                Type::Meaning {
                    index: b,
                    arguments: y,
                },
            ) => {
                if a != b || x.len() != y.len() {
                    return Ok(false);
                }
                work.pay(u64::try_from(x.len()).unwrap_or(u64::MAX))?;
                nodes.extend(x.iter().zip(y));
            }
            (Type::Tuple(x), Type::Tuple(y))
            | (Type::Packed(x), Type::Packed(y))
            | (Type::All(x), Type::All(y))
            | (Type::Sum(x), Type::Sum(y)) => {
                if x.len() != y.len() {
                    return Ok(false);
                }
                work.pay(u64::try_from(x.len()).unwrap_or(u64::MAX))?;
                nodes.extend(x.iter().zip(y));
            }
            (
                Type::Array {
                    element: a,
                    length: n,
                },
                Type::Array {
                    element: b,
                    length: m,
                },
            )
            | (
                Type::Function {
                    parameter: a,
                    result: n,
                },
                Type::Function {
                    parameter: b,
                    result: m,
                },
            ) => {
                nodes.push((a, b));
                nodes.push((n, m));
            }
            (Type::Not(a), Type::Not(b)) => nodes.push((a, b)),
            (Type::Unit, Type::Unit) => {}
            (Type::Number(a), Type::Number(b)) => {
                work.pay(
                    u64::try_from(
                        a.bit_length().max(b.bit_length()).div_ceil(64),
                    )
                    .unwrap_or(u64::MAX),
                )?;
                if a != b {
                    return Ok(false);
                }
            }
            (Type::Session(a), Type::Session(b)) if a == b => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}
pub(super) fn select(
    tables: &ir::Tables,
    member: usize,
    receiver: &Type,
    argument: Option<&Type>,
    work: &Work,
) -> Result<Option<Selection>, Reason> {
    let mut found = None;
    for (index, row) in tables.dispatch[member].rows.iter().enumerate() {
        work.pay(1)?;
        let mut bindings = Vec::new();
        if !matches(&row.receiver, receiver, &mut bindings, work)? {
            continue;
        }
        if let (Some(pattern), Some(actual)) = (&row.argument, argument) {
            if !matches(pattern, actual, &mut bindings, work)? {
                continue;
            }
        }
        let mut selected = Selection(crate::compile::dispatch::Selected {
            row: index,
            bindings: Vec::new(),
        });
        for (parameter, ty) in bindings {
            let mut copied = materialize(ty, &[], &[], tables, work, &mut Ok)?;
            selected
                .0
                .bindings
                .push((parameter, copied.0.pop().expect("binding")));
        }
        if matches!(row.target, ir::Implementation::Field(_)) {
            return Ok(Some(selected));
        }
        if found.is_none() {
            found = Some(selected);
        }
    }
    Ok(found)
}
pub(super) fn current(
    ty: &ResourceType,
    tables: &ir::Tables,
    work: &Work,
) -> Result<Forest, Reason> {
    materialize(&ty.0.root.0[0], &[], &[], tables, work, &mut |index| {
        work.pay(
            u64::try_from(tables.resource_cache.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )?;
        tables
            .resource_cache
            .index(&ty.0.stamps[index])
            .ok_or_else(|| {
                Reason::Host(
                    "resource type belongs to a retired or different context"
                        .into(),
                )
            })
    })
}

pub(super) fn substitute_bindings(
    ty: &Type,
    bindings: &[(usize, Type)],
    tables: &ir::Tables,
    work: &Work,
) -> Result<Type, Reason> {
    let ordinary = Work::new(None);
    let work = if tables.has_resources {
        work
    } else {
        &ordinary
    };
    work.pay(u64::try_from(bindings.len()).unwrap_or(u64::MAX))?;
    let borrowed: Vec<_> = bindings
        .iter()
        .map(|(parameter, ty)| (*parameter, ty))
        .collect();
    let mut copied =
        materialize_bound(ty, &borrowed, tables, work, true, &mut Ok)?;
    Ok(copied.0.pop().expect("substitution"))
}

pub(super) fn copy_guarded_closure(
    closure: &crate::value::Closure,
    program: &crate::mir::Program,
    tables: &ir::Tables,
    work: &Work,
) -> Result<crate::value::Closure, Reason> {
    copy_closure_mode(closure, program, tables, work, true)
}
fn copy_closure_mode(
    closure: &crate::value::Closure,
    program: &crate::mir::Program,
    tables: &ir::Tables,
    work: &Work,
    guarded: bool,
) -> Result<crate::value::Closure, Reason> {
    let ordinary = Work::new(None);
    if guarded {
        work.pay(1)?;
        work.pay(u64::try_from(closure.captures.len()).unwrap_or(u64::MAX))?;
        work.pay(u64::try_from(closure.filled.len()).unwrap_or(u64::MAX))?;
    }
    let callable = program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.protocol_result.as_ref())
        .map(|result| result.meaning);
    let mut meanings = Forest::default();
    for (position, ty) in closure.meanings.iter().enumerate() {
        let charged =
            guarded || tables.has_resources || callable == Some(position);
        let mut copied = materialize(
            ty,
            &[],
            &[],
            tables,
            if charged { work } else { &ordinary },
            &mut Ok,
        )?;
        meanings.0.push(copied.0.pop().expect("closure witness"));
    }
    // Keep all type roots guarded until the last fallible guard copy succeeds.
    let guards = copy_guards(&closure.guards, tables, work)?;
    Ok(crate::value::Closure {
        body: closure.body,
        captures: closure.captures.clone(),
        meanings: core::mem::take(&mut meanings.0),
        filled: closure.filled.clone(),
        guards,
    })
}

/// Capture raw-context syntax bindings without expanding layouts or demanding free slots.
pub(super) fn capture_bindings(
    roots: &[&Type],
    tables: &ir::Tables,
    work: &Work,
) -> Result<Vec<(usize, Rc<()>)>, Reason> {
    work.pay(u64::try_from(roots.len()).unwrap_or(u64::MAX))?;
    let mut pending = roots.to_vec();
    let mut bindings: Vec<(usize, Rc<()>)> = Vec::new();
    while let Some(ty) = pending.pop() {
        work.pay(1)?;
        if let Type::Meaning { index, .. } = ty {
            if tables.meanings.get(*index).is_none() {
                return Err(Reason::Host("invalid packed type".into()));
            }
            work.pay(u64::try_from(bindings.len()).unwrap_or(u64::MAX))?;
            if !bindings.iter().any(|(known, _)| known == index) {
                work.pay(
                    u64::try_from(tables.resource_cache.len())
                        .unwrap_or(u64::MAX)
                        .saturating_add(1),
                )?;
                bindings.push((*index, tables.resource_cache.stamp(*index)));
            }
        }
        children(ty, &mut pending, work)?;
    }
    Ok(bindings)
}
fn equal_bindings(
    a: &Type,
    a_bindings: &[(usize, Rc<()>)],
    b: &Type,
    b_bindings: &[(usize, Rc<()>)],
    work: &Work,
) -> Result<bool, Reason> {
    let mut failure = None;
    let same = equal(a, b, work, &mut |a, b| {
        if let Err(reason) = work.pay(
            u64::try_from(a_bindings.len() + b_bindings.len())
                .unwrap_or(u64::MAX),
        ) {
            failure = Some(reason);
            return false;
        }
        match (
            a_bindings.iter().find(|(i, _)| *i == a),
            b_bindings.iter().find(|(i, _)| *i == b),
        ) {
            (Some((_, a)), Some((_, b))) => Rc::ptr_eq(a, b),
            _ => false,
        }
    })?;
    if let Some(reason) = failure {
        return Err(reason);
    }
    Ok(same)
}
pub(super) fn packed_type(
    packed: &crate::value::Packed,
    target: bool,
    tables: &ir::Tables,
    work: &Work,
) -> Result<Forest, Reason> {
    let ty = if target {
        packed.target_type()
    } else {
        packed.source_type()
    };
    let proof = if data_only(packed.value(), work)? {
        None
    } else {
        packed.association()
    };
    materialize(ty, &[], &[], &ir::Tables::default(), work, &mut |index| {
        let current = if let Some(proof) = proof {
            work.pay(
                u64::try_from(
                    proof.bindings.len() + tables.resource_cache.len(),
                )
                .unwrap_or(u64::MAX),
            )?;
            let stamp = proof
                .bindings
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, stamp)| stamp)
                .ok_or_else(|| {
                    Reason::Host("missing packed identity".into())
                })?;
            tables.resource_cache.index(stamp).ok_or_else(|| {
                Reason::Host(
                    "packed type belongs to a retired or different context"
                        .into(),
                )
            })?
        } else {
            index
        };
        if tables.meanings.get(current).is_none() {
            return Err(Reason::Host("invalid packed type".into()));
        }
        Ok(current)
    })
}
pub(super) fn packed_link(
    outer: &crate::value::Packed,
    inner: &crate::value::Packed,
    work: &Work,
) -> Result<bool, Reason> {
    if data_only(outer.value(), work)? {
        return equal_current(outer.source_type(), inner.target_type(), work);
    }
    match (outer.association(), inner.association()) {
        (Some(a), Some(b)) => equal_bindings(
            outer.source_type(),
            &a.bindings,
            inner.target_type(),
            &b.bindings,
            work,
        ),
        _ => equal_current(outer.source_type(), inner.target_type(), work),
    }
}
pub(super) fn validate_current(
    value: &Value,
    tables: &ir::Tables,
    work: &Work,
) -> Result<(), Reason> {
    if data_only(value, work)? {
        return Ok(());
    }
    let mut pending = vec![value];
    let mut seen = BTreeSet::new();
    while let Some(value) = pending.pop() {
        work.pay(1)?;
        if allocation(value).is_some_and(|key| !seen.insert(key)) {
            continue;
        }
        match value {
            Value::Resource(reference) => {
                drop(current(reference.resource_type(), tables, work)?);
            }
            Value::Packed(packed) => {
                if packed.association().is_some() {
                    drop(packed_type(packed, false, tables, work)?);
                    drop(packed_type(packed, true, tables, work)?);
                }
                if let Value::Packed(inner) = packed.value() {
                    if !packed_link(packed, inner, work)? {
                        return Err(Reason::Host(
                            "packed layers do not agree".into(),
                        ));
                    }
                }
                pending.push(packed.value());
            }
            Value::Seq(x) | Value::Tuple(x) => {
                work.pay(u64::try_from(x.len()).unwrap_or(u64::MAX))?;
                pending.extend(x.iter());
            }
            Value::Record(x) => {
                work.pay(u64::try_from(x.fields.len()).unwrap_or(u64::MAX))?;
                pending.extend(&x.fields);
            }
            Value::Function(x) => {
                work.pay(
                    u64::try_from(x.captures.len() + x.filled.len())
                        .unwrap_or(u64::MAX),
                )?;
                pending.extend(&x.captures);
                pending.extend(&x.filled);
            }
            _ => {}
        }
    }
    Ok(())
}
impl Frozen {
    fn issued_stamp(
        &self,
        stamp: &Rc<()>,
        work: &Work,
    ) -> Result<bool, Reason> {
        for (_, known) in &self.issued {
            work.pay(1)?;
            if Rc::ptr_eq(stamp, known) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn associated(
        &self,
        expected: &Type,
        packed: &crate::value::Packed,
        work: &Work,
    ) -> Result<bool, Reason> {
        let Some(proof) = packed.association() else {
            return Ok(false);
        };
        if !equal_bindings(
            expected,
            &self.association,
            packed.target_type(),
            &proof.bindings,
            work,
        )? {
            return Ok(false);
        }
        if let Value::Packed(inner) = packed.value() {
            if !packed_link(packed, inner, work)? {
                return Ok(false);
            }
        }
        self.issued_value(packed.value(), work).and_then(|valid| {
            if !valid {
                return Ok(false);
            }
            for (_, stamp) in &proof.bindings {
                if !self.issued_stamp(stamp, work)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }
    fn issued_value(&self, value: &Value, work: &Work) -> Result<bool, Reason> {
        let mut pending = vec![value];
        let mut seen = BTreeSet::new();
        while let Some(value) = pending.pop() {
            work.pay(1)?;
            if allocation(value).is_some_and(|key| !seen.insert(key)) {
                continue;
            }
            match value {
                Value::Resource(reference) => {
                    for stamp in &reference.resource_type().0.stamps {
                        if !self.issued_stamp(stamp, work)? {
                            return Ok(false);
                        }
                    }
                }
                Value::Packed(packed) => {
                    let Some(proof) = packed.association() else {
                        if !data_only(value, work)? {
                            return Ok(false);
                        }
                        continue;
                    };
                    for (_, stamp) in &proof.bindings {
                        if !self.issued_stamp(stamp, work)? {
                            return Ok(false);
                        }
                    }
                    if let Value::Packed(inner) = packed.value() {
                        if !packed_link(packed, inner, work)? {
                            return Ok(false);
                        }
                    }
                    pending.push(packed.value());
                }
                Value::Seq(x) | Value::Tuple(x) => {
                    work.pay(u64::try_from(x.len()).unwrap_or(u64::MAX))?;
                    pending.extend(x.iter());
                }
                Value::Record(x) => {
                    work.pay(
                        u64::try_from(x.fields.len()).unwrap_or(u64::MAX),
                    )?;
                    pending.extend(&x.fields);
                }
                Value::Function(x) => {
                    work.pay(
                        u64::try_from(x.captures.len() + x.filled.len())
                            .unwrap_or(u64::MAX),
                    )?;
                    pending.extend(&x.captures);
                    pending.extend(&x.filled);
                }
                _ => {}
            }
        }
        Ok(true)
    }
}

pub(super) fn source_link(
    source: &Type,
    bindings: &[(usize, Rc<()>)],
    inner: &crate::value::Packed,
    work: &Work,
) -> Result<bool, Reason> {
    if data_only(inner.value(), work)? {
        return equal_current(source, inner.target_type(), work);
    }
    let Some(proof) = inner.association() else {
        return Ok(false);
    };
    equal_bindings(source, bindings, inner.target_type(), &proof.bindings, work)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    // These bytes are the admitted original generic Sorted runtime fixture.
    const CAPTURE_SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Sorted<a> -> Integer = xs => host.after ()\napply = f x => f x\ngo = xs => apply keep xs\ngo (host.items ())\n";

    fn capture_test_parameters(
        predicate: &ir::Predicate,
        test: usize,
    ) -> Vec<usize> {
        let mut predicates = vec![predicate];
        let mut types = Vec::new();
        while let Some(predicate) = predicates.pop() {
            match predicate {
                ir::Predicate::Test { index, meanings } if *index == test => {
                    types.extend(meanings);
                }
                ir::Predicate::Function { parameter, result } => {
                    predicates.extend([parameter.as_ref(), result.as_ref()]);
                }
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    predicates.extend(parts);
                }
                ir::Predicate::Not(inner) => predicates.push(inner),
                _ => {}
            }
        }
        let mut parameters = Vec::new();
        while let Some(ty) = types.pop() {
            match ty {
                Type::Parameter(id) => parameters.push(*id),
                Type::Meaning { arguments, .. }
                | Type::Tuple(arguments)
                | Type::Packed(arguments)
                | Type::All(arguments)
                | Type::Sum(arguments) => types.extend(arguments),
                Type::Array { element, length } => {
                    types.extend([element.as_ref(), length.as_ref()]);
                }
                Type::Function { parameter, result } => {
                    types.extend([parameter.as_ref(), result.as_ref()]);
                }
                Type::Not(inner) => types.push(inner),
                Type::Unit | Type::Session(_) | Type::Number(_) => {}
            }
        }
        parameters.sort_unstable();
        parameters.dedup();
        parameters
    }

    fn checked_capture_template(
    ) -> (crate::mir::Program, ir::Predicate, Vec<usize>, Vec<usize>) {
        let mut host = crate::decl::Declarations::new();
        host.module(
            "host",
            "pub items: () -> Vec<Integer>\npub after: () -> Integer\n",
        )
        .unwrap();
        let module =
            crate::parse::parse(crate::lex::lex(CAPTURE_SOURCE)).unwrap();
        let unit = crate::check::check(
            &module,
            &host,
            &[],
            crate::check::Options::default(),
        )
        .unwrap();
        let program = crate::compile::compile(&unit);
        let tables = program.tables.borrow();
        let tests: Vec<_> = tables
            .tests
            .iter()
            .enumerate()
            .filter_map(|(index, test)| {
                (tables.namespaces[test.namespace] == "main"
                    && CAPTURE_SOURCE.get(test.span.start..test.span.end)
                        == Some("is_sorted self"))
                .then_some(index)
            })
            .collect();
        assert_eq!(tests.len(), 1, "actual source-owned Sorted test");
        let test = tests[0];
        assert!(
            program.tests[test].is_some(),
            "semantic test has an emitted body"
        );
        let mut found = Vec::new();
        for body in &program.bodies {
            for block in &body.blocks {
                for (statement_index, statement) in
                    block.statements.iter().enumerate()
                {
                    let crate::mir::Statement::Enter { entry, .. } = statement
                    else {
                        continue;
                    };
                    let predicate = &tables.entries[*entry].predicate;
                    if !matches!(predicate, ir::Predicate::Function { .. }) {
                        continue;
                    }
                    let ids = capture_test_parameters(predicate, test);
                    if !ids.is_empty()
                        && ids.iter().all(|id| body.hidden.contains(id))
                    {
                        found.push((
                            predicate.clone(),
                            body.hidden.clone(),
                            ids,
                            body.node,
                            block.nodes[statement_index],
                        ));
                    }
                }
            }
        }
        assert_eq!(
            found.len(),
            1,
            "one actual creator Enter for the source-owned Sorted test"
        );
        let (predicate, hidden, ids, body_node, enter_node) =
            found.pop().unwrap();
        let creator = &tables.nodes[body_node];
        assert_eq!(tables.namespaces[creator.namespace], "main");
        assert_eq!(
            CAPTURE_SOURCE.get(creator.span.start..creator.span.end),
            Some("xs => apply keep xs")
        );
        let site = &tables.nodes[enter_node];
        assert_eq!(tables.namespaces[site.namespace], "main");
        let expected_start =
            CAPTURE_SOURCE.find("apply keep xs").unwrap() + "apply ".len();
        assert_eq!(site.span.start, expected_start);
        assert_eq!(site.span.end, expected_start + "keep".len());
        assert_eq!(
            CAPTURE_SOURCE.get(site.span.start..site.span.end),
            Some("keep")
        );
        drop(tables);
        (program, predicate, hidden, ids)
    }

    #[test]
    fn checked_predicate_capture_rejects_missing_creator_bindings() {
        let (program, predicate, hidden, ids) = checked_capture_template();
        let tables = program.tables.borrow();
        let integers: Vec<_> = tables
            .meanings
            .iter()
            .enumerate()
            .filter_map(|(index, meaning)| {
                (meaning.label == "Integer"
                    && tables.namespaces[meaning.namespace] == "prelude")
                    .then_some(index)
            })
            .collect();
        assert_eq!(
            integers.len(),
            1,
            "unique Integer meaning in unchanged fixture"
        );
        let integer = Type::Meaning {
            index: integers[0],
            arguments: vec![],
        };
        let meanings = vec![integer; hidden.len()];
        let work = Work::new(None);
        let captured = capture_predicate(
            &predicate, true, &hidden, &meanings, &tables, &work,
        )
        .unwrap();
        let mut actual: Vec<_> = captured.iter().map(|(id, _)| *id).collect();
        actual.sort_unstable();
        assert_eq!(actual, ids, "capture exactly real Test demands");
        assert!(work.spent() > 0);
        let position = hidden.iter().position(|id| *id == ids[0]).unwrap();
        let mut missing_hidden = hidden.clone();
        let mut missing_meanings = meanings.clone();
        missing_hidden.remove(position);
        missing_meanings.remove(position);
        for (bindings, values) in [
            (&missing_hidden[..], &missing_meanings[..]),
            (&hidden[..], &meanings[..position]),
        ] {
            let work = Work::new(None);
            let result = capture_predicate(
                &predicate, true, bindings, values, &tables, &work,
            );
            assert!(
                matches!(result, Err(Reason::Host(message)) if message == "missing predicate witness")
            );
            assert!(work.spent() > 0);
        }
        let work = Work::new(None);
        let unused =
            capture_predicate(&predicate, false, &[], &[], &tables, &work)
                .unwrap();
        assert!(
            unused.is_empty(),
            "Run does not descend a Function template"
        );
        let mut tail = &predicate;
        while let ir::Predicate::Function { result, .. } = tail {
            tail = result;
        }
        assert!(!matches!(tail, ir::Predicate::Function { .. }));
        let work = Work::new(None);
        let unused =
            capture_predicate(tail, true, &[], &[], &tables, &work).unwrap();
        assert!(
            unused.is_empty(),
            "Entry does not execute a scalar result tail"
        );
    }

    fn deep(depth: usize) -> Type {
        let mut ty = Type::Unit;
        for _ in 0..depth {
            ty = Type::Not(Box::new(ty));
        }
        ty
    }
    fn small_stack(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(test)
            .unwrap()
            .join()
            .unwrap();
    }
    fn copy_failure_source(
        body: usize,
        with_guards: bool,
    ) -> crate::value::Closure {
        let mut predicate = ir::Predicate::Nonzero { when_integer: true };
        for _ in 0..20_000 {
            predicate = ir::Predicate::Not(Box::new(predicate));
        }
        let guards = if with_guards {
            vec![
                crate::value::Guard {
                    spot: 0,
                    entry: 0,
                    predicate: ir::Predicate::Nonzero { when_integer: true },
                    witnesses: Rc::from([]),
                },
                crate::value::Guard {
                    spot: 1,
                    entry: 0,
                    predicate: ir::Predicate::All(vec![
                        ir::Predicate::Nonzero { when_integer: true },
                        predicate,
                    ]),
                    witnesses: Rc::from([]),
                },
            ]
        } else {
            drop(crate::value::resource::Predicates(vec![predicate]));
            Vec::new()
        };
        crate::value::Closure {
            body,
            captures: vec![Value::Int(3)],
            meanings: vec![deep(20_000)],
            filled: vec![Value::Int(7)],
            guards,
        }
    }

    #[test]
    fn guarded_copy_late_budget_failure_drains_deep_owned_forests() {
        small_stack(|| {
            // Copy-engine metadata only: no parsing, prelude or executable body.
            let program = crate::mir::Program {
                tables: Rc::new(
                    core::cell::RefCell::new(ir::Tables::default()),
                ),
                bodies: Vec::new(),
                receiver_adapters: Vec::new(),
                receiver_contexts: Vec::new(),
                constants: Vec::new(),
                definitions: Vec::new(),
                tests: Vec::new(),
                inits: Vec::new(),
                globals: Vec::new(),
                main: 0,
            };
            let tables = program.tables.borrow();
            let prefix_work = Work::new(None);
            let prefix_source = copy_failure_source(program.main, false);
            let prefix_copy = copy_guarded_closure(
                &prefix_source,
                &program,
                &tables,
                &prefix_work,
            )
            .unwrap();
            drop(prefix_copy);
            drop(prefix_source);
            let source = copy_failure_source(program.main, true);
            let first = Work::new(None);
            drop(copy_guard(&source.guards[0], &tables, &first).unwrap());
            let second = Work::new(None);
            drop(copy_guard(&source.guards[1], &tables, &second).unwrap());
            let prefix = prefix_work
                .spent()
                .checked_add(u64::try_from(source.guards.len()).unwrap())
                .unwrap();
            let later = prefix
                .checked_add(first.spent())
                .unwrap()
                .checked_add(second.spent() / 2)
                .unwrap();
            assert!(
                later > prefix + first.spent() + 8,
                "later failure follows the second guard's first built child"
            );
            assert!(later < prefix + first.spent() + second.spent());
            let final_finish = prefix
                .checked_add(first.spent())
                .unwrap()
                .checked_add(second.spent())
                .unwrap()
                .checked_sub(1)
                .unwrap();
            for allowance in [prefix, later, final_finish] {
                let work = Work::new(Some(allowance));
                let result =
                    copy_guarded_closure(&source, &program, &tables, &work);
                assert!(matches!(result, Err(Reason::Budget)));
                assert_eq!(work.spent(), allowance);
            }
            let work = Work::new(None);
            let copied =
                copy_guarded_closure(&source, &program, &tables, &work)
                    .unwrap();
            assert_eq!(copied.guards.len(), 2);
            assert_eq!(work.spent(), prefix + first.spent() + second.spent());
            drop(copied);
            drop(source);
        });
    }

    #[test]
    fn resource_partial_builder_and_last_descriptor_drop_are_iterative() {
        small_stack(|| {
            let input = Forest(vec![Type::Tuple(vec![deep(5000), deep(5000)])]);
            let work = Work::new(Some(20_000));
            assert!(matches!(
                materialize(
                    &input.0[0],
                    &[],
                    &[],
                    &ir::Tables::default(),
                    &work,
                    &mut Ok
                ),
                Err(Reason::Budget)
            ));
            assert_eq!(work.spent(), 20_000);
            let descriptor = ResourceType(Rc::new(Descriptor {
                root: Forest(vec![Type::Meaning {
                    index: 0,
                    arguments: vec![deep(5000)],
                }]),
                stamps: vec![Rc::new(())],
            }));
            let last = descriptor.clone();
            drop(descriptor);
            drop(last);
        });
    }
    #[test]
    fn resource_selection_guards_rejected_and_unselected_deep_bindings() {
        small_stack(|| {
            let input = Forest(vec![deep(5000)]);
            let work = Work::new(None);
            let mut tables = ir::Tables::default();
            tables.dispatch.push(ir::Dispatch {
                label: "member".into(),
                rows: vec![
                    ir::DispatchRow {
                        input_contract: None,
                        ty: Type::Unit,
                        declared_arity: Some(0),
                        receiver: Type::Parameter(0),
                        receiver_spot: None,
                        argument: None,
                        target: ir::Implementation::Declared(0),
                    },
                    ir::DispatchRow {
                        input_contract: None,
                        ty: Type::Unit,
                        declared_arity: None,
                        receiver: Type::Parameter(1),
                        receiver_spot: None,
                        argument: None,
                        target: ir::Implementation::Field(0),
                    },
                ],
            });
            let selected = select(&tables, 0, &input.0[0], None, &work)
                .unwrap()
                .unwrap();
            assert_eq!(selected.row, 1);
            drop(selected);
            let pattern =
                Type::Tuple(vec![Type::Parameter(0), Type::Parameter(0)]);
            let mut input_copy =
                materialize(&input.0[0], &[], &[], &tables, &work, &mut Ok)
                    .unwrap();
            let actual = Forest(vec![Type::Tuple(vec![
                input_copy.0.pop().unwrap(),
                Type::Unit,
            ])]);
            assert!(!matches(&pattern, &actual.0[0], &mut Vec::new(), &work)
                .unwrap());
        });
    }
    #[test]
    fn resource_work_failure_exhausts_only_the_finite_remainder() {
        let work = Work::new(Some(7));
        assert!(work.pay(3).is_ok());
        assert!(matches!(work.pay(8), Err(Reason::Budget)));
        assert_eq!(work.spent(), 7);
        assert_eq!(work.left.get(), Some(0));
        assert!(matches!(work.pay(1), Err(Reason::Budget)));
        assert_eq!(work.spent(), 7);
        let unlimited = Work::new(None);
        assert!(unlimited.pay(8).is_ok());
        assert_eq!(unlimited.spent(), 8);
    }
    #[test]
    fn resource_data_scan_meters_shared_allocations_without_expanding_paths() {
        let mut value = Value::Tuple(Rc::new(vec![Value::Int(7)]));
        for _ in 0..32 {
            value = Value::Tuple(Rc::new(vec![value.clone(), value]));
        }
        // Thirty-three shared allocations represent billions of value paths.
        let work = Work::new(Some(512));
        assert!(data_only(&value, &work).unwrap());
        assert!(work.spent() < 512);
        let exact = Work::new(Some(work.spent()));
        assert!(data_only(&value, &exact).unwrap());
        let short = Work::new(Some(work.spent() - 1));
        assert!(matches!(data_only(&value, &short), Err(Reason::Budget)));
        assert_eq!(short.spent(), work.spent() - 1);
        let ty = ResourceType(Rc::new(Descriptor {
            root: Forest(vec![Type::Meaning {
                index: 0,
                arguments: vec![],
            }]),
            stamps: vec![Rc::new(())],
        }));
        let alias = crate::value::resource::ResourceDomain::new()
            .issue(&ty, Rc::from(&b"hidden"[..]));
        // LIFO visits the shared ordinary subtree before the hidden reference.
        let hidden = Value::Tuple(Rc::new(vec![alias, value]));
        let work = Work::new(Some(512));
        assert!(!data_only(&hidden, &work).unwrap());
    }
    #[test]
    fn resource_materialized_witness_survives_body_handoff_and_normal_return() {
        const CHILD: &str = "FIPE_RESOURCE_NORMAL_HANDOFF_CHILD";
        if std::env::var_os(CHILD).is_none() {
            for mode in ["normal", "small"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "vm::resource::tests::resource_materialized_witness_survives_body_handoff_and_normal_return", "--nocapture"])
                .env(CHILD, mode)
                .status().unwrap();
                assert!(
                    status.success(),
                    "{mode} body handoff child: {status}"
                );
            }
            return;
        }
        // Private VM seam: the body is checked; the deep witness is materialized
        // directly so parser/checker recursion cannot masquerade as VM cleanup.
        let stack = if std::env::var(CHILD).unwrap() == "normal" {
            8 * 1024 * 1024
        } else {
            512 * 1024
        };
        std::thread::Builder::new().stack_size(stack).spawn(|| {
            let mut host = crate::decl::Declarations::new();
            host.module("csv", "pub type resource Rd<T>\npub open: Rd<()>\npub fresh: () -> Rd<a>\npub touch: Rd<a> -> Integer").unwrap();
            let module = crate::parse::parse(crate::lex::lex("use csv\nf: csv.Rd<a> -> Integer = x => { y: csv.Rd<a> = csv.fresh ()\n csv.touch y }\ng: csv.Rd<a> -> Integer = x => f x\ng csv.open")).unwrap();
            let unit = crate::check::check(&module, &host, &[], crate::check::Options::default()).unwrap();
            let program = crate::compile::compile(&unit);
            std::eprintln!("HANDOFF checked-program");
            let definition = program.tables.borrow().definitions.iter().position(|definition| definition.label == "g").expect("generic tail caller");
            let body = program.definitions[definition].expect("checked generic body");
            assert!(!program.bodies[body].hidden.is_empty());
            let input = Forest(vec![deep(20_000)]);
            let mut witness = materialize(&input.0[0], &[], &[], &program.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
            std::eprintln!("HANDOFF materialized");
            let mut machine = crate::vm::Machine::new(None);
            assert!(matches!(machine.run(&program), crate::embed::Outcome::Request(_)));
            assert!(machine.abandon());
            machine.invoke(&program, crate::vm::calls::Invoke::Body {
                body, args: vec![Value::Unit], meanings: core::mem::take(&mut witness.0),
            }, crate::vm::Return::Finish, program.bodies[body].node).unwrap();
            std::eprintln!("HANDOFF entered-body");
            let mut returned = false;
            let mut effects = Vec::new();
            for _ in 0..100 {
                match machine.step(&program).unwrap() {
                    crate::vm::Flow::Continue => {},
                    crate::vm::Flow::Done(value) => { assert_eq!(value, Value::Int(7)); returned = true; break; },
                    crate::vm::Flow::Request(crate::embed::Request::Call { declaration, .. }) => {
                        let label = program.tables.borrow().declarations[declaration].label.clone();
                        let value = match label.as_str() {
                            "fresh" => crate::value::resource::ResourceDomain::new().issue(&machine.pending_resource_type().unwrap(), Rc::from(&b"deep"[..])),
                            "touch" => Value::Int(7),
                            _ => panic!("unexpected effect {label}"),
                        };
                        std::eprintln!("HANDOFF host {label}");
                        effects.push(label);
                        assert!(matches!(machine.replied(&program, crate::embed::Reply::Value(Ok(value))).unwrap(), crate::vm::Flow::Continue));
                    },
                    crate::vm::Flow::Request(_) => panic!("unexpected file request"),
                }
            }
            std::eprintln!("HANDOFF returned");
            assert!(returned, "normal return must consume the frame witness");
            assert_eq!(effects, ["fresh", "touch"]);
            for _ in 0..2 {
                let mut witness = materialize(&input.0[0], &[], &[], &program.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
                machine.invoke(&program, crate::vm::calls::Invoke::Body { body, args: vec![Value::Unit], meanings: core::mem::take(&mut witness.0) }, crate::vm::Return::Finish, program.bodies[body].node).unwrap();
            }
            assert_eq!(machine.frames.len(), 2);
            machine.end();
            assert!(machine.frames.is_empty());
            let joined = machine.open(&program);
            let mut witness = materialize(&input.0[0], &[], &[], &joined.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
            machine.invoke(&joined, crate::vm::calls::Invoke::Body { body, args: vec![Value::Unit], meanings: core::mem::take(&mut witness.0) }, crate::vm::Return::Finish, joined.bodies[body].node).unwrap();
            assert!(machine.abandon());
            assert!(machine.frames.is_empty());
            std::eprintln!("HANDOFF tail-clear-abandon");
            drop(machine);
        }).unwrap().join().unwrap();
    }
    fn nonzero_then_test_program() -> (crate::mir::Program, usize) {
        let mut host = crate::decl::Declarations::new();
        host.module("csv", "pub type resource Unused\npub value: Integer")
            .unwrap();
        let module = crate::parse::parse(crate::lex::lex(
            "use csv\ntype P: Integer where self > 0\nx: P = csv.value\nx",
        ))
        .unwrap();
        let unit = crate::check::check(
            &module,
            &host,
            &[],
            crate::check::Options::default(),
        )
        .unwrap();
        let program = crate::compile::compile(&unit);
        let test = program
            .tables
            .borrow()
            .tests
            .iter()
            .enumerate()
            .find_map(|(index, test)| {
                (test.namespace == 0 && program.tests[index].is_some())
                    .then_some(index)
            })
            .expect("checked local limit body");
        (program, test)
    }

    #[test]
    fn resource_predicate_resume_pays_nonzero_owed_before_checked_test_body() {
        let (program, test) = nonzero_then_test_program();
        for finite in [false, true] {
            let mut machine =
                crate::vm::Machine::new(if finite { Some(0) } else { None });
            let joined = machine.open(&program);
            let run = crate::vm::predicates::Run::owned(
                crate::value::resource::Predicates(vec![ir::Predicate::All(
                    vec![
                        ir::Predicate::Nonzero { when_integer: true },
                        ir::Predicate::Test {
                            index: test,
                            meanings: vec![],
                        },
                    ],
                )]),
                vec![Value::Int(7)],
                crate::vm::predicates::After::Branch,
                Rc::from([]),
                &joined.tables.borrow(),
                &Work::new(None),
            )
            .unwrap();
            machine.frames.push(crate::vm::frames::Frame::Native(
                crate::vm::frames::NativeFrame {
                    node: joined.bodies[joined.main].node,
                    pending: crate::vm::pending::Pending::Predicate(run),
                    back: crate::vm::Return::Finish,
                    result: None,
                },
            ));
            let outcome = machine.resume(&joined, None);
            if finite {
                assert!(matches!(
                    outcome,
                    Err(crate::vm::Fault {
                        reason: Reason::Budget,
                        ..
                    })
                ));
                assert!(
                    matches!(
                        machine.frames.last(),
                        Some(crate::vm::frames::Frame::Native(_))
                    ),
                    "owed rejection must not enter the checked body"
                );
                assert_eq!(machine.charged(), 0);
            } else {
                assert!(matches!(outcome, Ok(crate::vm::Flow::Continue)));
                assert!(matches!(
                    machine.frames.last(),
                    Some(crate::vm::frames::Frame::Body(_))
                ));
                let mut called = false;
                for _ in 0..100 {
                    match machine.step(&joined).unwrap() {
                        crate::vm::Flow::Continue => {}
                        crate::vm::Flow::Done(value) => {
                            assert_eq!(
                                value,
                                crate::value::Prelude::new(
                                    &joined.tables.borrow()
                                )
                                .bool(true)
                            );
                            called = true;
                            break;
                        }
                        crate::vm::Flow::Request(_) => {
                            panic!("checked predicate must remain pure")
                        }
                    }
                }
                assert!(called);
            }
            assert!(machine.abandon());
        }
    }
    fn last_owner_probe(owner: &'static str, test: &'static str) {
        const CHILD: &str = "FIPE_RESOURCE_LAST_OWNER_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut passed = true;
            for mode in ["normal", "small"] {
                let status = std::process::Command::new(
                    std::env::current_exe().unwrap(),
                )
                .args(["--exact", test, "--nocapture"])
                .env(CHILD, mode)
                .status()
                .unwrap();
                std::eprintln!("OWNER {owner} {mode} STATUS {status}");
                passed &= status.success();
            }
            assert!(passed, "{owner} last-owner subprocess failure");
            return;
        }
        let stack = if std::env::var(CHILD).unwrap() == "normal" {
            8 * 1024 * 1024
        } else {
            512 * 1024
        };
        std::thread::Builder::new().stack_size(stack).spawn(move || {
            let mut host = crate::decl::Declarations::new();
            host.module("csv", "pub type resource Rd<T>\npub open: Rd<()>\npub fresh: () -> Rd<a>\npub touch: Rd<a> -> Integer").unwrap();
            let module = crate::parse::parse(crate::lex::lex("use csv\nf: csv.Rd<a> -> Integer = x => { y: csv.Rd<a> = csv.fresh ()\n csv.touch y }\nf csv.open")).unwrap();
            let unit = crate::check::check(&module, &host, &[], crate::check::Options::default()).unwrap();
            let program = crate::compile::compile(&unit);
            let body = program.bodies.iter().position(|body| body.arity == 1 && body.hidden.len() == 1).expect("checked generic callable, not its value thunk");
            assert_eq!(program.bodies[body].hidden.len(), 1);
            std::eprintln!("OWNER {owner} checked");
            let input = Forest(vec![deep(20_000)]);
            let mut witness = materialize(&input.0[0], &[], &[], &program.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
            let mut machine = crate::vm::Machine::new(None);
            let joined = machine.open(&program);
            let body = joined.bodies.iter().position(|body| body.arity == 1 && body.hidden.len() == 1).expect("joined generic callable");
            std::eprintln!("OWNER {owner} materialized joined-body={body} arity={}", joined.bodies[body].arity);
            match owner {
                "closure" => {
                    let function = Value::Function(Rc::new(crate::value::Closure { body, captures: vec![], meanings: core::mem::take(&mut witness.0), filled: vec![], guards: vec![] }));
                    let result = machine.call_value(&joined, &function, vec![], crate::vm::Return::Finish, joined.bodies[body].node).unwrap();
                    let Value::Function(original) = function else { unreachable!() };
                    let mut original = Rc::try_unwrap(original).unwrap();
                    drop(Forest(core::mem::take(&mut original.meanings)));
                    drop(original);
                    let crate::vm::Flow::Done(Value::Function(partial)) = result else { panic!("actual partial application") };
                    assert_eq!(partial.filled.len(), 0);
                    assert_eq!(partial.meanings.len(), 1);
                    std::eprintln!("OWNER closure actual-partial-last-drop");
                    drop(partial);
                },
                "native" => {
                    let result = witness.0.pop().unwrap();
                    std::eprintln!("OWNER native before-start");
                    let flow = machine.start(&joined, crate::vm::pending::Pending::Once(Some(crate::vm::calls::Invoke::Value(Value::Unit))), crate::vm::drive::Start::new(crate::vm::Return::Finish, joined.bodies[body].node, Some(result))).unwrap();
                    assert!(matches!(flow, crate::vm::Flow::Done(Value::Unit)));
                    for abandon in [false, true] {
                        let joined = machine.open(&program);
                        let mut copied = materialize(&input.0[0], &[], &[], &joined.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
                        machine.frames.push(crate::vm::frames::Frame::Native(crate::vm::frames::NativeFrame {
                            node: joined.bodies[joined.main].node,
                            pending: crate::vm::pending::Pending::Once(None),
                            back: crate::vm::Return::Finish,
                            result: copied.0.pop(),
                        }));
                        if abandon { assert!(machine.abandon()); } else { machine.end(); }
                        assert!(machine.frames.is_empty());
                    }
                },
                "hidden" => {
                    machine.invoke(&joined, crate::vm::calls::Invoke::Body { body, args: vec![Value::Unit], meanings: core::mem::take(&mut witness.0) }, crate::vm::Return::Finish, joined.bodies[body].node).unwrap();
                    let value = machine.operand(&joined, crate::mir::Operand::Hidden(0)).unwrap();
                    assert!(matches!(value, Value::Meaning(_)));
                    machine.end();
                    std::eprintln!("OWNER hidden copied-only-owner-last-drop");
                    drop(value);
                },
                _ => unreachable!(),
            }
            std::eprintln!("OWNER {owner} returned");
            machine.end();
        }).unwrap().join().unwrap();
    }
    #[test]
    fn resource_closure_partial_last_owner_drop_probe() {
        last_owner_probe("closure", "vm::resource::tests::resource_closure_partial_last_owner_drop_probe");
    }
    #[test]
    fn resource_native_result_last_owner_drop_probe() {
        last_owner_probe(
            "native",
            "vm::resource::tests::resource_native_result_last_owner_drop_probe",
        );
    }
    #[test]
    fn resource_hidden_operand_last_owner_drop_probe() {
        last_owner_probe("hidden", "vm::resource::tests::resource_hidden_operand_last_owner_drop_probe");
    }
    #[test]
    fn resource_data_constructor_cannot_wrap_checked_resource_alias() {
        let mut host = crate::decl::Declarations::new();
        host.module("csv", "pub type Probe where self.inspect: Integer\npub type resource Rd<T>\npub open: Rd<()>\npub take: Probe -> Integer\nimpl Rd<a>: Probe { inspect: (self: Self) -> Integer }").unwrap();
        let module = crate::parse::parse(crate::lex::lex("use csv\ng: csv.Probe -> Integer = x => x.inspect\ncsv.take csv.open")).unwrap();
        let unit = crate::check::check(
            &module,
            &host,
            &[],
            crate::check::Options::default(),
        )
        .unwrap();
        let program = crate::compile::compile(&unit);
        let mut machine = crate::vm::Machine::new(None);
        assert!(matches!(
            machine.run(&program),
            crate::embed::Outcome::Request(_)
        ));
        let alias = crate::value::resource::ResourceDomain::new().issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"entry"[..]),
        );
        let crate::embed::Outcome::Request(crate::embed::Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.reply(&program, crate::embed::Reply::Value(Ok(alias)))
        else {
            panic!("checked packed producer")
        };
        let tables = program.tables.borrow();
        assert_eq!(tables.declarations[declaration].label, "take");
        let [single @ Value::Packed(packed)] = arguments.as_slice() else {
            panic!("actual checker-created single pack")
        };
        assert!(matches!(packed.value(), Value::Resource(_)));
        let member = tables
            .dispatch
            .iter()
            .position(|member| member.label == "inspect")
            .expect("checked member row");
        let prelude = Prelude::new(&tables);
        assert!(crate::vm::dispatch::select_for(
            &tables,
            &prelude,
            member,
            None,
            Some(single),
            &Work::new(None)
        )
        .unwrap()
        .is_some());
        assert!(crate::value::Packed::from_data(
            packed.source_type().clone(),
            packed.target_type().clone(),
            single
        )
        .is_none());
    }
    #[test]
    fn resource_default_build_last_owner_drop() {
        const CHILD: &str = "FIPE_DEFAULT_BUILD_OWNER";
        if std::env::var_os(CHILD).is_none() {
            let mut passed = true;
            for mode in ["normal", "small"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", "vm::resource::tests::resource_default_build_last_owner_drop", "--nocapture"]).env(CHILD, mode).status().unwrap();
                std::eprintln!("DEFAULT {mode} STATUS {status}");
                passed &= status.success();
            }
            assert!(passed);
            return;
        }
        let stack = if std::env::var(CHILD).unwrap() == "normal" {
            8 * 1024 * 1024
        } else {
            512 * 1024
        };
        std::thread::Builder::new().stack_size(stack).spawn(|| {
            let mut declarations = crate::decl::Declarations::new();
            declarations.module("csv", "pub type resource Rd<T>").unwrap();
            let module = crate::parse::parse(crate::lex::lex("use csv\ntype Phantom<T> { value: Bool }\nx: Phantom<csv.Rd<()>> = Phantom\nx")).unwrap();
            let unit = crate::check::check(&module, &declarations, &[], crate::check::Options::default()).unwrap();
            let program = crate::compile::compile(&unit);
            let resource = program.tables.borrow().meanings.iter().position(|meaning| meaning.label == "Rd").unwrap();
            let root = program.tables.borrow().meanings.iter().position(|meaning| meaning.label == "Phantom").unwrap();
            std::eprintln!("DEFAULT checked");
            let mut argument = Type::Unit;
            for _ in 0..20_000 { argument = Type::Not(Box::new(argument)); }
            let source = Forest(vec![Type::Meaning { index: root, arguments: vec![Type::Meaning { index: resource, arguments: vec![argument] }] }]);
            let mut copied = materialize(&source.0[0], &[], &[], &program.tables.borrow(), &Work::new(None), &mut Ok).unwrap();
            let mut machine = crate::vm::Machine::new(None);
            let joined = machine.open(&program);
            std::eprintln!("DEFAULT materialized before-start");
            let flow = machine.start(&joined, crate::vm::pending::Pending::Walk(crate::vm::walk::Walk::default(copied.0.pop().unwrap())), crate::vm::drive::Start::new(crate::vm::Return::Finish, joined.bodies[joined.main].node, None)).unwrap();
            assert!(matches!(flow, crate::vm::Flow::Done(Value::Record(record)) if record.meaning == root && record.fields == vec![Prelude::new(&joined.tables.borrow()).bool(false)]));
            std::eprintln!("DEFAULT returned");
            machine.end();
        }).unwrap().join().unwrap();
    }

    #[test]
    fn unrelated_ordinary_records_do_not_allocate_production_stamps() {
        let mut source = alloc::string::String::new();
        for index in 0..128 {
            source.push_str(&alloc::format!(
                "type Data{index} {{ n: Integer }}\nvalue{index} = Data{index} {index}\n"
            ));
        }
        source.push_str("value127");
        let mut session =
            crate::session::Session::new(crate::decl::Declarations::new());
        let crate::embed::Outcome::Done(report) = session.line(&source) else {
            panic!("ordinary declarations have no source requests")
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let unit = report.unit.as_ref().unwrap();
        assert_eq!(unit.tables().resource_cache.len(), 0);
        assert!(!unit.tables().has_resources);
        let tables = unit.tables();
        let namespace = tables.nodes[*unit.main.last().unwrap()].namespace;
        assert_eq!(tables.namespaces[namespace], "line 1");
        let actual: Vec<_> = tables
            .meanings
            .iter()
            .enumerate()
            .filter(|(_, meaning)| meaning.namespace == namespace)
            .map(|(index, meaning)| (meaning.label.clone(), index))
            .collect();
        assert_eq!(actual.len(), 128);
        let labels: alloc::collections::BTreeSet<_> =
            actual.iter().map(|(label, _)| label.clone()).collect();
        let expected = (0..128)
            .map(|index| alloc::format!("Data{index}"))
            .collect::<alloc::collections::BTreeSet<_>>();
        assert_eq!(labels, expected);
        assert_eq!(
            actual
                .iter()
                .map(|(_, index)| *index)
                .collect::<alloc::collections::BTreeSet<_>>()
                .len(),
            128
        );
        drop(tables);
        let program = crate::compile::compile(unit);
        assert_eq!(program.tables.borrow().resource_cache.len(), 0);
        assert!(program.bodies.iter().all(|body| body.blocks.iter().all(
            |block| {
                block.statements.iter().all(|statement| {
                    !matches!(
                        statement,
                        crate::mir::Statement::Assign(
                            _,
                            crate::mir::Rvalue::Pack { .. }
                        )
                    )
                })
            }
        )));
        let mut machine = crate::vm::Machine::new(None);
        let crate::embed::Outcome::Done(Ok(Value::Record(record))) =
            machine.run(&program)
        else {
            panic!("ordinary records execute without effects")
        };
        assert_eq!(record.fields, [Value::Int(127)]);
        assert_eq!(program.tables.borrow().resource_cache.len(), 0);
        assert_eq!(
            machine
                .program()
                .unwrap()
                .tables
                .borrow()
                .resource_cache
                .len(),
            0
        );
    }

    #[test]
    fn resource_cache_clone_and_pruning_are_sparse() {
        let cache = crate::value::resource::ResourceCache::default();
        let empty_fork = cache.clone();
        assert_eq!(cache.len(), 0);
        assert_eq!(empty_fork.len(), 0);
        let before = cache.stamp(3);
        assert!(Rc::ptr_eq(&before, &cache.stamp(3)));
        let populated_fork = cache.clone();
        assert_eq!(populated_fork.len(), 0);
        assert!(!Rc::ptr_eq(&before, &populated_fork.stamp(3)));
        cache.truncate(4);
        assert!(Rc::ptr_eq(&before, &cache.stamp(3)));
        cache.truncate(3);
        assert!(!Rc::ptr_eq(&before, &cache.stamp(3)));
    }
    fn session_copy_program(
        fixed: bool,
        integer: &Type,
    ) -> crate::mir::Program {
        let mut tables = ir::Tables {
            meanings: vec![
                ir::Meaning {
                    resource: false,
                    label: "Integer".into(),
                    namespace: 0,
                    parameters: Vec::new(),
                    constructors: Vec::new(),
                },
                ir::Meaning {
                    resource: false,
                    label: "Box".into(),
                    namespace: 0,
                    parameters: Vec::new(),
                    constructors: Vec::new(),
                },
            ],
            ..Default::default()
        };
        tables
            .session_meanings
            .push(fixed.then_some(integer.clone()));
        crate::mir::Program {
            tables: Rc::new(core::cell::RefCell::new(tables)),
            bodies: Vec::new(),
            receiver_adapters: Vec::new(),
            receiver_contexts: Vec::new(),
            constants: Vec::new(),
            definitions: Vec::new(),
            tests: Vec::new(),
            inits: Vec::new(),
            globals: Vec::new(),
            main: 0,
        }
    }

    #[test]
    fn guarded_closure_session_meanings_are_semantic_but_test_syntax_is_raw() {
        for fixed in [false, true] {
            let integer = ir::Type::Meaning {
                index: 0,
                arguments: Vec::new(),
            };
            let raw = ir::Type::Meaning {
                index: 1,
                arguments: vec![ir::Type::Session(0)],
            };
            let program = session_copy_program(fixed, &integer);
            // Private copy-engine input, not a public compiler-produced closure.
            let source = crate::value::Closure {
                body: 0,
                captures: Vec::new(),
                meanings: vec![raw.clone()],
                filled: Vec::new(),
                guards: vec![crate::value::Guard {
                    spot: 0,
                    entry: 0,
                    predicate: ir::Predicate::Test {
                        index: 0,
                        meanings: vec![ir::Type::Session(0)],
                    },
                    witnesses: Rc::from([]),
                }],
            };
            let tables = program.tables.borrow();
            let work = Work::new(None);
            let copy = copy_guarded_closure(&source, &program, &tables, &work)
                .unwrap();
            let expected_meaning = if fixed {
                ir::Type::Meaning {
                    index: 1,
                    arguments: vec![integer],
                }
            } else {
                raw.clone()
            };
            assert_eq!(copy.meanings, [expected_meaning]);
            assert_eq!(source.meanings.as_slice(), core::slice::from_ref(&raw));
            assert_eq!(copy.guards[0].predicate, source.guards[0].predicate);
            assert!(Rc::ptr_eq(
                &copy.guards[0].witnesses,
                &source.guards[0].witnesses
            ));
            let expected = if fixed { 15 } else { 13 };
            assert_eq!(
                work.spent(),
                expected,
                "fixed={fixed}, exact semantic-copy events"
            );
            let raw_work = Work::new(None);
            let raw_copy =
                copy_predicate(&source.guards[0].predicate, &tables, &raw_work)
                    .unwrap();
            assert_eq!(
                raw_copy.0,
                [ir::Predicate::Test {
                    index: 0,
                    meanings: vec![ir::Type::Session(0)]
                }]
            );
            assert_eq!(
                raw_work.spent(),
                4,
                "raw syntax never follows the fixed Session"
            );
            let short = Work::new(Some(expected - 1));
            assert!(matches!(
                copy_guarded_closure(&source, &program, &tables, &short),
                Err(Reason::Budget)
            ));
            assert_eq!(short.spent(), expected - 1);
            assert_eq!(source.meanings, [raw]);
        }
    }
}

#[derive(Clone, Copy)]
enum PredicateShape {
    All,
    Any,
    Function,
    Not,
}
enum PredicateCopy<'a> {
    Read(&'a ir::Predicate),
    Finish(PredicateShape, usize),
}

/// Exact predicate syntax copy. This never substitutes a creator parameter or Session.
pub(super) fn copy_predicate(
    predicate: &ir::Predicate,
    tables: &ir::Tables,
    work: &Work,
) -> Result<crate::value::resource::Predicates, Reason> {
    use crate::value::resource::Predicates;
    work.pay(1)?;
    let mut jobs = vec![PredicateCopy::Read(predicate)];
    let mut first = true;
    let mut done = Predicates::default();
    while let Some(job) = jobs.pop() {
        if !first {
            work.pay(1)?;
        }
        first = false;
        match job {
            PredicateCopy::Read(predicate) => match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    work.pay(u64::try_from(parts.len()).unwrap_or(u64::MAX))?;
                    let shape = if matches!(predicate, ir::Predicate::All(_)) {
                        PredicateShape::All
                    } else {
                        PredicateShape::Any
                    };
                    jobs.push(PredicateCopy::Finish(shape, parts.len()));
                    jobs.extend(parts.iter().rev().map(PredicateCopy::Read));
                }
                ir::Predicate::Function { parameter, result } => {
                    work.pay(2)?;
                    jobs.push(PredicateCopy::Finish(
                        PredicateShape::Function,
                        2,
                    ));
                    jobs.push(PredicateCopy::Read(result));
                    jobs.push(PredicateCopy::Read(parameter));
                }
                ir::Predicate::Not(inner) => {
                    work.pay(1)?;
                    jobs.push(PredicateCopy::Finish(PredicateShape::Not, 1));
                    jobs.push(PredicateCopy::Read(inner));
                }
                ir::Predicate::Nonzero { when_integer } => {
                    work.pay(1)?;
                    done.0.push(ir::Predicate::Nonzero {
                        when_integer: *when_integer,
                    });
                }
                ir::Predicate::Test { index, meanings } => {
                    work.pay(
                        u64::try_from(meanings.len()).unwrap_or(u64::MAX),
                    )?;
                    let mut copied = Forest::default();
                    for ty in meanings {
                        let mut one = materialize_bound(
                            ty,
                            &[],
                            tables,
                            work,
                            false,
                            &mut Ok,
                        )?;
                        copied.0.push(one.0.pop().expect("raw predicate type"));
                    }
                    work.pay(1)?;
                    done.0.push(ir::Predicate::Test {
                        index: *index,
                        meanings: core::mem::take(&mut copied.0),
                    });
                }
            },
            PredicateCopy::Finish(shape, count) => {
                let mut parts = done.0.split_off(done.0.len() - count);
                let copied = match shape {
                    PredicateShape::All => ir::Predicate::All(parts),
                    PredicateShape::Any => ir::Predicate::Any(parts),
                    PredicateShape::Function => {
                        let result = parts.pop().expect("predicate result");
                        let parameter =
                            parts.pop().expect("predicate parameter");
                        ir::Predicate::Function {
                            parameter: Box::new(parameter),
                            result: Box::new(result),
                        }
                    }
                    PredicateShape::Not => ir::Predicate::Not(Box::new(
                        parts.pop().expect("predicate part"),
                    )),
                };
                done.0.push(copied);
            }
        }
    }
    Ok(done)
}

pub(super) fn copy_guard(
    guard: &crate::value::Guard,
    tables: &ir::Tables,
    work: &Work,
) -> Result<crate::value::Guard, Reason> {
    work.pay(2)?; // guard record and its shared witness association
    let mut predicate = copy_predicate(&guard.predicate, tables, work)?;
    Ok(crate::value::Guard {
        spot: guard.spot,
        entry: guard.entry,
        predicate: predicate.0.pop().expect("guard predicate"),
        witnesses: guard.witnesses.clone(),
    })
}

pub(super) fn copy_guards(
    guards: &[crate::value::Guard],
    tables: &ir::Tables,
    work: &Work,
) -> Result<Vec<crate::value::Guard>, Reason> {
    work.pay(u64::try_from(guards.len()).unwrap_or(u64::MAX))?;
    let mut copied = Vec::with_capacity(guards.len());
    for guard in guards {
        copied.push(copy_guard(guard, tables, work)?);
    }
    Ok(copied)
}
