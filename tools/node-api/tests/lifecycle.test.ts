// Handle validation and process lifecycle: released/foreign/invalid handles sent with the pinned client's own
// request path, server survival after rejected requests, re-entrant API calls from inside filesystem callbacks,
// async close with pending requests, and server crashes under both bindings. Anything that could hang runs in
// a separate process with a hard timeout; a timeout is recorded as a timeout, never as a pass.

import { API } from "@typescript/typescript/unstable/sync";
import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { cwd, golden, linuxOnly, runIsolated, softGolden } from "./utils.ts";
import { createVirtualFileSystem } from "../testUtils.ts";

const g = golden("lifecycle");
const soft = softGolden("lifecycle");
const files = {
    "/tsconfig.json": `{ "compilerOptions": { "strict": true } }`,
    "/src/index.ts": `export const value = 1;\nexport function f(n: number) { return n; }\n`,
};

function attempt(fn: () => unknown): { threw: boolean; message?: string; } {
    try {
        fn();
        return { threw: false };
    }
    catch (e) {
        return { threw: true, message: String((e as Error).message).split("\n")[0].slice(0, 200) };
    }
}

describe("parity: handles", () => {
    test("released, invalid and foreign handles are rejected and the server keeps serving", t => {
        using api = new API({ cwd, fs: createVirtualFileSystem(files) });
        using other = new API({ cwd, fs: createVirtualFileSystem(files) });
        const raw = (a: API) => (a as any).client as { apiRequest(method: string, params?: unknown): any; };
        const snap = api.createSnapshot({ openProject: "/tsconfig.json" });
        const project = snap.getConfiguredProject("/tsconfig.json")!;
        const symbol = project.checker.getSymbolAtPosition("/src/index.ts", files["/src/index.ts"].indexOf("value"))!;
        const symbolRef = { ...(symbol as any).reference ?? {}, id: (symbol as any).id };
        const live = { snapshot: snap.id, project: project.id };

        const cases: Record<string, () => unknown> = {
            "valid.names": () => raw(api).apiRequest("getSourceFileNames", live),
            "invalid.snapshot": () => raw(api).apiRequest("getSourceFileNames", { ...live, snapshot: 987654 }),
            "invalid.project": () => raw(api).apiRequest("getSourceFileNames", { ...live, project: "/nope/tsconfig.json" }),
            "invalid.position": () => raw(api).apiRequest("getSymbolAtPosition", { ...live, file: "/src/index.ts", position: 1_000_000 }),
            "invalid.file": () => raw(api).apiRequest("getSymbolAtPosition", { ...live, file: "/src/missing.ts", position: 0 }),
            "invalid.location": () => raw(api).apiRequest("getSymbolAtLocation", { ...live, location: "not-a-node-handle" }),
            "invalid.symbol": () => raw(api).apiRequest("getTypeOfSymbol", { ...live, symbol: { ...symbolRef, id: 2 ** 30 } }),
            "invalid.params": () => raw(api).apiRequest("getSourceFileNames", { snapshot: "one" } as any),
            "unknown.method": () => raw(api).apiRequest("noSuchMethod" as any, {}),
            "release.invalid": () => raw(api).apiRequest("release", { snapshot: 424242 }),
        };
        // Same numeric snapshot id in a second session: ids are session scoped, so this must not reach api's state.
        const otherSnap = other.createSnapshot({ openProject: "/tsconfig.json" });
        cases["foreign.sameIdOtherSession"] = () => raw(other).apiRequest("getSourceFileNames", { snapshot: snap.id, project: project.id });
        const outcome: Record<string, boolean> = {};
        for (const [name, fn] of Object.entries(cases)) {
            const r = attempt(fn);
            outcome[name] = r.threw;
            soft.check(t, `handles.${name}`, r.message ?? null);
        }
        // Release, then use the released id directly (bypassing the client's own disposed check).
        snap.dispose();
        const afterRelease = attempt(() => raw(api).apiRequest("getSourceFileNames", live));
        outcome["released.snapshot"] = afterRelease.threw;
        soft.check(t, "handles.released.snapshot", afterRelease.message ?? null);
        outcome["released.clientObject"] = attempt(() => project.program.getSourceFileNames()).threw;
        outcome["released.twice"] = attempt(() => raw(api).apiRequest("release", { snapshot: live.snapshot })).threw;
        // The server must still work after every rejection.
        const fresh = api.createSnapshot({ openProject: "/tsconfig.json" });
        outcome["survives"] = fresh.getConfiguredProject("/tsconfig.json")!.program.getSourceFileNames().includes("/src/index.ts");
        outcome["otherSurvives"] = otherSnap.getConfiguredProject("/tsconfig.json")!.program.getSourceFileNames().includes("/src/index.ts");
        g.check("handles.threw", outcome);
    });
});

