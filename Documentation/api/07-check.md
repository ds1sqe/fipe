# Checking and the intermediate form

[Checking](../spec/07-checks.md) precedes every program effect.

~~~text
check::check(module, declarations, files, options)
    -> Result<ir::Unit, Vec<check::Error>>
check::Options { strict: bool, compile_budget: u64 }
check::COMPILE_BUDGET: u64
check::Error::Compile { span, what: check::Evaluated, reason: value::Reason }
check::Evaluated = Entry { limit: String, value: Option<String> }
                 | Literal { meaning: String } | Demand
                 | Sequence { meaning: String }
check::Settled { check: typed::Check, value: String }
~~~

The inputs are parsed modules and loaded declarations, as for resolution
and inference. The file-loading request loop belongs to `Session`.
Resolution, inference, limits and coverage precede lowering. An error
carries its stage and, for a file module, its stem. No unit accompanies an
error. `resolve` and `infer` remain public; only `check` gives a unit.
Lowering cannot fail: checking has already recorded every choice.

## The compile phase

After lowering, the checker runs pure code on the machine
([MIR](08-mir.md)), each evaluation a node of the unit lowered alone as
the main of a program, on a fresh
`vm::Machine::new(Some(compile_budget))`: the budget is an allowance per
evaluation, counted in the checker's own steps, which are this machine's
units ([MIR](08-mir.md) §Running), and part of the checked unit.
`Options::default()` takes `COMPILE_BUDGET`, 2^20. In order:

1. Each literal whose reader runs, a program's `literal` or a prelude
   reader whose value is not finite, and each read or number cast of a
   written literal by the prelude's reader. A Fault is `Error::Compile`
   with `Evaluated::Literal`; a generic body's reader, which needs its
   caller's witness, runs when the literal does.
2. Each bracket of exact elements handed to a program's `sequence`. A
   Fault is `Error::Compile` with `Evaluated::Sequence`; a bracket of
   other elements runs its `sequence` when it is evaluated.
3. Each `Compile (value)` and each value a binding's place demands with
   `Compile & T`. A Fault, or the budget's end, is `Error::Compile` with
   `Evaluated::Demand`: a demanded value that cannot be ready is a check
   error. The value is exact from then on: the expression and its
   binding know it, so a later line reads it as exact.
4. Each plan entry with the reason `CompileTimeEvaluation`: its value,
   then its test on that value.
   - Proved: the entry leaves the plan and the open checks; a session's
     report lists it among `settled` with its value, and its `Enter`
     node keeps only the value, so no `Compile` entry reaches a body.
   - Refuted, or a Fault while computing the value or running the test:
     `Error::Compile` at the entry's place. `Evaluated::Entry` names the
     limit and the value tested, when computing it did not Fault;
     `reason` is the Fault, `Failed` for a refutation. A refuted entry
     inside the evaluated code is reported against its own limit, a
     refuted divisor as `DivideByZero`.
   - The budget runs out: the entry stays open with the reason
     `CompileBudget`, "compile budget exhausted", and is tested when it
     runs; a test written `Compile (test)` is then a check error.
   - A value that reads a local slot, a parameter, a witness its caller
     supplies or a part a destructuring line stores is not evaluated
     alone; its entry stays open with its reason.

A value with no closure and no witness becomes `ir::Constant` data in
`Tables.prepared`, by node index; the compiler lowers a prepared node as
that constant. Strict mode refuses only the entries the phase leaves
open. Purity was checked before, so an evaluation that reaches a host
declaration is a crate defect and panics.

## Units

A unit contains indexed tables of meanings, definitions, limit tests and
used modules, its main lines and its plan of open entries. Modules run in
dependency order, following the order of their first `use`. Every expression
keeps its source span and `ir::Type`; meanings are indices, never names to
resolve. A backend needs neither syntax, resolved names nor the typed tree.

The unit declares the main's free meaning parameters, such as the element
meaning of `[]` or `Nothing`. These unconstrained parameters never supply
a hidden meaning a body needs, so running the main needs no argument for
them. No default element meaning is invented.

A generic definition has one body. A member with no receiver to inspect,
such as `Self.literal`, `Self.read` or `Self.zero`, takes a hidden meaning
parameter; other generic members dispatch on the value's tag. A definition
reference supplies those hidden parameters. `Local` and `Capture` references
supply them in the same way, including when a function is passed without
being called. A function body declares its hidden parameters. Supplying
meanings preserves the closure's captured environment; it never rebuilds
that environment from the caller's slots. A nongeneric reference or a
lambda-bound name supplies no hidden meanings.
A definition records the value slot holding its closure, so indexed
references retain the environment created by its binding.

