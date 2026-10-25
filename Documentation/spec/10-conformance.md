# Conformance cases

Observables to test per stage. Lexical and syntax cases are covered by
public-API tests; checking and execution cases await their stages, and a
case citing [open decisions](11-open.md) is conditional on it.

The rows for syntax, types and checks are in
[checks](10-conformance-checks.md); the corner cases that reviews found
and fixed are in [corners](10-conformance-corners.md).

## Lexical (01)

Compare kinds, payloads, source slices and diagnostics.

| Source | Expected distinction |
| --- | --- |
| `x * 2` / `x*2` | name/operator/integer versus one word |
| `0x199 0o17 0b1010 1_000 -5` | magnitudes 409, 15, 10, 1000, negative 5 |
| `2024-01-01 0xZZ 1__0` / `1st` | words, not partial numbers or lexical errors / `1` with the suffix `st` |
| `18446744073709551616` | one exact integer: the lexer has no bound |
| `1e999` / `1e` | float spelling versus `1` with the suffix `e` |
| `10ms` / `2i` / `0r7Z` / `x10` | suffix `ms` / suffix `i` / prefix `r` and the digits `7Z` as written / a name |
| `0x1Fe` / `0ms` / `1e5` | all hex digits / the prefix `m` / an exponent |
| `re"a+b"i` / `f"abc"` / `f "abc"` | prefix and suffix / a prefix / a name and a string |
| `-n=-5 -l=False --la -- -` | attached flag tokens, word `--`, minus |
| `rd:4` | name, colon, integer; no resource literal exists |
| `Vec<Vec<U8>>` | two `>` symbols; no `>>` token |
| `cat x \` LF `y` | one line: names `cat`, `x`, `y`, no newline between |
| `cat x \ ` LF `y` (trailing space) | the same join |
| `a\b`, `\x` | words; `# note \` LF `y` | comment, newline, name |
| `/ ./x ../x /a` | slash symbol, three paths |
| `fs.Entry .name _ héllo` | qualified name, selector, wildcard name, word |
| `"\xC3\xA9" b"\xFF"` / `"\xFF"i` | UTF-8 string é, byte 255 with the prefix `b` / byte 255 with the suffix `i` |
| `"\xFF"` | invalid UTF-8 string error |
| `'a\nb'` / `"a\nb"` | raw backslash+n versus LF byte |
| `"#" # tail` | string hash, discarded comment |
| `a |` then LF then `b` | pipe continuation without that newline |
| `a ||` then two LFs then `b` | second newline retained |
| broken quote, LF, `x = 1` | one diagnostic; following line survives |
| `&` / `&&` / source NUL | symbol, pipe, stray; NUL never truncates source |
| initial U+FEFF then `ls` | name `ls` at byte 3, scalar column 2 |
| other U+FEFF / any literal U+200B | stray, including in comments, docs and quotes |
| `"\xEF\xBB\xBF"` / `"\xE2\x80\x8B"` | escaped marks are valid string data |

Also every keyword and non-keyword name, every operator beside its neighbor,
byte slices and scalar columns of `cat héllo "한글"`, CRLF, EOF inside an
escape, negative zero, radix boundaries and all five pipes.

## Docs and quotes (01, 02)

| Case | Expected distinction |
| --- | --- |
| `##x` / `## x` / `##  x` / `##! module` | doc `x` / `x` / ` x` / inner doc |
| `a | ## doc` LF `b` | pipe joins; doc token retained; no intervening newline |
| `# ordinary` / `## doc` | comment span outside tokens / doc token; trivia kept in tree |
| `## a` LF `## b` LF LF `x = 1` | binding doc `a\nb`, blank gap allowed |
| `## a` LF LF `## b` LF `x = 1` / `##! module` then `## item` | paragraph `a\n\nb` / module and item docs |
| public prelude and host declarations | each has a doc |
| docs on module, type, binding, field, constructor and impl member | attached to each owner; body binding too |
| `## orphan` at EOF / before a value or `use` / before `##!` | unattached-doc syntax error |
| `x = 1` LF `##! late` / inner doc inside a body | inner-doc-position syntax error |
| host doc `- depth: …` / `- missing: …` | existing spot accepted / declaration error |
| `:t ls` / `:ast` on documented code | doc above type / doc under its owner |
| raw quote across LF with indentation, `#` and `"` | exact bytes, one token and span; following positions physical |
| decoded quote with backslash LF and leading spacing | join drops all three; explicit `\n` retained |
| decoded quote with bare LF / raw quote at EOF | unterminated quote; decoded recovery keeps following line |
| affixed multiline quote / pipe after its closing quote | affix kept / same logical line |
| REPL raw quote opened on one input line, closed on the next | continuation, one string |

## Evaluation and hosts (08, 09)

Under a recording host that returns declared values and records every
operation, test values and ordered effects together:

- Named arguments evaluate once in source order (E1). A skipped `||`, `&&`,
`|:` or `|!` branch has no host effects.
- `Just 1 |: (_ => Just 2)` is `Just 2`, not nested (E1). `Nothing || 4`
returns 4; `Just 3 || 4` returns 3, not `Just 3`.
- `1.0 / 0.0`, `0.0 / 0.0`, `1e308 * 10.0` and `sqrt -1.0` Fault; no
infinity or NaN is a value (E2). `-0.0 == 0.0` is `True` and `show -0.0` is
`-0.0`; `show 0.1` is `0.1`, `show 1.0` is `1.0`; `round 2.5` is `2.0`.
`x: F64 = 0.1` is the nearest binary64; `big = _ => 9007199254740993` then
`x: F64 = big ()` is the check error that the value is not exact.
- `x = []` runs and shows `[]`; `x | show` after it is the check error
naming the unfixed element meaning; `x: Vec<Integer> = []` never asks.
- With a budget a run Faults `Budget` with the location of the action it
cannot pay, and prior effects stand; with none it runs to its end.
- Fault ends execution; prior effects remain; execution capabilities close.
A unit with any check error performs zero program effects.
- A reply or host input that does not fit its declared type Faults `Host`
"the reply is not a T" or "the input is not a T", the input before the run
starts; a last line of another open arity is refused before any effect. A
`put` whose write fails Faults `Host` at it. A closure made in a
`with` and called outside it sees the environment open at its call.
- VM and native backends produce identical observations from the same
typed unit and replayed host responses for a run no budget stops (H1).

