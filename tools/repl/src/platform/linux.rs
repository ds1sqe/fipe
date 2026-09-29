//! Linux on `x86_64` without `std`: raw syscalls, a bump allocator over a
//! static arena, `_start` and the panic handler. The only unsafe code.

#![allow(unsafe_code)]

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("the `linux` platform is x86_64 Linux only");

use alloc::{string::String, vec::Vec};
use core::{
    alloc::{GlobalAlloc, Layout},
    arch::{asm, naked_asm},
    cell::UnsafeCell,
    ffi::c_void,
    fmt::{self, Write as _},
    panic::PanicInfo,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

const READ: usize = 0;
const WRITE: usize = 1;
const OPEN: usize = 2;
const CLOSE: usize = 3;
const EXIT: usize = 60;

const ARENA: usize = 64 << 20;

/// A syscall with up to three arguments; negative results are `-errno`.
///
/// # Safety
/// The arguments must be valid for `number`, pointers included.
unsafe fn syscall(number: usize, a: usize, b: usize, c: usize) -> isize {
    let result;
    // SAFETY: the caller vouches for the arguments; the kernel clobbers
    // only rcx and r11 besides rax.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number => result,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result
}

/// Bytes read from stdin into `buffer`; 0 at end of input or on error.
pub fn read(buffer: &mut [u8]) -> usize {
    read_fd(0, buffer)
}

fn read_fd(fd: usize, buffer: &mut [u8]) -> usize {
    loop {
        let (at, len) = (buffer.as_mut_ptr() as usize, buffer.len());
        // SAFETY: the kernel writes at most `len` bytes into `buffer`.
        let read = unsafe { syscall(READ, fd, at, len) };
        if read != -4 {
            return usize::try_from(read).unwrap_or(0);
        }
    }
}

pub fn write(bytes: &[u8]) {
    write_fd(1, bytes);
}

fn write_fd(fd: usize, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let (at, len) = (bytes.as_ptr() as usize, bytes.len());
        // SAFETY: the kernel reads at most `len` bytes from `bytes`.
        let written = unsafe { syscall(WRITE, fd, at, len) };
        match usize::try_from(written) {
            Ok(written) => bytes = &bytes[written..],
            Err(_) if written == -4 => {}
            Err(_) => return,
        }
    }
}

pub fn read_file(path: &str) -> Option<String> {
    let mut name = Vec::with_capacity(path.len() + 1);
    name.extend_from_slice(path.as_bytes());
    name.push(0);
    // SAFETY: `name` is NUL-terminated; flags 0 is `O_RDONLY`.
    let fd = unsafe { syscall(OPEN, name.as_ptr() as usize, 0, 0) };
    let fd = usize::try_from(fd).ok()?;
    let mut text = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match read_fd(fd, &mut buffer) {
            0 => break,
            read => text.extend_from_slice(&buffer[..read]),
        }
    }
    // SAFETY: `fd` was opened above and is closed once.
    unsafe { syscall(CLOSE, fd, 0, 0) };
    String::from_utf8(text).ok()
}

fn exit(code: usize) -> ! {
    // SAFETY: `exit` takes no pointer and does not return.
    unsafe { syscall(EXIT, code, 0, 0) };
    unreachable!()
}

/// The kernel enters with rsp 16-byte aligned; a call realigns it for
/// the Rust function.
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!("xor ebp, ebp", "call {start}", "ud2", start = sym start);
}

extern "C" fn start() -> ! {
    crate::run();
    exit(0)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    struct Stderr;
    impl fmt::Write for Stderr {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            write_fd(2, text.as_bytes());
            Ok(())
        }
    }
    let _ = writeln!(Stderr, "fipe-repl: {info}");
    exit(101)
}

/// Never frees; the arena lives in `.bss`.
struct Bump {
    arena: UnsafeCell<[u8; ARENA]>,
    next: AtomicUsize,
}

// SAFETY: the program is single-threaded, and `next` is atomic anyway.
unsafe impl Sync for Bump {}

// SAFETY: every block is aligned, in bounds and handed out once.
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = self.arena.get().cast::<u8>();
        let next = self.next.load(Ordering::Relaxed);
        let start = next + base.wrapping_add(next).align_offset(layout.align());
        match start.checked_add(layout.size()) {
            Some(end) if end <= ARENA => {
                self.next.store(end, Ordering::Relaxed);
                base.wrapping_add(start)
            }
            _ => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static ALLOCATOR: Bump = Bump {
    arena: UnsafeCell::new([0; ARENA]),
    next: AtomicUsize::new(0),
};

// Without libc, the memory routines the compiler emits calls to come
// from here; `rep` string instructions keep them from compiling into
// calls to themselves.

/// # Safety
/// `dest` and `src` are valid for `n` bytes and do not overlap.
#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(
    dest: *mut c_void,
    src: *const c_void,
    n: usize,
) -> *mut c_void {
    // SAFETY: the caller vouches for both ranges.
    unsafe {
        asm!(
            "rep movsb",
            inout("rcx") n => _,
            inout("rdi") dest => _,
            inout("rsi") src => _,
            options(nostack, preserves_flags),
        );
    }
    dest
}

/// # Safety
/// `dest` and `src` are valid for `n` bytes.
#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(
    dest: *mut c_void,
    src: *const c_void,
    n: usize,
) -> *mut c_void {
    if (dest as usize).wrapping_sub(src as usize) >= n {
        // SAFETY: `dest` does not start inside `src`, so a forward copy
        // reads every byte before overwriting it.
        return unsafe { memcpy(dest, src, n) };
    }
    // SAFETY: the caller vouches for both ranges; copying backward with
    // the direction flag set, then cleared as the ABI requires.
    unsafe {
        asm!(
            "std",
            "rep movsb",
            "cld",
            inout("rcx") n => _,
            inout("rdi") dest.cast::<u8>().wrapping_add(n - 1) => _,
            inout("rsi") src.cast::<u8>().wrapping_add(n - 1) => _,
            options(nostack),
        );
    }
    dest
}

/// # Safety
/// `dest` is valid for `n` bytes.
#[unsafe(no_mangle)]
unsafe extern "C" fn memset(
    dest: *mut c_void,
    byte: i32,
    n: usize,
) -> *mut c_void {
    // SAFETY: the caller vouches for the range; `stosb` stores `al`.
    unsafe {
        asm!(
            "rep stosb",
            inout("rcx") n => _,
            inout("rdi") dest => _,
            in("eax") byte,
            options(nostack, preserves_flags),
        );
    }
    dest
}

/// # Safety
/// `a` and `b` are valid for `n` bytes.
#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(
    left: *const c_void,
    right: *const c_void,
    n: usize,
) -> i32 {
    let (left, right) = (left.cast::<u8>(), right.cast::<u8>());
    for i in 0..n {
        // SAFETY: `i < n`, within both ranges.
        let (x, y) = unsafe { (*left.add(i), *right.add(i)) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    0
}

/// # Safety
/// `a` and `b` are valid for `n` bytes.
#[unsafe(no_mangle)]
unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    // SAFETY: the same contract.
    unsafe { memcmp(a, b, n) }
}

// The prebuilt `alloc` is compiled for unwinding and names these; with
// `panic = "abort"` nothing unwinds, so they are never reached.

#[unsafe(no_mangle)]
const extern "C" fn rust_eh_personality() {}

#[unsafe(no_mangle)]
extern "C" fn _Unwind_Resume() -> ! {
    exit(134)
}
