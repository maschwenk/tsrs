// Callback re-entry repro against a real API server (tsrs or the pinned Go tsgo), using the pinned Node
// clients. Each scenario runs in its own child process with a hard timeout so a hang is reported, not
// suffered. Usage:
//   TS_REF=<ts-ref> node --conditions=@typescript/source reentry_repro.mjs <server-exe> [timeoutMs]
// Child mode (internal): ... reentry_repro.mjs --child <server-exe> <sync|async> <scenario>
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const api = `${process.env.TS_REF}/packages/typescript/src/api`;
const SCENARIOS = [
    // foreign / uncontended nested requests
    "ping",
    "parseConfigFile",
    "createSnapshotOther",
    "getDiagnosticsOnPrevious",
    // snapshot operations that touch the snapshot the outer request is building from / the same host
    "updateSnapshotOfPrevious",
    "createSnapshotSameProject",
    "releasePrevious",
    // the nested request needs what the outer request is building right now
    "createSnapshotInFlightProject",
    "openInFlightFile",
    // outer request is an orchestrator build holding the build lock; nested build on it
    "buildDuringBuild",
    "pingDuringBuild",
];

// `extra` adds independent files so the program is parsed by several worker threads and filesystem
// callbacks arrive concurrently from them while the nested request runs.
function makeProject(root, name, extra = 0) {
    const dir = join(root, name);
    mkdirSync(dir, { recursive: true });
    const files = ["a.ts", "b.ts"];
    for (let i = 0; i < extra; i++) {
        files.push(`x${i}.ts`);
        writeFileSync(join(dir, `x${i}.ts`), `import { a } from './a';\nexport const x${i} = a + ${i};\n`);
    }
    writeFileSync(join(dir, "tsconfig.json"), JSON.stringify({ compilerOptions: { strict: true, noEmit: true, types: [] }, files }));
    writeFileSync(join(dir, "a.ts"), "export const a: number = 1;\n");
    writeFileSync(join(dir, "b.ts"), "import { a } from './a';\nexport const b: string = a;\n");
    return dir;
}

