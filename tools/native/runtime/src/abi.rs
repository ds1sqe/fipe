//! The C exports the generated code calls. A local of that code is an
//! opaque slot holding one [`Value`]; every export takes slot pointers,
//! checks them and returns a status: [`OK`], or 1 for a Fault, which
//! the runtime keeps.
#![allow(unsafe_code)]

use fipe::value::Value;
use std::{mem, process};

/// The status of an export that succeeded.
const OK: u32 = 0;

// The generated code reserves 16 bytes, 8-aligned, for each slot.
const _: () = assert!(mem::size_of::<Value>() == 16);
const _: () = assert!(mem::align_of::<Value>() == 8);

#[cfg(not(test))] // The test harness has its own `main`.
extern "C" {
    /// The program's main body, defined by the generated object.
    fn fipe_main() -> u32;
}

/// The process entry: runs the program; exit status 0 when it
/// succeeded, 1 when it Faulted.
#[cfg(not(test))]
#[no_mangle]
extern "C" fn main() -> std::ffi::c_int {
    // SAFETY: `fipe_main` takes no argument and returns a status.
    std::ffi::c_int::from(unsafe { fipe_main() } != OK)
}

/// The slot behind `slot`. A null or misaligned pointer is a bug of the
/// code generator, not a Fault of the program, so it aborts.
fn checked(slot: *mut Value) -> *mut Value {
    if slot.is_null() || !slot.is_aligned() {
        process::abort();
    }
    slot
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
