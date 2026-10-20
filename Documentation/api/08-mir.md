# The MIR and its compiler

[Evaluation](../spec/08-evaluation.md) runs a lowered program; the MIR is
the one form every backend consumes.

~~~text
compile::compile(&unit) -> mir::Program
~~~

The input is a checked unit, as `check::check` or a session returns it.
Lowering is pure and cannot fail: the checker recorded every choice, so
the compiler panics only on a broken checker invariant, a crate defect.
The VM, the native backend and an LLVM backend consume the same program
and the runtime's values; each meters its own budget.

## Programs

A program holds its bodies, its prepared constants, the body index of
each definition with spots (`None` for a value definition, which is a
global, or one the program does not reach), the body of each limit test
a gate or an entry reaches, each used module's init body in the order
the checker recorded, and the main body. Every body reachable from the
main is lowered once: called definitions, closures, the targets of every
row of a run-time dispatch, and limit tests, transitively. A session's
units share tables, so a later line's program lowers again any earlier
definition it reaches; the values of earlier lines live in the machine.
Every reference is an index into the program or into `Unit::tables()`;
no name is executable and the MIR carries no string.

## Bodies and locals

A body lowers one node: a function, a limit test, a module's lines, the
main lines, or a thunk the compiler made. It has an arity, the number of
spots; a capture count, what a closure of it supplies; and its hidden
list, the meaning parameters whose witness the caller supplies, in order.
A `Type::Parameter` met at run time names the witness at that parameter's
position in the hidden list. Local `_0` is the result; locals `1..=arity`
are the spots; then come bound names, a call's spots and temporaries.
Each local keeps its type and the IR slot it holds, `None` for a temporary.

## Blocks, statements and terminators

A block is statements and one terminator, with the node of each: a Fault has
its span and a debugger steps by node or by block without a second table.
Statements assign an rvalue to a local, store a value definition's global,
enter an `Eval` or `Init` entry with the value it checks, push a `with` key and
value onto the environment a host call receives, pop it, or keep a main
expression line's value (`Line`). A block whose lowering charged anything opens
with `Charge`, the nodes it pays in the order their lowering completed.

The charges are the VM's metering, which [the host
chapter](../spec/09-host.md) §Budget leaves to each backend: one unit
per IR node evaluated, the size terms living inside the natives and the
arithmetic. A node is charged in the block where its lowering
completes, so a block's charge covers every node that leaves a statement
in it; a node a backend evaluates again, such as a callee or a shared
constant, is charged at each site. A branch's own node is charged on each
edge, never before the switch, so a side that is skipped costs nothing.

Terminators go to a block; switch on a value's constructor, with each
case naming a block and an optional `otherwise` (on a packed value the
cases are meaning indices, on an integer the numbers); test a value
against a gate's predicate with a pass and a fail block; call a callee
with arguments and the callee's witnesses, landing the result in a local
and continuing at a block; return `_0`; or mark a block coverage proves
unreachable, which reaching is a crate defect. Nothing is caught, so no
terminator has a cleanup edge: a Fault ends the run where it happens.

## Operands and rvalues

An operand reads a local, a prepared constant, a captured value, a value
definition's global, or a hidden witness as a `Value::Meaning`. An rvalue
uses an operand; constructs a record from fields in declaration order;
builds a tuple, an array, or a repeated element with a fixed length or
one read from a witness; reads a field, a tuple part, an element, or a
packed payload at index zero; reads a sequence's length; makes a closure
of a body with captured operands and the meanings the frame substitutes;
wraps a stage's result by a recorded rebuild; casts by a closed table row
with both meanings, after the selected reader or show operation was
called; adapts a conversion by its recorded mode with the target
predicate, after the selected member was called; packs a value under its
tag; or applies a prelude operation to arguments, which may Fault and
never suspends.

## Callees