async function child(exe, mode, scenario) {
    const { serverFS } = await import(`${api}/fs.ts`);
    const root = mkdtempSync(join(tmpdir(), "tsrs-reentry-"));
    const p1 = makeProject(root, "p1");
    const p2 = makeProject(root, "p2");
    const p3 = makeProject(root, "p3", 48);
    let client;
    let armed = false;
    let nested;
    const isAsync = mode === "async";
    const req = (m, params) => client.apiRequest(m, params);
    const firstProject = r => r.projects[0].id;
    let previous;
    let orchestrator;
    const isBuild = scenario.endsWith("DuringBuild");
    const outerRequest = () => isBuild
        ? req("build", { buildOrchestratorID: orchestrator })
        : req("createSnapshot", { openProjects: [join(p3, "tsconfig.json")] });
    const outerSummary = v => isBuild ? `status ${v.status}` : v.projects.length;
    const nestedCall = () => {
        switch (scenario) {
            case "ping": return req("ping", null);
            case "parseConfigFile": return req("parseConfigFile", { file: join(p2, "tsconfig.json") });
            case "createSnapshotOther": return req("createSnapshot", { openProjects: [join(p2, "tsconfig.json")] });
            case "getDiagnosticsOnPrevious": return req("getSemanticDiagnostics", { snapshot: previous.snapshot, project: firstProject(previous) });
            case "updateSnapshotOfPrevious": return req("updateSnapshot", { snapshot: previous.snapshot, changes: { openProjects: [join(p2, "tsconfig.json")] } });
            case "createSnapshotSameProject": return req("createSnapshot", { openProjects: [join(p1, "tsconfig.json")] });
            case "releasePrevious": return req("release", { snapshot: previous.snapshot });
            case "createSnapshotInFlightProject": return req("createSnapshot", { openProjects: [join(p3, "tsconfig.json")] });
            case "openInFlightFile": return req("createSnapshot", { openFiles: [join(p3, "b.ts")] });
            case "buildDuringBuild": return req("build", { buildOrchestratorID: orchestrator });
            case "pingDuringBuild": return req("ping", null);
        }
    };
    const summarize = v => v === undefined ? "undefined" : JSON.stringify(v).slice(0, 80);
    const onRead = p => {
        if (armed && p === join(p3, "b.ts")) {
            armed = false;
            if (isAsync) {
                return Promise.resolve(nestedCall()).then(
                    v => { nested = "ok " + summarize(v); return serverFS.useOS; },
                    e => { nested = "error " + e.message.split("\n")[0]; return serverFS.useOS; },
                ).then(() => serverFS.useOS);
            }
            try {
                nested = "ok " + summarize(nestedCall());
            }
            catch (e) {
                nested = "error " + e.message.split("\n")[0];
            }
        }
        return serverFS.useOS;
    };
    const fs = {
        readFile: isAsync ? undefined : onRead,
        fileExists: serverFS.useOS, directoryExists: serverFS.useOS, getAccessibleEntries: serverFS.useOS,
        realpath: serverFS.useOS, stat: serverFS.useOS, writeFile: serverFS.useOS, removeFile: serverFS.useOS,
    };
    const opts = { tsserverPath: exe, cwd: root, fs };
    let outer;
    if (isAsync) {
        // The pinned async fsCallbacks API takes plain values; register readFile as a raw callback that
        // may await nested API calls (vscode-jsonrpc handles requests concurrently).
        const { Client } = await import(`${api}/async/client.ts`);
        fs.readFile = p => p; // placeholder so --callbacks includes readFile
        client = new Client(opts);
        await client.connect();
        client.registerCallback("readFile", async p => {
            await onRead(p);
            return { kind: "useOS" };
        });
        previous = await req("createSnapshot", { openProjects: [join(p1, "tsconfig.json")] });
        if (isBuild) orchestrator = (await req("createBuildOrchestrator", { rootNames: [join(p3, "tsconfig.json")] })).buildOrchestratorID;
        armed = true;
        try {
            outer = "ok " + summarize(outerSummary(await outerRequest()));
        }
        catch (e) {
            outer = "error " + e.message.split("\n")[0];
        }
        await client.close();
    }
    else {
        const { Client } = await import(`${api}/sync/client.ts`);
        client = new Client(opts);
        previous = req("createSnapshot", { openProjects: [join(p1, "tsconfig.json")] });
        if (isBuild) orchestrator = req("createBuildOrchestrator", { rootNames: [join(p3, "tsconfig.json")] }).buildOrchestratorID;
        armed = true;
        try {
            outer = "ok " + summarize(outerSummary(outerRequest()));
        }
        catch (e) {
            outer = "error " + e.message.split("\n")[0];
        }
        client.close();
    }
    rmSync(root, { recursive: true, force: true });
    process.stdout.write(JSON.stringify({ nested: nested ?? "callback not reached", outer }) + "\n");
}

if (process.argv[2] === "--child") {
    await child(process.argv[3], process.argv[4], process.argv[5]);
    process.exit(0);
}
else {
    const exe = process.argv[2];
    const timeout = Number(process.argv[3] ?? 30000);
    const self = fileURLToPath(import.meta.url);
    const rows = [];
    const only = process.env.SCENARIOS?.split(",");
    for (const mode of ["sync", "async"]) {
        for (const scenario of SCENARIOS.filter(s => !only || only.includes(s))) {
            const start = Date.now();
            const r = spawnSync(process.execPath, ["--conditions=@typescript/source", self, "--child", exe, mode, scenario], { timeout, encoding: "utf8", killSignal: "SIGKILL" });
            const ms = Date.now() - start;
            let result;
            if (r.error?.code === "ETIMEDOUT" || r.signal === "SIGKILL") result = { outcome: "HANG", detail: `killed after ${timeout} ms` };
            else if (r.status !== 0) result = { outcome: "CRASH", detail: (r.stderr || "").trim().split("\n").slice(-3).join(" | ") };
            else result = { outcome: "DONE", ...JSON.parse(r.stdout.trim().split("\n").pop()) };
            rows.push({ mode, scenario, ms, ...result });
            console.log(JSON.stringify(rows.at(-1)));
        }
    }
}
