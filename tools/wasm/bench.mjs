#!/usr/bin/env node
// Times tsrs.wasm against native tsrs --singleThreaded (and other wasm modules, e.g. a second opt-level or ts-rust's
// module) on bench projects, interleaved so that every engine sees the same machine load.
//   cold = median of the first tsc() call in N new Node processes (includes compiling the module)
//   warm = median of the later calls in the first process
//   native = median of N `/usr/bin/time -l` runs (wall, peak RSS, instructions retired)
//
//   node tools/wasm/bench.mjs --native <tsrs> --wasm name=<module.wasm> [--wasm ...] [--ts-rust <repo>=<module>]
//        --project name=<cwd>:<args...> [...] [--runs 5] [--out <file.json>]
// Flags appended to every run: --singleThreaded (native) / nothing (wasm runs single-threaded anyway).

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const median = (v) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];

if (process.argv[2] === "--child") {
    // --child <package dir> <module> <case json> <calls>
    const [pkg, module, caseJson, calls] = process.argv.slice(3);
    process.env.TSRS_WASM = module;
    const { tsc } = await import(path.join(pkg, "node.js"));
    const c = JSON.parse(caseJson);
    const times = [];
    let r;
    for (let i = 0; i < Number(calls); i++) {
        const start = performance.now();
        r = await tsc(c.args, { cwd: c.cwd });
        times.push(performance.now() - start);
    }
    process.stdout.write(JSON.stringify({ times, exitCode: r.exitCode, memoryBytes: r.memoryBytes ?? 0, maxRssKiB: process.resourceUsage().maxRSS }));
    process.exit(0);
}

const o = { wasm: [], projects: [], runs: 5 };
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--native") o.native = path.resolve(argv[++i]);
    else if (a === "--wasm") o.wasm.push(argv[++i]);
    else if (a === "--ts-rust") o.tsRust = argv[++i];
    else if (a === "--project") o.projects.push(argv[++i]);
    else if (a === "--runs") o.runs = Number(argv[++i]);
    else if (a === "--out") o.out = path.resolve(argv[++i]);
    else throw new Error(`unknown argument ${a}`);
}

const engines = [];
for (const w of o.wasm) {
    const [name, module] = w.split("=");
    engines.push({ name, pkg: path.join(here, "../../npm/tsrs-wasm"), module: path.resolve(module) });
}
let tsRustPkg;
if (o.tsRust) {
    const [repo, module] = o.tsRust.split("=");
    tsRustPkg = fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-bench-tsrust-"));
    for (const f of fs.readdirSync(path.join(repo, "npm/wasm"))) if (f.endsWith(".js")) fs.copyFileSync(path.join(repo, "npm/wasm", f), path.join(tsRustPkg, f));
    fs.copyFileSync(module, path.join(tsRustPkg, "ts_rust.wasm"));
    engines.push({ name: "ts-rust", pkg: tsRustPkg, module: path.resolve(module) });
}
const projects = o.projects.map((p) => {
    const eq = p.indexOf("=");
    const colon = p.indexOf(":", eq);
    return { name: p.slice(0, eq), cwd: path.resolve(p.slice(eq + 1, colon)), args: p.slice(colon + 1).split(" ").filter(Boolean) };
});

function native(c) {
    const r = spawnSync("/usr/bin/time", ["-l", o.native, ...c.args, "--singleThreaded"], { cwd: c.cwd, env: { TZ: "UTC" }, maxBuffer: 1 << 30 });
    const err = r.stderr.toString();
    const num = (re) => Number((err.match(re) ?? [])[1] ?? NaN);
    return { wall: num(/([\d.]+) real/) * 1000, rss: num(/(\d+)\s+maximum resident set size/), instructions: num(/(\d+)\s+instructions retired/), exitCode: r.status };
}

function wasmRun(e, c, calls) {
    const out = execFileSync(process.execPath, [fileURLToPath(import.meta.url), "--child", e.pkg, e.module, JSON.stringify(c), String(calls)], { maxBuffer: 1 << 28 });
    return JSON.parse(out.toString());
}

const results = {};
for (const c of projects) {
    const r = (results[c.name] = { native: [], engines: Object.fromEntries(engines.map((e) => [e.name, { cold: [], warm: [] }])) });
    for (let rep = 0; rep < o.runs; rep++) {
        r.native.push(native(c));
        for (const e of engines) {
            const res = wasmRun(e, c, rep === 0 ? 1 + o.runs : 1);
            const s = r.engines[e.name];
            s.cold.push(res.times[0]);
            if (rep === 0) {
                s.warm = res.times.slice(1);
                s.maxRssKiB = res.maxRssKiB;
                s.memoryBytes = res.memoryBytes;
                s.exitCode = res.exitCode;
            }
        }
        process.stderr.write(".");
    }
}
process.stderr.write("\n");
if (tsRustPkg) fs.rmSync(tsRustPkg, { recursive: true, force: true });

const s = (ms) => (ms / 1000).toFixed(2);
console.log(`load ${os.loadavg().map((x) => x.toFixed(1)).join(" ")}; ${o.runs} interleaved runs; times in seconds (median)`);
for (const c of projects) {
    const r = results[c.name];
    const nwall = median(r.native.map((x) => x.wall));
    const line = [`${c.name.padEnd(16)} native ${s(nwall)} (rss ${(median(r.native.map((x) => x.rss)) / 2 ** 20).toFixed(0)} MiB, ${(median(r.native.map((x) => x.instructions)) / 1e9).toFixed(2)}e9 instr)`];
    for (const e of engines) {
        const x = r.engines[e.name];
        const warm = median(x.warm);
        line.push(`${e.name}: cold ${s(median(x.cold))} warm ${s(warm)} (${(warm / nwall).toFixed(2)}x) linear ${(x.memoryBytes / 2 ** 20).toFixed(0)} MiB maxRSS ${(x.maxRssKiB / 1024).toFixed(0)} MiB exit ${x.exitCode}`);
    }
    console.log(line.join("\n    "));
    r.summary = { nativeWallMs: nwall, engines: Object.fromEntries(engines.map((e) => [e.name, { coldMs: median(r.engines[e.name].cold), warmMs: median(r.engines[e.name].warm) }])) };
}
for (const e of engines) {
    const ratios = projects.map((c) => results[c.name].summary.engines[e.name].warmMs);
    console.log(`${e.name}: geomean warm ${s(Math.exp(ratios.reduce((a, b) => a + Math.log(b), 0) / ratios.length))}`);
}
if (o.out) fs.writeFileSync(o.out, JSON.stringify({ load: os.loadavg(), runs: o.runs, results }, null, 2));
