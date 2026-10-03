#!/usr/bin/env node
// Runs the pinned upstream API test suites (ts-ref/packages/typescript/test/{sync,async}) and the harness's own
// parity tests (tools/node-api/tests) with the *pinned upstream client* against a chosen `--api` server binary:
// the Go oracle built from ts-ref, or a tsrs build. Nothing in the client is replaced or patched.
//
//   node tools/node-api/run-upstream.mjs --binary <exe> --label <name> [--suite upstream|parity|all]
//        [--no-trace] [--filter <substring>]... [--timeout-min 30] [--concurrency N]
//
// Output goes to tools/node-api/.work/<label>/: results.jsonl (one line per test, from reporter.mjs), run.tap,
// trace/*.jsonl (per-process frame traces from proxy.mjs), summary.json (real counts) and meta.json.
// The exit code is 0 only if node --test ran and every test passed; inspect summary.json either way.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");

function usage(msg) {
    if (msg) console.error(`error: ${msg}`);
    console.error("usage: run-upstream.mjs --binary <exe> --label <name> [--suite upstream|parity|all] [--no-trace] [--record] [--filter <s>]... [--timeout-min N] [--concurrency N] [--ref <ts-ref>]");
    process.exit(2);
}

const opts = { suite: "all", trace: true, filters: [], timeoutMin: 30, ref: path.join(repoRoot, "ts-ref") };
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const v = () => argv[++i] ?? usage(`${a} needs a value`);
    if (a === "--binary") opts.binary = path.resolve(v());
    else if (a === "--label") opts.label = v();
    else if (a === "--suite") opts.suite = v();
    else if (a === "--no-trace") opts.trace = false;
    else if (a === "--record") opts.record = true;
    else if (a === "--filter") opts.filters.push(v());
    else if (a === "--timeout-min") opts.timeoutMin = Number(v());
    else if (a === "--concurrency") opts.concurrency = Number(v());
    else if (a === "--ref") opts.ref = path.resolve(v());
    else usage(`unknown argument ${a}`);
}
if (!opts.binary || !opts.label) usage("--binary and --label are required");
if (!/^[\w.-]+$/.test(opts.label)) usage("label must be [A-Za-z0-9_.-]+");
if (!fs.existsSync(opts.binary)) usage(`binary not found: ${opts.binary}`);
if (!["upstream", "parity", "all"].includes(opts.suite)) usage(`bad --suite ${opts.suite}`);
// Goldens are recorded from the Go oracle built by setup.sh, never from a candidate.
const oracleExe = path.join(here, ".work", "oracle", process.platform === "win32" ? "tsc.exe" : "tsc");
const isOracle = fs.existsSync(oracleExe) && fs.realpathSync(oracleExe) === fs.realpathSync(opts.binary);
if (opts.record && !isOracle) usage(`--record only accepts the Go oracle at ${oracleExe}`);

