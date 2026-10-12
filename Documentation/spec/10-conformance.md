# Conformance cases

Observables to test per stage. Lexical and syntax cases are covered by
public-API tests; checking and execution cases await their stages, and a
case citing [open decisions](11-open.md) is conditional on it. A row marked
*narrowing* waits for narrowing (what a body, a branch or a variable
proves), which is deferred.

The rows for syntax, types and checks are in
[checks](10-conformance-checks.md); the corner cases that reviews found
and fixed are in [corners](10-conformance-corners.md).

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

Also every keyword and non-keyword name, every operator beside its neighbor,
byte slices and scalar columns of `cat héllo "한글"`, CRLF, EOF inside an
escape, negative zero, radix boundaries and all five pipes.

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

## Evaluation and hosts (08, 09)

Under a recording host that returns declared values and records every
operation, test values and ordered effects together:

- Named arguments evaluate once in source order (E1). A skipped `||`, `&&`,
`|:` or `|!` branch has no host effects.
- `Just 1 |: (_ => Just 2)` is `Just 2`, not nested (E1). `Nothing || 4`
returns 4; `Just 3 || 4` returns 3, not `Just 3`.
- `1.0 / 0.0`, `0.0 / 0.0`, `1e308 * 10.0` and `sqrt -1.0` Fault; no
infinity or NaN is a value (E2). `-0.0 == 0.0` is `True` and `show -0.0` is
`-0.0`; `show 0.1` is `0.1`, `show 1.0` is `1.0`; `round 2.5` is `2`.
`x: F64 = 0.1` is the nearest binary64; `big = _ => 9007199254740993` then
`x: F64 = big ()` Faults at run time.
- `x = []` runs and shows `[]`; `x | show` after it is the check error
naming the unfixed element meaning; `x: Vec<Integer> = []` never asks.
- With a budget a run Faults `Budget` with the location of the action it
cannot pay, and prior effects stand; with none it runs to its end.
- Fault ends execution; prior effects remain; execution capabilities close.
A unit with any check error performs zero program effects.
- VM and native backends produce identical observations from the same
typed unit and replayed host responses for a run no budget stops (H1).

Test monad identity and associativity for `Maybe` and `Result`. Each case
states its outcome independently; agreement between backends alone proves
nothing.
