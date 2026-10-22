# Copied VM operations

The runtime copies the VM operations its generated code calls, one VM
family per file of `src/ops/`; the crate's API is unchanged. Each row
names a copy, the crate file it copies from, and the FNV-1a 64 hash of
that file's text when the copy was last reviewed. `tests/drift.rs`
hashes each source and fails when one changed: review the copy against
the change, then re-bless with `FIPE_BLESS=1 cargo test --test drift`.

A row marked *rewritten* is no copy of the VM's code: what the VM does
as a native frame resumed with each value it waits on is plain
recursion here, calling machine code where the VM's frame waits. Its
source is still hashed, and the drift fixtures of the backend
(`../tests/fixtures/drift/`) run each family on the VM and natively.

| Copy | Source | Hash | What |
| --- | --- | --- | --- |
| `src/ops/budget.rs` | `src/vm/budget.rs` | `24bcbccc53499125` | paying nodes and units; every native's size term |
| `src/ops/natives.rs` | `src/vm/natives.rs` | `84f2a99b51c39740` | the natives' table by prelude label and declared type; the members a walk consults |
| `src/ops/immediate.rs` | `src/vm/immediate.rs` | `1e7b823b09c5fa2a` | what each native does |
| `src/ops/immediate.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | a leaf value |
| `src/ops/slots.rs` | `src/vm/assign.rs` | `554ce44f797d1b69` | `Construct`, `Tuple`, `Array`, `Closure`, `Cast` and `Native` into a local |
| `src/ops/slots.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | a field read, a switch's key, an unset global's body run first |
| `src/ops/slots.rs` | `src/vm/globals.rs` | `bf8acd7711df0b27` | the globals a step reads, before it runs |
| `src/ops/calls.rs` | `src/vm/calls.rs` | `9ee2a6a250483583` | the callee kinds, a selected row, eval/apply and its guards, a definition's invocation |
| `src/ops/calls.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | a call terminator: its witnesses and result type substituted |
| `src/ops/calls.rs` | `src/vm/drive.rs` | `8c1145e5023aa21b` | what an invocation runs |
| `src/ops/calls.rs` | `src/vm.rs` | `06381d1929dc5b80` | a host call and its reply |
| `src/ops/mod.rs` | `src/vm.rs` | `06381d1929dc5b80` | a Fault at a node; a frame's witnesses substituted in a type |
| `src/ops/dispatch.rs` | `src/vm/dispatch.rs` | `d28657e669710376` | a row by a witness or a value's tag; a packed receiver unpacked |
| `src/ops/dispatch.rs` | `src/compile/dispatch.rs` | `c63e0e5b0f8219a6` | a row by type or head meaning; the witness substitution (`substitute`, `fixed`, `map`) |
| `src/ops/walk.rs` | `src/vm/walk.rs` | `4a80197bef0ad3a7` | the `eq`, `cmp` and `show` walks by tag, a written member where met |
| `src/ops/walk.rs` | `src/vm/defaults.rs` | `8bf901045ec97e96` | the default of a type, a written `default` where met |
| `src/ops/walk.rs` | `src/vm/pending.rs` | `145d2ecde102ac11` | a written row by a value's tag, and its call |
| `src/ops/higher.rs` | `src/vm/pending.rs` | `145d2ecde102ac11` | *rewritten*: `map`, `fold`, `filter`, `sum` |
| `src/ops/higher.rs` | `src/vm/order.rs` | `23d2ba79df9f369f` | *rewritten*: `sort` (the VM's merge order), `count` |
| `src/ops/predicates.rs` | `src/vm/predicates.rs` | `323b54ccdc703c4f` | *rewritten*: a predicate run, its owed units paid at each leaf |
| `src/ops/entries.rs` | `src/vm/entries.rs` | `c8bf08f77028feaf` | an entry's components tested; a function entry's guards |
| `src/ops/entries.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | the `Enter` statement |
| `src/ops/casts.rs` | `src/vm/casts.rs` | `aadbd4b5a7d742a4` | the casts' rows |
| `src/ops/fits.rs` | `src/vm/fits.rs` | `ca6b093619a2990e` | the shape a reply must have, whole |
| `src/abi.rs` | `src/vm/frames.rs` | `069d714063b34681` | a frame's locals as it opens |
