// Runs in a temp consumer project that installed the packed tsrs tarballs (see ../smoke-consumer.mjs).
// Drives the sync and the async JS API (the same steps; `await` is a no-op on sync results) against the packaged
// server: config parsing, diagnostics, emit to disk and to strings, AST traversal, checker queries, a snapshot
// update after an on-disk edit, an in-memory request filesystem, and host filesystem callbacks. Each step prints
// `ok` or `FAIL <reason>`; the process exits 1 if any step failed, after running all of them.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = require("tsrs");
assert.match(root.version, /^\d+\.\d+\.\d+.*-ts/);
assert.match(root.typescriptCommit, /^[0-9a-f]{40}$/);

const sync = await import("tsrs/unstable/sync");
const asyncApi = await import("tsrs/unstable/async");
const { SyntaxKind } = await import("tsrs/unstable/ast");
const { createFileSystemWithLib, serverFS } = await import("tsrs/unstable/fs");

const base = path.resolve(process.argv[2] ?? "project");
const only = process.argv[3]; // optional: "sync" or "async"
const slash = p => p.split(path.sep).join("/");
const MAIN_WITH_ERROR = 'import { twice } from "./lib.js";\nexport const answer = twice(21);\nexport const wrong: string = twice(1);\n';
const MAIN_FIXED = 'import { twice } from "./lib.js";\nexport const answer = twice(21);\nexport const right: number = twice(1);\n';

function writeProject(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(path.join(dir, "src"), { recursive: true });
    fs.writeFileSync(path.join(dir, "tsconfig.json"), JSON.stringify({
        compilerOptions: { strict: true, target: "es2022", module: "nodenext", rootDir: "src", outDir: "out", declaration: true, types: [] },
        include: ["src"],
    }));
    fs.writeFileSync(path.join(dir, "src", "lib.ts"), "export function twice(n: number): number {\n    return n * 2;\n}\n");
    fs.writeFileSync(path.join(dir, "src", "main.ts"), MAIN_WITH_ERROR);
    return { config: slash(path.join(dir, "tsconfig.json")), main: slash(path.join(dir, "src", "main.ts")) };
}

const results = [];
// An unhandled rejection would terminate Node (the unpatched vscode-jsonrpc did this on a failed write); record it
// as a failure instead so the remaining steps still run.
const unhandled = [];
process.on("unhandledRejection", reason => unhandled.push(String(reason?.message ?? reason)));
async function step(label, fn) {
    try {
        await fn();
        results.push([label, "ok"]);
        console.log(`ok   ${label}`);
    }
    catch (e) {
        results.push([label, "FAIL"]);
        console.log(`FAIL ${label}: ${String(e?.message ?? e).split("\n").slice(0, 6).join(" | ")}`);
    }
}

const codes = diagnostics => diagnostics.map(d => d.code);

// Settles within `ms` or reports a hang (the failure mode of the unpatched async client).
function settle(promise, ms = 10_000) {
    let timer;
    const hung = new Promise(resolve => timer = setTimeout(resolve, ms, { state: "hung" }));
    const settled = promise.then(value => ({ state: "resolved", value }), error => ({ state: "rejected", error }));
    return Promise.race([settled, hung]).finally(() => clearTimeout(timer));
}
const tick = () => new Promise(resolve => setImmediate(resolve));
// The server child process, reached through the client's private fields (no public accessor upstream).
const serverProcess = api => api.client?.process ?? api.client?.channel?.child;
async function waitForExit(child, ms = 10_000) {
    const start = Date.now();
    while (child.exitCode === null && child.signalCode === null) {
        if (Date.now() - start > ms) throw new Error(`server pid ${child.pid} still running after ${ms} ms`);
        await new Promise(resolve => setTimeout(resolve, 20));
    }
    return child.signalCode ?? child.exitCode;
}

