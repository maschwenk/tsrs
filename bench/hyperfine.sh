#!/usr/bin/env bash
# Run through .depot/workflows/perf-probe.yml with script=bench/hyperfine.sh.
set -euo pipefail

hyperfine_dir=$(mktemp -d /tmp/tsrs-hyperfine.XXXXXX)
trap 'rm -rf "$hyperfine_dir"' EXIT
archive=hyperfine-v2.0.0-x86_64-unknown-linux-gnu.tar.gz
curl -fsSL --retry 3 "https://github.com/sharkdp/hyperfine/releases/download/v2.0.0/$archive" \
  -o "$hyperfine_dir/$archive"
# SHA-256 published with the official GitHub release asset.
printf '%s  %s\n' ae2beda2ac99c098427e4f755244552daf20e514422889cca6d44421e2d93c87 \
  "$hyperfine_dir/$archive" | sha256sum --check
tar -xzf "$hyperfine_dir/$archive" -C "$hyperfine_dir"
python3 bench/hyperfine.py \
  --hyperfine "$hyperfine_dir/hyperfine-v2.0.0-x86_64-unknown-linux-gnu/hyperfine" \
  --tsrs "${TSRS_BIN:?}" --work "${BENCH_WORK:?}" --out-dir "${PROBE_OUT:?}/hyperfine"
