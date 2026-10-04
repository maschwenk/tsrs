#!/usr/bin/env python3
# usage: cmp-signatures.py <tsgo.tsbuildinfo> <tsrs.tsbuildinfo>
# The tsbuildinfo difference the hub-edit shortcut is allowed to make (notes/perf-hub-edit-shortcut.md): everything
# equal except that some fileInfos[].signature hold the file's version instead of the declaration hash, and that
# packageJsons / missingPackageJsons are subsets of tsgo's (the lookups of the skipped declaration emits).
# Exit status 1 for any other difference.
import json,sys
def norm(fi):
    if isinstance(fi,str): return {'version':fi,'impliedNodeFormat':1}, fi
    fi=dict(fi); sig=fi.pop('signature', fi['version']); fi.pop('noSignature',None)
    return fi, sig
a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2]))
ok=True
for k in sorted(set(a)|set(b)):
    if k in ('version','fileInfos','packageJsons','missingPackageJsons'): continue
    if a.get(k)!=b.get(k): print('differs:',k); ok=False
for k in ('packageJsons','missingPackageJsons'):
    sa,sb=set(map(str,a.get(k,[]))),set(map(str,b.get(k,[])))
    if not sb<=sa: print(k,'not a subset'); ok=False
    elif sa!=sb: print(f'{k}: {len(sb)} of {len(sa)}')
na=[norm(x) for x in a['fileInfos']]; nb=[norm(x) for x in b['fileInfos']]
if [x[0] for x in na]!=[x[0] for x in nb]: print('fileInfos differ beyond signatures'); ok=False
diff=[(x,y) for x,y in zip(na,nb) if x[1]!=y[1]]
toversion=sum(1 for x,y in diff if y[1]==y[0]['version'])
if toversion!=len(diff): print('a signature differs and is not the version'); ok=False
print(('OK' if ok else 'NOT EQUAL') + f': {len(diff)} signatures are the version instead of the declaration hash')
sys.exit(0 if ok else 1)
