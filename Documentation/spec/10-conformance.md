# Conformance cases

Observables to test per stage. Lexical and syntax cases are covered by public-API tests; checking and execution cases await their stages, and a case citing [open decisions](11-open.md) is conditional on it. A row marked *06.n* is specified and owed by that stage of the type model, not yet implemented: 06.5 limits, 06.6 timing and holding, 06.7 containers, 06.8 narrowing (what a body, a branch or a variable proves); *07* and *08* mark a row owed by that major.
## Lexical (01)

Compare kinds, payloads, source slices and diagnostics.

| Source | Expected distinction |
| --- | --- |
| `x * 2` / `x*2` | name/operator/integer versus one word |
| `0x199 0o17 0b1010 1_000 -5` | magnitudes 409, 15, 10, 1000, negative 5 |
| `2024-01-01 0xZZ 1__0` / `1st` | words, not partial numbers or lexical errors / `1` with the suffix `st` |
| `18446744073709551616` | one exact integer: the lexer has no bound |
| `1e999` / `1e` | float spelling versus `1` with the suffix `e` |
| `10ms` / `2i` / `0r7Z` / `x10` | suffix `ms` / suffix `i` / prefix `r` and the digits `7Z` as written / a name |
| `0x1Fe` / `0ms` / `1e5` | all hex digits / the prefix `m` / an exponent |
| `re"a+b"i` / `f"abc"` / `f "abc"` | prefix and suffix / a prefix / a name and a string |
| `-n=-5 -l=False --la -- -` | attached flag tokens, word `--`, minus |
| `rd:4` | name, colon, integer; no resource literal exists |
| `Vec<Vec<U8>>` | two `>` symbols; no `>>` token |
| `cat x \` LF `y` | one line: names `cat`, `x`, `y`, no newline between |
| `cat x \ ` LF `y` (trailing space) | the same join |
| `a\b`, `\x` | words; `# note \` LF `y` | comment, newline, name |
| `/ ./x ../x /a` | slash symbol, three paths |
| `fs.Entry .name _ héllo` | qualified name, selector, wildcard name, word |
| `"\xC3\xA9" b"\xFF"` / `"\xFF"i` | UTF-8 string é, byte 255 with the prefix `b` / byte 255 with the suffix `i` |
| `"\xFF"` | invalid UTF-8 string error |
| `'a\nb'` / `"a\nb"` | raw backslash+n versus LF byte |
| `"#" # tail` | string hash, discarded comment |
| `a |` then LF then `b` | pipe continuation without that newline |
| `a ||` then two LFs then `b` | second newline retained |
| broken quote, LF, `x = 1` | one diagnostic; following line survives |
| `&` / `&&` / source NUL | symbol, pipe, stray; NUL never truncates source |
| initial U+FEFF then `ls` | name `ls` at byte 3, scalar column 2 |
| other U+FEFF / any literal U+200B | stray, including in comments, docs and quotes |
| `"\xEF\xBB\xBF"` / `"\xE2\x80\x8B"` | escaped marks are valid string data |

Also every keyword and non-keyword name, every operator beside its neighbor, byte slices and scalar columns of `cat héllo "한글"`, CRLF, EOF inside an escape, negative zero, radix boundaries and all five pipes.
## Docs and quotes (01, 02)

