#!/usr/bin/env python3
"""Groups the reports of ThreadSanitizer logs (TSAN_OPTIONS=log_path=...) by kind and the first tsrs frame of each
access, so a race reported thousands of times reads as one line (tools/ci/tsan.sh).

    tools/ci/tsan_summary.py <log>... [--top N]

Prints `<n> reports, <m> unique`, then the most frequent groups with one full report each; exits 1 if any log holds
a report.
"""

import collections
import re
import sys


def frames(text: str) -> list[tuple[str, str]]:
    return [(re.sub(r"::h[0-9a-f]{16}$", "", m.group(1)), m.group(2)) for m in re.finditer(r"#\d+ (\S+) (\S+)", text)]


def first_tsrs_frame(fs: list[tuple[str, str]]) -> str:
    """The first frame in tsrs code (std, core, alloc and dependencies skipped), else the top frame."""
    for fn, loc in fs:
        # The function's own path, without generic arguments (`replace<Option<P<Node>>>` is std's, not tsrs's).
        if "tsrs" in fn.split("<", 1)[0]:
            return f"{fn[:110]} ({loc.rsplit('/', 1)[-1]})"
    return fs[0][0][:110] if fs else "?"


def main() -> int:
    args = sys.argv[1:]
    top = 10
    if "--top" in args:
        i = args.index("--top")
        top = int(args[i + 1])
        del args[i : i + 2]
    groups: collections.Counter = collections.Counter()
    example: dict = {}
    for path in args:
        text = open(path, errors="replace").read()
        for report in text.split("=================="):
            m = re.search(r"WARNING: ThreadSanitizer: ([^(\n]+)", report)
            if not m:
                continue
            parts = re.split(r"\n\s*Previous ", report, maxsplit=1)
            current, previous = parts[0], parts[1] if len(parts) > 1 else ""
            key = (m.group(1).strip(), first_tsrs_frame(frames(current)), first_tsrs_frame(frames(previous)) if previous else "-")
            groups[key] += 1
            example.setdefault(key, report.strip())
    print(f"{sum(groups.values())} reports, {len(groups)} unique")
    for (kind, now, before), n in groups.most_common(top):
        print(f"\n{n} x {kind}\n  access:          {now}\n  earlier access:  {before}\n")
        print("\n".join(line for line in example[(kind, now, before)].splitlines()[:40]))
    return 1 if groups else 0


if __name__ == "__main__":
    sys.exit(main())
