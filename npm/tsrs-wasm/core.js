// Runtime-agnostic host for tsrs.wasm (crates/tsrs_wasm): a small WASI preview1 shim (clocks, random, stdout/stderr,
// environ, proc_exit; no file system), the host file-system bridge (`tsrs_host.fs` / `tsrs_host.fs_take`), and
// `runTsc`. Node (node.js) and browsers (browser.js) build on it. No dependencies.

export const REQUEST_JSON_DIAGNOSTICS = 1;
export const REQUEST_CASE_INSENSITIVE = 2;
export const REQUEST_TTY = 4;

const OP_READ = 0;
const OP_STAT = 1;
const OP_READ_DIR = 2;
const OP_REALPATH = 3;
const OP_WRITE = 4;
const OP_APPEND = 5;
const OP_REMOVE = 6;
const OP_CHTIMES = 7;

const ESUCCESS = 0;
const EBADF = 8;
const EINVAL = 28;
const ENOSYS = 52;

/** Thrown by the shim's `proc_exit`; `runTsc` turns it into the exit code. */
export class WasiExit extends Error {
    constructor(code) {
        super(`exit ${code}`);
        this.code = code;
    }
}

/** Raised by a HostFileSystem method for a missing path (maps to "not found" in the module). */
export class NotFoundError extends Error {}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { ignoreBOM: true });

function nowNs(realtime) {
    const ms = realtime ? performance.timeOrigin + performance.now() : performance.now();
    return BigInt(Math.round(ms * 1e6));
}

