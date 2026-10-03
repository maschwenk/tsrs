# Checker-thread fix (draft, not merge-ready)

Status: DRAFT, not merge-ready. Base: main fb867b1. Result: this contract-preserving revision is output-neutral but does
NOT fix the memory regression (measured below); only the earlier six-line hunk (52be3a5) does.

## Change
`crates/tsrs_compiler/src/checkerpool.rs`: `for_each_checker_group_do` no longer starts a thread for a checker that owns
none of `files`. New `run_work_group_for(single_threaded, count, indices, task)`; `run_work_group` delegates to it with
all indices (all other callers unchanged).

## Why this revision differs from the earlier six-line hunk (52be3a5 on mfs-cx/emit-builders-review, kept unchanged)
The earlier hunk called `run_work_group(.., active.len(), ..)`, whose `count <= 1` rule ran a single active group on
the *calling* thread instead of a 512 MB `checker-N` thread, and renumbered thread names. That changes threading
semantics beyond filtering empty groups (stack size and thread identity of every one-file call). This revision keeps
the original contract exactly: the single-threaded decision still uses `checkers.len()`, each active group still gets
its own `checker-{index}` thread with `CHECKER_STACK_SIZE`; only groups with no files are skipped.

## Problem it addresses (measured on the incremental/build branches, earlier revision)
Incremental emit calls `Program::emit` once per affected file; each call started `checkers.len()` threads, each keeping
its own arena chunk and allocator heap. 8 composite projects x 150 files, `TSRS_EMIT=1 tsrs -b . --builders 4`:
535adce 1.73 GB / 1.21 s; earlier revision 52be3a5 0.40 GB / 0.47 s; tsgo 0.26 GB / 0.34 s. On main (no incremental
or `-b` yet) the CLI only calls this with all files, so the change is expected to be output-neutral there.

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
Measure (peak RSS via `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`; thread creations via `MIMALLOC_SHOW_STATS=1`,
`threads` total column), on a fresh copy each run:
```sh
cp -r /tmp/rv/mem/n8 /tmp/rv/memrun && cd /tmp/rv/memrun
TSRS_EMIT=1 python3 -c 'import resource,subprocess,sys;subprocess.run(sys.argv[1:]);print(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss//1024,"MB")' <tsrs> -b . --builders 4
cd p0 && rm -rf out *.tsbuildinfo && TSRS_EMIT=1 MIMALLOC_SHOW_STATS=1 <tsrs> -p . 2>&1 | grep -E 'threads|peak rss'
```
`-b`/incremental need a binary that has them (e.g. this hunk applied on the incremental/build branch); on main use the
gates below.

## Measurements of this exact revision (600723a)
`checkerpool.rs` is byte-identical at fb867b1 and 535adce, so the 600723a hunk was applied unchanged to 535adce (local
build, not pushed) for `-b`. n8 fixture, peak RSS / wall, fresh copy per run:

| binary | builders 1 | builders 4 | builders 8 | threads created (b4) |
|---|---|---|---|---|
| 535adce (no fix) | 1694 MB / 1.40 s | 1727 MB / 1.21 s | 1749 MB / 1.18 s | 4.9 K |
| 535adce + 52be3a5 six-line hunk | 354 MB / 0.52 s | 393 MB / 0.46 s | 409 MB / 0.47 s | 171 |
| 535adce + this revision | 1691 MB / 1.27 s | 1717 MB / 1.13 s | 1746 MB / 1.14 s | 1.3 K |

Conclusion: the memory comes from the one *active* `checker-N` thread that each one-file `Program::emit` still starts
(each new thread gets its own arena chunk and allocator heap that are never reused); the idle threads that this revision
removes cost little. The six-line hunk works because a single active group runs on the calling thread
(`run_work_group`'s `count <= 1` rule), i.e. exactly the threading change this revision avoids. Choosing between them is
a design decision for the coordinator: keep the original contract (this revision; no memory gain) or accept that
one-group calls run on the caller's thread (52be3a5; CLI callers are the 512 MB main/builder threads).

## Gates of this exact revision vs main fb867b1 (both built from these heads)
- conformance errors + `--baselines types,symbols`, `--timeout 60`: result trees identical, default and
  `TSRS_LAZY_MEMBERS=0` (13458 pass each).
- `--baselines js --timeout 60`: same pass list (8680 pass in both).
- fourslash 4066/63, same pass list; `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` clean;
  `tests/emit_gate.rs` 3/3.
- Timeout noise: `compiler/intersectionConstructorReductionCrash` needs ~20 s with the default 20 s per-test timeout
  (direct CLI, 3 alternating runs: main 20.50-20.69 s, this revision 20.71-20.76 s; `tsrs-test show`: 19.97 s vs
  20.24-20.30 s). It flips to timeout at the default limit and passes in both at 60 s; it is a single-file program, for
  which this revision starts one checker thread instead of four idle-plus-one. Treated as load/layout noise, not a
  source failure.
