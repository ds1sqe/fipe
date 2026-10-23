# fipe-native

Fipe's native backend: a checked Fipe program compiled ahead of time to
an x86-64 Linux executable, with no code generator dependency. The
backend (`fipe-native`) lowers each MIR body to its own x86-64
instructions and writes an ELF64 relocatable object; `cc` links it
against the runtime (`fipe-native-rt`, in `runtime/`), which holds the
value operations the machine code calls. The VM is the oracle: a
native run's observation and charged units equal the VM's.

Both packages depend on the `fipe` crate by path; the crate depends on
neither. Target: x86-64 Linux, SysV ABI. Toolchain: a Rust toolchain
and `cc` (GCC or Clang) with libc.

## Build

The runtime is a static library the executable links against:

```sh
(cd tools/native/runtime && cargo build --release --locked --offline)
export FIPE_NATIVE_RT=$PWD/tools/native/runtime/target/release/libfipe_native_rt.a
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

Every frame and reply is a length, then that many payload bytes. In a
payload:

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
| 9 | a packed value | number meaning, value |
| 10 | a function | none; shows as `<function>` |

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

A host in Python (`host.py`): it answers every call with the integer
8080 and prints each main line and the result. `python3 host.py exe`
runs `tests/fixtures/first.fipe` built with `--host
../../tests/fixtures/host`.

```python
#!/usr/bin/env python3
# A host for a fipe-native executable: answers each call 8080, prints
# each main line and the result. Usage: host.py EXE [ARG]...
import os, struct, sys

def num(b, i): return struct.unpack_from('<Q', b, i)[0], i + 8
def text(b, i): n, i = num(b, i); return b[i:i + n].decode(), i + n
def value(b, i):  # a value's display, and the offset past it
    tag, i = num(b, i)
    if tag == 0: return '()', i
    if tag == 1: return str(struct.unpack_from('<q', b, i)[0]), i + 8
    if tag == 2: neg, i = num(b, i); m, i = text(b, i); return '-' * neg + m, i
    if tag in (3, 4): bits, i = num(b, i); return f'<float {bits:#x}>', i
    if tag == 5: s, i = text(b, i); return repr(s), i
    if tag == 9: _, i = num(b, i); return value(b, i)
    if tag == 10: return '<function>', i
    head = ''
    if tag == 8: m, i = num(b, i); c, i = num(b, i); head = f'#{m}.{c}'
    n, i = num(b, i); parts = []
    for _ in range(n): v, i = value(b, i); parts.append(v)
    return head + '[(('[tag - 6] + ', '.join(parts) + ']))'[tag - 6], i

req_r, req_w = os.pipe(); rep_r, rep_w = os.pipe()
pid = os.posix_spawn(sys.argv[1], sys.argv[1:], os.environ, file_actions=[
    (os.POSIX_SPAWN_DUP2, req_w, 3), (os.POSIX_SPAWN_DUP2, rep_r, 4)])
os.close(req_w); os.close(rep_r)
frames, replies = os.fdopen(req_r, 'rb'), os.fdopen(rep_w, 'wb')
while True:
    b = frames.read(struct.unpack('<Q', frames.read(8))[0])
    tag, i = num(b, 0)
    if tag == 1: break  # done
    declaration, _ = num(b, i)
    print(f'call {declaration} -> 8080')
    reply = struct.pack('<QQq', 0, 1, 8080)  # a value: the I64 8080
    replies.write(struct.pack('<Q', len(reply)) + reply); replies.flush()
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
print('result', result, '| charged', charged, '| exit', os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1]))
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
