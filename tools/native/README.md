# fipe-native

Fipe's native backend: a checked Fipe program compiled ahead of time to
an x86-64 Linux executable, with no code generator dependency. The
backend (`fipe-native`) lowers each MIR body to its own x86-64
instructions and writes an ELF64 relocatable object; `cc` links it
against the runtime (`fipe-native-rt`, in `runtime/`), which holds the
value operations the machine code calls. The VM is the oracle: a
native run's observation and charged units equal the VM's.

Both packages depend on `fipe` and share formats through `fipe-native-core`;
the core language crate depends on neither backend nor runtime. Target: x86-64 Linux, SysV ABI. Toolchain: Rust 1.98.1 or newer
and `cc` (GCC or Clang) with libc.

## Build

The runtime is a static library the executable links against:

```sh
(cd tools/native/runtime && cargo build --release --locked --offline)
export FIPE_NATIVE_RT=$PWD/target/release/libfipe_native_rt.a
```

Then compile a program:

```sh
cd tools/native
cargo run --release --locked --offline -- \
    build file.fipe -o exe [--host DIR] [--module FILE]... [--object]
```

- `--host DIR` checks the program against the host declaration files
  `DIR/*.fipe` (`commands.fipe` first, then each module by its stem).
- `--module FILE` adds a file module the program may `use`, named by
  the file's stem; repeat it per module.
- `--object` stops at the ELF object `exe` instead of linking.
- `--runtime LIB` names the runtime archive instead of
  `FIPE_NATIVE_RT`.

The link is `cc exe.o libfipe_native_rt.a -lgcc_s -lutil -lrt -lpthread
-lm -ldl -lc -o exe`. The executable carries the program's sources and
recompiles them with `fipe` at startup (about 16 ms); it refuses to run
(exit 2) when they no longer compile to the program it was built from.

## Running

| Variable | Meaning |
| --- | --- |
| `FIPE_BUDGET` | the budget in units; unbounded when unset |
| `FIPE_STACK` | the run's stack in bytes; 256 MiB by default |

Exit code: 0 when the run ends with a value, 1 when it Faults, 2 when
it cannot start or its host pipe fails.

Without a host pipe (fd 3 not open) the std host (`fipe-host`,
[`tools/host/README.md`](../host/README.md)) answers in the process
under its contract: the working directory and the directories of
`FIPE_ALLOW_WRITE` (separated by `:`) are the write roots, the
arguments after the program's name are `cli.args`, stdin is host input
when the last line takes it, each main line goes to stdout or stderr by
the sink policy as the run goes, a Fault to stderr as `fault in
<namespace> at <line>:<column>: <reason>`, and the exit code is the
host's: 0 a value, 1 an `Err` value or a Fault, 2 a refused input. A
declaration the std host does not know Faults the call.

Stack: the program runs on a thread of its own, and native recursion
uses that machine stack, not the budget's frames. A recursion deeper
than `FIPE_STACK` allows overflows it and aborts the process: a
resource limit outside the observation, as running out of memory is.
Raise `FIPE_STACK` or bound the run with `FIPE_BUDGET`.

## Host pipe protocol

A host starts the executable with two pipes: the program writes frames
on fd 3 and reads replies on fd 4. Native code blocks on each reply.

Every frame and reply is a length, then that many payload bytes. Host replies
are limited to 64 MiB of encoded payload, excluding the eight-byte prefix; exactly
64 MiB is accepted. `codec::MAX_REPLY_BYTES` owns this limit. Hosts must call
`encode_reply` and handle `Error::TooLong` BEFORE writing any header. Reduce or
chunk application results through declared operations; never truncate a reply.
This is a new limit on previously unbounded replies and a new public Error variant;
update exhaustive Rust matches. It is not a limit on runtime-to-Host frames or
in-process Standard results, and encoding an already-owned large value can still
allocate before final size refusal.

A short header/body, excessive length, malformed reply or pipe I/O error makes the
transport unusable. The runtime exits 2 without a Done frame: writing Done while a
peer is blocked writing a rejected body could deadlock. Process exit closes all
inherited descriptor copies. A valid Host error or a well-formed wrong-type reply
still produces the ordinary source Fault/Done and exits 1. After any transport
error, hosts must close both pipe ends and reap the child under a deadline, killing
it if necessary; do not retry or fall back to Standard. Supervisors must drain or
file-back stderr. Length bounds do not time out a peer that stops sending a valid
body. Startup's existing no-pipe Standard selection is unchanged.

