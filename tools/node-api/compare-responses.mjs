#!/usr/bin/env node
// Paired-response comparison between two captured runs (run-upstream.mjs --capture), e.g. Go oracle vs tsrs.
//
//   node tools/node-api/compare-responses.mjs --a go-cap --b tsrs-cap [--out <dir>] [--drift <method>]
//
// Pairing: server processes are matched by the test that spawned them (preload.mjs) and their start order
// within that test. Within a pair, sync traffic is aligned request-by-request (the sync client is strictly
// sequential); async traffic is aligned by JSON-RPC request id. batchRequests are expanded into their inner
// method entries. Server->client filesystem callbacks are compared as per-pair multisets (their order depends
// on server-side parallelism).
//
// Normalization (documented, nothing else is removed or reordered):
//   - each run's private work-tree path is replaced by <TREE>;
//   - handle IDs are mapped through one bijection per process pair (see ID_KEYS / handle strings), applied to
//     requests and responses alike, so a handle returned by one call must be the one the next call sends;
//     a broken bijection is reported as a mismatch, not normalized away.
//   - binary (msgpack bin / AST) payloads are compared byte for byte.
// --drift <method> mutates the first successful <method> response of run B (negative control): the comparison
// must then report a mismatch for that method.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const opts = {};
const argv = process.argv.slice(2);
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const v = () => argv[++i];
    if (a === "--a") opts.a = v();
    else if (a === "--b") opts.b = v();
    else if (a === "--a2") opts.a2 = v();
    else if (a === "--out") opts.out = path.resolve(v());
    else if (a === "--drift") opts.drift = v();
    else if (a === "--drift-handle") opts.driftHandle = true;
    else if (a === "--ids") opts.idKeys = v().split(",");
    else {
        console.error(`unknown argument ${a}`);
        process.exit(2);
    }
}
if (!opts.a || !opts.b) {
    console.error("usage: compare-responses.mjs --a <label> --b <label> [--out dir] [--drift method]");
    process.exit(2);
}
opts.out ??= path.join(here, ".work", `compare-${opts.a}-vs-${opts.b}${opts.drift ? `-drift-${opts.drift}` : ""}${opts.driftHandle ? "-drift-handle" : ""}`);

// Fields whose values are server-assigned handles (numeric or string). Kept minimal and evidence-driven.
// Evidence (Go vs Go, two runs): symbol/type "id" values vary between runs of the same server because checking is
// parallel, so "id" is the only key mapped through the bijection by default.
const ID_KEYS = new Set(opts.idKeys ?? ["id"]);

