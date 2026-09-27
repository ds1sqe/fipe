# Lexical structure

This chapter defines the token stream for plan 01.0, independently of parsing
or hosts. [lexical.ebnf](lexical.ebnf) defines the character-level forms;
the boundary, priority and recovery rules here complete that grammar.

## Source and positions

A source is UTF-8 text. Hosts reject invalid UTF-8 before lexing. Offsets are
bytes; `[start, end)` always lies on scalar boundaries. EOF has span `[n, n)`
for a source of `n` bytes. Lines and columns are one-based; columns count
Unicode scalar values, not bytes, grapheme clusters or screen cells.

LF ends a physical line. CR is spacing; CRLF therefore yields one newline.
Spacing is TAB, CR, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028,
U+2029, U+202F, U+205F and U+3000. Controls are U+0000–U+001F and
U+007F–U+009F. Apart from TAB, CR and LF, they are lexical errors, including NUL;
NUL is never an EOF sentinel. No source normalization or identifier folding
occurs.

Skip one byte-order mark (U+FEFF) only at byte offset zero. Elsewhere U+FEFF
and every literal zero-width space (U+200B) are `Stray` errors, including in
comments and quoted literals. These two scalars cannot join words silently.
Escapes may encode them as literal data. Spans and scalar columns always
refer to the original source, including the skipped initial mark.

## Boundaries

A word is a maximal nonempty run excluding spacing, LF, controls,
U+FEFF, U+200B and:

```text
( ) { } [ ] , ; : = < > ! | & " ' #
```

Thus `a+b`, `x*2`, `my-file`, `2024-01-01` and `fs.Entry` are whole words.
Arithmetic `+ - * / %` needs token boundaries: `x * 2`, not `x*2`.
Classification occurs only after the whole word is known.

Three prefixes intervene before ordinary word scanning:

- `"` and `'` begin quoted literals; `b"` at a token boundary begins bytes.
- `rd:` followed by a whole unsigned decimal digit run is one resource-id
  literal. `rd:4x` instead yields `NAME("rd")`, `:`, `WORD("4x")`.
- A word consisting of `-` immediately followed by `>` becomes `->`.

Symbols use longest match from this finite set:

```text
|: |! || && => -> == != <= >= >>
( ) { } [ ] , ; : = < > ! |
```

Lone `&` is an error. `+ - * / %` are symbol tokens when they are whole words.
`>>` is one token even inside a type; syntax specifies its use as two closers.
`=` always delimits: `-n=-5` is `FLAG("n")`, `=`, `INTEGER(-5)`.

## Word classification

Apply the first matching row. Malformed numeric-looking words reach the final
row; numeric overflow is an error only after recognizing a valid spelling.

| Condition | Token/payload |
| --- | --- |
| valid integer or float | `INTEGER` or `FLOAT`, exact source spelling |
| exactly `+`, `-`, `*`, `/`, `%` | the corresponding symbol |
| one/two dashes then an ASCII letter | `FLAG`, text after the dashes |
| starts `/`, `./` or `../` | `PATH`, original text |
| `.` then a qualified name | `SELECTOR`, name segments |
| exact keyword below | keyword symbol |
| qualified name | `NAME`, name segments |
| any other word, including `--` | `WORD`, original text |

```ebnf
name           = name_start, { name_continue } ;
qualified_name = name, { ".", name } ;
name_start     = ascii_letter | "_" ;
name_continue  = name_start | decimal_digit ;
```

`ascii_letter` is A–Z or a–z; `decimal_digit` is 0–9. `_` is a name whose
pattern meaning is wildcard. Other Unicode bare text remains a `WORD`.
`True`, `False`, `Just`, `Nothing`, `Ok`, `Err`, `Left`, `Right`, `self`,
`fault`, `where`, `api`, `read` and `write` are names, not reserved words.

Keywords are exactly:

```text
let if else with fn struct enum trait impl for type use ver as
```

## Numbers

For each radix, `digits` means one or more valid digits with an optional
single underscore between adjacent digits. Prefixes are lowercase.

