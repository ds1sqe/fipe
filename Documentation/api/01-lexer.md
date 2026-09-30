# Lexer API

[Lexical structure](../spec/01-lexical.md) owns the behavior; this chapter owns its
Rust representation.

## Entry point

~~~text
lex::lex(text: &str) -> Lexed
Lexed { tokens: Vec<Token>, comments: Vec<Span>, errors: Vec<LexError> }
Token { kind: Kind, span: Span }
Span { start: usize, end: usize }
LexError { span: Span, reason: Reason }
~~~

Payloads are owned, so tokens and errors outlive the source; a span
belongs to the text passed to that call. The result ends with one EOF.
Invalid literals emit diagnostics instead of tokens, and the tokens around
them remain; a unit with errors is never parsed or executed.

## Representation

| Lexical terminal | Kind |
| --- | --- |
| reserved word | Kw(Keyword) |
| punctuation or operator | Symbol(Symbol) |
| NAME, SELECTOR | Name/Selector(Vec<String>), segments without dots |
| WORD, PATH | Word/Path(String), exact spelling |
| FLAG | Flag(String), without leading dash(es) |
| INTEGER | Int { negative: bool, magnitude: Magnitude } |
| FLOAT | Float(String), exact spelling including separators/sign |
| STRING | Str { value: String, raw: bool } |
| with a prefix or suffix | Affixed(Box<Affixed>) |
| docs | Doc(String), InnerDoc(String), one physical line per token |
| NEWLINE, EOF | Newline, Eof |

`raw` distinguishes single quotes. `Magnitude` is an exact natural of
any size held in `alloc`: `to_u64`, `is_zero`, `parse(text, radix)`,
order and decimal `Display`. `Affixed { prefix, form, suffix }` holds a
number or quote with a prefix or suffix, each `String` empty when not
written; for a number the prefix is the letter after `0`. `form` is a
`Written`: `Int` and `Float` as above, `Digits { negative, digits }` as
written after a program's prefix, or `Text { value: Vec<u8>, raw }`, so
`b"…"` is `Affixed` with the prefix `b`. `Symbol` and `Keyword` are
finite enums; `Keyword::from_word` recognizes exact keywords
and `Symbol::continues_line` the five pipes. `Reason` is the lexical
contract's diagnostic enum; `Stray` carries the scalar. `Display` gives a
canonical spelling that lexes back to the same kind and payload, not the
source spelling; EOF displays as `<eof>`.

Comment spans include `#` and exclude LF, in source order, outside tokens.
Docs are tokens, not comment trivia. A multiline quote has one span from
its prefix/opening quote through its suffix/closing quote; source lookup
after it counts every physical LF and scalar column in the original text.

## Source lookup

`Source::new(text)` borrows the text. `slice(span)` returns `None` for a
reversed, out-of-bounds or non-boundary span. `line_col(offset)` is
one-based line and scalar column, computed on demand in O(offset); an
offset beyond EOF clamps to EOF and one inside a scalar to its start.

Lexing is O(n) time and storage in the source length and never recurses,
except that a decimal integer's magnitude takes time quadratic in its
digits.
Allocation failure is not a lexical diagnostic.
