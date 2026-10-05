# perf-round3: type-check speed and memory once Go's design is not binding (2026-10-05)

The third performance round. The owner lifted the "mirror Go function for function" rule for type-check performance:
results must stay right, the implementation may differ. This note is the index: what landed, what the design space
looks like, what was rejected and why, and what is still open. Detail lives in the notes it points to.

## The bar this round

Diagnostics identical (text, order, location) and deterministic; emitted output unchanged; the conformance baselines
identical in the harness. Counters (`--extendedDiagnostics`), internal ids and scheduling may change. Where default
behaviour departs from tsgo, `--checkerAssignment go` keeps tsgo's behaviour (`tsrs_core::compat`), and the
tsgo-baseline harnesses run that way.

## Where the time and memory are (the 38k-file codebase, 18-core Apple Silicon)

- Cold run, 8 checkers: front end ~0.9 s (config 0.1, read + parse + resolve 0.6, bind) + check + exit 0.04 s.
- One checker executes ~276 G instructions; N checkers execute about `1 + 0.09 (N - 1)` times that, because every
  checker rebuilds the shared type graphs it touches. With cores to spare this costs CPU and memory but not time:
  **a slice's critical path is `S + U/N` (shared work + its own files) whoever computes `S`**. So wall time scales
  only until `S` dominates, and `S` can only get shorter if it is made smaller (algorithms, source) or is itself
  computed in parallel (a checker shared between threads).
- Inside one checker nothing is left above 0.3% at the function level; what remains is memory latency and the
  algorithm. A census by source pattern (`--features work-census`, `TSRS_WORK_CENSUS`, notes/perf-checker-algorithms.md)
  shows where the algorithm spends it: relation to union targets 13%, structural comparison of derived generic class
  instances with their generic base (schema and repository libraries) 10-17%, the same generic call inferred again at
  another call site ~8% (on four of five corpora), conditional types distributed over large discriminated unions ~7%,
  mapped types over large key sets 4-9%, union construction 7-9%, flow analysis 4% (16-19% on vscode and webpack,
  two thirds of it re-walking).

## Landed

| PR | change | effect |
| --- | --- | --- |
| #89 | transparent huge pages for the compressed arena on Linux (lost when the arena got its own reservation) | Linux: wall -2..-6%, sys -26..-36%, faults 0.9M -> 30K. Corrects the x86 cost of pointer compression to ~+1% wall |
| #92 | diagnostics no longer depend on which checker checks what (`getUndefinedProperty` keyed by the property, not its name) | identical output for 1-16 checkers and random assignments; one conformance baseline and <0.4% of declaration files differ from tsgo by design in default mode |
| #93 | checker threads steal unstarted files from the busiest checker | wall -11..-12% at 4-8 checkers (Mac), -14..-17% (Linux); +2% instructions; only the counters vary between runs |
| #94 | checker threads take node and symbol ids in blocks | Linux, 8 checkers: wall -3.7%, peak -4.7% |
| #90, #91 | the work census as a tool; tests pinning the TS2589 budget boundary | - |

## Rejected this round (numbers in the notes)

- **Forked checker processes sharing a warm checker copy-on-write** (notes/perf-checker-processes.md). Works and is
  byte-identical; 17-28% fewer instructions and 0.2-1.9 GiB less memory; wall parity on the Mac, 4-9% slower on
  Linux; an oracle warm-up saves 33-45% of instructions but costs 10-48% wall. The model above explains why. With a
  shared work queue it balances as well as stealing with less CPU and memory, but is slower on small programs.
  Parked on `perf/checker-processes` and `perf/checker-processes-queue`.
- **A discriminant pre-test for conditional types** (#88, closed): exact only by replaying the relater's side effects,
  which leaves 0.5%; the asymptotic version moves the instantiation-budget boundary (TS2589).
- Relation to union targets and mapped types over large key sets: measured as inherent work.
- `x86-64-v2`/`v3` builds, zero-based handles (re-measured with huge pages), front-end work on Linux (6% of a run),
  mimalloc purge/commit options, pre-assigned ids for thread mode (notes/linux-x86-round.md).

## Considered and not attempted

- **One checker shared by all threads** — the only design that shortens `S`. Every lazily filled field becomes a
  concurrent once-cell and every cache contended shared state; lazy resolution needs cross-thread cycle detection
  with a deterministic answer for which declaration reports a circularity; type ids and everything ordered by them
  would depend on timing. A rewrite of the checker's state model, not a hill climb.
- **Per-file summaries** (derive each file's exported types once as declaration text for other checkers): declaration
  emit is not total, import cycles defeat the ordering it needs, and the round trip is not identity-preserving.
- **A resident daemon / watch mode**: holds several GiB per checkout; the cold-process incremental path is ~1 s.
- **Per-file check regions**: any type created while checking a body can be stored in a long-lived cache.
- **mmap for source files**: a file truncated during the run becomes SIGBUS.

## Open at the time of writing

- Flow-analysis memo, union front cache, call-inference memo: branch `perf/flow-memo`, handoff in
  notes/perf-flow-union-inference.md on that branch.
- Derived-generic variance shortcut (`TSRS_DERIVED_VARIANCE`, off by default; -14% check time and -15% peak at four
  checkers on the 38k-file codebase, zero disagreements in shadow mode after three guards, not provable): branch
  `perf/algo-derived-variance-pr`; whether it may be on by default is the owner's decision.
- Build-level speed on Linux (BOLT, huge pages for text, `panic=abort`, allocator): branch `perf/build-level`.
- Source-level fixes in the checked codebase itself (two patterns were worth 10% of its cold check).
