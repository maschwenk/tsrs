#!/usr/bin/env python3
"""Completeness check: every exported name in Go's lsp_generated.go has its Rust counterpart in
crates/tsrs_lsproto/src/lsp_generated.rs (naming rules in docs/LSP.md, "Protocol types and JSON").

Checked: types (structures, unions, literal types, enumerations, type aliases, response types), struct and
union fields, enumeration constants (`SymbolKindFile` -> `SymbolKind::File`), method constants
(`MethodTextDocumentHover` -> `Method::TextDocumentHover`), request/notification infos
(`TextDocumentHoverInfo` -> `TEXT_DOCUMENT_HOVER_INFO`), and methods (`TextDocumentURI`,
`TextDocumentPosition`, `GetLocation(s)`, `Compare`, `String`, `Error`, `Resolve`/`resolve`, the JSON
codecs). Prints what is missing; exits 1 if anything is.

Usage: python3 tools/gen-lsproto/check.py
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GO = os.path.join(ROOT, "ts-ref/tsc/internal/lsp/lsproto/lsp_generated.go")
RS = os.path.join(ROOT, "crates/tsrs_lsproto/src/lsp_generated.rs")

KEYWORDS = {"type", "ref", "match", "loop", "mod", "in", "as", "fn", "impl", "move", "self", "static", "struct",
            "super", "trait", "true", "false", "use", "where", "while", "async", "await", "dyn", "box", "do", "final",
            "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen", "const", "crate",
            "else", "enum", "extern", "for", "if", "let", "pub", "return", "unsafe", "abstract", "become", "break",
            "continue"}


def snake(s):
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", s)
    s = s.lower()
    return s + "_" if s in KEYWORDS else s


def main():
    go = open(GO).read()
    rs = open(RS).read()
    missing = []
    checked = 0

    def need(cond, what):
        nonlocal checked
        checked += 1
        if not cond:
            missing.append(what)

    # Rust items
    rs_structs = {}  # name -> body
    for m in re.finditer(r"^pub struct (\w+) \{\n(.*?)^\}", rs, re.M | re.S):
        rs_structs[m.group(1)] = m.group(2)
    rs_tuple = set(re.findall(r"^pub struct (\w+)\(", rs, re.M))
    rs_unit = set(re.findall(r"^pub struct (\w+);", rs, re.M))
    rs_alias = set(re.findall(r"^pub type (\w+) =", rs, re.M))
    rs_consts = {}  # type -> set of const names
    for m in re.finditer(r"^impl (\w+) \{\n(.*?)^\}", rs, re.M | re.S):
        for c in re.findall(r"^    pub const (\w+): (\w+) =", m.group(2), re.M):
            rs_consts.setdefault(c[1], set()).add(c[0])
    rs_top_consts = set(re.findall(r"^pub const (\w+):", rs, re.M))
    rs_impls = set(re.findall(r"^impl ([\w:]+) for (\w+) \{", rs, re.M))
    rs_methods = {}  # type -> set of fn names
    for m in re.finditer(r"^impl (\w+) \{\n(.*?)^\}", rs, re.M | re.S):
        for f in re.findall(r"^    (?:pub(?:\(crate\))? )?fn (\w+)", m.group(2), re.M):
            rs_methods.setdefault(m.group(1), set()).add(f)
    rs_types = set(rs_structs) | rs_tuple | rs_unit | rs_alias

    # Go structs (structures, unions, literal types, resolved types) and their fields
    enum_types = {}
    for m in re.finditer(r"^type (\w+) (int32|uint32|string)$", go, re.M):
        enum_types[m.group(1)] = m.group(2)
    for m in re.finditer(r"^type (\w+) struct \{\n(.*?)^\}", go, re.M | re.S):
        name, body = m.group(1), m.group(2)
        need(name in rs_types, f"type {name}")
        rs_body = rs_structs.get(name, "")
        for f in re.findall(r"^\t(\w+) ", body, re.M):
            need(re.search(r"^    pub " + re.escape(snake(f)) + r":", rs_body, re.M), f"field {name}.{f} ({snake(f)})")
    for m in re.finditer(r"^type (\w+) struct\{\}$", go, re.M):
        need(m.group(1) in rs_types, f"type {m.group(1)}")
    for name in enum_types:
        need(name in rs_tuple, f"enum {name}")
    for m in re.finditer(r"^type (\w+) = ", go, re.M):
        need(m.group(1) in rs_alias, f"alias {m.group(1)}")

    # Constants
    for block in re.findall(r"^const \(\n(.*?)^\)", go, re.M | re.S):
        for name, typ in re.findall(r"^\t(\w+)\s+(\w+) = ", block, re.M):
            if typ == "Method":
                suffix = name[len("Method"):]
                need(suffix in rs_consts.get("Method", set()), f"const {name} -> Method::{suffix}")
            elif typ in enum_types:
                assert name.startswith(typ), name
                suffix = name[len(typ):]
                need(suffix in rs_consts.get(typ, set()), f"const {name} -> {typ}::{suffix}")
            else:
                missing.append(f"unexpected const {name} {typ}")

    # Request / notification infos
    for name in re.findall(r"^var (\w+Info) = (?:Request|Notification)Info", go, re.M):
        rust = snake(name).upper()
        need(rust in rs_top_consts, f"var {name} -> {rust}")

    # Methods
    for m in re.finditer(r"^func \((\w+) \*?(\w+)\) (\w+)\(", go, re.M):
        typ, meth = m.group(2), m.group(3)
        if meth in ("MarshalJSONTo", "UnmarshalJSONFrom"):
            need(("Json", typ) in rs_impls, f"{typ}.{meth} -> impl Json for {typ}")
        elif meth == "TextDocumentURI":
            need(("HasTextDocumentURI", typ) in rs_impls, f"{typ}.{meth}")
        elif meth == "TextDocumentPosition":
            need(("HasTextDocumentPosition", typ) in rs_impls, f"{typ}.{meth}")
        elif meth == "GetLocation":
            need(("HasLocation", typ) in rs_impls, f"{typ}.{meth}")
        elif meth == "GetLocations":
            need(("HasLocations", typ) in rs_impls, f"{typ}.{meth}")
        elif meth == "Error":
            need(("std::error::Error", typ) in rs_impls, f"{typ}.{meth}")
        elif meth == "String":
            need("string" in rs_methods.get(typ, set()) and ("fmt::Display", typ) in rs_impls, f"{typ}.{meth}")
        else:
            need(snake(meth) in rs_methods.get(typ, set()), f"{typ}.{meth} -> {snake(meth)}")

    for line in missing:
        print("missing:", line)
    print(f"checked {checked} names, {len(missing)} missing")
    sys.exit(1 if missing else 0)


if __name__ == "__main__":
    main()
