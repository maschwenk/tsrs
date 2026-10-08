#!/usr/bin/env python3
"""Classify use-census rows (notes/mem-use-census.md, notes/mem-never-read-apps.md) by the code that asked for each object.

  usecensus.py askers <tsv> [<tsv at N checkers> <N>]   per asker category: allocated and never-read reachable MB;
                                                        with a second run, the growth per extra checker
  usecensus.py pools <prefix> [N]                       the pools of notes/mem-never-read-apps.md from <prefix>-c1.tsv,
                                                        <prefix>-cN.tsv and the matching .err files (default N = 4)

The TSV comes from `TSRS_CENSUS=1 TSRS_USE_CENSUS=1 TSRS_USE_CENSUS_FRAMES=9 TSRS_CENSUS_TSV=<file>` (alloc-profile,
tsrs_core/plain-ptrs build); the rows read are 'use: arena by type and N frames'.
"""
import re
import sys
from collections import defaultdict

TITLE = re.compile(r"^use: arena by type and \d+ frames")


def demangle(sym: str) -> str:
    s = sym.strip()
    if not s.startswith("_R"):
        return s
    # strip a trailing generic/closure suffix like ...EB5_ or ...0B8_
    s = re.sub(r"(E?B[0-9A-Za-z]*_)+$", "", s)
    s = re.sub(r"s\d*_\d+B\w*_$", "", s)
    best = None
    i = len(s) - 1
    while i > 0:
        if s[i].isdigit():
            j = i
            while j > 0 and s[j - 1].isdigit():
                j -= 1
            L = int(s[j : i + 1])
            if i + 1 + L == len(s) and L > 0:
                best = s[i + 1 :]
                break
            i = j - 1
        else:
            i -= 1
    if best is None:
        m = re.findall(r"\d+([a-z_][a-z0-9_]{2,})", s)
        best = m[-1] if m else s[-40:]
    return best


CATS = [
    ("printing", ("type_to_string", "signature_to_string", "nodebuilder", "symbol_to_string", "type_to_type_node", "report_relation_error", "elaborate")),
    ("inference", ("infer_from_types", "infer_types", "infer_type_arguments", "get_inferred_type", "instantiate_signature_in_context_of", "infer_from_signatures", "infer_from_properties", "infer_from_object_types", "infer_to_mapped_type", "infer_reverse_mapped")),
    ("relation", ("structured_type_related_to", "is_related_to", "check_type_related_to", "properties_related_to", "signatures_related_to", "compare_signatures_related", "is_type_related_to", "is_type_assignable_to", "is_type_subtype_of", "remove_subtypes", "is_simple_type_related_to", "type_related_to_discriminated", "get_reduced_type", "get_normalized_type", "has_excess_properties", "is_weak_type", "union_or_intersection_related_to", "each_type_related", "some_type_related", "type_related_to_some", "is_type_comparable", "get_variances", "is_known_property")),
    ("signature resolution", ("resolve_call", "choose_overload", "get_resolved_signature", "is_signature_applicable", "resolve_call_expression", "resolve_new_expression", "get_signature_applicability_error", "has_correct_arity", "check_call_expression")),
    ("contextual typing", ("get_contextual_type", "check_expression_with_contextual_type", "get_apparent_type_of_contextual_type", "get_type_of_property_of_contextual_type", "instantiate_contextual_type", "check_function_expression_or_object_literal_method")),
    ("getPropertiesOfType", ("get_properties_of_type", "get_properties_of_object_type", "every_property_of_structured_type", "resolve_lazy_members", "get_properties_of_union_or_intersection_type", "for_each_property", "get_lazy_properties_in_order", "get_augmented_properties_of_type")),
    ("property lookup", ("get_property_of_type", "get_property_of_object_type", "get_member_of_unresolved_structured_type", "get_type_of_property_of_type", "check_property_access", "get_property_type_for_index_type", "get_indexed_access_type")),
    ("conditional/mapped", ("get_conditional_type", "resolve_mapped_type_members", "get_type_of_mapped_symbol", "get_lazy_mapped_table", "get_template_type_from_mapped_type", "mapped_type_add_member")),
    ("declarations/annotations", ("get_type_from_type_node", "get_type_of_symbol", "get_declared_type_of_symbol", "get_type_of_variable_or_parameter", "get_return_type_of_signature", "get_base_types", "check_source_element", "check_variable_like_declaration")),
]


def category(frames, outermost=False):
    seq = list(reversed(frames)) if outermost else frames
    for f in seq:
        for cat, keys in CATS:
            if any(k in f for k in keys):
                return cat
    return "other"


def load(path):
    rows = []
    with open(path) as fh:
        for line in fh:
            parts = line.rstrip("\n").split("\t")
            if len(parts) < 10 or not TITLE.match(parts[0]):
                continue
            nums = list(map(int, parts[1:9]))
            name = parts[9]
            ty, _, rest = name.partition("  ")
            frames = [demangle(f) for f in rest.split("  <-  ")]
            rows.append((ty, frames, nums))
    return rows


def agg(rows, key):
    a = defaultdict(lambda: [0, 0, 0])  # alloc bytes, never-read reachable bytes, count
    for ty, frames, nums in rows:
        k = key(ty, frames)
        a[k][0] += nums[0]
        a[k][1] += nums[6]
        a[k][2] += nums[1]
    return a


def mb(b):
    return b / 2**20