In a payload:

- a *number* is 8 bytes, an unsigned integer little-endian;
- a *text* is a number (its byte length), then UTF-8 bytes;
- a *list* is a number (its count), then each item.

Indices (declarations, `with` keys, meanings, constructors,
namespaces, nodes) are positions in the checked program's tables
(`fipe::ir::Tables`); a host built on the `fipe` crate compiles the
same unit to read them.

**Value**, a tag number then its payload:

| Tag | Value | Payload |
| --- | --- | --- |
| 0 | `()` | none |
| 1 | an integer that fits `I64` | 8 bytes, two's complement LE |
| 2 | any other integer | number (1 negative, 0 not), text of the decimal magnitude |
| 3 | `F32` | number whose low 32 bits are the float's bits |
| 4 | `F64` | number holding the float's bits |
| 5 | `String` | text |
| 6 | a sequence (`Vec`, `Deque`, array) | list of values |
| 7 | a tuple | list of values |
| 8 | a record (sums, `Bool`, `Maybe` too) | number meaning, number constructor, list of field values |
| 9 | retired packed encoding | rejected (`Tag(9)`) |
| 10 | a function | none; shows as `<function>` |
| 11 | a packed value | complete source type, complete target type, value |
| 12 | a shared data container | number: completed container ID in this message |

Packed tag 11 carries descriptive type syntax, not resource identity or membership
proof. Its immutable payload must be resource-free. Decoding requires no Tables;
the old head-only tag 9 is rejected rather than silently upgraded. Other value
tags keep their existing meanings. Both ends must use the same prerelease format.

Sequences, tuples, records and Packed values receive implicit IDs in completion
(postorder) order, starting at zero. Their first occurrence uses its ordinary
encoding above; a later alias may use tag 12 and that ID. A reference introduces
no new ID. Only already completed containers can be referenced: forward,
cyclic and unknown references are malformed. No function or witness is a
reference target. A shown function inside a container prevents that container
from being referenced where functions are forbidden. This policy is derived
from the content, never asserted by a wire flag.

One memo spans all arguments, environment values and lines of a Call, or the
result and lines of a Done; each reply has its own memo. IDs reset for every
message and cannot name an earlier frame. Aliases carry data only: Packed
metadata remains descriptive and decoding never restores its private membership
or resource association. Genuine resources remain forbidden throughout a frame.

Value depth is at most 256: a leaf has depth 0, a container has one plus the
maximum depth of its children. A reference must also fit at the depth where it
is inserted. Over-depth values are rejected symmetrically on encoding and
decoding. This explicit bound covers codec-owned/returned Value graphs and their
cleanup, not arbitrary Values a host constructs. Type syntax inside Packed is
separate: its existing iterative reader and guarded ownership continue to admit
deep Type roots (including 4096 levels).

Ordinary unshared encodings within this depth bound remain byte-for-byte
compatible. Older readers reject new references as unknown tag 12; there is no
silent downgrade or cross-message upgrade. Previously over-depth legacy trees
are now explicitly refused. Frame/reply outer tags and byte-length framing are
unchanged.

Each type starts with a number tag: 0 Unit; 1 Parameter and 2 Session followed by
an index; 3 Number followed by decimal magnitude text; 4 Meaning followed by its
index and list of type arguments; 5 Tuple, 8 Packed alternatives, 9 All, and 11 Sum
followed by a list of types; 6 Array followed by element and length; 7 Function
followed by parameter and result; 10 Not followed by its inner type. These types
describe a value in its source index context. Arbitrary indices are not authority
to execute a resource-bearing member. Native execution still refuses resource
programs and genuine resource values.

A function crosses only where the host shows it: in a main line or the
run's result, as tag 10. In a request's arguments or `with`
environment it cannot cross, and the call Faults `Host`; a reply may
not hold one. A witness never crosses.

**Frames**, program to host on fd 3, a tag number first:

