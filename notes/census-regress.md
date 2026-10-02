# census-regress: the free-gate's 25 violations on main were census misreads, not use-after-frees

notes/mem-overload-rollback.md reported that the census free-gate (notes/mem-recycle.md) on main (de3beaf) found
25-27 freed blocks still strongly reachable on the private monorepo; it was 0 when the recycling landed (37b748a).
A freed block that is still referenced and later reused would be a use-after-free in released code (0.1.5+), so
every violation was traced to the referring field and checked against the current layout (`offset_of!`).

**Result: all false positives.** The strong mark's hard-coded padding and header offsets went stale with the layout
pass (mem-small: Symbol 48 / 40 bytes, compact identifiers, the 24-byte type header). Rust also reordered the new
`Type` header: the symbol word is at +0 and the flags, ids and data tag at +8..+24. Each stale offset both invented
violations (padding read as pointers) and hid real references (pointer fields skipped). After the fix the gate is 0
in all runs and covers more of the heap than before. No recycling code changed; nothing released is affected.

## Reproduction (main d3bd85f, alloc-profile build, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`)

Precise walk: 50,362,689 program references, 0 to freed blocks (every run). Strong mark: 25 violations with 1 checker
in the default mode. With the census binary at c34c445 (latest main, before this fix): 24 single, 66 on 4 checkers,
75 with `TSRS_LAZY_MEMBERS=0`. They fall into four classes:

