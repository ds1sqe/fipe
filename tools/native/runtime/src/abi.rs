//! The C exports the generated code calls. A local of that code is an
//! opaque slot holding one [`Value`]; every export takes slot pointers,
//! checks them, calls a safe function of `ops` and returns a status:
//! [`OK`], or [`FAULT`] for a Fault, which the runtime keeps.
//!
//! A site is a statement or terminator by body, block and position, the
//! terminator's position being the block's statement count: the runtime
//! reads what the site does from the program it compiled at startup.
#![allow(unsafe_code)]

use crate::{
    ops::{self, Faulted, Site},
    start,
};
use fipe::value::Value;
use std::{ffi::c_void, mem, process};

/// The status of an export that succeeded.
const OK: u32 = 0;
/// The status of an export that Faulted.
const FAULT: u32 = 1;

// The generated code reserves 16 bytes, 8-aligned, for each slot.
const _: () = assert!(mem::size_of::<Value>() == 16);
const _: () = assert!(mem::align_of::<Value>() == 8);

#[cfg(not(test))] // The test harness has its own `main`.
extern "C" {
    /// The program's main body, defined by the generated object: its
    /// result into the slot `out`, its status returned.
    fn fipe_main(out: *mut std::ffi::c_void) -> u32;
    /// The unit the object carries: its length in eight bytes
    /// little-endian, then the bytes `codec::Unit` encodes.
    static fipe_unit: [u8; 8];
    /// The address of each body of the object, by index.
    static fipe_bodies: [Body; 0];
}

/// The process entry: runs the program; the exit code `start::main`
/// gives.
#[cfg(not(test))]
#[no_mangle]
extern "C" fn main() -> std::ffi::c_int {
    // SAFETY: the object defines `fipe_unit` as its length followed by
    // that many bytes, in `.rodata`, which lives as long as the process.
    let unit: &'static [u8] = unsafe {
        let start = std::ptr::addr_of!(fipe_unit).cast::<u8>();
        let length = u64::from_le_bytes(fipe_unit);
        let length = usize::try_from(length).unwrap_or_else(|_| abort());
        std::slice::from_raw_parts(start.add(8), length)
    };
    // SAFETY: `fipe_main` takes the slot of its result, which holds a
    // value, and returns a status.
    start::main(unit, |out| unsafe {
        fipe_main(std::ptr::from_mut(out).cast())
    })
}

fn abort() -> ! {
    process::abort()
}

/// The slot behind `slot`. A null or misaligned pointer is a bug of the
/// code generator, not a Fault of the program, so it aborts.
fn checked(slot: *mut Value) -> *mut Value {
    if slot.is_null() || !slot.is_aligned() {
        abort();
    }
    slot
}

fn checked_const(slot: *const Value) -> *const Value {
    checked(slot.cast_mut()).cast_const()
}

fn site(body: u64, block: u64, index: u64) -> Site {
    Site {
        body: size(body),
        block: size(block),
        index: size(index),
    }
}

fn size(n: u64) -> usize {
    usize::try_from(n).unwrap_or_else(|_| abort())
}

/// Clones of `count` slots from `slots`.
///
/// # Safety
///
/// `slots` points to `count` consecutive slots holding values, or
/// `count` is 0.
unsafe fn read(slots: *const Value, count: usize) -> Vec<Value> {
    if count == 0 {
        return Vec::new();
    }
    let slots = checked_const(slots);
    // SAFETY: the caller's slots hold `count` values.
    unsafe { std::slice::from_raw_parts(slots, count) }.to_vec()
}

/// Replace the value of the slot `out` with `value`.
///
/// # Safety
///
/// `out` points to a slot holding a value.
unsafe fn put(out: *mut Value, value: Value) {
    // SAFETY: the slot holds a value, which the assignment drops.
    unsafe { *checked(out) = value };
}

const fn status(result: Result<(), Faulted>) -> u32 {
    if result.is_ok() {
        OK
    } else {
        FAULT
    }
}

/// Writes `Unit` into the empty slot `out`.
///
/// # Safety
///
/// `out` points to a slot of the caller holding no value.
#[no_mangle]
unsafe extern "C" fn rt_unit(out: *mut Value) -> u32 {
    // SAFETY: the slot is valid and empty, so nothing is overwritten.
    unsafe { checked(out).write(Value::Unit) };
    OK
}

/// Writes `Unit` into `count` empty consecutive slots: a frame's
/// locals as it opens.
///
/// # Safety
///
/// `slots` points to `count` consecutive slots holding no value.
#[no_mangle]
unsafe extern "C" fn rt_units(slots: *mut Value, count: u64) {
    let slots = checked(slots);
    for at in 0..size(count) {
        // SAFETY: each of the slots is valid and empty.
        unsafe { slots.add(at).write(Value::Unit) };
    }
}

