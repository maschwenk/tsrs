#!/usr/bin/env bash
# Regression test for monorepo.sh's exit status (fail closed). Builds a tiny throwaway monorepo (a git repo under
# a temp dir) and runs the real script with the real compilers:
#   1. clean run (one package with diagnostics: both compilers exit 2) -> exit 0
#   2. a tsrs whose output differs (wrapper appends a line to one emitted .d.ts) -> exit 1
#   3. dropped / duplicate / malformed result rows, fed to summarize.py from run 1's real report -> exit 1
#   4. empty selection -> nonzero
#
#   TSGO=<reference tsgo> TSRS=<tsrs binary> tools/oracle/emit/test_monorepo.sh
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${TSGO:?set TSGO}" "${TSRS:?set TSRS}"
work="$(mktemp -d /tmp/emit-oracle-test.XXXXXX)"
trap 'rm -rf "$work"' EXIT
root="$work/repo"
mkdir -p "$root/packages/clean/src" "$root/packages/diag/src"
for p in clean diag; do
  echo '{ "name": "'$p'", "scripts": { "build": "tsc" } }' > "$root/packages/$p/package.json"
  echo '{ "compilerOptions": { "target": "es2022", "module": "nodenext", "declaration": true, "strict": true, "rootDir": "src" }, "include": ["src"] }' > "$root/packages/$p/tsconfig.json"
done
echo 'export interface A { x: number }
export function f(a: A): A { return a; }' > "$root/packages/clean/src/a.ts"
echo 'export const n: number = "not a number";' > "$root/packages/diag/src/b.ts"
git -C "$root" init -q && git -C "$root" add -A && git -C "$root" -c user.name=t -c user.email=t@t commit -qm init

flags=(-- --emitDeclarationOnly --sourceMap false --declarationMap false)
fails=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then echo "ok   $1 (exit $3)"; else echo "FAIL $1: expected exit $2, got $3"; fails=$((fails + 1)); fi
}
run() { # out-dir-suffix tsrs-binary args...
  local sfx="$1" bin="$2"; shift 2
  OUT_GO="$work/go$sfx" OUT_RS="$work/rs$sfx" TSRS="$bin" "$here/monorepo.sh" "$root" -j 2 "$@" > "$work/report$sfx.txt" 2>&1
}

run 1 "$TSRS" "${flags[@]}"; st=$?
check "clean run" 0 $st
grep -q "packages: 2 selected, 2 identical, 0 failed" "$work/report1.txt" || { echo "FAIL clean report:"; cat "$work/report1.txt"; fails=$((fails + 1)); }
grep -q "exit 2/2" "$work/report1.txt" || { echo "FAIL: diag package should exit 2 on both sides"; fails=$((fails + 1)); }

cat > "$work/tsrs-tamper" <<EOF
#!/usr/bin/env bash
"$TSRS" "\$@"; st=\$?
out=""; while [ \$# -gt 0 ]; do [ "\$1" = "--outDir" ] && out="\$2"; shift; done
f="\$(find "\$out" -name '*.d.ts' 2>/dev/null | sort | head -1)"; [ -n "\$f" ] && echo "// tampered" >> "\$f"
exit \$st
EOF
chmod +x "$work/tsrs-tamper"
run 2 "$work/tsrs-tamper" "${flags[@]}"; st=$?
check "differing output" 1 $st
grep -q "FAIL(files)" "$work/report2.txt" || { echo "FAIL: report2 lacks FAIL(files)"; cat "$work/report2.txt"; fails=$((fails + 1)); }

sel="$work/rs1.selected.txt"; res="$work/rs1.results.jsonl"
head -n 1 "$res" > "$work/dropped.jsonl"
python3 "$here/summarize.py" "$sel" "$work/dropped.jsonl" > "$work/report3.txt" 2>&1; check "dropped result" 1 $?
grep -q "no result for" "$work/report3.txt" || { echo "FAIL: report3 lacks 'no result for'"; fails=$((fails + 1)); }
cat "$res" "$res" > "$work/dup.jsonl"
python3 "$here/summarize.py" "$sel" "$work/dup.jsonl" > /dev/null 2>&1; check "duplicate result" 1 $?
{ cat "$res"; echo '{"name": "packages__clean", "identical": "x"}'; } > "$work/bad.jsonl"
python3 "$here/summarize.py" "$sel" "$work/bad.jsonl" > /dev/null 2>&1; check "malformed result" 1 $?
{ cat "$res"; grep '"packages__clean"' "$res" | sed 's/"packages__clean"/"packages__other"/'; } > "$work/unexpected.jsonl"
python3 "$here/summarize.py" "$sel" "$work/unexpected.jsonl" > /dev/null 2>&1; check "unexpected result" 1 $?

run 4 "$TSRS" --filter 'no-such-package' "${flags[@]}"; st=$?
[ $st -ne 0 ] && echo "ok   empty selection (exit $st)" || { echo "FAIL empty selection exited 0"; fails=$((fails + 1)); }

[ -z "$(git -C "$root" status --short)" ] || { echo "FAIL: the test repo was written"; fails=$((fails + 1)); }
echo "test_monorepo: $fails failure(s)"
[ $fails -eq 0 ]
