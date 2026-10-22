//! Which operands the generated code reads into consecutive slots for a
//! site the runtime performs. The lowering includes this file, so the
//! two never disagree on a count.
use fipe::mir::{Callee, Operand, Rvalue, Witness};

/// The operands of an assignment the runtime computes, in order; `None`
/// for one the generated code performs itself or does not lower.
#[must_use]
pub fn rvalue(rvalue: &Rvalue) -> Option<&[Operand]> {
    match rvalue {
        Rvalue::Construct { fields: parts, .. }
        | Rvalue::Native { args: parts, .. }
        | Rvalue::Closure {
            captures: parts, ..
        }
        | Rvalue::Tuple(parts)
        | Rvalue::Array(parts) => Some(parts),
        Rvalue::Cast { value, .. } => Some(std::slice::from_ref(value)),
        _ => None,
    }
}

/// The operand a callee reads besides the arguments: a function value,
/// or a value whose tag dispatches.
#[must_use]
pub const fn callee(callee: &Callee) -> Option<Operand> {
    match callee {
        Callee::Value(operand)
        | Callee::Dispatch {
            witness: Witness::Value(operand),
            ..
        } => Some(*operand),
        _ => None,
    }
}

/// How many slots a call the runtime performs reads: its arguments,
/// then its callee's operand.
#[must_use]
pub fn call(callee: &Callee, args: &[Operand]) -> usize {
    args.len() + usize::from(self::callee(callee).is_some())
}
