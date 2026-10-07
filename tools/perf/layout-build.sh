#!/usr/bin/env bash
# Builds the binaries the layout probe compares (.depot/workflows/perf-layout-probe.yml, notes/perf-binary-layout.md),
# all from one instrumented build and one training run, Linux x86-64:
#
#   A    today's bench pipeline (bench.yml `build`): -Cprofile-generate build, pgo-train.sh, merge, dist build
#   B    A's profile plus EXTRA_TRAIN runs of tsrs (the README workload: big projects, default and 32 checkers)
#   R    A linked with --emit-relocs, then BOLT as release.yml does it (bolt.sh: trained on xstate-main + webpack)
#   Rx   R with the BOLT training set extended by the EXTRA_TRAIN runs
#   RB   B linked with --emit-relocs, BOLT trained on xstate-main + webpack + the EXTRA_TRAIN runs
#   RBH  RB with -hugify (hot text remapped onto 2 MiB pages at startup)
#
#   layout-build.sh <out dir> <bench work dir>
#
# Writes <out>/bin/<name>/tsrs, <out>/times.txt (seconds per step) and <out>/profile-coverage.txt. Needs llvm-bolt,
# merge-fdata and the BOLT runtime libraries on PATH / next to them, ts-ref/tsc/testdata, and the bench projects set up.
set -euo pipefail

mkdir -p "$1"
out=$(cd "$1" && pwd)
work=$(cd "$2" && pwd)
tgt=x86_64-unknown-linux-gnu
profdata="$(rustc --print sysroot)/lib/rustlib/$tgt/bin/llvm-profdata"
EXTRA_TRAIN=${EXTRA_TRAIN:-vscode,t3code-server,formbricks-web}
mkdir -p "$out/bin" "$out/raw-base" "$out/raw-extra" "$out/bolt"
: > "$out/times.txt"

step() { # $1 = name; runs the rest and records its duration
  local name=$1 t0; shift; t0=$(date +%s.%N)
  "$@"
  awk -v n="$name" -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN { printf "%-34s %6.1f s\n", n, b - a }' | tee -a "$out/times.txt"
}
check_exit() { if [ "$1" -gt 2 ]; then echo "::error::$2 exited with $1"; exit 1; fi; }
# "<cwd>\t<-p path>" of a bench project (bench/run.py project_path).
project_path() {
  python3 -c 'import json, sys; sys.path.insert(0, "bench"); import run as rb
cfg = json.load(open("bench/projects.json")); p = next(p for p in cfg["projects"] if p["name"] == sys.argv[1])
cwd, proj = rb.project_path(cfg, p, rb.Path(sys.argv[2])); print(f"{cwd}\t{proj}")' "$1" "$work"
}
# Runs tsrs binary $1 on every EXTRA_TRAIN project at the default checker count and at 32; $2 = env assignment.
extra_runs() {
  local exe=$1 envset=$2 p cwd proj c
  IFS=, read -ra projects <<< "$EXTRA_TRAIN"
  for p in "${projects[@]}"; do
    IFS=$'\t' read -r cwd proj < <(project_path "$p")
    for c in default 32; do
      args=(); [ "$c" = default ] || args=(--checkers "$c")
      (cd "$cwd" && env "${envset//@P@/$p-$c}" "$exe" -p "$proj" --noEmit --incremental false --pretty false "${args[@]}" \
        > /dev/null) && status=0 || status=$?
      check_exit "$status" "$exe -p $p ($c)"
    done
  done
}

# 1. The instrumented build and the training of today's pipeline (bench.yml / release.yml).
step "instrumented build" env RUSTFLAGS=-Cprofile-generate="$out/raw-base" \
  cargo build --profile dist --locked -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash --target "$tgt"
inst=$PWD/target/$tgt/dist
train() { .github/scripts/pgo-train.sh "$inst" "$out/raw-base" "$work" > "$out/pgo-train.log" 2>&1 || { tail -40 "$out/pgo-train.log"; return 1; }; }
step "pgo-train.sh" train
step "extra PGO training ($EXTRA_TRAIN)" extra_runs "$inst/tsrs" "LLVM_PROFILE_FILE=$out/raw-extra/@P@-%p.profraw"

