# perf-emit: emit speed and memory (JavaScript, declarations, source maps, `--build`)

Emit had only been checked for correctness (`.js` baselines, the monorepo oracle). This round measured it against
tsgo and made it faster. The rules were the usual ones: output byte-identical to base, diagnostics identical, and
the conformance, `.types`/`.symbols`, fourslash and tsctests gates unchanged.

Base: origin/main 9850161. Reference: `tsgo-ref`, built from the pinned ts-ref commit.

## Result

All runs `--noEmit false --incremental false --declaration --sourceMap`, with every output redirected into the
worktree's `scratch/`. Each figure is the median of 3 interleaved runs (base, head, tsgo). The machine is the
shared 18-core M-series laptop at load 20-40. "Emit" is `--extendedDiagnostics` "Emit time"; peak is maximum RSS.

| corpus | wall base / head / tsgo (s) | emit base / head / tsgo (s) | peak base / head / tsgo (GiB) |
| --- | --- | --- | --- |
| the 38k-file codebase (82k output files) | 15.22 / **10.92** / 58.77 | 6.77 / **3.70** / 31.83 | 7.10 / 6.99 / 32.53 |
| vscode `src` (28k output files) | 6.15 / **4.37** / 12.83 | 3.25 / **1.43** / 6.92 | 3.14 / 3.06 / 11.37 |
| mui-docs (`--rootDir .`, 40k output files) | 6.06 / **4.85** / 13.16 | 4.07 / **2.86** / 8.13 | 1.39 / 1.40 / 7.41 |
| webpack (JS + maps only, see below) | 0.92 / 0.70 / 1.19 | 0.47 / 0.33 / 0.46 | 0.45 / 0.48 / 1.32 |
| xstate | 0.18 / 0.10 / 0.25 | 0.10 / 0.04 / 0.11 | 0.17 / 0.18 / 0.43 |
| `-b`, 101 monorepo packages, cold | 2.20 / 1.95 / 9.54 | aggregate 3.23 / 3.09 / 3.64 | 2.85 / 2.82 / 3.05 |
| `-b`, same, no-change warm | 0.23 / 0.23 / 0.66 | — | 0.42 / 0.43 / 0.83 |

Instructions retired, which do not depend on machine load: the 38k-file codebase 724.4 G → 618.5 G (-14.6%),
vscode 308.4 G → 253.6 G (-17.8%), mui 209.4 G → 204.0 G.

`-b` with `--builders` (head, cold, median of 3): 1 builder 5.36 s, 4 (the default) 1.95 s, 8 builders 1.80 s.

Head output matched base byte for byte on every corpus, with the same diagnostics. Compared with tsgo:

- vscode, mui and `-b` are identical with `TSRS_CHECKER_ASSIGNMENT=go` (the known property-order caveat, EMIT.md
  section 10). `-b` was compared on all 10,136 written files.
- On the 38k-file codebase, `TSRS_CHECKER_ASSIGNMENT=go` leaves 3 different files. All three come from
  `src/apiServer.test.ts`, which the `perf/round2` agent was editing between runs (`__tsrsRound2Leaf`).

## Where emit time went (base, 38k-file codebase, samply)

There were 26.7 CPU-s on the 4 checker threads, which emit ran on:

- 9.4 s writing files: `open` alone took 6.1 CPU-s.
- 12.9 s in transforms. Of that, 4.8 s was module-specifier generation for declarations, and the specifier path
  code spent most of its time in `compare_paths`, `has_relative_path_segment`, case-insensitive compares and
  `combine_paths` allocations.
- 4.0 s printing.

The checker threads were also unbalanced: they ran 5.4 to 7.9 CPU-s each.

File creation on this machine is mostly serialized by APFS. A Python microbenchmark creating 20k 3 KB files ran at
57-106 µs per file, with 1, 2, 4, 8 or 16 threads. So 82k output files set a floor of roughly 4-5 s of wall time
for any compiler.

## What was fixed (one commit each)

1. **`--extendedDiagnostics` emit rows.** These show JS transform, declaration transform, print, source-map
   serialization and write time, each summed over the threads that ran it. They appear after Go's table, like the
   other tsrs-only rows.
