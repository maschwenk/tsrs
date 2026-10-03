#!/usr/bin/env bash
# Monorepo emit oracle: for every package whose `build` script runs tsc, emit with tsgo and with `TSRS_EMIT=1 tsrs`
# into scratch directories and compare every emitted file byte for byte.
#
#   tools/oracle/emit/monorepo.sh [--only jsx|decorators|all] [--keep] [package-dir ...]
#
# Environment: REPO (the monorepo checkout, read-only), TSGO (reference tsgo binary), TSRS (tsrs binary),
# OUT (scratch root, default /tmp; outputs go to $OUT/emit-go/<pkg> and $OUT/emit-rs/<pkg>).
#
# Nothing is written into $REPO: outDir, declarationDir and tsBuildInfoFile are always redirected, and the script
# checks `git status --short` plus a newer-than-marker scan of the package directories before and after.
# Packages built with `tsc -b` are compiled with `-p` (the oracle compares single-project emit).
set -u
REPO=${REPO:-/root/Owner}
TSGO=${TSGO:-$REPO/node_modules/.pnpm/@typescript+typescript-linux-x64@7.1.0-dev.20260929.1/node_modules/@typescript/typescript-linux-x64/lib/tsc}
TSRS=${TSRS:-$(cd "$(dirname "$0")/../../.." && pwd)/target/release/tsrs}
OUT=${OUT:-/tmp}
ONLY=all
KEEP=0
PKGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --only) ONLY=$2; shift 2 ;;
        --keep) KEEP=1; shift ;;
        *) PKGS+=("$1"); shift ;;
    esac
done
HERE=$(cd "$(dirname "$0")" && pwd)
LIST=$OUT/emit-oracle-packages.tsv
[ -s "$LIST" ] || node "$HERE/list-packages.js" "$REPO" "$TSGO" > "$LIST"

before=$(git -C "$REPO" status --short)
marker=$(mktemp)
tot_same=0; tot_diff=0; tot_missing=0; tot_extra=0; npkg=0
printf '%-45s %9s %9s %11s %6s\n' package identical different not-emitted extra
while IFS=$'\t' read -r dir config build target module jsx expdec meta decldir; do
    if [ ${#PKGS[@]} -gt 0 ]; then
        hit=0; for p in "${PKGS[@]}"; do [ "$p" = "$dir" ] && hit=1; done; [ $hit = 1 ] || continue
    fi
    case "$ONLY" in
        jsx) [ "$jsx" != - ] || continue ;;
        decorators) [ "$expdec" = 1 ] || continue ;;
    esac
    name=${dir//\//_}
    go=$OUT/emit-go/$name; rs=$OUT/emit-rs/$name
    rm -rf "$go" "$rs" "$go.tsbuildinfo" "$rs.tsbuildinfo"
    mkdir -p "$go" "$rs"
    cfg=$REPO/$dir/$config
    dd=(); [ "$decldir" != - ] && dd=(--declarationDir)
    (cd "$REPO/$dir" && "$TSGO" -p "$cfg" --outDir "$go" ${dd[@]+"${dd[@]}" "$go"} --tsBuildInfoFile "$go.tsbuildinfo" > "$go.log" 2>&1)
    (cd "$REPO/$dir" && TSRS_EMIT=1 "$TSRS" -p "$cfg" --outDir "$rs" ${dd[@]+"${dd[@]}" "$rs"} --tsBuildInfoFile "$rs.tsbuildinfo" > "$rs.log" 2>&1)
    if [ -n "$(find "$REPO/$dir" -newer "$marker" -type f -not -path '*/node_modules/*' | head -1)" ]; then
        echo "ERROR: $dir: files were written into the repository:" >&2
        find "$REPO/$dir" -newer "$marker" -type f -not -path '*/node_modules/*' >&2
        exit 2
    fi
    same=0; diff=0; missing=0; extra=0
    while IFS= read -r f; do
        if [ ! -f "$rs/$f" ]; then missing=$((missing+1)); echo "$f" >> "$rs.missing"
        elif cmp -s "$go/$f" "$rs/$f"; then same=$((same+1))
        else diff=$((diff+1)); echo "$f" >> "$rs.different"; fi
    done < <(cd "$go" && find . -type f | sort)
    extra=$(cd "$rs" && find . -type f | while IFS= read -r f; do [ -f "$go/$f" ] || echo x; done | wc -l)
    printf '%-45s %9d %9d %11d %6d\n' "$dir" $same $diff $missing $extra
    tot_same=$((tot_same+same)); tot_diff=$((tot_diff+diff)); tot_missing=$((tot_missing+missing)); tot_extra=$((tot_extra+extra)); npkg=$((npkg+1))
    if [ $KEEP = 0 ] && [ $diff = 0 ] && [ $missing = 0 ] && [ $extra = 0 ]; then rm -rf "$go" "$rs"; fi
done < "$LIST"
printf '%-45s %9d %9d %11d %6d\n' "TOTAL ($npkg packages)" $tot_same $tot_diff $tot_missing $tot_extra

after=$(git -C "$REPO" status --short)
written=$(find "$REPO/apps" "$REPO/packages" "$REPO/kotlin" -newer "$marker" -type f -not -path '*/node_modules/*' 2>/dev/null)
rm -f "$marker"
if [ "$before" != "$after" ] || [ -n "$written" ]; then
    echo "ERROR: files were written into $REPO:" >&2
    echo "$written" >&2
    exit 2
fi
