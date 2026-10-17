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

def fmt(x, spec, scale=1.0):
    """A median, or `-` where the runs did not record it (counters come from STATS=1 runs, times from timed runs)."""
    return "-" if x is None else format(x * scale, spec)

print("| mode | build | n | symbols | types | instantiations | allocs (M) | check s | Go mem used GB | real s | peak footprint GB | max RSS GB | load |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for (mode, label), rs in sorted(rows.items(), key=lambda kv: (kv[0][0] != "single",)):
    syms = {r.get("Symbols") for r in rs}
    flag = "" if len(syms) == 1 else " (varies)"
    print(f"| {mode} | {label} | {len(rs)} | {fmt(med(rs, 'Symbols'), ',.0f')}{flag} | {fmt(med(rs, 'Types'), ',.0f')} | "
          f"{fmt(med(rs, 'Instantiations'), ',.0f')} | {fmt(med(rs, 'Memory allocs'), '.2f', 1e-6)} | "
          f"{fmt(med(rs, 'Check time'), '.2f')} | {fmt(med(rs, 'Memory used'), '.2f', 1024 / 1e9)} | {fmt(med(rs, 'real'), '.2f')} | "
          f"{fmt(med(rs, 'peak'), '.2f', 1e-9)} | {fmt(med(rs, 'maxrss'), '.2f', 1e-9)} | {fmt(med(rs, 'load'), '.0f')} |")
