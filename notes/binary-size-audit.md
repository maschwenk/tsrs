# Binary size reduction candidates

The largest data opportunity is compressing the embedded TypeScript libraries: 3.03 MiB of payload before decoder
overhead. The strongest code opportunities are a narrower matcher for auto-import exclusion regexes and sharing
the compiler-option traversal that is currently expanded into three large functions. These are candidates, not
measured source-change savings. The subsequent [regex implementation](binary-size-regex.md) saves 420.6 KiB
(2.09%) with existing features preserved. The baseline attribution and other candidates below are unchanged.

## Build and measurement

2026-10-10, macOS arm64, Rust 1.99.0 (`b940084d7`), cargo-bsize 0.0.2, committed HEAD
`2fbb51f412d8b0d7a29dcc6f88e8bcd04ac27d64`. The separate worktree is
`/private/tmp/tsrs-binary-size-audit`, branch `codex/binary-size-audit`; the original checkout's uncommitted Unicode
changes are excluded. This is the local release profile (opt-level 3, fat LTO, one codegen unit, unwind), without
the release workflow's PGO or Linux BOLT. Repeat on those builds before treating the ranking as release evidence.

```sh
mkdir -p target/size-audit
SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk \
CFLAGS='-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk' \
cargo bsize --bin=tsrs --limit=50 --frozen > target/size-audit/bsize.md 2> target/size-audit/bsize.log
/usr/bin/strip -o target/size-audit/tsrs-stripped target/bsize/release/tsrs
target/size-audit/tsrs-stripped --version
```