// The imports for one run, and `start(instance)`, which runs the request in that instance.
function prepare(module, request, host, io) {
    let instance;
    let memory;
    const view = () => new DataView(memory.buffer);
    const bytes = () => new Uint8Array(memory.buffer);
    const env = Object.entries(io.env ?? {}).map(([k, v]) => encoder.encode(`${k}=${v}\0`));
    const stdout = io.stdout ?? (() => {});
    const stderr = io.stderr ?? (() => {});
    const tty = (request.flags & REQUEST_TTY) !== 0;
    let staged = new Uint8Array(0);

    const wasi = {
        args_sizes_get(argc, size) {
            view().setUint32(argc, 0, true);
            view().setUint32(size, 0, true);
            return ESUCCESS;
        },
        args_get() {
            return ESUCCESS;
        },
        environ_sizes_get(count, size) {
            view().setUint32(count, env.length, true);
            view().setUint32(size, env.reduce((n, e) => n + e.length, 0), true);
            return ESUCCESS;
        },
        environ_get(ptrs, buf) {
            let p = buf;
            env.forEach((e, i) => {
                view().setUint32(ptrs + i * 4, p, true);
                bytes().set(e, p);
                p += e.length;
            });
            return ESUCCESS;
        },
        clock_time_get(id, _precision, out) {
            view().setBigUint64(out, nowNs(id === 0), true);
            return ESUCCESS;
        },
        clock_res_get(_id, out) {
            view().setBigUint64(out, 1000n, true);
            return ESUCCESS;
        },
        random_get(ptr, len) {
            for (let off = 0; off < len; off += 65536) {
                crypto.getRandomValues(new Uint8Array(memory.buffer, ptr + off, Math.min(65536, len - off)));
            }
            return ESUCCESS;
        },
        fd_write(fd, iovs, count, written) {
            if (fd !== 1 && fd !== 2) {
                return EBADF;
            }
            let total = 0;
            for (let i = 0; i < count; i++) {
                const ptr = view().getUint32(iovs + i * 8, true);
                const len = view().getUint32(iovs + i * 8 + 4, true);
                const chunk = bytes().slice(ptr, ptr + len);
                (fd === 1 ? stdout : stderr)(chunk);
                total += len;
            }
            view().setUint32(written, total, true);
            return ESUCCESS;
        },
        fd_fdstat_get(fd, out) {
            if (fd > 2) {
                return EBADF;
            }
            // filetype: 2 = character device (a tty), 0 = unknown.
            const v = view();
            for (let i = 0; i < 24; i++) {
                v.setUint8(out + i, 0);
            }
            v.setUint8(out, fd === 1 && tty ? 2 : 0);
            return ESUCCESS;
        },
        fd_prestat_get() {
            return EBADF;
        },
        fd_prestat_dir_name() {
            return EBADF;
        },
        fd_close() {
            return EBADF;
        },
        proc_exit(code) {
            throw new WasiExit(code);
        },
        sched_yield() {
            return ESUCCESS;
        },
        poll_oneoff() {
            return ESUCCESS;
        },
    };

    const text = (ptr, len) => decoder.decode(bytes().subarray(ptr, ptr + len));
    const splitNul = (ptr, len) => {
        const b = bytes().subarray(ptr, ptr + len);
        const i = b.indexOf(0);
        return [decoder.decode(b.subarray(0, i < 0 ? len : i)), b.slice(i < 0 ? len : i + 1)];
    };
    const stage = (data) => {
        staged = typeof data === "string" ? encoder.encode(data) : data;
        return staged.length;
    };
    const fsCall = (op, ptr, len) => {
        try {
            switch (op) {
                case OP_READ:
                    return stage(host.readFile(text(ptr, len)));
                case OP_STAT: {
                    const s = host.stat(text(ptr, len));
                    return stage(`${s.kind} ${s.size} ${s.mtimeNs}`);
                }
                case OP_READ_DIR:
                    return stage(host.readDir(text(ptr, len)).map(([kind, name]) => `${kind}${name}\0`).join(""));
                case OP_REALPATH:
                    return stage(host.realpath(text(ptr, len)));
                case OP_WRITE:
                case OP_APPEND: {
                    const [path, data] = splitNul(ptr, len);
                    host.writeFile(path, data, op === OP_APPEND);
                    return stage("");
                }
                case OP_REMOVE:
                    host.remove(text(ptr, len));
                    return stage("");
                case OP_CHTIMES: {
                    const [path, rest] = splitNul(ptr, len);
                    const [atime, mtime] = decoder.decode(rest).split("\0").map(BigInt);
                    host.chtimes(path, atime, mtime);
                    return stage("");
                }
                default:
                    return -2 - stage(`unknown op ${op}`);
            }
        } catch (e) {
            if (e instanceof NotFoundError) {
                return -1;
            }
            return -2 - stage(String(e?.message ?? e));
        }
    };

    const imports = { tsrs_host: { fs: fsCall, fs_take: (ptr) => bytes().set(staged, ptr) }, wasi_snapshot_preview1: {} };
    for (const imp of WebAssembly.Module.imports(module)) {
        if (imp.module === "wasi_snapshot_preview1") {
            imports.wasi_snapshot_preview1[imp.name] = wasi[imp.name] ?? (() => ENOSYS);
        }
    }
    const start = (inst) => {
        instance = inst;
        memory = instance.exports.memory;
        return finish();
    };
    return { imports, start };

    function finish() {
    let exitCode;
    let reply = new Uint8Array(0);
    try {
        instance.exports._initialize?.();
        const fields = [request.cwd, String(request.flags ?? 0), ...request.args];
        const req = encoder.encode(fields.join("\0"));
        const ptr = instance.exports.tsrs_input(req.length);
        bytes().set(req, ptr);
        io.beforeRun?.(memory);
        exitCode = instance.exports.tsrs_run();
        const out = instance.exports.tsrs_output();
        reply = bytes().slice(out, out + instance.exports.tsrs_output_len());
    } catch (e) {
        if (!(e instanceof WasiExit)) {
            throw e;
        }
        exitCode = e.code;
    }
    io.afterRun?.(memory);
    return { exitCode, reply, memoryBytes: memory.buffer.byteLength };
    }
}

/**
 * Runs one tsc invocation in a fresh instance of `module` (a compiled WebAssembly.Module).
 * request: { cwd, args, flags }; host: a HostFileSystem; io: { env, stdout(bytes), stderr(bytes), beforeRun(memory),
 * afterRun(memory) } (the run hooks let a harness inspect linear memory, e.g. the shadow-stack census).
 * Returns { exitCode, reply (Uint8Array), memoryBytes }.
 */
