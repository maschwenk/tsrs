# perf-shared-checker: is there a read-mostly part of the checker worth sharing between threads? (2026-10-05)

A bounded probe of the last design on the table in notes/perf-round3.md ("Considered and not attempted: one checker
shared by all threads"). Plan: measure the largest read-mostly subsystem that every checker rebuilds identically,
and only if it is at least ~5% of the shared work S build the narrowest sharing of it behind a switch.

**Verdict: stop at the measurement.** On the 38k-file codebase the read-mostly candidate (lazy resolution of lib
and node_modules declarations: declared types, their members, signatures, value types and module exports) is
**3.6% of S** at 8 checkers (4.3% at 4). Adding instantiations of library generics whose type arguments are
themselves library-only (Array<string>, Promise<void>, ...) brings it to 6.5%, but those live in the generic
targets' instantiation caches, which every checker also writes with project type arguments, so they are not
read-mostly. Nothing was built; there is no A/B and no switch. The full shared checker is a no-go as a
performance project (section 4).

Binary: origin/main e01197e, cargo build --release, macOS, Apple M5 Max (18 cores), 1-minute load 5-14 (another
agent was building). The instrumentation is commit 7fedb08 on this branch, reverted by the next commit so the PR
carries only this note.

## 1. Method

S is estimated per extra checker as (checker instructions at k - at 1) / (k - 1), with the front end (measured
with --listFilesOnly, /usr/bin/time -l) subtracted. That is the work a checker repeats because another checker
has already done it.

To split that work by what it resolves, the work census (--features work-census, TSRS_WORK_CENSUS) gained spans
around the checker's lazy resolution entry points: get_type_of_symbol past its cached fast path,
get_declared_type_of_symbol, and the miss paths of resolve_structured_type_members,
get_signature_from_declaration and get_exports_of_module. Each span is keyed by:

- origin: the source file of the resolved entity's first declaration: lib (the default library), node_modules
  (the path contains /node_modules/), project (everything else, workspace packages included), none (no
  declaration, e.g. unions);
- declared or instantiated, and for instantiations whether every type argument / mapper target is library-only
  (primitives, literals, or types whose symbol or alias is declared in lib/node_modules, checked up to 3-4 levels
  into type arguments, union constituents and composite mappers; types without a symbol count as project). Mapped
  symbols are classified by their containing mapped type, deferred-type symbols by their parent.

All checker time and all types + symbols created are then attributed exclusively to the innermost such span
("outside" when none is open). Library-only work is the "library declared" and "library inst, library args"
rows. Per class, instructions = the class's share of census time x that run's checker instructions; the
deterministic cross-check is the number of types + symbols created in the class.

## 2. Census

Checker instructions in G (process minus front end), shares from the census, types + symbols created in the
class (all checkers). One plain run per configuration; census at 1 and 8 checkers repeated three times on the
38k-file codebase, class shares within +-0.3 points.

The 38k-file codebase (37,942 files, 0 errors). Front end 49.6 G. Checkers 226.1 / 325.4 / 433.3 G
at 1 / 4 / 8; peak 4.12 / 5.39 / 6.75 GiB; S per extra checker 33.1 G (1 -> 4) and 29.6 G (1 -> 8).

| class | 1 checker G | 8 checkers G | delta G | % of delta 1 -> 8 | % of delta 1 -> 4 | objects delta 1 -> 8 (%) |
| --- | --- | --- | --- | --- | --- | --- |
| library declared | 8.7 | 16.1 | 7.4 | **3.6%** | 4.3% | 187K (1.0%) |
| library instantiated, library-only arguments | 5.6 | 11.7 | 6.1 | 2.9% | 1.8% | 1.25M (7.0%) |
| library instantiated, project arguments | 30.2 | 73.8 | 43.6 | 21.0% | 20.3% | 6.34M (35.4%) |
| other (project / workspace / no declaration) | 87.2 | 222.1 | 134.9 | 65.1% | 65.2% | 8.95M (50.0%) |
| outside any lazy resolution | 94.5 | 109.7 | 15.2 | 7.4% | 8.4% | 1.18M (6.6%) |

vscode (src, 371 errors). Front end 26.6 G. Checkers 86.9 / 92.9 / 96.8 G; S per extra checker 2.0 G (1 -> 4),
1.4 G (1 -> 8): duplication is ~11% of this project's checker work, so S does not bound its wall time.

| class | % of delta 1 -> 8 | % of delta 1 -> 4 | objects delta 1 -> 8 |
| --- | --- | --- | --- |
| library declared | 9.1% (0.9 G) | 8.7% | 4.4% |
| library instantiated, library-only arguments | 7.5% (0.7 G) | 6.2% | 12.2% |
| library instantiated, project arguments | 4.1% | 4.6% | 11.4% |
| other | 56.1% | 56.2% | 53.1% |
| outside | 23.2% | 24.2% | 18.8% |

mui-docs (docs, 0 errors). Front end 13.7 G. Checkers 41.4 / 72.7 / 103.4 G; S per extra checker 10.4 G (1 -> 4),
8.9 G (1 -> 8): heavy duplication (the MUI system / sx typings every checker builds).

| class | % of delta 1 -> 8 | % of delta 1 -> 4 | objects delta 1 -> 8 |
| --- | --- | --- | --- |
| library declared | 5.1% (3.2 G) | 5.0% | 4.4% |
| library instantiated, library-only arguments | 11.2% (6.9 G) | 10.6% | 30.0% |
| library instantiated, project arguments | 37.4% | 38.4% | 29.6% |
| other | 22.5% | 28.2% | 17.2% |
| outside | 23.8% | 17.8% | 18.8% |

Where S actually is: on the 38k-file codebase the outermost "project / declared / type of symbol" spans (the
values of project declarations: the service graph and router type of notes/perf-checker-scaling.md) take 8.0 s
of census time with one checker and 27.6 s summed over eight. That, and library generics instantiated with
project types (zod and ORM schemas over project shapes, 21%), is S. Library-only work is a thin slice.

Memory. Per extra checker the peak grows 0.38 GiB (38k-file), 51 MB (vscode), 77 MB (mui-docs). The library
declared objects duplicated per extra checker are 27K / 10K / 20K types + symbols, ~2.5 / 0.9 / 1.9 MB at the
average object sizes of notes/mem-shared-base.md (types 125 B, checker symbols 63 B; estimated, not measured);
with library-only instantiations 205K / 37K / 157K objects, ~19 / 3.5 / 15 MB (5% / 7% / 19% of an extra
checker). Two binder-level structures that are byte-identical in every checker are smaller still: the printer's
export indexes are 7 MB in a checker that builds them (TSRS_HEAP_CENSUS=1, 8 checkers: 3 of 8 built them, the
others never print a type), and the link records on shared binder keys ~7 MB per extra checker
(notes/mem-shared-base.md).

This agrees with notes/mem-shared-base.md once the two measures are told apart: there 36% of the *duplicated
bytes* were built only from lib/node_modules declarations, but by where they are created most of those objects
come out of project work (unions of primitives and literals, Array/Promise/Record instantiations made during
inference and relations of project expressions). Sharing those means sharing the union, literal and
instantiation caches themselves, which is the full design, not a read-mostly subsystem.

### What a perfect share could buy (model, not measured)

Critical path per checker = S + U/N, and only a share computed concurrently by whichever checker needs it first
shortens S (a serial pre-pass puts it back on the critical path: the forked-COW lesson, which this rule forbids).
On the 38k-file codebase at 8 checkers a checker executes 54 G on average; the library declared duplicate is
1.06 G per extra checker, library-closed work 1.93 G. Removed completely and for free, that is at most ~2% / ~3.6%
of the 4.7 s check (0.09 / 0.17 s), before the cost of synchronized reads on the hottest lookup paths (4.9M lib
type-of-symbol calls at 8 checkers in the census) and per-checker overlays for the caches below. mui-docs has
the largest relative ceiling (16% of its S, ~1.4 G per extra checker, ~0.1 s of a ~1 s check).

## 3. Determinism hazards (found reading the code; none exercised, since nothing was built)

1. **Type ids are per checker.** TypeId(self.type_count) (checker_12.rs:1903): a type shared from checker A has an
   id that means a different type in checker B. Every id-keyed structure in B (relation caches, union /
   intersection / instantiation caches via the key builder: ~45 write_type(s) occurrences plus get_type_list_key)
   would alias. A shared id space fixes aliasing but makes the ids of shared types depend on which thread
   created them first.
2. **Ids decide output in two places.** compare_types falls back to the id when flags, names and payload tie
   (utilities.rs:769; union construction sorts and binary-searches with it; 41 occurrences of compare_types), which orders e.g.
   fresh vs regular literals and intrinsics in printed unions. is_deeply_nested_type counts recursion only for
   increasing ids, "an indicator of newer instantiations" (relater_1.rs:1065): with timing-dependent ids the
   recursion cutoff, and with it relation results and TS2589-style diagnostics, can change. The identity
   relation's key normalization (checker_09.rs:679) is a cache key only. Restoring order would need a
   deterministic, thread-independent id for every shared type (e.g. derived from a structural key), not a counter;
   its cost was not measured.
3. **Declared library types are not read-only after resolution.** Generic targets carry the instantiation cache
   (InterfaceType.instantiations, Checker::object_type_instantiations, ConditionalRoot.instantiations) that every
   checker writes with its own (project) type arguments; lazily computed object-flag bits (109 flags.set /
   object_flags.set sites), lazy member tables (2.1M at 8 checkers on the 38k-file codebase, 271K resolved in
   full), union property caches, resolved base constraints / apparent types are filled by whoever asks first.
4. **Cycle detection and diagnostics belong to the resolving checker**: push_type_resolution's stack, the
   instantiation depth counters and current_node are checker state; a shared lazy resolution needs cross-thread
   cycle detection that picks the same declaration to report as single-threaded code does.
5. **Everything is !Sync by construction**: every arena struct field is a Cell / RefCell / GoMap (docs/CHECKER.md,
   "Uniform mutability rule") and the arena memory a checker owns may be recycled (notes/fix-arena-recycle-uaf.md).

## 4. Blast radius of a full shared checker

Counted in crates/tsrs_checker/src at e01197e (grep / awk; counts include a few false positives, nothing was
reviewed site by site):

| what would have to become concurrent | count | how counted |
| --- | --- | --- |
| interior-mutable fields of arena structs | 297 (251 without the node builder) | awk: Cell / RefCell / GoMap / MapperCell / OnceCell / PackedMap fields inside pub struct blocks of types.rs (169, of which 69 in 25 *Links structs), relater_types.rs 21, flow_types.rs 17, inference_types.rs 13, jsx_types.rs 5, nodebuilder_types.rs 46 |
| write sites | 860 Cell .set( + 113 borrow_mut() + 109 flag sets | grep -o over *.rs |
| link-store get-or-create sites | 272 | grep _links.get( |
| Checker fields | 329: 27 link stores, 54 maps / sets / caches, 60 memoized global lookups, 26 stacks / pools, 12 counters | fields of pub struct Checker in checker.rs |
| id-dependent behaviour | 2 that change output (compare_types fallback, is_deeply_nested_type), 1 key normalization, ~45 id-keyed cache-key writers | grep for .id comparisons and key-builder writes |

That is the "rewrite of the checker's state model" notes/perf-round3.md described, now with numbers: every lazily
filled field a concurrent once-cell or a per-checker overlay, every cache sharded or contended, a global
deterministic type identity, and cross-thread cycle detection.

## 5. Recommendation

- **No-go for the narrow probe.** The read-mostly subsystem is 3.6% of S on the 38k-file codebase, under the bar;
  its wall-time ceiling is ~2% at 8 checkers if shared for free, and it cannot be shared for free (section 3).
- **No-go for the full shared checker as a performance project.** Two thirds of S on the 38k-file codebase is
  resolution of project declarations and another fifth is library generics instantiated with project types;
  sharing them is the whole design (section 4), with output-determinism risks in type ids that have no cheap
  answer. The remaining levers for S stay the ones in notes/perf-round3.md: make S smaller (algorithms, the
  checked source) rather than share it.
- If it is ever revisited, start from the "project / declared / type of symbol" spans (the service graph and
  router type), not from the libraries.

## Unverified

- Census times include the bookkeeping of nested census spans of other categories (the origin attribution does
  not subtract it); the object counts, which are exact, point the same way (1.0% / 7.0% vs 3.6% / 2.9% of the
  delta on the 38k-file codebase).
- Origin is the first declaration's file: global augmentations merged into lib symbols count as lib, workspace
  packages reached through symlinks as project. The library-only test for instantiations is shallow (3-4 levels).
- Relation work between library types is not attributed (it is part of "outside", 7.4% of the delta).
- Memory bytes are estimated from object counts and average sizes, not measured.
- One plain run per configuration (instructions vary ~1% between runs with work stealing); Linux not measured.
- The wall-time ceilings are model estimates; nothing was built, so there is no A/B, and no diagnostics or
  conformance gates were run (the PR changes only this note).

## Reproduce

```sh
git checkout 7fedb08   # the instrumentation
cargo build --release -p tsrs_cli --features work-census
TSRS_WORK_CENSUS=census.md tsrs -p <project> --noEmit --incremental false --extendedDiagnostics --pretty false --checkers 8
# "exclusive attribution to the innermost lazy resolution" and "lazy resolution by origin / instantiation / kind"
# in census.md; instructions: /usr/bin/time -l with a plain build; front end: --listFilesOnly
```

