// Incremental snapshots: after edits, creations and deletions the new snapshot sees the change while the old
// snapshot (still undisposed) keeps answering program, diagnostic and checker queries from its own state, also
// after the newer snapshot is disposed. Upstream covers source text retention and filesystem layering; this adds
// diagnostics/types/file lists on the retained snapshot and created/deleted paths through host notifications.

import { describe, test } from "node:test";
import { diags, golden, syncAPI } from "./utils.ts";

const g = golden("snapshot");
const b = `import { value } from "./a";\nimport { later } from "./later";\nexport const use = value;\nexport const l = later;\n`;

function observe(snapshot: any) {
    const project = snapshot.getConfiguredProject("/tsconfig.json")!;
    const { program, checker } = project;
    const pos = b.indexOf("value;");
    let missing: unknown;
    try {
        missing = program.getSourceFile("/src/c.ts") === undefined ? "undefined" : "present";
    }
    catch (e) {
        missing = { threw: true };
    }
    return {
        names: program.getSourceFileNames().filter((f: string) => f.startsWith("/src/")).sort(),
        bDiags: diags(program.getSemanticDiagnostics("/src/b.ts")).map(d => [d.code, d.text]),
        typeAtUse: checker.typeToString(checker.getTypeAtPosition("/src/b.ts", pos)!),
        aText: program.getSourceFile("/src/a.ts")!.text,
        c: missing,
    };
}

describe("parity: snapshots during edits", () => {
    test("old snapshot stays queryable across change/create/delete and disposal of the newer one", () => {
        const { api, vfs } = syncAPI({
            "/tsconfig.json": `{ "compilerOptions": { "strict": true }, "include": ["src"] }`,
            "/src/a.ts": `export const value: number = 1;\n`,
            "/src/b.ts": b,
            "/src/c.ts": `export const c = 1;\n`,
        });
        using _ = api;
        const snap1 = api.createSnapshot({ openProject: "/tsconfig.json" });
        g.check("before", observe(snap1));

        vfs.writeFile("/src/a.ts", `export const value: string = "s";\n`);
        vfs.writeFile("/src/later.ts", `export const later = true;\n`);
        vfs.removeFile("/src/c.ts");
        const notifications = { changed: ["/src/a.ts"], created: ["/src/later.ts"], deleted: ["/src/c.ts"] };
        // Without ensurePrograms the updated snapshot's programs are not rebuilt yet (pinned semantics).
        const lazy = snap1.update({ fileNotifications: notifications });
        g.check("lazy", observe(lazy));
        lazy.dispose();
        const snap2 = snap1.update({ fileNotifications: notifications, ensurePrograms: true });
        g.check("new", observe(snap2));
        g.check("old.afterUpdate", observe(snap1));

        snap2.dispose();
        g.check("old.afterNewDisposed", observe(snap1));

        // Recreating from scratch sees the latest host state.
        const snap3 = api.createSnapshot({ openProject: "/tsconfig.json" });
        g.check("fresh", observe(snap3));
        snap1.dispose();
        g.check("fresh.afterOldDisposed", observe(snap3));
    });

    test("deleting a file a program imports and restoring it", () => {
        const { api, vfs } = syncAPI({
            "/tsconfig.json": `{ "compilerOptions": { "strict": true }, "files": ["src/main.ts"] }`,
            "/src/main.ts": `import { dep } from "./dep";\nexport const x: number = dep;\n`,
            "/src/dep.ts": `export const dep = 1;\n`,
        });
        using _ = api;
        const program = (s: any) => s.getConfiguredProject("/tsconfig.json")!.program;
        const s1 = api.createSnapshot({ openProject: "/tsconfig.json" });
        vfs.removeFile("/src/dep.ts");
        const s2 = s1.update({ fileNotifications: { deleted: ["/src/dep.ts"] }, ensurePrograms: true });
        vfs.writeFile("/src/dep.ts", `export const dep = "back";\n`);
        const s3 = s2.update({ fileNotifications: { created: ["/src/dep.ts"] }, ensurePrograms: true });
        for (const [name, s] of [["s1", s1], ["s2", s2], ["s3", s3]] as const) {
            g.check(`restore.${name}`, {
                names: program(s).getSourceFileNames().filter((f: string) => f.startsWith("/src/")).sort(),
                diags: diags(program(s).getSemanticDiagnostics("/src/main.ts")).map(d => [d.code, d.text]),
            });
        }
    });
});
