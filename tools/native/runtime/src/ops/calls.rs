//! The callees the runtime performs, copied from `src/vm/calls.rs` (a
//! `Native` callee, a host's declaration among them) and `src/vm.rs`
//! (a host call and its reply). A `Body` callee is a call of machine
//! code, which the generated code makes itself.
use super::{budget, fits, natives::Kind, Faulted, Site, State};
use fipe::{
    embed::Request,
    mir::{Callee, Terminator},
    value::{Reason, Value},
};

/// The result of the site's `Call` of a `Native` or `Host` callee, its
/// arguments read by the generated code.
///
/// # Errors
///
/// When the native or the host call Faults; the Fault is kept.
///
/// # Panics
///
/// At another terminator, a defect of the lowering.
pub fn call(
    state: &mut State,
    site: Site,
    args: Vec<Value>,
) -> Result<Value, Faulted> {
    let Terminator::Call { callee, dest, .. } = &state.block(site).terminator
    else {
        unreachable!("a call site")
    };
    let dest = *dest;
    let (op, host) = match callee {
        Callee::Native(op) => (*op, false),
        Callee::Host(declaration) => (*declaration, true),
        other => unreachable!("the generated code calls {other:?}"),
    };
    // A declared row of a host's namespace is a host call.
    let native = match state.natives.get(op) {
        _ if host => None,
        Kind::Host => None,
        Kind::Native(Some(native)) => Some(native),
        Kind::Native(None) => unreachable!("the lowering refuses this native"),
    };
    let Some(native) = native else {
        return state.host(site, op, args, dest);
    };
    let units = budget::size(native, &args);
    state
        .pay(units)
        .and_then(|()| super::natives::immediate(native, args, &state.prelude))
        .map_err(|reason| state.faulted(site, reason))
}

impl State {
    /// Hand a host call to the host and wait for its reply, whose value
    /// must have the shape of the type of `dest`, the call's local.
    fn host(
        &mut self,
        site: Site,
        declaration: usize,
        arguments: Vec<Value>,
        dest: usize,
    ) -> Result<Value, Faulted> {
        let request = Request::Call {
            declaration,
            arguments,
            environment: self.environment.clone(),
        };
        let reply = match self.host.call(&request, &self.lines) {
            Ok(reply) => reply,
            Err(error) => {
                return Err(self.faulted(site, Reason::Host(error.to_string())))
            }
        };
        match reply {
            Ok(value) => {
                let body = &self.program.bodies[site.body];
                assert!(body.hidden.is_empty(), "no witness is substituted");
                let ty = &body.locals[dest].ty;
                let misfit = {
                    let tables = self.program.tables.borrow();
                    (!fits::fits(&value, ty, &tables, &self.prelude))
                        .then(|| fits::named(ty, &tables))
                };
                if let Some(named) = misfit {
                    let message = format!("the reply is not {named}");
                    return Err(self.faulted(site, Reason::Host(message)));
                }
                Ok(value)
            }
            Err(message) => Err(self.faulted(site, Reason::Host(message))),
        }
    }
}
