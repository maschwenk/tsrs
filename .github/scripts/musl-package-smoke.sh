#!/usr/bin/env bash
# Run the installed CLI and both JS API checks with musl Node on the runner's native architecture.
# The main package and platform tarballs were packed on the Ubuntu build host.
set -euo pipefail

dist=$(cd "$1" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
pnpm_version=$(node -p 'require("./npm/package.json").packageManager.replace(/^pnpm@/, "")')
docker run --rm -v "$repo:/repo:ro" -v "$dist:/packages:ro" -w /repo -e "PNPM_VERSION=$pnpm_version" \
  node:24-alpine sh -ec '
    npm install --global "pnpm@$PNPM_VERSION"
    test "$(pnpm --version)" = "$PNPM_VERSION"
    node npm/sdk/smoke-consumer.mjs --dist /packages
    node tools/node-api/consumer.mjs --dist /packages
  '
