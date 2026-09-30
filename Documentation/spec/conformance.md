# Conformance cases

Observables to test per stage. Lexical and syntax cases are covered by
public-API tests; checking and execution cases await their stages, and a
case citing [open decisions](open.md) is conditional on it. A row marked
*06.n* is specified and owed by that stage of the type model, not yet
implemented: 06.1 surface, 06.2 members, 06.3 limits, 06.4 timing and
holding, 06.5 containers.

## Lexing

Compare kinds, payloads, source slices and diagnostics.

| Source | Expected distinction |
| --- | --- |
| `x * 2` / `x*2` | name/operator/integer versus one word |
| `0x199 0o17 0b1010 1_000 -5` | magnitudes 409, 15, 10, 1000, negative 5 |
| `1st 2024-01-01 0xZZ 1__0` | words, not partial numbers or lexical errors |
| `18446744073709551616` | one exact integer: the lexer has no bound *06.1* |
| `1e999` / `1e` | float spelling versus word |
| `-n=-5 -l=False --la -- -` | attached flag tokens, word `--`, minus |
| `rd:4` | name, colon, integer; no resource literal exists |
| `Vec<Vec<U8>>` | two `>` symbols; no `>>` token |
| `cat x \` LF `y` | one line: names `cat`, `x`, `y`, no newline between |
| `cat x \ ` LF `y` (trailing space) | the same join |
| `a\b`, `\x` | words; `# note \` LF `y` | comment, newline, name |
| `/ ./x ../x /a` | slash symbol, three paths |
| `fs.Entry .name _ héllo` | qualified name, selector, wildcard name, word |
| `"\xC3\xA9" b"\xFF"` | UTF-8 string é, byte 255 |
| `"\xFF"` | invalid UTF-8 string error |
| `'a\nb'` / `"a\nb"` | raw backslash+n versus LF byte |
| `"#" # tail` | string hash, discarded comment |
| `a |` then LF then `b` | pipe continuation without that newline |
| `a ||` then two LFs then `b` | second newline retained |
| broken quote, LF, `x = 1` | one diagnostic; following line survives |
| `&` / `&&` / source NUL | symbol *06.1*, pipe, stray; NUL never truncates source |
| initial U+FEFF then `ls` | name `ls` at byte 3, scalar column 2 |
| other U+FEFF / any literal U+200B | stray, including in comments and quotes |
| `"\xEF\xBB\xBF"` / `"\xE2\x80\x8B"` | escaped marks are valid string data |

Also every keyword and non-keyword name, every operator beside its
neighbor, byte slices and scalar columns of `cat héllo "한글"`, CRLF, EOF
inside an escape, negative zero, radix boundaries and all five pipes.

