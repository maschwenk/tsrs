# Picks the committed sample of the parser oracle output: every 10th case, plus every case with object markers,
# symlinks, parse errors, CR characters or non-ASCII text; contents over 20 KB are left out.
import json
import sys

src, dst = sys.argv[1], sys.argv[2]
out = []
with open(src) as f:
    for i, line in enumerate(f):
        o = json.loads(line)
        if len(o["content"]) > 20000:
            continue
        special = (
            o["error"]
            or any(m.get("data") for m in o.get("markers") or [])
            or o.get("symlinks")
            or "\r" in o["content"]
            or any(ord(c) > 127 for c in o["content"])
        )
        if special or i % 10 == 0:
            out.append(line)
with open(dst, "w") as f:
    f.writelines(out)
