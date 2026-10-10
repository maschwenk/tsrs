#!/usr/bin/env node
// Installed-package check: installs the tarballs from `node npm/build.mjs --pack --out <dist>` into a fresh project
// outside the repository (offline, no registry) and drives the *installed* tsrs binary through
//   - the CLI (ordinary type errors still fail, clean code still passes),
//   - unstable/sync and unstable/async with the default binary resolution (no tsserverPath), on the real OS
//     filesystem: config program, diagnostics, and emit that follows compiler options without any env opt-in,
//   - the published declarations, typechecked from a nodenext consumer with the installed tsrs, including a
//     deliberate error that only appears if the API types really resolved.
//
//   node tools/node-api/consumer.mjs --dist <dir> [--keep]

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const dist = path.resolve(args[args.indexOf("--dist") + 1] ?? "");
const keep = args.includes("--keep");
if (!args.includes("--dist") || !fs.existsSync(path.join(dist, "packages.json"))) {
    console.error("usage: consumer.mjs --dist <dir with packages.json from npm/build.mjs --pack> [--keep]");
    process.exit(2);
}
const tarballs = JSON.parse(fs.readFileSync(path.join(dist, "packages.json"), "utf8")).packages.map(p => path.resolve(dist, p.tarball));
const win = process.platform === "win32";
const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-node-api-consumer-")));
const env = { ...process.env };
delete env.TSRS_EMIT; // emit must follow compiler options alone
let failures = 0;
function check(cond, what) {
    console.log(`${cond ? "ok  " : "FAIL"} ${what}`);
    if (!cond) failures++;
}
function run(cmd, argv, opts = {}) {
    return spawnSync(cmd, argv, { cwd: dir, encoding: "utf8", env, shell: win, timeout: 120_000, ...opts });
}
const write = (rel, text) => {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), text);
};

