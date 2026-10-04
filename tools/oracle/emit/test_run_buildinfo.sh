#!/usr/bin/env bash
# Regression test for run.py --buildinfo with the real compilers on a throwaway public fixture:
#   1. default output dirs (<W>/ref/out, <W>/rs/out) -> identical tsbuildinfo, exit 0 (JS + .d.ts, and decl-only)
#   2. tsrs whose tsbuildinfo is altered (one hash changed) -> exit 1, reported as a different file
#   3. tsrs whose tsbuildinfo is deleted -> exit 1 (missing); tsrs whose .js output is altered -> exit 1
#   4. explicit --go-out/--rs-out with different basenames -> exit 2 (paths would serialize differently)
#   5. equal or nested --go-out/--rs-out (with the .js-altering tsrs) -> exit 2 before anything runs
#
#   TSGO=<reference tsgo> TSRS=<tsrs binary with incremental emit> tools/oracle/emit/test_run_buildinfo.sh
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${TSGO:?set TSGO}" "${TSRS:?set TSRS}"
work="$(mktemp -d /tmp/emit-buildinfo-test.XXXXXX)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/p/src"
echo '{ "compilerOptions": { "target": "esnext", "module": "nodenext", "declaration": true, "incremental": true, "strict": true, "rootDir": "src" }, "include": ["src"] }' > "$work/p/tsconfig.json"
printf 'export const a: number = 1;\nexport function f(x: string) { return x.length; }\n' > "$work/p/src/a.ts"
fails=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then echo "ok   $1 (exit $3)"; else echo "FAIL $1: expected exit $2, got $3"; fails=$((fails + 1)); fi
}
run() { # tsrs-binary args...
  local bin="$1"; shift
  TSRS="$bin" python3 "$here/run.py" "$work/p" --name "buildinfo-test-$$" "$@" > "$work/out.txt" 2>&1
}
wrapper() { # name, shell snippet run after tsrs with $out set to its --outDir
  cat > "$work/$1" <<EOF
#!/usr/bin/env bash
"$TSRS" "\$@"; st=\$?
out=""; while [ \$# -gt 0 ]; do [ "\$1" = "--outDir" ] && out="\$2"; shift; done
$2
exit \$st
EOF
  chmod +x "$work/$1"
}

run "$TSRS" --buildinfo; check "default dirs, JS + .d.ts" 0 $?
grep -q "0 different, 0 missing, 0 extra" "$work/out.txt" || { echo "FAIL: unexpected report"; cat "$work/out.txt"; fails=$((fails + 1)); }
run "$TSRS" --buildinfo -- --emitDeclarationOnly; check "default dirs, declarations only" 0 $?

wrapper alter 'sed -i "s/\"version\":\"[0-9a-f]\{4\}/\"version\":\"0000/" "$out.tsbuildinfo"'
run "$work/alter" --buildinfo; check "altered tsbuildinfo" 1 $?
grep -q "different" "$work/out.txt" && grep -q "out.tsbuildinfo" "$work/out.txt" || { echo "FAIL: altered buildinfo not reported"; fails=$((fails + 1)); }

wrapper drop 'rm -f "$out.tsbuildinfo"'
run "$work/drop" --buildinfo; check "deleted tsbuildinfo" 1 $?
wrapper js 'echo "// changed" >> "$out/a.js"'
run "$work/js" --buildinfo; check "altered .js output" 1 $?

run "$TSRS" --buildinfo --go-out "$work/x/ref" --rs-out "$work/x/rs"; check "different output basenames" 2 $?

# Equal or nested output trees would let tsrs overwrite the reference output (false green): rejected before running.
run "$work/js" --buildinfo --go-out "$work/same/out" --rs-out "$work/same/out"; check "equal output dirs" 2 $?
run "$work/js" --go-out "$work/same/out" --rs-out "$work/same/out"; check "equal output dirs, no --buildinfo" 2 $?
run "$work/js" --go-out "$work/nest/out" --rs-out "$work/nest/out/inner/out"; check "nested output dirs" 2 $?
[ ! -e "$work/same" ] && [ ! -e "$work/nest" ] || { echo "FAIL: a rejected run wrote or created output"; fails=$((fails + 1)); }

echo "test_run_buildinfo: $fails failure(s)"
[ $fails -eq 0 ]
