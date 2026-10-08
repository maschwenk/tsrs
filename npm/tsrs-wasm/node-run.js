// One tsc run on the current thread: the request, the file system (real or in-memory) and the output streams.
import fs from "node:fs";
import { makeRequest, memoryFileSystem, parseReply, runTsc } from "./core.js";
import { isCaseInsensitive, nodeFileSystem } from "./node-fs.js";

const SLEEP = new Int32Array(new SharedArrayBuffer(4));

/** Writes all of `chunk` to `fd`, waiting out EAGAIN (a non-blocking pipe) and dropping the rest on EPIPE. */
export function writeAll(fd, chunk) {
    let off = 0;
    while (off < chunk.length) {
        try {
            off += fs.writeSync(fd, chunk, off, chunk.length - off);
        } catch (e) {
            if (e.code === "EAGAIN") {
                Atomics.wait(SLEEP, 0, 0, 1);
                continue;
            }
            if (e.code === "EPIPE") return;
            throw e;
        }
    }
}

function sink(mode, fd, chunks) {
    if (mode === "inherit") return (chunk) => writeAll(fd, chunk);
    return (chunk) => chunks.push(chunk);
}

/** hooks: { beforeRun(memory), afterRun(memory) } (see core.js runTsc). */
export function run(module, options, hooks = {}) {
    const caseInsensitive = options.caseInsensitive ?? (options.files ? false : isCaseInsensitive());
    const memory = options.files ? memoryFileSystem(options.files, { caseInsensitive }) : null;
    const host = memory ?? nodeFileSystem();
    const cwd = options.cwd ?? (memory ? "/" : process.cwd());
    const json = options.diagnostics === "json";
    const out = [];
    const err = [];
    const request = makeRequest(options.args, { cwd, json, caseInsensitive, tty: !!options.tty });
    const result = runTsc(module, request, host, {
        env: options.env ?? {},
        stdout: sink(options.stdout, 1, out),
        stderr: sink(options.stderr, 2, err),
        ...hooks,
    });
    const text = (chunks) => Buffer.concat(chunks).toString("utf8");
    return {
        exitCode: result.exitCode,
        stdout: text(out),
        stderr: text(err),
        diagnostics: json ? parseReply(result.reply) : undefined,
        files: memory ? Object.fromEntries(memory.written) : undefined,
        memoryBytes: result.memoryBytes,
    };
}
