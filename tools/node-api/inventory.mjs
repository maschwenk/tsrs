#!/usr/bin/env node
// Per-method coverage inventory against the pinned proto.go, built from real run-upstream.mjs runs.
//
//   node tools/node-api/inventory.mjs --oracle go-oracle [--candidate tsrs-<sha>] [--md out.md] [--json out.json]
//
// For every `Method` constant in ts-ref/tsc/internal/api/proto.go it reports, per run: direct requests, requests
// carried inside batchRequests, OK responses, error responses (and how many were explicit "unsupported"/unknown
// method errors), and the upstream tests that sent the method with their pass/fail outcome in that run.
//
// Candidate status (never inferred from client exports or from methods.rs claims):
//   unexercised    the oracle run never sent the method; the suites prove nothing about it
//   unimplemented  the candidate only answered it with unsupported/unknown-method errors
//   passing        every candidate test that sent it passed and it got at least one non-error answer
//   failing        otherwise (some test that sent it failed, or it errored where the oracle did not)
// "passing" means "no failing test sent it" — it is per-test evidence, not a proof of full method parity.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");

const opts = {};
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const v = () => argv[++i];
    if (a === "--oracle") opts.oracle = v();
    else if (a === "--candidate") opts.candidate = v();
    else if (a === "--md") opts.md = path.resolve(v());
    else if (a === "--json") opts.json = path.resolve(v());
    else if (a === "--ref") opts.ref = path.resolve(v());
    else if (a === "--methods-rs") opts.methodsRs = path.resolve(v());
    else {
        console.error(`unknown argument ${a}`);
        process.exit(2);
    }
}
if (!opts.oracle) {
    console.error("usage: inventory.mjs --oracle <label> [--candidate <label>] [--md file] [--json file]");
    process.exit(2);
}
opts.ref ??= path.join(repoRoot, "ts-ref");
opts.methodsRs ??= path.join(repoRoot, "crates", "tsrs_api", "src", "methods.rs");

const proto = fs.readFileSync(path.join(opts.ref, "tsc", "internal", "api", "proto.go"), "utf8");
const methods = [...proto.matchAll(/^\s*Method\w+\s+Method\s*=\s*"([^"]+)"/gm)].map(m => m[1]);

// Owner/claimed status from core's methods.rs when present (claims are shown, never trusted).
const claims = new Map();
if (fs.existsSync(opts.methodsRs)) {
    for (const m of fs.readFileSync(opts.methodsRs, "utf8").matchAll(/name:\s*"([^"]+)",\s*owner:\s*Owner::(\w+),\s*status:\s*Status::(\w+)/g)) {
        claims.set(m[1], { owner: m[2].toLowerCase(), claimed: m[3] });
    }
}

const UNSUPPORTED = /unsupported|not implemented|unknown API method|method not found/i;
const stripHook = name => name?.replace(/ \((before|beforeEach|after|afterEach)\)$/, "");

