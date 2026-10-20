# Conformance cases: corners

Rows of [conformance](10-conformance.md) for corner cases in every stage,
each once a defect: a row stays so the defect cannot return.

## Corner cases (01 to 07)

| Case | Expected distinction |
| --- | --- |
| `x = ## d` LF `y = 2`, also impl and record values | unattached-doc error; docs never carry a value onto a new line |
| 100 000 documented name bindings | parse in linear time, under one second in release |
| `## d` CR LF | doc text `d`, no terminal CR |
| `## d` LF `use fs` | unattached-doc error at the doc |
| `### x` / `##########` | ordinary comments |
| raw quote across CR LF | both line-end bytes retained |
| decoded quote with backslash CR LF and indentation | join drops CR LF and leading spacing |
| host doc `- https://example.com/manual` / `- depth:` | opaque bullet / spot head |
| `x = [1, 2] \| sum` / `x = round 2` / `f = n => n + 1` | `Integer` / `F64` / generic `Add a, Literal a => a -> a` |
| `[x] = [1] \| map (n => n + 1)` then `y: F64 = x` | check error: the pattern binding fixed `x` to `Integer` |
| REPL accepted raw quote or doc with a blank line, then `:host` | replay preserves the complete entry and its docs |
| REPL doc above signature then `= value`, or above type then `where` | one documented binding or type item |
| REPL stray scalar inside an open raw quote | closing line stays with the failed quote; next binding survives |
| REPL `x = 1 ## note` then `y = 2` | first entry errors; `y` is accepted |
| REPL 1 MB raw quote over 25 000 lines | only new lines scanned while open; enters in seconds |
| REPL `g = f 1 \` LF `2`, `f = a b => a` | joined application; `g: Integer` |
| REPL `:t 'a` LF `  b'` | echoed literal retains LF and indentation |
| REPL `##! module` alone then `:ast` | accepted empty module; its doc is printed once |
| REPL lines up to a blank line, types with mutual claims | one compilation unit; EOF also submits |
| unannotated `From<Polar>.from = p => Self p.r p.t` | `p` is `Polar`; conversion is callable |
| a claim member with missing/extra receiver | check error; every member has the required receiver |
| overloaded `Mul` through `scale v k` or `(v \| .mul) k` | select after the argument meaning is known |
| `type Key: Eq & Show`, passing `1` at `Key` | accepted by meeting both parts |
| `Eq -> Eq -> Bool = a b => a == b`; `Vec<Show> \| map show` | packed equality; `Vec<String>` |
| `type N: Integer` with no limits or body | check error, no alias |
| a binding annotation continued after `->` or `&`, then `=` | one binding |
| REPL `8` LF `/ 2` | continued division |
| complete `:t` or `:r` followed by an operator line | query answers immediately, independent of read chunks |
| REPL `type Shape` LF `where self.area: F64` | one member-limit declaration |
| supplied `Show.show: (self: Self) -> Integer` | rejected while loading declarations |
| 3 000 prefix `!` operators | syntax nesting-limit error, no stack overflow |
| repeated literal lines at `F64`, including known places | member limits are not repeatedly expanded as numeric limits |
| long sums, invalid calls and concatenations | linear processing of settled obligations and literal groups |
| conversion with known unmatched source / unknown target | names the unsupported source / asks for the target meaning |
| bare signature then another binding; binding ending `=` LF value | signature errors alone, next binding survives; one binding |
| `impl Missing { … }` | unknown type once, no foreign-module error |
| comparison of two session types both named `A` | diagnostic qualifies distinct session origins |
| private field through a direct or deferred dot | private-field diagnostic naming the field |
| `pq.Hidden 1` or `pq.Hidden { x: 1 }` with `secret: U64` private and no default | check error naming `secret`; the derived constructor exists outside only when every private field defaults |
| `f = (n = f) => n`; `h = (n = 1) (m = h) => n + m` | check error at the default: it may not name its own binding |
| `n: I64 = 2.0`; `n: Integer = 1e3`; `n: I64 = 2.5`; `n: I64 = (2.5)` | exact `2` and `1000`, no cast; a fraction that is no integer is a check error, grouped or not, never a run-time cast |
| `to<U8> 300.0`; `to<U8> 2.5` | refuted as `U8 300` is; check error: not an integer |
| `type V8: Vec<U8> where self.len < 5`, field `v: V8` | `v` defaults to `[]`, proved by the static checks; with `self.len > 0` it stays a spot |
| `k = 5` LF `f = (m = k) => m` LF `f`; `type R { a: Integer = k }` LF `R`; a file's `limit = 5` in its `pub f = (n = limit) => n` | accepted: a default sees the session's earlier lines and its module's own bindings |
| `x = 1.0 / 0.0` LF `x + 1.0` LF `x = 1.0` LF `x + 1.0`; `type T { n: F64 = 1.0 / 0.0 }` LF `t = T` LF `t` | the Faulting line is withdrawn: unknown name, then `2.0`; unknown name `t`, while `T`, given on an earlier line, stays |
| `g = (n = 1) => n + 1.5` LF `g`; `f = (a = 1) (b = 2.5) => a + b` LF `f` | `F64`: a declared default's literal chooses with the body's, not alone |
| `big = s => s.area > 10.0`; `big 1` | missing `area` at the argument |
| plain member repeated in a later session impl | duplicate-member error |
| repeated literal/member requirement; duplicate claim block; missing host members | one error per cause; missing members named |
| loaded file with interleaved resolve and type errors | diagnostics in source order |
| missing claim on `Integer`; `1 ++ 2` | no promise of a future block; missing `concat` named |
| `:r x`, bare `:load`, top-level `Self` | readable resolution; path usage; uppercase `Self` diagnostic |
| opaque supplied type claiming `Eq`, `Ord`, `Show` or `Default` without members | declaration error; no derivation without a body |
| claimed `Vec<Integer>`, `Maybe<Integer>` or function member narrowed to `U8`; `Len.len = self => -1` | meaning checked; unimplemented function-limit proof stays open as narrowing |
| `kf: Key a => a -> Bool = p => p == p`, `Key: Eq & Show`, `kf 1` | written combination met by its parts |
| session `type S: Show { f: Integer -> Integer }`, then its `Show` block | type line accepted; claim pending until block |
| `g: Show -> String = s => s.show`; `h = n => g (n + 1)`; `h 2.5` | accepted; packing does not fix the outer parameter to `Integer` |
| `if True { 1 } else { 2 }`; `if p == Point { x: 1 } { y }` | `Integer`: the head's first top-level `{` is the body, so no record opens after the name; the record form is a parse error unless parenthesized |
| `g = a => { b => a + b }`; `g 1 2`; `compose inc inc 1` | the bound result, a function, takes the operands past the spot list; `inc 1 2` stays one argument too many |
| exported type with mixed `pub` claim members across overloads | operator, dot and conversion reach every claim member |
| cyclic bases, including bases with their own member limits | check error at the type declaration |
| `Eq<E, E>` or `Add<A, A, A>` | excess type-argument error |
| `Integer a => a -> a` / `Eq a => b -> b` | invalid member-limit prefix / ambiguous variable, at the binding |
| supplied `impl Nope: Show { … }` | unknown target at declaration load |
| `Default` through `String`, `ParseError` or another host type | no derived default |
| member-limit chain / many type and claim declarations / repeated literal bindings / user equality | no duplicate base expansion, indexed declarations, no substitution clone per single candidate |
| repeated uses of nested file modules; field typed by another imported module | resolve and type once per unit; field retains its imported type |
| inferred type deeper than the parser nesting limit | type-depth check error, no stack overflow |
| `k: Number a => a -> a = p => p + 1`; `f = k` | prints `Number a => a -> a`, no redundant `Literal` |

