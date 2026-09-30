#!/usr/bin/env python3
"""Cluster conformance failures by the shape of their first differing line pair.

  tools/cluster-diffs.py [--class codes|fail] [--baseline errors|types|symbols] [--results DIR] [--top N] [--examples K]

With `--baseline types|symbols` (after `tsrs-test run --baselines types,symbols`) it clusters the `.types`/`.symbols`
mismatches instead: names from `<results>/<baseline>-<class>.txt` (class defaults to `fail`), diffs from
`<suite>/<name>.<baseline>.diff`. Their `>expr : type` lines are normalized like quoted text.

Reads `<results>/<class>.txt` and each test's `<suite>/<name>.diff` written by `tsrs-test run`, takes the first
removed (`-`, expected) and added (`+`, actual) line of the first hunk, and normalizes both: quoted text ('...',
"..."), file positions, numbers and identifiers inside type text are replaced by placeholders, so e.g.
`Type 'Foo<string>' is not assignable to type 'Bar'.` and `Type 'X' is not assignable to type 'Y'.` land in one
cluster. Within quotes the *punctuation skeleton* is kept (`'A<B>'` -> `'_<_>'`), which is what usually tells printer
clusters apart (missing `typeof`, `import("…")` vs a bare name, `...` truncation, parenthesization).

Output: clusters sorted by size, each with a count and example test ids.
"""

import argparse
import collections
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

QUOTED = re.compile(r"'((?:[^'\\]|\\.)*)'|\"((?:[^\"\\]|\\.)*)\"")
POSITION = re.compile(r"^[^\s(]+\(\d+,\d+\)")
RELATED = re.compile(r"\S+\.\w+:\d+:\d+")
IDENT = re.compile(r"[A-Za-z_$#][\w$]*")
NUMBER = re.compile(r"\b\d+(\.\d+)?\b")
SPACES = re.compile(r"\s+")


def skeleton(text):
    """Keeps keywords that change printing (typeof, import, readonly, ...) and punctuation; drops names/numbers."""
    keep = {"typeof", "import", "readonly", "keyof", "unique", "symbol", "infer", "extends", "new", "abstract",
            "asserts", "is", "any", "unknown", "never", "undefined", "null", "void", "this", "true", "false"}
    text = NUMBER.sub("0", text)
    text = IDENT.sub(lambda m: m.group(0) if m.group(0) in keep else "_", text)
    return SPACES.sub(" ", text)


def normalize(line):
    line = line[1:] if line[:1] in "+-" else line
    line = POSITION.sub("FILE(L,C)", line.strip())
    line = RELATED.sub("FILE:L:C", line)
    line = QUOTED.sub(lambda m: "'" + skeleton(m.group(1) if m.group(1) is not None else m.group(2)) + "'", line)
    # squiggle lines: only their shape matters
    if set(line) <= {"~", " "}:
        return "~" * min(len(line.strip()), 1) + ("..." if len(line.strip()) > 1 else "")
    return line


def normalize_types_line(line):
    """`>source text : type-or-Symbol(...)` from a .types/.symbols diff: the source text is dropped, the type keeps
    its skeleton, and `Decl(file, line, col)` positions are placeholders."""
    line = line[1:] if line[:1] in "+-" else line
    if line.startswith(">") and " : " in line:
        _, _, rhs = line.partition(" : ")
        rhs = re.sub(r"Decl\([^)]*\)", "Decl(F)", rhs)
        return "> : " + skeleton(rhs)
    return skeleton(line.strip())


def first_pair(diff_path):
    """(expected, actual) of the first hunk's first -/+ lines; either may be None (pure addition/removal)."""
    minus = plus = None
    in_hunk = False
    with open(diff_path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("@@"):
                if in_hunk and (minus or plus):
                    break
                in_hunk = True
                continue
            if not in_hunk or line.startswith(("---", "+++")):
                continue
            if line.startswith("-") and minus is None:
                minus = line
            elif line.startswith("+") and plus is None:
                plus = line
            if minus is not None and plus is not None:
                break
    return minus, plus


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--class", dest="klass", default=None)
    ap.add_argument("--baseline", default="errors", choices=["errors", "types", "symbols"])
    ap.add_argument("--results", default=os.path.join(ROOT, "target", "test-results"))
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--examples", type=int, default=3)
    args = ap.parse_args()
    extra = args.baseline != "errors"
    if args.klass is None:
        args.klass = "fail" if extra else "codes"

    list_name = f"{args.baseline}-{args.klass}" if extra else args.klass
    diff_suffix = f".{args.baseline}.diff" if extra else ".diff"
    names_file = os.path.join(args.results, list_name + ".txt")
    if not os.path.exists(names_file):
        sys.exit(f"no {names_file}; run `tsrs-test run` first")
    with open(names_file) as f:
        ids = [l.strip() for l in f if l.strip()]

    clusters = collections.defaultdict(list)
    for test_id in ids:
        diff_path = os.path.join(args.results, test_id + diff_suffix)
        if not os.path.exists(diff_path):
            clusters[("<no diff file>", "")].append(test_id)
            continue
        minus, plus = first_pair(diff_path)
        norm = normalize_types_line if extra else normalize
        key = (norm(minus) if minus else "<none>", norm(plus) if plus else "<none>")
        clusters[key].append(test_id)

    ranked = sorted(clusters.items(), key=lambda kv: (-len(kv[1]), kv[0]))
    print(f"{len(ids)} tests in class '{list_name}', {len(clusters)} clusters\n")
    for (exp, act), tests in ranked[: args.top]:
        print(f"{len(tests):5d}  - {exp}")
        print(f"       + {act}")
        print(f"       e.g. {', '.join(tests[: args.examples])}")
        print()


if __name__ == "__main__":
    main()
