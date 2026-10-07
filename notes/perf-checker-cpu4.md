# perf-checker-cpu4: user-space instructions, round 4

Follow-up to notes/perf-checker-cpu3.md. Same rules: representation and code layout only; identical diagnostics,
baselines and `--extendedDiagnostics` counters. Base: origin/main ae33f10. Goal: cuts of at least 1% of
single-threaded user-space instructions on one of vscode, t3code-server, formbricks-web, supabase-studio, cal-diy,
counting the front end (`--listFilesOnly`) as its own measurement, with no project worse by more than 0.3%.

## Profile

Two profiles of the base, single-threaded (`--singleThreaded`, `RAYON_NUM_THREADS=1`):

- samply on the Mac (`--release`, 4 kHz), vscode and t3code-server, read with a small script over the presymbolicated
  profile (self and inclusive per symbol, callers, `atos` lines).
- `perf record -e instructions:u -c 200000 --call-graph fp` on the 64-vCPU Linux runner (`dist` with frame pointers),
  vscode, t3code-server and formbricks-web, the check and `--listFilesOnly` separately; `perf report` self and
  inclusive, callers, and `tools/perf/srclines.py` lines for the top 14 symbols.

Without PEBS the instruction samples skid onto slow instructions: `assign_symbol_id` / `assign_node_id` (1.5% / 1.3%
of vscode samples) are their `lock xadd` and `lock cmpxchg`, one instruction each, and `get_type_at_flow_node`'s
largest line (22% of its samples) is the compare after the flow memo's slot load, a cache miss. Those are cycles,
not instructions, and were left alone.

What the profiles show (shares of user-space instruction samples, Linux):

| | vscode | t3code-server | formbricks-web |
| --- | --- | --- | --- |
| check: top self symbols | `SymbolMap::search` 2.9, `get_type_at_flow_node` 2.7, scanner 2.2, `SymbolMap::position` 2.0, `get_type_of_symbol` 1.4, `NameResolver::resolve` 1.4 | `instantiate_type_with_alias_worker` 5.3, `SymbolMap::search` 3.1, `position` 2.0, `get_type_of_symbol` 1.8, `get_type_at_flow_node` 1.7 | (front end below) |
| check: `SyncMap<Path, Option<SourceOutputAndProjectReference>>::load` | 1.09 | 0.55 | |
| front end: scanner (`scan` inclusive, which holds `scan_identifier`) | 19.7 | 19.7 | 21.9 |
| front end: `normalize_path` inclusive | 4.5 | 3.6 | 4.2 |
| front end: of which `has_relative_path_segment` inclusive (its memchr calls) | 2.6 | 3.0 | 3.3 |

Two themes came out of it, one pull request each:

- **The checker's questions to the program** (vscode check). `get_external_module_member`, `resolve_external_module`
  and the module-format checks ask the program about a source file for every import and many declarations. Each
  question went through `projectReferenceFileMapper::get_redirect_for_resolution`, which hashed the file's path into
  `realpath_dts_to_source` (a shared map behind a read lock) and built a copy of the file name that every checker
  caller dropped; and `resolve_external_module` looked its target file up by name, which normalizes the name into a
  new path string (lowercased where the file system ignores case) and hashes it, for every import of every
  resolution.
- **Path normalization in module resolution** (the front end). `normalize_path` is 3.6-4.5% of `--listFilesOnly`;
  most of it is `has_relative_path_segment`, which called memchr once per segment of paths of ~100 bytes with 10-20
  short segments (`node_modules/.pnpm/<pkg>@<version>/node_modules/...`). `DirFS::join`, under every `stat` and
  read, validated the name with a `find('/')` per segment as well, copied it, and built the result with `format!`.

## Changes

1. **Redirect queries without references** (`projectreferencefilemapper.rs`, `program.rs`). The builder already
   answered "no redirect" up front when the config has no project references (`has_no_references`); the mapper the
   program keeps did not. With no references the source and output maps are empty and nothing ever stores into
   `realpath_dts_to_source` (only `resolve_symlink`, which the builder skips without references), so every query
   missed after a hash and a lock. `redirect_reference` returns the reference itself; `get_redirect_for_resolution`
   builds its file-name string from it only for the callers that use the string (the file loader), and the program's
   per-file helpers call `get_redirect_parsed_command_line_for_resolution`, which builds none.
2. **The source file of a resolved module, once per checker** (`checker_08.rs`). `resolve_external_module` memoizes
   `Program::get_source_file_for_resolved_module` by the address and length of `resolved_file_name`: the resolver's
   string, which lives and stays unchanged as long as the program, and the program's file tables do not change while
   it is checked. Nothing is created or cached that the checker's results depend on beyond the program's own answer.