| class (1 checker) | referrer word | what the bytes are | verdict | visible since |
| --- | --- | --- | --- | --- |
| 9 x `InferenceInfo +40` -> freed mappers, type lists, info slices | +40..+48: `top_level`, `is_fixed` (bytes 40, 41), 6 padding bytes | `P::new_recycled` copies the struct with its padding from the stack; the words read `0x00007c01xxxx0100`: the two bools over the low bytes of a stale pointer | false (padding; the census never knew this type's padding) | 867bf1e (LSP merge: different stack contents), 9 of 25 there |
| 7 x `TypeAlloc<LiteralType> +16` -> freed mappers, contexts, lists | +16..+24 of the header: `id` (u32), `data_tag` (u8), 3 padding bytes | `id \| tag << 32 \| pad << 40` with a stale padding byte 0x7c and tag 1 reads `0x7c01_xxxx_xxxx`, the census arena's address range (the coincidence mem-small step 2 noted). The old rule skipped +20..+28, the old header's id and tag | false (header scalars) | 509c052 (header 32 -> 24 bytes) |
| 5 x composite `TypeMapper +8` -> freed child mapper | `m2` of a composite mapper (a real field) | the composites themselves are garbage, reached only through `TypeAlloc<TypeParameter> +72` (the two bools and padding, at +72 since the new header; the old rule still skipped +76..+84), `InferenceInfo +40` or relater key bytes. Escape tracking is consistent: a mapper that is never stored may hold a freed child | false (unreachable referrer) | 509c052 |
| 4 x relater heap blocks (`recursive_type_related_to`) -> freed mappers | the 3rd word of a `RelationKey` in `maybe_keys` / `maybe_keys_set` | `RelationKey::Pair(u64)` leaves the last 8 bytes of the 24-byte enum uninitialized; the words before are packed id pairs (`0x110e33000059c` = ids 1436, 1117747). Cleared tables keep the stale entries (vector slack, hash buckets) | false (dead enum payload) | 894d7d5 (1 hit: stack contents; `RelationKey` dates from 4073a92, before the recycling) |

The same four classes cover the 66 / 75 control hits (4 checkers mostly relater keys, opt-out mostly composites via
`TypeParameter +72`).

Hidden references (stale offsets that dropped real edges, so the gate could have missed a real violation):

- `header_words` skipped +20, +24, +28 of every type: since 509c052 +24 is the first data word (e.g.
  `TypeReference.resolved_type_arguments`, whose type lists are a recycled class).
- `LiteralType`'s padding list (+36..+52) covered `fresh_type` at +48 since 509c052.
- `Symbol`'s parent word keeps flag bits 62/63 (886c8a8, 894d7d5) and `Type`'s symbol word bit 63 (509c052): the
  strong mark dropped words with high bits, so symbol tails (members, exports) and alias records were not walked.
- Compact identifiers (05f7b5c) keep their flow node as address / 8 with mode bits: not followed (the precise walk
  covers flow nodes, so this one only cost coverage).
- `NodeAllocRare<` (51b6692) had no header rule at all.

Strong-mark coverage of the conservatively reachable bytes: 88.7% before (4,597 of 5,180 MB), 98.2% after (4,918 of
5,010 MB; mem-recycle had 94%).

## Bisect (first parent, 37b748a..d3bd85f, census build + one private-monorepo run per step)

493358a 0, 69f1472 0 (after compact identifiers), e5fda9a 0, **894d7d5 1** (first bad: one relater key), 509c052 11,
07456d6 11, 867bf1e 25, d3bd85f 25. (a6fc45a and 05f7b5c were skipped: the run was killed for memory while another
census ran.) So the count rose with layout changes and stack-content changes, not with any change to what is freed.

## What changed (alloc-profile build only; normal builds unchanged)

- `tsrs_core::census_layout(type_name, &[CensusField])`: the crate that owns an arena type registers, from
  `offset_of!` / `size_of`, its `NoPointer` ranges (scalars, padding, header words, enum payload), `Tagged` words
  (address in the low 48 bits, flags above), `X8` words (address / 8 for some modes) and `Slice` pointers (no edge when
  the length is 0). By full type name or a generic prefix (`TypeAlloc<`, `NodeAlloc<`, `NodeAllocRare<`).
  `tsrs_checker::types::census_layouts` (called by `new_checker`; types, literal values, type parameters, mapped
  types, type references, conditional roots, inference infos) and `tsrs_ast::census_layouts` (node headers,
  identifier words, symbol parent words, diagnostics). The hard-coded `padding_words` / `header_words` are gone.
- `put_relater`: in census builds the cleared key set is replaced and the key vector's slack zeroed
  (`census_reset`, `census_scrub_slack`), so stale `RelationKey` bytes are gone at the source.
- Report: `violation class` rows (freed site <- referrer type + offset), the number of registered layouts, and
  `TSRS_CENSUS_CHAINS=N` referrer chains.
- docs/DEBUGGING.md: "The census free-gate (run it after any memory or layout change)".

## Gates

- Census (c34c445 + this change, private monorepo): 0 violations and precise walk 0 with 1 checker, 4 checkers and
  `TSRS_LAZY_MEMBERS=0` (1 checker). Would-free: 15.5M blocks / 352 MB single, 23.8M / 510 MB on 4 checkers.
- Positive control: with `MapperCell::set`'s escape barrier disabled, `conditionalTypes1.ts` reports 94 violations
  (`ConditionalType` +96 / +104 holding freed mappers); 0 with the barrier.
- Normal build vs c34c445: conformance result trees identical in the default mode, `TSRS_LAZY_MEMBERS=0` and
  `TS_TEST_PROGRAM_SINGLE_THREADED=false` (13,458 / 2 / 2; types/symbols 12,779); fourslash 4,066 pass / 63 fail,
  same pass list; private monorepo diagnostics and `--extendedDiagnostics` counters identical (default and opt-out,
  1 and 4 checkers, opt-out 4 with `--checkerAssignment go`); `RUSTFLAGS="-D warnings" cargo +1.99.0 check
  --workspace --locked` and `-p tsrs_cli --features alloc-profile` clean.

## Not done

- Node header parents (x8 in the header word) are still not followed by the strong mark (as before); the precise
  walk checks them.
- Heap blocks have no types, so their uninitialized bytes can only be scrubbed at the source (as for the relater keys
  and mem-recycle's pooled vectors). A new pooled table of enum keys may need the same.
