# Conformance cases: corners

Rows of [conformance](10-conformance.md) for corner cases in every stage,
each once a defect: a row stays so the defect cannot return.

## Corner cases (01 to 07)

| Case | Expected distinction |
| --- | --- |
| `x = ## d` LF `y = 2`, also impl and record values | unattached-doc error; docs never carry a value onto a new line |
| 100 000 documented name bindings | parse in linear time, under one second in release |
| `## d` CR LF | doc text `d`, no terminal CR |
| `## d` LF `use fs` | unattached-doc error at the doc |
| `### x` / `##########` | ordinary comments |
| raw quote across CR LF | both line-end bytes retained |
| decoded quote with backslash CR LF and indentation | join drops CR LF and leading spacing |
| host doc `- https://example.com/manual` / `- depth:` | opaque bullet / spot head |
| `x = [1, 2] \| sum` / `x = round 2` / `f = n => n + 1` | `Integer` / `F64` / generic `Add a, Literal a => a -> a` |
| `[x] = [1] \| map (n => n + 1)` then `y: F64 = x` | check error: the pattern binding fixed `x` to `Integer` |
| REPL accepted raw quote or doc with a blank line, then `:host` | replay preserves the complete entry and its docs |
| REPL doc above signature then `= value`, or above type then `where` | one documented binding or type item |
| REPL stray scalar inside an open raw quote | closing line stays with the failed quote; next binding survives |
| REPL `x = 1 ## note` then `y = 2` | first entry errors; `y` is accepted |
| REPL 1 MB raw quote over 25 000 lines | only new lines scanned while open; enters in seconds |
| REPL `g = f 1 \` LF `2`, `f = a b => a` | joined application; `g: Integer` |
| REPL `:t 'a` LF `  b'` | echoed literal retains LF and indentation |
| REPL `##! module` alone then `:ast` | accepted empty module; its doc is printed once |
| REPL `:{`, types with mutual claims, `:}` | one compilation unit; commands inside are text; EOF also submits |
| unannotated `From<Polar>.from = p => Self p.r p.t` | `p` is `Polar`; conversion is callable |
| a claim member with missing/extra receiver, or a narrower parameter | check error; every member is held to the required full type |
| overloaded `Mul` through `scale v k` or `(v \| .mul) k` | select after the argument meaning is known |
| `type Key: Eq & Show`, passing `1` at `Key` | accepted by meeting both parts |
| `Eq -> Eq -> Bool = a b => a == b`; `Vec<Show> \| map show` | packed equality; `Vec<String>` |
| `type N: Integer` with no limits or body | check error, no alias |
| a binding annotation continued after `->` or `&`, then `=` | one binding |
| REPL `8` LF `/ 2` | continued division |
| complete `:t` or `:r` followed by an operator line | query answers immediately, independent of read chunks |
| REPL `type Shape` LF `where self.area: F64` | one member-limit declaration |
| supplied `Show.show: (self: Self) -> Integer` | rejected while loading declarations |
| 3 000 prefix `!` operators | syntax nesting-limit error, no stack overflow |
| repeated literal lines at `F64`, including known places | member limits are not repeatedly expanded as numeric limits |
| long sums, invalid calls and concatenations | linear processing of settled obligations and literal groups |
| conversion with known unmatched source / unknown target | names the unsupported source / asks for the target meaning |
| bare signature then another binding; binding ending `=` LF value | signature errors alone, next binding survives; one binding |
| `impl Missing { … }` | unknown type once, no foreign-module error |
| comparison of two session types both named `A` | diagnostic qualifies distinct session origins |
| private field through a direct or deferred dot | private-field diagnostic naming the field |
| `big = s => s.area > 10.0`; `big 1` | missing `area` at the argument |
| plain member repeated in a later session impl | duplicate-member error |
| repeated literal/member requirement; duplicate claim block; missing host members | one error per cause; missing members named |
| loaded file with interleaved resolve and type errors | diagnostics in source order |
| missing claim on `Integer`; `1 ++ 2` | no promise of a future block; missing `concat` named |
| `:r x`, bare `:load`, top-level `Self` | readable resolution; path usage; uppercase `Self` diagnostic |
| opaque supplied type claiming `Eq`, `Ord`, `Show` or `Default` without members | declaration error; no derivation without a body |
| claimed `Vec<Integer>`, `Maybe<Integer>` or function member narrowed to `U8`; `Len.len = self => -1` | structural member-type error; result limits checked too |
| `kf: Key a => a -> Bool = p => p == p`, `Key: Eq & Show`, `kf 1` | written combination met by its parts |
| session `type S: Show { f: Integer -> Integer }`, then its `Show` block | type line accepted; claim pending until block |
| `g: Show -> String = s => s.show`; `h = n => g (n + 1)`; `h 2.5` | accepted; packing does not fix the outer parameter to `Integer` |
| exported type with mixed `pub` claim members across overloads | operator, dot and conversion reach every claim member |
| cyclic bases, including bases with their own member limits | check error at the type declaration |
| `Eq<E, E>` or `Add<A, A, A>` | excess type-argument error |
| `Integer a => a -> a` / `Eq a => b -> b` | invalid member-limit prefix / ambiguous variable, at the binding |
| supplied `impl Nope: Show { … }` | unknown target at declaration load |
| `Default` through `String`, `ParseError` or another host type | no derived default |
| member-limit chain / many type and claim declarations / repeated literal bindings / user equality | no duplicate base expansion, indexed declarations, no substitution clone per single candidate |
| repeated uses of nested file modules; field typed by another imported module | resolve and type once per unit; field retains its imported type |
| inferred type deeper than the parser nesting limit | type-depth check error, no stack overflow |
| `k: Number a => a -> a = p => p + 1`; `f = k` | prints `Number a => a -> a`, no redundant `Literal` |
