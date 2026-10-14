#!/usr/bin/env node
// Differential gate for the WebAssembly build: runs native tsrs (`--singleThreaded`) and tsrs.wasm (through
// npm/tsrs-wasm's runner, in a worker with a large stack) on the same inputs at the same absolute paths, and compares
// exit codes, stdout bytes and the files each side wrote (sha256; symlinks as `-> target`). stderr is compared when
// both exit 0. Normalised: only the build-status time prefixes.
//
//   node tools/wasm/diff.mjs --native <tsrs> --module <tsrs.wasm> --out <dir>
//        (--cases <materialized dir> | --regressions <testdata/regressions> | --fixtures <tools/wasm/fixtures>
//         | --project <name>=<cwd>:<args...> ...)
//        [--fs node|memory|node,memory] [--jobs N] [--timeout 120] [--repeat N] [--stack-census] [--worker-stack MB]
//        [--shard I/N]
//
// With several file systems (`--fs node,memory`), native runs once per case and the module once per file system,
// each compared with that native run; each file system's results go to <out>/<fs>/. `--shard I/N` takes every Nth
// case from the I-th (0-based), for splitting a run over machines. --jobs defaults to the number of CPUs.
//
// Cases (`tsrs-test materialize`, fixtures) are `<case>/{root/, case.json}`; case.json has cwd (relative to root),
// args, or steps [{args, write: {path: text}, remove: [path], chmod: {path: mode}}], optionally generate (see
// `generate`) and nativeFlag (default "--singleThreaded"). `${ROOT}` in an argument is the slot's absolute path. Each case is copied fresh into a work
// slot in a temporary directory outside the repo (resolved to its real path, so both sides print the same paths) before each side.
// Projects run in place and must not write: the harness checks `git status --short` before and after.
// Exit 1 on any differ, module-crash or timeout. <out>/summary.json has the counts; <out>/fail/<id>/ the evidence.

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Worker } from "node:worker_threads";
import { isCaseInsensitive } from "../../npm/tsrs-wasm/node-fs.js";

const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../..");

function parseArgs(argv) {
    const o = { projects: [], jobs: os.availableParallelism(), timeout: 120, repeat: 1, fs: ["node"], workerStack: 256, shard: [0, 1] };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        const next = () => argv[++i];
        switch (a) {
            case "--native": o.native = path.resolve(next()); break;
            case "--module": o.module = path.resolve(next()); break;
            case "--out": o.out = path.resolve(next()); break;
            case "--cases": o.cases = path.resolve(next()); break;
            case "--regressions": o.regressions = path.resolve(next()); break;
            case "--fixtures": o.fixtures = path.resolve(next()); break;
            case "--project": o.projects.push(next()); break;
            case "--fs": o.fs = next().split(","); break;
            case "--shard": o.shard = next().split("/").map(Number); break;
            case "--jobs": o.jobs = Number(next()); break;
            case "--timeout": o.timeout = Number(next()); break;
            case "--repeat": o.repeat = Number(next()); break;
            case "--stack-census": o.census = true; break;
            case "--worker-stack": o.workerStack = Number(next()); break;
            default: throw new Error(`unknown argument ${a}`);
        }
    }
    if (!o.native || !o.module || !o.out) throw new Error("--native, --module and --out are required");
    if (!o.fs.every((f) => f === "node" || f === "memory")) throw new Error("--fs takes node, memory or both");
    if (!(o.shard[1] >= 1 && o.shard[0] >= 0 && o.shard[0] < o.shard[1])) throw new Error("--shard is I/N with 0 <= I < N");
    return o;
}

// The initial value of the module's stack pointer: the first mutable i32 global's i32.const initializer.
function stackPointerInit(bytes) {
    let i = 8;
    const leb = () => { let r = 0, s = 0, b; do { b = bytes[i++]; r |= (b & 127) << s; s += 7; } while (b & 128); return r >>> 0; };
    const sleb = () => { let r = 0, s = 0, b; do { b = bytes[i++]; r |= (b & 127) << s; s += 7; } while (b & 128); if (s < 32 && (b & 64)) r |= -1 << s; return r; };
    while (i < bytes.length) {
        const id = bytes[i++];
        const size = leb();
        const end = i + size;
        if (id === 6) {
            const count = leb();
            for (let g = 0; g < count; g++) {
                const type = bytes[i++];
                const mut = bytes[i++];
                if (bytes[i] === 0x41) {
                    i++;
                    const v = sleb();
                    i++; // end
                    if (type === 0x7f && mut === 1) return v;
                } else {
                    while (bytes[i++] !== 0x0b);
                }
            }
            return 0;
        }
        i = end;
    }
    return 0;
}

