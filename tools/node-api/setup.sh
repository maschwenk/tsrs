#!/usr/bin/env bash
# Prepares the Node API parity harness:
#   1. ts-ref/ at exactly the microsoft/TypeScript commit pinned in Cargo.toml ([workspace.metadata.typescript]),
#      with packages/typescript (client + upstream tests) and tsc/ (Go server) checked out;
#   2. the upstream client's dev dependencies (npm ci in ts-ref, packages/typescript workspace only);
#   3. the Go oracle server built from ts-ref/tsc (not from npm) at tools/node-api/.work/oracle/tsc.
# An existing ts-ref at a different commit is an error; this script never moves someone else's checkout.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
ref="${TS_REF:-$root/ts-ref}"
pin="$(node "$root/npm/build.mjs" --print-typescript-commit)"
[[ "$pin" =~ ^[0-9a-f]{40}$ ]] || { echo "could not read pinned commit" >&2; exit 2; }

if [[ ! -d "$ref/.git" ]]; then
    mkdir -p "$ref"
    git -C "$ref" init -q
    git -C "$ref" remote add origin https://github.com/microsoft/TypeScript.git
    git -C "$ref" fetch -q --depth 1 origin "$pin"
    git -C "$ref" checkout -q FETCH_HEAD
fi
head="$(git -C "$ref" rev-parse HEAD)"
if [[ "$head" != "$pin" ]]; then
    echo "ts-ref is at $head, expected $pin; fix or remove it (it is gitignored)" >&2
    exit 2
fi
# CI's conformance checkout is sparse (tsc/testdata only); widen it to what the harness needs.
if [[ "$(git -C "$ref" config --get core.sparseCheckout || true)" == "true" ]]; then
    git -C "$ref" sparse-checkout add packages/typescript tsc go.work go.work.sum package.json package-lock.json
fi

if [[ ! -d "$ref/node_modules/tinybench" ]]; then
    (cd "$ref" && npm ci --ignore-scripts --no-audit --no-fund -w packages/typescript --include-workspace-root=false)
fi

oracle="$here/.work/oracle"
if [[ ! -x "$oracle/tsc" || "$(cat "$oracle/commit" 2>/dev/null || true)" != "$pin" ]]; then
    command -v go > /dev/null || { echo "go is required to build the oracle (see ts-ref/go.work for the version)" >&2; exit 2; }
    mkdir -p "$oracle"
    exe=tsc
    [[ "$(go env GOOS)" == windows ]] && exe=tsc.exe
    (cd "$ref/tsc" && go build -o "$oracle/$exe" ./cmd/tsc)
    echo "$pin" > "$oracle/commit"
fi
echo "ts-ref $pin ready; oracle: $oracle/tsc"
