#!/usr/bin/env python3
"""Reads `perf script -F ip,sym,dso` with call chains; for each hot leaf of interest prints its top callers
(the first two non-allocator, non-libc frames)."""
import collections, re, sys
INTEREST = re.compile(r"libc\.so|_mi_|mi_free|mi_theap|mi_malloc|reserve_rehash")
SKIP = re.compile(r"libc\.so|_mi_|mi_free|mi_theap|mi_malloc|hashbrown|alloc::|core::|__rust|__rdl|<std::")
leaf_total = collections.Counter()
callers = collections.defaultdict(collections.Counter)
total = 0
def flush(stack):
    global total
    if not stack: return
    total += 1
    leaf = stack[0]
    if not INTEREST.search(leaf): return
    key = re.sub(r"\+0x[0-9a-f]+", "", leaf)
    if "libc.so" in leaf:
        key = leaf.split()[0] + " (libc)"
    leaf_total[key] += 1
    ctx = [f for f in stack[1:] if not SKIP.search(f)][:2]
    callers[key][" <- ".join(re.sub(r"\s*\(.*\)$", "", c)[:110] for c in ctx)] += 1
stack = []
for line in sys.stdin:
    line = line.rstrip()
    if not line:
        flush(stack); stack = []; continue
    stack.append(line.strip())
flush(stack)
print(f"samples {total}")
for leaf, n in leaf_total.most_common(25):
    print(f"\n{100*n/total:5.2f}%  {leaf[:160]}")
    for c, m in callers[leaf].most_common(8):
        print(f"        {100*m/n:5.1f}%  {c}")
