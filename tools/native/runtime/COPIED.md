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
| `src/ops/budget.rs` | `src/vm/budget.rs` | `c95c5ea0b38df810` | paying nodes and units; every native's size term |
| `src/ops/natives.rs` | `src/vm/natives.rs` | `4c3f85bda0e51c95` | the natives' table by prelude label and declared type; the members a walk consults |
| `src/ops/immediate.rs` | `src/vm/immediate.rs` | `5f5001ea1f36e6e3` | what each native does |
| `src/ops/immediate.rs` | `src/vm/step.rs` | `02ca45d9cbf41787` | a leaf value |
| `src/ops/slots.rs` | `src/vm/assign.rs` | `9256891091af7936` | every rvalue but a use and a field into a local, including ordered `Admit` callee/argument transport; checked `Convert` modes use `predicates.rs` |
| `src/ops/slots.rs` | `src/vm/step.rs` | `02ca45d9cbf41787` | a field read, a switch's key, an unset global's body run first |
| `src/ops/slots.rs` | `src/vm/globals.rs` | `9d804dba464e8e36` | the globals a step reads, before it runs |
| `src/ops/calls.rs` | `src/vm/calls.rs` | `c4ec8e276183780b` | the callee kinds, selected rows, receiver admission, eval/apply and ordered guard admission, partial/beyond transfers and definition invocation |
| `src/ops/calls.rs` | `src/vm/resource.rs` | `7f91a20e6626da26` | charged iterative semantic C/closure meanings and raw predicate Type materialization: Read/Finish, substitution, limbs, Session modes and guarded partial roots |
| `src/ops/calls.rs` | `src/vm/step.rs` | `02ca45d9cbf41787` | a call terminator: its witnesses and result type substituted |
| `src/ops/calls.rs` | `src/vm/pending.rs` | `af5bd18fb7e68618` | supplied targets/caller result, lazy source-owner/index guard selection and ordered current-argument updates; synchronous instead of resumed frames |
| `src/ops/calls.rs` | `src/vm/drive.rs` | `01a45314db002f16` | what an invocation runs |
| `src/ops/calls.rs` | `src/vm.rs` | `edfa7f137e99c0ee` | a host call and its reply, with the `with` environment and the main lines kept; a module's init body before the main |
| `src/ops/mod.rs` | `src/vm.rs` | `edfa7f137e99c0ee` | a Fault at a node; a frame's witnesses substituted in a type |
| `src/ops/mod.rs` | `src/vm/step.rs` | `02ca45d9cbf41787` | the `Line`, `Store`, `Push` and `Pop` statements; the `Unreachable` Fault |
| `src/ops/dispatch.rs` | `src/vm/dispatch.rs` | `e6b4d274193e09ee` | a row by a witness or a value's tag; a packed receiver unpacked |
| `src/ops/dispatch.rs` | `src/compile/dispatch.rs` | `7b60d1d81c17a66c` | a row by type or head meaning; the witness substitution (`substitute`, `fixed`, `map`) |
| `src/ops/walk.rs` | `src/vm/walk.rs` | `12a03cb906e88b79` | the `eq`, `cmp` and `show` walks by tag, a written member where met |
| `src/ops/walk.rs` | `src/vm/defaults.rs` | `68d67a63d3aa60f6` | the default of a type, a written `default` where met |
| `src/ops/walk.rs` | `src/vm/pending.rs` | `af5bd18fb7e68618` | a written row by a value's tag, and its call |
| `src/ops/higher.rs` | `src/vm/pending.rs` | `af5bd18fb7e68618` | *rewritten*: `map`, `fold`, `filter`, `sum` |
| `src/ops/higher.rs` | `src/vm/order.rs` | `23d2ba79df9f369f` | *rewritten*: `sort` (the VM's merge order), `count` |
| `src/ops/predicates.rs` | `src/vm/predicates.rs` | `a25d6f881c760ed9` | *rewritten*: root snapshot and per-component preparation, immediate leaf payment, creator-bound witnesses; branch and checked conversion verdicts |
| `src/ops/predicates.rs` | `src/vm/resource.rs` | `7f91a20e6626da26` | charged raw Predicate snapshot/preparation traversal; Type events delegate to the existing `calls.rs` engine |
| `src/ops/predicates.rs` | `src/value/resource.rs` | `15ccd591dcf21aee` | iterative owned Predicate forest cleanup; Test meanings drain through the existing Type forest |
| `src/ops/entries.rs` | `src/vm/entries.rs` | `0e13913df15849b8` | projected components and creator-bound guards; ordinary snapshot before empty-component capture/run bypass, function-local attachment before snapshot |
| `src/ops/entries.rs` | `src/vm/step.rs` | `02ca45d9cbf41787` | the `Enter` statement |
| `src/ops/casts.rs` | `src/vm/casts.rs` | `aadbd4b5a7d742a4` | the casts' rows |
| `src/ops/fits.rs` | `src/vm/fits.rs` | `718e23b91f020ceb` | the shape a reply must have, whole |
| `src/ops/fits.rs` | `src/value/resource.rs` | `15ccd591dcf21aee` | shared API dependency, not a copied walker: proof-independent logical data traversal also used by `mod.rs` and `start.rs` admission |
| `src/abi.rs` | `src/vm/frames.rs` | `b47f09afaba72665` | a frame's locals as it opens |

## VM lifecycle/context review (2026-10-23)

The two vm.rs rows were reviewed from source399fb75a68acb455acfb84918d5d9947653a49a0
(FNV41ba18cbe822b93b) to SHA256
cce823e7c106a4c922beb32aa12beff5c7293417da655ef565167665240df1e7.
The delta is an InputError doc break, additive Machine::abandon, and fresh-Tables
cache reset. The copied host/reply, Fault and witness-substitution behavior is
unchanged. Native startup creates and takes one fresh State per Program, with
synchronous Host replies; it has no reusable Machine context or suspended-call
abandon API requiring adaptation. Other provenance rows remain unchanged.

## Receiver callable witness metering review

The receiver-adapter vm.rs/step/assign/calls/entries/pending changes are
mirrored for the new C witness; the separate supplied-result carrier is
identified below. Other ordinary witnesses keep
their prior charging policy. Native calls::copied_callable follows the exact
resource::materialize Read/Finish and fixed_session schedule in one guarded
iterative pass. Dynamic transport precedes row selection; static closure
capture, partial/full invocation, guard attachment and beyond-guard transfer
copy C through its receiver_contexts protocol_result slot. Full projection
stays after receiver admission. Native ABI enter copies the projected result
into its hidden Meaning owner; it does not retain the full C after peeling.
These are source-review anchors; runtime parity and exact-source drift are
separate checks.

## Supplied-result and shared logical policy provenance

The `calls.rs` / `pending.rs` row covers supplied Host target routing and its
actual caller result. That result is materialized and paid even for ordinary
resource-free calls; this is separate from the receiver adapter's C carrier.
Other ordinary raw operation witnesses keep their existing substitution policy.
The `value/resource.rs` row watches the shared logical data traversal API used
by native admission; the runtime calls that API rather than copying a walker.
These registrations distinguish the shared API dependency from copied logic.
Changes to predicate context require a fresh exact-source review before the
recorded hashes are updated.

## Predicate and admission provenance

Predicate and entry copies retain creator-ID witness associations, resolve
current Session meanings even with an empty captured context, and preserve
capture and leaf payment order. The hidden-context ABI is the second argument.
Ordinary empty projected components bypass capture; function guard attachment
does not. Source hashes include private tests, so a private-test-only change can
require a provenance update without changing the copied production behavior.

Native resource-program and resource-value refusal remains separate from
ordinary logical data scan accounting. These hashes do not convey resource
authority or certify every Host contract. Changes to a watched source or its
native consumer require source review before updating the recorded hash.

## Bound argument admission and guard-copy work review

This review uses upstream snapshot
`a364946a39b6150d9a5b5ecb55be58cdd132f460b05be6001224859b90a85491`.
The source hashes cover complete files, including private tests. The shared
dispatch matching/substitution region is unchanged; compiler adapter-demand
payload and fallback changes do not require a duplicate native emitter.
Core suspension/return enums become synchronous ordered argument updates here.
Both paths retain source-owned guard indices, current same-slot values and
creator witnesses, and discharge guards only on private copied aliases.

Ordinary native Entry first owns a charged predicate snapshot, then projects
components. Empty components skip witness capture and runs, not that snapshot.
Function-local Entry attaches guards before an ordinary snapshot is taken.
For branch and checked conversion, native borrowed traversals mirror Core's
snapshot and working preparation. Per-component preparation occurs only when
that component is reached. Native Predicate Type work delegates to
`calls.rs::predicate_type_mode`; it is not a second Type-copy engine. Nonzero's
one-unit payment occurs before any later preparation or test-meaning work.

Raw Predicate/Test syntax preserves Parameter and Session identities. Closure
meaning copies use the reviewed semantic Session mode; the ordinary unguarded
policy is unchanged. Fees precede copied slots/nodes and fallible operations
retain Forest/Types ownership until publication. Final Guard destruction uses
the linked Core value type and iterative predicate/type cleanup; this changes
Rust owned-field move/struct-update compatibility, not value representation,
wire protocol or resource authority.

Source review, home private deep-copy/failure checks and complete home native
and LLVM equivalence targets support this correspondence. The two full targets
each passed 69 primary tests, including every drift fixture, on this snapshot
(proof `1988eee784485634768590c06f31379539377b0888c099b51f1d16d37a6ba42d`).
Native `holds` still uses recursive execution; iterative copying/destruction is
not a claim of arbitrary predicate execution depth. Public Session producer,
backend/platform and release gates remain separately tracked. Refreshed hashes
record reviewed upstream bytes, not source equality or universal safety.

## CI helper-move provenance reconciliation

The preceding bound-admission review is historical a364 evidence. That helper-move review used source identity
`06db593ccac2b6353c2ab8781e6eda081867f531952c62e1937b1e09715215b4`.
Hashes cover complete source files, including private tests.
Core helper moves retain the reviewed operand, payment, owner and publication
order. Later Field extraction and selected-row unpack helpers preserve packed
validation, conditional payment and receiver-false behavior; native remains the
reviewed resource-free/refusal specialization.

Native deferred-guard appending retains the source owner across every copied
guard, after the first returned-closure copy succeeds. Closure-meaning extraction
returns owned Types while captures remain outer. Predicate Finish extraction keeps
charges and the no-output bypass in the outer loop; no operation price changes.
These helper-only correspondences add no resource transport or execution-depth
claim. Private-test and startup-helper cleanup changes provenance bytes without
changing the copied production behavior. Recorded hashes identify the reviewed
source; they do not replace exact-source drift or backend qualification.

## Composite-result projection provenance

ResourceStep and the projection errors are additive Rust embedding APIs. Frozen
result paths and their Budget refusal belong to a suspended Machine call; only
explicit pending-result selection can latch that refusal. Native execution rejects
marked Programs and has no pending getter. Its ordinary synchronous host reply,
Fault attribution, predicate preparation and Type materialization remain unchanged.
The fallible frozen-stamp lookup constructs resource descriptors, not native Type
copies. Shared logical-data traversal and iterative forest cleanup are unchanged.
These reviewed source hashes add no native resource transport or authority.
