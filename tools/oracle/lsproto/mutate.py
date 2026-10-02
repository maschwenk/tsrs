#!/usr/bin/env python3
"""Reads "Type\tJSON" sample lines on stdin; writes them plus mutated variants (dropped key, null value,
wrong-kind value, unknown key, reversed key order, wrong top-level kind) and hand-written edge cases."""

import json
import random
import sys

rng = random.Random(1)


def dump(v):
    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)


def wrong_kind(v):
    if isinstance(v, bool):
        return "true"
    if isinstance(v, (int, float)):
        return "x" if rng.random() < 0.5 else 1.5
    if isinstance(v, str):
        return 5
    if isinstance(v, list):
        return {}
    if isinstance(v, dict):
        return []
    return False


def paths(v, prefix=()):
    yield prefix
    if isinstance(v, dict):
        for k, x in v.items():
            yield from paths(x, prefix + (k,))
    elif isinstance(v, list):
        for i, x in enumerate(v):
            yield from paths(x, prefix + (i,))


def replace(v, path, new):
    if not path:
        return new
    if isinstance(v, dict):
        out = dict(v)
    else:
        out = list(v)
    out[path[0]] = replace(v[path[0]], path[1:], new)
    return out


def drop(v, path):
    if len(path) == 1:
        out = dict(v) if isinstance(v, dict) else list(v)
        del out[path[0]]
        return out
    return replace(v, path[:1], drop(v[path[0]], path[1:]))


out = sys.stdout
for line in sys.stdin:
    name, text = line.rstrip("\n").split("\t", 1)
    out.write(f"{name}\t{text}\n")
    v = json.loads(text)
    all_paths = [p for p in paths(v) if p]
    obj_paths = [p for p in all_paths if isinstance(p[-1], str)]
    if obj_paths:
        p = rng.choice(obj_paths)
        out.write(f"{name}\t{dump(drop(v, p))}\n")
        p = rng.choice(obj_paths)
        out.write(f"{name}\t{dump(replace(v, p, None))}\n")
    if all_paths:
        p = rng.choice(all_paths)
        target = v
        for k in p:
            target = target[k]
        out.write(f"{name}\t{dump(replace(v, p, wrong_kind(target)))}\n")
    if isinstance(v, dict):
        extra = dict(v)
        extra["someUnknownField"] = {"nested": [1, None]}
        out.write(f"{name}\t{dump(extra)}\n")
        out.write(f"{name}\t{dump(dict(reversed(list(v.items()))))}\n")
    out.write(f"{name}\t{dump(wrong_kind(v))}\n")
    if rng.random() < 0.1:
        out.write(f"{name}\tnull\n")
