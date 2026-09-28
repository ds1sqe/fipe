# Lexer API

[Lexical structure](lexical.md) owns the behavior; this chapter owns its
Rust representation.

## Entry point

~~~text
lex::lex(text: &str) -> Lexed
Lexed { tokens: Vec<Token>, errors: Vec<LexError> }
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
| INTEGER | Int { negative: bool, magnitude: u64 } |
| FLOAT | Float(String), exact spelling including separators/sign |
| STRING | Str { value: String, raw: bool } |
| BYTES | Bytes(Vec<u8>) |
| NEWLINE, EOF | Newline, Eof |

`raw` distinguishes single quotes. `Symbol` and `Keyword` are finite
enums; `Keyword::from_word` recognizes exact keywords and
`Symbol::continues_line` the five pipes. `Reason` is the lexical contract's
diagnostic enum; `Stray` carries the scalar. `Display` gives a canonical
spelling that lexes back to the same kind and payload, not the source
spelling; EOF displays as `<eof>`.

## Source lookup

`Source::new(text)` borrows the text. `slice(span)` returns `None` for a
reversed, out-of-bounds or non-boundary span. `line_col(offset)` is
one-based line and scalar column, computed on demand in O(offset); an
offset beyond EOF clamps to EOF and one inside a scalar to its start.

Lexing is O(n) time and storage in the source length and never recurses.
Allocation failure is not a lexical diagnostic.