function listCases(o) {
    const cases = [];
    const fromDir = (dir) => {
        for (const name of fs.readdirSync(dir).sort()) {
            const c = path.join(dir, name);
            if (!fs.existsSync(path.join(c, "case.json"))) continue;
            const spec = JSON.parse(fs.readFileSync(path.join(c, "case.json"), "utf8"));
            cases.push({ id: spec.id ?? name, root: path.join(c, "root"), spec });
        }
    };
    if (o.cases) fromDir(o.cases);
    if (o.fixtures) fromDir(o.fixtures);
    if (o.regressions) {
        for (const name of fs.readdirSync(o.regressions).sort()) {
            const root = path.join(o.regressions, name);
            if (!fs.statSync(root).isDirectory()) continue;
            cases.push({ id: `regressions/${name}`, root, spec: { cwd: ".", args: ["-p", ".", "--pretty", "false"], exclude: ["expected.txt"] } });
        }
    }
    for (const p of o.projects) {
        const eq = p.indexOf("=");
        const colon = p.indexOf(":", eq);
        const name = p.slice(0, eq);
        const cwd = path.resolve(p.slice(eq + 1, colon));
        const args = p.slice(colon + 1).split(" ").filter(Boolean);
        cases.push({ id: `project/${name}`, project: cwd, spec: { args: args.filter((a) => a !== "--singleThreaded") } });
    }
    return cases;
}

function walk(dir, base = dir, out = new Map()) {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        const rel = path.relative(base, p);
        if (e.isSymbolicLink()) out.set(rel, "-> " + fs.readlinkSync(p));
        else if (e.isDirectory()) walk(p, base, out);
        else out.set(rel, crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex"));
    }
    return out;
}

function hasSymlink(dir) {
    for (const v of walk(dir).values()) if (v.startsWith("-> ")) return true;
    return false;
}

function changes(before, after) {
    const out = [];
    for (const [k, v] of after) if (before.get(k) !== v) out.push(`${k} ${v}`);
    for (const k of before.keys()) if (!after.has(k)) out.push(`${k} (removed)`);
    return out.sort();
}