async function runVariant(variant, API) {
    const dir = path.join(base, variant);
    const { config, main } = writeProject(dir);
    const api = new API({ cwd: dir });
    try {
        await step(`${variant}: parseConfigFile`, async () => {
            const parsed = await api.parseConfigFile(config);
            assert.deepEqual(parsed.fileNames.map(f => path.basename(f)).sort(), ["lib.ts", "main.ts"]);
            assert.equal(parsed.options.strict, true);
        });

        let snapshot, project, sourceFile;
        await step(`${variant}: createSnapshot(openProject) + getSourceFile`, async () => {
            snapshot = await api.createSnapshot({ openProject: config });
            project = snapshot.getConfiguredProject(config);
            assert.ok(project, "configured project");
            sourceFile = await project.program.getSourceFile(main);
            assert.ok(sourceFile, "source file");
            assert.equal(sourceFile.text, MAIN_WITH_ERROR);
        });
        if (!project) return;

        await step(`${variant}: AST statements and forEachChild`, async () => {
            assert.deepEqual(sourceFile.statements.map(s => s.kind), [SyntaxKind.ImportDeclaration, SyntaxKind.VariableStatement, SyntaxKind.VariableStatement]);
            const identifiers = [];
            const visit = node => {
                if (node.kind === SyntaxKind.Identifier) identifiers.push(node.text);
                node.forEachChild(visit);
            };
            sourceFile.forEachChild(visit);
            assert.deepEqual(identifiers, ["twice", "answer", "twice", "wrong", "twice"]);
            const wrongName = sourceFile.statements[2].declarationList.declarations[0].name;
            assert.equal(wrongName.getStart(sourceFile), MAIN_WITH_ERROR.indexOf("wrong"));
        });

        await step(`${variant}: syntactic + semantic diagnostics`, async () => {
            assert.deepEqual(codes(await project.program.getSyntacticDiagnostics(main)), []);
            const semantic = await project.program.getSemanticDiagnostics(main);
            assert.deepEqual(codes(semantic), [2322]);
            assert.equal(semantic[0].pos, MAIN_WITH_ERROR.indexOf("wrong"));
        });

        await step(`${variant}: checker getTypeAtLocation / getSymbolAtLocation / typeToString`, async () => {
            const answer = sourceFile.statements[1].declarationList.declarations[0].name;
            const type = await project.checker.getTypeAtLocation(answer);
            assert.equal(await project.checker.typeToString(type), "number");
            const symbol = await project.checker.getSymbolAtLocation(answer);
            assert.equal(symbol?.name, "answer");
            const call = sourceFile.statements[1].declarationList.declarations[0].initializer;
            const callee = await project.checker.getSymbolAtLocation(call.expression);
            assert.equal(callee?.name, "twice");
        });

        await step(`${variant}: emit to disk follows compiler options`, async () => {
            const result = await project.program.emit();
            assert.equal(result.emitSkipped, false);
            for (const out of ["out/main.js", "out/main.d.ts", "out/lib.js", "out/lib.d.ts"]) {
                assert.ok(fs.existsSync(path.join(dir, out)), `${out} written`);
            }
            assert.match(fs.readFileSync(path.join(dir, "out/main.js"), "utf8"), /twice\)?\(21\)/);
            assert.match(fs.readFileSync(path.join(dir, "out/lib.d.ts"), "utf8"), /export declare function twice\(n: number\): number;/);
        });

        await step(`${variant}: emitToString`, async () => {
            const result = await project.program.emitToString();
            assert.equal(result.emitSkipped, false);
            const names = [...result.outputFiles.keys()].map(f => path.basename(f)).sort();
            assert.deepEqual(names, ["lib.d.ts", "lib.js", "main.d.ts", "main.js"]);
        });

        await step(`${variant}: snapshot update after an on-disk edit`, async () => {
            fs.writeFileSync(path.join(dir, "src", "main.ts"), MAIN_FIXED);
            // ensurePrograms: without it the update only marks the program dirty (upstream semantics).
            const updated = await snapshot.update({ fileNotifications: { changed: [main] }, ensurePrograms: [project.id] });
            const updatedProject = updated.getConfiguredProject(config);
            const updatedFile = await updatedProject.program.getSourceFile(main);
            assert.equal(updatedFile.text, MAIN_FIXED);
            assert.deepEqual(codes(await updatedProject.program.getSemanticDiagnostics(main)), []);
            await updated.dispose();
        });
        await snapshot.dispose();

        await step(`${variant}: in-memory request filesystem`, async () => {
            const memory = await api.createSnapshot({
                openProject: "/mem/tsconfig.json",
                fileSystem: createFileSystemWithLib([
                    ["/mem/tsconfig.json", JSON.stringify({ compilerOptions: { strict: true, noEmit: true }, files: ["a.ts"] })],
                    ["/mem/a.ts", "export const n: number = 'x';\n"],
                ]),
            });
            const memProject = memory.getConfiguredProject("/mem/tsconfig.json");
            assert.ok(memProject, "in-memory configured project");
            assert.deepEqual(codes(await memProject.program.getSemanticDiagnostics("/mem/a.ts")), [2322]);
            await memory.dispose();
        });
    }
    finally {
        await api.close();
    }

    await step(`${variant}: host filesystem callbacks overlay`, async () => {
        const overlay = "export const viaCallback: string = 1;\n";
        const reads = [];
        const callbackApi = new API({
            cwd: dir,
            fs: {
                directoryExists: serverFS.useOS,
                fileExists: serverFS.useOS,
                getAccessibleEntries: serverFS.useOS,
                realpath: serverFS.useOS,
                stat: serverFS.useOS,
                writeFile: serverFS.noop,
                removeFile: serverFS.noop,
                readFile: fileName => {
                    reads.push(fileName);
                    return slash(fileName) === main ? overlay : serverFS.useOS;
                },
            },
        });
        try {
            const callbackSnapshot = await callbackApi.createSnapshot({ openProject: config });
            const callbackProject = callbackSnapshot.getConfiguredProject(config);
            const file = await callbackProject.program.getSourceFile(main);
            assert.equal(file.text, overlay);
            assert.deepEqual(codes(await callbackProject.program.getSemanticDiagnostics(main)), [2322]);
            assert.ok(reads.some(f => slash(f) === main), "readFile callback was called for main.ts");
            await callbackSnapshot.dispose();
        }
        finally {
            await callbackApi.close();
        }
    });
}

