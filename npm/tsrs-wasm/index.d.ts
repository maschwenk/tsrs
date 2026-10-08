import type { Diagnostic, WasmModule } from "./core.js";

export type { Diagnostic, HostFileSystem, MemoryFileSystem, WasmModule } from "./core.js";

export interface TscOptions {
    /** The current directory (Node: process.cwd() on the real file system; "/" with `files`). */
    cwd?: string;
    /** Run over these in-memory files (absolute path -> contents) instead of the real file system. */
    files?: Record<string, string | Uint8Array>;
    /** Environment variables the compiler sees (none by default). */
    env?: Record<string, string | undefined>;
    /** "json": return the diagnostics as `diagnostics` instead of printing them (not with --build). */
    diagnostics?: "text" | "json";
    /** stdout is a terminal (the default for --pretty). */
    tty?: boolean;
    /** Node: "inherit" writes to this process's fd 1 / fd 2; collected into the result otherwise. */
    stdout?: "inherit" | "collect";
    stderr?: "inherit" | "collect";
    caseInsensitive?: boolean;
    /** Node: the worker's stack in MB (default 256). */
    stackSizeMb?: number;
    /** Browser: an already compiled module, or where to fetch tsrs.wasm from. */
    module?: WasmModule;
    url?: string | { readonly href: string };
}

export interface TscResult {
    exitCode: number;
    stdout: string;
    stderr: string;
    diagnostics?: Diagnostic[];
    /** With `files`: what the compiler wrote, path -> text. */
    files?: Record<string, string>;
    /** Linear memory at the end of the run, in bytes (the peak; it never shrinks). */
    memoryBytes: number;
}

/** Runs tsc with these arguments. Each call uses a fresh module instance. */
export declare function tsc(args: string[], options?: TscOptions): Promise<TscResult>;

/** Node: the compiled module. Browser: loads and compiles it (from `url`, default ./tsrs.wasm next to browser.js). */
export declare function loadModule(url?: string | { readonly href: string }): WasmModule | Promise<WasmModule>;
