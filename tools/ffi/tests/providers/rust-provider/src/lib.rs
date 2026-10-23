//! Independent Rust provider: only the public FFI ABI crosses this boundary.
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
        declarations: abi::Bytes::new(include_bytes!("../../api.fipe")),
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
    // The adapter lends valid label/argument storage until this callback returns.
    let name = unsafe { std::slice::from_raw_parts(name.data, name.len) };
    let arguments = unsafe { std::slice::from_raw_parts(arguments, len) };
    let mut status = 0;
    let value = match (name, arguments) {
        (b"unit", [value]) if value.tag == abi::UNIT => *value,
        (b"integer", [value]) if value.tag == abi::INTEGER => *value,
        (b"float", [value]) if value.tag == abi::FLOAT => *value,
        (b"text", [value]) if value.tag == abi::TEXT => *value,
        (b"choose", [first, second])
            if first.tag == abi::INTEGER && second.tag == abi::TEXT =>
        {
            *second
        }
        (b"wrong", [value]) if value.tag == abi::UNIT => {
            abi::Value::text("not an integer")
        }
        (b"fail", [value]) if value.tag == abi::UNIT => {
            status = 1;
            abi::Value::text("provider refused")
        }
        _ => {
            status = 1;
            abi::Value::text("unknown request")
        }
    };
    // Text is borrowed from arguments or static literals and emit copies it now.
    unsafe { emit(context, &raw const value) };
    status
}