2. **Print and write on the worker pool.** Go prints and writes each file while it still holds the file's checker.
   The printer does not use the checker: a printer call into the resolver would panic, because the checker slot is
   empty outside `lend`. So `emitter.emit` is now split into `transform` (on the checker thread) and `print` (on
   the rayon worker pool). Each file keeps Go's order of observable steps: print and write the JS file, add the
   declaration diagnostics, then print and write the declaration file. With `--singleThreaded` or an external
   checker pool, the print step runs inline.
   - Unlimited concurrent writes made things worse: the write rows summed to 125 s over 18 threads, and the
     transforming threads slowed down from kernel contention.
   - So writes are capped at 4 at a time (Go caps OS writes at 32 with `writeSema`). Emit on the 38k-file
     codebase with 1 / 2 / 4 / unlimited writers took 6.4 / 4.3 / 4.5 / 7.0 s.
   - vscode emit went from 3.69 s to 2.08 s.
3. **tspath.**
   - `has_relative_path_segment` scans slash to slash with memchr.
   - `compare_strings_case_insensitive` has an ASCII fast path. It falls back at the first non-ASCII byte,
     because U+212A lowercases to `k`.
   - `combine_paths` starts from the last absolute path instead of normalizing the current directory first.
   - These were checked against the old implementations on 3M random strings (backslashes, drive letters, URLs,
     non-ASCII), using a throwaway test that is not committed.
   - The 38k-file codebase went from 730.3 G to 706.9 G instructions.
4. **Memoized `try_get_module_name_from_exports`.** The result depends only on the target file, the package
   directory and name, the conditions and the program. The program keeps a mutex-guarded memo, used only when the
   caller passes the program's own options object (the `ModuleSpecifierGenerationHost::exports_module_name_cache`
   hook; other hosts return `None`).
   - Without it, every file that printed a zod or vitest type walked the package's exports map again.
   - The 38k-file codebase went from 706.9 G to 665.6 G instructions. The declaration-transform sum went from
     12.7 s to 8.1 s.
5. **Subtree-facts cache.** Go caches `SubtreeFacts` in every composite node. The port recomputes them, because it
   has no field for them. Every transformer asks at every node, so recomputing is quadratic in depth: 30% of
   vscode's transform CPU.
   - Emit now runs each file's transforms under `with_subtree_facts_cache`, a thread-local side table. This is
     Go's cache, scoped to one file. Nothing changes outside emit.
   - vscode went from 309.8 G to 285.1 G instructions.
6. **`NodeVisitor` is one `Rc`.** `Transformer::visitor()` clones the visitor for each `visit_each_child`, and that
   clone copied 14 `Rc`s. vscode went from 285.1 G to 268.6 G instructions.
7. **One walk in `markLinkedReferences`.** It looked up its five ancestors through `find_many_ancestors`, which
   allocated a `Vec` and made 5 dyn calls per ancestor. The five kinds are disjoint, so one `match` walk finds the
   same nodes. vscode went from 268.0 G to 262.1 G instructions.
8. **Faster source-map columns.** When a position moves back on the same line and is nearer the cached position
   than the line start, the column is counted back from the cache. `utf16_len` now calls `is_ascii` first. vscode
   went from 261.8 G to 253.1 G instructions.
9. **`EmitContext::reset` drops its tables.** Go pools contexts. tsrs allocates one per file in the arena and only
   cleared the maps, so their capacity stayed alive. vscode peak went from 3.23 to 3.05 GiB.

The incremental declaration-signature pass (`EmitOnlyBuilderSignature`, notes/perf-dev-loop2.md) goes through the
same `Program::emit`, so items 2 and 4-6 also apply to it. They were not measured separately.

## Gates

Base is 9850161 (worktree `perf-emit-base`); head is this branch.

- Conformance with `--baselines types,symbols`, default and with `TSRS_LAZY_MEMBERS=0`: 13,458 pass, 2 codes,
  2 fail; 12,779 `.types` and `.symbols`. The result trees are identical to base in both modes.
- `--baselines js,jsmap,sourcemap`: 13,392 / 149 / 156 pass, 0 fail, identical trees. This was also run with
  `TS_TEST_PROGRAM_SINGLE_THREADED=false`, so the threaded print path runs: identical trees there too.