Each call owns its instantiated defaults, keyed by call and spot; a default
may read the spots filled before it and module-level names, never an
enclosing function's locals. This does not duplicate a
generalized function's body or introduce a default thunk. Exact constants
and literals share the same indexed node across calls.

The unit also holds indexed member-dispatch row sets. A row records its
receiver tag, the receiver spot when there is one, an overload argument
meaning when needed, and a definition
index, supplied or host declaration index, or a derived operation (`eq`,
`cmp`, `show`, `default`) over the type table. `Dispatch` references its
row set by index; a known head references its definition directly. A row
may instead target a field index in its receiver layout: the dot reads a
field first, else a member. A known record head uses `Field` directly.
`Derived { kind, ty }` directly references a derived operation at a known
head and is called like any other callable.

## Nodes

| Node | Recorded decision |
| --- | --- |
| Literal | written form, meaning, and its literal member |
| Local, Capture | value slot and hidden meanings |
| Definition | definition index and hidden meanings |
| Declared | supplied or host namespace and declaration index |
| Construct | meaning, constructor, fields in declaration order |
| Constructor | callable constructor and physical field index per spot |
| Ordering | checked comparison result and accepted constructor indices |
| Field | receiver and field index |
| Tuple, Array | ordered parts |
| Dispatch | member selected by a value tag or a supplied hidden meaning |
| Call | callee; owned value slots; supplies in source order with spots; defaults and open spots in parameter order |
| Function | hidden parameters, captures and ordered arms, with patterns, guards and bodies |
| If, Block, Bind, With | their checked parts |
| Lift | good-side binding, value and remaining body |
| Marked | selected side, left, stage and rebuild: direct, wrap or discard |
| Or, And | fixed Bool, same-family or payload mode |
| Cast | closed cast-table row, both meanings and selected reader or show operation |
| Pack | value's meaning as tag |
| Enter | shared entry index, component path and checked value |
| Repeat | array length and the default operation for each element |
| Timing | compile or initialization demand and operand |

A bracket at a place whose type claims `Sequence` by a program's member
is a `Call` of that member, its one spot the `Array`; at the prelude's
`Vec`, `Deque` or an array it is the `Array` itself.

Patterns are `Any`, `Slot`, `Literal`, `Constructor`, `Record`, `Side`,
`Gate` with its tests, `Tag`, `List` and `Tuple`. A pipe stage remains a node
so a debugger can step by stage: a stage that calls lists the piped value,
cast to its spot, among its supplies and is typed as its result; a stage
that is a function value keeps its function type and the pipe applies it.
The plan retains the phase, reason and frequency of each open test.

A session unit holds its current line and indexes tables the session keeps.
`Unit::tables()` reads those shared tables, including `session_meanings`;
an unfixed meaning slot reads as `None`.
Earlier values are evaluation's responsibility. A weak monomorphic type
uses a session-owned meaning slot, distinct from a definition's bound
parameter. Its state is readable and may be unfixed; a later checked line
fills it, and prior units see that meaning without being lowered again.
A file unit has no session meaning slots. `:ir` displays a checked
line in words on both REPL builds, without internal byte offsets.

A literal pattern retains its prepared literal and its meaning's selected
`eq` callable, including the receiver spot. Preparation uses `Literal`;
comparison requires `Eq`, as it does for the equality operator.

`Convert` is a callable adapter around the selected conversion. It records
whether to return directly, wrap in `Ok`, Fault on `Err`, or perform a
bounded numeric check, and retains the parameter and target predicates.
Gate predicates compose indexed tests with conjunction, disjunction and
complement; each generic test reference carries its meaning arguments.

Record defaults carry indexed constructor calls, including private fields;
field defaults read the owning call's earlier slots. A written `Default`
member is selected directly. Array defaults retain a repeated element
operation and their checked length. No backend looks up a field default.
An entry's component path distinguishes record fields, tuple parts,
container elements and outcome payloads. `Enter` indexes `Tables.entries`,
so an older session definition keeps its entry identities in later units.

A sum's unknown constructor uses a `CaseField` entry projection: visit
that field only for the indexed constructor. Nested `Lift` bodies retain
the type of the remaining sequence, including any later lift.
An open check records its producing expression separately from its
reported place, so a field default is tested on its value. Instantiated
default expressions retain their own indexed entries.
Retracting a session line also removes the dispatch choices learned from
that line. Table identities remain allocated; surviving units keep their
indices.