| Callee | What a backend does |
| --- | --- |
| Body | enters the body with the arguments as its spots and the witnesses as its hidden meanings |
| Native | performs the prelude declaration's operation, mapped by its label once per tables |
| Host | hands the call to the host; the VM suspends and resumes on the reply |
| Derived | performs `eq`, `cmp`, `show` or `default` structurally on the type, substituted by the frame |
| Dispatch | selects a row by the witness, a value's tag or a substituted meaning, with the function the compiler used |
| Value | eval/apply: fewer arguments than the closure's arity fill its prefix, equal enter it, more enter it and apply the rest to the result |

A dispatch whose receiver is concrete after the frame's witnesses is
resolved at compile time and emitted as the row's target: a body, a
native, a host call, a field read in place, or a derived operation. One
with a parameter or an unfixed session meaning stays a run-time dispatch,
and every definition row of the member is lowered so the machine finds
its body. Rows are matched by head meaning, then by the overload
argument's meaning; a field row wins. A packed value selects by its tag,
and a row written for the tag receives the payload; `eq` on two packed
values compares their tags first, unequal tags giving `False`, then the
payloads. A `Type::Session` is read from the session's meaning slots
when the line compiles.

## What the compiler decides

The IR's slot numbers are unit-wide; a body maps the ones it uses to
locals in first-use order. Supplies are lowered in source order into the
call's spot locals, then the recorded defaults in their order, each
reading the earlier locals; element supplies collect into one array for
the trailing spot. A call whose callee is known and whose every spot is
filled is a body, native, host or derived callee; a call with open spots
makes a thunk body whose captures are the supplied values and whose spots
are the open ones in recorded order, the defaults running when it is
called; a zero-spot call is its callee's value. A declaration, a
constructor, a member or a derived operation written alone is a thunk of
its arity, except that one of no spots is its value. A bare member on a
value, `p.x`, reads a field row in place or calls the member.

An arm list is a decision tree over the spots: a switch for a constructor,
a side, taken by label (`Just`, `Ok` and `Right` are good; `Either`
declares its bad `Left` first), a packed tag, or a list on a `Vec` by
its length; a test for a gate; the recorded equality for a literal, feeding
a switch on `Bool`; parts read once per branch into locals; binds and
guards per path; each arm's body lowered once. Coverage is proved, so no
switch has an `otherwise` it cannot take. A tuple, an array, a record of
one constructor and a lift's payload split into parts with no switch.

A marked pipe switches on the left's side: the inactive side keeps its
value, the marked side's payload goes through the stage, whose supply of
the left reads the payload instead, and the result is rebuilt as the
checker recorded, direct, wrapped, or discarded to the bad side. A lift
is the same rule spelled as a binding. `||` and `&&` switch by their
recorded mode, a skipped side being an edge not taken. A comparison
switches the `Ordering` to `True` on an accepted constructor, else
`False`. A pipe whose stage calls lowers as its stage, which already
lists the piped value; the checker gives a stage that is a function value
as a `Call` of it with the piped value as its one spot.

A literal read by the prelude, by name or by a dispatch a concrete meaning
resolves to it, is a prepared constant: an integer that fits is `Int`,
otherwise `Big`; a fraction is the nearest float by `core`'s correctly
rounded parse of its decimal spelling. A literal whose nearest float is
not finite is left to the reader, which Faults in the compile phase. Any
other reader is called with the written form as a `Form` record
constant; the compile phase runs it and a node it prepared, by
`Tables.prepared`, lowers as that constant. A value definition is stored once, by the
main or the module's init body, and so is each name a line's pattern
binds, a lift's included, stored where the pattern binds it; one no
line stores, such as an `impl`
block's value member, has a body a backend runs on the first read of
the unset global. A definition with spots is a body and a
reference to it is a closure with the node's meanings. Captures copy the
outer slots' values at creation. A module's init body runs the module's
lines; a backend runs each not yet initialized before the main.

## What the compiler reads from the unit

The compiler rediscovers nothing the checker decided: the rebuild of a
marked pipe or lift, the mode of `||` and `&&`, the cast row and its
selected operation, the conversion's mode and predicates, a literal
pattern's selected equality and receiver spot, the order of supplies and
defaults, the physical field of each constructor spot, the captures of a
lambda, the hidden parameters of a body, the dispatch rows, coverage, and
the entries of the plan. `Enter` statements carry `Eval` and `Init`
entries; a `Compile` entry never reaches a body, since the compile phase
of the checker owns it.

