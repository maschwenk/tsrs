# The generic AST accessors through offset tables (`name()`, `expression()`, `modifiers()`, ...)

Question (from notes/perf-icount-round4.md, "Found on the way, not pursued"): the generated AST accessors are
per-kind switches, like the type accessors were before the object, union and intersection data structs went
`repr(C)` with the parent first; that note put `Node::modifiers`, `name` and `expression` at 0.4-0.7 G each
single-threaded. Do they cost that, and what does a fixed-offset form give? Base 5d3a17de (origin/main).

Answer: they cost 3.6% of single-threaded instructions on vscode together. A per-data-tag table of field offsets
(no layout change) takes 1.5-3.1% off every bench project measured, with peak memory unchanged. It clears the bar.

## 1. What the accessors cost

`Node::name()`, `modifiers()` and the base accessors (`declaration_data()` behind `symbol()`,
`locals_container_data()` behind `locals()`, `flow_node_base()` behind `flow_node()`, ...) were generated as a
`match self.data_tag()` with one arm per data struct; `expression()`, `type_node()` and `initializer()` are
hand-written in ast.rs as a `match self.kind()` whose arms call `as_<struct>()`, which asserts the data tag again.
None was inlined: on arm64 `name()` is a call, a bounds check, a jump-table load and an indirect branch, then the
field load (about 15 instructions); `expression()` adds a stack frame and a data-tag compare per arm (22).

Exact counts, vscode single-threaded (`--singleThreaded`, base build; instructions from a DynamoRIO block-count
profile, section 4; calls from a counting build with a per-accessor, per-kind counter, not committed):

| accessor | calls | instructions in the accessor | per call | most calls on |
| --- | ---: | ---: | ---: | --- |
| `expression()` | 32.8 M | 0.722 G (0.94%) | 22.0 | PropertyAccessExpression 53%, CallExpression 39% |
| `name()` | 37.8 M | 0.566 G (0.74%) | 15.0 | PropertyAccessExpression 34%, PropertyAssignment 22%, Parameter 9% |
| `modifiers()` | 19.9 M | 0.344 G (0.45%) | 17.3 | MethodDeclaration 15%, ArrowFunction 12%, PropertySignature 11% |
| `declaration_data()` (`symbol()`) | 19.8 M | 0.296 G (0.39%) | 15.0 | VariableDeclaration 22%, PropertyAssignment 12%, ImportSpecifier 12% |
| `initializer()` | 12.2 M | 0.248 G (0.32%) | 20.4 | Parameter 44% (a rare-tail field), VariableDeclaration 34% |
| `type_node()` | 8.8 M | 0.216 G (0.28%) | 24.6 | Parameter 22%, PropertyAssignment 21%, VariableDeclaration 17% |
| `locals_container_data()` (`locals()`) | 15.0 M | 0.204 G (0.27%) | 13.6 | Block 30%, Identifier 18% (no locals) |
| `flow_node_base()` (`flow_node()`) | 9.8 M | 0.148 G (0.19%) | 15.0 | PropertyAccessExpression 40%, ExpressionStatement 21% |
| together | | 2.744 G (3.58%) | | |

`function_like_data()` (7.4 M calls) and `body_data()` (6.9 M) were already inlined by LLVM at their hot call sites
(1 M and 20 M instructions left out of line); `exportable_data()` is 0.8 M calls. Fast paths that already existed:
`Node::text()` inlines the identifier case (30.8 M of its 33.9 M calls), and `flow_node()` /
`has_flow_node_data()` test for an identifier before the switch. In a samply profile (time, not instructions) the
eight accessors were 3.3% of the samples.

## 2. Layout: an offset table, not a fixed-offset struct prefix

The type-side fix put the parent first in three `repr(C)` structs that all have it, so `Type::target` reads one
offset with no test. The AST case is different on both counts:

- No accessor's field is in every data struct, so a fixed offset still needs a "has the field" test per data tag:
  a table or bitmask load, the same cost as a table of offsets.
- One offset per field across ~190 data structs needs a canonical `repr(C)` prefix (say modifiers, name, type,
  expression, initializer). The hottest structs have different subsets: PropertyAccessExpression has expression
  and name, CallExpression expression only, VariableDeclaration name, type and initializer, Parameter modifiers,
  name, type and a rare initializer. Every struct that has a later field but not an earlier one carries a 4-byte
  hole, on millions of nodes (vscode has 17.9 M), which is what the pinned sizes (docs/RUST.md rule 3) and the
  memory rounds fought for.

