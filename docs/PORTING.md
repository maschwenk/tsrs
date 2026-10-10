# Porting guide: TypeScript 7 (Go) -> Rust

`tsrs` is a faithful port of the TypeScript 7 type checker (the Go implementation in
`ts-ref/tsc/internal/`, a symlink to a checkout of microsoft/TypeScript at commit
b85298b6a81f, = nightly 7.1.0-dev.20260929) to Rust. Scope: everything needed to
**type-check** a project from the command line. No emit, no language service, no
build mode, no watch mode. Declaration emit is ported only as far as its diagnostics go (`tsc --noEmit` reports
them when `declaration`/`composite` is on): crate `tsrs_declarations`; no `.d.ts` text is printed.

The port is **mechanical and faithful**: same algorithms, same function decomposition,
same order of operations, same diagnostics (code, position, message, order). Behavior
that matches the Go code is correct; "improvements" are bugs. Every Rust function should
be recognizable next to its Go original. Do not redesign.

Many agents port different files at the same time. The conventions below exist so that
independently written code fits together. Follow them exactly, even where you would
personally choose differently.

## Layout

| Go package (`ts-ref/tsc/internal/…`)                                   | Rust crate        |
| ---------------------------------------------------------------------- | ----------------- |
| core, tspath, collections, stringutil, jsnum, semver, glob, debug, json | `tsrs_core` (one module per Go package: `tsrs_core::tspath`, …; `core` itself at the crate root) |
| diagnostics, diagnosticwriter, locale                                   | `tsrs_diagnostics` |
| ast                                                                     | `tsrs_ast`        |
| scanner                                                                 | `tsrs_scanner`    |
| parser                                                                  | `tsrs_parser`     |
| binder                                                                  | `tsrs_binder`     |
| vfs (+ osvfs, cachedvfs), bundled                                       | `tsrs_vfs`        |
| module, packagejson, symlinks                                           | `tsrs_module`     |
| tsoptions                                                               | `tsrs_tsoptions`  |
| checker, evaluator                                                      | `tsrs_checker`    |
| transformers/declarations (+ the `transformers` base it uses)           | `tsrs_declarations` |
| compiler                                                                | `tsrs_compiler`   |
| execute (type-check-only subset)                                        | `tsrs_cli` (binary `tsrs`) |
| testrunner, testutil/harnessutil, testutil/baseline (subset)            | `tsrs_testrunner` (binary `tsrs-test`) |

One Rust file per Go file, same base name in snake_case (`relater.go` -> `relater.rs`,
`compileroptions.go` -> `compileroptions.rs`). Keep functions in the same order as the Go
file. Very large Go files are split into `name_1.rs`, `name_2.rs`, … by line range, all
contributing `impl` blocks to the same type.

## Naming

- Go `func (c *Checker) getTypeOfSymbol(...)` -> `impl Checker { pub(crate) fn get_type_of_symbol(&mut self, ...) }`.
  Names are the snake_case of the Go name, always. Acronym runs are one word:
  `getJSXElementType` -> `get_jsx_element_type`, `isESSymbol` -> `is_es_symbol`,
  `getTypeOfJSDoc` -> `get_type_of_jsdoc`, `URL` -> `url`, `ID`/`Id` -> `id`.
- Exported (capitalized) Go identifiers -> `pub`. Unexported -> `pub(crate)`.
- Go package-level functions -> free functions in the corresponding module.
- If snake_casing produces a Rust keyword, append `_`: `type_`, `in_`, `as_`, `ref_`, `mod_`, `match_`, `loop_`.
  (`Type()` accessors on nodes are named `type_node()`.)
- Go types keep their names (`TypeFlags`, `NodeLinks`, `InferenceContext`).
- Struct fields: snake_case of the Go field name.
- Go getter `Foo()` over private field `foo` -> Rust method `foo()`. Setter `SetFoo` -> `set_foo`.

## Memory model (the important part)

