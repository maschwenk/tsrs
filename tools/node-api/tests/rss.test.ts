// Repeated create/update/dispose: the server's resident memory must plateau when old snapshots are disposed.
// Samples go to .work/<label>/evidence/rss-*.json for the report. The bound is deliberately loose (allocator and
// GC noise) and identical for every server: after warm-up, 150 more edit/ensure/query/dispose cycles may not
// grow RSS by more than max(96 MiB, 60% of the warm RSS), and the last third may not grow faster than the first.

import { API } from "@typescript/typescript/unstable/sync";
import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { createVirtualFileSystem } from "../testUtils.ts";
import { apiServerPid, cwd, evidence, linuxOnly, rssKiB } from "./utils.ts";

function source(i: number) {
    const lines = [`export const version = ${i};`];
    for (let k = 0; k < 150; k++) lines.push(`export function f${k}(a: { x: number; y: string }, b: readonly number[]): [number, string] { return [a.x + b.length + ${i}, a.y]; }`);
    lines.push(`export type U = ${Array.from({ length: 40 }, (_, k) => `{ k: ${k}; v${k}: string }`).join(" | ")};`);
    return lines.join("\n") + "\n";
}

const useSource = `import { f1, version, type U } from "./gen";\nexport const r = f1({ x: version, y: "" }, []);\nexport const u: U = { k: 0, v0: "" };\n`;

describe("parity: memory over repeated snapshots", () => {
    for (const mode of ["update", "create"] as const) {
        test(`RSS plateaus over ${mode}/dispose cycles`, { skip: linuxOnly, timeout: 300_000 }, () => {
            const vfs = createVirtualFileSystem({
                "/tsconfig.json": `{ "compilerOptions": { "strict": true }, "include": ["src"] }`,
                "/src/gen.ts": source(0),
                "/src/use.ts": useSource,
            });
            using api = new API({ cwd, fs: vfs });
            let snap = api.createSnapshot({ openProject: "/tsconfig.json" });
            const pid = apiServerPid(process.pid);
            assert.ok(pid, "server pid not found");
            const samples: { cycle: number; rssKiB: number; }[] = [];
            const total = 200;
            for (let i = 1; i <= total; i++) {
                vfs.writeFile("/src/gen.ts", source(i));
                const next = mode === "update"
                    ? snap.update({ fileNotifications: { changed: ["/src/gen.ts"] }, ensurePrograms: true })
                    : api.createSnapshot({ openProject: "/tsconfig.json", fileNotifications: { changed: ["/src/gen.ts"] } });
                const project = next.getConfiguredProject("/tsconfig.json")!;
                assert.equal(project.program.getSemanticDiagnostics("/src/use.ts").length, 0);
                const t = project.checker.getTypeAtPosition("/src/use.ts", useSource.indexOf("r ="));
                assert.ok(t);
                snap.dispose();
                snap = next;
                if (i % 10 === 0) samples.push({ cycle: i, rssKiB: rssKiB(pid!) });
            }
            snap.dispose();
            const warm = samples.find(s => s.cycle === 50)!.rssKiB;
            const last = samples[samples.length - 1].rssKiB;
            const third = Math.floor(samples.length / 3);
            const firstThirdGrowth = samples[third].rssKiB - samples[0].rssKiB;
            const lastThirdGrowth = last - samples[samples.length - 1 - third].rssKiB;
            const record = { mode, cycles: total, warmKiB: warm, lastKiB: last, growthKiB: last - warm, firstThirdGrowth, lastThirdGrowth, samples, binary: process.env.NODE_API_BINARY };
            evidence(`rss-${mode}`, record);
            const allowed = Math.max(96 * 1024, warm * 0.6);
            assert.ok(last - warm <= allowed, `RSS grew ${last - warm} KiB after warm-up (allowed ${Math.round(allowed)}): ${JSON.stringify(samples)}`);
            assert.ok(lastThirdGrowth <= Math.max(16 * 1024, firstThirdGrowth), `RSS still growing in the last third (${lastThirdGrowth} KiB vs ${firstThirdGrowth} KiB in the first)`);
        });
    }
});