| Case | Expected distinction |
| --- | --- |
| `##x` / `## x` / `##  x` / `##! module` | doc `x` / `x` / ` x` / inner doc |
| `a | ## doc` LF `b` | pipe joins; doc token retained; no intervening newline |
| `# ordinary` / `## doc` | comment span outside tokens / doc token; trivia kept in tree |
| `## a` LF `## b` LF LF `x = 1` | binding doc `a\nb`, blank gap allowed |
| `## a` LF LF `## b` LF `x = 1` / `##! module` then `## item` | paragraph `a\n\nb` / module and item docs |
| public prelude and host declarations | each has a doc |
| docs on module, type, binding, field, constructor and impl member | attached to each owner; body binding too |
| `## orphan` at EOF / before a value or `use` / before `##!` | unattached-doc syntax error |
| `x = 1` LF `##! late` / inner doc inside a body | inner-doc-position syntax error |
| host doc `- depth: …` / `- missing: …` | existing spot accepted / declaration error |
| `:t ls` / `:ast` on documented code | doc above type / doc under its owner |
| raw quote across LF with indentation, `#` and `"` | exact bytes, one token and span; following positions physical |
| decoded quote with backslash LF and leading spacing | join drops all three; explicit `\n` retained |
| decoded quote with bare LF / raw quote at EOF | unterminated quote; decoded recovery keeps following line |
| affixed multiline quote / pipe after its closing quote | affix kept / same logical line |
| REPL raw quote opened on one input line, closed on the next | continuation, one string |
## Review regressions (06.4)
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
| REPL `:{`, types with mutual claims, `:}` | one compilation unit; commands inside are text; EOF also submits |
| unannotated `From<Polar>.from = p => Self p.r p.t` | `p` is `Polar`; conversion is callable |
| a claim member with missing/extra receiver, or a narrower parameter | check error; every member is held to the required full type |
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
| `big = s => s.area > 10.0`; `big 1` | missing `area` at the argument |
| plain member repeated in a later session impl | duplicate-member error |
| repeated literal/member requirement; duplicate claim block; missing host members | one error per cause; missing members named |
| loaded file with interleaved resolve and type errors | diagnostics in source order |
| missing claim on `Integer`; `1 ++ 2` | no promise of a future block; missing `concat` named |
| `:r x`, bare `:load`, top-level `Self` | readable resolution; path usage; uppercase `Self` diagnostic |
| opaque supplied type claiming `Eq`, `Ord`, `Show` or `Default` without members | declaration error; no derivation without a body |
| claimed `Vec<Integer>`, `Maybe<Integer>` or function member narrowed to `U8`; `Len.len = self => -1` | structural member-type error; result limits checked too |
| `kf: Key a => a -> Bool = p => p == p`, `Key: Eq & Show`, `kf 1` | written combination met by its parts |
| session `type S: Show { f: Integer -> Integer }`, then its `Show` block | type line accepted; claim pending until block |
| `g: Show -> String = s => s.show`; `h = n => g (n + 1)`; `h 2.5` | accepted; packing does not fix the outer parameter to `Integer` |
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
## Syntax, types and checks (02 to 07)
| Case | Expected distinction |
| --- | --- |
| `add 2 - 1` / `add (2 - 1)` | rejected argument operator / grouped operand |
| `cat x > y` / `x > y` | rejected argument operator / comparison, at every position |
| `find x` LF `|: cat` LF `|! log` | one pipeline: leading operators continue |
| `f: A -> B` LF `  = x => x` / `x = 1` LF `  = 2` | one binding: leading `=` continues / error as `x = 1 = 2` |
| `a` LF `- b` / `a` LF `-b` / `a` LF `-5` | `a - b` / two lines, flag / two lines, literal |
| `a` LF LF `|: b`; `a` LF `# c` LF `|: b` | blank line ends continuation: error at `|:` |
| `ls /` LF `next` / `8` LF `/ 2` | root path, then a line / division |
| `f(x,` LF `y)` / `f(x` LF `, y)` | list continues / error at the first newline |
| `(a, b,)` / `(a,)` / `A &` LF `B` in a type | tuple / error: no tuple of one / one type |
| `(a` LF `b)` / `f (` LF `g` LF `)` / `f` LF `(g)` | two-line group / one argument / two lines |
| `map { x }` / `Point { x, y: 2 }` / `Self { x }` | block argument / record with a punned field / record |
| `if p == Point { x: 1 } { y }` | error: first `{` is the body |
| `{ [:e] => e.name` LF `[!] => "-" }` | error: expected `,` before the line break |
| `[:e]`, `[:]`, `[!e]`, `[!]` / `[!x]` / `[(!x)]` | outcome brackets / outcome / one-element list |
| `x | Rd<fs>`, `read<I64>`, `to<Point>` / `a < b` / `a<b` | instances / comparison / error: expected `>` |
| `x = 1`, `[x, y] = e`, `Just x = e`, `x: I64 = e` / `cmd -c=80` / `x => y = 1` | bindings / a call / an arm |
| `pub f = a (b = 0) => …` / `pub ls` / `{ pub x = 1 }` | exported binding / error / error |
| `a b => …`, `Just e rest => …` / `Just Just e => …` | two-pattern heads / one nested constructor |
| `a` LF `=> b` | one arm: a leading `=>` continues |
| `A -> B -> C` / `() -> T` / `(A -> B) -> C` | right-nested / unit parameter / grouped |
| `f(a,b)` / `f (x)` / `f()` | explicit arguments / grouped argument / empty call |
| `(e).name` / `f .name` | postfix field / selector argument |
| `type P { pub x: I64, y: I64 = 0 }` / `type S { C(r: F64), D }` | fields / constructors |
| `impl P { x (y = 0) => Self { x, y }, f = p => … }` | constructor arm and member |
| `impl P { f = a => 1, b => 2 }` / `impl P { f = a => 1 b => 2 }` | member then constructor arm / error: bracket the arms |
| `use x.inc x.Point` / `use text 2` / `use fs ver 2` | names / version / error |
| `foo 1`, `foo` / `cat foo` | unknown name / text |
| `f 1` before `f = a => …` / `lookup` before `use sys.lookup` / `f = n => f n` / `impl P { a = … Self.b …, b = … Self.a … }` | unknown name, twice / self-recursion / members see each other through `Self` |
| `pub x = 1` then `pub x = 2` in a file / `use a` inside `a` / a used module with a bare line | exports the final `x` / check error: import cycle / check error: no main |
| `pub id = x => x` / `pub add = a b => a + b` / `impl P { pub f = a => a }` | `a -> a` / `Add<b> a => a -> b -> a`, instantiated per use / `a -> a` |
| `type point {}`, `type S { circle(r: F64) }`, `Point = 1` | case rule at the declaration |
| `type Rd<Api>` / `Rd<fs>` | a host type's uppercase parameter / an API reference at the use site |
| `pub len: T`, `impl P { n: I64 }` in a program | declaration form |
| `sys.lookup`, `Polar.from`, `e.name`, `fs = e` then `fs.list` | module export / type member / members of a value, twice |
| `use nowhere`, `use geometry.Nope`, `fs.nope`, `Point.nope` | unknown module / not exported / unknown member, twice |
| `impl Vec { … }` in a program / `impl Natural { … }` | check error: not this module's meaning / a limited type has no `impl` |
| `add = a b => a + b` / `add 2` queried | `Add<b> a => a -> b -> a` / `Add<a> b, Literal b => a -> b` |
| `add2 = add 2` | one open spot |
| wrong record field | check error before any host call |
| `Maybe<Entry> | cat` versus `|: cat` | whole-value mismatch / payload bind |
| `find x | put out` / `find x || "" | put out` | outcome at a byte spot: check error / text written *07* |
| `m = [:1]` / `m: Maybe<Integer> = [:1]` | family unknown: check error / `Just 1` |
| `Point 3 4`, `Point 3`, `1 | Point 1` / `Point 3 4 5` | constructor, default, pipe fills `y` / extra argument |
| `Point { x: 3, y: 4, len: 5 }`, `Point 3 4 5`, `Point -len 5` outside its module | check error at the field or argument: `len` is private |
| a member without `pub` reached from another module | check error: the member is private |
| `pl | Point.from`, `pl | to<Point>`, `"3,4" | try<Point>` / `pl | Point` | conversions / `Point pl`: check error at an `I64` spot |
| `from` and `try_from` on one target for one source / `Point.from x`, `x` of unknown meaning | check error: never both / check error naming the candidates |
| `"42" | read<I64>` / `n: I64 = "42"` / `n: I64 = "x"` | `Result` / `42` (T5) / check error |
| `read p | { [!e] => … }` with `read p : Maybe<T>` | check error: `Nothing` has no payload |
| `Either<(),Integer> || 0` | check error preserving outcome identity (T1) |
| `older = d e => e.age > d` / `filter (older 7)` | `b.age: a, Ord a => a -> b -> Bool` / solved at `Entry` |
| `s = .name` / `(f => 1) .name` | `a.name: b => a -> b` / check error at the dot |
| `find x |! log "gone"` / `find x |! find y` / `find x |! log` | `Maybe<Entry>`, the stage is a value / fallback / check error: open spot |
| `x = 1`, `inc = x => x + 1`, `even 4`, `[] == []` | `Integer` / `Add a, Literal a => a -> a` / `Bool` (T5) / ambiguous |
| `z = if c { 2 } else { 2.5 }` / `w = if c { n } else { 2.5 }`, `n: Integer = 2` / `u = if c { 2 } else { "a" }` | `F64`, in either order / check error: `n` has a meaning / check error: no meaning reads both |
| non-total arms | check error, never runtime match failure *07* |
| `x: i64 = 1` / `g: I46 -> I46 = x => x` | check error with the hint `I64` / unknown type |
| `a++b` / `trait` / `grep where` | one word / a name / a command |
| `a ++ b` | `a.concat b` |
| `[!T; 3]` in a type / `A |! B` in a type | array / error with the hint `| !` |
| `!A & B | C * D -> E` / `f (a, b)` / `f(a, b)` | `((!A & B) | C) * D -> E` / one tuple argument / two arguments |
| `type U8: Natural & Bits<8> where self <= 255`, `type I8: Integer & Bits<8> where …`, `where` on the next line | limited type, chained; signed; the line continues |
| `type Shape where self.area: F64` / `type Key: Eq & Show` | member limits, no gate / named combination |
| `double = n => n + n` / `same = p q => p == q` | `Add a => a -> a` / `Eq a => a -> a -> Bool` |
| `type Add<R = Self, O = Self> where …` / `scale = v k => v * k` / `v * 2.0`, `v: Vec2` | parameter defaults / `Mul<b> a => a -> b -> a` / check error: `Vec2` is no `Literal` |
| `diff = p q => p - q`, `p => p - p` / `diff: Point -> Point -> Vector = p q => p - q` | `Sub<b> a => a -> b -> a`, `Sub a => a -> a` / `Point`'s `Sub<Point, Vector>` |
| `(Vec2 1.0 2.0) + (Vec2 3.0 4.0)` / written `show` / `impl Maybe { map: … }` in the prelude | `Vec2`'s `Add` claim / a `Show` block replaces the derived member / a supplied member |
| `m: Mask = 0b0000_1111` | typed through `Mask.literal` |
| `k: Mask = 15` | check error: the literal Faults *08* |
| `who: Name = "fipe"` | `String`'s literal, then `Name`'s limits |
| `total: Vec<Shape> -> F64` / `Add -> Add` / `| area` for a member | packed elements / packs and cannot add / unknown name |
| `id: Integer | String = 3` then `id | { Integer n => n.show, String s => s }` | packed; the gate pattern tells which |
| a gate and a constructor of one name / a test entering its own type / `!Shape` | check error, each |
| `U8 2.0` / `U8 300` / `try<U8> 300` / `U8 x` with `x` open | check error: no conversion / refuted / `Err` / Fault when the test fails |
| `n: U8 & Prime = 13` / `type Handy: U8 | Prime` | U8 proved, Prime open (compile-time evaluation) / either |
| `p: Prime = 7` / `Seed` with `Compile (is_prime self)` | named test settled at compile time *08* |
| `kind` over `Prime` and `NonPrime` / `classify` with `_` | covered by a gate and its `!` / gates tried in order |
| `c: U8 = (a + b) / 2` / `d: U8 = a + b` with `a: U8 = 200`, `b: U8 = 100` | proved: 150 / refuted: 300 |
| `h: U8 = half 200` / `clamp` / `m: U8 = max a b` / `next` over `Odd` | proved: by the body / by branches / at one variable / by the declared result *06.8* |
| `sum2: U8 -> U8 -> U8 = x y => x + y` / `:checks sum2` | open, tested per call / one plan entry |
| an open entry in a strict session / a gate in one | check error / allowed |
| `x / 0` / `x / y`, `y` known `0..5` | refuted / tested at run |
| `Range { lo: 2, hi: 1 }` / `Range 1 2` | refuted: braces test the limit / proved |
| `a + b` with `a: U8`, `b: U16` / `w: U16 = a` / `n: U8 = b` | `Integer`, known 1200 / proved / refuted |
| `y = if c { 2 } else { 200 }` / `a: U8 = y` / `b: I8 = y` | `Integer`, known 2..200 / proved / open *06.8* |
| `f: F64 = 2` | `2.0` by the literal member |
| `k: I64 = round 2.5` | `2` by the numeric cast *07* |
| `type Port … config.max_port` / `port: Port = cli.port` / `q: Seed = cli.seed` / `primes = Compile (sieve 1000)` | a host constant in a limit / tested at init / check error / compile demand met; preparation awaits *08* |
| `Compile` alone, as a value or in a pattern | check error |
| `x: Init & Integer = cli.port` / a parameter with `Init & Integer` | known at init / check error |
| `Init (cli.port)` in a module binding / inside an arm | demand met / check error |
| `Init (test)` open at a module binding / inside an arm | tested at init / must be proved |
| `rgb: [U8; 3] = [255, 128, 0]` | array literal enters its annotated place *06.7* |
| `Bits<8> & Bits<16>` / `type Odd8: Integer & Bits<8>` | check error, each: the holdings disagree / does not fit its holding |
| `type Flag: Bool & Bits<1>` / `type Mode: Size<1> { … }` / `rgb: [U8; 3]` supplied by the host | one bit / 3 of 8 bits / `Size<3>` |
| `pair: Integer * String = (1, "one")` / `(n, name) = pair` | tuple / both bound *06.7* |
| `swap = (a, b) => (b, a)` / `divmod` / `ends: U8 * U8 = (0, 255)` | `a * b -> b * a` / two results / each part a place: proved *06.7* |
| `f(a, b)` / `f (a, b)` / `f ((a, b))` / the wrong one / both readings check | two arguments / one tuple / one tuple, whatever the spacing / check error saying which was written / warning *06.7* |
| `type Pair<A, B> { … }`, `type Ring<T, n> { slots: [T; n], … }`, `type Sorted<T: Ord>: Vec<T> where …` in a program | type parameters, a number parameter, a member limit *06.7* |
| `shapes: Vec<Shape> = [Circle 1.0, Square 2.0]` | each element packed before unification, then `sequence` *06.7* |
| `xs = [1, 2, 3]` / `zs = xs ++ [4, 5]` / `v = [1, 2.5]` | `[Integer; 3]` / `[Integer; 5]`: `concat` adds the lengths / `[F64; 2]` *06.7* |
| `s: Sorted<Integer> = [1, 2, 3]` / `impl Ring<T, n> { sequence = … }` / a `sequence` that Faults on exact elements | `Vec`'s `sequence`, then the limit / a program's own `sequence` / check error *06.7* |
| `total xs` with `total` taking a `Vec` / `total (xs | to<Vec<Integer>>)` | check error: a bound array enters by a gate only / accepted *06.7* |
| `map` / `filter` on an array | keeps its length / a `Vec` *06.7* |
| plain `impl V { add = … }` / `impl V: Add { add = … }` with `Add` listed | dot works, `+` rejected / `+` accepted |
| plain `area` / a `Shape` claim at `Vec<Shape>` | rejected / packed |
| `Mul<F64>` and `Mul<V>` on one meaning | argument meaning selects for operator, instance dot, type dot and selector; unknown names the candidates |
| a listed claim before its session block / after it | use rejected / accepted; earlier lines stay frozen |
| supplied claim block with an unknown limit, extra or missing member, duplicate block, or plain-member conflict | declaration error at load |
| `type V: Zero { x: F64 }` with no `zero` / with `impl V: Zero { zero = Self 0.0 }` / `impl V: Show { … }` unlisted, or a member `Zero` does not ask | check error at the claim / accepted / check error at the block |
| `type V: Default { x: F64 = 0.0 }` / `type V: a.Zero & b.Zero` whose `zero` differ in type | met by the fields, no block / check error: one member per name |
| `[] | sum` at `Vec2` claiming `Zero` / a field `n: Integer` with no value / `impl Vec2: Default { default = Self.zero }` | `Vec2.zero` / a spot: a zero is not a default / the default is the zero *06.7* |
| `[T; n + m]` / `[T; m + n]` | one type: sums are equal in any order *06.7* |

The [corpus](02-syntax.fipe) follows the grammar; check AST shape, inferred types and diagnostics, not only successful parses.

## Evaluation and hosts (08, 09)

Under a recording host that returns declared values and records every operation, test values and ordered effects together:

- Named arguments evaluate once in source order (E1). A skipped `||`, `&&`, `|:` or `|!` branch has no host effects.
- `Just 1 |: (_ => Just 2)` is `Just 2`, not nested (E1). `Nothing || 4` returns 4; `Just 3 || 4` returns 3, not `Just 3`.
- Fault ends execution; prior effects remain; execution capabilities close. A unit with any check error performs zero program effects.
- The exact budget boundary faults before the first unaffordable action (H1). VM and native backends produce identical observations from the same typed unit and replayed host responses, including charge points (E2/H1).

Test monad identity and associativity for `Maybe` and `Result`. Each case states its outcome independently; agreement between backends alone proves nothing.
