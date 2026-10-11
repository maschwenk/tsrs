#!/usr/bin/env python3
"""Aggregate bbcount output into instruction profiles. usage: bbagg.py <bbcount.txt> <binary> <outprefix>
Writes <outprefix>.self.tsv (innermost function incl. inlined frames), .outer.tsv (physical function),
.line.tsv (innermost file:line), .incl.tsv (inclusive over the inline chain of the physical function)."""
import sys, subprocess, collections, re, os, pickle
bbf, binary, out = sys.argv[1:4]
pcs = collections.Counter(); other = 0; main = 0
for line in open(bbf):
    p = line.split()
    if p[0] == 'other': other = int(p[1]); continue
    if p[0] == 'main': main = int(p[1]); continue
    off, n, c = int(p[0], 16), int(p[1]), int(p[2])
    for k in range(n): pcs[off + 4 * k] += c
total = sum(pcs.values())
print(f'main {total} (reported {main}) other {other}', file=sys.stderr)
cache = binary + '.symcache.pkl'
symc = pickle.load(open(cache, 'rb')) if os.path.exists(cache) else {}
need = [a for a in pcs if a not in symc]
def clean(n):
    n = re.sub(r'::h[0-9a-f]{16}$', '', n); n = re.sub(r'<impl (.*?)>', r'\1', n); return n
if need:
    need.sort()
    r = subprocess.run([os.environ.get('LLVM_SYMBOLIZER', 'llvm-symbolizer'), '--obj=' + binary, '--inlining', '--functions=linkage', '--demangle'],
                       input='\n'.join(hex(a) for a in need) + '\n', capture_output=True, text=True).stdout
    groups = r.strip('\n').split('\n\n'); assert len(groups) == len(need), (len(groups), len(need))
    for a, g in zip(need, groups):
        lines = g.split('\n'); fr = []
        for i in range(0, len(lines) - 1, 2):
            loc = lines[i + 1].rsplit('/', 1)[-1]
            fr.append((clean(lines[i]), loc))
        symc[a] = fr or [('?', '?')]
    pickle.dump(symc, open(cache, 'wb'))
selfc = collections.Counter(); outer = collections.Counter(); linec = collections.Counter(); incl = collections.Counter()
for a, c in pcs.items():
    fr = symc[a]
    selfc[fr[0][0]] += c; outer[fr[-1][0]] += c
    loc = fr[0][1]; loc = ':'.join(loc.split(':')[:2])
    linec[(loc, fr[0][0])] += c
    for f in set(x[0] for x in fr): incl[f] += c
def dump(name, ctr, key=lambda k: k):
    with open(out + name, 'w') as f:
        for k, v in ctr.most_common():
            f.write(f'{v}\t{v / total * 100:.3f}%\t{key(k)}\n')
dump('.self.tsv', selfc); dump('.outer.tsv', outer); dump('.incl.tsv', incl)
dump('.line.tsv', linec, lambda k: k[0] + '\t' + k[1])
print(f'total main instructions {total}', file=sys.stderr)