## Values

`value::Value` is the runtime's value, shared by every backend: sixteen
bytes, a tag and one eight-byte payload, immutable and shared by
reference. It is the unit, an `Int` that fits `i64`, a `Big` integer that
does not (never one that fits), an always finite `F32` or `F64`, text,
a sequence (a `Vec`, a `Deque` or an array; `Vec<U8>` is a sequence of
integers, with no second layout), a tuple, a record by meaning and
constructor index with its fields (every sum and product, `Bool` and
`Maybe` included), a closure, a meaning witness, or a packed value with
the meaning index of its tag. A closure holds its body, its captured
values, its meanings and the prefix of spots a run-time partial
application filled. The operations are `value`'s, by module: `arith` on
integers, `float` on both widths, `text`, and `structural` for equality,
order, display and defaults by a value's tag; each Faults by a
`value::Reason` the machine spans.

`:mir` displays a checked line's program in words on both REPL builds,
and `:mir <line>` a queried one, without internal byte offsets.

## Running

~~~text
vm::Machine::new(budget: Option<u64>) -> Machine
run(&mut self, program: &mir::Program) -> Outcome<Result<Value, Fault>>
reply(&mut self, program: &mir::Program, reply: Reply)
    -> Outcome<Result<Value, Fault>>
charged(&self) -> u64; budget(&self) -> Option<u64>
lines(&self) -> &[(usize, Value)]        # each main expression line's value
set_budget(&mut self, budget: Option<u64>)
withdraw(&mut self, withdrawn: &session::Withdrawn)
vm::Fault { span: token::Span, namespace: usize, reason: value::Reason }
vm::Observation { result: Result<Value, Fault>, trace: Vec<(Request,
                  Reply)>, success: Vec<u8>, error: Vec<u8> }
~~~

One machine serves a session: its globals are the accepted lines' values, and
its `program` joins every run's, so a stored function value keeps its body.
`run` runs each module's init body not yet run, then the main, with frames on
the heap, never on the Rust stack. A `Host` callee, or a dispatch row naming a
host's declaration, is the one place a run returns `Request::Call` with the
arguments in spot order and the `with` environment open at the call; `reply`
with the same program continues it, a value going where the call wanted it and
a failure Faulting `Host` at the call. A Fault ends the run where it happens,
its span in its `namespace`'s source; the host's prior effects stand.
`withdraw` forgets a withdrawn line's globals and module inits
([embedding](09-embed.md) §Session): given again, it reruns them, effects
included.

The budget is per run, `None` for an unbounded one; `charged` reports
the units of the run in progress or of the last one, bounded or not. A
block pays its `Charge` nodes when it opens: with `r` units left and
more nodes, the run Faults `Budget` at the span of node `r` before any
statement of the block runs. A call's node is paid in the block whose
terminator makes it, so a host call that cannot be paid is never
requested. A native pays its size term before it acts: an `Integer`
`add`, `sub`, `eq` or `cmp` one per 64-bit limb of the larger operand,
`mul`, `div` and `rem` the product of the limb counts; a container
operation one per element of its input, `take` and `head` only those
they reach; a structural `eq`, `cmp`, `show` or `default` one per value
below the receiver. A test costs the nodes of its limit's body, the
division test one unit; a proved entry has no `Enter` and costs nothing.
A host call's bytes are not charged.

An `Enter` tests every component its entry's path reaches from the
value, in order; the first failure is `Reason::Failed { entry, value }`,
which `Reason::describe(tables)` words with the entry's limit. `Observation` is what every
backend must reproduce from the same unit and replies on a run no budget
stops; where a budget stops a run, and the units charged, are the VM's
and not part of it. A driver assembles it from its trace and the bytes
its sink policy wrote ([embedding](09-embed.md) §Requests and replies).

## What follows

The compile phase of the checker ([checking](07-check.md) §The compile
phase) runs the same machine on `Compile` entries, program-reader
literals and demands, turning them into constants. The deque's
representation, a repeated length read from a witness, and a debugger's
step granularity are not fixed by this chapter.
