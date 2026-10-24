//! The budget, copied from `src/vm/budget.rs`: a block pays for its
//! nodes when it opens, a native for its size before it acts; what
//! cannot be paid Faults `Budget` at the first node that cannot be
//! paid.
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

    /// Pay the unchanged logical price, bounding preflight by what remains.
    pub(super) fn pay_native(
        &mut self,
        native: Native,
        args: &[Value],
    ) -> Result<(), Reason> {
        let remaining = self.left();
        if let Some(units) = size(native, args, remaining) {
            self.pay(units)
        } else {
            self.pay(remaining)?;
            Err(Reason::Budget)
        }
    }

    /// The units the run may still spend; without a budget, all.
    const fn left(&self) -> u64 {
        match self.budget {
            Some(budget) => budget.saturating_sub(self.charged),
            None => u64::MAX,
        }
    }
}

/// A representable price within the exact allowance; overflow is not MAX.
fn capped(price: Option<u64>, allowance: u64) -> Option<u64> {
    price.filter(|price| *price <= allowance)
}

/// The 64-bit limbs of an integer; an `Int` is one.
fn limbs(value: &Value) -> Option<u64> {
    match value {
        Value::Big(big) => {
            Some(u64::try_from(big.magnitude.bit_length()).ok()?.div_ceil(64))
        }
        _ => Some(1),
    }
}

fn length(value: &Value) -> Option<u64> {
    match value {
        Value::Seq(items) => u64::try_from(items.len()).ok(),
        _ => Some(0),
    }
}

/// Root costs zero; every queued child is an already-counted occurrence.
fn visited(value: &Value, allowance: u64) -> Option<u64> {
    let mut count = 0_u64;
    let mut work: Vec<&Value> = vec![value];
    while let Some(value) = work.pop() {
        let below: &[Value] = match value {
            Value::Seq(parts) | Value::Tuple(parts) => parts,
            Value::Record(record) => &record.fields,
            Value::Packed(packed) => std::slice::from_ref(packed.value()),
            _ => &[],
        };
        let children = u64::try_from(below.len()).ok()?;
        count = capped(count.checked_add(children), allowance)?;
        // A wide node is rejected before growing the worklist.
        work.extend(below);
    }
    Some(count)
}

