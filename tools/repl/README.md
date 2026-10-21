# fipe-repl

The Fipe REPL: one pump over `fipe-repl-core`, built for any OS `std`
supports (default) or for x86_64 Linux with no `std`
(`--no-default-features --features linux`).

```sh
fipe-repl [--cwd <dir>] [--host <dir>] [--strict] [--budget <n>] \
  [--compile-budget <n>]
```

An entry is the input up to a blank line; `....> ` prompts for its next
line. A line starting with `:` is a command; `:help` lists them, one line
each, and `:help name` prints a name's doc and type as `:t` does.

At a terminal the line is edited in place: Left, Right, Home and End
(Ctrl-A, Ctrl-E) move; Backspace, Delete, Ctrl-K, Ctrl-U and Ctrl-W
delete; Up and Down recall earlier lines; Tab completes the names in
scope, the commands, a module's members (`fs.li` to `fs.list`) and a
value's fields and members, and a second Tab lists several candidates.
The line is coloured by token class unless `NO_COLOR` is set. Ctrl-C
drops the entry; Ctrl-D on an empty line ends input. Piped input is read
as it comes, with no editing, colour or banner.

`python3 tools/repl/tests/pty.py BIN...`, from the repository root,
drives a binary through a terminal.
