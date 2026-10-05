#!/usr/bin/env python3
"""Run a command and record the user-space instructions it retired, summed over all its threads (Linux only).

  bench/count.py <out.json> -- <command> [args...]

Opens a hardware instruction counter on this process (disabled, enabled when the child execs, inherited by every
thread and process the command creates), forks, execs the command, and after it exits reads the total. Writes
{"instructions": N, "exit": code} to <out.json> and exits with the command's status. Kernel work is excluded, so
for a deterministic single-threaded run the count is the same every time, unlike wall time (bench/README.md
"Regression flag").
"""

import ctypes
import ctypes.util
import json
import os
import platform
import struct
import sys

PERF_TYPE_HARDWARE = 0
PERF_COUNT_HW_INSTRUCTIONS = 1
DISABLED, INHERIT, EXCLUDE_KERNEL, EXCLUDE_HV, ENABLE_ON_EXEC = 1 << 0, 1 << 1, 1 << 5, 1 << 6, 1 << 12
SYS_PERF_EVENT_OPEN = {"x86_64": 298, "aarch64": 241}


def open_counter():
    """A file descriptor for the counter, or an error string."""
    number = SYS_PERF_EVENT_OPEN.get(platform.machine())
    if sys.platform != "linux" or number is None:
        return None, f"no perf_event_open on {sys.platform}/{platform.machine()}"
    libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
    attr = bytearray(128)
    struct.pack_into("IIQ", attr, 0, PERF_TYPE_HARDWARE, len(attr), PERF_COUNT_HW_INSTRUCTIONS)
    struct.pack_into("Q", attr, 40, DISABLED | INHERIT | EXCLUDE_KERNEL | EXCLUDE_HV | ENABLE_ON_EXEC)
    fd = libc.syscall(number, (ctypes.c_char * len(attr)).from_buffer(attr), 0, -1, -1, 0)
    if fd < 0:
        return None, f"perf_event_open: {os.strerror(ctypes.get_errno())}"
    return fd, None


def main(argv):
    if len(argv) < 4 or argv[2] != "--":
        sys.exit(__doc__)
    out, command = argv[1], argv[3:]
    fd, error = open_counter()
    pid = os.fork()
    if pid == 0:
        try:
            os.execvp(command[0], command)
        finally:
            os._exit(127)
    _, status = os.waitpid(pid, 0)
    code = os.waitstatus_to_exitcode(status)
    count = struct.unpack("Q", os.read(fd, 8))[0] if fd is not None else None
    with open(out, "w") as f:
        json.dump({"instructions": count, "exit": code, **({"error": error} if error else {})}, f)
    sys.exit(code if code >= 0 else 128 - code)


if __name__ == "__main__":
    main(sys.argv)
