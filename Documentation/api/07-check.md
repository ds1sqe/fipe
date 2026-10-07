# Checking and the intermediate form

[Checking](../spec/07-checks.md) precedes every program effect.

~~~text
check::check(module, declarations, files, options)
    -> Result<ir::Unit, Vec<check::Error>>
check::Options { strict: bool }
~~~

The inputs are parsed modules and loaded declarations, as for resolution
and inference. The file-loading request loop belongs to `Session`.
Resolution, inference, limits and coverage precede lowering. An error
carries its stage and, for a file module, its stem. No unit accompanies an
error. `resolve` and `infer` remain public; only `check` gives a unit.
Lowering cannot fail: checking has already recorded every choice.

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
