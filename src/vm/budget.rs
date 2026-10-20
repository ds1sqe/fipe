//! The budget: a block pays for its nodes when it opens, a native for
//! its size before it acts; what cannot be paid Faults `Budget` at the
//! first node that cannot be paid.
use super::{natives::Native, Fault, Machine};
use crate::{
    mir,
    value::{Reason, Value},
};
use alloc::vec::Vec;

impl Machine {
    /// Pay one unit per node, in order; the Fault is at the first node
    /// the budget left cannot pay, before anything of the block runs.
    pub(super) fn pay_nodes(
        &mut self,
        program: &mir::Program,
        nodes: &[usize],
    ) -> Result<(), Fault> {
        let left = self.left();
        let count = nodes.len() as u64;
        if count <= left {
            self.charged = self.charged.saturating_add(count);
            return Ok(());
        }
        self.charged = self.charged.saturating_add(left);
        let unpaid = usize::try_from(left).expect("below the node count");
        Err(Self::fault(program, nodes[unpaid], Reason::Budget))
    }

    /// Pay `units`, or Fault `Budget` (at the caller's node).
    pub(super) const fn pay(&mut self, units: u64) -> Result<(), Reason> {
        let left = self.left();
        if units <= left {
            self.charged = self.charged.saturating_add(units);
            return Ok(());
        }
        self.charged = self.charged.saturating_add(left);
        Err(Reason::Budget)
    }

    /// The units the run may still spend; without a budget, all.
    const fn left(&self) -> u64 {
        match self.budget {
            Some(budget) => budget.saturating_sub(self.charged),
            None => u64::MAX,
        }
    }
}

/// The 64-bit limbs of an integer; an `Int` is one.
fn limbs(value: &Value) -> u64 {
    match value {
        Value::Big(big) => (big.magnitude.bit_length() as u64).div_ceil(64),
        _ => 1,
    }
}

fn length(value: &Value) -> u64 {
    match value {
        Value::Seq(items) => items.len() as u64,
        _ => 0,
    }
}

/// The values below a value that a structural walk visits: every
/// element, part, field and payload, each once.
fn visited(value: &Value) -> u64 {
    let mut count = 0;
    let mut work: Vec<&Value> = alloc::vec![value];
    while let Some(value) = work.pop() {
        let below: &[Value] = match value {
            Value::Seq(parts) | Value::Tuple(parts) => parts,
            Value::Record(record) => &record.fields,
            Value::Packed(packed) => core::slice::from_ref(&packed.value),
            _ => &[],
        };
        count += below.len() as u64;
        work.extend(below);
    }
    count
}

/// A native's size term over its arguments, beyond its node's unit:
/// an `Integer` operation by limbs, a container operation one per
/// element it visits, a structural walk one per value below the
/// receiver.
pub(super) fn size(native: Native, args: &[Value]) -> u64 {
    let integers = matches!(
        args,
        [Value::Int(_) | Value::Big(_), Value::Int(_) | Value::Big(_)]
    );
    match native {
        Native::Add | Native::Sub | Native::Eq | Native::Cmp if integers => {
            limbs(&args[0]).max(limbs(&args[1]))
        }
        Native::Mul | Native::Div | Native::Rem if integers => {
            limbs(&args[0]) * limbs(&args[1])
        }
        // `other: Self -> (self: Self)`: the receiver is spot 1.
        Native::Eq | Native::Cmp => args.get(1).map_or(0, visited),
        Native::Show | Native::Default(_) => args.first().map_or(0, visited),
        Native::Map
        | Native::Fold
        | Native::Filter
        | Native::Sum
        | Native::Count
        | Native::Sort
        | Native::Collect
        | Native::Concat
        | Native::Join => args.iter().map(length).sum(),
        Native::Flatten => args.first().map_or(0, |outer| {
            let Value::Seq(items) = outer else { return 0 };
            items.len() as u64 + items.iter().map(length).sum::<u64>()
        }),
        Native::Take => match args {
            [Value::Int(count), over] => {
                u64::try_from(*count).unwrap_or(0).min(length(over))
            }
            [_, over] => length(over),
            _ => 0,
        },
        Native::First | Native::Head => {
            args.first().map_or(0, |over| length(over).min(1))
        }
        _ => 0,
    }
}
