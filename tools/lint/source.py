#!/usr/bin/env python3
"""Source checks that clippy cannot express (docs/RUST.md, "Source checks").

  tools/lint/source.py             check; exit 1 on a change to the Send/Sync inventory or a new uncommented ordering
  tools/lint/source.py --update    rewrite the inventory and lower the ordering counts (after reviewing the change)
  tools/lint/source.py --update --allow-increase   also raise ordering counts (first run, or a deliberate exception)
  tools/lint/source.py --self-test check the patterns on fixed snippets

1. `unsafe impl Send` / `unsafe impl Sync`: tools/lint/send_sync.tsv lists every one. Adding or removing one fails
   until --update records it, so each new cross-thread escape is a reviewed line in a diff (P<T> is Send + Sync by
   decree; docs/PORTING.md, "Threading"). After Bun's vm-thread-door inventory.
2. Weakened atomic orderings (Relaxed, Acquire, Release, AcqRel) need a `//` comment on the same line or in the three
   lines above saying why the ordering is enough. tools/lint/atomics.tsv holds the per-file count of those without
   one; a file may not go above it. Bun's rule: "default to seq_cst and comment any weakened ordering".

Scans crates/*/src/**/*.rs, without test files (*_test.rs, tests.rs, tests/ directories), the generated
tsrs_fourslash crate, and files whose first line says they are generated.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
INVENTORY = os.path.join(ROOT, "tools", "lint", "send_sync.tsv")
ATOMICS = os.path.join(ROOT, "tools", "lint", "atomics.tsv")
EXCLUDE_CRATES = {"tsrs_fourslash"}

SEND_SYNC = re.compile(r"\bunsafe\s+impl\b(?:\s*<(?P<generics>[^{]*?)>)?\s+(?P<trait>Send|Sync)\s+for\s+(?P<type>[^{]+?)\s*\{")
WEAK = ("Relaxed", "Acquire", "Release", "AcqRel")
QUALIFIED = re.compile(r"\bOrdering::(Relaxed|Acquire|Release|AcqRel)\b")
IMPORTED = re.compile(r"\buse\s+std::sync::atomic::(?:\{[^}]*\bOrdering::(?:\{[^}]*\}|\w+)[^}]*\}|Ordering::(?:\{[^}]*\}|\w+))\s*;")
LOOKBACK = 3


def source_files():
    crates = os.path.join(ROOT, "crates")
    for crate in sorted(os.listdir(crates)):
        src = os.path.join(crates, crate, "src")
        if crate in EXCLUDE_CRATES or not os.path.isdir(src):
            continue
        for dirpath, _dirs, files in os.walk(src):
            if os.sep + "tests" in dirpath[len(src):]:
                continue
            for name in sorted(files):
                if name.endswith(".rs") and not name.endswith(("_test.rs", "_tests.rs")) and name != "tests.rs":
                    path = os.path.join(dirpath, name)
                    text = open(path, encoding="utf-8", errors="replace").read()
                    first = text.split("\n", 1)[0]
                    if "Code generated" in first or "@generated" in first:
                        continue
                    yield os.path.relpath(path, ROOT), text


def code_part(line):
    """The line without a trailing `//` comment (string literals with `//` are rare enough here to ignore)."""
    i = line.find("//")
    return line if i < 0 else line[:i]


def send_sync_impls(rel, text):
    rows = []
    for m in SEND_SYNC.finditer(text):
        line_start = text.rfind("\n", 0, m.start()) + 1
        if text[line_start:m.start()].lstrip().startswith("//"):
            continue
        generics = f"<{m.group('generics').strip()}> " if m.group("generics") else ""
        rows.append((rel, m.group("trait"), generics + " ".join(m.group("type").split())))
    return rows


def imported_orderings(text):
    names = set()
    for m in IMPORTED.finditer(text):
        names.update(w for w in WEAK if re.search(r"\b" + w + r"\b", m.group(0)))
    return names


def uncommented_orderings(text):
    """(line number, line) of each weakened ordering without a nearby comment."""
    bare = imported_orderings(text)
    bare_re = re.compile(r"(?<![\w:])(" + "|".join(sorted(bare)) + r")\b") if bare else None
    lines = text.split("\n")
    found = []
    for i, line in enumerate(lines):
        code = code_part(line)
        if code.lstrip().startswith("use "):
            continue
        hits = len(QUALIFIED.findall(code)) + (len(bare_re.findall(code)) if bare_re else 0)
        if not hits:
            continue
        commented = "//" in line or any("//" in lines[j] for j in range(max(0, i - LOOKBACK), i))
        if not commented:
            found.extend([(i + 1, line.strip())] * hits)
    return found


def scan():
    inventory, atomics, details, files = [], {}, {}, 0
    for rel, text in source_files():
        files += 1
        inventory.extend(send_sync_impls(rel, text))
        found = uncommented_orderings(text)
        if found:
            atomics[rel] = len(found)
            details[rel] = found
    return files, sorted(inventory), atomics, details


def read_tsv(path):
    if not os.path.exists(path):
        return []
    return [tuple(l.rstrip("\n").split("\t")) for l in open(path) if l.strip() and not l.startswith("#")]


def write(path, header, rows):
    with open(path, "w") as f:
        f.write(header)
        for row in rows:
            f.write("\t".join(str(x) for x in row) + "\n")


def self_test():
    rows = send_sync_impls("x.rs", "unsafe impl Send for A {}\nunsafe impl<T: ?Sized> Sync for P<T> {}\n// unsafe impl Send for C {}\n")
    assert rows == [("x.rs", "Send", "A"), ("x.rs", "Sync", "<T: ?Sized> P<T>")], rows
    assert send_sync_impls("x.rs", "impl Send for Safe {}\n") == []
    text = "\n".join([
        "use std::sync::atomic::{AtomicU32, Ordering::Relaxed};",
        "fn f() {",
        "    a.load(Ordering::Relaxed);",            # flagged
        "    b.load(Relaxed); // counter only",       # comment on the line
        "    // ids only need to be unique",
        "    c.fetch_add(1, Relaxed);",               # comment above
        "    d.store(1, Ordering::SeqCst);",          # not weakened
        "    let x = 1;",
        "    e.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed);",  # two; the nearest comment is 4 lines up
        "}",
    ])
    found = uncommented_orderings(text)
    assert [n for n, _ in found] == [3, 9, 9], found
    assert uncommented_orderings("enum Kind { Release }\nlet k = Kind::Release;\n") == []
    files, inventory, _, _ = scan()
    assert files > 100 and inventory, "the scan found nothing; the paths or patterns are wrong"
    print("source checks self-test: ok")


def main(argv):
    if "--self-test" in argv:
        return self_test()
    files, inventory, atomics, details = scan()
    if "--update" in argv:
        write(INVENTORY, "# Every unsafe impl Send / Sync (tools/lint/source.py --update). Review each change.\n", inventory)
        old = {r[1]: int(r[0]) for r in read_tsv(ATOMICS)}
        if os.path.exists(ATOMICS) and "--allow-increase" not in argv:
            lowered = {f: min(n, old[f]) for f, n in atomics.items() if f in old}
        else:
            lowered = atomics
        write(ATOMICS, "# Weakened atomic orderings without a comment, per file (tools/lint/source.py). Counts only go down.\n",
              sorted((n, f) for f, n in lowered.items()))
        print(f"source checks: {len(inventory)} Send/Sync impls, {sum(lowered.values())} uncommented orderings recorded")
        return

    failed = False
    recorded = set(read_tsv(INVENTORY))
    current = set(inventory)
    for row in sorted(current - recorded):
        print(f"new unsafe impl {row[1]} for {row[2]} in {row[0]}: justify it with a SAFETY comment, then record it with "
              f"tools/lint/source.py --update")
        failed = True
    for row in sorted(recorded - current):
        print(f"unsafe impl {row[1]} for {row[2]} in {row[0]} is gone: record it with tools/lint/source.py --update")
        failed = True
    baseline = {r[1]: int(r[0]) for r in read_tsv(ATOMICS)}
    for f, n in sorted(atomics.items()):
        if n > baseline.get(f, 0):
            print(f"{f}: {n} weakened atomic orderings without a comment (baseline {baseline.get(f, 0)}); say why the "
                  f"ordering is enough in a // comment on or just above the line:")
            for line_no, line in details[f]:
                print(f"  {f}:{line_no}  {line}")
            failed = True
    if failed:
        sys.exit(1)
    lower = sum(n - atomics.get(f, 0) for f, n in baseline.items() if atomics.get(f, 0) < n)
    print(f"source checks: ok ({files} files, {len(inventory)} Send/Sync impls, {sum(atomics.values())} uncommented "
          f"orderings)" + (f"; {lower} fewer than recorded, run --update" if lower else ""))


if __name__ == "__main__":
    main(sys.argv[1:])
