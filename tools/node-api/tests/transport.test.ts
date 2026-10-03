// Malformed framing against the real server binary (no client): every case must end in bounded time, and
// stdout must only ever carry well-formed protocol frames (no logs or partial garbage). Exit codes and whether
// the server stops before or only at EOF are recorded softly from the oracle.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { describe, test } from "node:test";
import { golden, serverBinary, softGolden } from "./utils.ts";

const g = golden("transport");
const soft = softGolden("transport");

function bin(buf: Buffer) {
    const len = buf.length;
    if (len < 0x100) return Buffer.concat([Buffer.from([0xc4, len]), buf]);
    const h = Buffer.alloc(5);
    h[0] = 0xc6;
    h.writeUInt32BE(len, 1);
    return Buffer.concat([h, buf]);
}
const tuple = (type: number, method: string, payload: string) => Buffer.concat([Buffer.from([0x93, type]), bin(Buffer.from(method)), bin(Buffer.from(payload))]);
const rpc = (obj: unknown) => {
    const body = Buffer.from(JSON.stringify(obj));
    return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
};

/** Parses stdout as frames; returns frame summaries or "invalid" if any byte is not part of a whole frame. */
function frames(out: Buffer, async: boolean): string[] | "invalid" {
    const res: string[] = [];
    let off = 0;
    try {
        while (off < out.length) {
            if (async) {
                const end = out.indexOf("\r\n\r\n", off);
                if (end < 0) return "invalid";
                const m = /^Content-Length: (\d+)$/im.exec(out.toString("ascii", off, end));
                if (!m) return "invalid";
                const body = JSON.parse(out.toString("utf8", end + 4, end + 4 + Number(m[1])));
                if (end + 4 + Number(m[1]) > out.length) return "invalid";
                res.push(body.error ? `error:${body.id ?? "null"}` : `result:${body.id}`);
                off = end + 4 + Number(m[1]);
            }
            else {
                if (out[off] !== 0x93) return "invalid";
                const type = out[off + 1];
                let p = off + 2;
                const read = () => {
                    const mk = out[p];
                    const n = mk === 0xc4 ? out[p + 1] : mk === 0xc5 ? out.readUInt16BE(p + 1) : mk === 0xc6 ? out.readUInt32BE(p + 1) : -1;
                    if (n < 0) throw new Error("bad bin");
                    const hdr = mk === 0xc4 ? 2 : mk === 0xc5 ? 3 : 5;
                    const s = out.toString("utf8", p + hdr, p + hdr + n);
                    p += hdr + n;
                    return s;
                };
                const method = read();
                read();
                if (p > out.length) return "invalid";
                res.push(`${type}:${method}`);
                off = p;
            }
        }
    }
    catch {
        return "invalid";
    }
    return res;
}

interface Probe {
    exitedBeforeEOF: boolean;
    bounded: boolean;
    code: number | null;
    signal: string | null;
    frames: string[] | "invalid";
}

/** Writes `input`, waits `holdMs` with stdin open, then closes stdin and waits up to `afterMs` for exit. */
function probe(async: boolean, input: Buffer, holdMs = 1500, afterMs = 8000): Promise<Probe> {
    const dir = fs.mkdtempSync(`${os.tmpdir()}/node-api-transport-`);
    const args = ["--api", ...(async ? ["--async"] : []), "--cwd", dir, "--useCaseSensitiveFileNames=true"];
    const child = spawn(serverBinary(), args, { stdio: ["pipe", "pipe", "ignore"] });
    const out: Buffer[] = [];
    child.stdout.on("data", d => out.push(d));
    child.stdin.on("error", () => {});
    let exited = false;
    let exitedBeforeEOF = false;
    let code: number | null = null;
    let signal: string | null = null;
    const done = new Promise<void>(resolve =>
        child.on("close", (c, s) => {
            exited = true;
            code = c;
            signal = s;
            resolve();
        })
    );
    child.stdin.write(input);
    return new Promise(resolve => {
        setTimeout(() => {
            exitedBeforeEOF = exited;
            child.stdin.end();
            const timer = setTimeout(() => child.kill("SIGKILL"), afterMs);
            void done.then(() => {
                clearTimeout(timer);
                fs.rmSync(dir, { recursive: true, force: true });
                resolve({ exitedBeforeEOF, bounded: signal !== "SIGKILL", code, signal, frames: frames(Buffer.concat(out), async) });
            });
        }, holdMs);
    });
}

const syncCases: Record<string, Buffer> = {
    eofImmediately: Buffer.alloc(0),
    validEchoThenEOF: tuple(1, "echo", "hi"),
    unknownMethodThenEcho: Buffer.concat([tuple(1, "noSuchMethod", "{}"), tuple(1, "echo", "after")]),
    garbageText: Buffer.from("hello world\n"),
    badTupleArity: Buffer.from([0x92, 0x01, 0xc4, 0x00]),
    unknownMessageType: tuple(9, "echo", "x"),
    nonBinMethod: Buffer.from([0x93, 0x01, 0xa4, 0x65, 0x63, 0x68, 0x6f, 0xc4, 0x00]),
    hugeDeclaredPayload: Buffer.concat([Buffer.from([0x93, 0x01]), bin(Buffer.from("echo")), Buffer.from([0xc6, 0xff, 0xff, 0xff, 0xf0, 0x41])]),
    truncatedFrame: tuple(1, "echo", "truncated").subarray(0, 6),
    invalidJsonParams: tuple(1, "getSourceFileNames", "{not json"),
};

const asyncCases: Record<string, Buffer> = {
    eofImmediately: Buffer.alloc(0),
    validEchoThenEOF: rpc({ jsonrpc: "2.0", id: 1, method: "echo", params: "hi" }),
    unknownMethodThenEcho: Buffer.concat([rpc({ jsonrpc: "2.0", id: 1, method: "noSuchMethod", params: {} }), rpc({ jsonrpc: "2.0", id: 2, method: "echo", params: "after" })]),
    garbageText: Buffer.from("hello world\r\n\r\n"),
    nonNumericLength: Buffer.from("Content-Length: abc\r\n\r\n{}"),
    hugeDeclaredLength: Buffer.from("Content-Length: 4000000000\r\n\r\n{"),
    invalidJsonBody: Buffer.from("Content-Length: 9\r\n\r\n{not json"),
    nonObjectMessage: rpc([1, 2, 3]),
    truncatedBody: Buffer.from("Content-Length: 100\r\n\r\n{\"jsonrpc\":\"2.0\""),
    invalidParamsType: rpc({ jsonrpc: "2.0", id: 3, method: "getSourceFileNames", params: { snapshot: "one" } }),
};

describe("parity: malformed framing", () => {
    for (const [mode, cases] of [["sync", syncCases], ["async", asyncCases]] as const) {
        for (const [name, input] of Object.entries(cases)) {
            test(`${mode}: ${name}`, async t => {
                const r = await probe(mode === "async", input);
                assert.ok(r.bounded, `${mode} ${name}: server did not exit within 8s of EOF`);
                assert.notEqual(r.frames, "invalid", `${mode} ${name}: stdout carried non-protocol bytes`);
                // Async requests are dispatched concurrently, so response order is not part of the contract.
                const framesForGolden = mode === "async" && Array.isArray(r.frames) ? [...r.frames].sort() : r.frames;
                g.check(`${mode}.${name}`, { bounded: r.bounded, frames: framesForGolden });
                soft.check(t, `${mode}.${name}`, { exitedBeforeEOF: r.exitedBeforeEOF, code: r.code, signal: r.signal });
            });
        }
    }
});