describe("parity: callbacks and process lifecycle", () => {
    test("re-entrant API request from inside a filesystem callback (sync)", t => {
        const r = runIsolated(`
            const vfs = createVirtualFileSystem(${JSON.stringify(files)});
            let api;
            let nested = "not-called";
            const readFile = vfs.readFile.bind(vfs);
            vfs.readFile = (p) => {
                if (p === "/src/index.ts" && nested === "not-called") {
                    try { nested = api.parseCommandLine(["--strict"]).options.strict === true ? "ok" : "wrong"; }
                    catch (e) { nested = "threw: " + String(e.message).split("\\n")[0].slice(0, 160); }
                }
                return readFile(p);
            };
            api = new API({ cwd, fs: vfs });
            const names = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json").program.getSourceFileNames();
            const after = api.parseCommandLine(["--noEmit"]).options.noEmit;
            api.close();
            return { nested: nested.startsWith("threw") ? "threw" : nested, nestedMessage: nested, sawIndex: names.includes("/src/index.ts"), after };
        `);
        assert.equal(r.timedOut, false, `re-entrant sync request hung (> 20s); stderr: ${r.stderr}`);
        // sawIndex is not compared: see the todo test below.
        g.check("reentry.sync", { status: r.status, nested: r.result?.nested, after: r.result?.after });
        soft.check(t, "reentry.sync.message", r.result?.nestedMessage ?? null);
    });

    // Pinned Go defect (b85298b6): a nested sync request issued from inside a readFile callback intermittently
    // makes the file being read disappear from the program (observed 3/40 runs; 0/40 without the nested request,
    // 0/40 with the async binding). Desired behavior asserted; todo so neither server is credited by chance.
    test("re-entrant sync request inside readFile never drops the file being read", { todo: "pinned Go server intermittently drops the file when a nested sync request runs inside its readFile callback" }, () => {
        const r = runIsolated(`
            let dropped = 0;
            for (let i = 0; i < 30; i++) {
                const vfs = createVirtualFileSystem(${JSON.stringify(files)});
                let api;
                let nested = false;
                const readFile = vfs.readFile.bind(vfs);
                vfs.readFile = (p) => {
                    if (p === "/src/index.ts" && !nested) { nested = true; api.parseCommandLine(["--strict"]); }
                    return readFile(p);
                };
                api = new API({ cwd, fs: vfs });
                const names = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json").program.getSourceFileNames();
                if (!names.includes("/src/index.ts")) dropped++;
                api.close();
            }
            return { dropped };
        `, 60_000);
        assert.equal(r.timedOut, false, `hung (> 60s); stderr: ${r.stderr}`);
        assert.equal(r.result?.dropped, 0, `file dropped in ${r.result?.dropped}/30 runs`);
    });

    test("re-entrant API request from inside a filesystem callback (async)", t => {
        const r = runIsolated(`
            const vfs = createVirtualFileSystem(${JSON.stringify(files)});
            let api;
            let nested = "not-called";
            const readFile = vfs.readFile.bind(vfs);
            vfs.readFile = (p) => {
                if (p === "/src/index.ts" && nested === "not-called") {
                    nested = "pending";
                    api.parseCommandLine(["--strict"]).then(r => { nested = r.options.strict === true ? "ok" : "wrong"; }, e => { nested = "threw: " + String(e.message).slice(0, 160); });
                }
                return readFile(p);
            };
            api = new AsyncAPI({ cwd, fs: vfs });
            const snapshot = await api.createSnapshot({ openProject: "/tsconfig.json" });
            const names = await snapshot.getConfiguredProject("/tsconfig.json").program.getSourceFileNames();
            for (let i = 0; i < 100 && nested === "pending"; i++) await new Promise(r => setTimeout(r, 20));
            await api.close();
            return { nested: nested.startsWith("threw") ? "threw" : nested, nestedMessage: nested, sawIndex: names.includes("/src/index.ts") };
        `);
        assert.equal(r.timedOut, false, `re-entrant async request hung (> 20s); stderr: ${r.stderr}`);
        g.check("reentry.async", { status: r.status, nested: r.result?.nested, sawIndex: r.result?.sawIndex });
        soft.check(t, "reentry.async.message", r.result?.nestedMessage ?? null);
    });

    test("async close with requests in flight settles every promise", t => {
        const r = runIsolated(`
            const api = new AsyncAPI({ cwd, fs: createVirtualFileSystem(${JSON.stringify(files)}) });
            const snapshot = await api.createSnapshot({ openProject: "/tsconfig.json" });
            const program = snapshot.getConfiguredProject("/tsconfig.json").program;
            const pending = [];
            for (let i = 0; i < 50; i++) pending.push(program.getSemanticDiagnostics("/src/index.ts"));
            const closing = api.close();
            const settled = await Promise.allSettled(pending);
            await closing;
            const afterClose = await program.getSourceFileNames().then(() => "resolved", e => "rejected");
            return { fulfilled: settled.filter(s => s.status === "fulfilled").length, rejected: settled.filter(s => s.status === "rejected").length, afterClose, sample: settled.find(s => s.status === "rejected")?.reason?.message?.slice(0, 160) ?? null };
        `);
        assert.equal(r.timedOut, false, `async close with pending requests hung (> 20s); stderr: ${r.stderr}`);
        assert.equal(r.status, 0, r.stderr);
        assert.equal(r.result.fulfilled + r.result.rejected, 50);
        g.check("close.async", { settled: r.result.fulfilled + r.result.rejected, afterClose: r.result.afterClose });
        soft.check(t, "close.async.split", { fulfilled: r.result.fulfilled, rejected: r.result.rejected, sample: r.result.sample });
    });

    test("server crash under the sync binding fails the next request promptly", { skip: linuxOnly }, t => {
        const r = runIsolated(`
            const { apiServerPid } = await import("./utils.ts");
            const api = new API({ cwd, fs: createVirtualFileSystem(${JSON.stringify(files)}) });
            const snapshot = api.createSnapshot({ openProject: "/tsconfig.json" });
            const pid = apiServerPid(process.pid);
            if (!pid) return { error: "no server pid" };
            process.kill(pid, "SIGKILL");
            await new Promise(r => setTimeout(r, 200));
            let next;
            try { snapshot.getConfiguredProject("/tsconfig.json").program.getSourceFileNames(); next = "resolved"; }
            catch (e) { next = "threw"; var message = String(e.message).slice(0, 160); }
            let closeOk = true;
            try { api.close(); } catch { closeOk = false; }
            return { next, message, closeOk };
        `);
        assert.equal(r.timedOut, false, `sync request after server crash hung (> 20s); stderr: ${r.stderr}`);
        g.check("crash.sync", { status: r.status, next: r.result?.next, closeOk: r.result?.closeOk, error: r.result?.error ?? null });
        soft.check(t, "crash.sync.message", r.result?.message ?? null);
    });

    // Pinned upstream client defect (Go oracle, b85298b6): after the server dies, the async client never settles the
    // requests in flight, so the script's top-level await is abandoned and Node exits 13. The desired behavior is
    // asserted and the test is marked todo so it is reported, not counted as a pass, for either server.
    test("server crash under the async binding rejects pending and later requests", { skip: linuxOnly, todo: "upstream async client leaves in-flight requests unsettled after a server crash (node exit 13)" }, t => {
        const r = runIsolated(`
            const { apiServerPid } = await import("./utils.ts");
            const api = new AsyncAPI({ cwd, fs: createVirtualFileSystem(${JSON.stringify(files)}) });
            const snapshot = await api.createSnapshot({ openProject: "/tsconfig.json" });
            const program = snapshot.getConfiguredProject("/tsconfig.json").program;
            const pid = apiServerPid(process.pid);
            if (!pid) return { error: "no server pid" };
            const pending = [];
            for (let i = 0; i < 20; i++) pending.push(program.getSemanticDiagnostics("/src/index.ts"));
            process.kill(pid, "SIGKILL");
            const settled = await Promise.allSettled(pending);
            const later = await program.getSourceFileNames().then(() => "resolved", e => "rejected");
            const closed = await api.close().then(() => "resolved", e => "rejected");
            return { settled: settled.length, rejected: settled.filter(s => s.status === "rejected").length, later, closed };
        `);
        assert.equal(r.timedOut, false, `async requests after server crash hung (> 20s); stderr: ${r.stderr}`);
        g.check("crash.async.observed", { status: r.status });
        assert.equal(r.status, 0, "in-flight async requests did not settle after the server crashed");
        g.check("crash.async", { status: r.status, settled: r.result?.settled, later: r.result?.later, closed: r.result?.closed, error: r.result?.error ?? null });
        soft.check(t, "crash.async.rejected", r.result?.rejected ?? null);
    });
});
