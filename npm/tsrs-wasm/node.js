// Node entry point: `tsc(args, options)` runs tsrs.wasm in a fresh worker thread per call (the checker recurses
// deeply; a worker gets a large native stack through `resourceLimits.stackSizeMb`). The module is compiled once per
// process. Under Bun, which ignores the worker stack limit, the run stays on the calling thread.

import fs from "node:fs";
import { Worker } from "node:worker_threads";
import { run } from "./node-run.js";

export const DEFAULT_STACK_SIZE_MB = 256;

let compiled;

/** The compiled module (tsrs.wasm next to this file, or `TSRS_WASM` if set). */
export function loadModule() {
    compiled ??= new WebAssembly.Module(fs.readFileSync(process.env.TSRS_WASM || new URL("./tsrs.wasm", import.meta.url)));
    return compiled;
}

/**
 * Runs tsc. options: { cwd, files, env, diagnostics: "text" | "json", tty, stdout, stderr ("inherit" to write to
 * this process's fd 1 / 2; collected otherwise), caseInsensitive, stackSizeMb }.
 * Resolves to { exitCode, stdout, stderr, diagnostics?, files?, memoryBytes }.
 */
export function tsc(args, options = {}) {
    const module = loadModule();
    const opts = { ...options, args: [...args] };
    if (typeof Bun !== "undefined") {
        return Promise.resolve().then(() => run(module, opts));
    }
    return new Promise((resolve, reject) => {
        const worker = new Worker(new URL("./node-worker.js", import.meta.url), {
            workerData: { module, options: opts },
            resourceLimits: { stackSizeMb: options.stackSizeMb ?? DEFAULT_STACK_SIZE_MB },
            stdout: false,
            stderr: false,
        });
        let settled = false;
        worker.once("message", (result) => {
            settled = true;
            resolve(result);
        });
        worker.once("error", (e) => {
            settled = true;
            reject(e);
        });
        worker.once("exit", (code) => {
            if (!settled) reject(new Error(`tsrs-wasm worker exited with code ${code}`));
        });
    });
}
