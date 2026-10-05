# fix-arena-recycle-uaf: the `panic = "abort"` crash in the arena free list

notes/perf-build-level.md found that every `panic = "abort"` build of tsrs segfaults in `tsrs_core::arena`
`pop_free`. This note has the root cause, the fix and the guards.

## In plain words

`get_tail_recursion_root` makes a type-argument list (an arena slice) and a mapper for each step of a
tail-recursive conditional type; `getConditionalType` gives both back when it returns, through
`recycle_mapper_with_targets(m, targets)`. That function received the list as `targets: &'static [P<Type>]`, a
reference parameter, and freed it. Freeing writes the free-list link into the block's first word.

A reference parameter is more than a pointer to Rust and LLVM: for the whole call it promises that the memory does
not change (rustc marks a `&T` parameter of a type without interior mutability `noalias readonly`; Stacked and Tree
Borrows call it a protected borrow). So the link write was undefined behaviour, and LLVM was entitled to delete it.
In the `panic = "abort"` builds it did. The block went onto the free list, as the head, but its first word still
held the list's contents (two 32-bit type handles). The next allocation of that size took the block and made
those handles the new head; the one after that dereferenced them and crashed.

So this is (b) in the task's terms: undefined behaviour in how the arena code was used, not a use-after-free that
exists at run time. Nothing used the list after the free. In the shipped (unwind) build the write survived: a
missing link would leave type handles as the free-list head, which are never valid addresses, so the first reuse of
that size class would crash, and no unwind build has crashed or changed output (conformance, fourslash, the
38k-file codebase, also with `TSRS_ARENA_POISON=1`). It survived by luck of code generation: the same program shape
loses the write with either panic strategy when it is small enough (below), so any inlining change could have
turned it into a crash, or into silent sharing of one block by two live lists, in a released binary.

## How it was found

1. Smallest reproduction: `compiler/tailRecursiveConditionalTypes` was one of 44 conformance tests that crash under
   `panic = "abort"` (plain `--release`, no PGO). Bisected by hand to three lines, single-threaded:

   ```ts
   type G<S, A> = S extends `${infer C}${infer R}` ? G<R, C | A> : A;
   type T20 = G<"A", never>;
   type R<T, A extends unknown[]> = T extends [infer H, ...infer Tl] ? R<Tl, A> : A;
   ```

2. The crash showed the 8-byte class's free-list head holding `0x003eddda_001fec37`: two type handles, i.e. the
   contents of a 2-element type list, not a pointer.
3. gdb with hardware watchpoints on that class's free-list head and on the block's first word (the arena sits at the
   same addresses on every run): the block was filled by `alloc_slice_recycled` in `get_tail_recursion_root`, then
   pushed by `push_free` from `recycle_mapper_with_targets` in `getConditionalType`. The head changed to the block,
   but **the block's first word was never written**: the link store was gone. The next pop of that class made the
   handles the head.
4. Instrumenting `free_block` (a double-free set) made the crash disappear: any extra call there changes what LLVM
   can prove, which is the signature of an optimisation acting on UB rather than a run-time misuse.
5. A 26-line program with the same shape (a `#[inline(never)]` function that takes `&'static [u32]` and writes the
   link through a pointer made with `with_exposed_provenance_mut`) loses the write under rustc 1.99 `-O`, with
   `panic=unwind` and with `panic=abort` (prints the slice contents as the link); taking `*const [u32]` instead keeps
   it (prints the old head).
6. Miri, on a negative-control test that frees a `&'static [u64]` parameter inside the callee: "not granting access
   to tag <wildcard> because that would remove [SharedReadOnly for <..>] which is strongly protected".

The notes on the recycling work had met this class once already: notes/mem-recycle.md says the first `free!` took a
`P<T>` and "in a debug build the write of the free-list link through it was optimized away", which is why the
macros pass addresses. The protector of the *enclosing* function's parameter is the same problem one frame up.

## The fix

- `recycle_mapper_with_targets(m, targets: *const [P<Type>])`: the list is a raw slice, which carries no
  protector; callers pass the `&'static [P<Type>]` they hold (it coerces).