3. **`has_relative_path_segment` eight bytes at a time** (`tspath/path.rs`). Every segment but the first starts after
   a slash, so a relative segment ("." / ".." / empty between slashes) can start only where a slash is followed by
   '/' or '.'. The scan finds such pairs with a zero-byte test over 64-bit words (seven slash positions per word,
   the eighth byte is the next word's first), then checks each candidate exactly (`.pnpm` is one). A test checks it
   against splitting on '/' for every string of up to 11 bytes over '/', '.', 'a', bare and behind a prefix.
4. **`DirFS::join`** (`osvfs/os.rs`, `internal/internal.rs`). `valid_path` (Go's `fs.ValidPath`) is "not empty, no
   leading or trailing slash, and no relative segment", one call to the scan above (a test compares it with the
   element loop for every name of up to 9 bytes over the same alphabet); the joined path is one allocation of its
   final size instead of a copy of the name and `format!`.

## Result

`bench/count.py` (user-space instructions) on the 64-vCPU Linux runner, `--release`, single-threaded
(`--singleThreaded`, `RAYON_NUM_THREADS=1`), base ae33f10; each variant applied alone on the base and built in the
same job. The base repeated to 0.000% across two rounds; outputs byte-identical and `--extendedDiagnostics` counters
identical to the base in every cell. "check" is the normal run, "front" `--listFilesOnly`.

| project | mode | base | 1 alone | 1 + 2 (PR 1) | 3 alone | 3 + 4 (PR 2) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| vscode | check | 105.127 G | -0.60% | **-1.47%** | -0.57% | -0.63% |
| vscode | front | 20.637 G | 0.00% | 0.00% | **-1.37%** | **-1.69%** |
| t3code-server | check | 50.807 G | -0.28% | -0.65% | -0.25% | -0.31% |
| t3code-server | front | 6.941 G | 0.00% | 0.00% | **-1.04%** | **-1.43%** |
| formbricks-web | check | 52.053 G | -0.17% | -0.53% | -0.57% | -0.79% |
| formbricks-web | front | 16.116 G | 0.00% | 0.00% | **-1.38%** | **-2.07%** |
| supabase-studio | check | 63.208 G | -0.20% | -0.69% | -0.51% | -0.74% |
| supabase-studio | front | 10.667 G | 0.00% | 0.00% | **-1.87%** | **-3.20%** |
| cal-diy | check | 40.597 G | -0.13% | -0.41% | -0.61% | -0.90% |
| cal-diy | front | 8.399 G | 0.00% | 0.00% | **-2.25%** | **-3.64%** |

Changes 3 and 4 also cut the check runs: the checker normalizes paths too (`Program::get_source_file`), less so with
change 2, which skips most of those calls. Peak RSS (count.py, one run each): within 2 MiB of the base in every cell
except two front-end runs of PR 2, +15 MiB on vscode and supabase-studio, which allocate less, not more (one sized
string per join instead of two); one run, not repeated.

Wall time at 32 checkers, vscode, 64-vCPU runner, 11 interleaved runs per binary (`tools/perf/abprobe.py`, medians,
min-max in brackets); diagnostics, `--listFiles` and `--explainFiles` output identical:

| | base | PR 1 | base | PR 2 |
| --- | ---: | ---: | ---: | ---: |
| wall | 609.3 ms (593-632) | 612.6 ms (585-623) | 617.2 ms (605-630) | 617.7 ms (595-632) |
| Check time | 453 ms (439-476) | 451 ms (432-464) | 462 ms (449-470) | 457 ms (444-476) |
| peak RSS | 2709 MiB | 2710 MiB | 2702 MiB | 2711 MiB |

No change outside the run-to-run spread: at 32 checkers the 0.6-1.5 G instructions removed are spread over the
checkers and the parse pool, a few ms each, and these are cheap instructions (hashing, a read lock, short scans).

## Not done

- `assign_symbol_id` / `assign_node_id`, the flow memo slot compare: sampling skid onto atomics and a cache miss (see
  Profile); cycles, not instructions.
- The scanner (`scan`, `scan_identifier`, ~20% of the front end): its samples spread over many lines with no
  redundant step that stood out; notes/perf-parse.md already split its cold paths.
- `is_deprecated_symbol` (0.9% of vscode): the work is `get_parent_of_symbol` and cached combined node flags per
  declaration, not a repeated lookup; a shortcut would skip calls that may assign links, so out.
- `get_package_json_info` (4.4% of formbricks' front end inclusive): mostly reading and parsing package.json files.
- `get_source_file_meta_data` hashing the file's path for each module-format question: left after change 1 at about
  100-150 instructions a call; not measured.

## Measuring

On this Mac `/usr/bin/time -l` "instructions retired" includes kernel work; with other builds running it moved by
0.3-0.4% between runs of the same binary on vscode and formbricks-web (front-end runs, which stat thousands of files,
more), too coarse for a 1% gate. All numbers above are Linux `count.py` counts, which exclude the kernel and repeat
exactly; the Mac counts were used only to screen.

## Gates

Local, each PR's binary against origin/main 530787c built here: `--pretty false` output byte-identical on vscode,
t3code-server, formbricks-web, supabase-studio, cal-diy, mui-docs, webpack and xstate-main, single-threaded and at 4
and 32 checkers; `--extendedDiagnostics` counters identical single-threaded (timing rows excluded). `cargo check
--workspace` without warnings, `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_checker -p
tsrs_compiler` (and `-p tsrs_core -p tsrs_vfs` for PR 2). `pr-verify` (17 projects x 1/4/16/32 checkers, 64-vCPU runner): PR 1 (#180) 102 of 102 cells identical on a re-run;
its first run had 101, the miss being drizzle-orm at 16 checkers where main's own repetitions listed two diagnostics
in different orders. PR 2 (#181) 102 of 102. The single-threaded instruction deltas pr-verify measured match the
table above (vscode -1.470% / -0.626%).
