#!/usr/bin/env node
// Transparent tracing tap for `tsc --api` / `tsrs --api` processes, used only by the parity harness.
//
//   node proxy.mjs <real-binary> [args...]        (NODE_API_TRACE_DIR=<dir> selects where JSONL traces go)
//
// Bytes are forwarded unchanged in both directions; the tap only *observes* frames so that the inventory can
// count, per pinned proto.go method, how many requests a server answered and how many it rejected. It never
// rewrites, reorders, or synthesises messages, so protocol differences reach the upstream client untouched.
// Sync mode frames are MessagePack tuples [type:u8, method:bin, payload:bin] (syncChannel.ts); async mode frames
// are LSP-style `Content-Length` JSON-RPC (vscode-jsonrpc). Windows named-pipe mode (`--pipe`) is not traced.

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const [real, ...args] = process.argv.slice(2);
const isAsync = args.includes("--async");
const traceDir = process.env.NODE_API_TRACE_DIR;

function testFileOfAncestor() {
    // node --test runs each test file in its own process; its argv names the file. Some tests spawn a nested
    // `node -e` client, so walk up the parent chain until a *.test.ts argv is found.
    let pid = process.ppid;
    for (let depth = 0; depth < 4 && pid > 1; depth++) {
        try {
            const argv = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0");
            const file = argv.find(a => /\.test\.(m?[jt]s)$/.test(a));
            if (file) return file;
            const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
            pid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
        }
        catch {
            return undefined;
        }
    }
    return undefined;
}

const testFile = traceDir ? testFileOfAncestor() : undefined;
let traceFd;
const records = [];
function record(rec) {
    if (!traceDir) return;
    records.push(JSON.stringify(rec));
    if (records.length >= 256) flush();
}
function flush() {
    if (!traceDir || records.length === 0) return;
    traceFd ??= fs.openSync(path.join(traceDir, `${process.pid}.jsonl`), "a");
    fs.writeSync(traceFd, records.join("\n") + "\n");
    records.length = 0;
}
if (traceDir) record({ kind: "start", mode: isAsync ? "async" : "sync", testFile: testFile && path.basename(testFile), test: process.env.NODE_API_TEST || undefined });

const child = spawn(real, args, { stdio: ["pipe", "pipe", "inherit"] });

// Both parsers keep only frame headers (and a bounded body prefix) in memory, then skip the rest of each
// payload as it streams past, so multi-hundred-megabyte batch responses stay linear-time.
const KEEP = 1 << 20; // async bodies up to 1 MiB are JSON-parsed; larger ones are classified from a prefix

// batchRequests carries inner method names ({"requests":[{"method":...}]}); record them so the inventory sees
// methods that the client only sends inside batches. Only the first KEEP bytes are scanned.
function innerMethods(text) {
    const out = {};
    for (const m of text.matchAll(/"method"\s*:\s*"([A-Za-z]+)"/g)) out[m[1]] = (out[m[1]] ?? 0) + 1;
    return out;
}
// Batch responses: [{"method":m,"result":...,"error":"..."}]; count per-inner-method error entries.
function innerErrors(text) {
    const out = {};
    const starts = [...text.matchAll(/\{"method"\s*:\s*"([A-Za-z]+)"/g)];
    for (let i = 0; i < starts.length; i++) {
        const seg = text.slice(starts[i].index, i + 1 < starts.length ? starts[i + 1].index : undefined);
        const err = /"error"\s*:\s*"((?:[^"\\]|\\.){0,200})/.exec(seg);
        if (err) (out[starts[i][1]] ??= []).push(err[1]);
    }
    return out;
}
function finishBatch(rec) {
    if (rec.batch === undefined) return;
    const text = Buffer.concat(rec.batch).toString("utf8");
    if (rec.kind === "request") rec.inner = innerMethods(text);
    else rec.innerErrors = innerErrors(text);
    if (rec.batchBytes < rec.bytes) rec.innerTruncated = true;
    delete rec.batch;
    delete rec.batchBytes;
}