- `tsrs_core::free_slice_ptr(*const [T])`: frees a slice passed that way.
- `tsrs_core::free_raw` documents the rule: a function that frees a block must not have received it as a reference
  parameter.

Every other free site was checked: the other `free_slice!` calls free locals (`get_conditional_type_instantiation_ex`'s
list, `InferenceContext::recycle`'s info slice, the backreference mapper's sources), and `free!` takes `P<T>`, which
is a 32-bit handle (not a reference) in the shipped compressed build. In `plain-ptrs` builds (`P<T>` is a `&'static
T` there) `recycle_mapper(m: P<TypeMapper>)` and friends free a parameter that is a reference; every type freed that
way has interior mutability (`TypeMapper`'s `Cell` word, `InferenceInfo`, `LazyVec` cells, flow labels), so rustc
emits no `noalias readonly` for it and LLVM cannot drop the write, but Stacked Borrows still objects to the
non-`Cell` bytes. Left as is: `plain-ptrs` is a comparison build.

## Guards

| guard | catches | cost |
| --- | --- | --- |
| `tools/lint/source.py` check 3 (`lint-ratchet` job): an arena free (`free!`, `free_slice!`, `free_raw`, `free_slice_ptr`) of the enclosing function's reference parameter (or `self` of a `&self` method) fails | the direct pattern; found both calls in the old `recycle_mapper_with_targets` | < 1 s |
| Miri on `tsrs_core`'s arena tests (`cargo +nightly miri test -p tsrs_core --features plain-ptrs --lib arena::tests`), with a new test that frees a slice in a callee through `free_slice_ptr`, and an ignored negative control that frees a reference parameter, which the CI job requires Miri to reject | the free-list code itself, and that Miri still sees the class | 6 s with the Miri sysroot cached; about a minute for the nightly and its sysroot in CI |
| The conformance gate on a `panic = "abort"` release build of tsrs-test (new `arena-safety` job in `.depot/workflows/ci.yml`) | the indirect forms the lint cannot see (a caller's protected parameter freed by a callee): 44 tests crashed before the fix | one more release build (66 s here, cold, 18 cores; a few minutes on a depot 8-core runner) and 15 s of suite |

`plain-ptrs` because Miri does not support the compressed handles' `PROT_NONE` reservation. Running the arena tests
under Miri also showed three tests that used a freed `P` or slice to read its address after the free (a retag of a
dead reference); they now take the address before the free.

## Results

| check | result |
| --- | --- |
| conformance gate (as CI), normal release build | 13,458 pass / 0 crash / 0 timeout |
| conformance gate, `panic = "abort"` release build | before: 13,414 pass / 44 crash; after: 13,458 / 0 / 0 |
| `--baselines types,symbols,js,jsmap,sourcemap` trees, main vs fix vs fix with `panic = "abort"` | identical |
| `--baselines types,symbols` with `TSRS_ARENA_POISON=1` | 13,458 pass |
| fourslash gate | 4,066 pass / 63 fail |
| `tools/regressions.sh` | 8 / 8 pass |
| the 38k-file codebase, `--pretty false`, 1 and 4 checkers: main, fix, fix with `panic = "abort"` | byte-identical (8,213,694 bytes, 40,542 errors); before the fix the abort build crashed |
| xstate-main, webpack, vscode with `panic = "abort"` | complete (crashed before) |
| Miri, `tsrs_core` arena tests | 7 pass, negative control rejected |

Speed and memory of the normal build (the 38k-file codebase, `--release`, 3 interleaved rounds, paired medians, fix
against main): one checker instructions -0.01%, cycles -0.7%, max RSS 0.0%; four checkers instructions +0.00%,
cycles -1.1%, max RSS +0.1%. Unchanged: the fix only changes the parameter's type.

## Not done

- `panic = "abort"` can now be measured (notes/perf-build-level.md section 3); it still could not ship as is, for
  the reasons given there.
- Miri on more of `tsrs_core` (the region and handle code) needs the system allocator path that `plain-ptrs`
  provides; only the arena tests were run.
