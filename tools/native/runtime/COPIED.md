# Copied VM operations

The runtime copies the VM operations its generated code calls, one VM
family per file of `src/ops/`; the crate's API is unchanged. Each row
names a copy, the crate file it copies from, and the FNV-1a 64 hash of
that file's text when the copy was last reviewed. `tests/drift.rs`
hashes each source and fails when one changed: review the copy against
the change, then re-bless with `FIPE_BLESS=1 cargo test --test drift`.

| Copy | Source | Hash | What |
| --- | --- | --- | --- |
| `src/ops/budget.rs` | `src/vm/budget.rs` | `24bcbccc53499125` | paying nodes and units; the integer size terms |
| `src/ops/natives.rs` | `src/vm/natives.rs` | `84f2a99b51c39740` | the natives' table by prelude label |
| `src/ops/natives.rs` | `src/vm/immediate.rs` | `1e7b823b09c5fa2a` | the `Integer` operations |
| `src/ops/natives.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | a leaf value |
| `src/ops/slots.rs` | `src/vm/assign.rs` | `554ce44f797d1b69` | `Construct` and `Native` into a local |
| `src/ops/slots.rs` | `src/vm/step.rs` | `1cf62f78d5e61be5` | a field read, a switch's key |
| `src/ops/calls.rs` | `src/vm/calls.rs` | `9ee2a6a250483583` | a `Native` callee, a host's row among them |
| `src/ops/calls.rs` | `src/vm.rs` | `06381d1929dc5b80` | a host call and its reply |
| `src/ops/mod.rs` | `src/vm.rs` | `06381d1929dc5b80` | a Fault at a node |
| `src/ops/fits.rs` | `src/vm/fits.rs` | `ca6b093619a2990e` | the shape a reply must have, whole |
| `src/abi.rs` | `src/vm/frames.rs` | `069d714063b34681` | a frame's locals as it opens |
