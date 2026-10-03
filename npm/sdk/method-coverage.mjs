#!/usr/bin/env node
// Per-method inventory of the TypeScript 7 API protocol (the Method constants in the pinned tsc/internal/api/proto.go)
// against the vendored JS SDK and one or more API servers.
//
//   node npm/sdk/method-coverage.mjs --server tsgo=/path/to/pinned/tsgo [--server tsrs=target/release/tsrs] \
//       [--ts-ref ts-ref] [--out npm/sdk/METHODS.md]
//
// For every method it reports whether the sync/async SDK sends it, and, per server, how often the upstream SDK test
// suite (npm/tsrs/test, run from source) got a successful response or an error for it. Every run's pass/fail counts
// are recorded too. A method with successful responses is only "exercised", not "supported": tests may still fail
// on its results, and error responses include the tests' intentional error cases. Server-side support status for
// tsrs is tracked by the Rust server's own inventory; this file is the client/test view.

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const sdkDir = path.dirname(fileURLToPath(import.meta.url));
const npmDir = path.resolve(sdkDir, "..");
const repoRoot = path.resolve(npmDir, "..");
const pkgDir = path.join(npmDir, "tsrs");

function parseArgs(argv) {
    const opts = { servers: [], tsRef: path.join(repoRoot, "ts-ref"), out: path.join(sdkDir, "METHODS.md") };
    for (let i = 0; i < argv.length; i++) {
        const arg = argv[i];
        const value = () => argv[++i] ?? fail(`${arg} needs a value`);
        if (arg === "--server") {
            const [label, exe] = value().split(/=(.*)/s);
            if (!label || !exe) fail("--server needs <label>=<path>");
            opts.servers.push({ label, exe: path.resolve(exe) });
        }
        else if (arg === "--ts-ref") opts.tsRef = path.resolve(value());
        else if (arg === "--out") opts.out = path.resolve(value());
        else fail(`unknown argument ${arg}`);
    }
    return opts;
}

function fail(message) {
    console.error(`error: ${message}`);
    process.exit(2);
}

function pinnedCommit() {
    const text = fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8");
    return text.split(/^\[workspace\.metadata\.typescript\]$/m)[1]?.match(/^commit\s*=\s*"([0-9a-f]{40})"/m)?.[1];
}

function protoMethods(tsRef, commit) {
    const text = execFileSync("git", ["-C", tsRef, "show", `${commit}:tsc/internal/api/proto.go`], { encoding: "utf8", maxBuffer: 1 << 26 });
    const methods = [...text.matchAll(/^\s*(Method\w+)\s+Method\s*=\s*"([^"]+)"/gm)].map(m => ({ constant: m[1], name: m[2] }));
    if (methods.length === 0) throw new Error("no Method constants found in proto.go");
    return methods;
}

function listTs(dir) {
    return fs.readdirSync(dir, { withFileTypes: true, recursive: true })
        .filter(e => e.isFile() && e.name.endsWith(".ts"))
        .map(e => path.join(e.parentPath ?? e.path, e.name));
}

// Methods a client source tree names as a string literal (the generated method table is excluded).
function clientMethods(files, names) {
    const text = files.filter(f => !f.endsWith("proto.generated.ts")).map(f => fs.readFileSync(f, "utf8")).join("\n");
    return new Set(names.filter(name => text.includes(`"${name}"`)));
}

