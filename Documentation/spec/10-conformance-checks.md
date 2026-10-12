# Conformance cases: syntax, types and checks

The rows of [conformance](10-conformance.md) for chapters 02 to 07; its
opening says how a row is read and what a mark means.

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
| `ls -l /etc`, `ls -lr` / `ls -nope`, `ls -ll`, `ls -lr=True` | `Vec<Entry>` / unknown spot, duplicate supply, non-Bool chaining |
| `f x a b c` / `f x [a, b]`, with a trailing `Vec<String>` | three collected elements / one list, tried before one element |
| trailing `Vec` with no operands / a named Vec / a passed-on function | declared default or open / one list / no spot names or declared defaults |
| `dir (width = 80) (cols = (width / 8)) => cols` | earlier spots type later defaults; no default is evaluated |
| record Bool, Vec, Deque, Maybe, array and record fields / a limited field | table defaults / default only when entry is statically proved |
| duplicate spot names / Bool `long` and `links` / supplied member with non-final `self` | declaration check errors |
| host `(depth: U64 = True)` / program field default of the wrong type | error naming the spot at the stage needing it / error at its declaration |
| `big = _ => 9007199254740993` then `x: F64 = big ()` | literal form checked at the use; exactness error |
| recursive `f "a"` then `f True` | error at the conflicting use, with the literal as the other site |
| each call's supplied expressions, defaults and open spots | recorded once; supplies in source order, defaults and open spots in declaration order |
| wrong record field | check error before any host call |
| `Maybe<Entry> | cat` versus `|: cat` | whole-value mismatch / payload bind |
| `find x | put out` / `find x || "" | put out` | outcome guard: check error naming `||` and `|:` / text cast recorded |
| `m = [:1]` / `m: Maybe<Integer> = [:1]` | family unknown: check error / `Just 1` |
| `Point 3 4`, `Point 3`, `1 | Point 1` / `Point 3 4 5` | constructor, default, pipe fills `y` / extra argument |
| `Point { x: 3, y: 4, len: 5 }`, `Point 3 4 5`, `Point -len 5` outside its module | check error at the field or argument: `len` is private |
| a member without `pub` reached from another module | check error: the member is private |
| `pl | Point.from`, `pl | to<Point>`, `"3,4" | try<Point>` / `pl | Point` | conversions / `Point pl`: check error at an `I64` spot |
| `from` and `try_from` on one target for one source / `Point.from x`, `x` of unknown meaning | check error: never both / check error naming the candidates |
| `"42" | read<I64>` / `n: I64 = "42"` / `n: I64 = "x"` | `Result` / read cast recorded, read at compile time / check error: the literal's read Faults |
| `read p | { [!e] => … }` with `read p : Maybe<T>` | check error: `Nothing` has no payload |
| `Either<(),Integer> || 0` | check error preserving outcome identity (T1) |
| `older = d e => e.age > d` / `filter (older 7)` | `b.age: a, Ord a => a -> b -> Bool` / solved at `Entry` |
| `s = .name` / `(f => 1) .name` | `a.name: b => a -> b` / check error at the dot |
| `find x |! log "gone"` / `find x |! find y` / `find x |! log` | `Maybe<Entry>`, the stage is a value / fallback / check error: open spot |
| `x = 1`, `inc = x => x + 1`, `even 4`, `[] == []` | `Integer` / `Add a, Literal a => a -> a` / `Bool` (T5) / ambiguous |
| `z = if c { 2 } else { 2.5 }` / `w = if c { n } else { 2.5 }`, `n: Integer = 2` / `u = if c { 2 } else { "a" }` | `F64`, in either order / check error: `n` has a meaning / check error: no meaning reads both |
| non-total arms | check error naming an uncovered pattern, never runtime match failure |
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
| `k: Mask = 15` | check error: the literal Faults |
| `who: Name = "fipe"` | `String`'s literal, then `Name`'s limits |
| `total: Vec<Shape> -> F64` / `Add -> Add` / `| area` for a member | packed elements / packs and cannot add / unknown name |
| `id: Integer | String = 3` then `id | { Integer n => n.show, String s => s }` | packed; the gate pattern tells which |
| a gate and a constructor of one name / a test entering its own type / `!Shape` | check error, each |
| `U8 2.0` / `U8 300` / `try<U8> 300` / `U8 x` with `x` open | check error: no conversion / refuted / `Err` / Fault when the test fails |
| `n: U8 & Prime = 13` / `type Handy: U8 | Prime` | U8 proved, Prime proved at compile time / either |
| `p: Prime = 7` / `Seed` with `Compile (is_prime self)` | named test settled at compile time |
| `kind` over `Prime` and `NonPrime` / `classify` with `_` | covered by a gate and its `!` / gates tried in order |
| `c: U8 = (a + b) / 2` / `d: U8 = a + b` with `a: U8 = 200`, `b: U8 = 100` | proved: 150 / refuted: 300 |
| `h: U8 = half 200` / `clamp` / `m: U8 = max a b` / `next` over `Odd` | proved: by the body / by branches / at one variable / by the declared result *narrowing* |
| `sum2: U8 -> U8 -> U8 = x y => x + y` / `:checks sum2` | open, tested per call / one plan entry |
| an open entry in a strict session / a gate in one | check error / allowed |
| `x / 0` / `x / y`, `y` known `0..5` | refuted / tested at run |
| `Range { lo: 2, hi: 1 }` / `Range 1 2` | refuted: braces test the limit / proved |
| `a + b` with `a: U8`, `b: U16` / `w: U16 = a` / `n: U8 = b` | `Integer`, known 1200 / proved / refuted |
| `y = if c { 2 } else { 200 }` / `a: U8 = y` / `b: I8 = y` | `Integer`, known 2..200 / proved / open *narrowing* |
| `f: F64 = 2` | `2.0` by the literal member |
| `k: I64 = round 2.5` | numeric cast recorded, then open `I64` entry; evaluates to `2` |
| `type Port … config.max_port` / `port: Port = cli.port` / `q: Seed = cli.seed` / `primes = Compile (sieve 1000)` | a host constant in a limit / tested at init / check error / compile demand met, prepared and exact for later lines |
| `Compile` alone, as a value or in a pattern | check error |
| `x: Init & Integer = cli.port` / a parameter with `Init & Integer` | known at init / check error |
| `Init (cli.port)` in a module binding / inside an arm | demand met / check error |
| `Init (test)` open at a module binding / inside an arm | tested at init / must be proved |
| `rgb: [U8; 3] = [255, 128, 0]` | array literal enters its annotated place |
| `Bits<8> & Bits<16>` / `type Odd8: Integer & Bits<8>` | check error, each: the holdings disagree / does not fit its holding |
| `type Flag: Bool & Bits<1>` / `type Mode: Size<1> { … }` / `rgb: [U8; 3]` supplied by the host | one bit / 3 of 8 bits / `Size<3>` |
| `pair: Integer * String = (1, "one")` / `(n, name) = pair` | tuple / both bound |
| `swap = (a, b) => (b, a)` / `divmod` / `ends: U8 * U8 = (0, 255)` | `a * b -> b * a` / two results / each part a place: proved |
| `f(a, b)` / `f (a, b)` / `f ((a, b))` / the wrong one / both readings check | two arguments / one tuple / one tuple, whatever the spacing / check error saying which was written / warning |
| `type Pair<A, B> { … }`, `type Ring<T, n> { slots: [T; n], … }`, `type Sorted<T: Ord>: Vec<T> where …` in a program | type parameters, a number parameter, a member limit |
| `shapes: Vec<Shape> = [Circle 1.0, Square 2.0]` | each element packed before unification, then `sequence` |
| `xs = [1, 2, 3]` / `zs = xs ++ [4, 5]` / `v = [1, 2.5]` | `[Integer; 3]` / `[Integer; 5]`: `concat` adds the lengths / `[F64; 2]` |
| `impl Ring<T, n>: Sequence<T> { sequence = … }` | a program's claimed `sequence` |
| `s: Sorted<Integer> = [1, 2, 3]` / a `sequence` that Faults on exact elements | proved at compile time / check error |
| `total xs` with `total` taking a `Vec` / `total (xs | to<Vec<Integer>>)` | check error: a bound array enters by `to<Vec<T>>` / accepted |
| `map` / `filter` on an array | keeps its length / a `Vec` |
| plain `impl V { add = … }` / `impl V: Add { add = … }` with `Add` listed | dot works, `+` rejected / `+` accepted |
| plain `area` / a `Shape` claim at `Vec<Shape>` | rejected / packed |
| `Mul<F64>` and `Mul<V>` on one meaning | argument meaning selects for operator, instance dot, type dot and selector; unknown names the candidates |
| a listed claim before its session block / after it | use rejected / accepted; earlier lines stay frozen |
| supplied claim block with an unknown limit, extra or missing member, duplicate block, or plain-member conflict | declaration error at load |
| `type V: Zero { x: F64 }` with no `zero` / with `impl V: Zero { zero = Self 0.0 }` / `impl V: Show { … }` unlisted, or a member `Zero` does not ask | check error at the claim / accepted / check error at the block |
| `type V: Default { x: F64 = 0.0 }` / `type V: a.Zero & b.Zero` whose `zero` differ in type | met by the fields, no block / check error: one member per name |
| `[] | sum` at `Vec2` claiming `Zero` / a field `n: Integer` with no value / `impl Vec2: Default { default = Self.zero }` | checks by `Add & Zero`; its value is `Vec2`'s zero / a spot: a zero is not a default / the default is the zero |
| `[T; n + m]` / `[T; m + n]` | one type: sums are equal in any order |

The [corpus](02-syntax.fipe) follows the grammar; check AST shape, inferred
types and diagnostics, not only successful parses.

| `[x, y] = e` with `e: Vec<Integer>` / `Just x = e` in main | non-total binding check error / good-side lift of the remaining main |
