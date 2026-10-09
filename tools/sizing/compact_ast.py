#!/usr/bin/env python3
"""TOOL (branch notes/compact-ast-sizing): turns the TSRS_AST_SIZING rows into the tables of
notes/mem-compact-ast-sizing.md.

    python3 tools/sizing/compact_ast.py --dir <dir with <project>.tsv and <project>.split.err> \
        --bench bench/results/2026-10-08-55c2d9ce5c41.json [--projects a,b,...]

Bytes are arena bytes as the walk counted them (crates/tsrs_ast/src/sizing.rs). "post" rows (the tree alive at the end
of the type-check pass, freed check leaves excluded) are the base for every saving; "pre" adds the leaves.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import defaultdict
from pathlib import Path

MB = 1e6
MIB = 1 << 20
GIB = 1 << 30

PROJECTS = ["t3code-server", "supabase-studio", "mikro-orm", "cal-diy", "formbricks-web", "vscode", "webpack", "drizzle-orm"]
SHORT = {"t3code-server": "t3code", "supabase-studio": "supabase", "mikro-orm": "mikro-orm", "cal-diy": "cal-diy",
         "formbricks-web": "formbricks", "vscode": "vscode", "webpack": "webpack", "drizzle-orm": "drizzle"}

# Identifier roles grouped the way the note reports them.
GROUPS = {
    "expression": ["expr", "prop_access_expr", "shorthand_name", "jsx_tag_name"],
    "declaration name": ["decl_name"],
    "property-access name": ["prop_access_name"],
    "member declaration name": ["member_decl_name"],
    "object/JSX/binding property name": ["object_prop_name", "jsx_attr_name", "binding_prop_name"],
    "type-reference name": ["type_ref_name", "heritage_name", "type_query_name"],
    "import/export specifier": ["import_export_specifier"],
    "import binding": ["import_binding"],
    "JSDoc": ["jsdoc"],
    "other": ["qualified_other", "label", "meta_property_name", "type_predicate_param"],
}
B_CORE = ["prop_access_name", "member_decl_name", "import_export_specifier"]
B_EXT = B_CORE + ["object_prop_name", "jsx_attr_name", "binding_prop_name"]

LITERAL_KINDS = ["StringLiteral", "NumericLiteral", "BigIntLiteral", "RegularExpressionLiteral", "NoSubstitutionTemplateLiteral"]
TEMPLATE_KINDS = ["TemplateHead", "TemplateMiddle", "TemplateTail"]

# Interner cost per distinct text: an id -> (pointer, length) entry (8 B, PackedStr) and a hash-table slot (4 B id +
# 1 control byte, at hashbrown's 7/8 load: ~6 B); the text itself is a borrow of the source text when it has no escape.
INTERN_ENTRY = 14


def load(path: Path):
    rows = defaultdict(list)
    for line in path.read_text().splitlines():
        f = line.split("\t")
        rows[(f[0], f[2])].append(f)
    return rows


def ints(xs):
    return [int(x) for x in xs]


class Proj:
    def __init__(self, name: str, d: Path, bench: dict):
        self.name = name
        r = load(d / f"{name}.tsv")
        self.r = r
        self.layout = {f[3]: ints(f[4:8]) for f in r[("pre", "layout")]}
        cell = bench["projects"][name]["wide"]
        self.peak = cell["tsrs"]["peak_rss_bytes"]
        self.bun = cell["bun"]["peak_rss_bytes"]
        self.split = {}
        err = (d / f"{name}.split.err").read_text()
        for m in re.finditer(r"tsrs mem split: (parse end|check end)\n  thread arenas: .*?, ([\d.]+) MiB used.*\n(?:.*\n)*?  heap \(allocator walk\): .*?, ([\d.]+) MiB live blocks", err):
            self.split[m.group(1)] = (float(m.group(2)) * MIB, float(m.group(3)) * MIB)

    def classes(self, phase):
        return ["src", "dts"] if phase == "post" else ["src", "dts", "leaf"]

    def sel(self, phase, cat, classes=None):
        classes = classes or self.classes(phase)
        return [f for f in self.r[(phase, cat)] if f[1] in classes]

    # --- totals -------------------------------------------------------------------------------------------
    def files(self, phase, cls):
        for f in self.r[(phase, "file")]:
            if f[1] == cls:
                return int(f[4]), int(f[5])
        return 0, 0

    def node_count_bytes(self, phase, classes=None):
        c = b = 0
        for f in self.sel(phase, "kind", classes):
            c += int(f[4])
            b += int(f[5])
        return c, b

    def lists(self, phase, classes=None):
        out = defaultdict(lambda: [0, 0, 0, 0])  # (listkind, bucket) -> count, struct, slice, elements
        for f in self.sel(phase, "list", classes):
            k = (f[3], f[4])
            for i, v in enumerate(ints(f[5:9])):
                out[k][i] += v
        return out

    def list_totals(self, phase, classes=None):
        L = self.lists(phase, classes)
        count = sum(v[0] for k, v in L.items())
        struct = sum(v[1] for v in L.values())
        sl = sum(v[2] for v in L.values())
        return count, struct, sl

    def idents(self, phase, roles=None, classes=None):
        # count, bytes, compact, stored_borrow, stored_copy, copy_text_bytes, text_bytes, with_flow, with_id
        t = [0] * 9
        for f in self.sel(phase, "ident", classes):
            if roles is None or f[3] in roles:
                for i, v in enumerate(ints(f[4:13])):
                    t[i] += v
        return t

    def literals(self, phase, kinds, classes=None):
        t = [0] * 6  # count, bytes, text, copied, copied_bytes, raw_copied_bytes
        for f in self.sel(phase, "literal", classes):
            if f[3] in kinds:
                for i, v in enumerate(ints(f[4:10])):
                    t[i] += v
        return t

    def text_copies(self, phase, classes=None):
        ident_copy = self.idents(phase, None, classes)[5]
        lit = 0
        for f in self.sel(phase, "literal", classes):
            lit += int(f[8]) + int(f[9])
        return ident_copy + lit

    def jsdoc_cache_bytes(self, phase, classes=None):
        b = 0
        for f in self.sel(phase, "misc", classes):
            if f[3] == "jsdoc_cache":
                b += int(f[5])
        return b

    def tree_bytes(self, phase, classes=None):
        _, nb = self.node_count_bytes(phase, classes)
        _, st, sl = self.list_totals(phase, classes)
        return nb + st + sl + self.text_copies(phase, classes) + self.jsdoc_cache_bytes(phase, classes)

    def kinds(self, phase, classes=None):
        out = defaultdict(lambda: [0, 0, 0, 0, 0])
        for f in self.sel(phase, "kind", classes):
            for i, v in enumerate(ints(f[4:9])):
                out[f[3]][i] += v
        return out

    def tags(self, phase, classes=None):
        out = defaultdict(lambda: [0, 0, 0])
        for f in self.sel(phase, "tag", classes):
            for i, v in enumerate(ints(f[4:7])):
                out[f[3]][i] += v
        return out

    def unique(self, phase, key):
        for f in self.r[(phase, "unique")]:
            if f[3] == key and f[1] == "-":
                return int(f[4]), int(f[5])
        c = b = 0
        for f in self.r[(phase, "unique")]:
            if f[3] == key and f[1] in self.classes(phase):
                c += int(f[4])
                b += int(f[5])
        return c, b

    # --- designs ------------------------------------------------------------------------------------------
    def design_a(self, phase="post"):
        """Identifier text as a 32-bit atom in the node (atom + flow node = 8 bytes, as today's word)."""
        t = self.idents(phase)
        gross = 8 * (t[3] + t[4]) + t[5]
        ucount, ubytes = self.unique(phase, "global_all")
        interner = INTERN_ENTRY * ucount  # texts borrowed from the source where possible
        return gross, interner

    def design_b(self, roles, phase="post"):
        t = self.idents(phase, roles)
        gross = t[1] + t[5]
        ucount, ubytes = self.unique(phase, "global_names")
        return gross, INTERN_ENTRY * ucount, t[0]

    def design_c(self, phase="post"):
        L = self.lists(phase)
        n = sum(v[0] for (lk, b), v in L.items() if lk != "lazy_unforced")
        empty = sum(v[0] for (lk, b), v in L.items() if lk != "lazy_unforced" and b == "0")
        one = sum(v[0] for (lk, b), v in L.items() if lk != "lazy_unforced" and b == "1")
        return 12 * n, 4 * n, n, empty, one

    def design_tokens(self, phase="post"):
        tags = self.tags(phase)
        tok = tags.get("Token", [0, 0, 0])
        kw = tags.get("KeywordTypeNode", [0, 0, 0])
        return tok[0], tok[1], kw[0], kw[1]

    def design_literal_flags(self, phase="post"):
        """TokenFlags out of string / numeric / bigint / regexp literals: 40 -> 32 bytes."""
        n = self.literals(phase, ["StringLiteral", "NumericLiteral", "BigIntLiteral", "RegularExpressionLiteral"])[0]
        return 8 * n, n

    def design_header20(self, phase="post"):
        """Node id out of the header (24 -> 20 bytes, 4-aligned): saves 8 bytes on each node whose data struct is
        4-aligned and 4 mod 8 bytes long (rare tails included); costs a side table for the nodes that have an id."""
        saved = 0
        tags = self.tags(phase)
        for tag, (count, bytes_, rare) in tags.items():
            lay = self.layout.get(tag)
            if not lay or tag == "SourceFile":
                continue
            alloc, data, align, rare_alloc = lay
            if align > 4 or data == 0:
                continue
            if (24 + data + 7) // 8 * 8 - (20 + data + 7) // 8 * 8 == 8:
                saved += 8 * (count - rare)
            if rare and rare_alloc:
                rare_part = rare_alloc - 24 - data  # tail bytes incl. padding
                # tail fields are 4-aligned handles: today roundup8(24 + data + tail), then roundup8(20 + data + tail)
                tail = rare_part
                # recompute the unpadded tail: try the smallest multiple of 4 that pads to rare_alloc
                for t4 in range(0, rare_part + 1, 4):
                    if (24 + data + t4 + 7) // 8 * 8 == rare_alloc:
                        tail = t4
                        break
                if (24 + data + tail + 7) // 8 * 8 - (20 + data + tail + 7) // 8 * 8 == 8:
                    saved += 8 * rare
        with_id = sum(v[2] for v in self.kinds(phase).values())
        return saved, 12 * with_id, with_id


def fmt_mb(b):
    return f"{b / MIB:.1f}"


def pct(b, peak):
    return f"{100 * b / peak:.1f}%"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=Path, required=True)
    ap.add_argument("--bench", type=Path, required=True)
    ap.add_argument("--projects", default=",".join(PROJECTS))
    ap.add_argument("--note", action="store_true", help="print the note's tables only")
    args = ap.parse_args()
    bench = json.loads(args.bench.read_text())
    projs = [Proj(n, args.dir, bench) for n in args.projects.split(",") if (args.dir / f"{n}.tsv").exists()]
    if args.note:
        note_tables(projs)
        return
    names = [SHORT[p.name] for p in projs]

    def table(header, rows):
        print("| " + " | ".join(header) + " |")
        print("| " + " | ".join(["---"] + ["---:"] * (len(header) - 1)) + " |")
        for r in rows:
            print("| " + " | ".join(str(x) for x in r) + " |")
        print()

    print("## Peaks (bench wide mode, 16 vCPU, tsrs 8 checkers)\n")
    table(["project", "tsrs peak MiB", "bun peak MiB", "gap MiB", "gap % of tsrs"],
          [[SHORT[p.name], f"{p.peak / MIB:.0f}", f"{p.bun / MIB:.0f}", f"{(p.peak - p.bun) / MIB:.0f}",
            pct(p.peak - p.bun, p.peak)] for p in projs])

    print("## Program and tree (post = alive at the end of the pass; pre = before the checkers, leaves included)\n")
    rows = []
    for p in projs:
        fs, ts = p.files("post", "src")
        fd, td = p.files("post", "dts")
        fl, tl = p.files("pre", "leaf")
        nc, nb = p.node_count_bytes("post")
        lc, lst, lsl = p.list_totals("post")
        tb = p.tree_bytes("post")
        tb_pre = p.tree_bytes("pre")
        pe = p.split.get("parse end", (0, 0))
        ce = p.split.get("check end", (0, 0))
        rows.append([SHORT[p.name], f"{fs}/{fd}/{fl}", fmt_mb(ts + td), f"{nc / 1e6:.2f}M", fmt_mb(nb), f"{lc / 1e6:.2f}M",
                     fmt_mb(lst + lsl), fmt_mb(tb), pct(tb, p.peak), fmt_mb(tb_pre), fmt_mb(pe[0]), fmt_mb(pe[1]), fmt_mb(ce[0]), fmt_mb(ce[1])])
    table(["project", "files src/dts/leaf", "text MiB (src+dts)", "nodes", "node MiB", "lists", "list MiB (struct+slice)",
           "tree MiB (post)", "tree % of peak", "tree MiB pre (all files)", "arena MiB parse end", "heap MiB parse end",
           "arena MiB check end", "heap MiB check end"], rows)

    print("## Identifiers by role (post)\n")
    hdr = ["role"] + [f"{n} count" for n in names]
    rows = []
    for g, roles in GROUPS.items():
        rows.append([g] + [f"{p.idents('post', roles)[0] / 1e3:.0f}k / {fmt_mb(p.idents('post', roles)[1])}" for p in projs])
    tot = [p.idents("post") for p in projs]
    rows.append(["all identifiers"] + [f"{t[0] / 1e3:.0f}k / {fmt_mb(t[1])}" for t in tot])
    rows.append(["  compact (text from source)"] + [f"{t[2] / 1e3:.0f}k" for t in tot])
    rows.append(["  stored text, borrowed"] + [f"{t[3] / 1e3:.0f}k" for t in tot])
    rows.append(["  stored text, copied (text MB)"] + [f"{t[4] / 1e3:.0f}k ({fmt_mb(t[5])})" for t in tot])
    rows.append(["  with a flow node"] + [f"{t[7] / 1e3:.0f}k" for t in tot])
    rows.append(["  with a node id"] + [f"{t[8] / 1e3:.0f}k" for t in tot])
    rows.append(["identifiers % of tree"] + [pct(t[1], p.tree_bytes('post')) for t, p in zip(tot, projs)])
    table(hdr, rows)

    print("## Distinct identifier texts\n")
    rows = []
    for p in projs:
        ga = p.unique("post", "global_all")
        gn = p.unique("post", "global_names")
        pf = p.unique("post", "per_file_all")
        rows.append([SHORT[p.name], f"{ga[0] / 1e3:.0f}k ({fmt_mb(ga[1])})", f"{gn[0] / 1e3:.0f}k ({fmt_mb(gn[1])})",
                     f"{pf[0] / 1e3:.0f}k ({fmt_mb(pf[1])})", f"{p.idents('post')[0] / 1e3:.0f}k"])
    table(["project", "distinct, program", "distinct B-core names, program", "distinct per file, summed", "identifiers"], rows)

    print("## Literals (post)\n")
    rows = []
    for k in LITERAL_KINDS + TEMPLATE_KINDS + ["PrivateIdentifier", "JsxText"]:
        rows.append([k] + [f"{p.literals('post', [k])[0] / 1e3:.0f}k / {fmt_mb(p.literals('post', [k])[1])} / copied {fmt_mb(p.literals('post', [k])[4] + p.literals('post', [k])[5])}" for p in projs])
    table(["kind"] + names, rows)

    print("## Lists by length (post; count / MiB struct+slice)\n")
    rows = []
    for lk in ["plain", "modifiers", "lazy_forced", "lazy_unforced"]:
        for b in ["0", "1", "2-3", "4-8", ">8"]:
            vals = [p.lists("post").get((lk, b), [0, 0, 0, 0]) for p in projs]
            if sum(v[0] for v in vals) == 0:
                continue
            rows.append([f"{lk} {b}"] + [f"{v[0] / 1e3:.0f}k / {fmt_mb(v[1] + v[2])}" for v in vals])
    table(["list"] + names, rows)

    print("## Tokens and keyword nodes (post)\n")
    rows = []
    for p in projs:
        tc, tb, kc, kb = p.design_tokens()
        k = p.kinds("post")
        mods = 0
        rows.append([SHORT[p.name], f"{tc / 1e3:.0f}k / {fmt_mb(tb)}", f"{kc / 1e3:.0f}k / {fmt_mb(kb)}"])
    table(["project", "Token nodes", "KeywordTypeNode"], rows)

    print("## Designs (post bytes; saving MB and % of the Linux 8-checker peak; flips = saving > gap to bun)\n")
    rows = []
    for p in projs:
        gap = p.peak - p.bun
        a_gross, a_cost = p.design_a()
        bc_gross, bc_cost, bc_n = p.design_b(B_CORE)
        be_gross, be_cost, be_n = p.design_b(B_EXT)
        c_hi, c_lo, c_n, c_empty, c_one = p.design_c()
        lf, lf_n = p.design_literal_flags()
        h20, h20_cost, with_id = p.design_header20()
        tc, tb, kc, kb = p.design_tokens()

        def cell(b):
            flip = "yes" if b > gap else "no"
            return f"{fmt_mb(b)} ({pct(b, p.peak)}, {flip})"

        rows.append([SHORT[p.name], fmt_mb(gap), cell(a_gross) + f" - interner {fmt_mb(a_cost)}",
                     cell(bc_gross) + f" - interner {fmt_mb(bc_cost)}", cell(be_gross), cell(c_hi), cell(c_lo),
                     cell(lf), cell(h20) + f" - ids {fmt_mb(h20_cost)}", cell(tb)])
    table(["project", "gap MiB", "A atom in node", "B name-less (core)", "B ext", "C lists 12 B", "C lists 4 B (loc kept)",
           "D literal flags", "D header 20 B", "D all tokens"], rows)

    print("## All designs together (upper bound; post)\n")
    rows = []
    for p in projs:
        gap = p.peak - p.bun
        bc_gross, _, _ = p.design_b(B_CORE)
        c_hi, *_ = p.design_c()
        lf, _ = p.design_literal_flags()
        tot = bc_gross + c_hi + lf
        rows.append([SHORT[p.name], fmt_mb(tot), pct(tot, p.peak), fmt_mb(gap), "yes" if tot > gap else "no", fmt_mb(p.tree_bytes("post")), pct(tot, p.tree_bytes("post"))])
    table(["project", "B core + C 12 B + literal flags MiB", "% of peak", "gap MiB", "flips", "tree MiB", "% of tree"], rows)

    print("## Counts behind the designs (post)\n")
    rows = []
    for p in projs:
        bc = p.idents("post", B_CORE)
        pa = p.idents("post", ["prop_access_name"])
        md = p.idents("post", ["member_decl_name"])
        sp = p.idents("post", ["import_export_specifier"])
        c_hi, c_lo, c_n, c_empty, c_one = p.design_c()
        h20, h20_cost, with_id = p.design_header20()
        nc, _ = p.node_count_bytes("post")
        rows.append([SHORT[p.name], f"{pa[0] / 1e3:.0f}k", f"{md[0] / 1e3:.0f}k", f"{sp[0] / 1e3:.0f}k", f"{c_n / 1e3:.0f}k",
                     f"{c_empty / 1e3:.0f}k ({pct(c_empty, c_n)})", f"{c_one / 1e3:.0f}k ({pct(c_one, c_n)})",
                     f"{with_id / 1e3:.0f}k ({pct(with_id, nc)})"])
    table(["project", "prop-access names", "member names", "specifier names", "lists", "empty", "one element", "nodes with an id"], rows)

    print("## Top node kinds by bytes (post, vscode and t3code)\n")
    for p in projs:
        if p.name not in ("vscode", "t3code-server"):
            continue
        k = p.kinds("post")
        tops = sorted(k.items(), key=lambda kv: -kv[1][1])[:20]
        table([f"{SHORT[p.name]} kind", "count", "MiB", "with id"], [[n, f"{v[0] / 1e3:.0f}k", fmt_mb(v[1]), f"{v[2] / 1e3:.0f}k"] for n, v in tops])



