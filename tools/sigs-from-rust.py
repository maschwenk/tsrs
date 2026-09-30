#!/usr/bin/env python3
"""Regenerate docs/sigs/{checker,pseudochecker,modulespecifiers}.txt from the Rust sources of tsrs_checker,
tsrs_pseudochecker and tsrs_modulespecifiers.

Scans crates/<crate>/src/*.rs for free `fn` items and `fn` items of inherent `impl` blocks (trait impls and
functions nested in function bodies are skipped) and writes one line per function:

    rust_name | signature | origin | receiver

- signature: the declaration up to (not including) the body, whitespace collapsed.
- origin: the nearest `// file.go:LINE` marker in the comment block directly above the function; functions without
  one (hand-written data-model helpers) get `rust:<file>:<line>`.
- receiver: the `impl` type (`Checker`, `Relater`, ...) or `-` for free functions.

Usage: tools/sigs-from-rust.py [repo-root]   (default: the parent of this script's directory)
"""

import os
import re
import sys

ORIGIN_RE = re.compile(r"//\s*([a-z_]+\.go:\d+)")


def strip_code(src):
    """Replace comments, strings and char literals with spaces (keeping newlines and offsets)."""
    out = list(src)
    i, n = 0, len(src)

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and re.match(r'r#*"', src[i:i + 10]) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            m = re.match(r'r(#*)"', src[i:])
            close = '"' + m.group(1)
            j = src.find(close, i + len(m.group(0)))
            j = n if j < 0 else j + len(close)
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(i, j + 1)
            i = j + 1
        elif c == "'":
            if i + 1 < n and src[i + 1] == "\\":
                j = src.find("'", i + 2)
                blank(i, j + 1)
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'":
                blank(i, i + 3)
                i += 3
            else:
                i += 1  # lifetime
        else:
            i += 1
    return "".join(out)


def impl_receiver(header):
    """`impl<T: X> Foo<T>` -> 'Foo'; trait impls (`impl X for Y`) -> None."""
    h = re.sub(r"\s+", " ", header).strip()
    h = h[len("impl"):].strip()
    if h.startswith("<"):
        depth = 0
        for k, ch in enumerate(h):
            if ch == "<":
                depth += 1
            elif ch == ">":
                depth -= 1
                if depth == 0:
                    h = h[k + 1:].strip()
                    break
    if re.search(r"\bfor\b", h):
        return None
    m = re.match(r"([A-Za-z_][A-Za-z0-9_:]*)", h)
    return m.group(1).split("::")[-1] if m else None


def scan_file(path, rel):
    src = open(path, encoding="utf-8").read()
    code = strip_code(src)
    lines = src.split("\n")
    line_starts = [0]
    for ln in lines:
        line_starts.append(line_starts[-1] + len(ln) + 1)

    def line_of(off):
        lo, hi = 0, len(line_starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if line_starts[mid] <= off:
                lo = mid
            else:
                hi = mid - 1
        return lo  # 0-based

    results = []
    # stack of scopes: ('impl', receiver|None) | ('fn',) | ('other',) | ('test',)
    stack = []
    i, n = 0, len(code)
    item_re = re.compile(
        r"(?:#\[cfg\(test\)\]\s*)?(?:pub(?:\([a-z]+\))?\s+)?(?:const\s+|unsafe\s+|extern\s+\"C\"\s+)*(impl\b|fn\s+([A-Za-z_][A-Za-z0-9_]*)|mod\s+([A-Za-z_]\w*)|macro_rules!)"
    )
    while i < n:
        c = code[i]
        if c == "{":
            stack.append(("other",))
            i += 1
            continue
        if c == "}":
            if stack:
                stack.pop()
            i += 1
            continue
        if (c.isalpha() or c in "#_") and (i == 0 or not (code[i - 1].isalnum() or code[i - 1] == "_")):
            m = item_re.match(code, i)
            if m:
                # find the end of the item header: the first `{` or `;` at bracket depth 0
                j, depth = m.end(), 0
                while j < n:
                    ch = code[j]
                    if ch in "([<" and not (ch == "<" and code[j - 1] == "-"):
                        depth += 1
                    elif ch in ")]>" and not (ch == ">" and code[j - 1] in "-="):
                        depth -= 1
                    elif depth <= 0 and ch in "{;":
                        break
                    j += 1
                header = code[m.start():j]
                kind = m.group(1)
                in_test = any(s[0] == "test" for s in stack)
                if kind.startswith("fn"):
                    name = m.group(2)
                    parent = stack[-1] if stack else None
                    if not in_test and (parent is None or (parent[0] == "impl" and parent[1] is not None)):
                        sig = re.sub(r"\s+", " ", src[m.start():j]).strip()
                        sig = re.sub(r"^#\[cfg\(test\)\]\s*", "", sig)
                        sig = sig.replace("( ", "(").replace(" )", ")").replace(", )", ")").replace(",)", ")")
                        # binding-mode `mut` on parameters is not part of the signature
                        sig = re.sub(r"([(,] ?)mut ([A-Za-z_]\w*):", r"\1\2:", sig)
                        fn_line = line_of(m.start())
                        origin = None
                        k = fn_line - 1
                        while k >= 0:
                            t = lines[k].strip()
                            if t.startswith("//") or t.startswith("#[") or t.startswith("*") or t.startswith("/*"):
                                mm = ORIGIN_RE.search(t)
                                if mm and origin is None:
                                    origin = mm.group(1)
                                k -= 1
                                continue
                            break
                        if origin is None:
                            origin = "rust:%s:%d" % (rel, fn_line + 1)
                        receiver = parent[1] if parent else "-"
                        results.append((name, sig, origin, receiver))
                    if j < n and code[j] == "{":
                        stack.append(("fn",))
                        i = j + 1
                        continue
                    i = j + 1
                    continue
                if kind == "impl":
                    if j < n and code[j] == "{":
                        stack.append(("impl", None if in_test else impl_receiver(header)))
                        i = j + 1
                        continue
                    i = j + 1
                    continue
                if kind.startswith("mod"):
                    if j < n and code[j] == "{":
                        is_test = "cfg(test)" in header or m.group(3) == "tests"
                        stack.append(("test",) if is_test else ("other",))
                        i = j + 1
                        continue
                    i = j + 1
                    continue
                if kind == "macro_rules!":
                    if j < n and code[j] == "{":
                        stack.append(("test",))  # skip macro bodies
                        i = j + 1
                        continue
                i = m.end()
                continue
        i += 1
    return results


CRATES = [
    ("tsrs_checker", "checker.txt"),
    ("tsrs_pseudochecker", "pseudochecker.txt"),
    ("tsrs_modulespecifiers", "modulespecifiers.txt"),
]


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    for crate, out_name in CRATES:
        src_dir = os.path.join(root, "crates", crate, "src")
        rows = []
        for fname in sorted(os.listdir(src_dir)):
            if fname.endswith(".rs") and not fname.endswith("_test.rs"):
                rows.extend(scan_file(os.path.join(src_dir, fname), fname))
        rows.sort(key=lambda r: (r[0], r[3], r[2]))
        out_path = os.path.join(root, "docs", "sigs", out_name)
        with open(out_path, "w", encoding="utf-8") as f:
            f.write("# rust_name | signature | origin | receiver  (generated from the Rust sources by tools/sigs-from-rust.py)\n")
            for r in rows:
                f.write(" | ".join(r) + "\n")
        print("wrote %d signatures to %s" % (len(rows), out_path))


if __name__ == "__main__":
    main()
