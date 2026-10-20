# Trace fixtures

One file per example: the observation of a run of
`Documentation/examples/<example>.fipe` under the conformance host
(`tests/host/replies.rs`, its output policy in the header of
`tests/fixtures/host/commands.fipe`). `tests/host.rs` runs each example
twice and compares it with its file byte for byte; `FIPE_BLESS=1`
rewrites them. The debug adapter (`tools/dap`) replays the replies of
these files and writes the same lines.

A file is UTF-8, one item per line, LF-terminated, in this order:

1. The result, one line:
   - `value <v>`: the run's value in the language's spelling
     (`fipe::value::structural::show`);
   - or `fault in <namespace> at <line>:<column>: <reason>`: the Fault,
     located in the source its namespace names (`main` for the example,
     a file module's stem, a host declaration file's stem, `prelude`),
     line and column counted from one, the reason in the crate's words.
2. One line per host call, in the order of the run:
   `call <namespace>.<label> (<arguments>)[ with <key> <value>]... -> <reply>`.
   The arguments are shown values separated by `, `; each `with` pair
   is a key open at the call and its value, outermost first; the reply
   is a shown value, or `failure "<message>"` (the message quoted and
   escaped as a Rust string literal is).
3. `success "<bytes>"`: the success output.
4. `error "<bytes>"`: the error output.

Both outputs are written as Rust string literals (`{:?}` of the bytes
read as UTF-8): `\"`, `\\`, `\n`, `\t`, `\r` and `\u{..}` escapes.
Values are shown as `show` writes them: text in the language's
quotes (`\"`, `\\`, `\n`, `\t`, `\r`, `\0`, and `\xNN` for each
byte of another control character), records as their constructor and
fields, sequences in brackets.