// Killing the real server: the async client must reject in-flight, queued and later requests with one stable
// error (tsrs patch npm/sdk/patches/async-client-connection-loss.patch); the sync client throws on the next call.
async function crashAndClose(dir, config, main) {
    await step("async: server killed with requests in flight rejects them all", async () => {
        const api = new asyncApi.API({ cwd: dir });
        let child;
        try {
            const warm = await api.createSnapshot({ openProject: config });
            child = serverProcess(api);
            assert.ok(child?.pid, "async server process");
            // Whole-program semantic diagnostics (default lib included) keep the server busy for well over 20 ms;
            // the unpatched client never settles these once the server dies.
            const program = warm.getConfiguredProject(config).program;
            const inFlight = [program.getSemanticDiagnostics(), program.getGlobalDiagnostics()];
            await new Promise(resolve => setTimeout(resolve, 20)); // the batch is on the server
            child.kill("SIGKILL");
            const queued = api.parseConfigFile(config); // issued before the client noticed
            const results = await Promise.all([...inFlight, queued].map(p => settle(p)));
            const summary = results.map(r => r.state === "rejected" ? `rejected(${r.error.message})` : r.state).join(", ");
            assert.ok(results.slice(0, 2).every(r => r.state !== "resolved"), `in-flight work finished before the kill; the test needs more work in flight: ${summary}`);
            assert.ok(results.every(r => r.state === "rejected" && /^API server connection lost: /.test(r.error.message)), summary);
            assert.equal(await waitForExit(child), "SIGKILL");
            const later = await settle(api.parseConfigFile(config));
            // Same error for every request; the detail is the exit signal/code or "server closed the connection",
            // whichever the client observed first.
            assert.match(later.error?.message ?? later.state, /^API server connection lost: (server process exited with signal SIGKILL|server closed the connection|writing to the server failed: .*)$/);
        }
        catch (e) {
            child?.kill("SIGKILL");
            await settle(api.close(), 2000);
            throw e;
        }
        // close() releases the open snapshot first; with the server gone that release rejects with the same error,
        // so close() rejects promptly (it never hangs) and still closes the client.
        const closed = await settle(api.close());
        assert.notEqual(closed.state, "hung", "close() after a crash settles");
        if (closed.state === "rejected") assert.match(closed.error.message, /^API server connection lost: /);
        assert.match((await settle(api.parseConfigFile(config))).error?.message ?? "", /Client is closed/);
    });

    await step("async: request written after the server died, before the client noticed", async () => {
        const api = new asyncApi.API({ cwd: dir });
        try {
            await api.parseConfigFile(config);
            const child = serverProcess(api);
            child.kill("SIGKILL");
            // Block this thread until the process is a zombie (Linux /proc; elsewhere a fixed 300 ms), so the next
            // request is written to a dead pipe before the client has seen the exit or the stream end.
            const sleep = ms => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
            const stat = `/proc/${child.pid}/stat`;
            if (fs.existsSync(stat)) {
                const deadline = Date.now() + 5000;
                while (Date.now() < deadline && !/^\d+ \(.*\) [ZX]/s.test(fs.readFileSync(stat, "utf8"))) sleep(5);
            }
            else sleep(300);
            const results = await Promise.all([api.parseConfigFile(config), api.parseConfigFile(config)].map(p => settle(p)));
            assert.deepEqual(results.map(r => r.state), ["rejected", "rejected"]);
            for (const r of results) assert.match(r.error.message, /^API server connection lost: /);
        }
        finally {
            await settle(api.close());
        }
    });

    await step("async: close() with a request in flight settles it and stops the server", async () => {
        const api = new asyncApi.API({ cwd: dir });
        await api.parseConfigFile(config);
        const child = serverProcess(api);
        const pending = api.createSnapshot({ openProject: config });
        await tick();
        const closed = await settle(api.close());
        assert.equal(closed.state, "resolved");
        const result = await settle(pending);
        assert.notEqual(result.state, "hung", "pending request settled after close()");
        if (result.state === "rejected") assert.doesNotMatch(result.error.message, /connection lost/);
        const later = await settle(api.parseConfigFile(config));
        assert.equal(later.state, "rejected");
        assert.match(later.error.message, /Client is closed/);
        assert.equal(await waitForExit(child), 0, "server exits 0 after close()");
    });

    await step("sync: server killed between calls throws on the next call; close() is clean", async () => {
        const api = new sync.API({ cwd: dir });
        api.parseConfigFile(config);
        const child = serverProcess(api);
        assert.ok(child?.pid, "sync server process");
        child.kill("SIGKILL");
        await waitForExit(child);
        // Unpatched upstream sync client: the next call fails at once, with the raw pipe error (EBADF/EPIPE on write)
        // or "Unexpected EOF ..." on read.
        assert.throws(() => api.parseConfigFile(config), /Unexpected EOF|EPIPE|EBADF/);
        api.close();
        const normal = new sync.API({ cwd: dir });
        normal.parseConfigFile(config);
        const normalChild = serverProcess(normal);
        normal.close();
        assert.throws(() => normal.parseConfigFile(config), /closed/);
        const code = await waitForExit(normalChild);
        assert.ok(code === 0 || code === "SIGTERM", `sync server after close(): ${code}`);
    });
}

if (only !== "async") await runVariant("sync", sync.API);
if (only !== "sync") await runVariant("async", asyncApi.API);
if (!only) {
    const { config, main } = writeProject(path.join(base, "lifecycle"));
    await crashAndClose(path.join(base, "lifecycle"), config, main);
}

await new Promise(resolve => setTimeout(resolve, 100));
if (unhandled.length) {
    results.push(["no unhandled rejections", "FAIL"]);
    console.log(`FAIL no unhandled rejections: ${unhandled.join(" | ")}`);
}
const failed = results.filter(([, r]) => r !== "ok");
console.log(`${failed.length ? "smoke FAILED" : "smoke ok"}: ${results.length - failed.length}/${results.length} steps passed`);
process.exitCode = failed.length ? 1 : 0;