/// Drops the values of `count` consecutive slots: a frame's locals as
/// it closes, which hold no value after.
///
/// # Safety
///
/// `slots` points to `count` consecutive slots holding values.
#[no_mangle]
unsafe extern "C" fn rt_drops(slots: *mut Value, count: u64) {
    let slots = checked(slots);
    for at in 0..size(count) {
        // SAFETY: each slot holds a value, dropped once.
        unsafe { slots.add(at).drop_in_place() };
    }
}

/// A body's spots: clones of its `count` arguments into `count`
/// consecutive slots from `spots`.
///
/// # Safety
///
/// `spots` points to `count` consecutive slots holding values, `args`
/// to `count` consecutive slots holding values.
#[no_mangle]
unsafe extern "C" fn rt_spots(
    spots: *mut Value,
    args: *const Value,
    count: u64,
) {
    let count = size(count);
    // SAFETY: as the caller promises.
    let args = unsafe { read(args, count) };
    let spots = checked(spots);
    for (at, arg) in args.into_iter().enumerate() {
        // SAFETY: each spot holds a value.
        unsafe { put(spots.add(at), arg) };
    }
}

/// `out` = a clone of `from`, which may be `out`.
///
/// # Safety
///
/// Both point to slots holding values.
#[no_mangle]
unsafe extern "C" fn rt_copy(out: *mut Value, from: *const Value) {
    // SAFETY: `from` holds a value; the clone is made before `out` is
    // written.
    let value = unsafe { (*checked_const(from)).clone() };
    // SAFETY: `out` holds a value.
    unsafe { put(out, value) };
}

/// `out` = the value of `from`, which holds `Unit` after.
///
/// # Safety
///
/// Both point to distinct slots holding values.
#[no_mangle]
unsafe extern "C" fn rt_move(out: *mut Value, from: *mut Value) {
    // SAFETY: `from` holds a value, replaced by `Unit`.
    let value = unsafe { checked(from).replace(Value::Unit) };
    // SAFETY: `out` holds a value.
    unsafe { put(out, value) };
}

/// `out` = the prepared constant `index`.
///
/// # Safety
///
/// `out` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_const(out: *mut Value, index: u64) {
    let index = size(index);
    let value = start::with(|state| state.program.constants[index].clone());
    // SAFETY: `out` holds a value.
    unsafe { put(out, value) };
}

/// `out` = part `index` of `from`, which may be `out`.
///
/// # Safety
///
/// Both point to slots holding values.
#[no_mangle]
unsafe extern "C" fn rt_field(out: *mut Value, from: *const Value, index: u64) {
    let index = size(index);
    // SAFETY: `from` holds a value; the part is cloned before `out` is
    // written.
    let value = ops::field(unsafe { &*checked_const(from) }, index);
    // SAFETY: `out` holds a value.
    unsafe { put(out, value) };
}

/// The key the value of `from` switches on.
///
/// # Safety
///
/// `from` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_switch(from: *const Value) -> u64 {
    // SAFETY: `from` holds a value.
    ops::switch_key(unsafe { &*checked_const(from) })
}

/// The site's `Charge`.
#[no_mangle]
extern "C" fn rt_charge(body: u64, block: u64, index: u64) -> u32 {
    status(start::with(|state| state.charge(site(body, block, index))))
}

/// The site's `Line` of the value of `from`.
///
/// # Safety
///
/// `from` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_line(
    from: *const Value,
    body: u64,
    block: u64,
    index: u64,
) {
    // SAFETY: `from` holds a value.
    let value = unsafe { (*checked_const(from)).clone() };
    start::with(|state| state.line(site(body, block, index), value));
}

/// The witnesses of a frame of `body`, from its `hidden` slots.
///
/// # Safety
///
/// `hidden` points to as many slots holding values as the body has
/// witnesses, or the body has none.
unsafe fn witnesses(hidden: *const Value, body: u64) -> Vec<Value> {
    let count = start::with(|state| state.hidden(size(body)));
    // SAFETY: as the caller promises.
    unsafe { read(hidden, count) }
}

/// Write an operation's value into `out`, or return the status of its
/// Fault.
///
/// # Safety
///
/// `out` points to a slot holding a value.
unsafe fn land(out: *mut Value, value: Result<Value, Faulted>) -> u32 {
    match value {
        Ok(value) => {
            // SAFETY: `out` holds a value.
            unsafe { put(out, value) };
            OK
        }
        Err(Faulted) => FAULT,
    }
}

/// The site's `Assign` of an rvalue the runtime computes into `out`,
/// its operands in the consecutive slots from `args`, `hidden` the
/// frame's witnesses.
///
/// # Safety
///
/// `out` points to a slot holding a value; `args` to as many slots
/// holding values as the rvalue has operands; `hidden` as
/// [`witnesses`].
#[no_mangle]
unsafe extern "C" fn rt_assign(
    out: *mut Value,
    args: *const Value,
    hidden: *const Value,
    body: u64,
    block: u64,
    index: u64,
) -> u32 {
    let site = site(body, block, index);
    let count = start::with(|state| state.operands(site));
    // SAFETY: as the caller promises.
    let (args, hidden) =
        unsafe { (read(args, count), witnesses(hidden, body)) };
    let value = ops::assign(site, args, &hidden);
    // SAFETY: `out` holds a value.
    unsafe { land(out, value) }
}

