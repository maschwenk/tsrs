// npm/tsrs-wasm against native tsrs. Each test names the regression it catches.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { Worker } from "node:worker_threads";
import { loadModule, tsc } from "../node.js";
import { bin, pkg, project, rm, runBin, runNative, tree } from "./helpers.mjs";

const SOURCES = {
    "tsconfig.json": JSON.stringify({ compilerOptions: { rootDir: "src", outDir: "out", declaration: true, sourceMap: true, strict: true }, include: ["src"] }),
    "src/a.ts": "export function f(x: string): number { return x.length; }\nexport const bad: string = f('é😀');\n",
    "src/b.ts": "import { f } from './a';\nexport default f(1);\n",
};

// Regression: text drift between the module and native tsrs (diagnostic text, paths, emitted file bytes) when the
// module runs over in-memory files.
test("in-memory check and emit equal native", async () => {
    const dir = project(SOURCES);
    try {
        const files = Object.fromEntries(Object.entries(SOURCES).map(([k, v]) => [path.join(dir, k), v]));
        const r = await tsc(["-p", ".", "--pretty", "false"], { cwd: dir, files });
        const n = runNative(["-p", ".", "--pretty", "false"], dir);
        assert.equal(r.stdout, n.stdout);
        assert.equal(r.exitCode, n.exitCode);
        const emitted = Object.fromEntries(Object.entries(tree(path.join(dir, "out"))).map(([k, v]) => [path.join(dir, "out", k), v.toString("utf8")]));
        assert.deepEqual(r.files, emitted);
        assert.ok(Object.keys(emitted).length >= 6);
    } finally {
        rm(dir);
    }
});

// Regression: JSON diagnostics lose fields or file-less diagnostics (a bad command-line option has no file and no
// positions) break the encoder.
test("JSON diagnostics, including one without a file", async () => {
    const r = await tsc(["/p/a.ts", "--noEmit", "--strictt"], { files: { "/p/a.ts": "const s: string = 1;\n" }, cwd: "/p", diagnostics: "json" });
    assert.equal(r.stdout, "");
    const option = r.diagnostics.find((d) => d.code === 5025 || d.code === 5023);
    assert.ok(option, JSON.stringify(r.diagnostics));
    assert.equal(option.fileName, undefined);
    const r2 = await tsc(["/p/a.ts", "--noEmit"], { files: { "/p/a.ts": "const s: string = 1;\n" }, cwd: "/p", diagnostics: "json" });
    assert.deepEqual(r2.diagnostics.map((d) => [d.fileName, d.code, d.pos, d.end, d.startPosition]), [["/p/a.ts", 2322, 6, 7, { line: 0, character: 6 }]]);
    assert.equal(r2.exitCode, 2);
});

// Regression: the Node file-system host fails to read or write the real file system (stat, readDir, write with
// missing parent directories).
test("real file system read and write", async () => {
    const dir = project(SOURCES);
    try {
        const r = await tsc(["-p", ".", "--pretty", "false"], { cwd: dir });
        assert.equal(r.exitCode, 2);
        assert.match(r.stdout, /src\/a\.ts\(2,14\): error TS2322/);
        assert.ok(fs.existsSync(path.join(dir, "out/a.js")) && fs.existsSync(path.join(dir, "out/b.d.ts")));
        assert.deepEqual(tree(path.join(dir, "out")), (runNative(["-p", ".", "--pretty", "false"], dir), tree(path.join(dir, "out"))));
    } finally {
        rm(dir);
    }
});

// Regression: output lost on a non-blocking pipe (EAGAIN) when the reader is slow, so a CI log misses diagnostics.
test("a slow pipe reader gets every byte", async () => {
    const lines = Array.from({ length: 4000 }, (_, i) => `export const v${i}: string = ${i};`).join("\n");
    const dir = project({ "a.ts": lines });
    try {
        const child = spawn(process.execPath, [bin, "a.ts", "--noEmit", "--pretty", "false"], { cwd: dir });
        child.stdout.pause();
        await new Promise((r) => setTimeout(r, 500));
        const chunks = [];
        child.stdout.on("data", (c) => chunks.push(c));
        child.stdout.resume();
        const code = await new Promise((r) => child.on("close", r));
        const out = Buffer.concat(chunks).toString();
        assert.ok(out.length > 200_000, `${out.length} bytes`);
        assert.equal(out, runNative(["a.ts", "--noEmit", "--pretty", "false"], dir).stdout);
        assert.equal(code, 2);
    } finally {
        rm(dir);
    }
});

// Regression: the byte-order mark is dropped (or doubled) when emitted files come back from the module as text.
test("--emitBOM is kept in memory mode", async () => {
    const r = await tsc(["/p/a.ts", "--emitBOM", "--outDir", "/p/out"], { files: { "/p/a.ts": "export const a = 1;\n" }, cwd: "/p" });
    assert.equal(r.exitCode, 0);
    assert.ok(r.files["/p/out/a.js"].startsWith("﻿export const a = 1;"), JSON.stringify(r.files));
});

