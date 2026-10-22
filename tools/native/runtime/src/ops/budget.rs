//! The budget, copied from `src/vm/budget.rs`: a block pays for its
//! nodes when it opens, a native for its size before it acts; what
//! cannot be paid Faults `Budget` at the first node that cannot be
//! paid. Only the integer natives' size terms are copied so far.
use super::{fault, natives::Native, State};
use fipe::{
    value::{Reason, Value},
    vm::Fault,
};

impl State {
    /// Pay one unit per node, in order; the Fault is at the first node
    /// the budget left cannot pay, before anything of the block runs.
    pub(super) fn pay_nodes(&mut self, nodes: &[usize]) -> Result<(), Fault> {
        let left = self.left();
        let count = nodes.len() as u64;
        if count <= left {
            self.charged = self.charged.saturating_add(count);
            return Ok(());
        }
        self.charged = self.charged.saturating_add(left);
        let unpaid = usize::try_from(left).expect("below the node count");
        Err(fault(&self.program, nodes[unpaid], Reason::Budget))
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

/// A native's size term over its arguments, beyond its node's unit:
/// an `Integer` operation by limbs.
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
        _ => 0,
    }
}
