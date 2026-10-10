#!/usr/bin/env bash
# Install the pinned standalone release directly: no npm bootstrap, and only one pnpm on PATH.
# pnpm/action-setup adds both a bootstrap and an updated binary; Depot resolves the bootstrap first.
set -euo pipefail

version=12.10.1
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) platform=darwin-arm64; checksum=a617ae6a55c8b3cd494c611ada88d65cb7c3cf6d7828e91b2f44b4bed1bee217 ;;
  Darwin-x86_64) platform=darwin-x64; checksum=be78adee6612302667836dcde5a344c6ec03c231de34091868e6f272a6484fe6 ;;
  Linux-aarch64) platform=linux-arm64; checksum=18dbb118de1dbef0e4516c17bf847d6ee206fc4deffb2bb488ec8e1d2814e078 ;;
  Linux-x86_64) platform=linux-x64; checksum=502a275984a4cd01e5579ac72049a241091cac5118e44becf84951130c3e079e ;;
  *) echo "unsupported pnpm runner: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
test "$(node -p 'require("./npm/package.json").packageManager')" = "pnpm@$version"

pnpm_home="$RUNNER_TEMP/tsrs-pnpm"
archive="$RUNNER_TEMP/pnpm-$platform.tar.gz"
mkdir -p "$pnpm_home/bin"
curl --fail --location --silent --show-error --retry 3 \
  "https://github.com/pnpm/pnpm/releases/download/v$version/pnpm-$platform.tar.gz" -o "$archive"
node -e '
  const hash = require("node:crypto").createHash("sha256").update(require("node:fs").readFileSync(process.argv[1])).digest("hex");
  if (hash !== process.argv[2]) throw new Error("pnpm archive checksum mismatch");
' "$archive" "$checksum"
tar -xzf "$archive" -C "$pnpm_home/bin"
test "$("$pnpm_home/bin/pnpm" --pm-on-fail=ignore --version)" = "$version"
echo "$pnpm_home/bin" >> "$GITHUB_PATH"
echo "PNPM_HOME=$pnpm_home" >> "$GITHUB_ENV"
echo "Installed pnpm $version ($platform)"
