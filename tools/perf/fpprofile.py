#!/usr/bin/env python3
"""Attribute frame-pointer `perf script` samples of one thread (by comm) to regions named by a frame on the stack
(notes/perf-serial-steps.md; not committed). Reads `perf script -F comm,tid,time,ip,sym` from stdin.

    perf script -F comm,tid,time,ip,sym --comms tsrs | fpprofile.py --hz 20000 get_processed_files create_checkers
"""

import argparse
import collections
import re
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--hz", type=float, default=20000)
    ap.add_argument("--top", type=int, default=35)
    ap.add_argument("--timeline", type=int, default=0, help="also print the busiest thread's runs of its N outermost tsrs frames")
    ap.add_argument("regions", nargs="+")
    a = ap.parse_args()
    self_by = {r: collections.Counter() for r in a.regions}
    child_by = {r: collections.Counter() for r in a.regions}
    span = {r: [None, None] for r in a.regions}
    total = 0
    stack, t, tid = [], None, None
    samples = []  # (tid, time, label)

    def flush():
        nonlocal total
        if not stack:
            return
        total += 1
        if a.timeline:
            label = "other"
            for i, f in enumerate(stack):
                hit = next((r for r in a.regions if r in f), None)
                if hit:
                    label = hit + (" > " + re.sub(r"::h[0-9a-f]{16}$", "", stack[i - 1])[-70:] if i > 0 else "")
                    break
            samples.append((tid, t, label))
        for r in a.regions:
            # innermost-first stack; the outermost frame matching the region
            idx = [i for i, s in enumerate(stack) if r in s]
            if idx:
                i = idx[-1]
                self_by[r][stack[0]] += 1
                child_by[r][stack[i - 1] if i > 0 else "(self)"] += 1
                sp = span[r]
                sp[0] = t if sp[0] is None else min(sp[0], t)
                sp[1] = t if sp[1] is None else max(sp[1], t)
                break

    header = re.compile(r"^\S.*?\s(\d+)\s+(\d+\.\d+):")
    for line in sys.stdin:
        if not line.strip():
            flush()
            stack = []
            continue
        if not line.startswith((" ", "\t")):
            flush()
            stack = []
            m = header.match(line)
            if m:
                tid, t = int(m.group(1)), float(m.group(2))
            continue
        parts = line.strip().split(None, 1)
        sym = parts[1] if len(parts) > 1 else parts[0]
        sym = re.sub(r"\+0x[0-9a-f]+$", "", sym)
        sym = re.sub(r"::h[0-9a-f]{16}$", "", sym)
        stack.append(sym[:160])
    flush()
    if a.timeline and samples:
        program = collections.Counter(s[0] for s in samples if a.regions[0] in s[2])
        busiest = (program or collections.Counter(s[0] for s in samples)).most_common(1)[0][0]
        runs, t0 = [], None
        for s_tid, s_t, label in samples:
            if s_tid != busiest:
                continue
            t0 = s_t if t0 is None else t0
            if runs and runs[-1][2] == label and s_t - runs[-1][1] < 0.002:
                runs[-1][1] = s_t
                runs[-1][3] += 1
            else:
                runs.append([s_t, s_t, label, 1])
        print(f"timeline of tid {busiest} (ms from its first sample; runs of >= 4 samples)")
        for start, end, label, n in runs:
            if n >= 4:
                print(f"{1000 * (start - t0):8.2f} {1000 * (end - start):7.2f} ms {n:6d}  {label}")
    print(f"samples of the thread: {total} ({total / a.hz * 1000:.1f} ms at {a.hz:.0f} Hz)")
    for r in a.regions:
        n = sum(self_by[r].values())
        sp = span[r]
        wall = f", first-to-last sample {1000 * (sp[1] - sp[0]):.1f} ms" if sp[0] is not None else ""
        print(f"\n== {r}: {n} samples ({n / a.hz * 1000:.1f} ms){wall}")
        print("-- direct children (inclusive)")
        for s, c in child_by[r].most_common(a.top):
            print(f"{c:7d} {c / a.hz * 1000:7.2f} ms  {s}")
        print("-- self")
        for s, c in self_by[r].most_common(a.top):
            print(f"{c:7d} {c / a.hz * 1000:7.2f} ms  {s}")


if __name__ == "__main__":
    main()
