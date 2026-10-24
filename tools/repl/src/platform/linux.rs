//! Linux on `x86_64` without `std`: raw syscalls, `_start` with its
//! arguments, the panic handler and the allocator. The only unsafe code.
//!
//! The allocator bumps through mapped chunks and reuses freed blocks:
//! a request of alignment at most 16 is rounded up to a power-of-two size
//! class of at least 16 bytes; `dealloc` pushes the block onto its class's
//! free list, threaded through the blocks' first word, and `alloc` pops
//! from that list before bumping. Other alignments bump and are never
//! reused; a `realloc` within one class keeps its block. When a chunk is
//! used up the next is mapped, so a session grows as far as the kernel
//! allows; when a mapping is refused the REPL says so and exits with
//! status 3. Large allocations use anonymous mappings of their own and
//! return them to the kernel when freed, so growing syntax arenas do not
//! strand large blocks in a chunk.

#![allow(unsafe_code)]

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("the `linux` platform is x86_64 Linux only");

use alloc::{string::String, vec::Vec};
use core::{
    alloc::{GlobalAlloc, Layout},
    arch::{asm, naked_asm},
    cell::UnsafeCell,
    ffi::{c_char, c_void, CStr},
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
const CHDIR: usize = 80;
const IOCTL: usize = 16;
const TCGETS: usize = 0x5401;
/// Set after the output written so far is sent.
const TCSETSW: usize = 0x5403;

/// A chunk of the heap; a request larger than this maps its own.
const CHUNK: usize = 64 << 20;
const MAPPED: usize = 64 << 10;
const PAGE: usize = 4096;

const fn mapped(layout: Layout) -> bool {
    layout.size() >= MAPPED && layout.align() <= 4096
}

/// A page-aligned anonymous mapping; null on allocation failure.
unsafe fn map(size: usize) -> *mut u8 {
    let result: isize;
    // SAFETY: anonymous private read/write memory, no address or file supplied.
    unsafe {
        asm!("syscall", inlateout("rax") 9_usize => result,
            in("rdi") 0_usize, in("rsi") size, in("rdx") 3_usize,
            in("r10") 0x22_usize, in("r8") usize::MAX, in("r9") 0_usize,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    if result < 0 {
        ptr::null_mut()
    } else {
        result as *mut u8
    }
}

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

/// Bytes read from stdin into `buffer`; 0 only at end of input.
pub fn read(buffer: &mut [u8]) -> Result<usize, ()> {
    read_fd(0, buffer)
}

fn read_fd(fd: usize, buffer: &mut [u8]) -> Result<usize, ()> {
    loop {
        let (at, len) = (buffer.as_mut_ptr() as usize, buffer.len());
        // SAFETY: the kernel writes at most `len` bytes into `buffer`.
        let read = unsafe { syscall(READ, fd, at, len) };
        if read != -4 {
            return usize::try_from(read).map_err(|_| ());
        }
    }
}

pub fn write(bytes: &[u8]) -> Result<(), ()> {
    write_fd(1, bytes)
}

fn write_fd(fd: usize, mut bytes: &[u8]) -> Result<(), ()> {
    while !bytes.is_empty() {
        let (at, len) = (bytes.as_ptr() as usize, bytes.len());
        // SAFETY: the kernel reads at most `len` bytes from `bytes`.
        let written = unsafe { syscall(WRITE, fd, at, len) };
        crate::advance_write(&mut bytes, written)?;
    }
    Ok(())
}

pub fn read_file(path: &str) -> Option<String> {
    if path.contains('\0') {
        return None;
    }
    let mut name = Vec::with_capacity(path.len() + 1);
    name.extend_from_slice(path.as_bytes());
    name.push(0);
    // SAFETY: `name` is NUL-terminated; flags 0 is `O_RDONLY`.
    let fd = unsafe { syscall(OPEN, name.as_ptr() as usize, 0, 0) };
    let fd = usize::try_from(fd).ok()?;
    let mut text = Vec::new();
    let mut buffer = [0; 4096];
    let success = loop {
        match read_fd(fd, &mut buffer) {
            Ok(0) => break true,
            Ok(read) => text.extend_from_slice(&buffer[..read]),
            Err(()) => break false,
        }
    };
    // SAFETY: `fd` was opened above and is closed once.
    unsafe { syscall(CLOSE, fd, 0, 0) };
    success.then(|| String::from_utf8(text).ok()).flatten()
}

pub fn exit(code: usize) -> ! {
    cooked();
    // SAFETY: `exit` takes no pointer and does not return.
    unsafe { syscall(EXIT, code, 0, 0) };
    unreachable!()
}

/// Whether the working directory is now `path`.
pub fn chdir(path: &str) -> bool {
    if path.contains('\0') {
        return false;
    }
    let mut name = Vec::with_capacity(path.len() + 1);
    name.extend_from_slice(path.as_bytes());
    name.push(0);
    // SAFETY: `name` is NUL-terminated.
    unsafe { syscall(CHDIR, name.as_ptr() as usize, 0, 0) == 0 }
}

/// The kernel's `struct termios`.
#[derive(Clone, Copy)]
#[repr(C)]
struct Termios {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; 19],
}

const ICRNL: u32 = 0o400;
const IXON: u32 = 0o2000;
const ISIG: u32 = 0o1;
const ICANON: u32 = 0o2;
const ECHO: u32 = 0o10;
const IEXTEN: u32 = 0o100_000;
const VTIME: usize = 5;
const VMIN: usize = 6;

/// The settings of the terminal on `fd`, when it is one.
fn termios(fd: usize) -> Option<Termios> {
    let mut termios = Termios {
        iflag: 0,
        oflag: 0,
        cflag: 0,
        lflag: 0,
        line: 0,
        cc: [0; 19],
    };
    let at = ptr::from_mut(&mut termios) as usize;
    // SAFETY: `TCGETS` writes one `struct termios` at `at`.
    let result = unsafe { syscall(IOCTL, fd, TCGETS, at) };
    (result == 0).then_some(termios)
}

fn set_termios(termios: &Termios) {
    let at = ptr::from_ref(termios) as usize;
    // SAFETY: `TCSETSW` reads one `struct termios` at `at`.
    unsafe { syscall(IOCTL, 0, TCSETSW, at) };
}

/// The terminal settings before [`raw`], restored by [`cooked`].
struct Saved(UnsafeCell<Option<Termios>>);

// SAFETY: the program is single-threaded.
unsafe impl Sync for Saved {}

static SAVED: Saved = Saved(UnsafeCell::new(None));

/// Whether stdin and stdout are both a terminal.
pub fn terminal() -> bool {
    termios(0).is_some() && termios(1).is_some()
}

/// Whether stdin is now in raw mode: no echo, no line buffering, Ctrl-C
/// as a byte; output still maps LF to CR LF.
pub fn raw() -> bool {
    let Some(saved) = termios(0) else {
        return false;
    };
    let mut raw = saved;
    raw.iflag &= !(ICRNL | IXON);
    raw.lflag &= !(ICANON | ECHO | ISIG | IEXTEN);
    raw.cc[VMIN] = 1;
    raw.cc[VTIME] = 0;
    // SAFETY: single-threaded; no other reference to `SAVED` is live.
    unsafe { *SAVED.0.get() = Some(saved) };
    set_termios(&raw);
    true
}

/// The terminal as [`raw`] found it; nothing when it was not switched.
pub fn cooked() {
    // SAFETY: single-threaded; no other reference to `SAVED` is live.
    if let Some(saved) = unsafe { (*SAVED.0.get()).take() } {
        set_termios(&saved);
    }
}

/// Where `_start` found the initial stack: `argc`, then `argv`.
static STACK: AtomicUsize = AtomicUsize::new(0);

/// The arguments after the program's name.
pub fn args() -> Vec<String> {
    let stack = STACK.load(Ordering::Relaxed) as *const usize;
    if stack.is_null() {
        return Vec::new();
    }
    // SAFETY: the kernel's initial stack holds `argc` and then `argc`
    // pointers to NUL-terminated strings, which live for the whole run.
    let argc = unsafe { *stack };
    (1..argc)
        .map(|index| {
            // SAFETY: as above, `index < argc`.
            let arg = unsafe { CStr::from_ptr(*stack.add(1 + index) as _) };
            String::from_utf8_lossy(arg.to_bytes()).into_owned()
        })
        .collect()
}

/// Whether `NO_COLOR` is set to a non-empty value.
pub fn no_color() -> bool {
    let stack = STACK.load(Ordering::Relaxed) as *const usize;
    if stack.is_null() {
        return false;
    }
    // SAFETY: after `argc` and its `argv` pointers and their NULL, the
    // kernel's initial stack holds the environment's pointers to
    // NUL-terminated strings up to a NULL, living for the whole run.
    let mut at = unsafe { stack.add(2 + *stack) };
    loop {
        // SAFETY: as above; `at` stops at the NULL.
        let entry = unsafe { *at };
        if entry == 0 {
            return false;
        }
        // SAFETY: as above.
        let entry = unsafe { CStr::from_ptr(entry as _) }.to_bytes();
        if let Some(value) = entry.strip_prefix(b"NO_COLOR=") {
            return !value.is_empty();
        }
        // SAFETY: as above, before the NULL.
        at = unsafe { at.add(1) };
    }
}

/// The kernel enters with rsp 16-byte aligned at `argc`; a call
/// realigns it for the Rust function.
#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    naked_asm!(
        "xor ebp, ebp",
        "mov rdi, rsp",
        "call {start}",
        "ud2",
        start = sym start,
    );
}

