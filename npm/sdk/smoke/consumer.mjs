// Runs in a temp consumer project that installed the packed @maschwenk/tsrs tarballs (see ../smoke-consumer.mjs).
// Drives the sync and the async JS API (the same steps; `await` is a no-op on sync results) against the packaged
// server: config parsing, diagnostics, emit to disk and to strings, AST traversal, checker queries, a snapshot
// update after an on-disk edit, an in-memory request filesystem, and host filesystem callbacks. Each step prints
// `ok` or `FAIL <reason>`; the process exits 1 if any step failed, after running all of them.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = require("@maschwenk/tsrs");
assert.match(root.version, /^\d+\.\d+\.\d+.*-ts/);
assert.match(root.typescriptCommit, /^[0-9a-f]{40}$/);

const sync = await import("@maschwenk/tsrs/unstable/sync");
const asyncApi = await import("@maschwenk/tsrs/unstable/async");
const { SyntaxKind } = await import("@maschwenk/tsrs/unstable/ast");
const { createFileSystemWithLib, serverFS } = await import("@maschwenk/tsrs/unstable/fs");

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

if (only !== "async") await runVariant("sync", sync.API);
if (only !== "sync") await runVariant("async", asyncApi.API);

const failed = results.filter(([, r]) => r !== "ok");
console.log(`${failed.length ? "smoke FAILED" : "smoke ok"}: ${results.length - failed.length}/${results.length} steps passed`);
process.exitCode = failed.length ? 1 : 0;
