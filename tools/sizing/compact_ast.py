#!/usr/bin/env python3
"""TOOL: turns the TSRS_AST_SIZING rows into the tables of notes/mem-compact-ast-sizing.md.

    cargo build --release -p tsrs_cli --features ast-sizing
    (per project, from its root)  TSRS_AST_SIZING=<dir>/<project>.tsv tsrs -p <project> --noEmit --incremental false \
        --extendedDiagnostics --pretty false --checkers 8
    (optional, for the arena / heap columns)  TSRS_MEM_SPLIT=1 tsrs ... 2> <dir>/<project>.split.err
    python3 tools/sizing/compact_ast.py --dir <dir> --bench bench/results/2026-10-08-55c2d9ce5c41.json

Bytes are arena bytes as the walk counted them (crates/tsrs_ast/src/sizing.rs): node allocations (header, data
struct, rare tail, 8-byte rounding), 16-byte `NodeList`s, 24-byte `ModifierList`s, 4 bytes per list element, text
copied out of the source, JSDoc cache slices. "post" rows (the tree alive at the end of the type-check pass: every
file but the freed check leaves, with the lazy member lists and JSDoc the checkers forced) are the base for every
saving; "pre" leaf rows give the upper bound if every leaf tree were alive at the peak. MiB = 2^20 bytes.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import defaultdict
from pathlib import Path

MIB = 1 << 20

PROJECTS = ["t3code-server", "supabase-studio", "mikro-orm", "cal-diy", "formbricks-web", "vscode", "webpack", "drizzle-orm"]
SHORT = {"t3code-server": "t3code", "supabase-studio": "supabase", "mikro-orm": "mikro-orm", "cal-diy": "cal-diy",
         "formbricks-web": "formbricks", "vscode": "vscode", "webpack": "webpack", "drizzle-orm": "drizzle"}

# Identifier roles (sizing.rs `Role`) grouped the way the note reports them.
GROUPS = [
    ("expression (incl. `a` of `a.b`, shorthand, JSX tag)", ["expr", "prop_access_expr", "shorthand_name", "jsx_tag_name"]),
    ("declaration name", ["decl_name"]),
    ("property-access name (`b` of `a.b`)", ["prop_access_name"]),
    ("member declaration name", ["member_decl_name"]),
    ("object literal / JSX attribute / binding property name", ["object_prop_name", "jsx_attr_name", "binding_prop_name"]),
    ("type-reference name (incl. qualified parts, heritage, typeof)", ["type_ref_name", "heritage_name", "type_query_name"]),
    ("import/export specifier name", ["import_export_specifier"]),
    ("import binding (default, namespace, import =)", ["import_binding"]),
    ("JSDoc", ["jsdoc"]),
    ("other (labels, meta properties, predicates)", ["qualified_other", "label", "meta_property_name", "type_predicate_param"]),
]
B_CORE = ["prop_access_name", "member_decl_name", "import_export_specifier"]
B_EXT = B_CORE + ["object_prop_name", "jsx_attr_name", "binding_prop_name"]
# Every role whose identifiers the checker never keys a node link on (node id on < 1% of them, all eight projects).
B_MAX = B_EXT + ["decl_name", "type_ref_name", "import_binding", "qualified_other", "label", "meta_property_name",
                 "type_predicate_param", "jsdoc"]

# Interner cost per distinct text: an id -> text entry (8 B, PackedStr into the source) and a hash slot (4 B id + 1 control
# byte at hashbrown's 7/8 load, ~6 B). An estimate; the parse-time cost is separate (notes/mem-round2.md:203-212).
INTERN_ENTRY = 14
LITERALS_WITH_FLAGS = ["StringLiteral", "NumericLiteral", "BigIntLiteral", "RegularExpressionLiteral"]


def r8(n: int) -> int:
    return (n + 7) // 8 * 8


def list_fields(root: Path) -> dict[str, int]:
    """List fields (`P<NodeList>` / `P<ModifierList>`, plain or wrapped) per data struct and rare tail, embedded bases
    expanded, from crates/tsrs_ast/src/generated.rs; plus the rare tails' field counts as "<Struct>Rare#fields"."""
    src = (root / "crates/tsrs_ast/src/generated.rs").read_text()
    structs = {}
    for m in re.finditer(r"^pub struct (\w+) \{\n(.*?)^\}", src, re.S | re.M):
        structs[m.group(1)] = re.findall(r"^\s+pub (\w+): (.+?),\s*$", m.group(2), re.M)

    def count(name: str) -> int:
        k = 0
        for _, t in structs.get(name, []):
            words = re.sub(r"[^\w]", " ", t).split()
            if "NodeList" in words or "ModifierList" in words:
                k += 1
            elif t in structs:
                k += count(t)
        return k

    out = {n: count(n) for n in structs}
    out["SourceFile"] = 1  # hand-written (ast.rs): `statements`
    for n, fields in structs.items():
        if n.endswith("Rare"):
            out[n + "#fields"] = len(fields)
    return out


