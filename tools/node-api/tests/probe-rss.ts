// Standalone memory probe (not a test file): server RSS over repeated API operations, one variant per run.
//
//   cd tools/node-api/.work/<label>/tree/packages/typescript   # a tree prepared by run-upstream.mjs
//   node --conditions @typescript/source test/parity/probe-rss.ts <binary> <variant> [cycles]
//
// Variants: create-plain (createSnapshot + dispose, no queries), create-diag (+ semantic diagnostics),
// create-type (+ checker query), create-edit (+ file edit), update (Snapshot.update + dispose), transpile
// (transpileModule only, no snapshots). Prints one JSON line: samples every cycles/10 cycles, in MiB.

import { API } from "@typescript/typescript/unstable/sync";
import fs from "node:fs";
import { createVirtualFileSystem } from "../testUtils.ts";

const [binary, variant, cyclesArg] = process.argv.slice(2);
const cycles = Number(cyclesArg ?? 120);
const gen = (i: number) => `export const version = ${i};\n` + Array.from({ length: 150 }, (_, k) => `export function f${k}(a: { x: number }) { return a.x + ${i}; }`).join("\n");
const use = `import { f1 } from "./gen"; export const r = f1({ x: 1 });\n`;
const vfs = createVirtualFileSystem({ "/tsconfig.json": `{ "compilerOptions": { "strict": true }, "include": ["src"] }`, "/src/gen.ts": gen(0), "/src/use.ts": use });
const api = new API({ cwd: "/", fs: vfs, tsserverPath: binary });
let snap = api.createSnapshot({ openProject: "/tsconfig.json" });
const child = (api as any).client.channel.child;
const rss = () => Math.round(Number(/VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${child.pid}/status`, "utf8"))![1]) / 1024);
const samples: number[] = [rss()];
for (let i = 1; i <= cycles; i++) {
    if (variant === "transpile") {
        api.transpileModule(gen(i), { compilerOptions: { module: 99 as any } } as any);
    }
    else {
        if (variant === "create-edit" || variant === "update") vfs.writeFile("/src/gen.ts", gen(i));
        const changes = variant === "create-edit" || variant === "update" ? { fileNotifications: { changed: ["/src/gen.ts"] } } : {};
        const next = variant === "update" ? snap.update({ ...changes, ensurePrograms: true }) : api.createSnapshot({ openProject: "/tsconfig.json", ...changes });
        const p = next.getConfiguredProject("/tsconfig.json")!;
        if (variant === "create-diag") p.program.getSemanticDiagnostics("/src/use.ts");
        if (variant === "create-type") p.checker.getTypeAtPosition("/src/use.ts", use.indexOf("r ="));
        snap.dispose();
        snap = next;
    }
    if (i % Math.max(1, Math.floor(cycles / 10)) === 0) samples.push(rss());
}
console.log(JSON.stringify({ variant, cycles, mib: samples }));
api.close();