## Limit, phase and container corners

| Case | Expected distinction |
| --- | --- |
| `type Rc { n: U8 = cli.port }` then `a = Rc`, `g = k => Rc` | the field default's open entry is listed at each construction site, init once at a session binding, eval per call inside an arm, and not at the declaration; a refuted default is reported at the declaration |
| written constructor phase: `type C { n: Integer }` LF `impl C { n => Self { n: log n } }` LF `x: Compile & C = C 1` | timing error; written constructor bodies, guards and defaults must be pure |
| brackets at Sequence places: `xs: Vec<U8> = [1, 2, 300]` | refuted at 300; likewise Deque and nested elements |
| parameterized record fields: `type Pair<A, B> { first: A, second: B }` LF `p: Pair<U8, U8> = Pair 300 400` | both fields enter their limits |
| bound tuple parts: `t = (1, 300)` LF `ends: U8 * U8 = t` | refuted at the second part |
| union meaning: `id: U8 \| String = 300` | refuted; String cannot prove an Integer value |
| function limit narrowing: `f: U8 -> U8 = n => n` LF `apply = g x => g x` LF `y = apply f 300` LF `:checks y` | open narrowing; strict refuses the implicit entry |
| fault in a test: `type Q: Integer where 10 / self < 5` LF `z: Q = 0` | refuted test; no separate divisor entry |
| field default fault: `type R { n: Integer, d: Integer = (10 / n) }` LF `R 0`; `R 2`; `k => R k` | refuted; proved; one entry per construction, none at the declaration |
| cast value facts: `type Three: String where self.len == 3` LF `c: Three = [1, 2, 3]`; `d: Three = (1, 2)` | open entries; the list's length and the tuple's parts are not the text's |
| float ranges: `type A: F64 where self < 5` LF `type B: F64 where self <= 4` LF `a: A = 4.5` LF `b: B = a` LF `:checks b` | no integer interval proves the entry |
| written conversion facts: `type A { n: Integer }` LF `type B: From<A> { n: Integer }` LF `impl B: From<A> { from = a => B (a.n + 1000) }` LF `b = to<B> (A 5)` LF `x: U8 = b.n` LF `:checks x` | source field facts are dropped; x remains open |
| written sequence facts: `type Buf<T>: Sequence<T> { len: Integer }` LF `impl Buf<T>: Sequence<T> { sequence = slots => Self 1000 }` LF `b: Buf<Integer> = [1, 2, 3]` LF `x: U8 = b.len` LF `:checks x` | bracket length is not the result length; x remains open |
| Self entry: `type R { lo: Integer, hi: Integer } where self.lo <= self.hi` LF `impl R { bad = Self 5 0 }` | refuted construction |
| signed base cycle: `type S: !S` | cyclic-base check error, no stack overflow |
| length occurs check: `t: [Integer; n] -> [Integer; n + 1] = a => a ++ [0]` LF `u = xs => t (t xs)` LF `:t u` | length sums solve without recursion; an occurs check rejects a cyclic length |
| gate complement limits: `type Small: Integer where self < 10` LF `type Other: U8 & !Small` LF `kind = n => n \| { Small p => 1, Other q => 2 }` | not exhaustive; the complement has extra limits |
| nested gate coverage: `type Small: Integer where self < 10` LF `k = (Small p, y) => 1` | not exhaustive; every pattern position is read |
| `g = { [:e] => e, [!] => 0 }` with no family supplied | the family errors only; coverage is silent while a bracket's family is unknown |
| postfix phase: `type V { x: Integer }` LF `impl V { get = self => cli.port + self.x }` LF `y: Compile & Integer = (V 1).get` | timing error from the called member |
| claimed operation phase: `type V: Zero & Add { x: Integer }` LF `impl V: Add { add = o self => V (self.x + o.x + cli.port) }` LF `impl V: Zero { zero = V 0 }` LF `y: Compile & V = sum [V 1, V 2]` | timing follows the selected Add member |
| destructured phase: `(a, b) = (log 1, 5)` LF `z: Compile & Integer = b` | timing error; b belongs to the impure binding |
| array length is not exactness: `is_sorted = v => True` LF `type Sorted3: [Integer; 3] where Compile (is_sorted self)` LF `f: [Integer; 3] -> Sorted3 = xs => xs` | Compile test fails on unknown elements |
| empty interval: `type Empty: Integer where self > 10, self < 5` LF `f: Empty -> U8 = x => x` LF `:checks f` | empty interval proves the result entry |
| strict division: `q: Integer = 10 / cli.port` LF `:checks q` | nonconfigurable zero check remains planned under strict |
| holding order: `type Flag2: Show & Bool & Bits<1>` | accepted regardless of member-limit order |
| nested holding fit: `type Packed<T>: Bits<8> { v: T }` LF `f: Packed<U16> -> Integer = _ => 1` | unfit instantiation is a check error |
| binding gate entry: `U8 n = 300` | refuted before adding passed-limit facts |
| written constructor facts: `type P { n: Integer }` LF `impl P { n => Self (n + 1000) }` LF `p = P 1` LF `x: U8 = p.n` LF `:checks x` | argument facts do not describe the result; x stays open |
| pipe replaces default: `type P { x: Integer = 300 }` LF `p = 1 \| P` LF `x: U8 = p.x` | accepted; the piped value fills x |
| record pattern field type: `type P2 { a: Integer, b: String }` LF `f = (P2 x) => x` LF `:t f` | the payload binds at the first field type, Integer |
| limit purity: `type T: Integer where self > (ls \| len)` | impure limit is a check error |
| generic divisor: `dv = a b => a / b` LF `:checks dv` | one conditional zero check per call when Integer |
| claim result limit: `type L1: Len { n: Natural }` LF `impl L1: Len { len = self => self.n }` LF `:checks` | accepted with narrowing open, no meaning mismatch |
| operator bracket place: `xs: Vec<Integer> = [1]` LF `ys = xs ++ [4]` LF `:t ys` | bracket fills the known Vec place; result is Vec<Integer> |
| continued holding type: `type N: Integer & Bits<8>` LF `where 0 <= self, self < 256` LF `n: N = 255` | where continues the preceding type in the REPL |
| unit holding: `type P: Bits<8> { a: (), b: U8 }` | unit holds zero bits; type fits |
| timed holding: `x: Compile & F64 = 2.0` LF `:t x` | F64 retains Size<8> |
| field place facts: `type R { n: U8 }` LF `f: R -> U8 = r => r.n` LF `:checks f` | field read retains its declared limits |
| element location: `a: [U8; 3] = [1, 2, 300]` | refutation points at 300 |
| grouped printed type: `cf: Compile & (Integer -> Integer) = n => n` LF `:t cf` | printer retains function parentheses |
| timing input location: `:t Compile (sin 1.0)` | names sin as impure and points at sin |
| single unknown conversion error: `probe = input => input \| try` | one diagnostic asks for the target meaning |
| member arm results: `n = 1` LF `type P { x: Integer }` LF `impl P { f = { True => n, False => 2.5 } }` | incompatible member arm meanings are a mismatch |
| constructor arm result: `n = 1` LF `type P { x: Integer }` LF `impl P { f = True => n, False => 2.5 }` | bare second arm is a constructor and must return P |
| unknown sum payload: `type Choice<A> { Pick(value: A), Empty }` LF `f: Choice<Integer> -> Choice<U8> = x => x` LF `:checks f` | unknown payload limits remain open |
| single array length error: `z: [Integer; 2] = [1, 2, 3]` | one length diagnostic |
| single holding conflict: `hb: Bits<8> & Bits<16> = 1` | one holdings-disagree diagnostic |
| no-test gate diagnostic: `type Flag: Bool & Bits<1>` LF `Flag True` | no gate; suggest a typed place or to<Flag> |
| gate constructor conflict: `type Gate: Integer where self > 0` LF `type Sum { Gate }` LF LF | duplicate diagnostic points at the first declaration |
| failed imported binding; imported limit reading a host constant | failed binding carries no proof; host constant is read in its module |
| unannotated supplied constant; supplied `bad: U8 = 300` | infer the written constant; reject bad when read |
| repeated module uses; `:checks module.member`; global `:checks` | entries unique, source identities preserved, owning function named |
| exact String record field; limited F64 literal; unknown range | keep String and float spelling; omit unknown range; names list tested limits only |
| private named constructor field and positional inputs | one private-field diagnostic, no repeated positional report |
| `type Named: Shape where 0.0 < self.area`; `nn: Named = C 1.0`; `type PosS: Pos \| String`; `pp: PosS = 5` | the value packs as the base meaning and the test is entered; a literal takes its default meaning |
| `put out "abc"`, `x: Vec<U8> = "abc"`, `put out n`, `b"\xC3\xA9"`, strict or not | no per-element entry: text shown or taken as bytes is `U8` by construction |
| `dv8 = n => n / 8` | no divisor entry: a literal divisor is exact in every instance |

## REPL input and presentation

| Case | Expected distinction |
| --- | --- |
| Linux input is a directory or fails while reading | read error, never successful EOF or an empty module |
| host path contains NUL | rejected; never reads the prefix before NUL |
| command-line host path contains LF | passed as a path, never interpreted as REPL commands |
| `:t` followed by a tab and `1` | same query as a space-separated command |
| source contains a tab or terminal escape | control bytes escaped; caret follows the displayed text |
| `:ast` of public types, fields, members, or versioned imports | visibility, annotation and version are shown |
| `:r x` | source line and column, name and readable target; no byte offsets |
| REPL `y = 1` LF `  + 2` LF LF `:t y` | the blank line fires one entry: `Integer`, exact 3 |
| REPL `y = 1` LF `:t y` | the command fires the entry before it, then answers |
| REPL `y = 1` LF `  + "a"` LF LF `:t y` | the entry is rejected whole; `y` is unknown |
| REPL `w = (1` LF LF `)` LF LF | a blank line inside a bracket stays in the entry |