def load(path: Path):
    rows = defaultdict(list)
    for line in path.read_text().splitlines():
        f = line.split("\t")
        rows[(f[0], f[2])].append(f)
    return rows


def ints(xs):
    return [int(x) for x in xs]


class Proj:
    def __init__(self, name: str, d: Path, bench: dict, fields: dict[str, int]):
        self.name = name
        self.short = SHORT[name]
        r = load(d / f"{name}.tsv")
        self.r = r
        self.fields = fields
        self.layout = {f[3]: ints(f[4:8]) for f in r[("pre", "layout")]}  # alloc, data, align, rare alloc
        cell = bench["projects"][name]["wide"]
        self.peak = cell["tsrs"]["peak_rss_bytes"]
        self.bun = cell["bun"]["peak_rss_bytes"]
        self.gap = self.peak - self.bun
        self.split = {}
        err_path = d / f"{name}.split.err"
        if err_path.exists():
            err = err_path.read_text()
            pat = (r"tsrs mem split: (parse end|check end)\n  thread arenas: .*?, ([\d.]+) MiB used.*\n(?:.*\n)*?"
                   r"  heap \(allocator walk\): .*?, ([\d.]+) MiB live blocks")
            for m in re.finditer(pat, err):
                self.split[m.group(1)] = (float(m.group(2)) * MIB, float(m.group(3)) * MIB)

    # --- selection -------------------------------------------------------------------------------------------------
    @staticmethod
    def default_classes(phase):
        return ["src", "dts"] if phase == "post" else ["src", "dts", "leaf"]

    def sel(self, phase, cat, classes=None):
        classes = classes or self.default_classes(phase)
        return [f for f in self.r[(phase, cat)] if f[1] in classes]

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
        out = defaultdict(lambda: [0, 0, 0, 0])  # (list kind, bucket) -> count, struct bytes, slice bytes, elements
        for f in self.sel(phase, "list", classes):
            for i, v in enumerate(ints(f[5:9])):
                out[(f[3], f[4])][i] += v
        return out

    def list_totals(self, phase, classes=None, kinds=None):
        L = self.lists(phase, classes)
        keep = [v for (lk, _), v in L.items() if kinds is None or lk in kinds]
        return sum(v[0] for v in keep), sum(v[1] for v in keep), sum(v[2] for v in keep)

    def idents(self, phase, roles=None, classes=None):
        # count, bytes, compact, stored_borrow, stored_copy, copy_text_bytes, text_bytes, with_flow, with_id
        t = [0] * 9
        for f in self.sel(phase, "ident", classes):
            if roles is None or f[3] in roles:
                for i, v in enumerate(ints(f[4:13])):
                    t[i] += v
        return t

    def literals(self, phase, kinds, classes=None):
        t = [0] * 6  # count, bytes, text bytes, copied, copied bytes, raw copied bytes
        for f in self.sel(phase, "literal", classes):
            if f[3] in kinds:
                for i, v in enumerate(ints(f[4:10])):
                    t[i] += v
        return t

    def text_copies(self, phase, classes=None):
        lit = sum(int(f[8]) + int(f[9]) for f in self.sel(phase, "literal", classes))
        return self.idents(phase, None, classes)[5] + lit

    def jsdoc_cache_bytes(self, phase, classes=None):
        return sum(int(f[5]) for f in self.sel(phase, "misc", classes) if f[3] == "jsdoc_cache")

    def tree_bytes(self, phase, classes=None):
        _, nb = self.node_count_bytes(phase, classes)
        _, st, sl = self.list_totals(phase, classes)
        return nb + st + sl + self.text_copies(phase, classes) + self.jsdoc_cache_bytes(phase, classes)

    def kinds(self, phase, classes=None):
        out = defaultdict(lambda: [0, 0, 0, 0, 0, 0])  # count, bytes, with id, with rare, in JSDoc, nonzero flags
        for f in self.sel(phase, "kind", classes):
            for i, v in enumerate(ints(f[4:10])):
                out[f[3]][i] += v
        return out

    def tags(self, phase, classes=None):
        out = defaultdict(lambda: [0, 0, 0])  # count, bytes, with rare tail
        for f in self.sel(phase, "tag", classes):
            for i, v in enumerate(ints(f[4:7])):
                out[f[3]][i] += v
        return out

    def unique(self, phase, key):
        for f in self.r[(phase, "unique")]:
            if f[3] == key and f[1] == "-":
                return int(f[4]), int(f[5])
        return 0, 0

    # --- designs (bytes saved; positive = smaller) -------------------------------------------------------------------
    def design_a(self, phase="post", classes=None):
        """Identifier text as a 32-bit atom in the node: atom + flow handle = the 8-byte word of today, so a compact
        identifier stays 32 B; an identifier that stores its text (40 B) becomes 32 B. Interner: one entry per distinct
        text in the program."""
        t = self.idents(phase, None, classes)
        gross = 8 * (t[3] + t[4]) + t[5]
        return gross, INTERN_ENTRY * self.unique(phase, "global_all")[0]

    def design_b(self, roles, phase="post", classes=None, interner_key="global_names"):
        """Identifier nodes of `roles` removed; the parent's 4-byte name field holds the atom instead of the handle."""
        t = self.idents(phase, roles, classes)
        return t[1] + t[5], INTERN_ENTRY * self.unique(phase, interner_key)[0], t[0]

    def alloc_total(self, phase, classes, header20, list_bytes=None):
        """Arena bytes of every node with a data struct (Identifier excluded) under a layout: node id out of the header
        (20 B, data at offset 20 when it is 4-aligned) and/or `list_bytes(k)` bytes for a struct's k list fields instead
        of today's 4 * k (a rare tail's list fields get `list_bytes(1)` each)."""
        list_bytes = list_bytes or (lambda k: 4 * k)
        total = 0
        for tag, (count, _bytes, rare) in self.tags(phase, classes).items():
            lay = self.layout.get(tag)
            if not lay or tag == "Identifier":
                continue
            alloc, data, align, rare_alloc = lay
            if data == 0:  # payload-less: the header alone
                total += count * alloc
                continue
            off = 20 if header20 and align <= 4 else 24
            k = self.fields.get(tag, 0)
            data2 = data - 4 * k + (list_bytes(k) + 3) // 4 * 4
            plain = r8(off + data2) if align <= 8 else alloc + data2 - data
            total += (count - rare) * plain
            if rare:
                nf = self.fields.get(tag + "Rare#fields", 0)
                kr = self.fields.get(tag + "Rare", 0)
                total += rare * r8(off + data2 + 4 * (nf - kr) + kr * list_bytes(1))
        return total

    LIST_LAYOUTS = {
        # positions derived from the elements (pos = first element's pos, end = last element's end or the trailing
        # comma, one bit); an empty list keeps its pos in the unused start field
        "derived": lambda k: 8 * k,
        # the list's pos/end stored next to the range
        "kept": lambda k: 16 * k,
        # one start per node for all its lists (they are consecutive in the array) and a 16-bit length per list
        "shared": lambda k: 4 + 2 * k if k else 0,
    }

    def design_c(self, phase="post", classes=None, layout="derived"):
        """Lists as an inline (start, len) range into a per-file handle array: the 16-byte NodeList and 24-byte
        ModifierList structs go; the node's list fields grow (LIST_LAYOUTS); elements stay 4 bytes each. Lazy member
        lists keep their records."""
        _, structs, _ = self.list_totals(phase, classes, kinds=["plain", "modifiers"])
        growth = self.alloc_total(phase, classes, False, self.LIST_LAYOUTS[layout]) - self.alloc_total(phase, classes, False)
        return structs - growth, structs, growth

    def design_header20(self, phase="post", classes=None):
        """The node id leaves the header (24 -> 20 bytes, data at offset 20 when 4-aligned). Ids must stay dense
        (the id-keyed link stores are paged by consecutive ids, notes/mem-layout.md), so the nodes that get one keep it
        in a side table, ~12 B each (estimate). Returns (gross, side table)."""
        gross = self.alloc_total(phase, classes, False) - self.alloc_total(phase, classes, True)
        with_id = sum(v[2] for v in self.kinds(phase, classes).values())
        return gross, 12 * with_id

    def design_literal_flags(self, phase="post", classes=None):
        """TokenFlags out of string / numeric / bigint / regexp literals: 40 -> 32 bytes."""
        return 8 * self.literals(phase, LITERALS_WITH_FLAGS, classes)[0]

    def design_tokens(self, phase="post", classes=None):
        """Every payload-less `Token` node (modifiers, `?`, `=>`, operators, ...) replaced by its kind in the parent."""
        return self.tags(phase, classes).get("Token", [0, 0, 0])[1]

    def combined(self, phase="post", classes=None, net=True):
        """B core (net of its interner) + the better C (shared start) + literal flags."""
        b, bi, _ = self.design_b(B_CORE, phase, classes)
        c, _, _ = self.design_c(phase, classes, "shared")
        return (b - (bi if net else 0)) + c + self.design_literal_flags(phase, classes)

    def combined_max(self, phase="post", classes=None):
        """Every identifier the checker keys no link on removed (net of an all-texts interner) + C (shared start) +
        literal flags + token nodes."""
        b, _, _ = self.design_b(B_MAX, phase, classes)
        bi = INTERN_ENTRY * self.unique(phase, "global_all")[0]
        c, _, _ = self.design_c(phase, classes, "shared")
        return (b - bi) + c + self.design_literal_flags(phase, classes) + self.design_tokens(phase, classes)


