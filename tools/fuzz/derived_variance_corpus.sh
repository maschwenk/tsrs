#!/usr/bin/env bash
# Shadow-mode runs of TSRS_DERIVED_VARIANCE over real projects with generic class / interface hierarchies
# (notes/fuzz-derived-variance.md). Clones each project at its pinned commit into $CORPUS (default /tmp/corpus),
# installs dependencies (scripts off), and type-checks each listed tsconfig with TSRS_DERIVED_VARIANCE=shadow.
#   tools/fuzz/derived_variance_corpus.sh [clone|install|run] [path/to/tsrs]
set -uo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
tsrs="${2:-$repo/target/release/tsrs}"
corpus="${CORPUS:-/tmp/corpus}"
# name url commit
PINS="
zod https://github.com/colinhacks/zod 0b216ef674e297ebe41d8bf902262e56f8755822
typeorm https://github.com/typeorm/typeorm c64a1f052fc39f6688b6b73b83d065d7147ba8bb
mikro-orm https://github.com/mikro-orm/mikro-orm 189c1688ffd6fb47fa5ec5c209f68d199784efad
rxjs https://github.com/ReactiveX/rxjs 54796b38a57e6309f9861e174737479bb3f63f61
effect https://github.com/Effect-TS/effect b1d200c40a1dad69def51ebdbf0a1a612a12b8ac
mobx-state-tree https://github.com/mobxjs/mobx-state-tree 7a7bfa4b562de795dfca763db19ddb80339569e3
fp-ts https://github.com/gcanti/fp-ts c0a6472121c67a2b083e62fcff13e7d022e39d8f
three-types https://github.com/three-types/three-ts-types 5d4b38df3679e8383063a75539b6c71cdff4b66a
sequelize https://github.com/sequelize/sequelize 1fff58fc32a6efbd6d3c6ce625f0d31e83f2a84f
kysely https://github.com/kysely-org/kysely 78294ab71a44bd27df75b52cd1457a773795deff
drizzle-orm https://github.com/drizzle-team/drizzle-orm 15454dbe49d827c6081f3d0231e2e7985e517295
trpc https://github.com/trpc/trpc d756e591a5e37ef20b8d75ecd4d736c195497289
nest https://github.com/nestjs/nest 35142c3eca8edaaf6abc5984d915da2fbd458aa2
vue-core https://github.com/vuejs/core 4ab865a848a1da3d10fb674f857e5fff13094644
io-ts https://github.com/gcanti/io-ts 864a3a2f03c5d7b974afeb1da0faf46c21758779
"
# Projects to type-check: a directory with a tsconfig.json, then compiler options / top-level fields to override for
# TypeScript 7 (removed options: node10 resolution, ES5, baseUrl) or for a self-contained check. The overrides are
# written to tsconfig.tsrs-dv.json (extending tsconfig.json) next to the original.
PROJECTS='
zod/packages/zod|{"compilerOptions":{"types":[],"lib":["es2022","dom"]},"exclude":["**/tests/**","**/*.test.ts","**/benchmarks/**"]}
typeorm/packages/typeorm|{"compilerOptions":{"module":"esnext","moduleResolution":"bundler"}}
mikro-orm|{"include":["packages/*/src"]}
rxjs/packages/rxjs|{"compilerOptions":{"paths":{"@rxjs/observable-polyfill":["../observable-polyfill/src/index.ts"]}},"exclude":["**/*.spec.ts","docs"]}
effect/packages/effect|{"compilerOptions":{"composite":false,"incremental":false,"declaration":false,"declarationMap":false}}
mobx-state-tree|{"compilerOptions":{"module":"esnext","moduleResolution":"bundler","lib":["es2022","dom"]}}
fp-ts|{"compilerOptions":{"target":"es2015","module":"esnext","moduleResolution":"bundler"}}
three-types/types/three|{}
sequelize/packages/core|{}
kysely|{}
drizzle-orm/drizzle-orm|{"compilerOptions":{"baseUrl":null,"paths":{"~/*":["./src/*"]}}}
trpc/packages/server|{}
trpc/packages/client|{}
nest|{}
vue-core|{"compilerOptions":{"composite":false,"incremental":false,"declaration":true,"noEmit":true}}
io-ts|{"compilerOptions":{"target":"es2015","module":"esnext","moduleResolution":"bundler"}}
'
cmd="${1:-run}"
mkdir -p "$corpus"
case "$cmd" in
clone)
  echo "$PINS" | while read -r name url commit; do
    [ -n "$name" ] || continue
    [ -d "$corpus/$name" ] || { git init -q "$corpus/$name" && git -C "$corpus/$name" fetch -q --depth 1 "$url" "$commit" && git -C "$corpus/$name" checkout -q FETCH_HEAD; }
    echo "$name $(git -C "$corpus/$name" rev-parse HEAD)"
  done ;;
install)
  export COREPACK_ENABLE_DOWNLOAD_PROMPT=0 CI=1
  echo "$PINS" | while read -r name _url _commit; do
    [ -n "$name" ] || continue
    ( cd "$corpus/$name" || exit
      if [ -f pnpm-lock.yaml ]; then corepack pnpm install --ignore-scripts --frozen-lockfile=false
      elif [ -f yarn.lock ]; then corepack yarn install --mode=skip-build
      elif [ -f package-lock.json ]; then npm install --ignore-scripts --legacy-peer-deps --no-audit --no-fund
      else bun install --ignore-scripts; fi >"$corpus/$name.install.log" 2>&1
      echo "$name install exit $?" )
  done ;;
run)
  echo "$PROJECTS" | while IFS='|' read -r p overrides; do
    [ -n "$p" ] || continue
    out="$corpus/$(echo "$p" | tr / _).shadow"
    rm -f "$out.log"
    python3 -c 'import json,sys; o=json.loads(sys.argv[1]); o["extends"]="./tsconfig.json"; print(json.dumps(o))' "$overrides" >"$corpus/$p/tsconfig.tsrs-dv.json"
    start=$(date +%s)
    (cd "$corpus/$p" && TSRS_DERIVED_VARIANCE=shadow TSRS_DERIVED_VARIANCE_LOG="$out.log" "$tsrs" -p tsconfig.tsrs-dv.json --noEmit --pretty false >"$out.out" 2>"$out.err")
    code=$?
    decisions=$(cat "$out.log" 2>/dev/null | grep -c '^D ')
    bases=$(cat "$out.log" 2>/dev/null | grep '^D '  | awk '{print $2}' | sort -u | wc -l)
    errors=$(grep -c 'error TS' "$out.out")
    echo "$p exit=$code decisions=$decisions bases=$bases errors=$errors $(grep 'shadow:.*decisions' "$out.err") $(( $(date +%s) - start ))s"
    rm -f "$out.log.tmp"
  done ;;
esac