def main():
    p1 = sys.argv[1]
    r1 = load(p1)
    keys = {
        "nearest asker": lambda ty, f: category(f),
        "outermost asker (9 frames)": lambda ty, f: category(f, True),
    }
    second = None
    if len(sys.argv) > 3:
        second = (load(sys.argv[2]), int(sys.argv[3]))
    for title, key in keys.items():
        a1 = agg(r1, key)
        a2 = agg(second[0], key) if second else None
        tot1 = sum(v[0] for v in a1.values())
        nr1 = sum(v[1] for v in a1.values())
        print(f"\n## by {title}: {p1.split('/')[-1]} total alloc {mb(tot1):.1f} MB, never-read reachable {mb(nr1):.1f} MB")
        hdr = f"{'category':<26} {'alloc MB':>9} {'never+r MB':>10} {'nr%':>5}"
        if second:
            hdr += f" {'alloc/extra ckr':>15} {'nr/extra ckr':>12}"
        print(hdr)
        for k in sorted(a1, key=lambda k: -a1[k][0]):
            v = a1[k]
            line = f"{k:<26} {mb(v[0]):>9.1f} {mb(v[1]):>10.1f} {100*v[1]/max(v[0],1):>5.1f}"
            if second:
                w = a2.get(k, [0, 0, 0])
                n = second[1]
                line += f" {mb(w[0]-v[0])/(n-1):>15.2f} {mb(w[1]-v[1])/(n-1):>12.2f}"
            print(line)
    # by type x nearest category, never-read reachable
    a = agg(r1, lambda ty, f: (ty, category(f)))
    print("\n## never-read reachable by type and nearest asker (top 40)")
    for k in sorted(a, key=lambda k: -a[k][1])[:40]:
        v = a[k]
        print(f"{mb(v[1]):>8.2f} MB of {mb(v[0]):>8.2f}  {k[0]:<28} {k[1]}")
    # top stacks
    a = agg(r1, lambda ty, f: ty + "  " + "  <-  ".join(f[:7]))
    print("\n## never-read reachable by type and 7 frames (top 50)")
    for k in sorted(a, key=lambda k: -a[k][1])[:50]:
        v = a[k]
        print(f"{mb(v[1]):>7.2f} / {mb(v[0]):>7.2f}  {k}")




def pools(proj, n):
    POOLS=[
     ("P1 lazy-table prep: signature instantiation", lambda ty,s: re.search(r'prepare_lazy_members|create_lazy_member_table',s) and 'instantiate_signature' in s.split('create_lazy_member_table')[0]+s.split('prepare_lazy_members')[0]),
     ("P3 lazy-table prep: unaffected name lists", lambda ty,s: ty=='[&str]' and re.search(r'^tsrs_checker  <-  create_lazy_member_table|prepare_lazy_members',s)),
     ("P2 identity relation: full resolution of both sides", lambda ty,s: 'properties_identical_to' in s or re.search(r'get_properties_of_object_type  <-  properties_related_to',s)),
     ("P4 base members resolved in full for a derived resolve_object_type_members", lambda ty,s: re.search(r'resolve_object_type_members  <-  resolve_structured_type_members_worker  <-  get_properties_of_type  <-  resolve_object_type_members',s)),
     ("P5 base members resolved in full inside resolve_lazy_members (U2)", lambda ty,s: re.search(r'get_properties_of_type  <-  resolve_lazy_members',s)),
     ("P6 union/intersection property creation (getReducedType etc.)", lambda ty,s: 'create_union_or_intersection_property' in s),
     ("P7 mapped type members", lambda ty,s: 'resolve_mapped_type_members' in s or 'new_mapped_type_member' in s),
     ("P8 anonymous type members (signatures, props)", lambda ty,s: 'resolve_anonymous_type_members' in s.split('  <-  ')[:6].__str__()),
     ("P9 discriminant lookups (U1)", lambda ty,s: 'get_unmatched_properties' in s and 'infer' in s),
    ]
    def classify(ty,f):
        s="  <-  ".join(f)
        for name,pred in POOLS:
            if pred(ty,s): return name
        return "rest"
    def run(p):
        a=defaultdict(lambda:[0,0])
        for ty,f,n in load(p):
            k=classify(ty,f); a[k][0]+=n[0]; a[k][1]+=n[6]
        return a
    a1=run(f"{proj}-c1.tsv"); a4=run(f"{proj}-c{n}.tsv")
    def created(p):
        for line in open(p):
            m=re.search(r'checker phase .*created ([\d.]+) MB',line)
            if m: return float(m.group(1))
    c1=created(f"{proj}-c1.err"); c4=created(f"{proj}-c{n}.err")
    print(f"{proj}: checker-phase arena created 1 checker {c1} MB, {n} checkers {c4} MB, per extra {(c4-c1)/(n-1):.1f} MB")
    print(f"{'pool':<75} {'c1 alloc':>8} {'c1 nr':>7} {'extra alloc':>11} {'extra nr':>8} {'nr% of extra':>12}")
    for k in sorted(a1,key=lambda k:-(a4[k][1]-a1[k][1])):
        ea=(a4[k][0]-a1[k][0])/2**20/(n-1); en=(a4[k][1]-a1[k][1])/2**20/(n-1)
        print(f"{k:<75} {a1[k][0]/2**20:8.1f} {a1[k][1]/2**20:7.1f} {ea:11.2f} {en:8.2f} {100*en/((c4-c1)/(n-1)):11.1f}%")



if __name__ == "__main__":
    cmd = sys.argv.pop(1)
    if cmd == "askers":
        main()
    else:
        pools(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 4)