```ebnf
sign     = [ "-" ] ;
integer  = sign, ( dec | "0x", hex | "0o", oct | "0b", bin ) ;
exponent = ( "e" | "E" ), [ "+" | "-" ], dec ;
float    = sign, ( dec, ".", dec, [ exponent ] | dec, exponent ) ;
```

`dec`, `hex`, `oct`, `bin` use radices 10, 16, 8, 2 respectively; hex admits
A–F and a–f. A sign is adjacent. There are no suffixes or leading `+` literals.
`1.`, `.5`, `1e`, `0xZZ`, `1__0` and `1st` are words. A minus by itself is a
symbol. `-0` retains its sign in the lexical payload.

An integer payload records sign and magnitude without choosing a language
width; magnitude above `2^64 - 1` is `IntegerRange`. Slot checking owns signed
ranges, including whether a positive value fits an inferred `i64`.

The lexer preserves a float's decimal spelling without converting to
binary64. Checking converts directly to its selected `f32` or
`f64` width, round-to-nearest ties-to-even, and rejects finite-source overflow.
Underflow may round to a signed zero. This avoids double rounding through f64.

An `RD` payload is an unsigned decimal id at most `2^64 - 1`, without sign or
underscores. Overflow is `ResourceIdRange`. It performs no resource lookup.

## Quoted literals

`"…"` is a decoded string literal, `'…'` a raw string literal, `b"…"` a
decoded byte literal. Their byte spans include prefixes and quotes. A quote
does not cross LF. A raw quote closes at the next `'`; it has no escape form.
Ordinary scalars contribute their UTF-8 bytes. No interpolation exists.

Decoded literals admit exactly:

| Escape | Byte |
| --- | --- |
| `\n`, `\t`, `\r` | LF, TAB, CR |
| `\\`, `\"`, `\'` | backslash, double quote, single quote |
| `\0` | zero |
| `\xNN` | two hexadecimal digits, one byte |

An escape consumes exactly its spelling: `\x414` contributes `A4`.
Unknown escapes and incomplete hex escapes are `BadEscape`. Literal U+FEFF,
U+200B and controls other than TAB are `Stray`; escaped data is permitted.
LF before the closing quote is `UnterminatedQuote`, not a stray control.

A decoded string must be valid UTF-8 (`"\xFF"` is
`InvalidUtf8`), while bytes need not be (`b"\xFF"` is valid). Validation
occurs after all escapes so `"\xC3\xA9"` is `é`.

## Comments and logical lines

`#` outside a quote consumes through the next LF or EOF, excluding LF.
Comments discard syntax, but invalid controls, U+FEFF and U+200B still
produce `Stray` errors.
A comment can begin immediately after a word. Every LF emits `NEWLINE`,
including blank lines, except the first LF after a pipe token when only
spacing/comment intervenes. All five pipes participate: `| |: |! || &&`.
Suppression consumes one physical newline; a second blank newline remains.

```text
a | # comment
  b                 # NAME(a), |, NAME(b), NEWLINE
```

Grouping may let the parser ignore retained newlines. Lexing never needs to
know nesting or whether a word is a variable, command or filename.

## Terminal inventory

The syntax grammar uses payload terminals `NAME WORD FLAG PATH SELECTOR
INTEGER FLOAT STRING BYTES RD`, structural terminals `NEWLINE EOF`, and the
quoted keyword/symbol spellings above. Strings record decoded bytes and raw
versus decoded spelling; bytes record decoded bytes. Exact spelling always
remains recoverable from the source span.

## Diagnostics and progress

A failed quoted literal emits one diagnostic and no
literal token. Skip through its matching quote (respecting escapes) or stop
before LF/EOF. Preserve the newline. A stray character outside a quote is
consumed individually. A range error consumes the whole literal. Continue
scanning and emit EOF exactly once, even after errors.

Diagnostic reasons are `UnterminatedQuote`, `BadEscape`, `InvalidUtf8`,
`Stray`, `IntegerRange`, `ResourceIdRange`. Span a failed quote or number in
full; span an outside stray character alone. An unterminated span ends just
before LF or at EOF. If several quote faults exist, report the first detected
one. Tokens before and after an error retain their original source offsets.
