#!/usr/bin/env python3
"""Share of Time Profiler samples whose innermost frame is in the allocator.

usage: xcrun xctrace export --input X.trace --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' > tp.xml
       python3 -I tpshare.py tp.xml
Classes (innermost frame): mimalloc (mi_*, _mi_*, malloc/free/realloc/calloc entry points, mimalloc_safe),
rust-shim (alloc::alloc, alloc::raw_vec, __rust_alloc/__rdl_*, Global allocator impls), memcpy-in-alloc (memmove/memcpy/
bzero whose next 4 frames include an allocator or RawVec/hashbrown-resize frame), hashbrown-resize (reserve_rehash/
resize/prepare_resize innermost), other. Names are demangled with rustfilt when available.
"""
import collections, re, shutil, subprocess, sys, xml.etree.ElementTree as ET

def demangle(names):
    names = [n for n in names if n.startswith("_R") or n.startswith("_ZN")]
    if not names or not shutil.which("rustfilt"): return {}
    res = subprocess.run(["rustfilt"], input="\n".join(names) + "\n", capture_output=True, text=True)
    return dict(zip(names, res.stdout.split("\n")))

RE_MI = re.compile(r"^(_?_?mi_|mimalloc|_?malloc$|_?free$|_?realloc$|_?calloc$|_?posix_memalign$|_?aligned_alloc$|_?malloc_zone_|_?free_sized|_?malloc_size)")
RE_SHIM = re.compile(r"(^alloc::alloc::|^alloc::raw_vec::|^<alloc::raw_vec::|^<alloc::alloc::Global|^__rust_(alloc|dealloc|realloc)|^__rdl_|^__rg_|alloc::alloc::(alloc|dealloc|realloc|exchange_malloc)|raw_vec::finish_grow|RawVecInner|<mimalloc_safe::MiMalloc as core::alloc::global::GlobalAlloc>)")
RE_RESIZE = re.compile(r"hashbrown::raw::.*(reserve_rehash|resize|prepare_resize|fallible_with_capacity|new_uninitialized)|RawTableInner.*(resize|rehash|with_capacity)")
RE_MEM = re.compile(r"^_?_?(platform_)?(memmove|memcpy|memset|bzero)")

def main():
    frames = {}  # id -> name
    rows = []    # list of frame-name lists innermost first
    ctx = ET.iterparse(sys.argv[1], events=("end",))
    for ev, el in ctx:
        if el.tag == "frame":
            if el.get("id") is not None: frames[el.get("id")] = el.get("name", "?")
        elif el.tag == "row":
            bt = el.find("tagged-backtrace")
            if bt is None:
                ref = el.find("tagged-backtrace[@ref]")
            names = []
            if bt is not None:
                if bt.get("ref") is not None:
                    names = rows_by_id.get(bt.get("ref"), [])
                else:
                    for fr in bt.findall("frame"):
                        names.append(frames.get(fr.get("ref"), fr.get("name", "?")) if fr.get("ref") else fr.get("name", "?"))
                    rows_by_id[bt.get("id")] = names
            rows.append(names)
            el.clear()
    dm = demangle({n for r in rows for n in r[:6]})
    cls = collections.Counter(); top = collections.defaultdict(collections.Counter)
    def d(n): return dm.get(n, n)
    for r in rows:
        if not r: cls["(no stack)"] += 1; continue
        inner = d(r[0]); nxt = [d(x) for x in r[1:6]]
        if RE_MI.search(inner): c = "mimalloc"
        elif RE_SHIM.search(inner): c = "rust-shim"
        elif RE_MEM.search(inner) and any(RE_MI.search(x) or RE_SHIM.search(x) or RE_RESIZE.search(x) for x in nxt): c = "memcpy-in-alloc"
        elif RE_RESIZE.search(inner): c = "hashbrown-resize"
        else: c = "other"
        cls[c] += 1; top[c][inner[:90]] += 1
    total = len(rows)
    print(f"samples total: {total}")
    for c, n in cls.most_common():
        print(f"  {c:18s} {n:7d}  {100*n/total:6.2f}%")
    for c in ("mimalloc", "rust-shim", "memcpy-in-alloc", "hashbrown-resize"):
        print(f"\n{c}: top innermost frames")
        for name, n in top[c].most_common(12): print(f"  {n:6d}  {name}")
    print("\nother: top innermost frames")
    for name, n in top["other"].most_common(15): print(f"  {n:6d}  {name}")
    # inclusive: any mimalloc frame anywhere in the stack
    inc = sum(1 for r in rows if any(RE_MI.search(d(x)) for x in r[:8]))
    print(f"\ninclusive (a mimalloc frame within the innermost 8 frames): {inc} ({100*inc/total:.2f}%)")

rows_by_id = {}
if __name__ == "__main__":
    main()
