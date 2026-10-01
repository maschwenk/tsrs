#!/usr/bin/env python3
"""Write a ~2,000-file sample of a large project for `tsrs-test types-dump --sample`.

  project-types-sample.py <project-dir> > target/project-types-sample.txt

The list names files of the measured (private) project, so it is not committed.

Deterministic (sorted paths, fixed strides). Over-represents the shapes that stress the checker: zod schemas
(heaviest by `z.` call count first), ORM entities, Temporal workflows, router endpoints; plus a spread of the rest.
Paths are relative to the project's tsconfig directory, as in the types-dump manifests.
"""
import os
import re
import subprocess
import sys

QUOTAS = [("zod", 500), ("orm-entities", 450), ("workflows", 400), ("router", 450), ("rest", 200)]


def stride(items, n):
    if len(items) <= n:
        return list(items)
    return [items[i * len(items) // n] for i in range(n)]


def main():
    root = sys.argv[1]
    files = subprocess.run(["git", "ls-files", "*.ts", "*.tsx"], cwd=root, capture_output=True, text=True, check=True).stdout.split()
    files = sorted(f for f in files if not f.endswith(".d.ts") and not f.startswith(("scripts/", "client/")))
    zod_calls = {}
    for f in files:
        try:
            text = open(os.path.join(root, f), encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        if re.search(r"""from ['"]zod['"]""", text):
            zod_calls[f] = len(re.findall(r"\bz\.", text))
    groups = {
        "zod": sorted(zod_calls, key=lambda f: (-zod_calls[f], f)),
        "orm-entities": [f for f in files if "/orm/entities/" in f],
        "workflows": [f for f in files if "workflow" in f.lower()],
        "router": [f for f in files if f.startswith("src/router/")],
    }
    chosen = []
    seen = set()
    for name, n in QUOTAS:
        pool = groups.get(name) or files
        if name == "zod":
            heavy = [f for f in pool if f not in seen][: n * 3 // 5]
            rest = [f for f in pool if f not in seen and f not in heavy]
            picked = heavy + stride(rest, n - len(heavy))
        else:
            picked = stride([f for f in pool if f not in seen], n)
        for f in picked:
            seen.add(f)
            chosen.append((name, f))
    print("# tsrs-test types-dump --sample list for " + os.path.basename(os.path.abspath(root)) + "; regenerate with tools/project-types-sample.py")
    print("# groups: " + ", ".join(f"{name} {sum(1 for g, _ in chosen if g == name)}" for name, _ in QUOTAS))
    for f in sorted(f for _, f in chosen):
        print(f)


if __name__ == "__main__":
    main()