The Rust ownership migration is staged (`notes/rust-owned-arenas.md`). New persistent graph stores use
`tsrs_core::arena_owner::ArenaBuilder<T>`: an owned typed vector with four-byte local IDs and eight-byte
owner-qualified keys. Access returns a reference borrowed from the store. Keep a key across recursive mutation,
then resolve it again; never extend that reference to `'static`. Records run normal Rust destructors, and shared
sealed owners inherit thread-safety from their contents. The storage module forbids custom `unsafe`.

The checker's generic, keyed, symbol-ID and node-ID link stores use this model. `get` returns a short borrow;
`get_key` and `at` support recursion without retained pointers. The node builder and emit resolver use the core
link store's `RefCell`-checked `Ref` borrows: retain keys across callbacks, and release each guard before adding
or clearing records. Link maps and rare value-symbol tails own their heap data and drop with their record.
Lookup keys and graph edges still refer to legacy AST/symbol/type objects. Preserve Go's semantic IDs and
creation order independently of storage slots.

Program versions share processed-file containers and project-reference redirects through `Arc`. Resolvers own
their resolution hosts; cached DTS-faking hosts share redirect data without retaining the mapper that caches
them. Auto-import builders and alias resolvers retain their hosts and filesystems with strong Rust owners.
This removes manual frees for these containers, but not the `Program` root, parsed-config pointers or graph
edges inside them (`notes/rust-owned-program-data.md`).

Checkers, node-builder hosts and both compiler/project checker pools retain `Arc<ProgramData>`, independently
of the outer program's pool. Pool factories receive this owned data rather than a static program reference;
the data must not retain a pool. Program and alias-resolver file lists use shared Rust arrays. Getters borrow
those arrays or clone their owner, never fabricate a static slice. The `P<SourceFile>` entries still depend on
the legacy graph lifetime; retaining the array does not retain those referents (`notes/rust-owned-checker-inputs.md`).

Checker leases own the checker exclusively and return it on drop. External pools use
`CheckerHandle::new(PooledChecker, FnOnce(PooledChecker))`; there is no raw-pointer lease constructor. The
built-in pool's slot array is shared Rust storage retained by each lease, with no leaked array or static mutex
guard. A project `PooledChecker` owns its region directly; canceled/idle checkers remain parked because their
graph data can still be referenced by program caches (`notes/rust-owned-checker-leases.md`).

The following describes the **remaining legacy graph**, not a rule for new stores. Go objects that are referenced
by pointer, live long, reference each other cyclically and
are compared by identity — AST nodes, symbols, types, signatures, links, flow nodes,
mappers, inference contexts … — are allocated through an Oxc-backed owner and
referenced through `tsrs_core::P<T>`:

- `P<T>`: `Copy`, `Deref<Target = T>`, equality/hash/order **by identity** (like Go pointers).
  Create with `P::new(value)`. `p.get()` returns `&'static T`. It is a native non-null pointer; `Option<P<T>>` uses
  the null niche and is pointer-sized. `P::from_static(r)` accepts a true static; `P::from_arena(r)` is the unsafe
  constructor for a value kept alive by an arena owner or sidecar. Packed words store `p.to_bits()` / `p.key()`.
- Go `*T` that can be nil -> `Option<P<T>>`. Go `*T` that is never nil -> `P<T>`.
  Decide from the Go code (nil checks, `return nil`). When unsure, use `Option`.
- These legacy interfaces expose arena data as `'static`. This is the contract being removed; owner-borrowed
  references in migrated stores must not be converted to it.
- Fixed data with no destructor lives in `oxc_allocator::Allocator`. Values that need `Drop` live in heap sidecars
  owned by the same arena; sidecars are dropped in reverse allocation order before the Oxc chunks. Thread arenas
  remain process-lived. Explicit `Region` owners release all of their fixed data and sidecars together.
- Oxc does not expose the old allocator's individual recycling or stable partial rewind. `free!`, `free_slice!`,
  `new_recycled`, and parser checkpoint functions remain compatibility APIs, but individual frees/rewinds are
  no-ops and `arena_rewindable` is always false. Do not write new code that depends on block reuse or rewind.
