#!/usr/bin/env bash
# Head-to-head on Linux x64: tsgo 7.0.2, tsc-rs (Theo Browne's pingdotgg/ts-rust, npm tsc-rs), tsrs (npm
# @maschwenk/tsrs) and `bun check`, on the six real-world apps from the tsc-rs README (same commits and the same
# config fixes, scripts/bench-apps/setup.sh there). Each tool at its own default thread count, `--noEmit
# --incremental false`, hyperfine median of RUNS runs; one extra run per tool records errors and peak RSS.
# usage: bench/tscrs-compare.sh <work-dir> [app...]    env: RUNS (5), TSRS_VERSION, TSCRS_VERSION, BUN (path)
set -uo pipefail
[[ $# -ge 1 ]] || { sed -n '8p' "$0" >&2; exit 2; }
mkdir -p "$1"; work=$(cd "$1" && pwd); shift
RUNS=${RUNS:-5}; TSRS_VERSION=${TSRS_VERSION:-latest}; TSCRS_VERSION=${TSCRS_VERSION:-0.1.0}
BUN=${BUN:?path to bun canary}
cd "$work"; mkdir -p repos logs results
[[ -f package.json ]] || echo '{ "private": true }' >package.json
npm i -q --no-audit --no-fund "tsc-rs@$TSCRS_VERSION" "@maschwenk/tsrs@$TSRS_VERSION" typescript@7.0.2

RS=$(find node_modules/@tsc-rs/linux-x64 -type f -name tsc -perm -u+x | head -1)
GO=$(find node_modules/@typescript -path '*linux-x64*' -type f -name tsc -perm -u+x | head -1)
TSRS=$(find node_modules/@maschwenk -path '*linux-x64*' -type f -name tsrs -perm -u+x | head -1)
for v in RS GO TSRS; do [[ -n ${!v} ]] || { echo "binary not found: $v" >&2; exit 1; }; declare "$v=$work/${!v}"; done
{ echo "tsgo: $($GO --version)"; echo "tsc-rs: $($RS --version)"; echo "tsrs: $($TSRS --version)"
  echo "bun: $($BUN --revision)"; echo "tsrs npm: $(node -p "require('./node_modules/@maschwenk/tsrs/package.json').version")"
  echo "tsc-rs npm: $(node -p "require('./node_modules/tsc-rs/package.json').version")"; } | tee results/versions.txt

checkout() { [[ -d repos/$1 ]] && return
  git init -q "repos/$1" && git -C "repos/$1" fetch -q --depth 1 "$2" "$3" && git -C "repos/$1" checkout -q FETCH_HEAD; }
install() { cd "repos/$1"
  if [[ -f pnpm-lock.yaml ]]; then pnpm install --frozen-lockfile --ignore-scripts
  elif [[ -f yarn.lock ]]; then corepack yarn install --frozen-lockfile --ignore-scripts
  else npm ci --ignore-scripts --no-audit --no-fund; fi; }
while read -r name url sha; do
  [[ $# == 0 || " $* " == *" $name "* ]] || continue
  checkout "$name" "$url" "$sha"
  (install "$name") >"logs/install-$name.log" 2>&1 || { echo "install failed: $name" >&2; tail -20 "logs/install-$name.log" >&2; exit 1; }
done <<'APPS'
vscode https://github.com/microsoft/vscode 3f07e1aba32acacb8b08ae91bfdc954b580ad1fd
sentry https://github.com/getsentry/sentry 8294650589dbd26f230c73f4ab26b62a68aede8f
playwright https://github.com/microsoft/playwright d469960fdfc461e2d5795a3fa48a58a52a91ecaf
typeorm https://github.com/typeorm/typeorm c64a1f052fc39f6688b6b73b83d065d7147ba8bb
excalidraw https://github.com/excalidraw/excalidraw 53973c3a423fbd75a4ce68107786b4fcb90e4968
trpc https://github.com/trpc/trpc d756e591a5e37ef20b8d75ecd4d736c195497289
APPS

# Config fixes from the tsc-rs README: each app checks with 0 errors under tsc 7 afterwards.
if [[ -d repos/vscode && ! -d repos/vscode/node_modules/electron ]]; then
  mkdir -p tmp-electron && (cd tmp-electron && echo '{ "private": true }' >package.json &&
    npm i -q --ignore-scripts --no-audit --no-fund electron@43.7.7)
  cp -R tmp-electron/node_modules/electron repos/vscode/node_modules/electron && rm -rf tmp-electron
fi
[[ -d repos/playwright ]] && (cd repos/playwright && node utils/generate_injected.js)
[[ -d repos/excalidraw ]] && grep -v '"baseUrl"' repos/excalidraw/tsconfig.json >repos/excalidraw/tsconfig.bench.json
[[ -d repos/typeorm ]] && echo '{ "extends": "./tsconfig.json", "compilerOptions": { "module": "nodenext", "moduleResolution": "nodenext" } }' \
  >repos/typeorm/packages/typeorm/tsconfig.bench.json

APPLIST='vscode vscode src/tsconfig.json
sentry sentry tsconfig.json
playwright playwright tsconfig.json
typeorm typeorm packages/typeorm/tsconfig.bench.json
excalidraw excalidraw tsconfig.bench.json
trpc-server trpc packages/server/tsconfig.json'
names=(tsgo tsc-rs tsrs bun)
while read -r app dir cfg; do
  [[ $# == 0 || " $* " == *" $app "* || " $* " == *" $dir "* ]] || continue
  flags="-p $cfg --noEmit --incremental false"
  cmds=("$GO $flags --pretty false" "$RS $flags --pretty false" "$TSRS $flags --pretty false"
        "$BUN check $flags --no-pretty --all")
  cd "$work/repos/$dir"
  : >"$work/results/$app.errors"
  for i in "${!names[@]}"; do
    /usr/bin/time -f '%M' -o "$work/results/$app.${names[$i]}.rss" ${cmds[$i]} >"$work/results/$app.${names[$i]}.out" 2>&1
    rc=$?
    grep 'error TS' "$work/results/$app.${names[$i]}.out" | sort >"$work/results/$app.${names[$i]}.diag"
    echo "${names[$i]} rc=$rc errors=$(wc -l <"$work/results/$app.${names[$i]}.diag" | tr -d ' ') rss_kb=$(tail -1 "$work/results/$app.${names[$i]}.rss")" >>"$work/results/$app.errors"
  done
  echo "== $app: $(tr '\n' ' ' <"$work/results/$app.errors")"
  args=()
  for i in "${!names[@]}"; do args+=(-n "${names[$i]}" "${cmds[$i]}"); done
  hyperfine -N -i --warmup 1 --runs "$RUNS" --export-json "$work/results/$app.json" "${args[@]}"
done <<<"$APPLIST"
