//! Experimental C-compatible scalar layouts, mirrored in `include/fipe.h`.
use std::ffi::c_void;

/// Scalar-only experimental ABI; the earlier tree layout is unsupported.
pub const VERSION: u32 = 2;
pub const UNIT: u32 = 0;
pub const INTEGER: u32 = 1;
pub const FLOAT: u32 = 2;
pub const TEXT: u32 = 3;

/// Borrowed bytes. A zero length permits a null pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Bytes {
    pub data: *const u8,
    pub len: usize,
}
impl Bytes {
    #[must_use]
    pub const fn new(bytes: &[u8]) -> Self {
        Self {
            data: bytes.as_ptr(),
            len: bytes.len(),
        }
    }
}
impl Default for Bytes {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }
}

/// A borrowed scalar with every field initialized, including inactive fields.
///
/// Padding need not be initialized. Pointer backing is needed only for TEXT;
/// initialize inactive pointers to null. In C start with `FipeValue reply = {0};`.
/// INTEGER is signed i64, FLOAT is finite F64, and TEXT is UTF-8 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Value {
    pub tag: u32,
    pub integer: i64,
    pub float: f64,
    pub bytes: Bytes,
}
impl Default for Value {
    fn default() -> Self {
        Self {
            tag: UNIT,
            integer: 0,
            float: 0.0,
            bytes: Bytes::default(),
        }
    }
}
impl Value {
    #[must_use]
    pub fn integer(value: i64) -> Self {
        Self {
            tag: INTEGER,
            integer: value,
            ..Self::default()
        }
    }
    #[must_use]
    pub fn float(value: f64) -> Self {
        Self {
            tag: FLOAT,
            float: value,
            ..Self::default()
        }
    }
    #[must_use]
    pub fn text(value: &str) -> Self {
        Self {
            tag: TEXT,
            bytes: Bytes::new(value.as_bytes()),
            ..Self::default()
        }
    }
}

/// Copy one scalar reply: 0 accepted, nonzero invalid/duplicate reply.
///
/// Invoke synchronously on the calling thread, without overlap or reentrancy.
/// The value and its text remain valid until emit returns; retain no pointers.
pub type Emit = unsafe extern "C" fn(*mut c_void, *const Value) -> u32;
/// Return 0 for a scalar, 1 for a host Fault (emit TEXT); other codes fail.
///
/// Emit exactly once. Never retain inputs/context/emitter or unwind across FFI.
/// Validate the function label, argument count and tags before using inputs.
pub type Call =
    unsafe extern "C" fn(Bytes, *const Value, usize, Emit, *mut c_void) -> u32;

/// Immutable static provider descriptor; code/buffers outlive its registration.
///
/// Shared mutable provider state requires synchronization across all wrappers.
/// This proof has no resources, structured outcomes or concurrent callbacks.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Provider {
    pub version: u32,
    pub size: usize,
    pub module: Bytes,
    pub declarations: Bytes,
    pub call: Option<Call>,
}