- Fourslash: 4,066 pass, 63 fail, with the same pass list as base.
- `cargo test -p tsrs_cli`, with `api::memory_tests` cfg'd out locally (glibc `malloc_trim`; not committed): green.
  - tsctests, using the dump in `wt/dev-loop`: 374 / 32 / 1, the same lists as base.
  - Of the result files, only `internal-symbolname-in-tsbuildInfo` differs. It is in fail.txt on both sides,
    because its `@iterator@<symbol id>` names depend on run order.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`: clean.
- Monorepo emit oracle: `tools/oracle/emit/monorepo.sh` with the git guard removed, because the local copy is not
  a git checkout; a `find -newer` check replaces it.
  - It ran over 101 packages with the configs' own options, with base and with head: 101/101 packages and 11,650
    files identical to tsgo both times.
  - The base and head output trees are identical to each other.

## What remains

- **The 38k-file codebase is now write-bound.** The write row sums to 14 s inside the 4 permits, against 3.7 s of
  emit. The checker threads finish their transforms in 2.2-3.5 CPU-s.
  - Further transform CPU cuts will not show in wall time on this machine.
  - On Linux, where file creation runs in parallel, they would. The writer cap may also want to be higher there;
    it was not measured.
- **Checker-thread imbalance.** In emit, the threads ran 2.2 / 2.8 / 3.4 / 4.1 CPU-s. A file has to be transformed
  by the checker that checked it: printed inferred types depend on which checker saw a file first.
- **Next CPU candidates on the critical checker thread**, each about 3% or less of emit:
  - `get_local_module_specifier`: relative-path computation and `paths` matching.
  - `get_nearest_ancestor_directory_with_package_json`: a memo would save about 0.1 s.
  - `get_accessible_symbol_chain_from_symbol_table`.
- **webpack with `--declaration`** (JS input, `checkJs`):
  - tsrs aborts after 32 s: the compressed-pointer arena range is exhausted at 32 GiB.
  - tsgo takes 163 s and peaks at 52.7 GiB.
  - The node builder serializes huge inferred types, and none of it is freed. A per-file emit region would need
    the checker's allocations routed elsewhere; not attempted. The `plain-ptrs` build lifts the limit.
  - The table above uses JS and source maps only for webpack.
- **`-b` never frees finished programs** (EMIT.md 10b).
- **tsgo-ref nondeterminism.** On mui-docs, tsgo-ref left out the outputs of 1-6 source files (3-18 output files)
  in 5 of 7 runs, once with `--singleThreaded`. tsrs always wrote all 39,595, and matches tsgo's complete runs byte
  for byte.

## How to reproduce

- **Scripts.** They live in `scratch/`, which is not committed.
  - `run1.sh <bin> <corpus> <tag>` adds `--noEmit false --incremental false --outDir/--declarationDir/
    --tsBuildInfoFile` pointing into `scratch/out/<tag>`, plus `--declaration --sourceMap` (`DECL=0` drops
    declarations).
  - It runs the compiler under `/usr/bin/time -l sandbox-exec` with a profile that denies writes to the corpus
    checkouts.
- **mui-docs is run from an APFS clone (`cp -cR`) with `--rootDir .`, for two reasons.**
  - TypeScript 6+ defaults `rootDir` to the tsconfig directory. The `paths`-mapped files under `packages/` are
    outside it, and such files are emitted next to their sources even with `--outDir`. Both compilers do this.
  - A first run without the sandbox did exactly that in the bench checkout. 13,140 files were written next to the
    sources, and the tracked `prism.d.mts` was overwritten. The coordinator was told; the cleanup was left to the
    owner.
- **`-b` runs on an APFS clone of the monorepo.** The cold run deletes the clone's `*.tsbuildinfo` first. The
  command is `tsrs -b <101 tsconfigs> --extendedDiagnostics`.
  - 93 of the 101 packages export from `dist/`, and nothing orders the builds, so a project can read a dependency's
    `.d.ts` while it is being rewritten.
  - Errors were the same 9 in every run once all `dist/` outputs existed.
- **Profiles:** `samply record -r 3000 --unstable-presymbolicate -s`, read with the JSON reader from
  `wt/dev-loop/scratch/sprof.py`.