export function runTsc(module, request, host, io = {}) {
    const run = prepare(module, request, host, io);
    return run.start(new WebAssembly.Instance(module, run.imports));
}

/** `runTsc` with an asynchronous instantiation (browsers forbid a synchronous one of a large module on the page). */
export async function runTscAsync(module, request, host, io = {}) {
    const run = prepare(module, request, host, io);
    return run.start(await WebAssembly.instantiate(module, run.imports));
}

/** The tsc argument list plus the request flags, as `runTsc` takes them. */
export function makeRequest(args, { cwd, json = false, caseInsensitive = false, tty = false }) {
    let flags = 0;
    if (json) flags |= REQUEST_JSON_DIAGNOSTICS;
    if (caseInsensitive) flags |= REQUEST_CASE_INSENSITIVE;
    if (tty) flags |= REQUEST_TTY;
    return { cwd, args, flags };
}

function normalize(path) {
    const parts = [];
    for (const part of path.replace(/\\/g, "/").split("/")) {
        if (part === "" || part === ".") continue;
        if (part === "..") parts.pop();
        else parts.push(part);
    }
    return "/" + parts.join("/");
}

/**
 * An in-memory HostFileSystem over a map of absolute path -> contents (string or Uint8Array). Directories are implied
 * by the paths below them. `written` collects every file the compiler writes (decoded as UTF-8, BOM kept). With
 * `caseInsensitive`, lookups ignore case (as on macOS's default file system) and names keep the case they were
 * written with.
 */
export function memoryFileSystem(files = {}, { caseInsensitive = false } = {}) {
    const fold = caseInsensitive ? (p) => p.toLowerCase() : (p) => p;
    const map = new Map(); // folded path -> { path, data }
    const put = (p, data) => map.set(fold(p), { path: p, data });
    for (const [path, data] of Object.entries(files)) {
        put(normalize(path), typeof data === "string" ? encoder.encode(data) : data);
    }
    const written = new Map();
    const isDir = (folded) => {
        if (folded === "/") return true;
        const prefix = folded + "/";
        for (const key of map.keys()) {
            if (key.startsWith(prefix)) return true;
        }
        return false;
    };
    return {
        written,
        files: map,
        readFile(path) {
            const entry = map.get(fold(normalize(path)));
            if (entry === undefined) throw new NotFoundError(path);
            return entry.data;
        },
        stat(path) {
            const p = fold(normalize(path));
            const entry = map.get(p);
            if (entry !== undefined) return { kind: "f", size: entry.data.length, mtimeNs: 0n };
            if (isDir(p)) return { kind: "d", size: 0, mtimeNs: 0n };
            throw new NotFoundError(path);
        },
        readDir(path) {
            const p = fold(normalize(path));
            if (!isDir(p)) throw new NotFoundError(path);
            const prefix = p === "/" ? "/" : p + "/";
            const entries = new Map();
            for (const [key, entry] of map) {
                if (!key.startsWith(prefix)) continue;
                const rest = entry.path.slice(prefix.length);
                const slash = rest.indexOf("/");
                if (slash < 0) entries.set(rest, "f");
                else entries.set(rest.slice(0, slash), "d");
            }
            return [...entries].map(([name, kind]) => [kind, name]);
        },
        realpath(path) {
            return normalize(path);
        },
        writeFile(path, data, append) {
            const p = normalize(path);
            const prev = append ? map.get(fold(p))?.data ?? new Uint8Array(0) : new Uint8Array(0);
            const next = new Uint8Array(prev.length + data.length);
            next.set(prev);
            next.set(data, prev.length);
            put(map.get(fold(p))?.path ?? p, next);
            written.set(map.get(fold(p)).path, decoder.decode(next));
        },
        remove(path) {
            const p = fold(normalize(path));
            for (const [key, entry] of [...map]) {
                if (key === p || key.startsWith(p + "/")) {
                    map.delete(key);
                    written.delete(entry.path);
                }
            }
        },
        chtimes() {},
    };
}

/** Parses the JSON reply of a REQUEST_JSON_DIAGNOSTICS run. */
export function parseReply(reply) {
    return reply.length === 0 ? [] : JSON.parse(decoder.decode(reply));
}