def table(header, rows):
    print("| " + " | ".join(header) + " |")
    print("| " + " | ".join(["---"] + ["---:"] * (len(header) - 1)) + " |")
    for r in rows:
        print("| " + " | ".join(str(x) for x in r) + " |")
    print()


def M(b):
    return b / MIB


def note_tables(projs):
    print("### peaks\n")
    table(["project", "tsrs peak", "bun check peak", "gap", "gap / tsrs peak"],
          [[p.name, f"{M(p.peak):,.0f} MiB", f"{M(p.bun):,.0f} MiB", f"{M(p.gap):,.0f} MiB",
            f"{100 * p.gap / p.peak:.1f}%"] for p in projs])

    print("### program and tree\n")
    rows = []
    for p in projs:
        fs, ts = p.files("post", "src")
        fd, td = p.files("post", "dts")
        fl, tl = p.files("pre", "leaf")
        nc, nb = p.node_count_bytes("post")
        lc, lst, lsl = p.list_totals("post")
        tree = p.tree_bytes("post")
        leaf = p.tree_bytes("pre", ["leaf"])
        a, h = p.split.get("parse end", (0, 0))
        rows.append([p.short, f"{fs:,} / {fd:,} / {fl:,}", f"{M(ts + td):.0f}", f"{nc / 1e6:.2f}M", f"{M(tree):.1f}",
                     f"{100 * tree / p.peak:.1f}%", f"{100 * 24 * nc / tree:.0f}%", f"{100 * p.idents('post')[1] / tree:.0f}%",
                     f"{100 * (lst + lsl) / tree:.0f}%", f"{M(leaf):.0f}", f"{M(a):.0f} + {M(h):.0f}"])
    table(["project", "files: source / declaration / freed leaves", "text of source + declaration files, MiB",
           "nodes", "tree, MiB", "tree / peak", "headers / tree", "identifiers / tree", "lists / tree",
           "freed leaf trees, MiB", "arena + heap at parse end, MiB (Mac)"], rows)

    print("### identifiers\n")
    rows = []
    for label, roles in GROUPS:
        rows.append([label] + [f"{p.idents('post', roles)[0] / 1e3:,.0f}k / {M(p.idents('post', roles)[1]):.1f}" for p in projs])
    tot = [p.idents("post") for p in projs]
    rows.append(["**all identifiers** (count / MiB)"] + [f"{t[0] / 1e3:,.0f}k / {M(t[1]):.1f}" for t in tot])
    rows.append(["text kept in the node (40 B, not derived from the source)"] + [f"{100 * (t[3] + t[4]) / t[0]:.1f}%" for t in tot])
    rows.append(["text copied out of the source, MiB"] + [f"{M(t[5]):.2f}" for t in tot])
    rows.append(["with a node id (checker links)"] + [f"{100 * t[8] / t[0]:.0f}%" for t in tot])
    rows.append(["distinct texts in the program"] + [f"{p.unique('post', 'global_all')[0] / 1e3:,.0f}k" for p in projs])
    rows.append(["distinct texts of the B-core names"] + [f"{p.unique('post', 'global_names')[0] / 1e3:,.0f}k" for p in projs])
    table(["role (count / MiB)"] + [p.short for p in projs], rows)

    print("### literals, tokens, lists\n")
    rows = []

    def lit(p, kinds):
        t = p.literals("post", kinds)
        return f"{t[0] / 1e3:,.0f}k / {M(t[1]):.1f}"
    rows.append(["string literals (40 B)"] + [lit(p, ["StringLiteral"]) for p in projs])
    rows.append(["numeric, bigint, regexp literals (40 B)"] + [lit(p, ["NumericLiteral", "BigIntLiteral", "RegularExpressionLiteral"]) for p in projs])
    rows.append(["template parts, no-substitution templates"] + [lit(p, ["TemplateHead", "TemplateMiddle", "TemplateTail", "NoSubstitutionTemplateLiteral"]) for p in projs])
    rows.append(["`Token` nodes (24 B: modifiers, `?`, `=>`, operators)"] + [f"{p.tags('post').get('Token', [0, 0, 0])[0] / 1e3:,.0f}k / {M(p.design_tokens()):.1f}" for p in projs])
    rows.append(["keyword type nodes (24 B: `string`, ...)"] + [f"{p.tags('post').get('KeywordTypeNode', [0, 0, 0])[0] / 1e3:,.0f}k / {M(p.tags('post').get('KeywordTypeNode', [0, 0, 0])[1]):.1f}" for p in projs])
    for b, label in [("0", "0"), ("1", "1"), ("2-3", "2-3"), (("4-8"), "4-8"), (">8", "more than 8")]:
        vals = []
        for p in projs:
            L = p.lists("post")
            c = sum(v[0] for (lk, bb), v in L.items() if bb == b and lk in ("plain", "modifiers"))
            st = sum(v[1] for (lk, bb), v in L.items() if bb == b and lk in ("plain", "modifiers"))
            sl = sum(v[2] for (lk, bb), v in L.items() if bb == b and lk in ("plain", "modifiers"))
            vals.append(f"{c / 1e3:,.0f}k / {M(st):.1f} + {M(sl):.1f}")
        rows.append([f"lists of {label} (count / MiB struct + elements)"] + vals)
    rows.append(["of which modifier lists (24 B)"] + [f"{sum(v[0] for (lk, _), v in p.lists('post').items() if lk == 'modifiers') / 1e3:,.0f}k" for p in projs])
    rows.append(["lazy member lists, forced / not forced"] + [f"{sum(v[0] for (lk, _), v in p.lists('post').items() if lk == 'lazy_forced') / 1e3:,.0f}k / {sum(v[0] for (lk, _), v in p.lists('post').items() if lk == 'lazy_unforced') / 1e3:,.0f}k" for p in projs])
    rows.append(["nodes with nonzero flags"] + [f"{100 * sum(v[5] for v in p.kinds('post').values()) / p.node_count_bytes('post')[0]:.0f}%" for p in projs])
    rows.append(["nodes with a node id"] + [f"{100 * sum(v[2] for v in p.kinds('post').values()) / p.node_count_bytes('post')[0]:.0f}%" for p in projs])
    table(["post"] + [p.short for p in projs], rows)

    print("### designs (MiB saved at the peak, % of the 8-checker peak; **bold** = alone enough to go below bun)\n")
    rows = []
    for p in projs:
        def cell(b):
            s = f"{M(b):.1f} ({100 * b / p.peak:.1f}%)"
            return f"**{s}**" if b > p.gap and p.gap > 0 else s
        a_g, a_i = p.design_a()
        b_g, b_i, _ = p.design_b(B_CORE)
        e_g, _, _ = p.design_b(B_EXT)
        x_g, _, _ = p.design_b(B_MAX)
        x_i = INTERN_ENTRY * p.unique("post", "global_all")[0]
        c, _, _ = p.design_c()
        c2, _, _ = p.design_c(layout="shared")
        ck, _, _ = p.design_c(layout="kept")
        h_g, h_t = p.design_header20()
        rows.append([p.short, f"{M(p.gap):.0f}", cell(a_g - a_i), cell(b_g - b_i), cell(e_g - x_i), cell(x_g - x_i), cell(c),
                     cell(c2), cell(ck), cell(h_g - h_t), cell(p.design_literal_flags()), cell(p.design_tokens()),
                     cell(p.combined()), cell(p.combined_max()), cell(p.tree_bytes("post"))])
    table(["project", "gap to bun, MiB", "A atom in the node", "B core", "B ext", "B max",
           "C ranges", "C shared start", "C positions kept", "D1 20-byte header", "D2 literal flags",
           "D3 no token nodes", "B core + C shared + D2", "B max + C shared + D2 + D3", "ceiling: the whole tree"], rows)

    print("### gross and interner (MiB)\n")
    rows = []
    for p in projs:
        a_g, a_i = p.design_a()
        b_g, b_i, b_n = p.design_b(B_CORE)
        c, cs, cg = p.design_c()
        _, _, cg2 = p.design_c(layout="shared")
        _, _, cgk = p.design_c(layout="kept")
        h_g, h_t = p.design_header20()
        rows.append([p.short, f"{M(a_g):.1f} - {M(a_i):.1f}", f"{M(b_g):.1f} - {M(b_i):.1f} ({b_n / 1e3:,.0f}k nodes)",
                     f"{M(cs):.1f} - {M(cg):.1f} / {M(cg2):.1f} / {M(cgk):.1f}", f"{M(h_g):.1f} - {M(h_t):.1f}"])
    table(["project", "A: gross - interner", "B core: gross - interner",
           "C: list structs - node growth (ranges / shared start / positions kept)", "D1: gross - id side table"], rows)

    print("### leaf trees (pre): extra saving if every freed leaf tree were alive at the peak (MiB)\n")
    rows = []
    for p in projs:
        lc = ["leaf"]
        b_g, _, _ = p.design_b(B_CORE, "pre", lc)
        c, _, _ = p.design_c("pre", lc, "shared")
        rows.append([p.short, f"{M(p.tree_bytes('pre', lc)):.1f}", f"{M(b_g):.1f}", f"{M(c):.1f}",
                     f"{M(p.combined('pre', lc, net=False)):.1f}"])
    table(["project", "leaf trees", "B core (gross)", "C shared start", "B core + C shared + D2"], rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=Path, required=True)
    ap.add_argument("--bench", type=Path, required=True)
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    ap.add_argument("--projects", default=",".join(PROJECTS))
    args = ap.parse_args()
    bench = json.loads(args.bench.read_text())
    fields = list_fields(args.root)
    projs = [Proj(n, args.dir, bench, fields) for n in args.projects.split(",") if (args.dir / f"{n}.tsv").exists()]
    note_tables(projs)


if __name__ == "__main__":
    main()
