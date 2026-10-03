// Request-filesystem symlink alias invalidation through the real API (Go requestFileSystem.ExpandFileChanges):
// a request layer adds symlink p/link -> p/target (host directory); the project compiles link/a.ts. The host
// file p/target/a.ts is fixed on disk and the client notifies the change at the TARGET path only.
//   TS_REF=<ts-ref> node --conditions=@typescript/source requestfs_alias_repro.mjs <server-exe>
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
const { Client } = await import(`${process.env.TS_REF}/packages/typescript/src/api/sync/client.ts`);
const exe = process.argv[2];
const out = {};
for (const notify of ["target", "alias", "none"]) {
    const root = mkdtempSync(join(tmpdir(), "tsrs-alias-"));
    const p = join(root, "p");
    mkdirSync(join(p, "target"), { recursive: true });
    const config = join(p, "tsconfig.json");
    writeFileSync(config, JSON.stringify({ compilerOptions: { strict: true, types: [] }, files: ["link/a.ts"] }));
    writeFileSync(join(p, "target", "a.ts"), "export const a: number = 'bad';\n");
    const c = new Client({ tsserverPath: exe, cwd: root });
    const s1 = c.apiRequest("createSnapshot", { openProjects: [config], fileSystem: { kind: "layer", files: {}, symlinks: { [join(p, "link")]: { target: join(p, "target") } } } });
    const proj = s1.projects[0].id;
    const diags = s => c.apiRequest("getSemanticDiagnostics", { snapshot: s, project: proj }).length;
    const before = diags(s1.snapshot);
    writeFileSync(join(p, "target", "a.ts"), "export const a: number = 1;\n");
    const changed = notify === "target" ? [join(p, "target", "a.ts")] : notify === "alias" ? [join(p, "link", "a.ts")] : [];
    const s2 = c.apiRequest("updateSnapshot", { snapshot: s1.snapshot, changes: { openProjects: [config], fileNotifications: { changed } } });
    out[notify] = { before, after: diags(s2.snapshot) };
    c.close();
    rmSync(root, { recursive: true, force: true });
}
console.log(JSON.stringify(out));
