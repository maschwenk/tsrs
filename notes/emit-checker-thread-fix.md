# Checker-thread fix (draft, not merge-ready)

Status: DRAFT checkpoint for independent validation. Base: main fb867b1. Gating and measurement of this exact revision
are in progress; numbers below marked "earlier revision" do NOT measure this code.

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
