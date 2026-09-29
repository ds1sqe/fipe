# Conformance cases

Observables to test per stage. Lexical and syntax cases are covered by
public-API tests; checking and execution cases await their stages, and a
case citing [open decisions](open.md) is conditional on it.

## Lexing

Compare kinds, payloads, source slices and diagnostics.

| Source | Expected distinction |
| --- | --- |
| `x * 2` / `x*2` | name/operator/integer versus one word |
| `0x199 0o17 0b1010 1_000 -5` | magnitudes 409, 15, 10, 1000, negative 5 |
| `1st 2024-01-01 0xZZ 1__0` | words, not partial numbers or lexical errors |
| `18446744073709551616` | integer range error, not truncation |
| `1e999` / `1e` | float spelling versus word |
| `-n=-5 -l=False --la -- -` | attached flag tokens, word `--`, minus |
| `rd:4` | name, colon, integer; no resource literal exists |
| `Vec<Vec<u8>>` | two `>` symbols; no `>>` token |
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
| `&` / `&&` / source NUL | stray, pipe, stray; NUL never truncates source |
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
| `a` LF `- b` / `a` LF `-b` / `a` LF `-5` | `a - b` / two lines, flag / two lines, literal |
| `a` LF LF `|: b`; `a` LF `# c` LF `|: b` | blank line ends continuation: error at `|:` |
| `ls /` LF `next` / `8` LF `/ 2` | root path, then a line / division |
| `f(x,` LF `y)` / `f(x` LF `, y)` | list continues / error at the first newline |
| `(a` LF `b)` / `f (` LF `g` LF `)` / `f` LF `(g)` | two-line group / one argument / two lines |
| `map { x }` / `Point { x, y: 2 }` / `Self { x }` | block argument / record with a punned field / record |
| `if p == Point { x: 1 } { y }` | error: first `{` is the body |
| `{ [:e] => e.name` LF `[!] => "-" }` | error: expected `,` before the line break |
| `[:e]`, `[:]`, `[!e]`, `[!]` / `[!x]` / `[(!x)]` | outcome brackets / outcome / one-element list |
| `x | Rd<fs>`, `read<i64>`, `pl.into<Point>` / `a < b` / `a<b` | instances / comparison / error: expected `>` |
| `x = 1`, `[x, y] = e`, `Just x = e`, `x: i64 = e` / `cmd -c=80` / `x => y = 1` | bindings / a call / an arm |
| `pub f = a (b = 0) => …` / `pub ls` / `{ pub x = 1 }` | exported binding / error / error |
| `a b => …`, `Just e rest => …` / `Just Just e => …` | two-pattern heads / one nested constructor |
| `a` LF `=> b` | one arm: a leading `=>` continues |
| `A -> B -> C` / `() -> T` / `(A -> B) -> C` | right-nested / unit parameter / grouped |
| `f(a,b)` / `f (x)` / `f()` | explicit arguments / grouped argument / empty call |
| `(e).name` / `f .name` | postfix field / selector argument |
| `type P { pub x: i64, y: i64 = 0 }` / `type S { C(r: f64), D }` | fields / constructors |
| `impl P { x (y = 0) => Self { x, y }, f = p => … }` / `impl P: T { f = … }` | constructor arm and member / trait impl |
| `impl P { f = a => 1, b => 2 }` / `impl P { f = a => 1 b => 2 }` | member then constructor arm / error: bracket the arms |
| `use x.inc x.Point` / `use text 2` / `use fs ver 2` | names / version / error |
| `foo 1`, `foo` / `cat foo` | unknown name / text |
| `f 1` before `f = a => …` / `f = n => f n` / `impl P { a = … b …, b = … a … }` | unknown name / self-recursion / members see each other |
| `pub x = 1` then `pub x = 2` in a file / `use a` inside `a` / a used module with a bare line | exports the final `x` / check error: import cycle / check error: no main |
| `type point {}`, `type S { circle(r: f64) }`, `Point = 1` | case rule at the declaration |
| `type T<a> {}`, `x: Show a => …`, `pub len: T` in a program | declaration form |
| `sys.lookup`, `Polar.from`, `e.name`, `fs = e` then `fs.list` | module export / type member / members of a value, twice |
| `use nowhere`, `use geometry.Nope`, `fs.nope`, `Point.nope` | unknown module / not exported / unknown member, twice |
| `impl Vec: Len {}` / `impl P: Display { nope = … }` | `Self` is local / not the trait's method |
| `add = a b => a + b` | generalized Num-constrained function |
| `add2 = add 2` | one open spot |
| wrong record field | check error before any host call |
| `Maybe<Entry> | cat` versus `|: cat` | whole-value mismatch / payload bind |
| `find x | put out` / `find x || "" | put out` | outcome at a byte spot: check error / text written |
| `m = [:1]` / `m: Maybe<i64> = [:1]` | family unknown: check error / `Just 1` |
| `Point 3 4`, `Point 3`, `1 | Point 1` / `Point 3 4 5` | constructor, default, pipe fills `y` / extra argument |
| `Point { x: 3, y: 4, len: 5 }`, `Point 3 4 5`, `Point -len 5` outside its module | check error at the field or argument: `len` is private |
| `pl | Point.from`, `pl.into<Point>`, `pl | to<Point>` / `pl | Point` | conversions / `Point pl`: check error at an `i64` spot |
| `impl Point: From<Polar>` and `impl Polar: Into<Point>` | check error: one impl per direction |
| `"42" | read<i64>` / `n: i64 = "42"` / `n: i64 = "x"` | `Result` / `42` / check error |
| `read p | { [!e] => … }` with `read p : Maybe<T>` | check error: `Nothing` has no payload |
| `Either<(),i64> || 0` | check error preserving outcome identity (T1) |
| `older = d e => e.age > d` / `filter (older 7)` | member obligation in the scheme / solved at `Entry` |
| `s = .name` / `(f => 1) .name` | `a.name: b => a -> b` / check error at the dot |
| `find x |! log "gone"` / `find x |! find y` / `find x |! log` | `Maybe<Entry>`, the stage is a value / fallback / check error: open spot |
| `x = 1`, `inc = x => x + 1`, `even 4`, `[] == []` | `i64` / `Num a => a -> a` / `Bool` / ambiguous |
| non-total arms | check error, never runtime match failure |
| `300` at a `u8` literal spot | check error |
| dynamic numeric value outside `u8` | admitted cast then runtime Fault |

The [corpus](examples.fipe) supplies the complete surface; check AST
shape, inferred types and diagnostics, not only successful parses.

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
