// Connection-loss races in the async client (run against the tsrs SDK source, npm/tsrs, which carries the
// connection-loss patch; or any client tree via SDK_API). Each case runs in a child process with a hard
// timeout and records settle outcomes plus any uncaughtException / unhandledRejection.
//   SDK_API=<npm/tsrs/src/api> node --conditions=@typescript/source connloss_repro.mjs <server-exe>
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const CASES = ["killDuringCallback", "closeAfterDeath", "killThenRequestThenClose", "closeDuringCallback", "killDuringBatch"];

async function child(exe, name) {
    const api = process.env.SDK_API;
    const events = [];
    process.on("uncaughtException", e => events.push("uncaughtException: " + e.message));
    process.on("unhandledRejection", e => events.push("unhandledRejection: " + (e?.message ?? e)));
    const { Client } = await import(`${api}/async/client.ts`);
    const { serverFS } = await import(`${api}/fs.ts`);
    const root = mkdtempSync(join(tmpdir(), "tsrs-connloss-"));
    const dir = join(root, "p");
    mkdirSync(dir);
    const files = Array.from({ length: 30 }, (_, i) => `f${i}.ts`);
    for (const f of files) writeFileSync(join(dir, f), `export const v: number = "${f}";\n`);
    writeFileSync(join(dir, "tsconfig.json"), JSON.stringify({ compilerOptions: { strict: true, types: [] }, files }));
    let inCallback;
    const callbackEntered = new Promise(r => (inCallback = r));
    let callbackFinished;
    const fs = {
        readFile: p => p, fileExists: serverFS.useOS, directoryExists: serverFS.useOS, getAccessibleEntries: serverFS.useOS,
        realpath: serverFS.useOS, stat: serverFS.useOS, writeFile: serverFS.useOS, removeFile: serverFS.useOS,
    };
    const client = new Client({ tsserverPath: exe, cwd: root, fs });
    await client.connect();
    const blockCallback = name.endsWith("DuringCallback");
    client.registerCallback("readFile", async p => {
        if (blockCallback && p.endsWith("f3.ts")) {
            inCallback();
            await new Promise(r => setTimeout(r, 300));
            callbackFinished = true;
        }
        return { kind: "useOS" };
    });
    const settle = p => p.then(v => "resolved", e => "rejected: " + e.message.split("\n")[0]);
    const out = {};
    const kill = () => client.process.kill("SIGKILL");
    const exited = () => new Promise(r => client.process.exitCode !== null || client.process.signalCode ? r() : client.process.once("exit", r));
    const config = join(dir, "tsconfig.json");
    switch (name) {
        case "killDuringCallback": {
            const req = settle(client.apiRequest("createSnapshot", { openProjects: [config] }));
            await callbackEntered;
            kill();
            out.request = await req;
            await new Promise(r => setTimeout(r, 400)); // let the callback finish and try to reply
            out.callbackFinished = !!callbackFinished;
            out.later = await settle(client.apiRequest("ping", null));
            out.close = await settle(client.close());
            break;
        }
        case "closeDuringCallback": {
            const req = settle(client.apiRequest("createSnapshot", { openProjects: [config] }));
            await callbackEntered;
            const closing = settle(client.close());
            out.close = await closing;
            out.request = await Promise.race([req, new Promise(r => setTimeout(() => r("pending after 2s"), 2000))]);
            const proc = client.process ?? null;
            await new Promise(r => setTimeout(r, 400));
            out.callbackFinished = !!callbackFinished;
            break;
        }
        case "closeAfterDeath": {
            await settle(client.apiRequest("ping", null));
            const proc = client.process;
            kill();
            await new Promise(r => proc.once("exit", r));
            out.later = await settle(client.apiRequest("ping", null));
            out.close = await settle(client.close());
            await new Promise(r => setTimeout(r, 200));
            break;
        }
        case "killThenRequestThenClose": {
            await settle(client.apiRequest("ping", null));
            kill(); // no await: the request races the exit event
            out.request = await settle(client.apiRequest("createSnapshot", { openProjects: [config] }));
            out.close = await settle(client.close());
            await new Promise(r => setTimeout(r, 200));
            break;
        }
        case "killDuringBatch": {
            const reqs = [0, 1, 2].map(() => settle(client.apiRequest("ping", null)));
            const big = settle(client.apiRequest("createSnapshot", { openProjects: [config] }));
            setImmediate(kill);
            out.batch = await Promise.all(reqs);
            out.request = await big;
            out.close = await settle(client.close());
            await new Promise(r => setTimeout(r, 200));
            break;
        }
    }
    rmSync(root, { recursive: true, force: true });
    out.events = events;
    process.stdout.write(JSON.stringify(out) + "\n");
}

if (process.argv[2] === "--child") {
    await child(process.argv[3], process.argv[4]);
    process.exit(0);
}
else {
    const self = fileURLToPath(import.meta.url);
    for (const name of CASES) {
        const r = spawnSync(process.execPath, ["--conditions=@typescript/source", self, "--child", process.argv[2], name], { timeout: 20000, encoding: "utf8", killSignal: "SIGKILL", env: process.env });
        const outcome = r.signal === "SIGKILL" || r.error?.code === "ETIMEDOUT" ? { outcome: "HANG" } : r.status !== 0 ? { outcome: `EXIT ${r.status}`, stderr: r.stderr.trim().split("\n").slice(-4).join(" | ") } : { outcome: "DONE", ...JSON.parse(r.stdout.trim().split("\n").pop()) };
        console.log(JSON.stringify({ case: name, ...outcome }));
    }
}