- `0` call: number declaration, list of argument values, the `with`
  environment as a list of (number key, value), then the main lines
  kept so far as a list of (number node, value). The lines are
  cumulative: a host showing them writes only those past the last
  frame's. A reply must follow on fd 4.
- `1` done: number 0 then the result value, or number 1 then a Fault;
  every main line kept, as above; number of units charged. The program
  then exits.

A **Fault** is number span start, number span end (byte offsets in its
namespace's source), number namespace, then a reason tag: 0 budget, 1
division by zero, 2 not finite, 3 not exact (text value, text target),
4 a failed limit (text value, number entry), 5 cast, 6 unreachable, 7
host (text message), 8 message (text).

**Reply**, host to program on fd 4: number 0 then the value, which
must have the call's declared result type (otherwise a `Host` Fault),
or number 1 then a text failure message, which Faults `Host` at the
call.

A small host in Python (`host.py`) for the scalar/unshared subset: it answers
every call with the integer 8080 and prints each main line and the result.
This example deliberately refuses Packed and shared-container tags 11/12; use
the full Rust `fipe_native::codec` for the complete protocol. Retired tag 9 is
always rejected, not interpreted as an old Packed value. `python3 host.py exe`
runs `tests/fixtures/first.fipe` built with `--host
../../tests/fixtures/host`.

```python
#!/usr/bin/env python3
# A host for a fipe-native executable: answers each call 8080, prints
# each main line and the result. Usage: host.py EXE [ARG]...
import os, signal, struct, sys, time

def num(b, i): return struct.unpack_from('<Q', b, i)[0], i + 8
def text(b, i): n, i = num(b, i); return b[i:i + n].decode(), i + n
def value(b, i):  # a value's display, and the offset past it
    tag, i = num(b, i)
    if tag == 0: return '()', i
    if tag == 1: return str(struct.unpack_from('<q', b, i)[0]), i + 8
    if tag == 2: neg, i = num(b, i); m, i = text(b, i); return '-' * neg + m, i
    if tag in (3, 4): bits, i = num(b, i); return f'<float {bits:#x}>', i
    if tag == 5: s, i = text(b, i); return repr(s), i
    if tag == 9: raise ValueError('retired Packed tag 9')
    if tag in (11, 12): raise ValueError('use the full Rust codec for Packed/shared values')
    if tag == 10: return '<function>', i
    if tag not in (6, 7, 8): raise ValueError(f'unknown value tag {tag}')
    head = ''
    if tag == 8: m, i = num(b, i); c, i = num(b, i); head = f'#{m}.{c}'
    n, i = num(b, i); parts = []
    for _ in range(n): v, i = value(b, i); parts.append(v)
    return head + '[(('[tag - 6] + ', '.join(parts) + ']))'[tag - 6], i

MAX_REPLY_BYTES = 64 * 1024 * 1024

def exact(stream, count):
    chunks = bytearray()
    while len(chunks) < count:
        part = stream.read(count - len(chunks))
        if not part: raise EOFError('native pipe ended inside a frame')
        chunks.extend(part)
    return chunks

def reap(pid):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        found, status = os.waitpid(pid, os.WNOHANG)
        if found: return status
        time.sleep(0.01)
    try: os.kill(pid, signal.SIGKILL)
    except ProcessLookupError: pass
    os.waitpid(pid, 0)
    raise TimeoutError('native peer did not exit after transport closure')

req_r, req_w = os.pipe(); rep_r, rep_w = os.pipe()
pid = os.posix_spawn(sys.argv[1], sys.argv[1:], os.environ, file_actions=[
    (os.POSIX_SPAWN_DUP2, req_w, 3), (os.POSIX_SPAWN_DUP2, rep_r, 4)])
os.close(req_w); os.close(rep_r)
frames, replies = os.fdopen(req_r, 'rb'), os.fdopen(rep_w, 'wb')
try:
    while True:
        length = struct.unpack('<Q', exact(frames, 8))[0]
        b = exact(frames, length)  # This example trusts its generated scalar peer.
        tag, i = num(b, 0)
        if tag == 1: break  # done
        declaration, _ = num(b, i)
        print(f'call {declaration} -> 8080')
        reply = struct.pack('<QQq', 0, 1, 8080)
        if len(reply) > MAX_REPLY_BYTES: raise ValueError('native reply too long')
        replies.write(struct.pack('<Q', len(reply)) + reply); replies.flush()
finally:
    # Also runs on malformed frames, pre-header refusal or a failed write.
    try: replies.close()
    finally:
        try: frames.close()
        finally: status = reap(pid)
ok, i = num(b, i)
if ok == 0: result, i = value(b, i)
else:  # a Fault: span, namespace, reason, the reason's payload
    (start, end, ns, reason), i = struct.unpack_from('<4Q', b, i), i + 32
    if reason in (3, 4, 7, 8): _, i = text(b, i)
    if reason == 3: _, i = text(b, i)
    if reason == 4: _, i = num(b, i)
    result = f'fault {reason} at bytes {start}..{end} of namespace {ns}'
n, i = num(b, i)
for _ in range(n): _, i = num(b, i); line, i = value(b, i); print('line', line)
charged, _ = num(b, i)
print('result', result, '| charged', charged, '| exit', os.waitstatus_to_exitcode(status))
```

`tests/equivalence.rs` is the conformance host written against the
crate.

## Equivalence and drift

`tests/equivalence.rs` compiles each program once, runs it on the VM
under the conformance host (`../../tests/host/`), builds and runs the
executable as a child under the same host over the pipe, and compares
the observations (value or located Fault, ordered host calls and
replies, success and error bytes) and the units charged, unbounded and
under budgets that stop the run partway. The six examples and
`tests/fixtures/first.fipe` also equal their trace fixtures byte for
byte. A function in a result compares as the pipe carries it.

The runtime copies the VM operations it calls: one VM family per file
of `runtime/src/ops/`, each listed in `runtime/COPIED.md` with its
source file in the crate and that file's FNV-1a hash. `runtime/tests/
drift.rs` fails when a source changes. Review the copy against the
change, then re-bless the ledger:

```sh
(cd tools/native/runtime && FIPE_BLESS=1 cargo test --locked --offline --test drift)
```

Each family has a program in `tests/fixtures/drift/` run on the VM and
natively. `FIPE_BLESS=1` also rewrites the golden instruction listings
in `tests/golden/`.

## Checks

`FIPE_BACKEND=llvm cargo test --test equivalence` runs the same suite
on objects from the LLVM backend ([`tools/llvm`](../llvm/README.md)),
which links this runtime.

From each of `tools/native` and `tools/native/runtime`: `cargo fmt
--check`, `cargo test --locked --offline` (also `--release`), `cargo
clippy --locked --offline --all-targets -- -D warnings -D clippy::all
-D clippy::pedantic -D clippy::nursery`, `cargo doc`. Then
`sh tools/native/scripts/probes.sh` checks the depgraph, the unsafe
boundary (only `runtime/src/abi.rs`) and the link libraries.

`tests/speed.rs` measures the VM against the native backend on a deep
recursion, `fib` and a `map`/`fold` loop; it is ignored, a measurement
and not a gate:

```sh
cargo test --release --locked --offline --test speed -- --ignored --nocapture
```

See the [support policy](../../Documentation/development.md#compatibility-and-support)
for the Rust 1.98.1 floor and platform qualification status.
Executable support is x86_64-unknown-linux-gnu only. Match compiler, generated
artifacts and runtime to the exact Fipe version; this policy does not imply a
mechanical mismatched-version rejection guard.


Checked programs with resource declarations are refused by native execution,
including private or unused declarations. Native `lower`, `object`, `listing`
and LLVM `module` return `Result<_, String>`; callers must propagate refusal.
`Unit::new` and `Unit::compile` also refuse. Raw `Unit` source metadata still
encodes/decodes unchanged; transporting sources does not certify executable
support. Startup refuses reconstructed marked sources before opening the Host,
reading input or calling generated code. Raw assembler/ELF and LLVM text APIs
remain low-level interfaces without a checked-program support guarantee.

Receiver-first method adapters use the existing body/call ABI. Their active
locators and immutable invocation contexts participate in the compiler/runtime
fingerprint along with bodies. Native reconstruction must reproduce these
collections before generated code runs; this adds no portable artifact or
cross-version compatibility promise.
