// Round trips through the pinned Node clients (sync SyncRpcChannel/MessagePack and async
// vscode-jsonrpc) against examples/transport_test_server.rs.
// Run: TS_REF=<ts-ref> SERVER=<transport_test_server> node --conditions=@typescript/source --test <this file>
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, describe, test } from "node:test";

const api = `${process.env.TS_REF}/packages/typescript/src/api`;
const SERVER = process.env.SERVER;
const { Client: SyncClient } = await import(`${api}/sync/client.ts`);
const { Client: AsyncClient } = await import(`${api}/async/client.ts`);
const { serverFS } = await import(`${api}/fs.ts`);

const ASTRAL = "const \u{1F600} = '\u{1D7D8}\u00e9';\n";
const tmp = mkdtempSync(join(tmpdir(), "tsrs-api-transport-"));
after(() => rmSync(tmp, { recursive: true, force: true }));
writeFileSync(join(tmp, "real.ts"), ASTRAL);
mkdirSync(join(tmp, "dir"));
writeFileSync(join(tmp, "dir", "a.ts"), "a");
symlinkSync(join(tmp, "dir", "a.ts"), join(tmp, "dir", "link.ts"));

function allUseOS(overrides = {}) {
    return {
        readFile: serverFS.useOS,
        fileExists: serverFS.useOS,
        directoryExists: serverFS.useOS,
        getAccessibleEntries: serverFS.useOS,
        realpath: serverFS.useOS,
        stat: serverFS.useOS,
        writeFile: serverFS.useOS,
        removeFile: serverFS.useOS,
        ...overrides,
    };
}

function virtualFS(log) {
    const files = new Map([["/v/main.ts", ASTRAL], ["/v/lone.ts", "x\ud800y"]]);
    return allUseOS({
        readFile: p => {
            log?.push(["readFile", p]);
            if (p === "/v/throws.ts") throw new Error("callback exploded \u{1F600}");
            if (p === "/v/big.ts") return "z".repeat(5 * 1024 * 1024);
            if (p.startsWith("/v/")) return files.get(p);
            return serverFS.useOS;
        },
        fileExists: p => p.startsWith("/v/") ? files.has(p) : serverFS.useOS,
        directoryExists: p => p === "/v" ? true : serverFS.useOS,
        getAccessibleEntries: p => p === "/v" ? { files: ["main.ts", "lone.ts"], directories: ["sub"], symlinks: ["lone.ts"] } : serverFS.useOS,
        realpath: p => p.startsWith("/v/") ? serverFS.identity : serverFS.useOS,
        stat: p => p === "/v/main.ts" ? { mode: 0o100644, size: 12, mtime: new Date("2024-02-29T12:34:56.789Z") } : p === "/v/none.ts" ? undefined : serverFS.useOS,
        writeFile: (p, data) => {
            log?.push(["writeFile", p, data]);
            if (p.startsWith("/v/")) return undefined;
            return serverFS.useOS;
        },
        removeFile: p => {
            log?.push(["removeFile", p]);
            return p.startsWith("/v/") ? undefined : serverFS.useOS;
        },
    });
}

function syncClient(options = {}) {
    return new SyncClient({ tsserverPath: SERVER, cwd: tmp, ...options });
}

