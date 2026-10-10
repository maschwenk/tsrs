#!/usr/bin/env python3
"""Aggregate liballocstacks.dylib output (exact malloc counts per return-address stack) into per-call-site tables.

usage: python3 -I allocsites.py --binary <dSYM DWARF or binary> --stacks <stacks.tsv> --out <prefix> [--top 40] [--depth 12]

For every stack the script symbolicates frames innermost-first (atos -i -fullPath, so inlined frames carry their own
file:line) until it finds (a) the innermost frame that is not in the Rust standard library / hashbrown / smallvec
("innermost non-allocator frame") and (b) the first frame inside a tsrs_* crate. It writes
  <out>.sites.tsv   one row per (first tsrs frame file:line) with count, bytes, kinds
  <out>.inner.tsv   one row per innermost non-allocator frame file:line
  <out>.funcs.tsv   one row per first tsrs function (all lines merged)
and prints the top N of each. Mangled (_R...) names are demangled with rustfilt when it is on PATH.
"""
import argparse, collections, os, re, shutil, subprocess, sys

LIB_CRATES = {"alloc", "core", "std", "__rustc", "hashbrown", "smallvec", "indexmap", "rustc_hash", "allocator_api2"}
RE_LINE = re.compile(r"^(.*?) \(in ([^)]*)\) (?:\((.*):(\d+)\)|\+ \d+)?\s*$")

def crate_of(path, name):
    if not path:
        return "sys"
    m = re.search(r"/(?:rustlib/src/rust|rustc/[0-9a-f]+)/library/([a-z_]+)/", path)
    if m:
        return m.group(1)
    m = re.search(r"/rust/deps/([A-Za-z0-9_]+)-[0-9][^/]*/", path) or re.search(r"/registry/src/[^/]+/([A-Za-z0-9_]+)-[0-9][^/]*/", path)
    if m:
        return m.group(1).replace("-", "_")
    m = re.search(r"/crates/([A-Za-z0-9_]+)/", path)
    if m:
        return m.group(1)
    return "other"

def symbolize(binary, load, addrs):
    """addr -> list of (name, crate, file:line) innermost-first (inlined chain)."""
    out = {}
    addrs = list(addrs)
    for i in range(0, len(addrs), 4000):
        batch = addrs[i:i + 4000]
        inp = "\n".join(hex(a - 1) for a in batch) + "\n"
        res = subprocess.run(["atos", "-o", binary, "-arch", "arm64", "-l", hex(load), "-fullPath", "-i"], input=inp, capture_output=True, text=True)
        blocks, cur = [], []
        for line in res.stdout.split("\n"):
            if line.strip() == "":
                if cur:
                    blocks.append(cur); cur = []
            else:
                cur.append(line)
        if cur:
            blocks.append(cur)
        if len(blocks) != len(batch):
            sys.exit(f"atos returned {len(blocks)} blocks for {len(batch)} addresses")
        for a, block in zip(batch, blocks):
            frames = []
            for line in block:
                m = RE_LINE.match(line)
                if not m:
                    frames.append((line.strip(), "sys", "")); continue
                name, path, ln = m.group(1), m.group(3), m.group(4)
                frames.append((name, crate_of(path, name), f"{path}:{ln}" if path else ""))
            out[a] = frames
    return out

def demangle(names):
    names = [n for n in names if n.startswith("_R") or n.startswith("_ZN")]
    if not names or not shutil.which("rustfilt"):
        return {}
    res = subprocess.run(["rustfilt"], input="\n".join(names) + "\n", capture_output=True, text=True)
    return dict(zip(names, res.stdout.split("\n")))