## Parsing and checking

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
| `impl P { x (y = 0) => Self { x, y }, f = p => … }` / `impl P: T { f = … }` | constructor arm and member / syntax error *06.1* |
| `impl P { f = a => 1, b => 2 }` / `impl P { f = a => 1 b => 2 }` | member then constructor arm / error: bracket the arms |
| `use x.inc x.Point` / `use text 2` / `use fs ver 2` | names / version / error |
| `foo 1`, `foo` / `cat foo` | unknown name / text |
| `f 1` before `f = a => …` / `lookup` before `use sys.lookup` / `f = n => f n` / `impl P { a = … b …, b = … a … }` | unknown name, twice / self-recursion / members see each other |
| `pub x = 1` then `pub x = 2` in a file / `use a` inside `a` / a used module with a bare line | exports the final `x` / check error: import cycle / check error: no main |
| `type point {}`, `type S { circle(r: F64) }`, `Point = 1` | case rule at the declaration |
| `type Rd<Api>` / `Rd<fs>` | a host type's uppercase parameter / an API reference at the use site *06.1* |
| `pub len: T`, `impl P { n: I64 }` in a program | declaration form *06.2* |
| `sys.lookup`, `Polar.from`, `e.name`, `fs = e` then `fs.list` | module export / type member / members of a value, twice |
| `use nowhere`, `use geometry.Nope`, `fs.nope`, `Point.nope` | unknown module / not exported / unknown member, twice |
| `impl Vec { … }` in a program / `impl Natural { … }` | check error: not this module's meaning / a limited type has no `impl` *06.2* |
| `add = a b => a + b` | `Add a => a -> a -> a` *06.2* |
| `add2 = add 2` | one open spot |
| wrong record field | check error before any host call |
| `Maybe<Entry> | cat` versus `|: cat` | whole-value mismatch / payload bind |
| `find x | put out` / `find x || "" | put out` | outcome at a byte spot: check error / text written |
| `m = [:1]` / `m: Maybe<Integer> = [:1]` | family unknown: check error / `Just 1` *06.1* |
| `Point 3 4`, `Point 3`, `1 | Point 1` / `Point 3 4 5` | constructor, default, pipe fills `y` / extra argument |
| `Point { x: 3, y: 4, len: 5 }`, `Point 3 4 5`, `Point -len 5` outside its module | check error at the field or argument: `len` is private |
| a member without `pub` reached from another module | check error: the member is private |
| `pl | Point.from`, `pl | to<Point>`, `"3,4" | try<Point>` / `pl | Point` | conversions / `Point pl`: check error at an `I64` spot *06.2* |
| `from` and `try_from` on one target for one source / `Point.from x`, `x` of unknown meaning | check error: never both / check error naming the candidates *06.2* |
| `"42" | read<I64>` / `n: I64 = "42"` / `n: I64 = "x"` | `Result` / `42` (T5) / check error *06.1* |
| `read p | { [!e] => … }` with `read p : Maybe<T>` | check error: `Nothing` has no payload |
| `Either<(),Integer> || 0` | check error preserving outcome identity (T1) *06.1* |
| `older = d e => e.age > d` / `filter (older 7)` | `b.age: a, Ord a => a -> b -> Bool` / solved at `Entry` *06.2* |
| `s = .name` / `(f => 1) .name` | `a.name: b => a -> b` / check error at the dot |
| `find x |! log "gone"` / `find x |! find y` / `find x |! log` | `Maybe<Entry>`, the stage is a value / fallback / check error: open spot |
| `x = 1`, `inc = x => x + 1`, `even 4`, `[] == []` | `Integer` / `Add a, Literal a => a -> a` / `Bool` (T5) / ambiguous *06.2* |
| `z = if c { 2 } else { 2.5 }` / `w = if c { n } else { 2.5 }`, `n: Integer = 2` / `u = if c { 2 } else { "a" }` | `F64`, in either order / check error: `n` has a meaning / check error: no meaning reads both *06.2* |
| non-total arms | check error, never runtime match failure |
| `x: i64 = 1` / `g: I46 -> I46 = x => x` | hint: `I64` / unknown type *06.1* |
| `a ++ b` / `a++b` / `trait` / `grep where` | `a.concat b` / one word / a name / a command *06.1* |
| `[!T; 3]` in a type / `A |! B` in a type | array / error with the hint `| !` *06.1* |
| `type U8: Natural & Bits<8> where self <= 255`, `type I8: Integer & Bits<8> where …`, `where` on the next line | limited type, chained; signed; the line continues *06.1* |
| `type Shape where self.area: F64` / `type Key: Eq & Show` | member limits, no gate / named combination *06.2* |
| `double = n => n + n` / `same = p q => p == q` | `Add a => a -> a` / `Eq a => a -> a -> Bool` *06.2* |
| `(Vec2 1.0 2.0) + (Vec2 3.0 4.0)` / written `show` / `impl Maybe { map: … }` in the prelude | `Vec2`'s `add` / replaces the derived one / a supplied member *06.2* |
| `m: Mask = 0b0000_1111` / `k: Mask = 15` / `who: Name = "fipe"` | `Mask.literal` at compile time / check error: the literal Faults / `String`'s literal, then `Name`'s limits *06.2* |
| `total: Vec<Shape> -> F64` / `Add -> Add` / `| area` for a member | packed elements / packs and cannot add / unknown name *06.2* |
| `id: Integer | String = 3` then `id | { Integer n => n.show, String s => s }` | packed; the gate pattern tells which *06.2* |
| a gate and a constructor of one name / a test entering its own type / `!Shape` | check error, each *06.3* |
| `U8 2.0` / `U8 300` / `try<U8> 300` / `U8 x` with `x` open | check error: no conversion / refuted / `Err` / Fault when the test fails *06.3* |
| `n: U8 & Prime = 13` / `p: Prime = 7` / `type Handy: U8 | Prime` | both / proved at compile time / either *06.3* |
| `kind` over `Prime` and `NonPrime` / `classify` with `_` | covered by a gate and its `!` / gates tried in order *06.3* |
| `c: U8 = (a + b) / 2` / `d: U8 = a + b` with `a: U8 = 200`, `b: U8 = 100` | proved: 150 / refuted: 300 *06.3* |
| `h: U8 = half 200` / `clamp` / `m: U8 = max a b` / `next` over `Odd` | proved: by the body / by branches / at one variable / by the declared result *06.3* |
| `sum2: U8 -> U8 -> U8 = x y => x + y` / `:checks sum2` | open, tested per call / one plan entry *06.3* |
| an open entry in a strict session / a gate in one | check error / allowed *06.3* |
| `x / 0` / `x / y`, `y` known `0..5` | refuted / tested at run *06.3* |
| `Range { lo: 2, hi: 1 }` / `Range 1 2` | refuted: braces test the limit / proved *06.3* |
| `a + b` with `a: U8`, `b: U16` / `w: U16 = a` / `n: U8 = b` | `Integer`, known 1200 / proved / refuted *06.3* |
| `y = if c { 2 } else { 200 }` / `a: U8 = y` / `b: I8 = y` | `Integer`, known 2..200 / proved / open *06.3* |
| `f: F64 = 2` / `k: I64 = round 2.5` | `2.0` *06.2* / `2` *06.3* |
| `type Port … config.max_port` / `port: Port = cli.port` / `q: Seed = cli.seed` / `primes = Compile (sieve 1000)` | a host constant in a limit / tested at init / check error / prepared at compile time *06.4* |
| `Compile` alone, as a value or in a pattern | check error *06.4* |
| `Bits<8> & Bits<16>` / `type Odd8: Integer & Bits<8>` | check error, each: the holdings disagree / does not fit its holding *06.4* |
| `type Flag: Bool & Bits<1>` / `type Mode: Size<1> { … }` / `rgb: [U8; 3]` | one bit / 3 of 8 bits / `Size<3>` *06.4* |
| `pair: Integer * String = (1, "one")` / `(n, name) = pair` | tuple / both bound *06.5* |
| `swap = (a, b) => (b, a)` / `divmod` / `ends: U8 * U8 = (0, 255)` | `a * b -> b * a` / two results / each part a place: proved *06.5* |
| `f(a, b)` / `f (a, b)` / `f ((a, b))` / the wrong one / both readings check | two arguments / one tuple / one tuple, whatever the spacing / check error saying which was written / warning *06.5* |
| `type Pair<A, B> { … }`, `type Ring<T, n> { slots: [T; n], … }`, `type Sorted<T: Ord>: Vec<T> where …` in a program | type parameters, a number parameter, a member limit *06.5* |
| `shapes: Vec<Shape> = [Circle 1.0, Square 2.0]` | each element packed before unification, then `sequence` *06.5* |
| `xs = [1, 2, 3]` / `zs = xs ++ [4, 5]` / `v = [1, 2.5]` | `[Integer; 3]` / `[Integer; 5]`: `concat` adds the lengths / `[F64; 2]` *06.5* |
| `s: Sorted<Integer> = [1, 2, 3]` / `impl Ring<T, n> { sequence = … }` / a `sequence` that Faults on exact elements | `Vec`'s `sequence`, then the limit / a program's own `sequence` / check error *06.5* |
| `total xs` with `total` taking a `Vec` / `total (xs | to<Vec<Integer>>)` | check error: a bound array enters by a gate only / accepted *06.5* |
| `map` / `filter` on an array | keeps its length / a `Vec` *06.5* |
| `[T; n + m]` / `[T; m + n]` | one type: sums are equal in any order *06.5* |

The [corpus](examples.fipe) follows the parser in 06.1; check AST shape, inferred types and diagnostics, not only successful parses.

## Evaluation and hosts

Under a recording host that returns declared values and records every
operation, test values and ordered effects together:

- Named arguments evaluate once in source order (E1).
- A skipped `||`, `&&`, `|:` or `|!` branch has no host effects.
- `Just 1 |: (_ => Just 2)` is `Just 2`, not nested (E1).
- `Nothing || 4` returns 4; `Just 3 || 4` returns 3, not `Just 3`.
- Fault ends execution; prior effects remain; execution capabilities close.
- A unit with any check error performs zero program effects.
- The exact budget boundary faults before the first unaffordable action (H1).
- VM and native backends produce identical observations from the same typed
  unit and replayed host responses, including charge points (E2/H1).

Test monad identity and associativity for `Maybe` and `Result`. Each case
states its outcome independently; agreement between backends alone proves
nothing.