describe("sync client (MessagePack, pinned SyncRpcChannel)", () => {
    test("ping, echo text and binary with astral characters", () => {
        const c = syncClient();
        try {
            assert.equal(c.apiRequest("ping", null), "pong");
            assert.equal(c.echo(ASTRAL), ASTRAL);
            const bytes = new Uint8Array([0, 1, 2, 0xff, 0xf0, 0x9f, 0x98, 0x80]);
            assert.deepEqual([...c.echoBinary(bytes)], [...bytes]);
            const big = new Uint8Array(70000).fill(7);
            assert.equal(c.echoBinary(big).length, 70000);
        }
        finally {
            c.close();
        }
    });

    test("filesystem callbacks: value, missing, useOS, sentinels, writes", () => {
        const log = [];
        const c = syncClient({ fs: virtualFS(log) });
        try {
            const fs = (op, path, data) => c.apiRequest("test/fs", { op, path, data });
            assert.equal(fs("readFile", "/v/main.ts"), ASTRAL);
            assert.equal(fs("readFile", "/v/missing.ts"), null);
            assert.equal(fs("readFile", join(tmp, "real.ts")), ASTRAL);
            assert.equal(fs("readFile", "/v/lone.ts"), "x\ufffdy");
            assert.equal(fs("readFile", "/v/big.ts").length, 5 * 1024 * 1024);
            assert.equal(fs("fileExists", "/v/main.ts"), true);
            assert.equal(fs("fileExists", "/v/nope.ts"), false);
            assert.equal(fs("fileExists", join(tmp, "real.ts")), true);
            assert.equal(fs("directoryExists", "/v"), true);
            assert.deepEqual(fs("getAccessibleEntries", "/v"), { files: ["main.ts", "lone.ts"], directories: ["sub"], symlinks: ["lone.ts"] });
            const real = fs("getAccessibleEntries", join(tmp, "dir"));
            assert.deepEqual(real.files.sort(), ["a.ts", "link.ts"]);
            assert.deepEqual(real.symlinks, ["link.ts"]);
            assert.equal(fs("realpath", "/v/main.ts"), "/v/main.ts");
            assert.equal(fs("realpath", join(tmp, "dir", "link.ts")), join(tmp, "dir", "a.ts"));
            const st = fs("stat", "/v/main.ts");
            assert.equal(st.name, "main.ts");
            assert.equal(st.size, 12);
            assert.equal(st.isDir, false);
            assert.equal(st.mode & 0o777, 0o644);
            assert.equal(st.mtimeMs, Date.parse("2024-02-29T12:34:56.789Z"));
            assert.equal(fs("stat", "/v/none.ts"), null);
            assert.equal(fs("stat", join(tmp, "dir")).isDir, true);
            assert.equal(fs("writeFile", "/v/out.js", "out \u{1F600}"), null);
            assert.equal(fs("removeFile", "/v/out.js"), null);
            assert.equal(fs("writeFile", join(tmp, "written.txt"), "disk \u{1F600}"), null);
            assert.equal(readFileSync(join(tmp, "written.txt"), "utf8"), "disk \u{1F600}");
            assert.equal(fs("removeFile", join(tmp, "written.txt")), null);
            assert.equal(existsSync(join(tmp, "written.txt")), false);
            assert.deepEqual(log.filter(e => e[0] !== "readFile"), [
                ["writeFile", "/v/out.js", "out \u{1F600}"],
                ["removeFile", "/v/out.js"],
                ["writeFile", join(tmp, "written.txt"), "disk \u{1F600}"],
                ["removeFile", join(tmp, "written.txt")],
            ]);
            assert.equal(fs("useCaseSensitiveFileNames", "/"), true);
        }
        finally {
            c.close();
        }
    });

    test("concurrent server-side callbacks from several threads are serialized", () => {
        const c = syncClient({ fs: virtualFS() });
        try {
            const ops = Array.from({ length: 16 }, (_, i) => ({ op: "readFile", path: i % 2 ? "/v/main.ts" : join(tmp, "real.ts") }));
            assert.deepEqual(c.apiRequest("test/parallelFs", { ops }), ops.map(() => ASTRAL));
        }
        finally {
            c.close();
        }
    });

    test("nested API request from inside a client callback", () => {
        const c = syncClient();
        try {
            c.registerCallback("nested", params => ({ depth: c.apiRequest("test/depth", null), ping: c.apiRequest("ping", null), params }));
            const res = c.apiRequest("test/callClient", { method: "nested", params: { s: "\u{1F600}" } });
            assert.deepEqual(JSON.parse(res.raw), { depth: 1, ping: "pong", params: { s: "\u{1F600}" } });
            assert.equal(c.apiRequest("test/depth", null), 0);
        }
        finally {
            c.close();
        }
    });

    test("handler errors and panics become error responses; the connection survives", () => {
        const c = syncClient();
        try {
            assert.throws(() => c.apiRequest("test/fail", null), { message: "api: client error: requested failure \u{1F600}" });
            assert.throws(() => c.apiRequest("test/panic", null), { message: "panic: requested panic \u{1F600}" });
            assert.throws(() => c.apiRequest("test/badJson", null), /invalid JSON result/);
            assert.equal(c.apiRequest("ping", null), "pong");
        }
        finally {
            c.close();
        }
    });

    test("serverFS.error and a throwing callback fail the triggering request", () => {
        const a = syncClient({ fs: allUseOS({ readFile: serverFS.error }) });
        try {
            assert.throws(() => a.apiRequest("test/fs", { op: "readFile", path: join(tmp, "real.ts") }), {
                message: "panic: filesystem operation configured with serverFS.error: readFile",
            });
            assert.equal(a.apiRequest("ping", null), "pong");
        }
        finally {
            a.close();
        }
        const b = syncClient({ fs: virtualFS() });
        try {
            // The pinned channel replies CallError and then rethrows (a failed callback is unrecoverable).
            assert.throws(() => b.apiRequest("test/fs", { op: "readFile", path: "/v/throws.ts" }), {
                message: "Error calling callback `readFile`: callback exploded \u{1F600}",
            });
        }
        finally {
            b.close();
        }
    });

    test("server timing", () => {
        const c = syncClient({ collectTiming: true });
        try {
            c.apiRequest("ping", null);
            c.apiRequest("test/sleep", 20);
            const info = c.getTimingInfo();
            assert.equal(info.enabled, true);
            const last = info.recentRequests.at(-1);
            assert.equal(last.method, "test/sleep");
            assert.ok(last.serverTimeMs >= 20, JSON.stringify(info));
            assert.ok(info.totals.serverTimeMs >= 20, JSON.stringify(info));
            c.resetTimingInfo();
        }
        finally {
            c.close();
        }
    });

    test("server exit while a request is in flight is reported, not hung", () => {
        const c = syncClient();
        try {
            assert.throws(() => c.apiRequest("test/exit", null), /Unexpected EOF while reading from child process/);
        }
        finally {
            c.close();
        }
    });
});