/// The `Call` terminating the block, performed by the runtime, into
/// `out`: its arguments, then its callee's operand, in the consecutive
/// slots from `args`; `hidden` the frame's witnesses.
///
/// # Safety
///
/// `out` points to a slot holding a value; `args` to as many slots
/// holding values as the call reads; `hidden` as [`witnesses`].
#[no_mangle]
unsafe extern "C" fn rt_call(
    out: *mut Value,
    args: *const Value,
    hidden: *const Value,
    body: u64,
    block: u64,
) -> u32 {
    let site = start::with(|state| state.terminator(site(body, block, 0)));
    let count = start::with(|state| state.operands(site));
    // SAFETY: as the caller promises.
    let (args, hidden) =
        unsafe { (read(args, count), witnesses(hidden, body)) };
    let value = ops::call(site, args, &hidden);
    // SAFETY: `out` holds a value.
    unsafe { land(out, value) }
}

/// `out` = a clone of slot `index` from `base`: a captured value or a
/// witness of the frame.
///
/// # Safety
///
/// `base` points to more than `index` consecutive slots holding values;
/// `out` to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_index(out: *mut Value, base: *const Value, index: u64) {
    let base = checked_const(base);
    // SAFETY: slot `index` of `base` holds a value.
    let value = unsafe { (*base.add(size(index))).clone() };
    // SAFETY: `out` holds a value.
    unsafe { put(out, value) };
}

/// `out` = a value definition's global, computed by its body first when
/// unset.
///
/// # Safety
///
/// `out` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_global(out: *mut Value, definition: u64) -> u32 {
    let value = ops::global(size(definition));
    // SAFETY: `out` holds a value.
    unsafe { land(out, value) }
}

/// The site's `Store` of the value of `from`.
///
/// # Safety
///
/// `from` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_store(
    from: *const Value,
    body: u64,
    block: u64,
    index: u64,
) {
    // SAFETY: `from` holds a value.
    let value = unsafe { (*checked_const(from)).clone() };
    start::with(|state| state.store(site(body, block, index), value));
}

/// The site's `Enter` of the value of `slot`, which takes the guarded
/// function when the entry rebuilds it.
///
/// # Safety
///
/// `slot` points to a slot holding a value.
#[no_mangle]
unsafe extern "C" fn rt_enter(
    slot: *mut Value,
    body: u64,
    block: u64,
    index: u64,
) -> u32 {
    // SAFETY: `slot` holds a value.
    let value = unsafe { (*checked(slot)).clone() };
    match ops::enter(site(body, block, index), value) {
        Ok(Some(guarded)) => {
            // SAFETY: `slot` holds a value.
            unsafe { put(slot, guarded) };
            OK
        }
        Ok(None) => OK,
        Err(Faulted) => FAULT,
    }
}

/// A body of the object, as `fipe_bodies` holds it.
type Body = unsafe extern "C" fn(
    *const c_void,
    *const c_void,
    *const c_void,
    *mut c_void,
) -> u32;

/// Enter body `body` of the object through `fipe_bodies`: its result,
/// or the Fault it kept.
///
/// # Errors
///
/// When the body Faults.
pub fn invoke(
    body: usize,
    args: &[Value],
    captures: &[Value],
    hidden: &[Value],
) -> Result<Value, Faulted> {
    // Slots the body reads past these would be a defect of the code
    // generator, not a Fault of the program.
    let fits = start::with(|state| {
        state.program.bodies.get(body).is_some_and(|shape| {
            shape.arity == args.len()
                && shape.captures == captures.len()
                && shape.hidden.len() == hidden.len()
        })
    });
    if !fits {
        abort();
    }
    let mut out = Value::Unit;
    // SAFETY: the object's `fipe_bodies` holds one body per index of the
    // program it was built from, which startup checked; each takes its
    // spots, captures and witnesses as slots, which these slices hold
    // and outlive the call, and its result slot, which holds a value.
    let status = unsafe {
        let entry = bodies_table().add(body).read();
        entry(
            args.as_ptr().cast(),
            captures.as_ptr().cast(),
            hidden.as_ptr().cast(),
            std::ptr::from_mut(&mut out).cast(),
        )
    };
    if status == OK {
        Ok(out)
    } else {
        Err(Faulted)
    }
}

/// The object's table of bodies.
#[cfg(not(test))]
fn bodies_table() -> *const Body {
    std::ptr::addr_of!(fipe_bodies).cast()
}

/// The test harness links no object: no body is entered.
#[cfg(test)]
fn bodies_table() -> *const Body {
    abort()
}

/// The block's switch had no case for its value: Faults `Unreachable`.
#[no_mangle]
extern "C" fn rt_unreachable(body: u64, block: u64) -> u32 {
    start::with(|state| {
        let site = state.terminator(site(body, block, 0));
        state.unreachable(site)
    });
    FAULT
}
