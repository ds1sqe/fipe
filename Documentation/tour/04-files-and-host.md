# Files and the host

The std host declares the commands a script may call; `ls`, `cat`,
`find`, `rm`, `put` and `append` are the ones a shell user expects
([`tools/host/README.md`](../../tools/host/README.md) lists them all,
with their shapes). Paths resolve against the working directory.

```fipe
ls ./
```

`ls` gives a `Vec<Entry>`; each entry has `name`, `path`, `size` and
`age` (seconds since its last change). A flag names a labeled spot:
`ls ./ -r -depth 2` descends, `ls ./ -name "*.txt"` filters by a
pattern of `*` and `?`, matching case and a leading dot as written.
`du` reports sizes, largest first:

```fipe
# Report each entry under a directory with its size, largest first, then the total.
use cli
dir = first cli.args || "."
entries = ls dir | sort (e => 0 - e.size)
line = e => join "\t" [show e.size, e.name]
total = entries | map (e => e.size) | sum
entries | map line | concat (to<Vec<String>> [join "\t" [show total, "total"]]) | join "\n"
```

`e.size` reads a field; `sort` orders by the number its function
gives, so `0 - e.size` puts the largest first.

## Reading and writing

`cat` reads an entry's text. `put` and `append` write bytes to a sink:
a path, or `out`, the standard output.

```fipe
"first line\n" | bytes | put ./notes.txt
"second line\n" | bytes | append ./notes.txt
find ./notes.txt |: (e => Just (cat e)) || "missing"
```

A `String` reads as a sink by its path, so a lower-case name bound in
the script shadows the host's `out`: `out = "report"` then `put out`
writes a file named `report`. Keep `out` for the host.

## Write roots

Reads go anywhere the process may read. Writes, appends and removals
land only under the working directory, or a directory passed with
`--allow-write DIR`; `put ../x` or a link leaving the root is refused
with a Fault, and `rm ../x` gives `Err` with the reason. No script
spawns a process.

## Dry runs

`rm-old` lists what it would remove and removes nothing:

```fipe
# Dry run: list the files under a directory older than N seconds.
use cli
dir = first cli.args || "."
limit = (cli.args | filter (a => a != dir) | first || "0") | read<U64> || 0
old = ls dir -r -depth 8 | filter (e => e.age > limit)
old | map (e => join " " ["rm", e.path]) | join "\n"
```

The plan prints; a second script, or a flag of this one, could pipe
the paths into `rm`, whose `Result` says which succeeded.