// ── sync tuple parser ───────────────────────────────────────────────
const SYNC_TYPES = { 1: "request", 2: "callResponse", 3: "callError", 4: "response", 5: "error", 6: "call" };
function binLen(buf, off) {
    if (off >= buf.length) return undefined;
    const m = buf[off];
    if (m === 0xc4) return off + 2 <= buf.length ? [buf[off + 1], off + 2] : undefined;
    if (m === 0xc5) return off + 3 <= buf.length ? [buf.readUInt16BE(off + 1), off + 3] : undefined;
    if (m === 0xc6) return off + 5 <= buf.length ? [buf.readUInt32BE(off + 1), off + 5] : undefined;
    throw new Error(`bad bin marker 0x${m?.toString(16)}`);
}
function syncParser(dir) {
    let head = Buffer.alloc(0); // unparsed header bytes
    let skip = 0; // payload bytes still to pass
    let cur; // record being completed while its payload streams
    let prefix = []; // first bytes of an error payload
    let broken = false;
    return chunk => {
        if (!traceDir || broken) return;
        try {
            let buf = chunk;
            for (;;) {
                if (skip > 0) {
                    const n = Math.min(skip, buf.length);
                    if (cur.message !== undefined && prefix.length < 300) prefix.push(...buf.subarray(0, Math.min(n, 300 - prefix.length)));
                    if (cur.batch !== undefined && cur.batchBytes < KEEP) {
                        const take = buf.subarray(0, Math.min(n, KEEP - cur.batchBytes));
                        cur.batch.push(take);
                        cur.batchBytes += take.length;
                    }
                    skip -= n;
                    buf = buf.subarray(n);
                    if (skip > 0) return;
                    if (cur.message !== undefined) cur.message = Buffer.from(prefix).toString("utf8");
                    finishBatch(cur);
                    record(cur);
                    cur = undefined;
                    prefix = [];
                }
                if (buf.length === 0) return;
                head = head.length ? Buffer.concat([head, buf]) : buf;
                buf = Buffer.alloc(0);
                if (head.length < 2) return;
                if (head[0] !== 0x93) throw new Error(`bad tuple marker 0x${head[0].toString(16)}`);
                let type = head[1];
                let off = 2;
                if (type === 0xcc) {
                    if (head.length < 3) return;
                    type = head[2];
                    off = 3;
                }
                const n = binLen(head, off);
                if (!n) return;
                const [nameLen, nameOff] = n;
                if (nameLen > 4096) throw new Error(`method name too long (${nameLen})`);
                if (nameOff + nameLen > head.length) return;
                const p = binLen(head, nameOff + nameLen);
                if (!p) return;
                const [payLen, payOff] = p;
                const kind = SYNC_TYPES[type] ?? `type${type}`;
                cur = { dir, kind, method: head.toString("utf8", nameOff, nameOff + nameLen), bytes: payLen };
                if (kind === "error" || kind === "callError") cur.message = "";
                if ((kind === "request" || kind === "response") && cur.method === "batchRequests") {
                    cur.batch = [];
                    cur.batchBytes = 0;
                }
                buf = head.subarray(payOff);
                head = Buffer.alloc(0);
                skip = payLen;
                if (skip === 0) {
                    finishBatch(cur);
                    record(cur);
                    cur = undefined;
                }
            }
        }
        catch (e) {
            broken = true;
            record({ dir, kind: "unparseable", message: String(e.message) });
        }
    };
}

