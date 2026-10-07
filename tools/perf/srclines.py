#!/usr/bin/env python3
"""Source lines of the samples whose innermost frame is in SYMBOL (not committed; notes/perf-serial-steps.md).
Reads `perf script --max-stack 1 -F comm,tid,ip,sym,symoff` on stdin; uses nm and llvm-addr2line on BINARY.

    srclines.py BINARY SYMBOL [ADDR2LINE]
"""
import collections, re, subprocess, sys

binary, symbol = sys.argv[1], sys.argv[2]
addr2line = sys.argv[3] if len(sys.argv) > 3 else "llvm-addr2line"
syms = {}
for line in subprocess.run(["nm", "-C", "--defined-only", binary], capture_output=True, text=True).stdout.splitlines():
    parts = line.split(" ", 2)
    if len(parts) == 3 and symbol in parts[2]:
        syms[re.sub(r"::h[0-9a-f]{16}$", "", parts[2])] = int(parts[0], 16)
offsets = collections.Counter()
for line in sys.stdin:
    m = re.search(r"\s([^\s].*?)\+0x([0-9a-f]+)", line)
    if m and symbol in m.group(1):
        name = re.sub(r"::h[0-9a-f]{16}$", "", m.group(1).strip())
        base = syms.get(name) or next(iter(syms.values()), None)
        if base is not None:
            offsets[base + int(m.group(2), 16)] += 1
total = sum(offsets.values())
print(f"{total} samples in {symbol} ({len(syms)} symbols matched: {list(syms)[:3]})")
addrs = sorted(offsets)
out = subprocess.run([addr2line, "-e", binary, "-a", "-i", "-f", "-C"] + [hex(a) for a in addrs], capture_output=True, text=True).stdout
# -a starts each address's answer with the address itself.
blocks, cur = [], None
for l in out.splitlines():
    if re.fullmatch(r"0x[0-9a-f]+", l.strip()):
        cur = []
        blocks.append("\n".join(cur) if False else cur)
    elif cur is not None and l.strip():
        cur.append(l)
blocks = ["\n".join(b) for b in blocks]
inner, outer, chain = collections.Counter(), collections.Counter(), collections.Counter()
for a, b in zip(addrs, blocks):
    lines = b.splitlines()
    pairs = [(lines[i], lines[i + 1].split("/")[-1]) for i in range(0, len(lines) - 1, 2)]
    if not pairs:
        continue
    n = offsets[a]
    inner[f"{pairs[0][1]}  {pairs[0][0][:80]}"] += n
    outer[pairs[-1][1]] += n
    chain[" < ".join(p[1] for p in pairs[:4])] += n
for title, c in (("by line of the outermost function", outer), ("by innermost line", inner), ("by inline chain", chain)):
    print(f"\n== {title}")
    for k, v in c.most_common(50):
        print(f"{v:6d} {100 * v / max(total, 1):5.1f}%  {k}")
