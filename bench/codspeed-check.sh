#!/bin/sh
# Type-checks one project for CodSpeed's simulation run (bench/codspeed.yml) with the flags and the single thread of
# the bench's instruction count (bench/run.py), output discarded. tsc's statuses 1 and 2 mean "ran, reported errors",
# so only a crash fails the benchmark. CodSpeed measures the tsrs child of this shell because the workflow sets
# `simulation-track-subprocess`.
RAYON_NUM_THREADS=1 target/release/tsrs -p "$1" --noEmit --incremental false --singleThreaded --pretty false > /dev/null
[ $? -le 2 ]