"$profdata" merge -o "$out/A.profdata" "$out"/raw-base/*.profraw
"$profdata" merge -o "$out/B.profdata" "$out"/raw-base/*.profraw "$out"/raw-extra/*.profraw
for v in A B; do
  echo "== $v.profdata"; "$profdata" show "$out/$v.profdata" | tail -4
  # Functions with a non-zero entry count, per crate of the workspace (the first tsrs_* path component).
  "$profdata" show --all-functions "$out/$v.profdata" | python3 -c '
import re, sys, collections
seen, hot, name = collections.Counter(), collections.Counter(), None
for line in sys.stdin:
    if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
        m = re.search(r"\d(tsrs_[a-z0-9_]+?)(?:\d|$|17h|[A-Z])", line); name = m.group(1) if m else "other"
    elif line.strip().startswith("Function count:") and name:
        seen[name] += 1; hot[name] += int(line.split(":")[1]) > 0
for k in sorted(seen, key=lambda k: -seen[k])[:25]:
    print(f"  {k:28} {hot[k]:7} / {seen[k]:7} functions executed")'
done > "$out/profile-coverage.txt"
cat "$out/profile-coverage.txt"

# 2. The final builds, in parallel (fat LTO links on one core). Relocations only change addresses, not code.
dist_build() { # $1 = name, $2 = profile, $3 = extra RUSTFLAGS
  CARGO_TARGET_DIR="$out/t-$1" RUSTFLAGS="-Cprofile-use=$2 $3" \
    cargo build --profile dist --locked -p tsrs_cli --target "$tgt" > "$out/build-$1.log" 2>&1 \
    || { tail -40 "$out/build-$1.log"; return 1; }
}
t0=$(date +%s)
pids=()
dist_build A "$out/A.profdata" "" & pids+=($!)
dist_build Ar "$out/A.profdata" "-Clink-arg=-Wl,--emit-relocs" & pids+=($!)
dist_build B "$out/B.profdata" "" & pids+=($!)
dist_build Br "$out/B.profdata" "-Clink-arg=-Wl,--emit-relocs" & pids+=($!)
for pid in "${pids[@]}"; do wait "$pid"; done
echo "final builds (4 in parallel)          $(($(date +%s) - t0)) s" | tee -a "$out/times.txt"
for v in A B; do mkdir -p "$out/bin/$v"; cp "$out/t-$v/$tgt/dist/tsrs" "$out/bin/$v/tsrs"; done

# 3. BOLT (instrumentation mode, as .github/scripts/bolt.sh; LBR sampling is not assumed on a VM).
flags=(-reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-all-cold -split-eh -icf=1
  -use-gnu-stack -update-debug-sections -dyno-stats)
instrument() { # $1 = input binary, $2 = name
  rm -rf "$out/bolt/$2.d"; mkdir -p "$out/bolt/$2.d"
  llvm-bolt "$1" -instrument -o "$out/bolt/$2.inst" --instrumentation-file="$out/bolt/$2.d/prof" \
    --instrumentation-file-append-pid > "$out/bolt/$2.inst.log" 2>&1 || { cat "$out/bolt/$2.inst.log"; return 1; }
}
release_runs() { # bolt.sh's tsrs workload: xstate-main and webpack, default checker count
  local exe=$1 p
  for p in xstate-main webpack; do
    (cd "$work/solutions/$p" && "$exe" -p . --noEmit --incremental false --pretty false > /dev/null) && status=0 || status=$?
    check_exit "$status" "$exe -p $p"
  done
}
optimize() { # $1 = input binary, $2 = output name, $3.. = fdata files, then optional -- extra flags
  local in=$1 name=$2; shift 2
  local fd=() extra=()
  while [ $# -gt 0 ]; do [ "$1" = -- ] && { shift; extra=("$@"); break; }; fd+=("$1"); shift; done
  merge-fdata "${fd[@]}" > "$out/bolt/$name.fdata"
  llvm-bolt "$in" -o "$out/bolt/$name.bolt" -data="$out/bolt/$name.fdata" "${flags[@]}" "${extra[@]}" \
    > "$out/bolt/$name.log" 2>&1 || { cat "$out/bolt/$name.log"; return 1; }
  grep -E 'BOLT-INFO: (basic block reordering|splitting|ICF|hugify)|BOLT-WARNING' "$out/bolt/$name.log" | head -8 || true
  mkdir -p "$out/bin/$name"; cp "$out/bolt/$name.bolt" "$out/bin/$name/tsrs"
}
Ar=$out/t-Ar/$tgt/dist/tsrs; Br=$out/t-Br/$tgt/dist/tsrs
step "BOLT instrument (R)" instrument "$Ar" Ar-rel
instrument "$Ar" Ar-ext
instrument "$Br" Br
step "BOLT training, release set (R)" release_runs "$out/bolt/Ar-rel.inst"
step "BOLT training, release set (RB)" release_runs "$out/bolt/Br.inst"
step "BOLT training, extra set (Rx)" extra_runs "$out/bolt/Ar-ext.inst" "BOLT_UNUSED=1"
step "BOLT training, extra set (RB)" extra_runs "$out/bolt/Br.inst" "BOLT_UNUSED=1"
step "BOLT optimize R" optimize "$Ar" R "$out"/bolt/Ar-rel.d/prof*
optimize "$Ar" Rx "$out"/bolt/Ar-rel.d/prof* "$out"/bolt/Ar-ext.d/prof*
optimize "$Br" RB "$out"/bolt/Br.d/prof*
optimize "$Br" RBH "$out"/bolt/Br.d/prof* -- -hugify || echo "::warning::llvm-bolt -hugify failed; no RBH"

ls -l "$out"/bin/*/tsrs
for b in "$out"/bin/*/tsrs; do size -A "$b" | awk -v b="$b" '$1 ~ /^\.(text|bolt|text\.cold)/ {printf "%s %s %.2f MB\n", b, $1, $2/1048576}'; done
cat "$out/times.txt"