Test monad identity and associativity for `Maybe` and `Result`. Each case
states its outcome independently; agreement between backends alone proves
nothing.

## Resource boundaries

These observations have executable owners; names below are test functions, not
additional language API. Refusal of an unsupported artifact is not successful
resource execution on that backend.

| Observation | Existing oracle |
| --- | --- |
| Contextual modifier/raw spans/joins, ordinary names | `tests/parse/items.rs::resource_modifier_is_contextual_and_keeps_original_byte_spans`; Tree-sitter `resource_headers_match_core_flags_and_name_spans` |
| Trusted declaration origin; main/Session/query/File refusal | `tests/decl.rs::resource_registration_is_a_trusted_host_api_and_is_atomic`; Session `resource_markers_in_main_session_query_and_files_are_not_host_registration` |
| No automatic Codec or data constructor; explicit Host operations and Packet preserved | `tests/check/casts.rs::resource_types_exclude_automatic_codec_and_data_construction`, `resource_host_written_constructor_full_check` |
| Exact generic/context identity, typed containers, opaque hiding refusal | VM `resource_pending_generic_arguments_and_typed_containers`, `resource_fresh_tables_reject_same_name_and_slot`, `resource_cannot_hide_in_opaque_data` |
| Frozen input/reply and direct/stepped refusal before later effects | VM `resource_direct_and_stepped_input_gate_module_init_and_preserve_alias`, `resource_stepped_reply_admits_exact_alias_and_refuses_before_later_effect`, `resource_reply_uses_frozen_fields_and_frozen_error_site` |
| NEW composite issuance, caller-only generic arguments and checked Packed association | VM `resource_projection_generic_envelope_factory_uses_only_pending_witness`, `resource_projection_fresh_packed_factory_has_no_seed_witness` |
| Paid paths, early data-only results, original-site pending Budget latch | VM `resource_projection_invalid_paths_keep_call_and_backend_untouched`, `resource_projection_proven_free_precedence_matches_ordinary_fastpath`, direct/stepped projection Budget tests |
| Success retention; NEW-only fault/abandon; grants/lease/close/disposal | Seven `tests/resource_lifecycle.rs` scenarios and actual REPL `resource_owner_boundaries_commit_abandon_and_dispose_real_tables` test |
| Current native/LLVM refusal; ordinary control; metadata not execution | Native `resource_program_emitters_refuse_private_unused_declarations`, `resource_unit_metadata_roundtrips_but_cannot_compile`, `resource_value_boundary_refuses_every_root_and_hidden_function_slot`; LLVM `resource_program_module_refuses_private_unused_declarations` |

### Finite recursive resource conformance

The checker and frozen traversal already admit finite recursive nominal
templates. VM `resource_recursive_templates_stabilize_without_instantiation`
distinguishes an ordinary unresolved `N<Vec<T>>` template from a required
nonconcrete `N<Rd<T>>` resource identity before a callback.
`resource_projection_recursive_paths_change_all_generic_arguments` also admits
an actual one-edge `Next(Leaf(reference))` reply with a changed generic
argument and rejects the shallow witness at that deeper position.

The following bounded finite-value observations use checked declarations and
actual Host replies, independently of the earlier type-only and one-edge
tests:

| Case | Qualified bounded observation and exact oracle |
| --- | --- |
| `N<T>{Leaf(value:T),Next(value:N<Vec<T>>)}` at `N<Integer>` | `resource_recursion_conformance_ordinary_depths_and_shape_negatives`: replies at depths 0, 1 and 2 retain exact values. Wrong scalar, constructor index, field count, recursive child and instantiated leaf each refuse with Host Fault. |
| `N<T>{Leaf(value:T),Next(value:N<Rd<T>>)}` at `N<Integer>` | `resource_recursion_conformance_concrete_nested_alias_and_scalar_negative`: one actual `acquire` issues `Rd<Integer>`; a later `get` admits `Next(Leaf(alias))` retaining the identical Resource Rc. A scalar stand-in refuses in a separate run. No acquisition replay or fresh nested-factory claim follows. |

Both oracles live in `tests/vm/resources.rs`. They are finite public boundary
observations, not an arbitrary-depth normalization or cyclic-value test.

The following controls already have qualified executable owners and are
reused, not new pending language requirements:

| Established control | Existing qualified observation |
| --- | --- |
| Unresolved resource versus ordinary data | `resource_recursive_templates_stabilize_without_instantiation`: unresolved Vec may reach the Host; required unresolved Rd refuses before callback. `resource_full_module_marker_and_unit_are_distinct_arguments` and `resource_zero_length_values_skip_demand_but_identity_does_not` retain fixed module/unit and zero-length distinctions. |
| Acyclic resource container and finite Budget refusal | `resource_pending_generic_arguments_and_typed_containers` admits exact aliases; `resource_work_budget_refuses_before_call_and_after_reply` exercises finite expansion/admission refusal at the relevant action. |

These cases do not demand eager infinite instantiation, cyclic value graphs,
unrestricted recursive runtime normalization or polymorphic recursive
inference. No source-language rejection is introduced merely to make this
table pass; broader recursion termination remains an open decision.