So the layout stays as it is and each accessor gets a table: `static SLOTS: [u16; 256]`, indexed by the 8-bit data
tag that is already in the header word, holding the field's offset from the node header (0: no such field). The
read is: extract the tag, load the table entry, test for 0, load the field; inlined, that is four instructions at
the call site (the table address is hoisted out of loops). Pieces (crates/tsrs_ast/src/ast.rs):

- `slot_table!(V; Struct.path.to.field, ...)` builds the table at compile time from `offset_of!` (nested paths for
  promoted fields, e.g. `ClassDeclaration.class_like_base.name`), so it follows whatever field order rustc picks
  and both pointer widths (`plain-ptrs` builds). Each entry's field type is checked against `V` at compile time
  through an `unsafe` marker trait `Slot<V>`: `P<T>`, `Option<P<T>>`, either in an `OwnedCell`, read as
  `Option<P<T>>` (same layout: a non-zero handle with the zero niche; `OwnedCell`/`Cell` are `repr(transparent)`),
  and a base struct as itself.
- `Node::slot` / `Node::base_at` do the read; their safety argument is the one `payload()` already relies on (the
  data tag names the struct the node was allocated with).
- Rare-tail fields (`ParameterDeclaration`, `BindingElement` and `PropertySignatureDeclaration` initializers) are
  entries with the high bit set (`rare_slot!`), offset taken in `NodeAllocRare`; they read as `None` unless the
  header's rare bit is set.
- The generator (tools/gen-ast/gen-ast.ts) emits `name()`, `modifiers()` and the nine base accessors this way and
  refuses a field that is rare or not a node or modifier-list handle. `expression()`, `type_node()` and
  `initializer()` stay hand-written: their tables list only data structs all of whose kinds Go reads that field from
  (`ForInOrOfStatement` serves both for-in and for-of; `CaseOrDefaultClause` is left out because Go's `Expression()`
  panics on a default clause), and a 0 entry falls to the old kind match in a `#[cold]` function, so every kind,
  including the panics, behaves as before. `type_node()`'s table also holds the function-like structs'
  `function_like_base.type_`, which was the match's fallback.

No struct changes size: `Node` stays 24 bytes and no pinned size moves. The tables are 11 x 512 bytes of static
data. `__text` grows by 78 KB on macOS (+0.58%) and 84 KB on Linux (+0.63%) from inlining the reads at their call
sites.

## 3. Numbers

### Exact single-threaded instructions (the decision instrument)

User-space instructions counted by DynamoRIO (the round-4 `icount` client from branch `perf/icount-round4`,
`tools/perf/dynamorio`), Linux AArch64 in an OrbStack container on this Mac, `--release` builds of base and branch
from the same target directory, `-p <project> --noEmit --incremental false --pretty false --singleThreaded`,
`RAYON_NUM_THREADS=1`. Repeats: Compiler 1,714,345,653 / 1,714,369,964, webpack 12,749,737,397 / 12,750,245,006
(under 0.005%).

| project | base | branch | change | tables out of line |
| --- | ---: | ---: | ---: | ---: |
| vscode | 76.692 G | 74.320 G | -3.09% | 75.250 G (-1.88%) |
| webpack | 12.750 G | 12.472 G | -2.18% | 12.584 G (-1.30%) |
| xstate-main | 5.523 G | 5.440 G | -1.50% | |
| Compiler | 1.714 G | 1.662 G | -3.05% | 1.687 G (-1.61%) |
| Compiler-Unions | 3.239 G | 3.191 G | -1.50% | |
| drizzle-orm | 10.819 G | 10.609 G | -1.94% | |

