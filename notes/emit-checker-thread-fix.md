# Checker-thread fix

Status (2026-10-10): landed. `for_each_checker_group_do` (crates/tsrs_compiler/src/checkerpool.rs) builds groups only
from the checkers that own at least one of `files`, and runs on the calling thread when one group is active (the
comment there states the rule; this note has the numbers).

## Change

Only checkers that own at least one of `files` form groups. With one active group, `run_work_group`'s `count <= 1`
rule runs it on the calling thread (no checker thread); with several, one `checker-{k}` thread (512 MB stack,
`CHECKER_STACK_SIZE`) per active group. This is the built-in checker pool (emit, checker and declaration
diagnostics); the language server uses an external pool and is not affected. The calling thread is the 512 MB CLI main thread or `builder-N` thread, or a 256 MB `tsrs-test`
or fourslash test thread; the gates passed on the 256 MB test threads.

## Problem

Incremental emit calls `Program::emit` once per affected file; before the fix each call started `checkers.len()`
threads, each with its own arena chunk and allocator heap that are never reused.

## Measurements (`TSRS_EMIT=1 tsrs -b . --builders N`, 8 composite projects x 150 files, peak RSS / wall)

Each hunk applied unchanged to 535adce (local builds). Peak RSS via `getrusage(RUSAGE_CHILDREN).ru_maxrss`; thread
creations via `MIMALLOC_SHOW_STATS=1`.

| binary | builders 1 | builders 4 | builders 8 | threads created (b4) |
|---|---|---|---|---|
| 535adce (no fix) | 1694 MB / 1.40 s | 1727 MB / 1.21 s | 1749 MB / 1.18 s | 4.9 K |
| 535adce + this fix | 354 MB / 0.52 s | 393 MB / 0.46 s | 409 MB / 0.47 s | 171 |
| 535adce + 600723a (rejected, below) | 1691 MB / 1.27 s | 1717 MB / 1.13 s | 1746 MB / 1.14 s | 1.3 K |
| tsgo (ts-ref b85298b6) | 192 MB / 0.55 s | 257 MB / 0.34 s | 353 MB / 0.36 s | — |

Single 150-file composite project, `TSRS_EMIT=1 tsrs -p .`, 4 checkers: threads created 654 -> 54, peak RSS 306 -> 131 MB.

## Reproduction
Generator (python3), writes /tmp/rv/mem/n{1,2,4,8}:
```python
import os, json
def gen(root, n):
    os.makedirs(root, exist_ok=True); refs=[]
    for p in range(n):
        d=f"{root}/p{p}"; os.makedirs(f"{d}/src", exist_ok=True)
        json.dump({"compilerOptions":{"composite":True,"outDir":"out","rootDir":"src","target":"es2022","module":"esnext",
          "moduleResolution":"bundler","strict":True,"skipLibCheck":True,"declaration":True,"emitDeclarationOnly":True},
          "include":["src"]}, open(f"{d}/tsconfig.json","w"))
        for i in range(150):
            imp = f'import {{ T{i-1} }} from "./f{i-1}";\n' if i>0 else ""
            prev = f"prev: T{i-1}<number>;" if i>0 else ""
            open(f"{d}/src/f{i}.ts","w").write(imp + f"""export interface T{i}<A = string> {{ id{i}: A; nested: {{ [K in 'a'|'b'|'c'|'d'|'e']: Array<A | K> }}; {prev} }}
export type M{i}<X> = {{ [K in keyof X as `get${{Capitalize<string & K>}}`]: () => X[K] }};
export function make{i}<A>(a: A): T{i}<A> {{ return {{ id{i}: a, nested: {{ a: [a], b: [a], c: [a], d: [a], e: [a] }} }} as any; }}
export const v{i} = make{i}({{ x: 1, y: "s", z: [1,2,3] as const }});
export type G{i} = M{i}<T{i}<{{ q: number; r: string; s: boolean }}>>;
export class C{i}<A> {{ constructor(public a: A) {{}} map<B>(f: (a: A) => B): C{i}<B> {{ return new C{i}(f(this.a)); }} }}
export const c{i} = new C{i}(v{i}).map(x => x.nested).map(n => n.a.length);
""")
        refs.append({"path":f"p{p}"})
    json.dump({"files":[],"references":refs}, open(f"{root}/tsconfig.json","w"))
for n in (1,2,4,8): gen(f"/tmp/rv/mem/n{n}", n)
```
Measure on a fresh copy each run (peak RSS via `getrusage(RUSAGE_CHILDREN).ru_maxrss`; thread creations via
`MIMALLOC_SHOW_STATS=1`, `threads` total column):
```sh
cp -r /tmp/rv/mem/n8 /tmp/rv/memrun && cd /tmp/rv/memrun
TSRS_EMIT=1 python3 -c 'import resource,subprocess,sys;subprocess.run(sys.argv[1:]);print(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss//1024,"MB")' <tsrs> -b . --builders 4
TSRS_EMIT=1 MIMALLOC_SHOW_STATS=1 <tsrs> -b . --builders 4 2>&1 | grep -E '^ *threads'
```
`-b` needs a binary that has it (this hunk applied to the incremental/build branch).

## Rejected

A contract-preserving revision (`run_work_group_for`, commits 600723a/4bf81a6) was output-neutral but gave no memory
gain (table above), so the simpler hunk was kept.

## Gates of the code at 4121002 (vs main fb867b1, both built from these heads)
- conformance errors + `--baselines types,symbols --timeout 60`: result trees identical, default and
  `TSRS_LAZY_MEMBERS=0` (13458 pass each). (`compiler/intersectionConstructorReductionCrash` needs ~20 s on this box
  for both binaries; at the default 20 s timeout it is load-sensitive, hence `--timeout 60`.)
- `--baselines js --timeout 60`: same pass list (8680); fourslash 4066/63, same pass list;
  `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` clean; `tests/emit_gate.rs` 3/3.

## Output checks

Against tsgo built from ts-ref b85298b6: CLI non-incremental fixtures, `-b` diamond/chain edit sequences at builders
1/4/8 (including tsbuildinfo) and incremental JS emit sequences were identical, except one allowJs step (a `lib`
change) where tsgo itself is nondeterministic (programtosnapshot.go:162-179 stops at the first removed file in
sync.Map order; tsgo emits 11 or 1 files across runs, tsrs 1).