extern "C" fn start(stack: *const usize) -> ! {
    STACK.store(stack as usize, Ordering::Relaxed);
    #[cfg(test)]
    crate::output_worker();
    #[cfg(not(test))]
    crate::program();
    exit(0)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    struct Stderr;
    impl fmt::Write for Stderr {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            write_fd(2, text.as_bytes()).map_err(|()| fmt::Error)
        }
    }
    cooked();
    let _ = writeln!(Stderr, "fipe-repl: {info}");
    exit(101)
}

/// Size classes `16 << k` for every `k` a `usize` size can need.
const CLASSES: usize = usize::BITS as usize - 4;

/// The heap's state; see the module documentation.
struct Heap {
    state: UnsafeCell<State>,
}

struct State {
    /// The current chunk's unused part, as addresses; empty at first.
    next: usize,
    end: usize,
    free: [*mut u8; CLASSES],
}

/// No memory is left: say so and exit, never panic.
fn exhausted() -> ! {
    let _ = write_fd(2, b"fipe-repl: out of memory\n");
    exit(3)
}

/// A layout's size class, when its block may be reused.
fn class(layout: Layout) -> Option<usize> {
    if layout.align() > 16 {
        return None;
    }
    let size = layout.size().max(16).checked_next_power_of_two()?;
    Some(size.trailing_zeros() as usize - 4)
}

