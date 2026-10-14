#!/usr/bin/env bash
# ThreadSanitizer gate: tsrs built with `-Zsanitizer=thread` runs the multi-threaded paths, and any report fails it.
# `P<T>` is `Send`/`Sync` by decree (tsrs_core::ptr), so only a run like this checks that threads do not race on arena
# data. Workloads: type checks at 2 and 8 checkers, emit, `-b` (clean and incremental) on a generated project graph,
# `--watch` and `-b --watch` with edits, and a language-server session (tools/lsp-mem/lsp_mem.py).
#
#   tools/ci/tsan.sh <tsan tsrs binary> [--projects xstate-main,webpack] [--work-dir bench/.work]
#
# Build the binary with the system allocator (ThreadSanitizer cannot see mimalloc's reuse of memory across threads)
# and plain pointers (its shadow memory takes the address range of the compressed handles' reservation):
#
#   RUSTFLAGS=-Zsanitizer=thread cargo +nightly-2026-10-04 build --release -p tsrs_cli -Zbuild-std \
#     --features tsrs_core/plain-ptrs,tsrs_cli/system-alloc --target <host triple>
#
# Suppressions (tools/ci/tsan.supp) are for reports that are not tsrs races, each with its reason.
set -uo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
[ $# -ge 1 ] || { echo "usage: $0 <tsan tsrs binary> [--projects a,b] [--work-dir DIR]"; exit 2; }
tsrs="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; shift
projects="xstate-main,webpack"; work="$repo/bench/.work"
while [ $# -gt 0 ]; do
  case "$1" in
    --projects) projects=$2; shift 2 ;;
    --work-dir) work="$(cd "$2" && pwd)"; shift 2 ;;
    *) echo "unknown argument $1"; exit 2 ;;
  esac
done
out="$(mktemp -d "${TMPDIR:-/tmp}/tsrs-tsan.XXXXXX")"
logs="$out/logs"; mkdir -p "$logs"
base_options="suppressions=$repo/tools/ci/tsan.supp halt_on_error=0 exitcode=0 history_size=4"
fail=0

# A gate that cannot fail gates nothing: the binary must run under ThreadSanitizer.
banner="$(TSAN_OPTIONS="verbosity=1" "$tsrs" --version 2>&1)"
if ! grep -q "Running under ThreadSanitizer" <<< "$banner"; then
  echo "FAIL: $tsrs is not built with -Zsanitizer=thread"; exit 2
fi

# run <name> <exit statuses> <command...>: runs under ThreadSanitizer, its reports in $logs/<name>.<pid>. A status not in
# the comma list fails the gate too: a workload that broke would otherwise pass without having run (type checks exit
# 2 when they report errors).
run() {
  local name=$1 expected=$2 allowed=",$2," t0 status; shift 2
  t0=$(date +%s)
  TSAN_OPTIONS="$base_options log_path=$logs/$name" "$@" > "$out/$name.out" 2>&1; status=$?
  local n; n=$(cat "$logs/$name".* 2>/dev/null | grep -c "WARNING: ThreadSanitizer")
  echo "$name: exit $status, $(( $(date +%s) - t0 )) s, $n reports"
  [ "$n" = 0 ] || fail=1
  if [[ "$allowed" != *",$status,"* ]]; then
    echo "FAIL $name: exit $status (expected $expected):"; tail -20 "$out/$name.out"; fail=1
  fi
}

project_dir() {
  python3 -c '
import json, sys
p = next(p for p in json.load(open(sys.argv[1]))["projects"] if p["name"] == sys.argv[2])
print(sys.argv[3] + "/solutions/" + p["name"] + "/" + p["project"])' "$repo/bench/projects.json" "$1" "$work"
}

IFS=',' read -r -a names <<< "$projects"
for name in "${names[@]}"; do
  dir="$(project_dir "$name")"
  for n in 2 8; do run "check-$name-$n" 0,2 "$tsrs" -p "$dir" --noEmit --incremental false --pretty false --checkers "$n"; done
done
first="$(project_dir "${names[0]}")"
run "emit-${names[0]}" 0,2 "$tsrs" -p "$first" --noEmit false --declaration --sourceMap --outDir "$out/emit" \
  --incremental false --pretty false --checkers 8