function loadRun(label) {
    const dir = path.join(here, ".work", label);
    const summary = JSON.parse(fs.readFileSync(path.join(dir, "summary.json"), "utf8"));
    const results = fs.readFileSync(path.join(dir, "results.jsonl"), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
    const outcome = new Map(); // test path -> ok (leaf tests and suites)
    for (const r of results) outcome.set(r.path, outcome.has(r.path) ? outcome.get(r.path) && r.ok : r.ok);
    const per = new Map(methods.map(m => [m, { direct: 0, batched: 0, ok: 0, errors: 0, unsupported: 0, messages: new Set(), tests: new Set() }]));
    const extra = new Map(); // methods on the wire that proto.go does not declare (echo/ping/timing...)
    const get = m => per.get(m) ?? extra.get(m) ?? (extra.set(m, { direct: 0, batched: 0, ok: 0, errors: 0, unsupported: 0, messages: new Set(), tests: new Set() }), extra.get(m));
    let processes = 0;
    let unparseable = 0;
    const traceDir = path.join(dir, "trace");
    for (const f of fs.existsSync(traceDir) ? fs.readdirSync(traceDir) : []) {
        let test;
        for (const line of fs.readFileSync(path.join(traceDir, f), "utf8").split("\n")) {
            if (!line) continue;
            const rec = JSON.parse(line);
            if (rec.kind === "start") {
                processes++;
                test = stripHook(rec.test) ?? `(file) ${rec.testFile}`;
                continue;
            }
            if (rec.kind === "unparseable") unparseable++;
            if (rec.dir === "c2s" && rec.kind === "request") {
                const e = get(rec.method);
                e.direct++;
                e.tests.add(test);
                for (const [m, n] of Object.entries(rec.inner ?? {})) {
                    const ie = get(m);
                    ie.batched += n;
                    ie.tests.add(test);
                }
            }
            else if (rec.dir === "s2c" && (rec.kind === "response" || rec.kind === "error") && rec.method) {
                const e = get(rec.method);
                if (rec.kind === "response") e.ok++;
                else {
                    e.errors++;
                    if (UNSUPPORTED.test(rec.message ?? "")) e.unsupported++;
                    if (e.messages.size < 3) e.messages.add(String(rec.message).slice(0, 160));
                }
                for (const [m, msgs] of Object.entries(rec.innerErrors ?? {})) {
                    const ie = get(m);
                    for (const msg of msgs) {
                        ie.errors++;
                        if (UNSUPPORTED.test(msg)) ie.unsupported++;
                        if (ie.messages.size < 3) ie.messages.add(msg.slice(0, 160));
                    }
                }
            }
        }
    }
    // Batched methods have no separate response frame; count inner sends without an inner error as answered.
    for (const e of per.values()) {
        if (e.batched > 0) e.ok += Math.max(0, e.batched - e.errors);
    }
    return { label, summary, outcome, per, extra, processes, unparseable };
}

function testOutcome(run, test) {
    if (run.outcome.has(test)) return run.outcome.get(test);
    return undefined; // hook of a suite, or a test file crash before reporting
}

const oracle = loadRun(opts.oracle);
const cand = opts.candidate ? loadRun(opts.candidate) : undefined;

const rows = methods.map((name, i) => {
    const o = oracle.per.get(name);
    const row = {
        index: i + 1,
        method: name,
        owner: claims.get(name)?.owner ?? "?",
        claimed: claims.get(name)?.claimed,
        oracle: { sent: o.direct + o.batched, ok: o.ok, errors: o.errors, tests: o.tests.size, testsFailed: [...o.tests].filter(t => testOutcome(oracle, t) === false).length },
    };
    if (cand) {
        const c = cand.per.get(name);
        const tests = [...c.tests];
        const failed = tests.filter(t => testOutcome(cand, t) === false);
        let status;
        if (o.direct + o.batched === 0) status = "unexercised";
        else if (c.direct + c.batched === 0) status = "not-sent"; // an earlier failure stopped the tests before this call
        else if (c.unsupported > 0 && c.ok === 0) status = "unimplemented";
        else if (failed.length === 0 && c.ok > 0 && c.errors <= o.errors) status = "passing";
        else status = "failing";
        row.candidate = { status, sent: c.direct + c.batched, ok: c.ok, errors: c.errors, unsupported: c.unsupported, tests: tests.length, testsFailed: failed.length, sampleErrors: [...c.messages], sampleFailedTests: failed.slice(0, 3) };
    }
    else row.status = o.direct + o.batched === 0 ? "unexercised" : row.oracle.testsFailed === 0 ? "oracle-passing" : "oracle-failing";
    return row;
});

const count = key => rows.reduce((acc, r) => ((acc[key(r)] = (acc[key(r)] ?? 0) + 1), acc), {});
const report = {
    tsRef: oracle.summary.tsRef,
    protoMethods: methods.length,
    oracle: { label: oracle.label, ...pick(oracle.summary), processes: oracle.processes, unparseableFrames: oracle.unparseable, exercisedMethods: rows.filter(r => r.oracle.sent > 0).length },
    candidate: cand && { label: cand.label, binary: cand.summary.binary, ...pick(cand.summary), processes: cand.processes, unparseableFrames: cand.unparseable, statusCounts: count(r => r.candidate.status) },
    extraWireMethods: Object.fromEntries([...oracle.extra].map(([m, e]) => [m, e.direct])),
    rows,
};
function pick(s) {
    return { tests: s.tests, pass: s.pass, fail: s.fail, skip: s.skip, todo: s.todo, crashedFiles: s.crashedFiles, nodeExit: s.nodeExit, timedOut: s.timedOut };
}

if (opts.json) fs.writeFileSync(opts.json, JSON.stringify(report, null, 2) + "\n");
const lines = [];
lines.push(`# Node API method inventory`, ``);
lines.push(`Generated by \`tools/node-api/inventory.mjs\` from real runs of the pinned upstream client test suites (and tools/node-api/tests) against the \`--api\` server. ts-ref \`${report.tsRef}\`; ${methods.length} \`Method\` constants in proto.go.`, ``);
lines.push(`- oracle \`${oracle.label}\`: ${oracle.summary.pass}/${oracle.summary.tests} tests passed, ${oracle.summary.fail} failed, ${oracle.processes} server processes, ${report.oracle.exercisedMethods}/${methods.length} methods sent by at least one test.`);
if (cand) {
    lines.push(`- candidate \`${cand.label}\`: ${cand.summary.pass}/${cand.summary.tests} tests passed, ${cand.summary.fail} failed, crashed files: ${cand.summary.crashedFiles.length ? cand.summary.crashedFiles.join(", ") : "none"}.`);
    lines.push(`- candidate status counts: ${Object.entries(report.candidate.statusCounts).map(([k, v]) => `${k} ${v}`).join(", ")}.`);
}
lines.push(``, `"passing" = every test that sent the method passed in that run; it is not a proof of full method parity. "unexercised" methods need dedicated tests before any claim.`, ``);
if (cand) {
    lines.push(`| # | method | owner | claimed | status | sent (oracle/cand) | cand ok/err/unsupported | tests (cand failed/total) | sample error |`);
    lines.push(`|---|---|---|---|---|---|---|---|---|`);
    for (const r of rows) {
        const c = r.candidate;
        lines.push(`| ${r.index} | \`${r.method}\` | ${r.owner} | ${r.claimed ?? ""} | ${c.status} | ${r.oracle.sent}/${c.sent} | ${c.ok}/${c.errors}/${c.unsupported} | ${c.testsFailed}/${c.tests} | ${(c.sampleErrors[0] ?? "").replace(/\|/g, "\\|").replace(/\n/g, " ")} |`);
    }
}
else {
    lines.push(`| # | method | owner | oracle status | sent | ok/err | tests |`);
    lines.push(`|---|---|---|---|---|---|---|`);
    for (const r of rows) lines.push(`| ${r.index} | \`${r.method}\` | ${r.owner} | ${r.status} | ${r.oracle.sent} | ${r.oracle.ok}/${r.oracle.errors} | ${r.oracle.tests} |`);
}
const md = lines.join("\n") + "\n";
if (opts.md) fs.writeFileSync(opts.md, md);
else process.stdout.write(md);
