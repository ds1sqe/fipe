# fipe-repl-core

An embeddable Fipe REPL with no I/O. It uses `core` and `alloc` and depends
only on `fipe`. It is an optional Embed facility in the
[layer ownership model](../../Documentation/architecture.md); application Hosts
can use it without Standard. The driver owns I/O and output presentation.

Feed bytes to `Repl::feed`, handle each `Out` in order, and call `Repl::flush`
at end of input. The driver writes `Out::Write`, supplies file sources through
`Repl::file`, and answers host calls through `Repl::answer`. The `edit` and
`highlight` modules provide terminal editing and token classes.

Use `fipe-repl` for a terminal driver, or `fipe-cli` for the `fipe repl`
command with filesystem and environment access through the standard host.
See the crate's API documentation for the request/response protocol.

`Repl::host_sources` accepts declarations by their full module names, including
`text.open`. Supply `commands` first when present, then modules in dependency
order. The directory-based `:host` command still reads the conformance host's
fixed file list; custom hosts should supply named sources through the API.