# A generated graph of 12 composite projects (40 files each, layered references), so that `-b` builds several at once.
graph="$out/graph"
python3 - "$graph" <<'EOF'
import json, os, sys
root, n = sys.argv[1], 12
deps = {i: [j for j in range(max(0, i - 3), i) if (i + j) % 2 == 0 or j == i - 1] for i in range(n)}
for i in range(n):
    os.makedirs(f"{root}/p{i}/src")
    json.dump({"compilerOptions": {"composite": True, "outDir": "out", "rootDir": "src", "strict": True, "target": "es2020",
                                   "module": "esnext", "moduleResolution": "bundler", "types": []},
               "include": ["src"], "references": [{"path": f"../p{j}"} for j in deps[i]]}, open(f"{root}/p{i}/tsconfig.json", "w"))
    for f in range(40):
        if f == 0:
            imports = "".join(f'import {{ v{j}_0 }} from "../../p{j}/src/f0";\n' for j in deps[i])
            use = " + ".join(f"v{j}_0" for j in deps[i]) or "0"
        else:
            imports, use = f'import {{ v{i}_{f - 1} }} from "./f{f - 1}";\n', f"v{i}_{f - 1}"
        body = "\n".join(f"export interface S{k} {{ a: number; b: string; c: S{k}[] }}\n"
                         f"export function g{k}(x: S{k}): number {{ return x.a + x.c.length; }}" for k in range(15))
        open(f"{root}/p{i}/src/f{f}.ts", "w").write(f"{imports}{body}\nexport const v{i}_{f}: number = {use};\n")
json.dump({"files": [], "references": [{"path": f"p{i}"} for i in range(n)]}, open(f"{root}/tsconfig.json", "w"))
EOF
run build-clean 0 "$tsrs" -b "$graph" --pretty false
printf '\nexport const edited = 1;\n' >> "$graph/p0/src/f5.ts"
run build-incremental 0 "$tsrs" -b "$graph" --pretty false

# watch_session <file...> -- <command...>: starts a watch, appends a line to each file and restores it, each time
# waiting for the cycle, then stops it with SIGINT.
watch_session() {
  python3 - "$@" <<'EOF'
import queue, signal, subprocess, sys, threading, time
args = sys.argv[1:]
files, cmd = args[:args.index("--")], args[args.index("--") + 1:]
p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
lines = queue.Queue()
threading.Thread(target=lambda: [lines.put(l) for l in p.stdout], daemon=True).start()
def settle(timeout=600):
    seen, deadline = False, time.time() + timeout
    while time.time() < deadline:
        try:
            line = lines.get(timeout=3 if seen else 60)
        except queue.Empty:
            if seen:
                return
            continue
        seen = seen or "Watching for file changes" in line
    sys.exit("no watch report")
settle()
for i, f in enumerate(files):
    text = open(f).read()
    try:
        open(f, "w").write(text + f"\nexport const tsanEdit{i} = {i};\n")
        settle()
    finally:
        open(f, "w").write(text)
    settle()
p.send_signal(signal.SIGINT)
p.wait(timeout=60)
EOF
}
run watch-build 0 watch_session "$graph/p0/src/f3.ts" "$graph/p5/src/f0.ts" "$graph/p11/src/f7.ts" -- \
  "$tsrs" -b "$graph" --watch --preserveWatchOutput --pretty false
lsp_file="$(cd "$first" && git ls-files '*.ts' | grep -v '\.d\.ts$' | while read -r f; do grep -qE '^[[:space:]]+return\b' "$f" && { echo "$f"; break; }; done)"
watch_file="$first/$lsp_file"
run watch 0 watch_session "$watch_file" "$watch_file" -- "$tsrs" --watch -p "$first" --noEmit --preserveWatchOutput --pretty false
run lsp 0 python3 "$repo/tools/lsp-mem/lsp_mem.py" --cmd "$tsrs --lsp -stdio" --project "$first" --file "$lsp_file" \
  --edits 40 --every 20 --completion --log "$out/lsp"

if [ "$fail" = 1 ]; then
  echo; echo "FAIL (logs and outputs kept in $out)"
  ls "$logs"/* > /dev/null 2>&1 && python3 "$repo/tools/ci/tsan_summary.py" "$logs"/*
  exit 1
fi
rm -rf "$out"
echo "ThreadSanitizer: no reports"
