#!/usr/bin/env python3
"""Compare number formatting with the pinned Go implementation.

Build main.go inside the pinned TypeScript module and the Rust jsnum_oracle example, then:
  run.py --go <go-oracle> --rust <rust-oracle>
"""

import argparse
import math
import random
import struct
import subprocess


def bits(value):
    return struct.unpack(">Q", struct.pack(">d", value))[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", required=True)
    parser.add_argument("--rust", required=True)
    parser.add_argument("--samples", type=int, default=100_000)
    parser.add_argument("--seed", type=int, default=20261011)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    values = {rng.getrandbits(64) for _ in range(args.samples)}
    values.update([0, 1, (1 << 52) - 1, 1 << 52, 0x7FEFFFFFFFFFFFFF, 0x7FF0000000000000, 0x7FF8000000000000])
    for value in [1e-6, 1e21, 0.1, 0.3, 1.0, 2.0**53, 2.0**63, 2.0**64]:
        values.update(bits(v) for v in [math.nextafter(value, 0), value, math.nextafter(value, math.inf)])
    for exponent in range(1, 2047):
        value = exponent << 52
        values.update([value - 1, value, value + 1])
    values.update(value | (1 << 63) for value in list(values) if value < (1 << 63))
    requests = [f"format {value:016x}" for value in sorted(values)]
    data = "\n".join(requests) + "\n"
    outputs = [subprocess.run([exe], input=data, text=True, capture_output=True, check=True).stdout.splitlines()
               for exe in [args.go, args.rust]]
    assert all(len(output) == len(requests) for output in outputs), "missing oracle output"
    mismatches = [(request, go, rust) for request, go, rust in zip(requests, *outputs) if go != rust]
    for mismatch in mismatches[:20]:
        print(mismatch)
    print(f"{len(requests)} values, {len(mismatches)} differences (seed {args.seed})")
    raise SystemExit(bool(mismatches))


if __name__ == "__main__":
    main()
