//! `sort` and `count`: natives that compare the values they walk. A
//! comparison of two leaves, or with no written member in the unit, is
//! answered at once; any other is a structural walk in its own native
//! frame, which calls a written `cmp` or `eq` where it meets one.
use super::{
    calls::Invoke,
    immediate::seq,
    natives::{Ctx, Step},
    pending::Pending,
    walk::Walk,
};
use crate::value::{structural, Value};
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::cmp::Ordering;

/// An `Ordering` value's order.
fn ordering_of(value: &Value) -> Ordering {
    match value {
        Value::Record(record) => match record.constructor {
            0 => Ordering::Less,
            1 => Ordering::Equal,
            _ => Ordering::Greater,
        },
        other => unreachable!("an Ordering, not {other:?}"),
    }
}

/// Whether two values compare without a call: both leaves, or no
/// written member of this name in the unit.
const fn plain(member: Option<usize>, a: &Value, b: &Value) -> bool {
    member.is_none() || (super::step::leaf(a) && super::step::leaf(b))
}

/// A walk in its own native frame, answering to the frame that asked.
fn walk(walk: Walk) -> Step {
    Step::Call(Invoke::Native(Box::new(Pending::Walk(walk))))
}

/// `sort`: the keys first, by the `by` function, then a stable bottom-up
/// merge of the positions, each comparison of two keys a move.
#[derive(Debug)]
pub(super) struct Sort {
    items: Rc<Vec<Value>>,
    by: Value,
    desc: bool,
    keys: Vec<Value>,
    /// The positions in their current order, and the merge's output.
    order: Vec<usize>,
    out: Vec<usize>,
    width: usize,
    /// The merge in progress: left `[i, mid)`, right `[j, end)`, the
    /// output at `k`, the pair `start..end` it covers.
    start: usize,
    i: usize,
    mid: usize,
    j: usize,
    end: usize,
    k: usize,
}

impl Sort {
    pub(super) fn start(by: Value, desc: bool, over: &Value) -> Step {
        let items = Rc::clone(seq(over));
        let count = items.len();
        Step::Start(Pending::Sort(Self {
            items,
            by,
            desc,
            keys: Vec::with_capacity(count),
            order: (0..count).collect(),
            out: alloc::vec![0; count],
            width: 1,
            start: 0,
            i: 0,
            mid: 0,
            j: 0,
            end: 0,
            k: 0,
        }))
    }

    /// One move: a key or a comparison returned, or none to begin.
    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        let count = self.items.len();
        if self.keys.len() < count {
            if let Some(key) = returned {
                self.keys.push(key);
            }
            if self.keys.len() < count {
                let item = self.items[self.keys.len()].clone();
                return Step::Call(Invoke::Function(
                    self.by.clone(),
                    alloc::vec![item],
                ));
            }
            self.open(0);
        } else if let Some(answer) = returned {
            self.decide(ordering_of(&answer));
        }
        let member = ctx.natives.members.cmp;
        while self.width < count {
            if self.k == self.end {
                self.open(self.end);
                continue;
            }
            if self.i < self.mid && self.j < self.end {
                let right = &self.keys[self.order[self.j]];
                let left = &self.keys[self.order[self.i]];
                if plain(member, right, left) {
                    let ordering = structural::cmp(right, left);
                    self.decide(ordering);
                    continue;
                }
                return walk(Walk::cmp(right.clone(), left.clone()));
            }
            let from = if self.i < self.mid {
                &mut self.i
            } else {
                &mut self.j
            };
            self.out[self.k] = self.order[*from];
            *from += 1;
            self.k += 1;
        }
        let sorted = self
            .order
            .iter()
            .map(|index| self.items[*index].clone())
            .collect();
        Step::Done(Value::seq(sorted))
    }

    /// The right key against the left: the right goes first only when it
    /// strictly precedes, so equal keys keep their order.
    fn decide(&mut self, right_to_left: Ordering) {
        let wanted = if self.desc {
            Ordering::Greater
        } else {
            Ordering::Less
        };
        let from = if right_to_left == wanted {
            &mut self.j
        } else {
            &mut self.i
        };
        self.out[self.k] = self.order[*from];
        *from += 1;
        self.k += 1;
    }

    /// Open the merge at `start`, or the next width when this pass ends.
    fn open(&mut self, start: usize) {
        let count = self.items.len();
        let mut start = start;
        if start >= count {
            core::mem::swap(&mut self.order, &mut self.out);
            self.width *= 2;
            start = 0;
        }
        self.start = start;
        self.i = start;
        self.mid = (start + self.width).min(count);
        self.j = self.mid;
        self.end = (start + 2 * self.width).min(count);
        self.k = start;
    }
}

/// `count`: each distinct item in its first order with how often it
/// occurs, items compared by `eq`.
#[derive(Debug)]
pub(super) struct Count {
    items: Rc<Vec<Value>>,
    index: usize,
    distinct: Vec<(Value, usize)>,
    /// The item being placed and the distinct one it is compared with.
    current: Option<Value>,
    probe: usize,
}

impl Count {
    pub(super) fn start(over: &Value) -> Step {
        Step::Start(Pending::Count(Self {
            items: Rc::clone(seq(over)),
            index: 0,
            distinct: Vec::new(),
            current: None,
            probe: 0,
        }))
    }

    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        if let Some(answer) = returned {
            self.answer(ctx.prelude.is_true(&answer));
        }
        let member = ctx.natives.members.eq;
        loop {
            let Some(item) = self.current.clone() else {
                if self.index == self.items.len() {
                    let counts = core::mem::take(&mut self.distinct)
                        .into_iter()
                        .map(|(item, n)| ctx.prelude.count(item, n))
                        .collect();
                    return Step::Done(Value::seq(counts));
                }
                self.current = Some(self.items[self.index].clone());
                self.index += 1;
                self.probe = 0;
                continue;
            };
            if self.probe == self.distinct.len() {
                self.distinct.push((item, 1));
                self.current = None;
                continue;
            }
            let known = &self.distinct[self.probe].0;
            if plain(member, &item, known) {
                let equal = structural::eq(&item, known);
                self.answer(equal);
                continue;
            }
            return walk(Walk::eq(item, known.clone()));
        }
    }

    /// The current item against the probed distinct one.
    fn answer(&mut self, equal: bool) {
        if equal {
            self.distinct[self.probe].1 += 1;
            self.current = None;
        } else {
            self.probe += 1;
        }
    }
}