function asyncClient(options = {}) {
    return new AsyncClient({ tsserverPath: SERVER, cwd: tmp, ...options });
}

describe("async client (JSON-RPC, pinned vscode-jsonrpc client)", () => {
    test("ping, echo, initialize", async () => {
        const c = asyncClient();
        try {
            assert.equal(await c.apiRequest("ping", null), "pong");
            assert.deepEqual(await c.apiRequest("echo", { s: ASTRAL }), { s: ASTRAL });
            const init = await c.apiRequest("initialize", null);
            assert.equal(init.currentDirectory, tmp);
        }
        finally {
            await c.close();
        }
    });

    test("filesystem callbacks and concurrent requests", async () => {
        const log = [];
        const c = asyncClient({ fs: virtualFS(log) });
        try {
            const fs = (op, path, data) => c.apiRequest("test/fs", { op, path, data });
            const [a, b, missing, lone, big] = await Promise.all([
                fs("readFile", "/v/main.ts"),
                fs("readFile", join(tmp, "real.ts")),
                fs("readFile", "/v/missing.ts"),
                fs("readFile", "/v/lone.ts"),
                fs("readFile", "/v/big.ts"),
            ]);
            assert.equal(a, ASTRAL);
            assert.equal(b, ASTRAL);
            assert.equal(missing, null);
            assert.equal(lone, "x\ufffdy");
            assert.equal(big.length, 5 * 1024 * 1024);
            assert.equal((await fs("stat", "/v/main.ts")).mtimeMs, Date.parse("2024-02-29T12:34:56.789Z"));
            assert.equal(await fs("writeFile", "/v/out.js", "o"), null);
            const ops = Array.from({ length: 16 }, () => ({ op: "readFile", path: "/v/main.ts" }));
            assert.deepEqual(await c.apiRequest("test/parallelFs", { ops }), ops.map(() => ASTRAL));
        }
        finally {
            await c.close();
        }
    });

    test("async callback that re-enters the API", async () => {
        const c = asyncClient();
        try {
            await c.connect();
            c.registerCallback("nested", async params => ({ ping: await c.apiRequest("ping", null), params }));
            const res = await c.apiRequest("test/callClient", { method: "nested", params: { n: 1 } });
            assert.deepEqual(JSON.parse(res.raw), { ping: "pong", params: { n: 1 } });
        }
        finally {
            await c.close();
        }
    });

    test("errors, panics and callback failures; connection survives", async () => {
        const c = asyncClient({ fs: virtualFS() });
        try {
            await assert.rejects(c.apiRequest("test/fail", null), { message: "api: client error: requested failure \u{1F600}" });
            await assert.rejects(c.apiRequest("test/panic", null), { message: "panic: requested panic \u{1F600}" });
            await assert.rejects(c.apiRequest("test/fs", { op: "readFile", path: "/v/throws.ts" }), /panic: ipc: remote error \[-32603\]: .*callback exploded/);
            await assert.rejects(c.apiRequest("test/callClient", { method: "notRegistered" }), /ipc: remote error \[-32601\]/);
            assert.equal(await c.apiRequest("ping", null), "pong");
        }
        finally {
            await c.close();
        }
    });

    test("server exit mid-request: process ends with its exit code", async () => {
        // The pinned async client does not reject pending requests when the server dies (vscode-jsonrpc
        // only rejects them on dispose), so observe the process instead of the promise.
        const c = asyncClient();
        try {
            await c.connect();
            const exited = new Promise(resolve => c.process.once("exit", code => resolve(code)));
            c.apiRequest("test/exit", null).catch(() => {});
            assert.equal(await exited, 3);
        }
        finally {
            await c.close();
        }
    });

    test("unix socket transport (--pipe)", { skip: process.platform === "win32" }, async () => {
        const pipe = join(tmp, "api.sock");
        const child = spawn(SERVER, ["--api", "--async", "--cwd", tmp, "--pipe", pipe], { stdio: ["ignore", "pipe", "inherit"] });
        let stdout = "";
        child.stdout.on("data", d => stdout += d);
        const exited = new Promise(resolve => child.once("exit", code => resolve(code)));
        for (let i = 0; i < 200 && !existsSync(pipe); i++) await new Promise(r => setTimeout(r, 10));
        const c = new AsyncClient({ pipe });
        try {
            assert.equal(await c.apiRequest("ping", null), "pong");
            assert.equal(await c.apiRequest("test/depth", null), 0);
        }
        finally {
            await c.close();
        }
        assert.equal(await exited, 0);
        assert.equal(stdout, "", "socket mode must not write to stdout");
        assert.equal(existsSync(pipe), false, "socket file is removed on exit");
    });
});