"Tables out of line" is the same change with the table accessors `#[inline(never)]` (text -4 KB): the table
itself gives about 60% of the gain, inlining the four-instruction read the rest. On webpack the eight accessors
held 333 M instructions out of line in base (2.65% of the main module's); in the branch 3 M are left out of line
(`initializer()` at a few call sites) and the main module runs 277 M fewer (-2.2%), so the inlined reads cost about
55 M in their callers.

### macOS, `/usr/bin/time -l` (instructions retired include the kernel)

Same machine, release builds (no debug info), base and branch interleaved, three runs each, single-threaded as
above; load average 30-55 on 18 cores (about ten other agents building) during these runs.

| project | instructions base -> branch (median) | change | min -> min | max RSS base -> branch (MiB) | peak footprint (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 83.87 -> 81.80 G | -2.5% | -2.0% | 1611.1 -> 1611.2 | 1598.5 -> 1598.5 |
| webpack | 14.27 -> 13.93 G | -2.3% | -1.9% | 439.8 -> 440.0 | 425.8 -> 426.4 |
| xstate-main | 6.58 -> 6.52 G | -1.0% | -0.3% | 192.6 -> 192.7 | 179.2 -> 179.2 |
| Compiler | 1.825 -> 1.777 G | -2.6% | -2.7% | 57.2 -> 57.4 | 46.4 -> 46.4 |
| Compiler-Unions | 3.363 -> 3.322 G | -1.2% | -1.3% | 59.9 -> 60.0 | 49.0 -> 49.1 |
| drizzle-orm | 13.26 -> 13.16 G | -0.7% | -1.4% | 392.3 -> 392.4 | 381.2 -> 381.2 |

Under this load the Mac count is not the 0.1% instrument earlier notes describe: eight runs of the base binary
spread by 0.8% (Compiler-Unions) to 8.4% (xstate-main), 3.6% on vscode, moving with system time and context
switches (drizzle-orm: 0.33 s sys and 7 voluntary switches in one run, 0.61 s and 4,045 in the next).
`thread_selfcounts` counts the kernel too (checked with a syscall loop), and the stock CPU Counters template
(`xctrace`, no sudo needed) records bottleneck metrics, not a user-mode instruction total. Hence the DynamoRIO
counts above. Peak memory: unchanged; the 0.1-0.2 MiB of RSS is the larger text.

### Default checker count, vscode (macOS)

Three interleaved runs each, load average 49-53 on 18 cores: instructions 95.30 / 95.74 / 96.04 G -> 92.78 / 93.10 /
93.19 G (median -2.8%); max RSS 2078-2092 -> 2085-2089 MiB (stealing moves it between runs); wall 4.73 / 4.92 /
6.61 s -> 3.90 / 5.81 / 6.06 s, which at this load is noise, so no wall claim.

## 4. How it was measured

- Profile first: samply on a `--release` build with `debug = 1`, `strip = false` and `-C force-frame-pointers=yes`
  (not committed), vscode single-threaded, leaf samples attributed with inline frames from samply's symbol sidecar.
- Calls per accessor and per kind: a throwaway build with a relaxed atomic counter per (accessor, kind), dumped at
  exit.
- Exact counts and the per-accessor instruction profile: DynamoRIO 11.91 (AArch64) with the round-4 `icount` and
  `bbcount` clients in a `rust:1.99.0` container, binaries built there with `CARGO_PROFILE_RELEASE_STRIP=none
  CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` (no codegen change), `bbagg.py` for the per-function totals.

## 5. Gates

- Diagnostics byte-identical (`--pretty false`) on all six projects single-threaded (macOS and Linux) and on vscode
  at the default checker count.
- `--extendedDiagnostics` single-threaded: every counter line identical on all six projects (Files, Lines,
  Identifiers, Symbols, Types, Instantiations and the lazy-table counters). No allocation changes, so the lazy-list
  chunk-boundary effect of notes/dod-ast-tables.md section 4 does not come up: Symbols matches exactly.
- Conformance (`.github/scripts/conformance-gate.sh`): pass lists identical to base in check history (13,458;
  `.types` and `.symbols` 12,779) and in default mode (13,458; 12,778, `objectLiteralNormalization` as on main).
- `tools/regressions.sh`: 36/36. `cargo test -p tsrs_ast -p tsrs_parser -p tsrs_binder -p tsrs_checker`: pass.
- `cargo clippy --workspace --locked --exclude tsrs_fourslash` with warnings denied and `-D dead-code`: clean.
  `tools/lint/source.py`: ok (its "1 fewer than recorded" notice is in `alloc_profile.rs` and is there on main
  too). `tools/gen-check.sh`: passes on the committed tree. `cargo check -p tsrs_ast --features
  tsrs_core/plain-ptrs`: ok.

## 6. Verdict

Clears the bar: -1.5% to -3.1% of exact single-threaded instructions on all six bench projects measured (1% needed
on one), no project worse, peak memory unchanged, output identical. It adds no thread, cache or invariant; the
tables are derived from the struct layouts at compile time.

## 7. What remains

- `Node::text()`: LLVM keeps three out-of-line copies of the inlined identifier path, 1.21 G (1.6%) on vscode
  in base, the largest AST item left in the profile; the cost is the identifier's packed text decoding, not
  dispatch.
- `for_each_child` dispatch (0.25% of samples on vscode) needs per-struct code, not a field read.
- The other hand-written kind matches (`member_list`, `statement_list`, `argument_list`, `type_argument_list`,
  `type_parameter_list`, `property_name`, `tag_name`) are under 1 M calls each on vscode; `set_modifiers_data`
  stays a match (no calls in a check).
