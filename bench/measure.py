#!/usr/bin/env python3
"""Run a command and record its wall time, peak RSS and exit status.

  bench/measure.py <out.json> -- <command> [args...]

Writes {"wall_s": W, "maxrss": M, "status": S} to <out.json> (`maxrss` is `ru_maxrss` as `wait4` reports it: KiB on
Linux, bytes on macOS; `status` is the raw wait status) and exits with the command's status.

bench/run.py starts every timed run through this script instead of spawning the compiler itself. On Linux a child's
`ru_maxrss` starts at its parent's resident size at the spawn, so a compiler spawned by the long-running bench runner
(which grows past 500 MiB over a run) could never report a peak below the runner's size: on the 16-vCPU jobs bun
check and tsrs both read exactly 566 MiB on several small projects. This process is a fresh interpreter of a few MiB,
so that floor drops to its size. The wall time is measured here too, so the interpreter's startup is not counted.
"""

import json
import os
import sys
import time


def main() -> None:
    out, sep, argv = sys.argv[1], sys.argv[2], sys.argv[3:]
    if sep != "--" or not argv:
        sys.exit("usage: measure.py <out.json> -- <command> [args...]")
    t0 = time.perf_counter()
    pid = os.posix_spawnp(argv[0], argv, os.environ)
    _, status, ru = os.wait4(pid, 0)
    wall = time.perf_counter() - t0
    with open(out, "w") as f:
        json.dump({"wall_s": wall, "maxrss": ru.ru_maxrss, "status": status}, f)
    sys.exit(os.waitstatus_to_exitcode(status) % 256)


if __name__ == "__main__":
    main()