function runSuite(server) {
    const log = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-methods-")), "methods.log");
    fs.writeFileSync(log, "");
    const preload = new URL("coverage/record-methods.mjs", import.meta.url).href;
    const result = spawnSync(process.execPath, ["--conditions", "@typescript/source", "--test", "--test-reporter=spec", "test/**/*.test.ts"], {
        cwd: pkgDir,
        encoding: "utf8",
        maxBuffer: 1 << 28,
        env: { ...process.env, TSRS_BINARY: server.exe, TSRS_METHOD_LOG: log, NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --import=${preload}`.trim() },
    });
    const output = `${result.stdout}\n${result.stderr}`;
    const count = key => Number(output.match(new RegExp(`^ℹ ${key} (\\d+)$`, "m"))?.[1] ?? NaN);
    const totals = { tests: count("tests"), pass: count("pass"), fail: count("fail"), cancelled: count("cancelled"), skipped: count("skipped") };
    if (Number.isNaN(totals.tests)) throw new Error(`${server.label}: could not read test totals; exit ${result.status}\n${output.slice(-4000)}`);
    const counts = new Map();
    for (const line of fs.readFileSync(log, "utf8").split("\n")) {
        const [variant, method, outcome] = line.split("\t");
        if (!outcome) continue;
        const entry = counts.get(method) ?? { syncOk: 0, syncError: 0, asyncOk: 0, asyncError: 0 };
        entry[`${variant}${outcome === "ok" ? "Ok" : "Error"}`]++;
        counts.set(method, entry);
    }
    const failing = [...output.matchAll(/^test at (\S+)$/gm)].map(m => m[1]);
    return { ...server, totals, counts, failing, exit: result.status };
}

function main() {
    const opts = parseArgs(process.argv.slice(2));
    const commit = pinnedCommit();
    const methods = protoMethods(opts.tsRef, commit);
    const names = methods.map(m => m.name);
    const sync = clientMethods([...listTs(path.join(pkgDir, "src/api/sync")), ...listTs(path.join(pkgDir, "src/api")).filter(f => !f.includes(`${path.sep}async${path.sep}`))], names);
    const asyncSet = clientMethods([...listTs(path.join(pkgDir, "src/api/async")), ...listTs(path.join(pkgDir, "src/api")).filter(f => !f.includes(`${path.sep}sync${path.sep}`))], names);
    const runs = opts.servers.map(server => {
        console.error(`running the SDK tests against ${server.label} (${server.exe})`);
        const run = runSuite(server);
        console.error(`${server.label}: ${run.totals.pass}/${run.totals.tests} pass, ${run.totals.fail} fail`);
        return run;
    });

    const lines = [];
    lines.push("# TypeScript 7 API method inventory (JS SDK view)", "");
    lines.push(`Generated by \`node npm/sdk/method-coverage.mjs\` from microsoft/TypeScript@${commit.slice(0, 12)} \`tsc/internal/api/proto.go\` (${methods.length} methods) and the vendored SDK in \`npm/tsrs/src\`.`, "");
    lines.push("- **sync / async SDK**: the method name appears in that client's sources (not counting the generated method table).");
    lines.push("- **per server**: requests the upstream SDK test suite sent, as `ok/error` response counts for sync | async (inner batch requests included). `ok` means the server answered, not that the test passed; errors include intentional error-path tests. A method a test suite never sends is `-`.");
    lines.push("- This is not a server support claim; see the Rust server inventory for implemented/stubbed status.", "");
    for (const run of runs) {
        lines.push(`Test run **${run.label}**: ${run.totals.tests} tests, ${run.totals.pass} pass, ${run.totals.fail} fail, ${run.totals.cancelled} cancelled, ${run.totals.skipped} skipped.` +
            (run.failing.length ? ` Failing files: ${[...new Set(run.failing)].map(f => `\`${f}\``).join(", ")}.` : ""));
    }
    const exercised = run => methods.filter(m => run.counts.has(m.name) && (run.counts.get(m.name).syncOk + run.counts.get(m.name).asyncOk) > 0).length;
    for (const run of runs) lines.push(`Methods with at least one ok response under **${run.label}**: ${exercised(run)}/${methods.length}.`);
    lines.push("");
    const header = ["method", "Go constant", "sync SDK", "async SDK", ...runs.map(r => `${r.label} sync`), ...runs.map(r => `${r.label} async`)];
    lines.push(`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`);
    const cell = (entry, variant) => {
        if (!entry) return "-";
        const ok = entry[`${variant}Ok`], error = entry[`${variant}Error`];
        return ok + error === 0 ? "-" : `${ok}/${error}`;
    };
    for (const m of methods) {
        const row = [`\`${m.name}\``, `\`${m.constant}\``, sync.has(m.name) ? "yes" : "no", asyncSet.has(m.name) ? "yes" : "no"];
        row.push(...runs.map(r => cell(r.counts.get(m.name), "sync")), ...runs.map(r => cell(r.counts.get(m.name), "async")));
        lines.push(`| ${row.join(" | ")} |`);
    }
    const unknown = runs.flatMap(r => [...r.counts.keys()].filter(n => !names.includes(n)).map(n => `${r.label}:${n}`));
    if (unknown.length) lines.push("", `Requests not in proto.go (protocol plumbing): ${[...new Set(unknown)].map(n => `\`${n}\``).join(", ")}.`);
    fs.writeFileSync(opts.out, lines.join("\n") + "\n");
    console.error(`wrote ${path.relative(process.cwd(), opts.out)}`);
}

main();
