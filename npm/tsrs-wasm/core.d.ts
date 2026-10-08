/** Request flag: reply with the diagnostics as JSON (`DiagnosticResponse[]`) instead of printing them. */
export declare const REQUEST_JSON_DIAGNOSTICS: 1;
/** Request flag: the host file system is case-insensitive. */
export declare const REQUEST_CASE_INSENSITIVE: 2;
/** Request flag: stdout is a terminal (the default for `--pretty`). */
export declare const REQUEST_TTY: 4;
/** Exit status of a crashed run (a panic, or a trap such as a stack overflow or running out of memory); tsc's own are 0-4. */
export declare const EXIT_CRASHED: 5;

/** A compiled `WebAssembly.Module`, typed structurally so that these declarations need neither lib dom nor @types/node. */
export type WasmModule = object;

/** The module's `WasmMemory`. */
export interface WasmMemory {
    readonly buffer: ArrayBuffer;
}

export declare class WasiExit extends Error {
    code: number;
    constructor(code: number);
}

/** Thrown by a HostFileSystem method for a missing path. */
export declare class NotFoundError extends Error {}

export interface FileStat {
    kind: "f" | "d" | "o";
    size: number | bigint;
    mtimeNs: bigint;
}

/** The file system the module reads and writes through. Paths are absolute, with `/` separators. */
export interface HostFileSystem {
    readFile(path: string): Uint8Array;
    stat(path: string): FileStat;
    /** [kind, name] pairs; kind "l" is a symbolic link (followed by a later stat). */
    readDir(path: string): Array<["f" | "d" | "l" | "o", string]>;
    realpath(path: string): string;
    writeFile(path: string, data: Uint8Array, append: boolean): void;
    remove(path: string): void;
    chtimes(path: string, atimeNs: bigint, mtimeNs: bigint): void;
}

export interface Request {
    cwd: string;
    args: string[];
    flags: number;
}

export interface RunIO {
    env?: Record<string, string | undefined>;
    stdout?(chunk: Uint8Array): void;
    stderr?(chunk: Uint8Array): void;
    beforeRun?(memory: WasmMemory): void;
    afterRun?(memory: WasmMemory): void;
}

export interface RunResult {
    exitCode: number;
    reply: Uint8Array;
    /** Linear memory at the end of the run (it never shrinks, so this is the peak). */
    memoryBytes: number;
}

/** Runs one tsc invocation in a fresh instance of `module`. A trap gives exitCode `EXIT_CRASHED` and a message on stderr. */
export declare function runTsc(module: WasmModule, request: Request, host: HostFileSystem, io?: RunIO): RunResult;

/** `runTsc` with an asynchronous instantiation (browsers forbid a synchronous one of a large module on the page). */
export declare function runTscAsync(module: WasmModule, request: Request, host: HostFileSystem, io?: RunIO): Promise<RunResult>;

export declare function makeRequest(args: string[], options: { cwd: string; json?: boolean; caseInsensitive?: boolean; tty?: boolean }): Request;

export interface MemoryFileSystem extends HostFileSystem {
    /** Files the compiler wrote: path -> text (UTF-8, BOM kept). */
    written: Map<string, string>;
}

/** An in-memory file system over absolute path -> contents; directories are implied by the paths below them. */
export declare function memoryFileSystem(files?: Record<string, string | Uint8Array>, options?: { caseInsensitive?: boolean }): MemoryFileSystem;

export interface DiagnosticPosition {
    line: number;
    character: number;
}

/** The TypeScript API's diagnostic shape; positions are UTF-16 offsets. */
export interface Diagnostic {
    fileName?: string;
    pos: number;
    end: number;
    startPosition?: DiagnosticPosition;
    endPosition?: DiagnosticPosition;
    sourceLines?: Array<{ line: number; text: string }>;
    code: number;
    /** 0 warning, 1 error, 2 suggestion, 3 message */
    category: 0 | 1 | 2 | 3;
    source?: string;
    text: string;
    reportsUnnecessary?: boolean;
    reportsDeprecated?: boolean;
    messageChain?: Diagnostic[];
    relatedInformation?: Diagnostic[];
}

export declare function parseReply(reply: Uint8Array): Diagnostic[];
