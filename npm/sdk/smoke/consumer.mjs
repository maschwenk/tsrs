// Runs in a temp consumer project that installed the packed @maschwenk/tsrs tarballs (see ../smoke-consumer.mjs).
// It compiles a small on-disk project through both the sync and the async JS API: config, diagnostics, emit, AST
// traversal and checker queries, then closes the server. Any failure exits non-zero.
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
const ast = await import("@maschwenk/tsrs/unstable/ast");
const { SyntaxKind } = ast;

const project = path.resolve(process.argv[2] ?? "project");
fs.rmSync(project, { recursive: true, force: true });
fs.mkdirSync(path.join(project, "src"), { recursive: true });
fs.writeFileSync(path.join(project, "tsconfig.json"), JSON.stringify({
    compilerOptions: { strict: true, target: "es2022", module: "nodenext", rootDir: "src", outDir: "out", declaration: true, types: [] },
    include: ["src"],
}));
fs.writeFileSync(path.join(project, "src", "lib.ts"), "export function twice(n: number): number {\n    return n * 2;\n}\n");
fs.writeFileSync(path.join(project, "src", "main.ts"), 'import { twice } from "./lib.js";\nexport const answer = twice(21);\nexport const wrong: string = twice(1);\n');
const configPath = path.join(project, "tsconfig.json").split(path.sep).join("/");
const mainPath = path.join(project, "src", "main.ts").split(path.sep).join("/");

function checkProgramResult({ diagnostics, emit, answerType, statementKinds }, label) {
    assert.equal(diagnostics.length, 1, `${label}: one semantic diagnostic`);
    assert.equal(diagnostics[0].code, 2322, `${label}: TS2322`);
    assert.equal(emit.emitSkipped, false, `${label}: emit not skipped`);
    for (const out of ["out/main.js", "out/main.d.ts", "out/lib.js", "out/lib.d.ts"]) {
        assert.ok(fs.existsSync(path.join(project, out)), `${label}: ${out} written`);
    }
    assert.match(fs.readFileSync(path.join(project, "out/main.js"), "utf8"), /twice\)?\(21\)/, `${label}: emitted JS`);
    assert.equal(answerType, "number", `${label}: checker type of answer`);
    assert.deepEqual(statementKinds, [SyntaxKind.ImportDeclaration, SyntaxKind.VariableStatement, SyntaxKind.VariableStatement], `${label}: AST`);
}

function answerNode(sourceFile) {
    const statement = sourceFile.statements[1];
    return statement.declarationList.declarations[0].name;
}

// --- sync ---
{
    const api = new sync.API({ cwd: project });
    try {
        const config = api.parseConfigFile(configPath);
        assert.deepEqual(config.fileNames.map(f => path.basename(f)).sort(), ["lib.ts", "main.ts"]);
        const snapshot = api.createSnapshot({ openProject: configPath });
        const p = snapshot.getConfiguredProject(configPath);
        assert.ok(p, "sync: configured project");
        const sourceFile = p.program.getSourceFile(mainPath);
        assert.ok(sourceFile, "sync: source file");
        const type = p.checker.getTypeAtLocation(answerNode(sourceFile));
        checkProgramResult({
            diagnostics: p.program.getSemanticDiagnostics(mainPath),
            emit: p.program.emit(),
            answerType: p.checker.typeToString(type),
            statementKinds: sourceFile.statements.map(s => s.kind),
        }, "sync");
        snapshot.dispose();
    }
    finally {
        api.close();
    }
}

fs.rmSync(path.join(project, "out"), { recursive: true, force: true });

// --- async ---
{
    const api = new asyncApi.API({ cwd: project });
    try {
        const snapshot = await api.createSnapshot({ openProject: configPath });
        const p = snapshot.getConfiguredProject(configPath);
        assert.ok(p, "async: configured project");
        const sourceFile = await p.program.getSourceFile(mainPath);
        assert.ok(sourceFile, "async: source file");
        const type = await p.checker.getTypeAtLocation(answerNode(sourceFile));
        checkProgramResult({
            diagnostics: await p.program.getSemanticDiagnostics(mainPath),
            emit: await p.program.emit(),
            answerType: await p.checker.typeToString(type),
            statementKinds: sourceFile.statements.map(s => s.kind),
        }, "async");
        await snapshot.dispose();
    }
    finally {
        await api.close();
    }
}

console.log("smoke ok: sync + async config/diagnostics/emit/AST/checker");