function loadRun(label) {
    const dir = path.join(here, ".work", label);
    const traceDir = path.join(dir, "trace");
    const treeDir = path.join(dir, "tree");
    const byTest = new Map();
    for (const f of fs.readdirSync(traceDir)) {
        if (!f.endsWith(".capture.jsonl")) continue;
        const pid = f.split(".")[0];
        const start = JSON.parse(fs.readFileSync(path.join(traceDir, `${pid}.jsonl`), "utf8").split("\n")[0]);
        const records = fs.readFileSync(path.join(traceDir, f), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
        // Test names repeat across files (sync and async suites share names), so the key includes the file.
        const test = `${start.testFile} :: ${start.test ?? "(file)"}`;
        if (!byTest.has(test)) byTest.set(test, []);
        byTest.get(test).push({ pid, t: start.t, mode: start.mode, records });
    }
    for (const list of byTest.values()) list.sort((x, y) => x.t - y.t);
    return { label, treeDir, byTest };
}

const PLACEHOLDER_CHAR = "#";
function decode(rec, treeDir, mode) {
    if (rec.enc === "truncated") return { hash: rec.sha256, len: rec.len };
    if (rec.enc === "base64") {
        // Binary payloads (AST encodings) embed paths. Work-tree paths are replaced byte for byte with a
        // same-length placeholder, so offsets in the encoding are unaffected (labels must have equal length).
        const buf = Buffer.from(rec.data, "base64");
        const needle = Buffer.from(treeDir);
        let i = buf.indexOf(needle);
        while (i >= 0) {
            buf.fill(PLACEHOLDER_CHAR, i, i + needle.length);
            i = buf.indexOf(needle, i + needle.length);
        }
        return { binary: buf.toString("base64") };
    }
    currentTreeDir = treeDir;
    let text = rec.data.split(treeDir + "/").join("<TREE>/").split(treeDir).join("<TREE>");
    let json;
    try {
        json = JSON.parse(text);
    }
    catch {
        return { text: normalizeText(text) };
    }
    // Async frames are JSON-RPC envelopes; compare their params / result / error like sync payloads.
    if (mode === "async" && json && typeof json === "object" && json.jsonrpc) {
        if ("params" in json || "method" in json) json = json.params ?? null;
        else if ("error" in json) json = { error: json.error };
        else json = json.result ?? null;
    }
    return { json: normalizeStrings(json) };
}

// Go panics carry a goroutine stack (goroutine numbers, Go source paths). Only the panic message before the
// stack is compared; the stack is runtime-internal. Profiling temp directories are per-run.
function normalizeText(t) {
    return t.replace(/\ngoroutine \d+ \[[\s\S]*$/, "").replace(/\/tmp\/node-api-profile-[A-Za-z0-9]+(\/[^"]*)?/g, "<PROFILE_TMP>");
}
let currentTreeDir = "";
function normalizeStrings(v, key) {
    // Async binary responses are {"data": <base64>}; apply the same byte-level work-tree placeholder.
    if (typeof v === "string" && key === "data" && /^[A-Za-z0-9+/]+={0,2}$/.test(v) && v.length >= 16) {
        const buf = Buffer.from(v, "base64");
        const needle = Buffer.from(currentTreeDir);
        let i = buf.indexOf(needle);
        while (i >= 0) {
            buf.fill(PLACEHOLDER_CHAR, i, i + needle.length);
            i = buf.indexOf(needle, i + needle.length);
        }
        return buf.toString("base64");
    }
    if (typeof v === "string") return normalizeText(v);
    if (Array.isArray(v)) return v.map(x => normalizeStrings(x));
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, normalizeStrings(x, k)]));
    return v;
}

/** Bijection between handle values of run A and run B within one process pair. */
class Bijection {
    constructor() {
        this.ab = new Map();
        this.ba = new Map();
        this.conflicts = [];
    }
    map(a, b, where) {
        const ka = JSON.stringify(a);
        const kb = JSON.stringify(b);
        const prevB = this.ab.get(ka);
        const prevA = this.ba.get(kb);
        if (prevB === undefined && prevA === undefined) {
            this.ab.set(ka, kb);
            this.ba.set(kb, ka);
            return true;
        }
        if (prevB === kb && prevA === ka) return true;
        this.conflicts.push({ where, a, b, aWasMappedTo: prevB, bWasMappedFrom: prevA });
        return false;
    }
}

// Handle namespaces (proto.go): symbol references ({id, file?} / SymbolResponse.reference / CompactSymbolReference),
// types (TypeResponse.id and the type-id fields below, request params type/types/source/target), signatures
// (SignatureResponse.id/target, request param signature). Each namespace has its own bijection.
const TYPE_ID_KEYS = new Set(["typeParameters", "outerTypeParameters", "localTypeParameters", "objectType", "indexType", "checkType", "extendsType", "baseType", "substConstraint", "typeParameter", "constraintType", "nameType", "templateType", "freshType", "regularType", "thisType", "aliasTypeArguments"]);
const REQUEST_TYPE_KEYS = new Set(["type", "types", "source", "target"]);
function objectKind(o) {
    if ("value" in o && "flags" in o) return "type";
    if ("id" in o && "flags" in o && !("reference" in o) && !("name" in o)) return "sig";
    if ("id" in o && !("flags" in o)) return "sym";
    return "plain";
}
// CompactSymbolReference.file and SourceFileDescriptor.nodeId are the owning source file's node ID (proto.go).
const isCompactRef = o => Object.keys(o).every(k => k === "id" || k === "file");
// Escaped names of unique ES symbols and pattern ambient modules embed a server counter: "__@iterator@42",
// '__"*.css"pattern@4'. The trailing number goes through its own bijection; the rest must match exactly.
const COUNTER_SUFFIX = /^(__.+@)(\d+)$/;
function mapIds(a, b, ns, bij, p) {
    if (Array.isArray(a) && Array.isArray(b)) {
        if (a.length !== b.length) return { path: `${p}.length`, a: a.length, b: b.length };
        for (let i = 0; i < a.length; i++) if (!bij.map(`${ns}:${a[i]}`, `${ns}:${b[i]}`, `${p}[${i}]`)) return { path: `${p}[${i}]`, a: a[i], b: b[i], bijection: ns };
        return undefined;
    }
    if (typeof a === "number" && typeof b === "number") return bij.map(`${ns}:${a}`, `${ns}:${b}`, p) ? undefined : { path: p, a, b, bijection: ns };
    return diff(a, b, bij, p);
}
/** Deep compare; handle IDs go through namespaced bijections. Returns the first difference or undefined. */
function diff(a, b, bij, p = "$", top = false) {
    if (typeof a !== typeof b || Array.isArray(a) !== Array.isArray(b) || (a === null) !== (b === null)) return { path: p, a, b };
    if (a === null || typeof a !== "object") return a === b ? undefined : { path: p, a, b };
    if (Array.isArray(a)) {
        if (a.length !== b.length) return { path: `${p}.length`, a: a.length, b: b.length };
        for (let i = 0; i < a.length; i++) {
            const d = diff(a[i], b[i], bij, `${p}[${i}]`, top);
            if (d) return d;
        }
        return undefined;
    }
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (ka.join("\0") !== kb.join("\0")) return { path: `${p}{keys}`, a: ka, b: kb };
    const kind = objectKind(a);
    for (const k of ka) {
        let d;
        if (k === "nodeId" && typeof a[k] === "string" && typeof b[k] === "string") d = bij.map(`file:${a[k]}`, `file:${b[k]}`, `${p}.${k}`) ? undefined : { path: `${p}.${k}`, a: a[k], b: b[k], bijection: "file" };
        else if (k === "file" && kind === "sym" && isCompactRef(a) && typeof a[k] === "string" && typeof b[k] === "string") d = bij.map(`file:${a[k]}`, `file:${b[k]}`, `${p}.${k}`) ? undefined : { path: `${p}.${k}`, a: a[k], b: b[k], bijection: "file" };
        else if ((k === "name" || k === "escapedName") && typeof a[k] === "string" && typeof b[k] === "string" && COUNTER_SUFFIX.test(a[k]) && COUNTER_SUFFIX.test(b[k])) {
            const [, pa, na] = COUNTER_SUFFIX.exec(a[k]);
            const [, pb, nb] = COUNTER_SUFFIX.exec(b[k]);
            d = pa === pb && bij.map(`name:${na}`, `name:${nb}`, `${p}.${k}`) ? undefined : { path: `${p}.${k}`, a: a[k], b: b[k] };
        }
        else if (ID_KEYS.has(k) && typeof a[k] === "number") d = mapIds(a[k], b[k], kind, bij, `${p}.${k}`);
        else if (kind === "type" && (TYPE_ID_KEYS.has(k) || k === "target")) d = mapIds(a[k], b[k], "type", bij, `${p}.${k}`);
        else if (kind === "sig" && k === "target") d = mapIds(a[k], b[k], "sig", bij, `${p}.${k}`);
        else if (kind === "sig" && k === "typeParameters") d = mapIds(a[k], b[k], "type", bij, `${p}.${k}`);
        else if (top && REQUEST_TYPE_KEYS.has(k) && (typeof a[k] === "number" || Array.isArray(a[k]))) d = mapIds(a[k], b[k], "type", bij, `${p}.${k}`);
        else if (top && k === "signature" && typeof a[k] === "number") d = mapIds(a[k], b[k], "sig", bij, `${p}.${k}`);
        else d = diff(a[k], b[k], bij, `${p}.${k}`);
        if (d) return d;
    }
    return undefined;
}

function comparePayload(x, y, bij, isRequest = false) {
    if (x.hash || y.hash) return x.hash === y.hash && x.len === y.len ? undefined : { path: "$sha256", a: x.hash ?? "(captured)", b: y.hash ?? "(captured)" };
    if (x.binary !== undefined || y.binary !== undefined) return x.binary === y.binary ? undefined : { path: "$binary", a: `${(x.binary ?? "").length}b64`, b: `${(y.binary ?? "").length}b64` };
    if (x.text !== undefined || y.text !== undefined) return x.text === y.text ? undefined : { path: "$text", a: x.text, b: y.text };
    return diff(x.json, y.json, bij, "$", isRequest);
}

/** Request/response exchanges of one process, aligned for pairing. */
function exchanges(proc, treeDir) {
    const out = [];
    const calls = new Map();
    if (proc.mode === "async") {
        const byId = new Map();
        for (const r of proc.records) {
            if (r.kind === "call" || r.kind === "callResponse" || r.kind === "callError") {
                if (r.kind === "call") calls.set(`${r.method} ${r.data ?? r.sha256}`, (calls.get(`${r.method} ${r.data ?? r.sha256}`) ?? 0) + 1);
                continue;
            }
            if (r.kind === "request") byId.set(r.id, { method: r.method, id: r.id, req: decode(r, treeDir, "async") });
            else if ((r.kind === "response" || r.kind === "error") && byId.has(r.id)) Object.assign(byId.get(r.id), { kind: r.kind, res: decode(r, treeDir, "async") });
        }
        out.push(...[...byId.values()].sort((x, y) => x.id - y.id));
    }
    else {
        let pending;
        for (const r of proc.records) {
            if (r.kind === "call") {
                const k = `${r.method} ${r.data ?? r.sha256}`;
                calls.set(k, (calls.get(k) ?? 0) + 1);
                continue;
            }
            if (r.kind === "callResponse" || r.kind === "callError") continue;
            if (r.kind === "request") pending = { method: r.method, req: decode(r, treeDir) };
            else if ((r.kind === "response" || r.kind === "error") && pending) {
                Object.assign(pending, { kind: r.kind, res: decode(r, treeDir) });
                out.push(pending);
                pending = undefined;
            }
        }
        if (pending) out.push(pending);
    }
    return { out, calls };
}

/** Expand batchRequests into inner entries (method, params, result/error) for per-method attribution. */
function expand(ex) {
    if (ex.method !== "batchRequests" || !ex.req?.json || !ex.res?.json) return [ex];
    const reqs = ex.req.json.requests ?? [];
    const ress = ex.res.json.responses ?? [];
    if (reqs.length === 0 || reqs.length !== ress.length) return [ex];
    return reqs.map((r, i) => ({ method: r.method, batched: true, kind: ress[i].error ? "error" : "response", req: { json: r.params ?? null }, res: { json: ress[i] } }));
}

const A = loadRun(opts.a);
const B = loadRun(opts.b);
// Optional second oracle run: where the oracle itself is nondeterministic (map iteration order, counters in
// synthesized names, the known nested-request defect), a candidate exchange equal to EITHER oracle run counts
// as equal ("matchesOracleRun2"); differing from both is still a difference.
const A2 = opts.a2 ? loadRun(opts.a2) : undefined;

if (opts.drift) {
    // Negative control: perturb one semantic value in the first successful response of the method in run B.
    let done = false;
    const mutate = v => {
        if (Array.isArray(v)) {
            if (v.length) {
                v[0] = mutate(v[0]);
                return v;
            }
            v.push("drift");
            return v;
        }
        if (v && typeof v === "object") {
            const k = Object.keys(v).find(k => !ID_KEYS.has(k) && (typeof v[k] === "string" || typeof v[k] === "number" || typeof v[k] === "boolean")) ?? Object.keys(v).find(k => !ID_KEYS.has(k));
            if (k === undefined) return { drift: true };
            v[k] = typeof v[k] === "string" ? `${v[k]}~drift` : typeof v[k] === "number" ? v[k] + 1 : typeof v[k] === "boolean" ? !v[k] : mutate(v[k]);
            return v;
        }
        return typeof v === "string" && /^[A-Za-z0-9+/]{16,}={0,2}$/.test(v) ? `A${v.slice(1)}`.replace(/^AA/, "AB") : typeof v === "string" ? `${v}~drift` : typeof v === "number" ? v + 1 : typeof v === "boolean" ? !v : "drift";
    };
    for (const [test, list] of B.byTest) {
        for (const proc of list) {
            for (const r of proc.records) {
                if (!done && r.kind === "response" && r.method === opts.drift && r.enc === "utf8") {
                    opts.driftAt = { test, pid: proc.pid, seq: r.seq, before: r.data.slice(0, 120) };
                    const json = JSON.parse(r.data);
                    // Mutate the payload the comparison sees: the JSON-RPC result, not the envelope.
                    if (json && typeof json === "object" && json.jsonrpc && "result" in json) json.result = mutate(json.result);
                    else mutate(json);
                    r.data = JSON.stringify(json);
                    opts.driftAt.after = r.data.slice(0, 120);
                    r.drifted = true;
                    done = true;
                }
            }
        }
    }
    if (!done) {
        console.error(`--drift: no successful utf8 response for ${opts.drift} in ${opts.b}`);
        process.exit(2);
    }
}

let unstableHit = false;
// Order-insensitive view used ONLY where the two oracle runs themselves disagree on order: arrays are sorted by
// their elements' JSON with handle fields blanked (for the sort key only; the comparison still checks them).
function sortKey(v) {
    return JSON.stringify(v, (k, x) => (k === "id" || k === "file" || k === "nodeId" ? undefined : x));
}
function sortArrays(v) {
    if (Array.isArray(v)) return v.map(sortArrays).sort((x, y) => (sortKey(x) < sortKey(y) ? -1 : sortKey(x) > sortKey(y) ? 1 : 0));
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, sortArrays(x)]));
    return v;
}
if (opts.driftHandle) {
    // Handle-identity negative control: in run B, make the first request that sends back a symbol reference
    // point at the next symbol id. The id was received earlier in that process, so the bijection must flag it.
    outer: for (const [test, list] of B.byTest) {
        for (const proc of list) {
            for (const r of proc.records) {
                const m = r.kind === "request" && r.enc === "utf8" && /("symbol":\{[^{}]*"id":)(\d+)/.exec(r.data);
                if (m) {
                    opts.driftAt = { test, method: r.method, before: m[0] };
                    r.data = r.data.replace(m[0], `${m[1]}${Number(m[2]) + 1}`);
                    break outer;
                }
            }
        }
    }
}
const perMethod = new Map();
const counter = m => {
    if (!perMethod.has(m)) perMethod.set(m, { pairs: 0, equal: 0, differ: 0, okPairs: 0, okEqual: 0, sync: 0, async: 0, batched: 0, examples: [] });
    return perMethod.get(m);
};
const unpaired = [];
const callDiffs = [];
let pairedProcesses = 0;
// Processes of one test started concurrently can start in either order; pair each A process with the first
// unpaired B process of the same mode and request-method sequence, falling back to start order.
const signature = proc => proc.mode + ":" + proc.records.filter(r => r.kind === "request").map(r => r.method).join(",");
function pairUp(listA, listB) {
    const used = new Set();
    const pairs = listA.map(pa => {
        const j = listB.findIndex((pb, j) => !used.has(j) && signature(pb) === signature(pa));
        if (j >= 0) {
            used.add(j);
            return [pa, listB[j]];
        }
        return [pa, undefined];
    });
    for (const pair of pairs) {
        if (pair[1]) continue;
        const j = listB.findIndex((pb, j) => !used.has(j) && pb.mode === pair[0].mode);
        if (j >= 0) {
            used.add(j);
            pair[1] = listB[j];
        }
    }
    listB.forEach((pb, j) => {
        if (!used.has(j)) pairs.push([undefined, pb]);
    });
    return pairs;
}
// todo tests document known upstream defects (Go crash, Go nondeterministic drop, client hang); their traffic is
// excluded from per-method outcomes and listed separately, as in inventory.mjs.
const todoTests = new Set();
for (const run of [A, B]) {
    const res = path.join(here, ".work", run.label, "results.jsonl");
    if (!fs.existsSync(res)) continue;
    for (const l of fs.readFileSync(res, "utf8").split("\n").filter(Boolean)) {
        const r = JSON.parse(l);
        if (r.todo) todoTests.add(`${path.basename(r.file)} :: ${r.path}`);
    }
}
for (const [test, listA] of A.byTest) {
    if (todoTests.has(test)) continue;
    const listB = B.byTest.get(test) ?? [];
    const pairs = pairUp(listA, listB);
    for (let i = 0; i < pairs.length; i++) {
        const [pa, pb] = pairs[i];
        if (!pa || !pb) {
            unpaired.push({ test, index: i, missingIn: pa ? opts.b : opts.a });
            continue;
        }
        pairedProcesses++;
        const xa = exchanges(pa, A.treeDir);
        const xb = exchanges(pb, B.treeDir);
        const ea = xa.out.flatMap(expand);
        const eb = xb.out.flatMap(expand);
        const bij = new Bijection();
        let ea2;
        const bij2 = new Bijection();
        if (A2) {
            const p2 = pairUp([pa], A2.byTest.get(test) ?? [])[0]?.[1];
            if (p2) ea2 = exchanges(p2, A2.treeDir).out.flatMap(expand);
        }
        const n = Math.min(ea.length, eb.length);
        for (let k = 0; k < n; k++) {
            const a = ea[k];
            const b = eb[k];
            const c = counter(a.method);
            c.pairs++;
            c[pa.mode]++;
            if (a.batched) c.batched++;
            let d;
            if (a.method !== b.method) d = { path: "$method", a: a.method, b: b.method };
            else if (a.kind !== b.kind) d = { path: "$kind", a: a.kind, b: b.kind };
            else {
                const cmp = (x, y, bj) => {
                    const dr = comparePayload(x.req, y.req, bj, true);
                    return dr ? { ...dr, in: "request" } : x.res && y.res ? comparePayload(x.res, y.res, bj) : undefined;
                };
                d = cmp(a, b, bij);
                const a2 = ea2?.[k];
                // The second-run bijection is maintained on every exchange (not only on differences), so an
                // identity break is caught against run 2 as well.
                const d2 = a2 && a2.method === b.method && a2.kind === b.kind ? cmp(a2, b, bij2) : { path: "$unaligned" };
                if (d && a2 && a2.method === b.method && a2.kind === b.kind) {
                    if (!d2) {
                        c.matchesOracleRun2 = (c.matchesOracleRun2 ?? 0) + 1;
                        d = undefined;
                    }
                    else if (cmp(a, a2, new Bijection()) && a.res?.json !== undefined && b.res?.json !== undefined && !diff(sortArrays(a.res.json), sortArrays(b.res.json), new Bijection())) {
                        // Oracle runs disagree only in array order here, and the candidate has the same elements.
                        c.equalUnordered = (c.equalUnordered ?? 0) + 1;
                        d = undefined;
                    }
                    else if (cmp(a, a2, new Bijection())) {
                        // The two oracle runs disagree here too: the oracle is nondeterministic at this exchange
                        // and the candidate matches neither observed variant. Reported separately.
                        c.oracleUnstable = (c.oracleUnstable ?? 0) + 1;
                        if ((c.unstableExamples ??= []).length < 2) c.unstableExamples.push({ test, exchange: k, diff: truncate(d) });
                        d = undefined;
                        unstableHit = true;
                    }
                }
            }
            if (a.kind === "response") c.okPairs++;
            if (unstableHit) {
                unstableHit = false;
                continue;
            }
            if (d) {
                c.differ++;
                if (c.examples.length < 3) c.examples.push({ test, process: i, exchange: k, mode: pa.mode, batched: !!a.batched, diff: truncate(d) });
                if (a.method !== b.method || a.kind !== b.kind) {
                    // sequences diverged: stop pairing this process
                    unpaired.push({ test, index: i, divergedAt: k, a: `${a.method}/${a.kind}`, b: `${b.method}/${b.kind}` });
                    break;
                }
            }
            else {
                c.equal++;
                if (a.kind === "response") {
                    c.okEqual++;
                    c[`okEqual_${pa.mode}`] = (c[`okEqual_${pa.mode}`] ?? 0) + 1;
                }
            }
        }
        if (ea.length !== eb.length) unpaired.push({ test, index: i, lengths: [ea.length, eb.length] });
        const ca = [...xa.calls].sort();
        const cb = [...xb.calls].sort();
        if (JSON.stringify(ca) !== JSON.stringify(cb)) callDiffs.push({ test, index: i, onlyA: ca.filter(([k, v]) => xb.calls.get(k) !== v).slice(0, 5).map(([k, v]) => `${v}x ${k.slice(0, 160)}`), onlyB: cb.filter(([k, v]) => xa.calls.get(k) !== v).slice(0, 5).map(([k, v]) => `${v}x ${k.slice(0, 160)}`) });
    }
}
for (const test of B.byTest.keys()) if (!A.byTest.has(test) && !todoTests.has(test)) unpaired.push({ test, missingIn: opts.a });

function truncate(d) {
    const s = v => {
        const j = JSON.stringify(v);
        return j && j.length > 300 ? `${j.slice(0, 300)}...` : v;
    };
    return { ...d, a: s(d.a), b: s(d.b) };
}

const proto = fs.readFileSync(path.join(here, "..", "..", "ts-ref", "tsc", "internal", "api", "proto.go"), "utf8");
const methods = [...proto.matchAll(/^\s*Method\w+\s+Method\s*=\s*"([^"]+)"/gm)].map(m => m[1]);
const rows = methods.map(m => {
    const c = perMethod.get(m);
    let status;
    if (!c) status = "unpaired";
    else if (c.differ > 0) status = "differs";
    else if (c.okEqual > 0) status = "equal";
    else if (c.equalUnordered > 0) status = "equal-unordered";
    else if (c.oracleUnstable > 0) status = "oracle-unstable-unverified";
    else if (c.matchesOracleRun2 > 0) status = "equal";
    else status = "equal-errors-only";
    return { method: m, status, ...(c ?? {}) };
});
const report = {
    a: opts.a,
    b: opts.b,
    drift: opts.drift ?? null,
    driftAt: opts.driftAt ?? null,
    idKeys: [...ID_KEYS],
    pairedProcesses,
    excludedTodoTests: [...todoTests],
    statusCounts: rows.reduce((acc, r) => ((acc[r.status] = (acc[r.status] ?? 0) + 1), acc), {}),
    unpairedCount: unpaired.length,
    callMultisetDiffs: callDiffs.length,
    rows,
    extraMethods: Object.fromEntries([...perMethod].filter(([m]) => !methods.includes(m)).map(([m, c]) => [m, { pairs: c.pairs, differ: c.differ }])),
    unpaired: unpaired.slice(0, 200),
    callDiffs: callDiffs.slice(0, 50),
};
fs.mkdirSync(opts.out, { recursive: true });
fs.writeFileSync(path.join(opts.out, "compare.json"), JSON.stringify(report, null, 2) + "\n");
// Markdown: every proto.go method with its outcome, per-mode successful equal pairs, and the exact differences.
const md = [`# Paired responses: ${opts.a} vs ${opts.b}${opts.a2 ? ` (second oracle run ${opts.a2})` : ""}${opts.drift ? ` [negative control: drift ${opts.drift}]` : ""}`, ""];
md.push(`Paired server processes: ${pairedProcesses}. Status counts: ${Object.entries(report.statusCounts).map(([k, v]) => `${k} ${v}`).join(", ")}. Unpaired/diverged processes: ${unpaired.length}. Callback multiset differences: ${callDiffs.length} processes (callback order/duplication is server-internal; not used for method status).`, "");
md.push("| method | status | pairs | ok equal (sync/async) | errors equal | matches oracle run 2 | equal unordered | oracle unstable | differ | first difference |");
md.push("|---|---|---|---|---|---|---|---|---|---|");
for (const r of rows) {
    const ex = r.examples?.[0];
    const first = ex ? `${ex.diff.in === "request" ? "request " : ""}${ex.diff.path}: ${JSON.stringify(ex.diff.a)} vs ${JSON.stringify(ex.diff.b)} (${ex.test})`.replace(/\|/g, "\\|").slice(0, 300) : "";
    md.push(`| \`${r.method}\` | ${r.status} | ${r.pairs ?? 0} | ${r.okEqual_sync ?? 0}/${r.okEqual_async ?? 0} | ${(r.equal ?? 0) - (r.okEqual ?? 0)} | ${r.matchesOracleRun2 ?? 0} | ${r.equalUnordered ?? 0} | ${r.oracleUnstable ?? 0} | ${r.differ ?? 0} | ${first} |`);
}
fs.writeFileSync(path.join(opts.out, "compare.md"), md.join("\n") + "\n");
console.log(JSON.stringify({ out: opts.out, driftAt: opts.driftAt, pairedProcesses, statusCounts: report.statusCounts, unpaired: unpaired.length, callMultisetDiffs: callDiffs.length, differing: rows.filter(r => r.status === "differs").map(r => `${r.method}(${r.differ}/${r.pairs})`) }, null, 1));
