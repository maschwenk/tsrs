// Project references and .tsbuildinfo through the BuildOrchestrator API. Upstream already covers build/clean,
// selected projects and rebuilds after edits; this adds the no-op up-to-date build, up-to-date detection from
// buildinfo in a new session over the same filesystem, buildinfo shape, and a referenced project's API change
// surfacing as a downstream diagnostic.

import { API } from "@typescript/typescript/unstable/sync";
import { describe, test } from "node:test";
import { cwd, diags, golden, recordingFS, softGolden } from "./utils.ts";

const g = golden("build");
const soft = softGolden("build");
const files = {
    "/a/tsconfig.json": JSON.stringify({ compilerOptions: { composite: true, outDir: "dist", rootDir: "src", strict: true }, files: ["src/index.ts"] }),
    "/a/src/index.ts": `export const a: number = 1;\nexport function twice(n: number) { return n * 2; }\n`,
    "/c/tsconfig.json": JSON.stringify({ compilerOptions: { composite: true, outDir: "dist", rootDir: "src", strict: true }, files: ["src/index.ts"], references: [{ path: "../a" }] }),
    "/c/src/index.ts": `import { a, twice } from "../../a/src/index";\nexport const c: number = twice(a);\n`,
};

function summary(r: any) {
    return { status: r.status, statistics: r.statistics, diagnostics: r.diagnostics && diags(r.diagnostics).map((d: any) => [d.fileName, d.code, d.text]) };
}
function buildinfoShape(text: string | undefined) {
    if (typeof text !== "string") return { present: false };
    const json = JSON.parse(text);
    return { present: true, keys: Object.keys(json).sort(), root: json.root, fileNames: json.fileNames };
}

describe("parity: project references and buildinfo", () => {
    test("up-to-date builds, buildinfo across sessions and downstream errors", t => {
        const { vfs, writes } = recordingFS(files);
        const take = () => {
            const w = Object.keys(writes).sort();
            for (const k of w) delete writes[k];
            return w;
        };
        {
            using api = new API({ cwd, fs: vfs });
            const orchestrator = api.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/" });
            g.check("first.response", summary(orchestrator.build()));
            g.check("first.writes", take());
            g.check("first.buildinfo.a", buildinfoShape(vfs.readFile("/a/tsconfig.tsbuildinfo")));
            g.check("first.buildinfo.c", buildinfoShape(vfs.readFile("/c/tsconfig.tsbuildinfo")));
            soft.check(t, "first.buildinfo.c.text", vfs.readFile("/c/tsconfig.tsbuildinfo"));
            g.check("first.output.c", vfs.readFile("/c/dist/index.js"));
            g.check("noop.response", summary(orchestrator.build()));
            g.check("noop.writes", take());
        }
        {
            // A new session over the same files: buildinfo alone must make the build up to date.
            using api = new API({ cwd, fs: vfs });
            const orchestrator = api.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/" });
            g.check("newSession.response", summary(orchestrator.build()));
            g.check("newSession.writes", take());

            // Breaking change in the referenced project.
            vfs.writeFile("/a/src/index.ts", `export const a: string = "one";\nexport function twice(n: number) { return n * 2; }\n`);
            take();
            g.check("breaking.response", summary(orchestrator.build()));
            g.check("breaking.writes", take());

            // Body-only change in the referenced project: its d.ts is unchanged.
            vfs.writeFile("/a/src/index.ts", `export const a: number = 2;\nexport function twice(n: number) { return n + n; }\n`);
            take();
            g.check("bodyOnly.response", summary(orchestrator.build()));
            g.check("bodyOnly.writes", take());
        }
    });

    test("build options: dry, force, stopBuildOnErrors, clean then rebuild, use after dispose", t => {
        const broken = { ...files, "/a/src/index.ts": `export const a: number = "bad";\nexport function twice(n: number) { return n * 2; }\n` };
        const { vfs, writes } = recordingFS(files);
        const take = () => {
            const w = Object.keys(writes).sort();
            for (const k of w) delete writes[k];
            return w;
        };
        using api = new API({ cwd, fs: vfs });
        const dry = api.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/", dry: true });
        g.check("opts.dry", { response: summary(dry.build()), writes: take() });
        dry.dispose();
        const o = api.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/" });
        g.check("opts.first", { response: summary(o.build()), writes: take() });
        const forced = api.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/", force: true });
        g.check("opts.force", { response: summary(forced.build()), writes: take() });
        forced.dispose();
        const clean = o.clean();
        g.check("opts.clean", { status: clean.status, filesDeleted: [...(clean.filesDeleted ?? [])].sort() });
        g.check("opts.afterClean", { response: summary(o.build()), writes: take() });
        o.dispose();
        let threw = false;
        try {
            o.build();
        }
        catch {
            threw = true;
        }
        g.check("opts.useAfterDispose", threw);

        const { vfs: bvfs, writes: bwrites } = recordingFS(broken);
        using api2 = new API({ cwd, fs: bvfs });
        const stop = api2.createBuildOrchestrator(["/c/tsconfig.json"], { cwd: "/", stopBuildOnErrors: true });
        g.check("opts.stopBuildOnErrors", { response: summary(stop.build()), writes: Object.keys(bwrites).sort() });
        soft.check(t, "opts.stopBuildOnErrors.buildinfo", buildinfoShape(bvfs.readFile("/a/tsconfig.tsbuildinfo") as any));
    });
});
