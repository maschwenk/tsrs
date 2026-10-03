// Async-mode false positive check for the re-entrancy gates: request C waits on a slow client callback
// (no nested API call), while two independent requests A and B contend for the same exclusive resource
// (the program's API checker, or the build orchestrator). Nobody re-enters, so nothing can deadlock;
// pinned Go serves A and B. Usage (pinned client from ts-ref):
//   TS_REF=<ts-ref> node --conditions=@typescript/source contention_repro.mjs <server-exe> [rounds]
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const api = `${process.env.TS_REF}/packages/typescript/src/api`;
const { Client } = await import(`${api}/async/client.ts`);
const exe = process.argv[2];
const rounds = Number(process.argv[3] ?? 5);

function project(root, name, n) {
    const dir = join(root, name);
    mkdirSync(dir, { recursive: true });
    const files = [];
    for (let i = 0; i < n; i++) {
        files.push(`f${i}.ts`);
        // Enough checker work per file that two diagnostics requests overlap.
        writeFileSync(join(dir, `f${i}.ts`), `type Deep${i}<T, N extends unknown[] = []> = N["length"] extends 25 ? T : Deep${i}<{ v: T }, [...N, 0]>;\nexport const v${i}: Deep${i}<string> = null!;\nexport const bad${i}: number = "x";\n`);
    }
    writeFileSync(join(dir, "tsconfig.json"), JSON.stringify({ compilerOptions: { strict: true, noEmit: true, types: [] }, files }));
    return dir;
}

const root = mkdtempSync(join(tmpdir(), "tsrs-contention-"));
const heavy = project(root, "heavy", 300);
const slow = project(root, "slow", 2);
const tick = () => new Promise(r => setImmediate(r));
const { serverFS } = await import(`${api}/fs.ts`);
let gate = null;
const results = [];
const fs = {
    readFile: p => p, fileExists: serverFS.useOS, directoryExists: serverFS.useOS, getAccessibleEntries: serverFS.useOS,
    realpath: serverFS.useOS, stat: serverFS.useOS, writeFile: serverFS.useOS, removeFile: serverFS.useOS,
};
const client = new Client({ tsserverPath: exe, cwd: root, fs });
await client.connect();
client.registerCallback("readFile", async p => {
    if (gate && p.startsWith(slow)) {
        const g = gate;
        gate = null;
        g.reached();
        await g.release;
    }
    return { kind: "useOS" };
});
const snap = await client.apiRequest("createSnapshot", { openProjects: [join(heavy, "tsconfig.json")] });
const project0 = snap.projects[0].id;
const resource = process.env.RESOURCE ?? "build";
const orchestrator = (await client.apiRequest("createBuildOrchestrator", { rootNames: [join(heavy, "tsconfig.json")] })).buildOrchestratorID;
const typeAt = () => client.apiRequest("getTypeAtPosition", { snapshot: snap.snapshot, project: project0, file: join(heavy, "f1.ts"), position: 140 });
for (let round = 0; round < rounds; round++) {
    let reached, release;
    const reachedP = new Promise(r => (reached = r));
    const releaseP = new Promise(r => (release = r));
    gate = { reached, release: releaseP };
    const outer = client.apiRequest("createSnapshot", { openProjects: [join(slow, "tsconfig.json")] }).then(() => "ok", e => "error " + e.message);
    await reachedP; // C is now waiting on the client
    const op = resource === "build"
        ? () => client.apiRequest("build", { buildOrchestratorID: orchestrator }).then(d => `ok status ${d.status}`, e => "error " + e.message.split("\n")[0])
        : () => typeAt().then(d => `ok ${d ? "type" : "none"}`, e => "error " + e.message.split("\n")[0]);
    const diag = op;
    const a = diag();
    await tick();
    const b = diag();
    await tick();
    const d = diag();
    const r = { round, a: await a, b: await b, c: await d };
    release();
    r.outer = await outer;
    results.push(r);
    console.log(JSON.stringify(r));
}
await client.close();
rmSync(root, { recursive: true, force: true });
const spurious = results.flatMap(r => [r.a, r.b, r.c]).filter(x => x.startsWith("error")).length;
console.log(JSON.stringify({ rounds, spuriousErrors: spurious }));