// ── pinned reference check ──────────────────────────────────────────
const pinned = /\[workspace\.metadata\.typescript\][^[]*?commit\s*=\s*"([0-9a-f]{40})"/s.exec(fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8"))?.[1];
const refHead = spawnSync("git", ["-C", opts.ref, "rev-parse", "HEAD"], { encoding: "utf8" }).stdout.trim();
if (!pinned || refHead !== pinned) {
    console.error(`ts-ref HEAD ${refHead || "(missing)"} does not match Cargo.toml commit ${pinned}; run tools/node-api/setup.sh`);
    process.exit(2);
}
const refPkg = path.join(opts.ref, "packages", "typescript");
if (!fs.existsSync(path.join(opts.ref, "node_modules", "tinybench"))) {
    console.error("ts-ref node_modules missing; run tools/node-api/setup.sh");
    process.exit(2);
}

// ── private work tree: <out>/tree/{packages/typescript,built/local/tsc,node_modules} ──
const out = path.join(here, ".work", opts.label);
fs.rmSync(out, { recursive: true, force: true });
const tree = path.join(out, "tree");
const pkg = path.join(tree, "packages", "typescript");
fs.mkdirSync(pkg, { recursive: true });
for (const entry of ["package.json", "src", "lib", "vendor", "test", "tsconfig.base.json", "tsconfig.json"]) {
    const src = path.join(refPkg, entry);
    if (fs.existsSync(src)) fs.cpSync(src, path.join(pkg, entry), { recursive: true });
}
fs.cpSync(path.join(here, "tests"), path.join(pkg, "test", "parity"), { recursive: true, filter: src => !src.includes(`${path.sep}golden`) });
// node_modules: everything ts-ref installed except the workspace self-link, so the client resolves to this copy.
const nm = path.join(tree, "node_modules");
fs.mkdirSync(nm);
for (const entry of fs.readdirSync(path.join(opts.ref, "node_modules"))) {
    if (entry === "@typescript" || entry.startsWith(".")) continue;
    fs.symlinkSync(path.join(opts.ref, "node_modules", entry), path.join(nm, entry));
}
// The upstream client spawns built/local/tsc when run from source (lib/getExePath.js).
const local = path.join(tree, "built", "local");
fs.mkdirSync(local, { recursive: true });
const traceDir = path.join(out, "trace");
fs.mkdirSync(traceDir);
const exe = path.join(local, process.platform === "win32" ? "tsc.exe" : "tsc");
if (opts.trace) {
    if (process.platform !== "linux") usage("--trace needs Linux (/proc); pass --no-trace");
    fs.writeFileSync(exe, `#!/bin/sh\nexec "${process.execPath}" "${path.join(here, "proxy.mjs")}" "${opts.binary}" "$@"\n`, { mode: 0o755 });
}
else if (process.platform === "win32") fs.copyFileSync(opts.binary, exe);
else fs.symlinkSync(opts.binary, exe);

// ── select test files ───────────────────────────────────────────────
const files = [];
const dirs = { upstream: ["test/sync", "test/async"], parity: ["test/parity"], all: ["test/sync", "test/async", "test/parity"] }[opts.suite];
for (const d of dirs) {
    for (const f of fs.readdirSync(path.join(pkg, d)).sort()) {
        if (!f.endsWith(".test.ts")) continue;
        const rel = `${d}/${f}`;
        if (opts.filters.length && !opts.filters.some(s => rel.includes(s))) continue;
        files.push(rel);
    }
}
if (files.length === 0) usage("no test files selected");

const nodeArgs = [
    "--conditions", "@typescript/source",
    "--import", pathToFileURL(path.join(here, "preload.mjs")).href,
    "--test",
    "--test-reporter", path.join(here, "reporter.mjs"), "--test-reporter-destination", path.join(out, "results.jsonl"),
    "--test-reporter", "tap", "--test-reporter-destination", path.join(out, "run.tap"),
];
if (opts.concurrency) nodeArgs.push(`--test-concurrency=${opts.concurrency}`);
nodeArgs.push(...files);

const started = Date.now();
const res = spawnSync(process.execPath, nodeArgs, {
    cwd: pkg,
    stdio: ["ignore", "inherit", "inherit"],
    env: {
        ...process.env,
        NODE_API_TRACE_DIR: opts.trace ? traceDir : "",
        NODE_API_GOLDEN_DIR: path.join(here, "tests", "golden"),
        NODE_API_RECORD: opts.record ? "1" : "",
        NODE_API_ORACLE: isOracle ? "1" : "",
        NODE_API_BINARY: opts.binary,
        NODE_API_EVIDENCE_DIR: path.join(out, "evidence"),
    },
    timeout: opts.timeoutMin * 60_000,
    killSignal: "SIGKILL",
});

// ── summarize from the JSONL results (never from the exit code alone) ──
const results = fs.existsSync(path.join(out, "results.jsonl"))
    ? fs.readFileSync(path.join(out, "results.jsonl"), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l))
    : [];
const leaves = results.filter(r => r.kind === "test");
const summary = {
    label: opts.label,
    binary: opts.binary,
    tsRef: refHead,
    suite: opts.suite,
    files: files.length,
    tests: leaves.length,
    pass: leaves.filter(r => r.ok && !r.skip && !r.todo).length,
    fail: leaves.filter(r => !r.ok && !r.todo).length,
    skip: leaves.filter(r => r.skip).length,
    todo: leaves.filter(r => r.todo).length,
    // todo tests document known upstream defects; list whether each still fails rather than hiding it.
    todoTests: leaves.filter(r => r.todo).map(r => `${r.ok ? "passing" : "failing"}: ${r.path}`),
    // A file whose process crashed shows up as a failed nesting-0 entry named after the file.
    softMismatches: fs.existsSync(path.join(out, "run.tap")) ? (fs.readFileSync(path.join(out, "run.tap"), "utf8").match(/^\s*# soft-mismatch .*/gm) ?? []).map(l => l.trim().slice(2, 300)) : [],
    crashedFiles: results.filter(r => !r.ok && files.includes(r.path)).map(r => r.path),
    nodeExit: res.status,
    nodeSignal: res.signal,
    timedOut: res.error?.code === "ETIMEDOUT",
    seconds: Math.round((Date.now() - started) / 1000),
};
fs.writeFileSync(path.join(out, "summary.json"), JSON.stringify(summary, null, 2) + "\n");
const binHead = spawnSync(opts.binary, ["--version"], { encoding: "utf8", timeout: 10_000 });
fs.writeFileSync(path.join(out, "meta.json"), JSON.stringify({ argv: process.argv.slice(2), node: process.version, platform: `${process.platform}-${process.arch}`, binaryVersion: (binHead.stdout ?? "").trim() }, null, 2) + "\n");
console.log(JSON.stringify(summary, null, 2));
process.exit(summary.fail === 0 && summary.tests > 0 && res.status === 0 ? 0 : 1);
