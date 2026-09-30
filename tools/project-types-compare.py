#!/usr/bin/env python3
"""Compare two project-wide .types/.symbols dumps (tools/oracle/project-types vs `tsrs-test types-dump`).

  project-types-compare.py <go-out> <rs-out> [--kind types|symbols] [--list <file>] [--show N]

Prints matching files / total from the manifests and the checker counters; with --list writes the differing
relative paths (input for `--text <list>` on a rerun). When both dumps have the text of a differing file, counts
the differing lines (lines only in one side, per file) and with --show prints the first N differing line pairs.
"""
import argparse
import difflib
import os
import sys


def load(path):
    entries, counts = {}, None
    order = []
    with open(path) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if parts[0] == "#counts":
                counts = parts[1:]
                continue
            h, n, rel = parts
            entries[rel] = (h, int(n))
            order.append(rel)
    return entries, order, counts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("go")
    ap.add_argument("rs")
    ap.add_argument("--kind", default="types")
    ap.add_argument("--list")
    ap.add_argument("--show", type=int, default=0)
    a = ap.parse_args()
    g, gorder, gc = load(os.path.join(a.go, "manifest." + a.kind))
    r, _, rc = load(os.path.join(a.rs, "manifest." + a.kind))
    only_g = [p for p in gorder if p not in r]
    only_r = [p for p in r if p not in g]
    diff = [p for p in gorder if p in r and g[p][0] != r[p][0]]
    same = sum(1 for p in gorder if p in r and g[p][0] == r[p][0])
    print(f"files: {same}/{len(g)} identical ({len(diff)} differ, {len(only_g)} only in go, {len(only_r)} only in rs)")
    print(f"lines (go): {sum(n for _, n in g.values())}")
    print(f"counts go: {gc}  rs: {rc}")
    for p in only_g[:10]:
        print("only go:", p)
    for p in only_r[:10]:
        print("only rs:", p)
    if a.list:
        with open(a.list, "w") as f:
            f.write("".join(p + "\n" for p in diff))
    total_lines = 0
    shown = 0
    missing = 0
    for p in diff:
        gp = os.path.join(a.go, a.kind, p + "." + a.kind)
        rp = os.path.join(a.rs, a.kind, p + "." + a.kind)
        if not (os.path.exists(gp) and os.path.exists(rp)):
            missing += 1
            continue
        gl = open(gp, encoding="utf-8", errors="surrogateescape", newline="").read().split("\r\n")
        rl = open(rp, encoding="utf-8", errors="surrogateescape", newline="").read().split("\r\n")
        n = 0
        for op, i1, i2, j1, j2 in difflib.SequenceMatcher(None, gl, rl, autojunk=False).get_opcodes():
            if op == "equal":
                continue
            n += max(i2 - i1, j2 - j1)
            if shown < a.show:
                shown += 1
                print(f"--- {p}:{i1 + 1}")
                for l in gl[i1:i2][:4]:
                    print("  go:", l[:400])
                for l in rl[j1:j2][:4]:
                    print("  rs:", l[:400])
        total_lines += n
    if diff:
        print(f"differing lines: {total_lines} (text missing for {missing} files)")


if __name__ == "__main__":
    sys.exit(main())