// The slot as a fresh copy of `src` (rsync -a --delete): modes, modification times and symlinks kept. In process:
// spawning rsync twice per case cost more than the native run.
function restore(src, slot) {
    makeWritable(slot);
    fs.rmSync(slot, { recursive: true, force: true });
    fs.cpSync(src, slot, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
}

// A case can make its files or directories read-only (`chmod` steps): give the owner write access back before the
// slot is removed.
function makeWritable(dir) {
    let entries;
    try {
        entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
        return;
    }
    fs.chmodSync(dir, fs.statSync(dir).mode | 0o700);
    for (const e of entries) {
        if (e.isDirectory()) makeWritable(path.join(dir, e.name));
    }
}

// Build status times: `[hh:mm:ss AM]` (pretty) or `hh:mm:ss AM - ` at a line start (not pretty).
const normalize = (s) => s.replace(/\[\d\d:\d\d:\d\d [AP]M\]/g, "[TIME]").replace(/^\d\d:\d\d:\d\d [AP]M - /gm, "TIME - ");

function steps(spec) {
    return spec.steps ?? [{ args: spec.args }];
}

// case.json `generate`: { path: { prefix, repeat, count, suffix, close } } writes prefix + repeat x count + suffix +
// close x count (deep-recursion inputs too large to keep in the repository).
function generate(slot, spec) {
    for (const [rel, g] of Object.entries(spec.generate ?? {})) {
        const close = g.close ?? "";
        fs.writeFileSync(path.join(slot, rel), g.prefix + g.repeat.repeat(g.count) + g.suffix + close.repeat(g.count) + (close ? ";\n" : ""));
    }
}

function applyEdits(slot, step) {
    for (const [rel, mode] of Object.entries(step.chmod ?? {})) fs.chmodSync(path.join(slot, rel), parseInt(mode, 8));
    for (const [rel, text] of Object.entries(step.write ?? {})) {
        fs.mkdirSync(path.dirname(path.join(slot, rel)), { recursive: true });
        fs.writeFileSync(path.join(slot, rel), text);
    }
    for (const rel of step.remove ?? []) fs.rmSync(path.join(slot, rel), { recursive: true, force: true });
}

const subst = (args, slot) => args.map((a) => a.replaceAll("${ROOT}", slot));

function runNative(o, c, slot, cwd, args) {
    const flag = c.spec.nativeFlag ?? "--singleThreaded";
    const r = spawnSync(o.native, [...args, ...flag.split(" ").filter(Boolean)], { cwd, env: { TZ: "UTC" }, timeout: o.timeout * 1000, maxBuffer: 1 << 30 });
    if (r.error?.code === "ETIMEDOUT") return { timeout: true };
    return { exitCode: r.status ?? (r.signal ? 128 : -1), signal: r.signal, stdout: r.stdout.toString("utf8"), stderr: r.stderr.toString("utf8") };
}

function memoryFiles(slot) {
    const files = {};
    for (const [rel, v] of walk(slot)) {
        if (!v.startsWith("-> ")) files[path.join(slot, rel)] = fs.readFileSync(path.join(slot, rel));
    }
    return files;
}

// One worker per slot, reused across runs (a run instantiates the module afresh; starting a worker with its large
// stack cost ~25 ms a run). A timeout or a crash ends the worker; the slot starts a new one for its next run.
const workers = new Map();

function slotWorker(o, moduleObj, slot) {
    let w = workers.get(slot);
    if (!w) {
        w = new Worker(new URL("./diff-worker.mjs", import.meta.url), {
            workerData: { module: moduleObj },
            resourceLimits: { stackSizeMb: o.workerStack },
            stdout: true,
            stderr: true,
        });
        // The worker's own output is not the run's (that comes back in the message); drain it.
        w.stdout.resume();
        w.stderr.resume();
        workers.set(slot, w);
    }
    return w;
}

function dropWorker(slot) {
    const w = workers.get(slot);
    workers.delete(slot);
    if (w) w.terminate();
}

function runModule(o, moduleObj, spInit, slot, cwd, args, files) {
    return new Promise((resolve) => {
        const worker = slotWorker(o, moduleObj, slot);
        let done = false;
        const finish = (r, drop) => {
            if (done) return;
            done = true;
            clearTimeout(timer);
            worker.off("message", onMessage);
            worker.off("error", onError);
            worker.off("exit", onExit);
            if (drop) dropWorker(slot);
            resolve(r);
        };
        const onMessage = (r) => finish(r, false);
        const onError = (e) => finish({ crash: String(e?.stack ?? e) }, true);
        const onExit = (code) => finish({ crash: `worker exited with ${code}` }, true);
        const timer = setTimeout(() => finish({ timeout: true }, true), o.timeout * 1000);
        worker.on("message", onMessage);
        worker.once("error", onError);
        worker.once("exit", onExit);
        worker.postMessage({ options: { args, cwd, env: {}, files, caseInsensitive: files ? isCaseInsensitive() : undefined }, census: o.census ? spInit : 0 });
    });
}

function isPanic(r) {
    return r.exitCode === 5 && /panicked at/.test(r.stderr ?? "");
}

// One side of a case: native, or the module on one file system. Its results and the files it wrote.
async function runSide(o, moduleObj, spInit, c, slot, side, memory) {
    if (!c.project) {
        restore(c.root, slot);
        generate(slot, c.spec);
    }
    const before = c.project ? null : walk(slot);
    const results = [];
    for (const step of steps(c.spec)) {
        if (!c.project) applyEdits(slot, step);
        const cwd = c.project ?? path.join(slot, c.spec.cwd ?? ".");
        const args = subst(step.args, slot);
        let r;
        if (side === "native") {
            r = runNative(o, c, slot, cwd, args);
        } else {
            r = await runModule(o, moduleObj, spInit, slot, cwd, args, memory && !c.project ? memoryFiles(slot) : undefined);
            if (memory && r.files) {
                for (const [p, text] of Object.entries(r.files)) {
                    fs.mkdirSync(path.dirname(p), { recursive: true });
                    fs.writeFileSync(p, text);
                }
            }
        }
        results.push(r);
        if (r.timeout || r.crash) break;
    }
    const written = c.project ? [] : changes(before, walk(slot)).filter((l) => !(c.spec.exclude ?? []).some((x) => l.startsWith(x + " ")));
    return { results, written };
}

// The case on every file system of `o.fs`: one result per file system.
async function runCase(o, moduleObj, spInit, c, slot) {
    const out = {};
    if (c.spec.skipped) {
        for (const f of o.fs) out[f] = { id: c.id, status: "skipped", reason: c.spec.skipped };
        return out;
    }
    let native;
    for (const f of o.fs) {
        const memory = f === "memory";
        if (memory && !c.project && hasSymlink(c.root)) {
            out[f] = { id: c.id, status: "skipped", reason: "symlink (memory fs)" };
            continue;
        }
        native ??= await runSide(o, moduleObj, spInit, c, slot, "native", false);
        const module = await runSide(o, moduleObj, spInit, c, slot, "module", memory);
        out[f] = compare(o, c, slot, native, module, f);
    }
    return out;
}

function compare(o, c, slot, native, module, f) {
    const n = native.results;
    const m = module.results;
    const last = (rs) => rs[rs.length - 1];
    let status = "same";
    let detail = "";
    if (m.some((r) => r.timeout) || n.some((r) => r.timeout)) status = "timeout";
    else if (n.some((r) => r.signal || isPanic(r))) status = m.some((r) => r.crash || isPanic(r)) ? "both-crash" : "native-crash";
    else if (m.some((r) => r.crash || isPanic(r))) {
        status = "module-crash";
        detail = last(m).crash ?? last(m).stderr;
    } else {
        for (let i = 0; i < n.length; i++) {
            const a = n[i];
            const b = m[i];
            if (!b || a.exitCode !== b.exitCode) { status = "differ"; detail = `step ${i}: exit ${a.exitCode} vs ${b?.exitCode}`; break; }
            if (normalize(a.stdout) !== normalize(b.stdout)) { status = "differ"; detail = `step ${i}: stdout`; break; }
            if (a.exitCode === 0 && a.stderr !== b.stderr) { status = "differ"; detail = `step ${i}: stderr`; break; }
        }
        if (status === "same" && native.written.join("\n") !== module.written.join("\n")) {
            status = "differ";
            detail = "written files";
        }
    }
    const stack = m.map((r) => r.stackBytes ?? 0).reduce((a, b) => Math.max(a, b), 0);
    const result = { id: c.id, status, detail, stackBytes: stack, memoryBytes: last(m)?.memoryBytes ?? 0, written: native.written.length };
    if (status !== "same" && status !== "both-crash") {
        const dir = path.join(outDir(o, f), "fail", c.id.replace(/[^A-Za-z0-9._-]/g, "_"));
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(path.join(dir, "case.json"), JSON.stringify(c.spec, null, 2));
        n.forEach((r, i) => { fs.writeFileSync(path.join(dir, `native.${i}.stdout`), r.stdout ?? ""); fs.writeFileSync(path.join(dir, `native.${i}.stderr`), r.stderr ?? ""); });
        m.forEach((r, i) => { fs.writeFileSync(path.join(dir, `module.${i}.stdout`), r.stdout ?? ""); fs.writeFileSync(path.join(dir, `module.${i}.stderr`), (r.stderr ?? "") + (r.crash ?? "")); });
        fs.writeFileSync(path.join(dir, "native.written"), native.written.join("\n"));
        fs.writeFileSync(path.join(dir, "module.written"), module.written.join("\n"));
        fs.writeFileSync(path.join(dir, "repro.sh"), `# ${c.id}: ${status} ${detail}\n# case root: ${c.root ?? c.project}\nmkdir -p ${slot}\nrsync -a --delete ${c.root ?? ""}/ ${slot}/ && cd ${path.join(slot, c.spec.cwd ?? ".")}\n${o.native} ${(c.spec.args ?? steps(c.spec)[0].args).join(" ")} --singleThreaded\nnode ${path.join(repo, "npm/tsrs-wasm/bin/tsrs-wasm.js")} ${(c.spec.args ?? steps(c.spec)[0].args).join(" ")}${f === "memory" ? " (in-memory files)" : ""}\n`);
        if (c.project) cleanSlot(slot);
    }
    return result;
}

// Where a file system's results go: <out> for a single file system, <out>/<fs> for several.
function outDir(o, f) {
    return o.fs.length > 1 ? path.join(o.out, f) : o.out;
}

function cleanSlot(slot) {
    makeWritable(slot);
    fs.rmSync(slot, { recursive: true, force: true });
}

function gitStatus(dir) {
    const r = spawnSync("git", ["-C", dir, "status", "--short"]);
    return r.status === 0 ? r.stdout.toString().split("\n").filter(Boolean).length : -1;
}

function summarize(o, f, results, spInit, statusBefore) {
    const counts = {};
    for (const r of results) counts[r.status] = (counts[r.status] ?? 0) + 1;
    const stacks = results.map((r) => r.stackBytes).filter((x) => x > 0).sort((a, b) => a - b);
    const summary = {
        total: results.length,
        counts,
        fs: f,
        shard: o.shard[1] > 1 ? `${o.shard[0]}/${o.shard[1]}` : undefined,
        repeat: o.repeat,
        workerStackMb: o.workerStack,
        stackPointerInit: spInit,
        stack: stacks.length ? { max: stacks[stacks.length - 1], p999: stacks[Math.min(stacks.length - 1, Math.floor(stacks.length * 0.999))], median: stacks[Math.floor(stacks.length / 2)] } : undefined,
        maxMemoryBytes: Math.max(0, ...results.map((r) => r.memoryBytes ?? 0)),
        writtenFiles: results.reduce((n, r) => n + (r.written ?? 0), 0),
        casesWithWrites: results.filter((r) => r.written > 0).length,
        projects: Object.fromEntries([...statusBefore].map(([dir, before]) => [dir, { gitStatusBefore: before, gitStatusAfter: gitStatus(dir) }])),
        failures: results.filter((r) => !["same", "skipped", "both-crash"].includes(r.status)).map((r) => ({ id: r.id, status: r.status, detail: (r.detail ?? "").slice(0, 300) })),
    };
    fs.mkdirSync(outDir(o, f), { recursive: true });
    fs.writeFileSync(path.join(outDir(o, f), "summary.json"), JSON.stringify(summary, null, 2));
    const unchanged = Object.values(summary.projects).every((p) => p.gitStatusBefore === p.gitStatusAfter);
    const label = o.fs.length > 1 ? `${f}: ` : "";
    console.log(`${label}${results.length} cases: ${Object.entries(counts).map(([k, v]) => `${v} ${k}`).join(", ")}${summary.stack ? `; shadow stack max ${summary.stack.max} B, p99.9 ${summary.stack.p999} B` : ""}${Object.keys(summary.projects).length ? `; checkouts ${unchanged ? "unchanged" : "MODIFIED"}` : ""}`);
    const bad = (counts.differ ?? 0) + (counts["module-crash"] ?? 0) + (counts.timeout ?? 0);
    return bad === 0 && unchanged;
}

async function main() {
    const o = parseArgs(process.argv.slice(2));
    fs.rmSync(o.out, { recursive: true, force: true });
    fs.mkdirSync(o.out, { recursive: true });
    const bytes = fs.readFileSync(o.module);
    const moduleObj = new WebAssembly.Module(bytes);
    const spInit = stackPointerInit(bytes);
    const cases = listCases(o).filter((_, i) => i % o.shard[1] === o.shard[0]);
    const statusBefore = new Map(cases.filter((c) => c.project).map((c) => [c.project, gitStatus(c.project)]));
    const results = Object.fromEntries(o.fs.map((f) => [f, []]));
    // The native compiler must not see the repo's package.json or node_modules: memoryFiles only copies the slot.
    const slots = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-wasm-diff-")));
    try {
        for (let rep = 0; rep < o.repeat; rep++) {
            let next = 0;
            const jobs = Math.max(1, Math.min(o.jobs, cases.length));
            await Promise.all(Array.from({ length: jobs }, async (_, slotIndex) => {
                const slot = path.join(slots, String(slotIndex).padStart(2, "0"));
                while (next < cases.length) {
                    const c = cases[next++];
                    const byFs = await runCase(o, moduleObj, spInit, c, slot);
                    for (const f of o.fs) {
                        const r = byFs[f];
                        if (rep === 0) results[f].push(r);
                        else if (r.status !== results[f].find((x) => x.id === r.id)?.status) r.status !== "same" && results[f].push({ ...r, id: `${r.id} (repeat ${rep})` });
                        const tag = o.fs.length > 1 ? ` (${f})` : "";
                        process.stderr.write(r.status === "same" || r.status === "skipped" ? "." : `\n${r.status} ${r.id}${tag} ${r.detail ?? ""}\n`);
                    }
                }
            }));
        }
    } finally {
        for (const slot of [...workers.keys()]) dropWorker(slot);
        cleanSlot(slots);
    }
    process.stderr.write("\n");
    let ok = true;
    for (const f of o.fs) ok = summarize(o, f, results[f], spInit, statusBefore) && ok;
    process.exit(ok ? 0 : 1);
}

await main();
