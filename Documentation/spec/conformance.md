# Conformance cases

These cases define observables to test as each stage is implemented. They do
not imply that parsing or execution is implemented. Lexical cases are covered
by the current public-API tests.
Cases depending on [open decisions](open.md) are conditional on approval.

## Lexing

Compare token kinds, semantic payloads, exact source slices and diagnostics.
EOF occurs once. All spans are monotonic byte ranges on scalar boundaries.

| Source | Expected distinction |
| --- | --- |
| `x * 2` / `x*2` | name/operator/integer versus one word |
| `0x199 0o17 0b1010 1_000 -5` | magnitudes 409, 15, 10, 1000, negative 5 |
| `1st 2024-01-01 0xZZ 1__0` | words, not partial numbers or lexical errors |
| `18446744073709551616` | integer range error, not truncation |
| `1e999` / `1e` | float spelling versus word |
| `-n=-5 -l=False --la -- -` | attached flag tokens, word `--`, minus |
| `rd:4 rd:4x` | resource 4; name/colon/word for malformed prefix |
| `/ ./x ../x /a` | slash symbol, three paths |
| `fs.Entry .name _ héllo` | qualified name, selector, wildcard name, word |
| `"\xC3\xA9" b"\xFF"` | UTF-8 string é, byte 255 |
| `"\xFF"` | invalid UTF-8 string error |
| `'a\nb'` / `"a\nb"` | raw backslash+n versus LF byte |
| `"#" # tail` | string hash, discarded comment |
| `a |` then LF then `b` | pipe continuation without that newline |
| `a ||` then two LFs then `b` | second newline retained |
| broken quote, LF, `let x = 1` | one diagnostic; following line survives |
| `&` / `&&` / source NUL | stray, pipe, stray; NUL never truncates source |
| initial U+FEFF then `ls` | name `ls` at byte 3, scalar column 2 |
| other U+FEFF / any literal U+200B | stray, including in comments and quotes |
| `"\xEF\xBB\xBF"` / `"\xE2\x80\x8B"` | escaped marks are valid string data |

Test every keyword and every one-/two-character operator beside its neighbor.
For `cat héllo "한글"`, verify byte slices and scalar columns independently.
Include trailing comments, CRLF, source EOF in an escape, invalid hex,
negative zero, all radix boundaries and all five continued pipes.

## Parsing and checking

| Case | Expected distinction |
| --- | --- |
| `add 2 - 1` / `add (2 - 1)` | rejected argument operator / grouped operand |
| `cat x > y` / `(x > y)` | redirect / comparison (S2) |
| `Vec<Vec<u8>>` | two generic closers, not redirect (S2) |
| `let add = fn(a,b) { a + b }` | generalized Num-constrained function |
| `let add2 = add 2` | one open spot |
| wrong record field | check error before any host call |
| `Maybe<Entry> | cat` versus `|: cat` | whole-value mismatch / payload bind |
| `Either<(),i64> || 0` | check error preserving outcome identity (T1) |
| non-total arms | check error, never runtime match failure |
| `300` at a `u8` literal spot | check error |
| dynamic numeric value outside `u8` | admitted cast then runtime Fault |

The [corpus](examples.fipe) supplies complete surface forms, including the
opening language example, items, marked pipes, arrays, comments and Unicode.
Check AST shape, inferred types and diagnostics, not merely successful parses.

## Evaluation and hosts

Use a deterministic recording host that returns declared values and records
every invoked operation. Test values and ordered effects together:

- Named arguments evaluate once in source order (E1).
- A skipped `||`, `&&`, `|:` or `|!` branch has no host effects.
- `Just 1 |: (_ => Just 2)` is `Just 2`, not nested (E1).
- `Nothing || 4` returns 4; `Just 3 || 4` returns 3, not `Just 3`.
- Fault ends execution; prior effects remain; execution capabilities close.
- A unit with any check error performs zero program effects.
- The exact budget boundary faults before the first unaffordable action (H1).
- VM and native backends produce identical observations from the same typed
  unit and replayed host responses, including charge points (E2/H1).

Test prelude monad identity/associativity for `Maybe` and `Result`, including
error propagation. Equality of optimized implementations alone cannot prove
the language rules; each case also needs an independently stated outcome.
