#!/usr/bin/env bash
# Builds the WebAssembly module (crates/tsrs_wasm) into npm/tsrs-wasm/tsrs.wasm and checks it.
#   tools/wasm/build.sh                 opt-level from [profile.wasm], then wasm-opt at the same level
#   OPT=z tools/wasm/build.sh           opt-level z (wasm-opt --flatten --rereloop -Oz -Oz)
#   OPT=s tools/wasm/build.sh           opt-level s + -C llvm-args=-inlinehint-threshold=150 (wasm-opt -Os)
#   WASM_OPT=none tools/wasm/build.sh   skip wasm-opt (fast development builds)
#   OUT=<file>                          also copy the module there
# Fails when a build path (worktree, $HOME, cargo registry, rust sysroot) is left in the module, when the module
# imports anything but tsrs_host.{fs,fs_take} and wasi_snapshot_preview1.*, or when it imports thread-spawn.
# notes/wasm-build.md has the sizes and how the settings were chosen.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
OPT=${OPT:-s}
CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
SYSROOT=$(rustc --print sysroot)
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}

flags="--remap-path-prefix=$ROOT=/tsrs --remap-path-prefix=$CARGO_HOME/registry/src=/cargo --remap-path-prefix=$SYSROOT/lib/rustlib/src/rust=/rustc --remap-path-prefix=$HOME=/home"
case "$OPT" in
  z) export CARGO_PROFILE_WASM_OPT_LEVEL=z ;;
  s) export CARGO_PROFILE_WASM_OPT_LEVEL=s; flags="$flags -C llvm-args=-inlinehint-threshold=150" ;;
  *) echo "OPT must be z or s" >&2; exit 2 ;;
esac
# The target-only variable: native build scripts and proc macros keep their flags.
CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="$flags" cargo build --profile wasm -p tsrs_wasm --target wasm32-wasip1 --locked
RAW="$TARGET_DIR/wasm32-wasip1/wasm/tsrs_wasm.wasm"
WORK="$TARGET_DIR/wasm-build"
mkdir -p "$WORK"
cp "$RAW" "$WORK/tsrs.raw.wasm"

if [ "${WASM_OPT:-}" = none ]; then
  cp "$RAW" "$WORK/tsrs.wasm"
else
  # The features rustc enabled, from the module's target_features section.
  features=$(node -e '
    const m = new WebAssembly.Module(require("fs").readFileSync(process.argv[1]));
    const s = WebAssembly.Module.customSections(m, "target_features")[0];
    const out = [];
    if (s) {
      const b = new Uint8Array(s); let i = 0;
      const leb = () => { let r = 0, sh = 0, x; do { x = b[i++]; r |= (x & 127) << sh; sh += 7; } while (x & 128); return r; };
      const n = leb();
      for (let k = 0; k < n; k++) { const prefix = String.fromCharCode(b[i++]); const len = leb(); const name = new TextDecoder().decode(b.subarray(i, i + len)); i += len; if (prefix === "+") out.push("--enable-" + (name === "nontrapping-fptoint" ? "nontrapping-float-to-int" : name)); }
    }
    console.log(out.join(" "));' "$RAW")
  if [ "$OPT" = z ]; then passes="--flatten --rereloop -Oz -Oz"; else passes="-Os"; fi
  # shellcheck disable=SC2086 # word splitting of the flag lists is intended
  wasm-opt $features $passes --strip-debug --strip-producers "$RAW" -o "$WORK/tsrs.wasm"
fi
MOD="$WORK/tsrs.wasm"

# Leak check: no build path may remain.
for prefix in "$ROOT" "$HOME" "$CARGO_HOME" "$SYSROOT"; do
  if grep -qaF "$prefix" "$MOD"; then
    echo "error: $prefix is in the module (remap-path-prefix missed it)" >&2
    exit 1
  fi
done

# Imports check.
node -e '
  const m = new WebAssembly.Module(require("fs").readFileSync(process.argv[1]));
  const wasi = [];
  let bad = false;
  for (const i of WebAssembly.Module.imports(m)) {
    if (i.module === "tsrs_host" && (i.name === "fs" || i.name === "fs_take")) continue;
    if (i.module === "wasi_snapshot_preview1") { wasi.push(i.name); continue; }
    console.error("error: unexpected import " + i.module + "." + i.name); bad = true;
  }
  if (wasi.includes("thread-spawn") || WebAssembly.Module.imports(m).some((i) => i.name === "thread-spawn")) { console.error("error: thread-spawn import"); bad = true; }
  console.log("WASI imports: " + wasi.join(" "));
  process.exit(bad ? 1 : 0);' "$MOD"

size() {
  # gzip -9 and brotli quality 11 (node's zlib; no brotli binary needed).
  node -e '
    const zlib = require("zlib"); const b = require("fs").readFileSync(process.argv[1]);
    const br = zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11, [zlib.constants.BROTLI_PARAM_SIZE_HINT]: b.length } }).length;
    const gz = zlib.gzipSync(b, { level: 9 }).length;
    console.log(process.argv[2].padEnd(20) + " raw " + String(b.length).padStart(10) + "  gzip -9 " + String(gz).padStart(10) + "  brotli -q 11 " + String(br).padStart(10));' "$1" "$2"
}
echo "opt-level $OPT"
size "$WORK/tsrs.raw.wasm" "before wasm-opt"
size "$MOD" "after wasm-opt"
libs=$(python3 -c '
import re,sys,os
src=open("crates/tsrs_vfs/src/bundled/embed_generated.rs").read()
total=0
for m in re.finditer(r"include_str!\(\"([^\"]+)\"\)", src):
    total+=os.path.getsize(os.path.join("crates/tsrs_vfs/src/bundled", m.group(1)))
print(total)')
echo "embedded libs: $libs bytes raw"
cp "$MOD" npm/tsrs-wasm/tsrs.wasm
if [ -n "${OUT:-}" ]; then cp "$MOD" "$OUT"; fi
echo "wrote npm/tsrs-wasm/tsrs.wasm"