- Emit still selects a scratch owner per file (notes/mem-emit-regions.md). Code that stores what it allocates beyond
  the file (the checker, program caches, diagnostics) escapes it (`arena::escape_scratch`; `CheckerSlot::with` does
  it for the checker), and a checker cache keyed by a node must not retain a key past owner teardown
  (`Checker::forget_scratch_keyed_caches`).
- Fields that are assigned after construction use interior mutability:
  `Cell<T>` for `Copy` data (flags, numbers, `Option<P<T>>`, `&'static [T]`, `&'static str`),
  `RefCell<T>` for growable collections (`Vec`, maps). Fields that are set at construction and
  never change are plain fields. Never hold a `RefCell` borrow across a call that might
  touch the same cell; copy out what you need first.
  Objects shared between checker threads (AST, binder symbols, …) use `OwnedCell` / `FrozenCell` instead
  (see "Threading").
- Go slices stored in long-lived objects -> `&'static [T]` built with `tsrs_core::alloc_slice(&v)` /
  `alloc_vec(v)`; wrapped in `Cell` if reassigned. A nil slice and an empty slice are both `&[]`
  unless the Go code distinguishes nil from empty, in which case use `Option<&'static [T]>`.
- Go strings stored in long-lived objects -> `&'static str` (`tsrs_core::alloc_str`). String
  literals are already `&'static str`.
- Go value structs (`core.TextRange`, small option structs) -> plain `#[derive(Clone, Copy)]` structs.
- Types with a name alias for the common pointer: none. Write `P<Node>`, `Option<P<Type>>` out.

### Threading

