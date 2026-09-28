# Lexer API

The public library uses core + alloc and forbids unsafe code. It has no
dependency, host lookup or I/O. Lexical behavior is owned by
[lexical structure](lexical.md); this chapter defines its Rust representation.

## Entry point

~~~text
lex::lex(text: &str) -> Lexed
Lexed { tokens: Vec<Token>, errors: Vec<LexError> }
Token { kind: Kind, span: Span }
Span { start: usize, end: usize }
LexError { span: Span, reason: Reason }
~~~

Payloads are owned, so tokens and errors can outlive the source. A span belongs
to the source passed to that call. Source identity remains the caller's.
The result always ends with one EOF token. Invalid literals emit diagnostics
instead of tokens; successful tokens before and after them remain available.
Consumers must not parse/execute a compilation unit whose errors are nonempty.

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

A string's decoded bytes are value.as_bytes(); raw distinguishes single quotes.
Symbol and Keyword are finite enums. Keyword::from_word recognizes only exact
keywords. Symbol::continues_line identifies the five pipeline symbols whose
following newline the lexer suppresses; a backslash line join emits nothing.
Reason is the diagnostic category enum from the lexical contract; Stray
carries the offending scalar.

Display gives a canonical spelling for diagnostics. It need not reproduce
radix, leading zeros, flag dash count or escape spelling. Source::slice gives
exact spelling. Canonical output from a lexer-produced non-EOF token lexes to
the same kind and payload. EOF displays as <eof>, not source code.

## Source lookup

Source::new(text) borrows the text. Source::slice(span) returns an optional
borrowed slice; reversed, out-of-bounds and non-boundary spans return None.
Source::line_col(offset) returns a one-based line/scalar-column pair. It
clamps an offset beyond EOF to EOF and one inside a scalar to its start.
Positions are calculated on demand; tabs do not expand to screen columns.

## Complexity

Scanning moves forward through char_indices. Word classification and bounded
lookahead inspect each source segment a constant number of times. Lexing costs
O(n) time and O(n) token/diagnostic storage for n source bytes. It does not
recurse with nesting depth. Position lookup independently costs O(offset).
Allocation uses the host Rust allocator; allocation exhaustion is not a lexical
diagnostic. Execution budgets are a later-stage host contract.