try {
    write("package.json", JSON.stringify({ name: "consumer", private: true, type: "module" }));
    execFileSync("pnpm", ["add", "--offline", ...tarballs], { cwd: dir, stdio: "inherit", env, shell: win });
    const pkgDir = path.join(dir, "node_modules", "tsrs");
    const pkg = JSON.parse(fs.readFileSync(path.join(pkgDir, "package.json"), "utf8"));
    for (const sub of ["./unstable/sync", "./unstable/async"]) check(pkg.exports?.[sub] !== undefined, `package exports ${sub}`);
    const bin = path.join(dir, "node_modules", ".bin", win ? "tsrs.cmd" : "tsrs");

    // ── CLI regressions ──
    const version = run(bin, ["--version"]);
    check(version.status === 0 && /Version/.test(version.stdout), `tsrs --version (${version.stdout.trim()})`);
    write("cli/ok.ts", "export const x: number = 1;\n");
    write("cli/bad.ts", "export const x: number = \"no\";\n");
    check(run(bin, ["--noEmit", "cli/ok.ts"]).status === 0, "CLI: clean file passes");
    const bad = run(bin, ["--noEmit", "cli/bad.ts"]);
    check(bad.status !== 0 && /TS2322/.test(bad.stdout), "CLI: type error reported with TS2322");

    // ── API projects on the OS filesystem ──
    write("proj/tsconfig.json", JSON.stringify({ compilerOptions: { strict: true, declaration: true, outDir: "dist", rootDir: "src", target: "es2022", module: "nodenext" } }));
    write("proj/package.json", JSON.stringify({ type: "module" }));
    write("proj/src/index.ts", "import { helper } from \"./helper.js\";\nexport const value: number = helper(2);\n");
    write("proj/src/helper.ts", "export function helper(n: number) { return n * 2; }\nexport const wrong: string = 1;\n");
    const proj = path.join(dir, "proj").replace(/\\/g, "/");
    const script = (api, flavor) => `
import { API } from "tsrs/unstable/${api}";
import fs from "node:fs";
const out = {};
const api = new API({ cwd: ${JSON.stringify(proj)} });
try {
    const snapshot = await api.createSnapshot({ openProject: ${JSON.stringify(proj + "/tsconfig.json")} });
    const project = snapshot.getConfiguredProject(${JSON.stringify(proj + "/tsconfig.json")});
    out.files = (await project.program.getSourceFileNames()).filter(f => f.includes("/src/")).sort();
    out.diags = (await project.program.getSemanticDiagnostics(${JSON.stringify(proj + "/src/helper.ts")})).map(d => [d.code, d.startPosition?.line, d.startPosition?.character]);
    const emitted = await project.program.emit();
    out.emitSkipped = emitted.emitSkipped;
    out.emitted = emitted.emittedFiles.map(f => f.slice(${JSON.stringify(proj.length)})).sort();
    out.js = fs.readFileSync(${JSON.stringify(proj + "/dist/index.js")}, "utf8");
    out.dts = fs.readFileSync(${JSON.stringify(proj + "/dist/helper.d.ts")}, "utf8");
    out.flavor = ${JSON.stringify(flavor)};
} finally {
    await api.close();
}
console.log(JSON.stringify(out));
`;
    const expected = {
        files: [`${proj}/src/helper.ts`, `${proj}/src/index.ts`],
        diags: [[2322, 1, 13]],
        emitSkipped: false,
        emitted: ["/dist/helper.d.ts", "/dist/helper.js", "/dist/index.d.ts", "/dist/index.js"],
        js: "import { helper } from \"./helper.js\";\nexport const value = helper(2);\n",
        dts: "export declare function helper(n: number): number;\nexport declare const wrong: string;\n",
    };
    for (const flavor of ["sync", "async"]) {
        fs.rmSync(path.join(dir, "proj", "dist"), { recursive: true, force: true });
        write(`${flavor}.mjs`, script(flavor, flavor));
        const r = run(process.execPath, [`${flavor}.mjs`], { shell: false });
        let got;
        try {
            got = JSON.parse(r.stdout.trim().split("\n").pop());
        }
        catch {
            got = undefined;
        }
        if (!got) {
            check(false, `${flavor} API: script exited ${r.status} ${r.signal ?? ""}\n${(r.stderr || r.stdout).slice(0, 2000)}`);
            continue;
        }
        for (const key of Object.keys(expected)) {
            check(JSON.stringify(got[key]) === JSON.stringify(expected[key]), `${flavor} API: ${key}${JSON.stringify(got[key]) === JSON.stringify(expected[key]) ? "" : ` = ${JSON.stringify(got[key])}, expected ${JSON.stringify(expected[key])}`}`);
        }
    }

    // ── published declarations ──
    write("types/use.mts", [
        `import { API, type Snapshot } from "tsrs/unstable/sync";`,
        `import { API as AsyncAPI } from "tsrs/unstable/async";`,
        `import { SyntaxKind } from "tsrs/unstable/ast";`,
        `export function open(api: API): Snapshot { return api.createSnapshot({ openProject: "/tsconfig.json" }); }`,
        `export async function openAsync(api: AsyncAPI) { return (await api.createSnapshot()).getProjects().length; }`,
        `export const kind: SyntaxKind = SyntaxKind.Identifier;`,
        `export const wrong: number = new API({}).createSnapshot();`,
    ].join("\n") + "\n");
    write("types/tsconfig.json", JSON.stringify({ compilerOptions: { module: "nodenext", target: "es2022", lib: ["es2022", "esnext.disposable"], strict: true, noEmit: true, types: [] }, files: ["use.mts"] }));
    const types = run(bin, ["-p", "types/tsconfig.json"]);
    const typeErrors = types.stdout.split("\n").filter(l => /error TS\d+/.test(l));
    check(typeErrors.length === 1 && /use\.mts\(7,14\).*TS2322.*Snapshot/.test(typeErrors[0]), `declarations resolve from nodenext (exactly the deliberate TS2322): ${typeErrors.join(" | ") || "no errors"}`);
}
finally {
    if (keep) console.log(`kept ${dir}`);
    else fs.rmSync(dir, { recursive: true, force: true });
}
console.log(failures === 0 ? "consumer: all checks passed" : `consumer: ${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
