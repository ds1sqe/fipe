# Lexical structure

[lexical.ebnf](lexical.ebnf) defines the character-level forms; the
boundary, priority and recovery rules here complete it.

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
comments and quoted literals; escapes may encode them. Spans and scalar
columns refer to the original source, including the skipped mark.

## Boundaries

A word is a maximal nonempty run excluding spacing, LF, controls,
U+FEFF, U+200B and:

```text
( ) { } [ ] , ; : = < > ! | & " ' #
```

Thus `a+b`, `x*2`, `my-file`, `2024-01-01` and `fs.Entry` are whole words,
and arithmetic needs boundaries: `x * 2`. A word is classified only once it
is complete. Before word scanning: `"` and `'` begin quoted literals, the
word `->` is a symbol, and a backslash line join is consumed (below). A
word that is a prefix and touches a quote begins it (§Quoted literals).

Symbols use longest match from this finite set:

```text
|: |! || && => -> == != <= >=
( ) { } [ ] , ; : = < > ! | &
```

`+ - * / % ++` are symbols when they are whole words. There is no `>>`
token: `Vec<Vec<U8>>` ends with two `>`. `=` always
delimits: `-n=-5` is `FLAG("n")`, `=`, `INTEGER(-5)`.

## Word classification

Apply the first matching row. Malformed numeric-looking words reach the final
row.

| Condition | Token/payload |
| --- | --- |
| number, with a prefix or suffix (§Numbers) | `INTEGER` or `FLOAT`, exact source spelling |
| exactly `+`, `-`, `*`, `/`, `%`, `++` | the corresponding symbol |
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

`ascii_letter` is A–Z or a–z; `decimal_digit` is 0–9. `_` is a name; other
Unicode bare text is a `WORD`. The keywords are exactly

```text
if else with type impl use pub
```

Everything else is a name: `True`, `Just`, `self`, `Self`, `where`, `fault`,
`let`, `fn`, `as`, `mod`, `struct`, `enum`, `for`, `ver`, `read`.
[Syntax](syntax.md) defines binding, function and conversion by position.

## Numbers

For each radix, `digits` means one or more valid digits with an optional
single underscore between adjacent digits. Prefixes are lowercase.

```ebnf
sign     = [ "-" ] ;
integer  = sign, ( dec | "0x", hex | "0o", oct | "0b", bin ) ;
exponent = ( "e" | "E" ), [ "+" | "-" ], dec ;
float    = sign, ( dec, ".", dec, [ exponent ] | dec, exponent ) ;
```

`dec`, `hex`, `oct`, `bin` use radices 10, 16, 8, 2; hex admits either
case. A sign is adjacent; there is no leading `+`. `1.`, `.5`, `0x`,
`0xZZ`, `1__0` and `2024-01-01` are words; a lone minus is a symbol. `-0`
keeps its sign.

A number may carry a program's prefix or suffix, read with no type
([members](members.md) §Literals). A numeric word is read in order:

1. After `0x`, `0o` or `0b` come that radix's digits, else it is a word.
2. The longest number from the start; the whole word is that number, so
   `1e5` is an exponent and `0x1Fe` all hex digits.
3. After a lone `0`, a lowercase letter other than `x`, `o` and `b`, then
   one or more letters and digits: a prefix and its digits, as written,
   with no suffix: `0r7Z`, `0ms`.
4. A letter, then letters, digits and underscores: a suffix, `10ms`,
   `2i`, `1st`, `1e`, `0x1Fg`. Anything else makes the word a word.

A name before digits is a name: `x10`. `0x`, `0o` and `0b` arrive
decoded; the digits after a program's prefix arrive as written.

An integer payload is sign and magnitude with no width. The lexer has no
bound on a number; a place has, and ranges are the checker's. A float
payload is its spelling; the checker converts it once, to its selected
`F32` or `F64` width, round-to-nearest ties-to-even, rejecting
finite-source overflow.

There is no resource literal: `rd:4` is `NAME("rd")`, `:`, `INTEGER(4)`.

## Quoted literals

