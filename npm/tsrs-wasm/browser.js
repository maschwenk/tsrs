// Browser entry point: `tsc(args, { files, cwd, diagnostics, env })` over in-memory files. Run it in a Web Worker
// (examples/browser): the checker recurses deeply and a worker's stack is larger than the page's. The module is
// compiled once (streaming when the server sends application/wasm); each call gets a new instance, created
// asynchronously.
import { makeRequest, memoryFileSystem, parseReply, runTscAsync } from "./core.js";

let loading;

/** Fetches and compiles tsrs.wasm (next to this file by default). A failed load is not cached. */
export function loadModule(url = new URL("./tsrs.wasm", import.meta.url)) {
    loading ??= (async () => {
        const response = await fetch(url);
        if (!response.ok) throw new Error(`tsrs-wasm: fetching ${url} failed with ${response.status}`);
        if (WebAssembly.compileStreaming && (response.headers.get("content-type") ?? "").startsWith("application/wasm")) {
            return WebAssembly.compileStreaming(response);
        }
        return WebAssembly.compile(await response.arrayBuffer());
    })();
    loading.catch(() => {
        loading = undefined;
    });
    return loading;
}

const decoder = new TextDecoder();

/** options.module: an already compiled WebAssembly.Module (otherwise `loadModule(options.url)`). */
export async function tsc(args, options = {}) {
    const module = options.module ?? (await loadModule(options.url));
    const memory = memoryFileSystem(options.files ?? {}, { caseInsensitive: !!options.caseInsensitive });
    const json = options.diagnostics === "json";
    const out = [];
    const err = [];
    const request = makeRequest([...args], { cwd: options.cwd ?? "/", json, caseInsensitive: !!options.caseInsensitive, tty: !!options.tty });
    const result = await runTscAsync(module, request, memory, { env: options.env ?? {}, stdout: (c) => out.push(c), stderr: (c) => err.push(c) });
    const text = (chunks) => chunks.map((c) => decoder.decode(c, { stream: true })).join("") + decoder.decode();
    return {
        exitCode: result.exitCode,
        stdout: text(out),
        stderr: text(err),
        diagnostics: json ? parseReply(result.reply) : undefined,
        files: Object.fromEntries(memory.written),
        memoryBytes: result.memoryBytes,
    };
}
