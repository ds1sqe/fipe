//! A separate linked Rust provider using only the public scalar ABI.
use fipe_ffi::abi;
use std::{cell::Cell, ffi::c_void};

thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }

#[must_use]
pub fn calls() -> usize {
    CALLS.with(Cell::get)
}

#[must_use]
pub fn descriptor() -> abi::Provider {
    abi::Provider {
        version: abi::VERSION,
        size: size_of::<abi::Provider>(),
        module: abi::Bytes::new(b"proof.ops"),
        declarations: abi::Bytes::new(include_bytes!("../../api.fipi")),
        call: Some(call),
    }
}

unsafe extern "C" fn call(
    name: abi::Bytes,
    arguments: *const abi::Value,
    len: usize,
    emit: abi::Emit,
    context: *mut c_void,
) -> u32 {
    CALLS.with(|count| count.set(count.get() + 1));
    let mut value = abi::Value::text("invalid request");
    let mut status = 1;
    if !name.data.is_null() && len == 1 && !arguments.is_null() {
        // The adapter lends initialized storage for this synchronous invocation.
        let name = unsafe { std::slice::from_raw_parts(name.data, name.len) };
        let argument = unsafe { &*arguments };
        match (name, argument.tag) {
            (b"integer", abi::INTEGER) | (b"text", abi::TEXT) => {
                value = *argument;
                status = 0;
            }
            (b"fail", abi::UNIT) => {
                value = abi::Value::text("provider refused");
            }
            _ => {}
        }
    }
    // Borrowed argument/static text remains live until emit copies it.
    if unsafe { emit(context, &raw const value) } == 0 {
        status
    } else {
        2
    }
}
