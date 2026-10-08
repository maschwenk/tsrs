// Shared helpers: temp projects, the native tsrs binary (TSRS_NATIVE, default <repo>/target/release/tsrs), the bin.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const pkg = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const bin = path.join(pkg, "bin/tsrs-wasm.js");
export const native = process.env.TSRS_NATIVE ?? path.resolve(pkg, "../../target/release/tsrs");

/** A fresh temp directory (real path) with these files. */
export function project(files) {
    const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-wasm-test-")));
    for (const [rel, text] of Object.entries(files)) {
        fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
        fs.writeFileSync(path.join(dir, rel), text);
    }
    return dir;
}

export function runNative(args, cwd) {
    const r = spawnSync(native, [...args, "--singleThreaded"], { cwd, env: { TZ: "UTC" } });
    return { exitCode: r.status, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}

export function runBin(args, cwd) {
    const r = spawnSync(process.execPath, [bin, ...args], { cwd, env: { ...process.env, TZ: "UTC" } });
    return { exitCode: r.status, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}

/** Every file under dir: relative path -> bytes. */
export function tree(dir, base = dir, out = {}) {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) tree(p, base, out);
        else out[path.relative(base, p)] = fs.readFileSync(p);
    }
    return out;
}

export function rm(dir) {
    spawnSync("chmod", ["-R", "u+w", dir]);
    fs.rmSync(dir, { recursive: true, force: true });
}