`"…"` is a decoded string, `'…'` a raw string; spans include the quotes,
prefix and suffix. A name with no uppercase letter touching the opening
quote is its prefix, `re"a+b"`; a one-segment name touching the closing
quote is its suffix, `"a+b"i`. A keyword is neither, and another word
beside a quote is a token of its own. `f "abc"` applies `f`; `f"abc"` is
a prefix. `b"…"` is the prefix `b`, bytes by the prelude's claim. With a
prefix or suffix a decoded quote carries its bytes, which need not be
UTF-8. A raw quote crosses LF, keeping its bytes and indentation; it
closes at the next `'` and has no escapes. A decoded quote crosses LF only
by a `\` right before it: the `\`, the LF and the next line's leading
spacing are dropped, and `\n` writes an LF to keep. There is no triple
quote and no interpolation. Decoded literals admit exactly:

| Escape | Byte |
| --- | --- |
| `\n`, `\t`, `\r` | LF, TAB, CR |
| `\\`, `\"`, `\'` | backslash, double quote, single quote |
| `\0` | zero |
| `\xNN` | two hexadecimal digits, one byte |

An escape consumes exactly its spelling: `\x414` is `A4`. Unknown or
incomplete escapes are `BadEscape`. Literal U+FEFF, U+200B and controls
other than TAB, and LF in a decoded quote, are `Stray`; an unjoined LF in
a decoded quote is `UnterminatedQuote`. A decoded string with neither must
be valid UTF-8:
`"\xFF"` is `InvalidUtf8`, `"\xC3\xA9"` is `é`, `b"\xFF"` is valid.

## Comments and logical lines

`#` outside a quote, except `##`, is a comment through the next LF,
excluding it; it may begin right after a word. Its span is kept as trivia
outside the token stream. Stray scalars inside comments and docs are
still errors. `##` emits `Doc(text)` and `##!` emits `InnerDoc(text)`
through LF, excluding it, with one space after the marker stripped.
Consecutive lines of one form join with LF in the parser
([syntax](syntax.md)); runs separated by blank lines join with one empty
line, a paragraph. A doc line is blank for the logical-line rule, as a
comment line is. A quote across lines is one token of its opening logical
line; a pipe after its closing quote continues that line.
Every LF emits `NEWLINE`, including blank lines, except the first LF after
one of the five pipes `| |: |! || &&` when only spacing, a doc or a comment
intervenes; a second newline remains.

```text
a | # comment
  b                 # NAME(a), |, NAME(b), NEWLINE
```

A `\` followed only by spacing before LF or EOF, outside a quote or
comment, joins the lines: it, the spacing and the LF are discarded, no
`NEWLINE` is emitted, and the join is a token boundary (`x\` LF `y` is
`NAME(x)`, `NAME(y)`). Any other `\` is a word character (`a\b` is one
word), comment text, or an escape inside a decoded quote.

```text
cat x \
    y                 # NAME(cat), NAME(x), NAME(y), NEWLINE
cat x # note \
    y                 # NAME(cat), NAME(x), NEWLINE, NAME(y), NEWLINE
```

Which retained newlines end a line is the parser's rule; lexing knows no
nesting and no name's meaning.

## Diagnostics and progress

The reasons are `UnterminatedQuote`, `BadEscape`, `InvalidUtf8` and
`Stray`. A failed quote emits one diagnostic spanning the whole literal
and no token; an unterminated decoded quote ends before an unjoined LF or
at EOF, keeping that newline; an unterminated raw quote ends at EOF. A
stray scalar outside a quote is consumed and spanned alone.
Scanning continues, offsets stay original, and EOF is emitted exactly
once.

```text
banner = '
  Fipe
    a pipeline language
'                                 # "⏎  Fipe⏎    a pipeline language⏎"
msg = "the file \
       was not found"             # "the file was not found"
msg = "line one\n\
       line two"                  # "line one⏎line two"
sql = 'where name = "a" # text
       and size > 1'              # the # and the " are text
'user: fipe
 role: admin
' | put ./config.yaml             # three lines, the space before role kept
x = "unterminated
y = 2                             # error, as today
```