def short_path(fl):
    m = re.search(r"/crates/(.*)$", fl)
    if m: return m.group(1)
    m = re.search(r"/library/(.*)$", fl)
    if m: return "rust/" + m.group(1)
    m = re.search(r"/rust/deps/(.*)$", fl)
    if m: return m.group(1)
    return fl

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True); ap.add_argument("--stacks", required=True); ap.add_argument("--out", required=True)
    ap.add_argument("--top", type=int, default=40); ap.add_argument("--depth", type=int, default=12)
    args = ap.parse_args()
    load = None; totals = ""; rows = []
    with open(args.stacks) as f:
        for line in f:
            if line.startswith("# load"):
                load = int(line.split()[2], 16); continue
            if line.startswith("# totals"):
                totals = line.strip(); continue
            count, nbytes, kind, frames = line.rstrip("\n").split("\t")
            rows.append((int(count), int(nbytes), kind, [int(x, 16) for x in frames.split()]))
    print(f"{totals}\nstacks={len(rows)} allocations={sum(r[0] for r in rows)} bytes={sum(r[1] for r in rows)}", file=sys.stderr)
    sym = {}
    # resolve frames lazily, depth by depth
    inner = [None] * len(rows); first_tsrs = [None] * len(rows); fullchain = [[] for _ in rows]; enclosing = [None] * len(rows)
    for d in range(args.depth):
        need = {r[3][d] for i, r in enumerate(rows) if d < len(r[3]) and (inner[i] is None or first_tsrs[i] is None) and r[3][d] not in sym}
        if need:
            print(f"depth {d}: symbolizing {len(need)} addresses", file=sys.stderr)
            sym.update(symbolize(args.binary, load, need))
        for i, r in enumerate(rows):
            if d >= len(r[3]) or (inner[i] is not None and first_tsrs[i] is not None):
                continue
            chain = sym.get(r[3][d], [])
            fullchain[i].extend(chain)
            for fr in chain:
                if inner[i] is None and fr[1] not in LIB_CRATES:
                    inner[i] = fr
                if first_tsrs[i] is None and fr[1].startswith("tsrs_"):
                    first_tsrs[i] = fr
                if first_tsrs[i] is not None and enclosing[i] is None and fr[0].startswith("_R"):
                    enclosing[i] = fr
    dm = demangle({fr[0] for i in range(len(rows)) for fr in ((inner[i] or ("", "", "")), (first_tsrs[i] or ("", "", "")), (enclosing[i] or ("", "", "")))} | {fr[0] for ch in fullchain for fr in ch[:6]})
    def nm(fr): return dm.get(fr[0], fr[0]) if fr else "?"
    def fname(i):
        if first_tsrs[i] is None: return "-"
        n = nm(first_tsrs[i])
        if not first_tsrs[i][0].startswith("_R") and enclosing[i] is not None:
            n += " [in " + nm(enclosing[i]).split("::")[-1] + "]"
        return n
    def agg(keyfn):
        t = collections.defaultdict(lambda: [0, 0, collections.Counter(), collections.Counter()])
        for i, r in enumerate(rows):
            k = keyfn(i); e = t[k]; e[0] += r[0]; e[1] += r[1]; e[2][r[2]] += r[0]
            e[3][" <- ".join(short_path(fr[2]) for fr in fullchain[i][:6] if fr[1] not in LIB_CRATES)[:200]] += r[0]
        return t
    total = sum(r[0] for r in rows)
    def write(table, path, title, with_name):
        items = sorted(table.items(), key=lambda kv: -kv[1][0])
        with open(path, "w") as f:
            f.write("count\tpct\tbytes\tmean\tkinds\tsite\tfunction\ttop_caller_chain\n")
            for k, e in items:
                site, fn = k
                kinds = " ".join(f"{kd}={c}" for kd, c in e[2].most_common())
                chain = e[3].most_common(1)[0][0] if e[3] else ""
                f.write(f"{e[0]}\t{100*e[0]/total:.3f}\t{e[1]}\t{e[1]/e[0]:.1f}\t{kinds}\t{site}\t{fn}\t{chain}\n")
        print(f"\n## {title} (top {args.top} of {len(items)}; total allocations {total})\n")
        print("| # | count | % | bytes | mean B | kinds | site | function |"); print("|---|---|---|---|---|---|---|---|")
        for n, (k, e) in enumerate(items[:args.top], 1):
            site, fn = k
            kinds = " ".join(f"{kd}={c}" for kd, c in e[2].most_common())
            print(f"| {n} | {e[0]:,} | {100*e[0]/total:.2f} | {e[1]:,} | {e[1]/e[0]:.0f} | {kinds} | `{site}` | `{fn[:110]}` |")
    write(agg(lambda i: (short_path(first_tsrs[i][2]) if first_tsrs[i] else "(no tsrs frame)", fname(i))), args.out + ".sites.tsv", "first frame inside tsrs crates (file:line)", True)
    write(agg(lambda i: (short_path(inner[i][2]) if inner[i] else "(unresolved)", nm(inner[i]) if inner[i] else "-")), args.out + ".inner.tsv", "innermost non-allocator frame (file:line)", True)
    write(agg(lambda i: (short_path(first_tsrs[i][2]).rsplit(":", 1)[0] if first_tsrs[i] else "(no tsrs frame)", fname(i))), args.out + ".funcs.tsv", "first tsrs function (all lines merged)", True)

if __name__ == "__main__":
    main()
