#!/usr/bin/env python3
"""Summarize a TSRS_XFILE dump (exp/crossfile-diagnostics). Usage: analyze.py dump.tsv [top]"""
import sys, collections

path = sys.argv[1]
top = int(sys.argv[2]) if len(sys.argv) > 2 else 15
owners = {}  # file -> (checker, seq)
A, D, F, B = [], [], set(), []
for line in open(path, encoding='utf-8', errors='replace'):
    f = line.rstrip('\n').split('\t')
    t = f[0]
    if t == 'A':
        A.append(f)
    elif t == 'C':
        if f[4] == '0':
            owners[f[3]] = (int(f[1]), int(f[2]))
    elif t == 'D':
        D.append(f)
    elif t == 'F':
        F.add((f[1], f[2], f[3], f[4]))
    elif t == 'B':
        B.append(f)

def fate(checker, xfile, xstate):
    if xstate == 1:
        return 'lost-already'
    o = owners.get(xfile)
    if o is None:
        return 'lost-unchecked'
    if o[0] == checker:
        return 'kept'
    return 'lost-other'

MiB = 1 << 20
def fmt(v):
    return f"{v[0]/MiB:.1f}/{v[1]/MiB:.1f}/{v[2]/1e9:.2f}"

# A rows: kind checker cross xfile xstate site code flag n ta th ti pa ph pi
tot = collections.defaultdict(lambda: [0, 0, 0, 0, 0, 0, 0])
scope_sites = collections.defaultdict(lambda: [0, 0, 0, 0, 0, 0, 0])
print_sites = collections.defaultdict(lambda: [0, 0, 0, 0, 0, 0, 0])
for f in A:
    kind, checker, cross, xfile, xstate, site, code, flag = int(f[1]), int(f[2]), int(f[3]), f[4], int(f[5]), f[6], f[7], f[8]
    n, ta, th, ti, pa, ph, pi = map(int, f[9:16])
    fa = fate(checker, xfile, xstate) if cross == 1 else ('same' if cross == 0 else 'nocheck')
    k = (kind, 'lost' if fa.startswith('lost') else fa, flag)
    v = tot[k]
    for i, x in enumerate((n, ta, th, ti, pa, ph, pi)):
        v[i] += x
    if kind == 0 and cross == 1:
        v = scope_sites[(site, fa, flag)]
        for i, x in enumerate((n, ta, th, ti, pa, ph, pi)):
            v[i] += x
    if kind == 1 and cross == 1 and flag == '0':
        v = print_sites[(site, fa)]
        for i, x in enumerate((n, ta, th, ti, pa, ph, pi)):
            v[i] += x

names = {0: 'reporting relation', 1: 'print', 2: 'error add', 3: 'suggestion add'}
print("## totals (kind, fate, flag): n | total arena/heap MiB/instr G | print arena/heap MiB/instr G")
for k in sorted(tot):
    v = tot[k]
    print(f"{names[k[0]]:20s} {k[1]:8s} flag={k[2]} n={v[0]:>10} total={fmt(v[1:4])} print={fmt(v[4:7])}")

print("\n## cross-file reporting relations by site (lost ones first), sorted by print arena bytes")
rows = sorted(scope_sites.items(), key=lambda kv: -kv[1][4])
for (site, fa, flag), v in rows[:top * 2]:
    print(f"{site:40s} {fa:14s} produced={flag} n={v[0]:>8} total={fmt(v[1:4])} print={fmt(v[4:7])}")

print("\n## cross-file prints outside reporting relations, by site")
rows = sorted(print_sites.items(), key=lambda kv: -kv[1][4])
for (site, fa), v in rows[:top]:
    print(f"{site:40s} {fa:14s} n={v[0]:>8} print={fmt(v[4:7])}")

print("\n## cross-file diagnostics (D rows): by code, add site, scope site, fate, reproduced in output")
dg = collections.Counter()
fates = collections.Counter()
for f in D:
    checker, seq, xstate, sugg, code, xfile, site, scope_site, pos, yfile, text = f[1:12]
    fa = fate(int(checker), xfile, int(xstate))
    rep = (xfile, pos, code, text) in F
    dg[(code, site, scope_site, fa, rep, sugg)] += 1
    fates[(fa, rep, sugg)] += 1
print("fate/reproduced/suggestion:", dict(fates))
for k, n in dg.most_common(top * 2):
    print(n, *k)

print("\n## backtrace samples")
seen = set()
for f in B:
    key = (f[1], f[2])
    if key in seen:
        continue
    seen.add(key)
    print(f[1], f[2], f[3], f[-1][:1500].replace('\\n', ' '))
print(f"\nfiles checked: {len(owners)}; output diagnostics: {len(F)}")
# summary: upper bound of the work spent on cross-file reporting whose diagnostics are never read
ub = [0, 0, 0, 0]  # n, arena, heap, instr
ubp = [0, 0, 0]
kept = [0, 0, 0, 0]
noch = [0, 0, 0, 0]
for f in A:
    kind, checker, cross, xfile, xstate, flag = int(f[1]), int(f[2]), int(f[3]), f[4], int(f[5]), f[8]
    n, ta, th, ti, pa, ph, pi = map(int, f[9:16])
    if cross == 2 and kind == 1:
        for i, x in enumerate((n, ta, th, ti)): noch[i] += x
    if cross != 1 or kind not in (0, 1) or (kind == 1 and flag == '1'):
        continue
    fa = fate(checker, xfile, xstate)
    tgt = ub if fa.startswith('lost') else kept
    for i, x in enumerate((n, ta, th, ti)): tgt[i] += x
    if fa.startswith('lost'):
        for i, x in enumerate((pa, ph, pi)): ubp[i] += x
nd = collections.Counter()
for f in D:
    if f[4] == '1':
        continue
    fa = fate(int(f[1]), f[6], int(f[3]))
    nd[('lost' if fa.startswith('lost') else 'kept', (f[6], f[9], f[5], f[11]) in F)] += 1
print(f"SUMMARY lost_scopes+prints n={ub[0]} arena={ub[1]/MiB:.2f}MiB heap={ub[2]/MiB:.2f}MiB instr={ub[3]/1e9:.3f}G "
      f"| of which printing arena={ubp[0]/MiB:.2f}MiB heap={ubp[1]/MiB:.2f}MiB instr={ubp[2]/1e9:.3f}G "
      f"| kept n={kept[0]} instr={kept[3]/1e9:.3f}G | nocheck prints n={noch[0]} arena={noch[1]/MiB:.2f}MiB instr={noch[3]/1e9:.3f}G "
      f"| error diags lost/kept x reproduced: {dict(nd)}")