impl Heap {
    /// A fresh block from the current chunk, or from a new one when
    /// that is used up; the old chunk's tail is left behind.
    ///
    /// # Safety
    /// No other reference to the state is live.
    unsafe fn bump(&self, size: usize, align: usize) -> *mut u8 {
        // SAFETY: the program is single-threaded; the caller vouches.
        let state = unsafe { &mut *self.state.get() };
        let start = state.next.next_multiple_of(align);
        if let Some(end) = start.checked_add(size) {
            if end <= state.end {
                state.next = end;
                return start as *mut u8;
            }
        }
        let length =
            CHUNK.max(size.saturating_add(align)).next_multiple_of(PAGE);
        // SAFETY: an anonymous mapping of `length` bytes, page-aligned.
        let chunk = unsafe { map(length) };
        if chunk.is_null() {
            exhausted();
        }
        let start = (chunk as usize).next_multiple_of(align);
        state.next = start + size;
        state.end = chunk as usize + length;
        start as *mut u8
    }
}

// SAFETY: the program is single-threaded.
unsafe impl Sync for Heap {}

// SAFETY: a block is aligned, in bounds, at least its layout's size, and
// owned by one caller: a freed block sits on one list until popped once.
unsafe impl GlobalAlloc for Heap {
    #[expect(
        clippy::cast_ptr_alignment,
        reason = "a block of a class is 16-aligned"
    )]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            let Some(size) = layout.size().checked_next_power_of_two() else {
                return ptr::null_mut();
            };
            // SAFETY: a mapping is page-aligned, enough for this layout.
            let block = unsafe { map(size) };
            if block.is_null() {
                exhausted();
            }
            return block;
        }
        let Some(k) = class(layout) else {
            // SAFETY: no reference to the state is live.
            return unsafe { self.bump(layout.size(), layout.align()) };
        };
        // SAFETY: single-threaded; the reference ends before `bump`.
        let head = unsafe { &mut (*self.state.get()).free[k] };
        if head.is_null() {
            // SAFETY: as above.
            return unsafe { self.bump(16 << k, 16) };
        }
        let block = *head;
        // SAFETY: a listed block is 16-aligned, at least 16 bytes, and
        // holds the next block of its list in its first word.
        *head = unsafe { *block.cast::<*mut u8>() };
        block
    }

    #[expect(
        clippy::cast_ptr_alignment,
        reason = "a block of a class is 16-aligned"
    )]
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        if mapped(layout) {
            // SAFETY: this block came from map with this rounded length.
            unsafe {
                syscall(
                    11,
                    block as usize,
                    layout.size().next_power_of_two(),
                    0,
                );
            }
            return;
        }
        let Some(k) = class(layout) else { return };
        // SAFETY: single-threaded; the caller gives up `block`, which was
        // handed out for this class, so its first word is ours.
        unsafe {
            let head = &mut (*self.state.get()).free[k];
            *block.cast::<*mut u8>() = *head;
            *head = block;
        }
    }

    unsafe fn realloc(
        &self,
        block: *mut u8,
        layout: Layout,
        size: usize,
    ) -> *mut u8 {
        // SAFETY: the caller vouches that `size` is valid with this
        // alignment, as for `GlobalAlloc::realloc`.
        let grown =
            unsafe { Layout::from_size_align_unchecked(size, layout.align()) };
        if mapped(layout) == mapped(grown)
            && class(layout).is_some()
            && class(layout) == class(grown)
        {
            return block;
        }
        // SAFETY: the caller's contract for `realloc`, passed on.
        let new = unsafe { self.alloc(grown) };
        if !new.is_null() {
            // SAFETY: both blocks hold at least the smaller size and
            // are distinct, as `block` is still owned by the caller.
            unsafe {
                ptr::copy_nonoverlapping(block, new, layout.size().min(size));
                self.dealloc(block, layout);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Heap = Heap {
    state: UnsafeCell::new(State {
        next: 0,
        end: 0,
        free: [ptr::null_mut(); CLASSES],
    }),
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

/// # Safety
/// `text` is NUL-terminated.
#[unsafe(no_mangle)]
unsafe extern "C" fn strlen(text: *const c_char) -> usize {
    let left: usize;
    // SAFETY: the caller vouches for the terminator, where `scasb` stops;
    // it counts `rcx` down from all ones past the terminator.
    unsafe {
        asm!(
            "repne scasb",
            inout("rcx") usize::MAX => left,
            inout("rdi") text => _,
            in("al") 0_u8,
            options(nostack, readonly),
        );
    }
    !left - 1
}

// The prebuilt `alloc` is compiled for unwinding and names these; with
// `panic = "abort"` nothing unwinds, so they are never reached.

#[unsafe(no_mangle)]
const extern "C" fn rust_eh_personality() {}

#[unsafe(no_mangle)]
extern "C" fn _Unwind_Resume() -> ! {
    exit(134)
}

#[cfg(test)]
pub(super) fn saved() -> bool {
    unsafe { (*SAVED.0.get()).is_some() }
}