The explicit SDK avoids this host's old Xcode linker reading the newer Command Line Tools SDK. Other hosts should
use their matching SDK or omit those environment variables. [cargo-bsize](https://docs.rs/crate/cargo-bsize/0.0.2)
retains symbols and full debug information for attribution and emits the final assembly under `target/bsize`.

The unstripped executable is 25,179,448 bytes. The actually stripped copy is **20,579,296 bytes (19.626 MiB)** and
passes `--version`. The report estimates 19.4 MiB shipped; its Mach-O model subtracts the entire `__LINKEDIT`,
whereas stripping retains about 240 KiB there. Use the actual stripped size for future A/B comparisons.

| Category | Bytes in the report | Share of its shipped estimate |
| --- | ---: | ---: |
| Code | 12.2 MiB | 63.0% |
| Read-only data | 5.4 MiB | 28.0% |
| Unwind and exception tables | 1.7 MiB | 8.9% |

The raw report, build log, stripped executable, `embedded-libs.json`, and `measurements.json` (including hashes)
are in the worktree's ignored `target/size-audit/`. Section, symbol, inlining, and retained-graph views overlap;
their numbers must not be added together. Retained sizes model removing an entire function and its exclusively
reachable code/data, not replacing it with a smaller implementation.

## Ranked source opportunities

### Compress the embedded libraries

`crates/tsrs_vfs/src/bundled/embed_generated.rs` embeds 113 files through `include_str!`. Reading those exact files
and compressing each with Python `zlib.compress(bytes, 9)` gives:

| Representation | Bytes |
| --- | ---: |
| Current source payload | 3,791,746 |
| Independent compressed payloads | 612,895 |
| Payload saving | 3,178,851 (3.03 MiB; 15.4% of the stripped executable) |
| One concatenated compressed payload | 553,407 |

The report confirms the bytes are linked: `embedded_contents` retains 3.6 MiB, and the largest strings are
`lib.dom.d.ts` (2,349,483 bytes), `lib.webworker.d.ts` (787,076), and `lib.es5.d.ts` (218,855). Compression has not
been implemented; these figures exclude the decoder, metadata, and binary-layout changes.

A possible implementation would generate independent compressed blobs and decompress each requested library once,
keeping the exact UTF-8 bytes available for the existing `&'static str` API. Independent blobs allow unused
libraries to stay compressed. Preserve the full library contents, including comments and offsets, since
diagnostics and compact identifiers use the source text. Change the generator as well as its output.

`notes/mem-zero-copy-libs.md` deliberately removed heap copies of these strings. Decompression would give that up
and add startup work. Measure single-threaded instructions and default-count peak RSS on the pinned projects,
including LSP reuse, before choosing this tradeoff. This targets installed executable size; the npm archive
already compresses the original text, so the same saving does not follow for download size. The new constraint
relative to the earlier zero-copy work is the explicit binary-size investigation.

### Narrow the auto-import regex matcher

`crates/tsrs_modulespecifiers/src/util.rs:41` only asks whether a pattern matches a module specifier. The reference
graph attributes **944.8 KiB** of exclusively retained code and constants to `is_excluded_by_regex`; `Regex::new`
alone retains 931.2 KiB. This is an upper bound on the whole current implementation, not a predicted saving.
The CLI feature tree confirms `tsrs_modulespecifiers` requests regex's default Unicode and performance features.

Investigate a matcher specialized to boolean searches, such as a single PikeVM engine, with the same regex syntax,
Unicode and case folding. Keep the existing pattern cache, invalid-pattern handling, slash/flag parsing and
process-wide allocation lifetime. A replacement must still handle arbitrary user patterns; an ASCII-only or
literal-only matcher would change behavior. A smaller engine may make this LSP path slower, so compare both the
executable and exclusion-heavy completion requests. Merely deleting `is_excluded_by_regex` would remove a supported
preference and is not a suitable optimization.

Implemented after this audit: [the Unicode-capable PikeVM adapter](binary-size-regex.md) saves **420.6 KiB (2.09%)**
in the linked CLI. It retains the old syntax, matching, compilation limits and literal-alternation acceptance paths.
The implementation note records differential checks and the measured completion-latency tradeoff.

### Share compiler-option traversal

`crates/tsrs_tsoptions/src/declscompiler.rs:1413` expands the same field comparisons for three different filters.
`options_have_changes` occupies **101.2 KiB** in three copies of about 33.7 KiB. The report's one-copy estimate is
**67.4 KiB** recoverable before adapter costs. A non-generic body receiving a filter function could preserve the
existing comparison order and strict/allow-JS handling. Verify that fat LTO actually keeps one body: the estimate
does not establish that changing the signature saves those bytes.

The adjacent macro-generated traversal at `parsinghelpers.rs:851`, `for_each_compiler_option_value`, is the largest
function in the binary at **71.3 KiB**; `compiler_options_to_go_json` is another 43.7 KiB. Inspect shared field
access/conversion helpers rather than rewriting option semantics. These are cold configuration/build-info paths,
unlike the narrowing callbacks rejected in `notes/monomorphization-audit.md`. The new evidence is the final linked
whole-program footprint outside the checker, rather than pre-optimization checker LLVM IR.

### Reduce repeated LSP construction and codec bodies

`crates/tsrs_lsp/src/server.rs:481`, `register_content_mapper_extensions`, is **62.8 KiB**. It constructs many
`RegisterOptions` values with one populated field and repeated defaults/clones. Shared constructors that remain
out of line are a candidate; preserve registration order and capabilities.

The generated `TextDocumentClientCapabilities::from_json` is **42.7 KiB**, including **35.7 KiB (84%)** attributed
to inlined helpers. `lsp_generated.rs` has 205.2 KiB of surviving code. Inspect the generator and
`crates/tsrs_lsproto/src/structcodec.rs` for repeated field-reading scaffolding. These are footprint figures, not
saving estimates, and need LSP JSON round trips and request benchmarks before adoption.

## Lower priority and prior results

- No duplicate dependency versions were found.
- Identical assembly bodies account for an estimated 242.7 KiB across 859 groups. They are mostly map/drop
  specializations spread across many sites; this is not one easy source change.
- Stable-sort quicksort and drift families total 513.9 KiB across 89 instances each. Do not replace stable sorts
  indiscriminately: equal-key order can reach diagnostics and emitted text. Type-erasing comparators would add
  runtime calls and needs evidence beyond this attribution.
- `Debug` impls total 168.5 KiB across 473 impls, with the largest individual impl only 3.0 KiB. Blanket derive
  removal is lower value than the targets above; unused derives are already eliminated.
- `checkerPool::for_each_checker_group_do_ex` has seven copies totaling 102.0 KiB, plus generic closures. Splitting
  its type-independent setup is a secondary candidate, but it schedules checking and needs stronger runtime
  evidence than the cold option walkers. Keep hot checker predicates and arena fast paths as previously decided.
- The 1.7 MiB of unwind tables is not freely removable: LSP/API panic recovery and project rollback need unwinding
  (`notes/perf-build-level.md`). No abort build was repeated.
- Stripping/fat LTO/one codegen unit are already in place (`notes/release-profile-size.md`). The earlier cold-crate
  size profile shrank Linux text 0.8 MB with unchanged checking speed (`notes/perf-build-level.md`); the current
  source candidates are more specific than repeating that configuration experiment.

The initial audit changed documentation only. Its subsequent regex source change and before/after measurements
are recorded in [binary-size-regex.md](binary-size-regex.md). The lint ratchet passes (three existing findings,
none new), source checks pass, and the stripped binary's version smoke check passes. Capability counts and README
status are unchanged.

## Complete footprint and opportunity inventory

The following partition reads section sizes from the stripped Mach-O file and sums exactly to its file size.
The writable zero-fill sections are excluded: they reserve runtime memory but occupy no source payload bytes.

| Component | Bytes | MiB | File share |
| --- | ---: | ---: | ---: |
| Machine code | 12,820,592 | 12.227 | 62.30% |
| Embedded library source | 3,791,746 | 3.616 | 18.43% |
| Other read-only data | 1,899,233 | 1.811 | 9.23% |
| Unwind and exception tables | 1,804,384 | 1.721 | 8.77% |
| Writable data | 14,520 | 0.014 | 0.07% |
| Loader, headers, padding and signature | 248,821 | 0.237 | 1.21% |

### All source targets and overhead

Footprints overlap the crate, generic, inlining and constant views. One-copy ceilings describe collapsing a
family before adapters and optimizer changes; they are not measured savings or additive budgets. A retained
footprint describes removing the whole reachable implementation, so a replacement saves only some of it.

| Target | Current footprint | Saving evidence | Candidate and constraint |
| --- | ---: | --- | --- |
| Embedded libraries | 3.62 MiB | 3.03 MiB payload measured | Generate compressed blobs; decode requested libraries once. Adds startup work and heap storage; current reads are zero-copy. |
| Auto-import regex matcher | 944.8 KiB retained | 420.6 KiB measured linked CLI saving | Implemented Unicode-capable PikeVM adapter; syntax, flags, invalid patterns, compilation limits and cache behavior preserved. See binary-size-regex.md for latency. |
| Compiler-option comparisons | 101.2 KiB / 3 copies | 67.4 KiB one-copy ceiling | Keep one non-generic comparison body with small filter adapters. Fat LTO may specialize it again; preserve strict and allow-JS semantics. |
| Compiler-option field traversal | 71.3 KiB body | Unknown | Share field access and conversion scaffolding. Also inspect 43.7 KiB JSON conversion and 24.9 KiB merge routines. |
| Unicode case mappings | 191.3 KiB constants | Unknown | Pack offsets/lengths or simple scalar mappings in generated data. 2,927 records currently carry three string slices; preserve final sigma and WTF-8. |
| Unicode normalization and collation | 56.3 KiB exclusive constants | Unknown | Pack decomposition records and string offsets. Preserve NFD, combining-mark order and import collation. |
| AST Kind formatting tables | 53.4 KiB across 17 reported bodies | Unknown | Share an out-of-line formatter or enum-name lookup. Preserve Debug output; verify LTO does not recreate specialized tables. |
| Generated LSP JSON codecs | 205.2 KiB surviving code | Unknown | Share field parsing and writing helpers in the generator. Largest parser: 42.7 KiB; 35.7 KiB is inlined code already inside that body. |
| Content-mapper registration | 62.8 KiB body | Unknown | Share constructors for repeated registration options. Preserve registration order, capabilities and JSON. |
| LSP request wrappers | 97.7 KiB / 48 copies | 78.6 KiB combined one-copy ceiling | Move common request handling into non-generic workers. Typed parsing and responses still need adapters; measure request latency. |
| Checker-pool scheduling | 102.0 KiB / 7 copies | 86.9 KiB one-copy ceiling | Split type-independent setup from the per-file callback. Another 36.5 KiB sits in related closures; this is on the compiler path. |
| AST declaration-map visitor | 45.2 KiB body | Unknown | Share cold visit and metadata helpers. 28.1 KiB of inlining is already part of the body; preserve visit order. |
| Checker heap census | 41.6 KiB body + 3.4 KiB constants | Unknown | Gate developer census tooling behind a profiling feature. Changes availability of TSRS_HEAP_CENSUS; related helpers are extra. |
| Stable sorting | 513.9 KiB / 89 instances per family | Unknown; generic ceiling is unrealistic | Reduce comparator/type variants in cold callers. Stable equal-key order can affect diagnostics, output and Unicode normalization. |
| Go-style pdqsort | 76.6 KiB / 17 copies | 66.8 KiB one-copy ceiling | Investigate shared cold sorting workers. Do not substitute a different algorithm without reference-equivalence evidence. |
| Identical assembly bodies | 242.7 KiB estimated duplicates | 242.7 KiB whole-set ceiling | Unify naturally equivalent map or helper representations. Spread across 859 groups; overlaps other generic views and may vary by linker. |
| Debug formatting | 168.5 KiB / 473 impls | Unknown | Audit costly runtime Debug uses and dyn Debug retention. Unused derives are already eliminated; largest single impl is 3.0 KiB. |
| Destructors and drop glue | 576.0 KiB / 2,678 symbols | Unknown; 569 KiB tool ceiling is unrealistic | Look for avoidable owning wrapper variants on cold paths. Different types require different destructors; LSP lifetime cleanup must remain. |
| Panic location records and paths | 265.5 KiB | Unknown | Share genuinely repeated cold failure helpers. Preserve useful failure context; do not introduce unchecked operations for size. |
| Vtables | 74.1 KiB | Unknown | Review trait requirements that retain unused methods. Vtable bytes exclude method bodies; pointer slots elsewhere overlap this view. |
| Inlining and hashing | 4.6 MiB attributed inline code | Unknown | Use the source-line view to inspect large cold call sites. Already included in code; hash_bytes alone is 153.8 KiB across 936 sites and is hot. |
| Unwind and exception tables | 1.72 MiB | Not removable in the current behavior | Retain for LSP/API recovery and project rollback. Abort changes recovery semantics; previously investigated. |

The additional Unicode evidence comes from `js_case_generated.rs` (2,927 case-mapping records with three
string-slice fields), `norm_generated.rs` (normalization tables reached through `natural_collation_key`),
and the report's lookup-table rows for 17 `Kind::fmt` bodies (53.4 KiB in total, rounded). Compact generated
offsets are a candidate for the first two. Sharing the formatter is a candidate for the third. None has been
implemented or timed. The existing Unicode and normalization rules must remain identical.

### Packaging variants

These change the delivered capabilities or file layout. The default full-feature executable cannot take these
whole-function removals while preserving its current behavior. The graph does not see every indirect edge;
independent feature builds are required, and the figures must not be summed into a promised total.

| Variant | Retained footprint | Consequence |
| --- | ---: | --- |
| Separate compiler-only variant | 1.9 MiB LSP retained footprint | LSP entry point absent; some language-service code remains shared. |
| Variant without Node API transport | 868.4 KiB API retained footprint | Node API unavailable; shared compiler code remains. |
| Check-only variant without emit | 734.5 KiB Program::emit retained footprint | Emit unavailable; other entry points may still keep emit code. |
| Libraries next to the executable | 3.62 MiB embedded payload | Moves bytes into separate files; package may not shrink. |

### Code by crate

These are the report's 50 ranked surviving-symbol rows. Inlined instructions remain charged to their caller,
so do not add the separate inlining-by-origin totals. Sizes are rounded; the rows sum to approximately
12.112 MiB, leaving about 118 KiB of code outside the ranked rows or their rounding. Percentages below use
the actual 12.227 MiB code total rather than the report's shipped-size denominator.

| Crate | Code | Share of code |
| --- | ---: | ---: |
| `tsrs_checker` | 1.7 MiB | 13.90% |
| `core` | 1.5 MiB | 12.27% |
| `tsrs_ls` | 1.2 MiB | 9.81% |
| `tsrs_project` | 560.0 KiB | 4.47% |
| `hashbrown` | 534.0 KiB | 4.27% |
| `tsrs_compiler` | 518.3 KiB | 4.14% |
| `tsrs_ast` | 514.2 KiB | 4.11% |
| `tsrs_transformers` | 513.7 KiB | 4.10% |
| `tsrs_lsp` | 488.8 KiB | 3.90% |
| `tsrs_tsoptions` | 449.2 KiB | 3.59% |
| `tsrs_core` | 421.3 KiB | 3.36% |
| `tsrs_api` | 383.1 KiB | 3.06% |
| `std` | 293.1 KiB | 2.34% |
| `tsrs_lsproto` | 270.0 KiB | 2.16% |
| `alloc` | 249.8 KiB | 2.00% |
| `tsrs_parser` | 243.1 KiB | 1.94% |
| `regex_automata` | 227.8 KiB | 1.82% |
| `tsrs_printer` | 218.2 KiB | 1.74% |
| `tsrs_incremental` | 192.7 KiB | 1.54% |
| `tsrs_execute` | 153.7 KiB | 1.23% |
| `tsrs_declarations` | 130.5 KiB | 1.04% |
| `tsrs_module` | 130.3 KiB | 1.04% |
| `rayon_core` | 129.0 KiB | 1.03% |
| `regex_syntax` | 125.6 KiB | 1.00% |
| `tsrs_fswatch` | 117.8 KiB | 0.94% |
| `tsrs` | 114.7 KiB | 0.92% |
| `tsrs_vfs` | 92.8 KiB | 0.74% |
| `tsrs_binder` | 89.3 KiB | 0.71% |
| `tsrs_scanner` | 77.2 KiB | 0.62% |
| `aho_corasick` | 74.9 KiB | 0.60% |
| `tsrs_api_transport` | 74.7 KiB | 0.60% |
| `serde_json` | 62.1 KiB | 0.50% |
| `tsrs_projectutil` | 59.9 KiB | 0.48% |
| `tsrs_modulespecifiers` | 53.9 KiB | 0.43% |
| `indexmap` | 53.5 KiB | 0.43% |
| `tsrs_api_codec` | 49.1 KiB | 0.39% |
| `tsrs_linter` | 43.7 KiB | 0.35% |
| `gimli` | 32.3 KiB | 0.26% |
| `serde_core` | 28.1 KiB | 0.22% |
| `rayon` | 25.4 KiB | 0.20% |
| `rustc_demangle` | 16.4 KiB | 0.13% |
| `tsrs_pseudochecker` | 15.9 KiB | 0.13% |
| `tsrs_astnav` | 14.8 KiB | 0.12% |
| `tsrs_sourcemap` | 13.2 KiB | 0.11% |
| `addr2line` | 10.1 KiB | 0.08% |
| `memchr` | 8.6 KiB | 0.07% |
| `rustc_hash` | 7.4 KiB | 0.06% |
| `regex` | 5.5 KiB | 0.04% |
| `crossbeam_epoch` | 4.7 KiB | 0.04% |
| `xxhash_rust` | 4.3 KiB | 0.03% |

The interactive breakdown is in `target/size-audit/binary-size-breakdown.html`; exact section accounting is in
`footprint.json`, and the complete opportunity inventory is in `opportunities.json`. These are ignored local
artifacts; the measured figures and decisions above remain in this note.

## Cross-check against the Rust Performance Improvement Plan

On 2026-10-10, checked the [plan's binary-size section](https://github.com/Boshen/rust-performance-improvement-plan#reduce-binary-size)
against this linked report and the production source. Its source-level checks cover heavy dependencies,
generic bodies that can be shared, Unicode conversion on ASCII-only data, and repeated string lookup tables.
The mapping below adds two table leads and separates safe ASCII cleanup from Unicode behavior changes.

| Plan check | Evidence in this binary | Assessment |
| --- | --- | --- |
| Heavy dependencies | Baseline auto-import regex retained 944.8 KiB; no duplicate dependency versions | The compatible PikeVM adapter saves 420.6 KiB in the linked CLI. Unicode and regex syntax are retained. |
| Thin generic shim, shared non-generic body | Option comparisons: 101.2 KiB / 3 copies. LSP wrapper families: 97.7 KiB / 48 copies. Checker scheduling: 102.0 KiB / 7 copies | Start with the cold option and request scaffolding. The existing rejection of hot checker-predicate type erasure still applies. One-copy ceilings exclude adapters and are not linked savings. |
| Unicode conversion for ASCII-only data | Three regex scanner diagnostic calls lowercase ScriptTarget names; parser extract_name explicitly scans only ASCII letters and hyphens | These calls can use ASCII conversion without changing their input domain. Required Unicode conversion elsewhere still keeps the standard tables reachable. This is not evidence for a large table-removal win. |
| Repeated static string lookup tables | Kind formatting tables, compiler feature membership, regex property aliases, and LSP preference metadata | Compact string IDs and shared tables deserve explicit linked A/B experiments. See the source counts and current attributed footprints below. |

### Repeated string-table leads

The distinction is between repeated text bytes and repeated **records**. The compiler can already pool equal
string literals and whole equal arrays. On this 64-bit target, an `&str` record is still 16 bytes, and a pair of
string slices is 32 bytes. Compact IDs can shrink per-table membership/value records even when text is pooled.
A shared pool must include its own offsets/lengths, bounds handling and decoding/accessor code; counting source
occurrences alone overstates the linked saving.

- **Compiler feature membership:** `crates/tsrs_checker/src/utilities_types.rs:52` has 57 feature names, 107 library
  entries and 439 logical string references, using 245 unique strings (2,151 unique UTF-8 bytes). Its 107 property
  lists contain only 63 distinct lists. `build_feature_map` reaches 8.9 KiB of exclusive constants, 9.5 KiB including
  shared constants. Store feature/property/library names as compact IDs and share property-list definitions.
  Preserve feature lookup, property order, library selection and diagnostic text. The current public representation
  uses string slices, so retaining it may require decoding once at initialization; that costs heap memory and work.
  This is a small candidate, not a runtime identifier interner: the latter was rejected in `notes/mem-round2.md`.
- **Regex property aliases:** the pinned `regex-syntax 0.8.11` generator produces 271 alias/name pairs and 908
  alias/value pairs. The former references 413 distinct strings; the latter 802. These are source counts before
  array pooling. The report attributes 12.0 KiB of exclusive constants to `canonical_prop` and 21.0 KiB to
  `property_values`. Generated compact IDs could replace wide alias/value records while preserving canonical
  names, Unicode properties and binary-search ordering. This would require an upstream generator change or a
  maintained dependency patch. It is part of the existing 944.8 KiB regex footprint, not additional savings.
- **LSP preference metadata:** `FieldTag` at `crates/tsrs_ls/src/lsutil/userpreferences.rs:191` carries four string
  slices and two accessor pointers per record. `field_info_cache` reaches 15.2 KiB of exclusive constants,
  15.5 KiB including shared constants. Compact name/path records may reduce it, but the attribution also includes
  accessors and initialization data. Preserve raw/config/fallback tags, invert handling and field declaration order.
- **AST Kind formatting:** the earlier 53.4 KiB lookup-table attribution across 17 reported bodies is another match
  for this check. A single name lookup/formatter needs a linked experiment to establish whether fat LTO keeps it
  shared. Its constants overlap Debug and inlining views.

The source-count artifact is `target/size-audit/plan-string-tables.json`. None of these table representations has
been changed, and no saving is promised from the counts.

### ASCII-only calls and required Unicode paths

The four clearly bounded call sites are `crates/tsrs_scanner/src/regexp.rs:64`, `:359`, `:629`, and
`crates/tsrs_parser/src/parser_3.rs:2946`. ScriptTarget's string representation contains only ASCII, and the parser
helper's loop admits only `[A-Za-z-]`. These are safe candidates for `to_ascii_lowercase`.

Do not apply that replacement mechanically to user input. For example, `parse_indent_style` currently recognizes
`blocK` as `block` after Unicode lowercasing; ASCII lowercasing would reject it. Completion identifiers, path/glob
folding and the core range-table helpers also handle non-ASCII characters. They retain standard-library case
conversion independently of the four bounded calls. The `≤ 262.2 KiB` UPPERCASE_LUT row is the span to the next
named symbol, including unnamed constants; it is not an exact table size or a recoverable saving.

The plan also asks for a Linux/musl section-level comparison to avoid macOS page-layout effects. That is still a
measurement gap: this report is macOS arm64 without release PGO/BOLT. Repeat promising source changes on the
actual release targets; choosing another libc or build profile is not itself evidence of a source-level fix.
Fat LTO, one codegen unit and stripping are already used. The previously rejected cold-crate size profile and
the requirement for unwind recovery remain unchanged. This follow-up changes documentation only.