Parsing and binding run per file (in parallel) and finish before checking. Checking runs on N checkers
(`--checkers N`, `--singleThreaded` = 1; Go's default is 4, tsrs's default is half the available parallelism
clamped to 4..32 and to one checker per 32 checked files, and 4 in build mode: `default_checker_count`), each on
its own OS thread with a 512 MB stack (`tsrs_compiler::checkerpool`). Files are assigned to checkers by directory
locality; in the type-check pass a checker that runs out steals unstarted files from the busiest one
(notes/perf-checker-stealing.md), which is safe because output does not depend on which checker checks a file
(notes/perf-order-independence.md; `--checkerAssignment go` keeps Go's assignment and history). Each thread allocates in its own Oxc-backed arena;
`P<T>` is `Send + Sync` by decree, so the compiler does not police sharing. The rules:

- **Shared, frozen after binding**: AST nodes and node lists, `SourceFile`, binder symbols and symbol tables,
  flow nodes, parse/bind diagnostics, `TsConfigSourceFile`. Their mutable fields are written by the parser/binder
  (or config parser) that builds them and are read-only once the file is bound. Checkers only read them.
- **Checker-owned**: types, signatures, links, transient/merged/late-bound symbols and the tables a checker
  creates, synthetic nodes and diagnostics a checker creates. Only that checker touches them. Go keeps all
  checker state about shared objects in link stores keyed by the object; so do we. Go clones a symbol before
  mutating it unless it is transient (e.g. `mergeSymbol`); so do we.
- **Lazily initialized shared data** mirrors Go's synchronization: node/symbol ids are atomics, `SourceFile`
  line maps, position maps and identifier sets are `OnceLock`s, binding runs under a `std::sync::Once`
  (Go `bindOnce`), the lazily parsed JSDoc cache is guarded by `jsdoc_mu` (Go `jsdocMu`).

Cell types for shared objects (`tsrs_core::frozen`):

- `OwnedCell<T>`: `Cell` API (`get`/`set`/`replace`) for `Copy` fields of shared objects. Only the owner writes:
  the parser/binder before the file is bound, or a checker on objects it created.
- `FrozenCell<T>`: `RefCell` API (`borrow`/`borrow_mut`) without a runtime borrow flag, for collections in shared
  objects (`SymbolTable`, the lazily filled JSDoc cache, `TsConfigSourceFile`). `RefCell` cannot be used there: its flag is written by
  `borrow()`, so concurrent readers race. Same owner rule for `borrow_mut`; `borrow_mut_locked` is for a cache
  whose every access holds one lock. Never hold a `borrow_mut` guard across a call that reads the same cell.
- `Cell`/`RefCell` stay fine for checker-owned objects (types, links, checker state).

Checking the contract: build with `--features tsrs_core/checked-cells` (or a debug build) and run with
`TSRS_CHECK_SHARED=1`. The pool then binds all files, records every arena allocation made so far as shared, and
any `OwnedCell::set` / `FrozenCell::borrow_mut` on such an object panics with a backtrace pointing at the write.
Checked builds also keep an atomic borrow counter in `FrozenCell` to catch aliasing like `RefCell` does.
ThreadSanitizer is not usable here (the installed nightly's TSan runtime crashes at startup on this macOS).

### Function signatures

These rules make independently ported call sites agree:

| Go                                   | Rust parameter                          | Rust return |
| ------------------------------------ | --------------------------------------- | ----------- |
| `*T` (arena object)                  | `P<T>` / `Option<P<T>>`                 | same |
| `[]T` / `[]*T`                       | `&[T]` / `&[P<T>]`                      | `Vec<…>` when freshly built; `&'static […]` when returning a stored/cached slice |
| `string`                             | `&str`                                  | `String` when freshly built; `&'static str` when returning stored text |
| `bool`, enums, flags                 | by value                                | by value |
| `int` text position / offset         | `i32`                                   | `i32` |
| `int` count / index / length         | `usize` (`i32` if it can be negative)   | same |
| `int32`, `uint32`, `float64`, …      | `i32`, `u32`, `f64`, …                  | same |
| `func(...)` callback                 | `impl FnMut(...)` (or `&mut dyn FnMut` when recursion/object safety needs it) | — |
| `(A, B)` multiple results            | —                                       | `(A, B)` |
| `(T, bool)` "comma ok"               | —                                       | `Option<T>` |
| `error` result                       | —                                       | `Result<T, String>` unless a richer error is needed |
| `map[K]V`                            | `&FxHashMap<K, V>`                      | `FxHashMap<K, V>` |
| interface value                      | enum or `&dyn Trait` — see the owning crate | |

Method receivers: types with a big mutable state machine (`Checker`, `Parser`, `Scanner`,
`Binder`, `Relater`, `Program` while loading) take `&mut self` and keep plain (non-`Cell`)
fields. Arena objects (`Node`, `Symbol`, `Type`, …) take `&self`.

Callbacks invoked by a `&mut self` method receive the receiver back as their first
argument instead of capturing it:
Go `c.mapType(t, func(t *Type) *Type { return c.getWidenedType(t) })` ->
`self.map_type(t, |c, t| c.get_widened_type(t))` where the parameter type is
`impl FnMut(&mut Checker, P<Type>) -> P<Type>`. Apply this to every checker/parser/binder
callback, even when the closure does not need it.

### Flags and enums

- Go bit-flag types (`type TypeFlags uint32` + `iota` constants) -> `bitflags!` structs with the
  **Go constant name minus the type prefix, in CamelCase**: `TypeFlagsObject` -> `TypeFlags::Object`,
  `ast.NodeFlagsAwaitContext` -> `NodeFlags::AwaitContext`, `TypeFlagsNone` -> `TypeFlags::None` (= empty).
  - `x&F != 0` -> `x.intersects(F)`; `x&F == 0` -> `!x.intersects(F)`; `x&F == F` -> `x.contains(F)`
  - `x | y`, `x & y`, `x &^ y` -> `x | y`, `x & y`, `x & !y`; `x |= y` -> `x |= y` (through `Cell`: `c.set(c.get() | y)`)
- Go enumerations (`type Kind int16` + `iota`) -> `#[repr(…)] enum` with `#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]`,
  variants named like flags: `ast.KindBinaryExpression` -> `Kind::BinaryExpression`. Range markers
  (`KindFirstKeyword`) are associated consts.
- Go `Tristate` -> `tsrs_core::Tristate` enum (`Unknown`, `False`, `True`).

### Collections and strings

- `map[K]V` -> `rustc_hash::FxHashMap<K, V>`; sets -> `FxHashSet`. Go map iteration order is random, so ported
  code never depends on it; if Rust needs determinism where Go sorted afterwards, keep the sort.
- `collections.OrderedMap` / `OrderedSet` -> `indexmap::IndexMap` / `IndexSet` with `FxBuildHasher`
  (re-exported as `tsrs_core::collections::{OrderedMap, OrderedSet}`).
- Pointers are valid map keys directly (`FxHashMap<P<Node>, …>`).
- Strings are UTF-8 bytes in both languages. Go `s[i]` -> `s.as_bytes()[i]`; Go `for _, r := range s` -> `s.chars()`;
  positions are **byte offsets** everywhere.
- `strings.Builder` -> `String`. `fmt.Sprintf` -> `format!`. `strconv.Itoa` -> `to_string()`.
- `slices.Contains` etc. -> the std slice/iterator equivalent. Generic helpers from `core` (`core.Map`, `core.Filter`,
  `core.Some`, `core.Find`, …) exist in `tsrs_core` with snake_case names, but plain iterator chains are fine.

### Control flow

- Go `panic(...)` -> `panic!(...)`. `debug.Assert(cond, msg)` -> `debug_assert!`-free: use `assert!(cond, msg)`.
- `defer` -> restructure, or a small scope guard; keep the same effect on all return paths.
- Goroutines / `sync.WaitGroup` / work groups -> sequential code unless the owning crate documents a parallel entry
  point (file parsing and binding use rayon; the checker pool runs one OS thread per checker, see "Threading"). `sync.Once` -> `std::sync::OnceLock`. `sync.Mutex` -> `std::sync::Mutex` only if
  the data really is shared between threads, otherwise drop it. `atomic.*` -> the `std::sync::atomic` equivalent.
- `switch` with fallthrough, labeled `break`/`continue`, `goto` -> loops with labels / `match`; preserve evaluation order.
- Type switches on node kind -> `match node.kind()`.

## What not to port

Emit, transformers, printer and node builder (except what diagnostics need; the declaration transformer is
ported for its diagnostics),
language service, LSP, API/IPC, build mode (`-b`), incremental/tsbuildinfo, watch, tracing,
pprof, source maps, localization of messages (English only), JS-file/JSDoc type support is
**lower priority** but the parser must still parse `.js`/JSX files. If a function only serves an
excluded feature, leave it out. If a needed function has a branch that only serves an excluded
feature, keep the branch structure and put `unimplemented!("emit")`-style markers only where the
code is truly unreachable for type checking.

## Working rules for agents

- The Go source is the specification. Read the Go function, write the Rust function. Do not port from memory
  of the old TypeScript `checker.ts`; the Go port differs in many details.
- No `unsafe` outside `tsrs_core` and generated AST code without a very good reason.
- No lint/clippy cleanups, no doc comments restating the code, no `// ported from` banners. Keep comments that
  the Go source has when they explain *why*. Do not add `#[allow]`s; workspace lints already silence the noisy ones.
  The lints that are on (`docs/RUST.md`) are gated by `tools/lint/ratchet.py`: new code must not add findings, and
  an intended exception is `#[expect(clippy::<lint>, reason = "...")]`.
- Never leave `todo!()`/`unimplemented!()` for in-scope behavior without listing it in your final report.
- Your crate must compile (`cargo check -p <crate>`) when you finish, with zero warnings of the kinds not
  silenced workspace-wide. Use your own target dir to avoid lock contention with other agents:
  `CARGO_TARGET_DIR=target/<your-agent-name> cargo check -p <crate>`.
- Only edit files in the crate(s) you were assigned. If you need something from another crate that is missing or
  wrong, note it in your report (and, if tiny and clearly correct, add it and say so).
- Commit early and often, only your own paths: `git add crates/<your crate> && git commit -m "<crate>: <what>"`.
  If `git` reports `index.lock`, wait a second and retry. Never `git add -A`, never rebase/reset/stash, never
  touch other agents' uncommitted files.
- Report back concisely: what is done, what is not, known deviations from Go, anything other crates need.

## Reference tooling

- Go source of truth: `ts-ref/tsc/internal/` (read-only, except oracle tools below).
- Reference compiler binary (same commit): `$TSRS_WORK/bin/tsgo-ref` (use like `tsc`).
- Go toolchain: run Go with `GOTOOLCHAIN=auto` from `ts-ref/tsc` (module needs Go 1.27; it auto-downloads).
  To compare against the Go implementation you may write small oracle programs. Go `internal` packages can only be
  imported from inside that module, so put them in `ts-ref/tsc/cmd/tsrs-oracle-<name>/main.go` and keep a copy of the
  source in this repo under `tools/oracle/<name>/main.go`. Build outputs go to `$TSRS_WORK/bin/`.
- Test corpus: `ts-ref/tsc/testdata/tests/cases/{compiler,conformance}` (12.7k cases) with reference baselines in
  `ts-ref/tsc/testdata/baselines/reference/` (`*.errors.txt`, `*.types`, `*.symbols`). Lib files:
  `ts-ref/tsc/internal/bundled/libs/*.d.ts`.
- Scratch files go in `target/scratch/<agent-name>/` (git-ignored), never in the source tree.

## `tsrs_core` as landed (read before guessing names)

- Constants/statics are SCREAMING_CASE: `tspath::EXTENSION_TS`, `SUPPORTED_TS_EXTENSIONS_FLAT`, `RESOLUTION_MODE_ESM`, `EMPTY_COMPILER_OPTIONS`.
- Enum variants keep Go casing: `ScriptKind::JS`, `ModuleKind::CommonJS`, `ModuleKind::ESM`, `JsxEmit::ReactJSX`, `ScriptTarget::Latest`.
  `ResolutionMode` is an alias of `ModuleKind`.
- `CompilerOptions`: every Go field, snake_case. `[]string` -> `Option<Vec<String>>`, `*int` -> `Option<i32>`,
  `paths` -> `Option<OrderedMap<String, Vec<String>>>`.
- `tspath::Path` is `Path(pub String)` (derefs to `str`). Pure-slicing functions return `&str`; allocating ones return `String`.
- `for_each_ancestor_directory` callback returns `Option<T>` (`Some` stops). Go zero-value-or-found helpers return `Option<T>`.
- Identity-preserving helpers (`filter`, `same_map`, `concatenate`, `deduplicate`) return `Cow` (borrowed when unchanged).
- stringutil: predicates take `impl AsRune`; `Rune = i32`; Go stdlib equivalents `equal_fold`, `decode_rune`, `push_rune`, …
- `collections::{OrderedMap, OrderedSet}` = IndexMap/IndexSet (Fx) with Go-named methods via `OrderedMapExt`/`OrderedSetExt`
  (`set`, `has`, `delete`, `size`, `entry_at`).
- `LinkStore<K, V>` keyed by `P<K>`; `get`/`try_get`/`has` take `&self`, return `P<V>`.
- Go `String()` methods are `string()` + `Display`. `get_spelling_suggestion_for_strings` is at the crate root.
- json: `tsrs_core::json::Value` with `marshal*`/`unmarshal` (key order preserved).

## `P::get` shadows `T::get`

`P<T>` has an inherent `get(self) -> &'static T`. Method lookup finds it before any `get` defined on `T`, so
`p.get(key)` on a `P<SymbolTable>`/`P<Relation>`-like value does not compile. Arena types therefore do not name
methods `get`: use `lookup` (`SymbolTable::lookup(name)`), or call through the deref explicitly: `(*p).get(key)`.
Field access is unaffected (`p.field.get()` is `Cell::get`).