def note_tables(projs):
    """The tables of notes/mem-compact-ast-sizing.md, in MiB (2^20 bytes)."""
    M = lambda b: b / MIB

    def table(header, rows, align=None):
        print("| " + " | ".join(header) + " |")
        print("| " + " | ".join(["---"] + ["---:"] * (len(header) - 1)) + " |")
        for r in rows:
            print("| " + " | ".join(str(x) for x in r) + " |")
        print()

    print("### peaks\n")
    table(["project", "tsrs peak", "bun peak", "gap", "gap / tsrs peak"],
          [[p.name, f"{M(p.peak):,.0f} MiB", f"{M(p.bun):,.0f} MiB", f"{M(p.peak - p.bun):,.0f} MiB", f"{100 * (p.peak - p.bun) / p.peak:.1f}%"] for p in projs])

    print("### tree\n")
    rows = []
    for p in projs:
        fs, _ = p.files("post", "src")
        fd, _ = p.files("post", "dts")
        fl, _ = p.files("pre", "leaf")
        nc, nb = p.node_count_bytes("post")
        lc, lst, lsl = p.list_totals("post")
        tree = p.tree_bytes("post")
        leaf = p.tree_bytes("pre", ["leaf"])
        a, h = p.split["parse end"]
        rows.append([SHORT[p.name], f"{fs:,} / {fd:,} / {fl:,}", f"{nc / 1e6:.2f}M", f"{M(tree):.1f}", f"{100 * tree / p.peak:.1f}%",
                     f"{100 * 24 * nc / tree:.0f}%", f"{100 * p.idents('post')[1] / tree:.0f}%", f"{100 * (lst + lsl) / tree:.0f}%",
                     f"{M(leaf):.1f}", f"{M(a):.0f} + {M(h):.0f}", f"{100 * (a + h) / p.peak:.0f}%"])
    table(["project", "files: source / declaration / freed leaves", "nodes at the peak", "tree at the peak, MiB",
           "tree / peak", "headers / tree", "identifiers / tree", "lists / tree", "freed leaf trees, MiB",
           "program at parse end, arena + heap MiB (Mac)", "program / peak"], rows)

    print("### identifiers\n")
    groups = [("expression (incl. `a` of `a.b`, shorthand, JSX tags)", GROUPS["expression"]),
              ("declaration name", GROUPS["declaration name"]),
              ("property-access name (`b` of `a.b`)", GROUPS["property-access name"]),
              ("member declaration name", GROUPS["member declaration name"]),
              ("object literal / JSX attribute / binding property name", GROUPS["object/JSX/binding property name"]),
              ("type-reference name (incl. qualified parts, heritage, typeof)", GROUPS["type-reference name"]),
              ("import/export specifier name", GROUPS["import/export specifier"]),
              ("import binding (default, namespace, import =)", GROUPS["import binding"]),
              ("JSDoc", GROUPS["JSDoc"]),
              ("other (labels, meta properties, predicates)", GROUPS["other"])]
    rows = []
    for label, roles in groups:
        rows.append([label] + [f"{p.idents('post', roles)[0] / 1e3:,.0f}k / {M(p.idents('post', roles)[1]):.1f}" for p in projs])
    tot = [p.idents("post") for p in projs]
    rows.append(["**all identifiers** (count / MiB)"] + [f"{t[0] / 1e3:,.0f}k / {M(t[1]):.1f}" for t in tot])
    rows.append(["text stored in the node (not derived from the source)"] + [f"{(t[3] + t[4]) / 1e3:,.0f}k ({100 * (t[3] + t[4]) / t[0]:.1f}%)" for t in tot])
    rows.append(["text copied out of the source, MiB"] + [f"{M(t[5]):.2f}" for t in tot])
    rows.append(["with a flow node"] + [f"{100 * t[7] / t[0]:.0f}%" for t in tot])
    rows.append(["with a node id (checker links)"] + [f"{100 * t[8] / t[0]:.0f}%" for t in tot])
    rows.append(["distinct texts in the program"] + [f"{p.unique('post', 'global_all')[0] / 1e3:,.0f}k" for p in projs])
    rows.append(["distinct texts of the three name kinds"] + [f"{p.unique('post', 'global_names')[0] / 1e3:,.0f}k" for p in projs])
    table(["role (post)"] + [SHORT[p.name] for p in projs], rows)

    print("### literals, tokens, lists\n")
    rows = []
    def lit(p, kinds):
        t = p.literals("post", kinds)
        return f"{t[0] / 1e3:,.0f}k / {M(t[1]):.1f}"
    rows.append(["string literals (40 B)"] + [lit(p, ["StringLiteral"]) for p in projs])
    rows.append(["numeric, bigint, regexp literals (40 B)"] + [lit(p, ["NumericLiteral", "BigIntLiteral", "RegularExpressionLiteral"]) for p in projs])
    rows.append(["template parts and no-substitution templates (64-72 B)"] + [lit(p, TEMPLATE_KINDS + ["NoSubstitutionTemplateLiteral"]) for p in projs])
    rows.append(["literal text copied out of the source, MiB"] + [f"{M(sum(int(f[8]) + int(f[9]) for f in p.sel('post', 'literal'))):.2f}" for p in projs])
    rows.append(["token nodes (24 B: modifiers, `?`, `=>`, operators, ...)"] + [f"{p.design_tokens()[0] / 1e3:,.0f}k / {M(p.design_tokens()[1]):.1f}" for p in projs])
    rows.append(["keyword type nodes (24 B: `string`, `number`, ...)"] + [f"{p.design_tokens()[2] / 1e3:,.0f}k / {M(p.design_tokens()[3]):.1f}" for p in projs])
    for b, label in [("0", "lists of 0"), ("1", "lists of 1"), ("2-3", "lists of 2-3"), ("4-8", "lists of 4-8"), (">8", "lists of more than 8")]:
        vals = []
        for p in projs:
            L = p.lists("post")
            c = sum(v[0] for (lk, bb), v in L.items() if bb == b and lk != "lazy_unforced")
            st = sum(v[1] for (lk, bb), v in L.items() if bb == b and lk != "lazy_unforced")
            sl = sum(v[2] for (lk, bb), v in L.items() if bb == b and lk != "lazy_unforced")
            vals.append(f"{c / 1e3:,.0f}k / {M(st):.1f} + {M(sl):.1f}")
        rows.append([f"{label} (count / MiB struct + elements)"] + vals)
    rows.append(["of which modifier lists (24 B)"] + [f"{sum(v[0] for (lk, b), v in p.lists('post').items() if lk == 'modifiers') / 1e3:,.0f}k" for p in projs])
    rows.append(["unforced lazy member lists (88 B records)"] + [f"{sum(v[0] for (lk, b), v in p.lists('post').items() if lk == 'lazy_unforced') / 1e3:,.0f}k / {M(sum(v[1] for (lk, b), v in p.lists('post').items() if lk == 'lazy_unforced')):.1f}" for p in projs])
    rows.append(["SourceFile nodes (736 B)"] + [f"{p.tags('post').get('SourceFile', [0, 0, 0])[0]:,} / {M(p.tags('post').get('SourceFile', [0, 0, 0])[1]):.1f}" for p in projs])
    table(["post"] + [SHORT[p.name] for p in projs], rows)

    print("### designs\n")
    rows = []
    for p in projs:
        gap = p.peak - p.bun
        pk = lambda b: f"{M(b):.1f} ({100 * b / p.peak:.1f}%)"
        a_g, a_c = p.design_a()
        b_g, b_c, _ = p.design_b(B_CORE)
        e_g, _, _ = p.design_b(B_EXT)
        c_hi, c_lo, *_ = p.design_c()
        lf, _ = p.design_literal_flags()
        tc, tb, kc, kb = p.design_tokens()
        h, hc, _ = p.design_header20()
        tot = b_g + c_hi + lf
        rows.append([SHORT[p.name], f"{M(gap):.0f}", pk(a_g - a_c), pk(b_g - b_c), pk(e_g), pk(c_hi), pk(c_lo), pk(lf), pk(tb),
                     pk(h - hc), pk(tot), "yes" if tot > gap else "no"])
    table(["project", "gap MiB", "A: atom in the node, net of interner", "B: name-less (3 kinds), net of interner",
           "B ext: + object/JSX/binding names", "C: lists as ranges, positions derived (12 B/list)",
           "C: positions kept (4 B/list)", "D: literal TokenFlags out", "D: every token node gone",
           "D: 20-byte header, net of id table", "B + C (12 B) + literal flags", "flips"], rows)


if __name__ == "__main__":
    main()