/// The original logical size term, or None before work exceeds its allowance.
fn size(native: Native, args: &[Value], allowance: u64) -> Option<u64> {
    let integers = matches!(
        args,
        [Value::Int(_) | Value::Big(_), Value::Int(_) | Value::Big(_)]
    );
    let price = match native {
        Native::Add | Native::Sub | Native::Eq | Native::Cmp if integers => {
            limbs(&args[0])?.max(limbs(&args[1])?)
        }
        Native::Mul | Native::Div | Native::Rem if integers => {
            limbs(&args[0])?.checked_mul(limbs(&args[1])?)?
        }
        Native::Eq | Native::Cmp => {
            return args
                .get(1)
                .map_or(Some(0), |value| visited(value, allowance))
        }
        Native::Show | Native::Default(_) => {
            return args
                .first()
                .map_or(Some(0), |value| visited(value, allowance))
        }
        Native::Map
        | Native::Fold
        | Native::Filter
        | Native::Sum
        | Native::Count
        | Native::Sort
        | Native::Collect
        | Native::Concat
        | Native::Join
        | Native::Reverse
        | Native::Enumerate
        | Native::Zip
        | Native::Contains
        | Native::StartsWith
        | Native::EndsWith
        | Native::Replace => {
            return args.iter().try_fold(0_u64, |total, value| {
                capped(total.checked_add(length(value)?), allowance)
            });
        }
        Native::Flatten => {
            let Some(Value::Seq(items)) = args.first() else {
                return Some(0);
            };
            let outer =
                capped(Some(u64::try_from(items.len()).ok()?), allowance)?;
            // Reject the outer length before inspecting any inner item.
            return items.iter().try_fold(outer, |total, value| {
                capped(total.checked_add(length(value)?), allowance)
            });
        }
        Native::Drop | Native::Nth | Native::Take => match args {
            [Value::Int(count), over] => {
                u64::try_from(*count).unwrap_or(0).min(length(over)?)
            }
            [_, over] => length(over)?,
            _ => 0,
        },
        Native::First | Native::Head => match args.first() {
            Some(Value::Seq(items)) => u64::from(!items.is_empty()),
            _ => 0,
        },
        _ => 0,
    };
    capped(Some(price), allowance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn shared(depth: usize) -> Value {
        let mut value = Value::Unit;
        for _ in 0..depth {
            value = Value::Tuple(Rc::new(vec![value.clone(), value]));
        }
        value
    }

    #[test]
    fn size_zero_exact_and_shared_occurrence_prices() {
        assert_eq!(visited(&Value::Unit, 0), Some(0));
        assert_eq!(visited(&Value::Tuple(Rc::new(Vec::new())), 0), Some(0));
        let value = shared(4);
        assert_eq!(visited(&value, 29), None);
        assert_eq!(visited(&value, 30), Some(30));
        assert_eq!(visited(&value, 31), Some(30));
        assert_eq!(
            size(Native::Show, core::slice::from_ref(&value), 30),
            Some(30)
        );
        assert_eq!(size(Native::Eq, &[value.clone(), value], 30), Some(30));
    }

    #[test]
    fn size_wide_empty_and_deep_shared_bounds() {
        let empty = Value::Tuple(Rc::new(Vec::new()));
        let wide = Value::Tuple(Rc::new(vec![empty; 1024]));
        assert_eq!(visited(&wide, 1), None);
        assert_eq!(visited(&wide, 1023), None);
        assert_eq!(visited(&wide, 1024), Some(1024));
        assert_eq!(visited(&shared(14), 4), None);
    }

    #[test]
    fn size_checked_arithmetic_distinguishes_max_from_overflow() {
        assert_eq!(capped(Some(0), 0), Some(0));
        assert_eq!(capped(u64::MAX.checked_add(0), u64::MAX), Some(u64::MAX));
        assert_eq!(capped(u64::MAX.checked_add(1), u64::MAX), None);
        assert_eq!(capped(u64::MAX.checked_mul(1), u64::MAX), Some(u64::MAX));
        assert_eq!(capped(u64::MAX.checked_mul(2), u64::MAX), None);
        assert_eq!(capped(Some(u64::MAX), u64::MAX - 1), None);
    }

    #[test]
    fn size_flatten_and_scalar_prices_keep_their_bounds() {
        let two = Value::seq(vec![Value::Unit; 2]);
        let one = Value::seq(vec![Value::Unit]);
        let outer = Value::seq(vec![two.clone(), one.clone()]);
        assert_eq!(
            size(Native::Flatten, core::slice::from_ref(&outer), 4),
            None
        );
        assert_eq!(size(Native::Flatten, &[outer], 5), Some(5));
        let wide = Value::seq(vec![Value::seq(Vec::new()); 1024]);
        assert_eq!(
            size(Native::Flatten, core::slice::from_ref(&wide), 1),
            None
        );
        assert_eq!(size(Native::Flatten, &[wide], 1024), Some(1024));
        assert_eq!(size(Native::Zip, &[two.clone(), one], 2), None);
        assert_eq!(
            size(
                Native::Zip,
                &[two.clone(), Value::seq(vec![Value::Unit])],
                3
            ),
            Some(3)
        );
        assert_eq!(size(Native::Take, &[Value::Int(1), two], 1), Some(1));
        assert_eq!(size(Native::Mul, &[Value::Int(3), Value::Int(4)], 0), None);
        assert_eq!(
            size(Native::Mul, &[Value::Int(3), Value::Int(4)], 1),
            Some(1)
        );
        assert_eq!(size(Native::First, &[Value::seq(Vec::new())], 0), Some(0));
    }

    fn payer(allowance: u64) -> State {
        let (_, program) =
            fipe_native_core::codec::Unit::new("()", vec![], vec![]).unwrap();
        State::new(program, Some(allowance), crate::host::Host::open().unwrap())
    }

    #[test]
    fn size_payment_consumes_exact_remaining_without_a_root_fee() {
        let mut zero = payer(0);
        assert_eq!(zero.pay_native(Native::Show, &[Value::Unit]), Ok(()));
        assert_eq!(zero.charged, 0);
        let value = shared(1);
        for allowance in [0, 1, 2, 3] {
            let mut state = payer(allowance);
            let result =
                state.pay_native(Native::Show, core::slice::from_ref(&value));
            assert_eq!(
                result,
                if allowance < 2 {
                    Err(Reason::Budget)
                } else {
                    Ok(())
                }
            );
            assert_eq!(state.charged, allowance.min(2));
        }
        let mut last = payer(u64::MAX);
        assert_eq!(last.pay(u64::MAX - 1), Ok(()));
        assert_eq!(
            last.pay_native(Native::Show, &[value]),
            Err(Reason::Budget)
        );
        assert_eq!(last.charged, u64::MAX);
    }
}
