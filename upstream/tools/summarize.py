#!/usr/bin/env python3
"""Medians per (label, mode) from measure.py JSONL output, as a markdown table."""
import json, statistics, sys
from collections import OrderedDict

rows = OrderedDict()
for path in sys.argv[1:]:
    for line in open(path):
        r = json.loads(line)
        rows.setdefault((r["mode"], r["label"]), []).append(r)

def med(rs, k):
    v = [r[k] for r in rs if r.get(k) is not None]
    return statistics.median(v) if v else None

print("| mode | build | n | symbols | types | instantiations | allocs (M) | check s | Go mem used GB | peak footprint GB | max RSS GB | load |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for (mode, label), rs in sorted(rows.items(), key=lambda kv: (kv[0][0] != "single",)):
    syms = {r.get("Symbols") for r in rs}
    flag = "" if len(syms) == 1 else " (varies)"
    allocs = med(rs, "Memory allocs")
    mem = med(rs, "Memory used")
    print(f"| {mode} | {label} | {len(rs)} | {int(med(rs,'Symbols')):,}{flag} | {int(med(rs,'Types')):,} | {int(med(rs,'Instantiations')):,} | "
          f"{allocs/1e6:.2f} | {med(rs,'Check time'):.2f} | {mem*1024/1e9:.2f} | {med(rs,'peak')/1e9:.2f} | {med(rs,'maxrss')/1e9:.2f} | {med(rs,'load'):.0f} |"
          if allocs is not None else
          f"| {mode} | {label} | {len(rs)} | {int(med(rs,'Symbols')):,}{flag} | {int(med(rs,'Types')):,} | {int(med(rs,'Instantiations')):,} | - | {med(rs,'Check time'):.2f} | "
          f"{(mem*1024/1e9) if mem else 0:.2f} | {med(rs,'peak')/1e9:.2f} | {med(rs,'maxrss')/1e9:.2f} | {med(rs,'load'):.0f} |")