// ── async JSON-RPC parser ───────────────────────────────────────────
const idMethods = new Map(); // `${dir}:${id}` -> method; requests from either side
function classifyAsync(dir, body, len) {
    let msg;
    if (body.length === len) msg = JSON.parse(body.toString("utf8"));
    else {
        // Large body: vscode-jsonrpc writes "jsonrpc", "id", then "method"/"result"/"error" first.
        const text = body.toString("utf8", 0, Math.min(body.length, 4096));
        msg = {};
        const id = /"id"\s*:\s*(\d+|"(?:[^"\\]|\\.)*")/.exec(text);
        if (id) msg.id = JSON.parse(id[1]);
        const method = /"method"\s*:\s*"((?:[^"\\]|\\.)*)"/.exec(text);
        if (method) msg.method = JSON.parse(`"${method[1]}"`);
        if (/"error"\s*:/.test(text) && !/"result"\s*:/.test(text)) msg.error = { message: "(large error body)" };
    }
    const other = dir === "c2s" ? "s2c" : "c2s";
    if (msg.method !== undefined) {
        const isReq = msg.id !== undefined;
        if (isReq) idMethods.set(`${dir}:${msg.id}`, msg.method);
        // Server-initiated requests are filesystem callbacks, as in sync mode.
        const rec = { dir, kind: dir === "c2s" ? (isReq ? "request" : "notification") : (isReq ? "call" : "notification"), method: msg.method, bytes: len };
        if (msg.method === "batchRequests") {
            rec.inner = innerMethods(body.toString("utf8"));
            if (body.length < len) rec.innerTruncated = true;
        }
        record(rec);
        return;
    }
    const method = idMethods.get(`${other}:${msg.id}`);
    idMethods.delete(`${other}:${msg.id}`);
    const failed = msg.error !== undefined;
    const kind = dir === "s2c" ? (failed ? "error" : "response") : (failed ? "callError" : "callResponse");
    const rec = { dir, kind, method, bytes: len };
    if (method === "batchRequests" && !failed) rec.innerErrors = innerErrors(body.toString("utf8"));
    if (failed) rec.message = String(msg.error?.message ?? "").slice(0, 300);
    record(rec);
}
function asyncParser(dir) {
    let head = Buffer.alloc(0);
    let need = 0; // body bytes still expected
    let len = 0;
    let parts = [];
    let kept = 0;
    let broken = false;
    return chunk => {
        if (!traceDir || broken) return;
        try {
            let buf = chunk;
            for (;;) {
                if (need > 0) {
                    const n = Math.min(need, buf.length);
                    if (kept < KEEP) {
                        const take = buf.subarray(0, Math.min(n, KEEP - kept));
                        parts.push(take);
                        kept += take.length;
                    }
                    need -= n;
                    buf = buf.subarray(n);
                    if (need > 0) return;
                    classifyAsync(dir, Buffer.concat(parts), len);
                    parts = [];
                    kept = 0;
                }
                if (buf.length === 0) return;
                head = head.length ? Buffer.concat([head, buf]) : buf;
                buf = Buffer.alloc(0);
                const hdrEnd = head.indexOf("\r\n\r\n");
                if (hdrEnd < 0) {
                    if (head.length > 8192) throw new Error("header too long");
                    return;
                }
                const m = /Content-Length:\s*(\d+)/i.exec(head.toString("ascii", 0, hdrEnd));
                if (!m) throw new Error("missing Content-Length");
                len = need = Number(m[1]);
                buf = head.subarray(hdrEnd + 4);
                head = Buffer.alloc(0);
                if (need === 0) classifyAsync(dir, Buffer.alloc(0), 0);
            }
        }
        catch (e) {
            broken = true;
            record({ dir, kind: "unparseable", message: String(e.message) });
        }
    };
}

const parse = isAsync ? asyncParser : syncParser;
const c2s = parse("c2s");
const s2c = parse("s2c");

process.stdin.on("data", chunk => {
    c2s(chunk);
    if (!child.stdin.destroyed) child.stdin.write(chunk);
});
process.stdin.on("end", () => child.stdin.end());
process.stdin.on("error", () => {});
child.stdin.on("error", () => {});
child.stdout.on("data", chunk => {
    s2c(chunk);
    process.stdout.write(chunk);
});
process.stdout.on("error", () => {});

for (const sig of ["SIGTERM", "SIGINT", "SIGHUP"]) {
    process.on(sig, () => {
        record({ kind: "signal", signal: sig });
        child.kill(sig);
    });
}
// "close" fires after the child's stdout has ended, so every forwarded byte is queued before we exit.
child.on("close", (code, signal) => {
    record({ kind: "exit", code, signal });
    flush();
    process.stdout.write("", () => {
        if (signal) {
            process.removeAllListeners(signal);
            process.kill(process.pid, signal);
        }
        process.exit(code ?? 1);
    });
});
child.on("error", e => {
    record({ kind: "spawnError", message: String(e.message) });
    flush();
    process.stderr.write(`node-api proxy: failed to spawn ${real}: ${e.message}\n`);
    process.exit(127);
});