// Regression: write errors (permission denied, a path through a file) print a different text than native tsrs.
test("write error texts equal native", async () => {
    const dir = project({ "a.ts": "export const a = 1;\n", "locked/.keep": "" });
    try {
        fs.chmodSync(path.join(dir, "locked"), 0o555);
        // root ignores the read-only bit, so only the path-through-a-file case fails there.
        const cases = process.getuid?.() === 0 ? [] : [["a.ts", "--outDir", "locked/out"]];
        for (const args of [...cases, ["a.ts", "--outDir", "a.ts/sub"]]) {
            const r = await tsc([...args, "--pretty", "false"], { cwd: dir });
            const n = runNative([...args, "--pretty", "false"], dir);
            assert.equal(r.stdout, n.stdout);
            assert.match(r.stdout, /TS5033/);
            assert.equal(r.exitCode, n.exitCode);
        }
    } finally {
        rm(dir);
    }
});

// Regression: watch mode starts (and never exits) instead of being refused, in any spelling, or `--watch false`
// is refused.
test("--watch is refused in every spelling; --watch false runs", async () => {
    const dir = project({ "a.ts": "export const a = 1;\n", "args.txt": "a.ts\n--watch\n" });
    try {
        for (const args of [["a.ts", "--watch"], ["a.ts", "-w"], ["a.ts", "--WATCH"], ["@args.txt"]]) {
            const r = await tsc([...args, "--noEmit"], { cwd: dir });
            const n = runNative([...args, "--noEmit"], dir);
            assert.equal(r.stdout, n.stdout, args.join(" "));
            assert.equal(r.exitCode, n.exitCode);
            assert.match(r.stdout, /not supported/);
        }
        const ok = await tsc(["a.ts", "--watch", "false", "--noEmit"], { cwd: dir });
        assert.equal(ok.exitCode, 0);
        assert.equal(ok.stdout, "");
    } finally {
        rm(dir);
    }
});

// Regression: `-b` with JSON diagnostics runs and its build-mode text is lost.
test("JSON diagnostics with -b are refused", async () => {
    const r = await tsc(["-b"], { files: { "/p/tsconfig.json": "{}", "/p/a.ts": "export const a = 1;\n" }, cwd: "/p", diagnostics: "json" });
    assert.equal(r.exitCode, 1);
    assert.equal(r.stdout, "error: diagnostics as JSON are not supported with --build\n");
});

// Regression: a symlinked current directory prints other paths than native tsrs (native uses the real cwd).
test("a symlinked cwd prints the same paths as native", () => {
    const dir = project({ "real/tsconfig.json": JSON.stringify({ compilerOptions: { noEmit: true }, files: ["a.ts"] }), "real/a.ts": "const s: string = 1;\n" });
    try {
        fs.symlinkSync(path.join(dir, "real"), path.join(dir, "link"));
        const cwd = path.join(dir, "link");
        const r = runBin(["-p", ".", "--pretty", "false", "--listFiles"], cwd);
        const n = runNative(["-p", ".", "--pretty", "false", "--listFiles"], cwd);
        assert.equal(r.stdout, n.stdout);
        assert.equal(r.exitCode, 2);
    } finally {
        rm(dir);
    }
});

// Regression: the package's own declarations stop type-checking (for TypeScript users of the package), with or
// without the DOM library.
test("index.d.ts type-checks, with and without lib dom", async () => {
    const files = {};
    for (const f of ["index.d.ts", "core.d.ts"]) files[`/pkg/${f}`] = fs.readFileSync(path.join(pkg, f));
    files["/pkg/use.ts"] = 'import { tsc, type TscResult } from "./index.js";\nexport async function check(): Promise<number> { const r: TscResult = await tsc(["-p", "."], { files: { "/a.ts": "" }, diagnostics: "json" }); return r.diagnostics?.length ?? r.exitCode; }\n';
    for (const lib of ["es2022", "es2022,dom"]) {
        const r = await tsc(["/pkg/use.ts", "--noEmit", "--strict", "--module", "nodenext", "--target", "es2022", "--lib", lib, "--types", ""], { files, cwd: "/pkg" });
        assert.equal(r.stdout, "", lib);
        assert.equal(r.exitCode, 0);
    }
});

// Regression: browser.js (the browser entry) breaks; a Node worker with in-memory files stands in for a browser.
test("browser.js runs in a worker with memoryFileSystem", async () => {
    const module = loadModule();
    const worker = new Worker(
        `import { tsc } from ${JSON.stringify(new URL("../browser.js", import.meta.url).href)};
         import { parentPort, workerData } from "node:worker_threads";
         const r = await tsc(["/p/a.ts", "--noEmit", "--pretty", "false"], { module: workerData.module, files: { "/p/a.ts": "const s: string = 1;\\n" }, cwd: "/p" });
         parentPort.postMessage(r);`,
        { eval: true, workerData: { module }, resourceLimits: { stackSizeMb: 256 } },
    );
    const r = await new Promise((resolve, reject) => {
        worker.once("message", resolve);
        worker.once("error", reject);
    });
    assert.equal(r.stdout, "a.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.\n");
    assert.equal(r.exitCode, 2);
});
