#!/usr/bin/env python3
"""Markdown table from bench/tscrs-compare.sh results: python3 bench/tscrs-summary.py <work-dir>/results"""
import json, sys, pathlib

res = pathlib.Path(sys.argv[1])
tools = ["tsgo", "tsc-rs", "tsrs", "bun"]
apps = [p.stem for p in sorted(res.glob("*.json"))]
order = ["vscode", "sentry", "playwright", "excalidraw", "typeorm", "trpc-server"]
apps.sort(key=lambda a: order.index(a) if a in order else 99)
print((res / "versions.txt").read_text().replace("\n", "  \n"))
print("| app | " + " | ".join(f"{t} wall (s)" for t in tools) + " | tsrs vs tsc-rs | tsrs vs bun | tsc-rs vs bun | errors (tsgo/tsc-rs/tsrs/bun) | peak RSS GiB (tsgo/tsc-rs/tsrs/bun) |")
print("| --- |" + " ---: |" * (len(tools) + 3) + " --- | --- |")
geo = {"tsrs/tsc-rs": [], "tsrs/bun": [], "tsc-rs/bun": []}
for a in apps:
    runs = json.load(open(res / f"{a}.json"))["results"]
    m = dict(zip(tools, (r["median"] for r in runs)))
    errs, rss = [], []
    for t in tools:
        n = sum(1 for _ in open(res / f"{a}.{t}.diag"))
        errs.append(str(n))
        rss.append(f"{int((res / f'{a}.{t}.rss').read_text().split()[-1]) / 1048576:.2f}")
    r1, r2, r3 = m["tsc-rs"] / m["tsrs"], m["bun"] / m["tsrs"], m["bun"] / m["tsc-rs"]
    geo["tsrs/tsc-rs"].append(r1); geo["tsrs/bun"].append(r2); geo["tsc-rs/bun"].append(r3)
    print(f"| {a} | " + " | ".join(f"{m[t]:.2f}" for t in tools) + f" | {r1:.2f}x | {r2:.2f}x | {r3:.2f}x | {'/'.join(errs)} | {'/'.join(rss)} |")
g = lambda v: (lambda p: p ** (1 / len(v)))(__import__("math").prod(v))
print(f"| **geomean** |" + " |" * len(tools) + f" **{g(geo['tsrs/tsc-rs']):.2f}x** | **{g(geo['tsrs/bun']):.2f}x** | **{g(geo['tsc-rs/bun']):.2f}x** | | |")
print("\nspeed columns are `other wall / first wall`: above 1x means the first-named tool is faster (tsc-rs vs bun: bun wall / tsc-rs wall).")
