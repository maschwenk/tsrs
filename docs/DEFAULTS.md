# Defaults that differ from tsgo

This page lists every default where tsrs behaves differently from tsgo built from the pinned commit (`Cargo.toml`,
`[workspace.metadata.typescript]`; currently `b85298b6a81f`). It covers the command line, watch mode, the language
server, the API, option parsing, environment variables and the npm launcher. The capability summary is in the
[compatibility table](COMPATIBILITY.md#what-it-does-and-doesnt-do); this page has the detail.

Go paths are relative to `tsc/internal/` in a microsoft/TypeScript checkout, Rust paths to `crates/`.

## What is the same

These were compared entry by entry and match tsgo:

- All 126 compiler option declarations in `tsoptions/declscompiler.go` (common and compiler-only, in the same
  order): name, short name, kind, default value description, category, description, `isCommandLineOnly`,
  `isTSConfigOnly`, `isFilePath`, `minValue`, `strictFlag`, `allowJsFlag`, `listPreserveFalsyValues`, every
  `affects*` flag, and the list element options.
- The build options (`build`, `verbose`, `dry`, `force`, `clean`, `builders`, `stopBuildOnErrors`), the watch options
  (`watchInterval`, `watchFile`, `watchDirectory`, `fallbackPolling`, `synchronousWatchDirectory`,
  `excludeDirectories`, `excludeFiles`) and the type acquisition options (`enable`, `include`, `exclude`,
  `disableFilenameBasedTypeAcquisition`), including `enable` defaulting to true only for `jsconfig.json`.
- Every enum map in `tsoptions/enummaps.go` (`lib`, `target`, `module`, `moduleResolution`, `moduleDetection`,
  `jsx`, `newLine`, `watchFile`, `watchDirectory`, `fallbackPolling`), the deprecated keys and the numeric values.
- Every computed default in `core/compileroptions.go`: emit target (`LatestStandard`), module kind, module
  resolution, module detection, package.json exports and imports, `allowJs`, strict flags, `isolatedModules`,
  `useDefineForClassFields`, declaration emit, type roots, new line and the rest.
- tsconfig parsing defaults: `include` of `**/*` when neither `files` nor `include` is set, `exclude` of `outDir` and
  `declarationDir`, the always-skipped `node_modules`, `bower_components` and `jspm_packages`, `compileOnSave` false,
  `${configDir}` substitution, `extends` merging and project reference resolution.
- The default lib for each target, and the 113 bundled lib files, byte for byte.
- `--pretty` detection (`FORCE_COLOR`, `NO_COLOR`, `TERM=dumb`, then whether stdout is a terminal), config file lookup,
  `--builders` (4, or 1 with `--singleThreaded`), the exit status values and the tsbuildinfo `version` stamp.
- Watch timing: the 50 ms / 500 ms debounce, the 2000 ms `watchInterval` default, and no polling on either side.
- Language server: user preference defaults and config sections, format code settings (indent 4, spaces, `"\n"`),
  the inferred project's compiler options, log level, server capabilities, position encoding, trigger characters, the
  semantic tokens legend, every timer (diagnostics debounce 500 ms, watcher throttle 75 ms, idle cache clean 30 s,
  checker pool idle 30 s and the rest), and the `--lsp` and `--api` flags and their defaults.

## Command line

| setting | tsgo | tsrs |
| --- | --- | --- |
| Checker count without `--checkers` | 4 | Every core up to 16, then half the cores, from 4 to 32, and no more than one checker per 32 type-checked files (never below 4). `--build` uses 4 per project. Both then cap at the file count and 256 (`compiler/checkerpool.go`, `tsrs_compiler/src/checkerpool.rs` `default_checker_count`) |
| How files are split across checkers | FENNEL association | Directory-subtree groups with work stealing, file splitting and module affinity. `--checkerAssignment go` or `TSRS_CHECKER_ASSIGNMENT=go` restores tsgo's split |
| Checker history | tsgo's: which file reports some diagnostics depends on check order | Canonical. Also turns on tsrs-only checker caches (inference, union, indexed access and flow memos). `go` assignment restores tsgo's history ([details](../notes/fix-history-dependent-diagnostics.md)) |
| Lazy checker members | no | On. `--noLazyMembers` or `TSRS_LAZY_MEMBERS=0` turns it off; never changes diagnostics ([patches](../upstream/)) |
| Memory-saving modes | none | Lazy `.d.ts` members when `skipLibCheck` or `noCheck` is set, and freeing finished leaf files on eligible `--noEmit` runs. `--maxMemory` is off by default |
| `--extendedDiagnostics` counters | Types, Symbols and Instantiations for 4 checkers | The same counters, for however many checkers ran, so they depend on the machine unless `--checkers` is given |
| Parse and other parallel work | One goroutine per task, bounded by `GOMAXPROCS` (the CPU count, respecting cgroup quotas) | A thread pool of `min(available parallelism, 32)`, or `RAYON_NUM_THREADS` |
| Incremental startup | Reads tsbuildinfo, then builds the program | Both at once on two threads, unless `--singleThreaded` |
| tsbuildinfo contents | tsgo's fields | Adds `tsrsCanonicalCheckSignature`. In `go` history mode or API mode, tsrs won't reuse a tsbuildinfo that has this field |
| `--version` | `Version 7.1.0-dev` | `Version 7.1.0-dev (tsrs <version>, microsoft/TypeScript@<commit>)` |
| `--init` | Writes a tsconfig.json | `error: --init is not supported by tsrs.`, exit status 5, no file written |
| `--showConfig` | Prints the resolved config as JSON | Not supported, exit status 5 |
| `--help`, `--all`, `-b --help` | Full option help | A version line and a two-line usage, exit status 0 |
| No tsconfig.json and no files | Version and full help, exit status 1 | Version and one usage line, exit status 1 |
| `--locale` | Translates messages | Accepted; messages stay in English |
| `--locale` validation | Checks the tag against the language registry: `--locale zz` gives TS6048 and exit status 1 | Checks only that the tag is well formed: `--locale zz` is accepted |
| `--generateTrace` | Writes a trace | Warns `Failed to start tracing` and runs without it |
| `--pprofDir` | Writes CPU and memory profiles | Parsed and ignored. `--lsp -pprofDir` prints the "will be written" line but writes nothing |
| `Memory used` in `--diagnostics` | Live heap after two garbage collections | Process resident set size |
| `Memory allocs` in `--diagnostics` | Number of allocations | Always 0 |
| `--diagnostics`, `--extendedDiagnostics` output | One table | The same table, then phase timings, filesystem call counts and lazy-member counters |
| Panic | Exit status 2 with a Go stack trace | Exit status 5. Buffered stdout that wasn't written yet is lost |
| Stdout | Unbuffered | Buffered, flushed at exit and on watch status lines, so stdout and stderr can interleave differently |
| Ctrl-C, SIGTERM | Caught; a non-incremental run finishes | Default action: the process stops at once. `--lsp` and `--api` catch them as tsgo does |
| Stack size | Go's 1 GB goroutine maximum, or `TS_GO_DEBUG_STACK_LIMIT` | Fixed: 512 MiB for the main, checker and API threads, 256 MiB for parse workers. No override |
| Concurrent filesystem calls | At most 128 blocking calls, 128 reads and 32 writes | No limit |
| Default lib location | Embedded build: the embedded libs. npm build: next to the executable, and panics without `lib.d.ts` there | `TSRS_LIB_PATH` if set, else next to the executable if `lib.d.ts` is there, else the embedded libs |
| Case sensitivity detection | At startup | On first use (same test) |
| Realpath on Linux | `O_PATH` and `/proc/self/fd`, falling back to walking symlinks | `realpath(3)` |
| Memory management | Go's garbage collector with `GOGC=100` and no memory limit; honors `GOGC`, `GOMEMLIMIT`, `GOMAXPROCS` | mimalloc with transparent huge pages on; the Go variables have no effect |
| Arguments that aren't UTF-8 | Accepted | Panic |
| tsrs-only flags with `-b` | — | The tsrs-only flags (`--checkerAssignment`, `--checkerCostCache`, `--maxMemory`, `--lazyMembers`, `--noLazyMembers`) are rejected with TS5072 |

The tsrs-only flags are described in [Performance options](USAGE.md#performance-options).

## Watch mode

| setting | tsgo | tsrs |
| --- | --- | --- |
| Linux backend | fanotify on kernel 5.13 and later, otherwise inotify | inotify (fanotify isn't ported) |
| Windows | Native backend | Not ported: `--watch` exits with "not supported" |
| When a change starts a rebuild | First event after a quiet period | Waits for the writer to close the file (inotify), or while the file is empty (FSEvents), for up to 1000 ms without writes or 2000 ms after the first write |
| FSEvents batch touching several files | Fires at once | Waits for 50 ms of quiet |
| FSEvents streams | One shared stream list, default queue | One stream per root (up to 256), `USER_INITIATED` queue |
| When to watch a parent recursively | More than 10 watches under the parent | More than 10 distinct children of the parent |
| kqueue descriptor limit | Raised by the Go runtime | Raised by tsrs, which reports when it isn't enough |
| Events that start a rebuild | Every relevant event | Skips the build's own outputs and files whose modification time didn't change; rescans files changed during the first build; watches directories reached through symlinks; rebuilds on case-only renames |
| Parsed files in `-b --watch` | Reparsed every cycle | Reused when the text is unchanged |
| Backend override | none | `TSRS_WATCH_BACKEND=kqueue` uses the macOS fallback |

The write settling above applies to `--watch` and `-b --watch`. The language server's built-in watcher behaves like
tsgo's.

## Language server

| setting | tsgo | tsrs |
| --- | --- | --- |
| Automatic type acquisition | On by default for `--lsp` (the CLI passes a typings location and npm) | Not ported |
| Telemetry | Performance and project telemetry when the client enables it | Only the flaky-diagnostics event |
| `trackFlakyDiagnostics` check | Runs a full emit (writing nothing) between the two diagnostic passes | Runs the two passes back to back, so it finds fewer flakes |
| Indent, tab and base indent size | No limit | Capped at 100 |
| Integer preferences (`tabSize`, `indentSize`, `baseIndentSize`, `maximumHoverLength`) | 64-bit | 32-bit: values beyond ±2³¹ wrap. The same applies to `tabSize` in LSP formatting options |
| `locale` from the client | Ignored unless it parses as a BCP 47 tag | Any non-empty string is accepted (messages are English either way) |
| `GOMEMLIMIT` (from `js/ts.server.goMemLimit`) | Memory limit | No effect |
| Async request threads | A goroutine per request, bounded by `GOMAXPROCS` | A worker pool that keeps `max(4, cores)` idle threads; at most `available parallelism` run at once |
| Background queue | Unbounded goroutines | 6 workers |
| After the idle cache clean | `runtime.GC()` | Nothing |
| Profiling commands, `runGC`, `custom/initializeAPISession`, `setContentMapperContributions` | Supported | `MethodNotFound` |
| Content mappers | Supported | Not ported; `runExternalCode` does nothing |
| Parent process watchdog on Windows | Supported | Not ported |

[Language server](LSP.md) has the threading model and the other known gaps.

## API

| setting | tsgo | tsrs |
| --- | --- | --- |
| `--builders` in API mode | The requested value | Always 1 |
| Content mappers | A spawner is passed to the session | None; `--runExternalCode` does nothing |
| Re-entrant callback in async mode | Blocks | Rejected after a 10 s grace period |
| `-cwd=""` | Panics | Uses the current directory |
| `-useCaseSensitiveFileNames` after a non-flag argument | Ignored (the flag parser has stopped) | Read |
| Unknown `-callbacks` name | Panics with a stack trace, exit status 2 | Error message, exit status 2 |
| Windows named pipes | Supported | Not ported |

[Node API](NODE_API.md) has the rest.

## Option parsing edge cases

- **`moduleDetection` number with no name, sent through the API.** tsgo keeps the raw value, which the parser treats
  like legacy detection. tsrs keeps the field unset and computes the default (`force` for `node16` through
  `nodenext`, otherwise `auto`), so files can become modules where tsgo leaves them scripts. `newLine` behaves the same
  on both sides.
- **Enum option given as a number with no name** (reachable through tsbuildinfo options). tsgo converts any number to
  the enum (`"module": 42` gives `ModuleKind(42)`). tsrs looks the number up in the enum map and falls back to unset.
- **`watchInterval`** is a 64-bit integer in tsgo and 32-bit in tsrs. Neither side uses it for watching.
- **`builders` from the API below 1** queues nothing in tsgo; tsrs ignores it because API mode always uses 1.
- **Content mappers:** with `contentMappers`, tsrs leaves a mapped file's own `.d<ext>.ts` output out of wildcard
  includes. tsgo includes it once it has been built, which gives TS5055 on the next run.

## Environment variables

tsrs reads the same `FORCE_COLOR`, `NO_COLOR`, `TERM`, `TS_WATCH_DEBUG` and `TS_CONTENT_MAPPER_DEBUG` as tsgo, and
`HOME` and `XDG_CACHE_HOME` for the typings cache location.

tsgo variables tsrs ignores:

- `TS_GO_DEBUG_STACK_LIMIT` (stack sizes are fixed).
- `OS`, `WT_SESSION`, `TERM_PROGRAM`, `COLORTERM` (tsgo uses them only to color `--help`).
- `TERMUX_EXEC__PROC_SELF_EXE` (Android).
- The Go runtime's `GOGC`, `GOMEMLIMIT`, `GOMAXPROCS`, `GODEBUG` and `GOTRACEBACK`.

tsrs-only variables that change behavior:

| variable | default | effect |
| --- | --- | --- |
| `TSRS_CHECKER_ASSIGNMENT` | `locality` | `go` uses tsgo's file split and checker history |
| `TSRS_MAX_MEMORY` | off | Same as `--maxMemory` |
| `TSRS_LAZY_MEMBERS` | on | Same as `--lazyMembers` / `--noLazyMembers` |
| `TSRS_LIB_PATH` | unset | Directory to load the default libs from |
| `TSRS_WATCH_BACKEND` | unset | `kqueue` uses the macOS fallback watcher |
| `TSRS_BINARY` | unset | Path of the binary the npm launcher runs |
| `RAYON_NUM_THREADS` | `min(cores, 32)` | Parse thread count |
| `MIMALLOC_*` | mimalloc's defaults | Allocator settings; `MIMALLOC_ALLOW_THP` keeps mimalloc's huge page choice |

The remaining `TSRS_*` variables tune or report on checker internals; [debugging](DEBUGGING.md) describes them.

## npm launcher

| setting | tsgo (`typescript`) | tsrs (`tsrs`) |
| --- | --- | --- |
| Command | `tsc` | `tsrs` |
| Binary package | `@typescript/<os>-<arch>` | `@tsrs/<os>-<arch>[-gnu\|-musl]`, chosen by libc |
| Launch | `execve` if available, else `execFileSync` | `execve` if available, else `spawn` |
| Child killed by a signal | Node throws and exits with a stack trace | Forwards signals to the child and re-raises the child's signal |
| Binary override | none | `TSRS_BINARY` |
