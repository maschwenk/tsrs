#!/usr/bin/env node
// Paired-response comparison between two captured runs (run-upstream.mjs --capture), e.g. Go oracle vs tsrs.
//
//   node tools/node-api/compare-responses.mjs --a go-cap --b tsrs-cap [--out <dir>] [--drift <method>]
//
// Pairing: server processes are matched by the test that spawned them (preload.mjs); within a test, each A process
// is paired with the first unused B process of the same mode and request-method sequence, else the first unused B
// process of the same mode. Within a pair, sync traffic is aligned request-by-request (the sync client is strictly
// sequential); async traffic is aligned by JSON-RPC request id. batchRequests are expanded into their inner
// method entries. Server->client filesystem callbacks are compared as per-pair multisets (their order depends
// on server-side parallelism).
//
// Normalization (only these; JSON is compared as parsed values, so object key order is ignored; everything else,
// binary payloads included, is compared exactly; payloads over the capture limit are compared by sha256, and a
// mismatch is inconclusive):
//   - each run's private work-tree path is replaced by <TREE> in text, and by a same-length `#` fill in binary
//     (msgpack bin / AST) payloads;
//   - async JSON-RPC envelopes are reduced to their params / result / error;
//   - Go goroutine stacks and /tmp/node-api-profile-* paths are stripped from strings (normalizeText);
//   - handle IDs, and the counters in server-synthesized `__@x@N` / `__"p"pattern@N` names, are mapped through
//     bijections per process pair (see ID_KEYS / handle strings / COUNTER_SUFFIX), applied to requests and
//     responses alike, so a handle returned by one call must be the one the next call sends; a broken bijection
//     is reported as a mismatch, not normalized away;
//   - with --a2, an exchange equal to oracle run 2 also counts as equal (outcome "run2"), and arrays whose order
//     the two oracle runs disagree on are reordered to run A's order before comparing (outcome "unordered").
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
    currentMethod = rec.method;
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
let currentMethod = "";
// Methods whose responses are binary (msgpack bin on the sync channel); on the async channel the same bytes
// arrive as {"data": <base64>}. Filled from the upstream schema before decoding (see below).
const BINARY_METHODS = new Set();
function canonicalBase64(v) {
    return typeof v === "string" && v.length > 0 && Buffer.from(v, "base64").toString("base64") === v;
}
function normalizeStrings(v, key, depth = 0) {
    // Only the top-level "data" field of a binary method's async response, and only canonical base64: the
    // work-tree path is replaced byte for byte. Any other string (including non-canonical base64) is compared as is.
    if (depth === 1 && key === "data" && BINARY_METHODS.has(currentMethod) && canonicalBase64(v)) {
        const buf = Buffer.from(v, "base64");
        const needle = Buffer.from(currentTreeDir);
        let i = needle.length ? buf.indexOf(needle) : -1;
        while (i >= 0) {
            buf.fill(PLACEHOLDER_CHAR, i, i + needle.length);
            i = buf.indexOf(needle, i + needle.length);
        }
        return buf.toString("base64");
    }
    if (typeof v === "string") return normalizeText(v);
    if (Array.isArray(v)) return v.map(x => normalizeStrings(x, undefined, depth + 1));
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, normalizeStrings(x, k, depth + 1)]));
    return v;
}

/** Bijection between handle values of run A and run B within one process pair. */
class Bijection {
    constructor() {
        this.ab = new Map();
        this.ba = new Map();
        this.log = []; // keys added, in order, so a failed trial can be undone without copying the maps
        this.conflicts = [];
    }
    mark() {
        return this.log.length;
    }
    rollback(mark) {
        while (this.log.length > mark) {
            const [ka, kb] = this.log.pop();
            this.ab.delete(ka);
            this.ba.delete(kb);
        }
    }
    map(a, b, where) {
        const ka = JSON.stringify(a);
        const kb = JSON.stringify(b);
        const prevB = this.ab.get(ka);
        const prevA = this.ba.get(kb);
        if (prevB === undefined && prevA === undefined) {
            this.ab.set(ka, kb);
            this.ba.set(kb, ka);
            this.log.push([ka, kb]);
            return true;
        }
        if (prevB === kb && prevA === ka) return true;
        if (this.conflicts.length < 20) this.conflicts.push({ where, a, b, aWasMappedTo: prevB, bWasMappedFrom: prevA });
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
// Only the two server-synthesized forms are renamed: unique ES symbols "__@<identifier>@<n>" and pattern ambient
// modules '__"<pattern>"pattern@<n>'. Anything else (e.g. a user property "__k@1") is compared exactly.
const COUNTER_SUFFIX = /^(__@[A-Za-z_$][\w$]*@|__".*"pattern@)(\d+)$/;
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
    // Payloads over the capture limit are compared by sha256 of the raw bytes. Equal hashes mean identical bytes;
    // a differing hash cannot be attributed (handles or content), so it is inconclusive, never equal or a proven diff.
    if (x.hash || y.hash) return x.hash === y.hash && x.len === y.len ? undefined : { path: "$sha256", a: x.hash ?? "(captured)", b: y.hash ?? "(captured)", inconclusive: true };
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
// N4: the binary-response methods are pinned from the upstream schema at the pinned commit (APIMethodInfo entries
// whose result is SourceFileResponse), never learned from captured output (a candidate could otherwise widen it).
{
    const generated = fs.readFileSync(path.join(here, "..", "..", "ts-ref", "packages", "typescript", "src", "api", "proto.generated.ts"), "utf8");
    for (const m of generated.matchAll(/^\s+(\w+): APIMethod<\w+, SourceFileResponse(?: \| null)?>;/gm)) BINARY_METHODS.add(m[1]);
    if (BINARY_METHODS.size === 0) throw new Error("no SourceFileResponse methods found in proto.generated.ts");
}

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
                if (!done && r.kind === "response" && r.method === opts.drift && r.enc === "base64") {
                    // Sync binary payload (AST encoding): flip one byte in the middle.
                    const buf = Buffer.from(r.data, "base64");
                    buf[buf.length >> 1] ^= 0x01;
                    opts.driftAt = { test, pid: proc.pid, seq: r.seq, binaryByte: buf.length >> 1 };
                    r.data = buf.toString("base64");
                    done = true;
                    continue;
                }
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
        console.error(`--drift: no successful response for ${opts.drift} in ${opts.b}`);
        process.exit(2);
    }
}

// Order relaxation (only where the two oracle runs disagree on order). orderPaths(x, y, bjAA, budget) returns the
// exact array paths at which run 2 is a permutation of run 1 with everything else equal, or null if they differ in
// any other way. Oracle-vs-oracle comparisons use the per-process run1<->run2 handle mapping established by
// earlier exchanges (R2): elements that differ only in handles already known to be distinct are not mistaken for
// a renaming. Bindings made here are committed to bjAA only by the caller when the whole relation holds.
//
// structKey(v): the value with every handle-valued field blanked and counter suffixes cut. Two values that are
// equal under any handle bijection have the same key, so bucketing by key never rejects a valid match.
const HANDLE_FIELDS = new Set(["id", "file", "nodeId", "target", ...TYPE_ID_KEYS]);
function structKey(v) {
    return JSON.stringify(v, (k, x) => (HANDLE_FIELDS.has(k) ? undefined : typeof x === "string" && COUNTER_SUFFIX.test(x) ? COUNTER_SUFFIX.exec(x)[1] : x));
}
const looseEqual = (x, y) => !diff(x, y, new Bijection());
// Matching attempts allowed per exchange (oracle-order detection and candidate reordering share it); beyond it the
// exchange is inconclusive (never equal, never a proven difference).
const MATCH_BUDGET = 20000;
class BudgetExceeded extends Error {}
/**
 * Bounded backtracking search for a permutation `chosen` with xa[i] equal to xb[chosen[i]] under `bij`,
 * candidates restricted to the same structural key. On success the bindings stay in `bij` and the permutation is
 * returned; on exhaustion `bij` is restored and null is returned (no matching exists under the given mapping).
 * Throws BudgetExceeded (with `bij` restored) when budget.attempts exceeds MATCH_BUDGET.
 */
function matchPermutation(xa, xb, bij, budget) {
    const start = bij.mark();
    const buckets = new Map();
    xb.forEach((e, j) => {
        const k = structKey(e);
        if (!buckets.has(k)) buckets.set(k, []);
        buckets.get(k).push(j);
    });
    const bucketOf = xa.map(e => buckets.get(structKey(e)) ?? []);
    const used = new Set();
    const chosen = new Array(xa.length);
    const frames = [];
    let idx = 0;
    let pos = 0;
    try {
        while (idx < xa.length) {
            const bucket = bucketOf[idx];
            let advanced = false;
            for (; pos < bucket.length; pos++) {
                const j = bucket[pos];
                if (used.has(j)) continue;
                if (++budget.attempts > MATCH_BUDGET) throw new BudgetExceeded();
                const m = bij.mark();
                if (!diff(xa[idx], xb[j], bij)) {
                    used.add(j);
                    chosen[idx] = j;
                    frames.push({ pos, m, j });
                    idx++;
                    pos = 0;
                    advanced = true;
                    break;
                }
                bij.rollback(m);
            }
            if (advanced) continue;
            const prev = frames.pop();
            if (!prev) {
                bij.rollback(start);
                return null;
            }
            idx--;
            used.delete(prev.j);
            bij.rollback(prev.m);
            pos = prev.pos + 1;
        }
    }
    catch (e) {
        bij.rollback(start);
        throw e;
    }
    return chosen;
}
const isContainer = v => v !== null && typeof v === "object";
function orderPaths(x, y, bj, budget, p = "$", out = []) {
    if (Array.isArray(x) && Array.isArray(y)) {
        if (x.length !== y.length) return null;
        const m = bj.mark();
        if (x.every((e, i) => !diff(e, y[i], bj))) return out;
        bj.rollback(m);
        if (matchPermutation(x, y, bj, budget)) {
            out.push(p);
            return out;
        }
        for (let i = 0; i < x.length; i++) if (!orderPaths(x[i], y[i], bj, budget, `${p}[${i}]`, out)) return null;
        return out;
    }
    if (isContainer(x) && isContainer(y) && !Array.isArray(x) && !Array.isArray(y)) {
        const kx = Object.keys(x).sort();
        if (kx.join("\0") !== Object.keys(y).sort().join("\0")) return null;
        // Scalar fields first (container fields nulled), so handle fields keep their object-kind namespace.
        const scal = o => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, isContainer(v) ? null : v]));
        if (diff(scal(x), scal(y), bj)) return null;
        for (const k of kx) if (isContainer(x[k]) && !orderPaths(x[k], y[k], bj, budget, `${p}.${k}`, out)) return null;
        return out;
    }
    return diff(x, y, bj) ? null : out;
}
function getAt(v, p) {
    for (const part of p.slice(1).match(/\.[^.[]+|\[\d+\]/g) ?? []) v = part[0] === "." ? v?.[part.slice(1)] : v?.[Number(part.slice(1, -1))];
    return v;
}
function setAt(v, p, value) {
    const parts = p.slice(1).match(/\.[^.[]+|\[\d+\]/g) ?? [];
    if (parts.length === 0) return value;
    const root = structuredClone(v);
    let cur = root;
    for (let i = 0; i < parts.length - 1; i++) cur = parts[i][0] === "." ? cur[parts[i].slice(1)] : cur[Number(parts[i].slice(1, -1))];
    const last = parts[parts.length - 1];
    if (last[0] === ".") cur[last.slice(1)] = value;
    else cur[Number(last.slice(1, -1))] = value;
    return root;
}
/**
 * Reorders the candidate's arrays at exactly the given paths to the oracle's order (matchPermutation under the
 * process A<->B mapping). Returns the reordered value, or undefined if no matching exists (search exhausted).
 */
function reorderCandidate(aVal, bVal, paths, bij, budget) {
    let out = bVal;
    for (const p of paths) {
        const xa = getAt(aVal, p);
        const xb = getAt(out, p);
        if (!Array.isArray(xa) || !Array.isArray(xb) || xa.length !== xb.length) return undefined;
        const chosen = matchPermutation(xa, xb, bij, budget);
        if (!chosen) return undefined;
        out = setAt(out, p, chosen.map(j => xb[j]));
    }
    return out;
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
const slowExchanges = [];
const callDiffs = [];
let pairedProcesses = 0;
// Processes of one test started concurrently can start in either order; pair each A process with the first
// unpaired B process of the same mode and request-method sequence, else the first unpaired B process of the same mode.
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
            // Fail closed: every exchange of a process without a counterpart (missing candidate process, or a
            // surplus candidate process the oracle never started) is unverified.
            const lone = pa ?? pb;
            for (const e of exchanges(lone, (pa ? A : B).treeDir).out.flatMap(expand)) {
                const c = counter(e.method);
                c.unverified = (c.unverified ?? 0) + 1;
                if (pa) c.unpairedExchanges = (c.unpairedExchanges ?? 0) + 1;
                else c.surplusExchanges = (c.surplusExchanges ?? 0) + 1;
            }
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
        let branch = "both"; // "both" | "A" | "A2": which oracle history the accepted candidate exchanges follow
        const bijAA = new Bijection(); // per-process run1 <-> run2 handle mapping
        if (A2) {
            const p2 = pairUp([pa], A2.byTest.get(test) ?? [])[0]?.[1];
            if (p2) ea2 = exchanges(p2, A2.treeDir).out.flatMap(expand);
        }
        const n = Math.min(ea.length, eb.length);
        const cmp = (x, y, bj) => {
            const dr = comparePayload(x.req, y.req, bj, true);
            return dr ? { ...dr, in: "request" } : x.res && y.res ? comparePayload(x.res, y.res, bj) : undefined;
        };
        let k = 0;
        for (; k < n; k++) {
            const a = ea[k];
            const b = eb[k];
            const c = counter(a.method);
            c.pairs++;
            c[pa.mode]++;
            if (a.batched) c.batched++;
            const a2early = ea2?.[k];
            const b2aligned = Boolean(a2early && a2early.method === b.method && a2early.kind === b.kind);
            if (a.method !== b.method || (a.kind !== b.kind && !b2aligned)) {
                const d = a.method !== b.method ? { path: "$method", a: a.method, b: b.method } : { path: "$kind", a: a.kind, b: b.kind };
                if (b2aligned) {
                    // The candidate follows oracle run 2's request sequence where run 1 went elsewhere: the oracle
                    // runs themselves diverge, so this is not a proven difference; the rest is unverified.
                    c.unverified = (c.unverified ?? 0) + 1;
                    c.oracleUnstable = (c.oracleUnstable ?? 0) + 1;
                    if ((c.unverifiedExamples ??= []).length < 3) c.unverifiedExamples.push({ test, exchange: k, outcome: "unstable", diff: truncate(d) });
                }
                else {
                    c.differ++;
                    if (c.examples.length < 3) c.examples.push({ test, process: i, exchange: k, mode: pa.mode, batched: !!a.batched, diff: truncate(d) });
                }
                // Sequences diverged: everything after this point is unverified on both sides.
                unpaired.push({ test, index: i, divergedAt: k, a: `${a.method}/${a.kind}`, b: `${b.method}/${b.kind}` });
                k++;
                break;
            }
            // From here a.method === b.method; a.kind may differ from b.kind only when run 2 is aligned with b
            // (e.g. run 1 errored where run 2 and the candidate answered): run 1 is then simply not viable.
            // outcome: "strict" | "run2" | "unordered" | "unstable" | "inconclusive" | "differ"
            const started = Date.now();
            let outcome;
            const m1 = bij.mark();
            let d = a.kind !== b.kind ? { path: "$kind", a: a.kind, b: b.kind } : cmp(a, b, bij);
            const a2 = ea2?.[k];
            const aligned2 = Boolean(a2 && a2.method === b.method && a2.kind === b.kind);
            let d2;
            const m2 = bij2.mark();
            if (aligned2) d2 = cmp(a2, b, bij2);
            // Oracle agreement is judged under the per-process run1<->run2 mapping (R2), committed when they agree.
            let dAA;
            if (aligned2) {
                const mAA = bijAA.mark();
                dAA = a.kind !== a2.kind ? { path: "$kind", a: a.kind, b: a2.kind } : cmp(a, a2, bijAA);
                if (dAA) bijAA.rollback(mAA);
            }
            // Run 2 present for this request but with another kind than run 1 (e.g. run 1 answered, run 2 errored)
            // is visible oracle disagreement (D1), not absent evidence. Only a missing run-2 exchange (or no second
            // run) leaves run 1 as the sole reference.
            const kindSplit2 = Boolean(!aligned2 && a2 && a2.method === a.method && a2.kind !== a.kind);
            const oraclesAgree = aligned2 ? !dAA : !kindSplit2;
            // Truth table (R1/N1). viable1/viable2: the candidate equals run 1 / run 2 under that run's process
            // mapping. An acceptance against exactly one viable run (the other differs, errors, is unaligned or
            // missing) locks the process to that run; afterwards only that run's history can accept exchanges.
            //   both viable                  -> strict (branch unchanged; both mappings extended)
            //   only run 1 viable, not locked to run 2 -> strict, lock run 1
            //   only run 2 viable, not locked to run 1 -> run2,   lock run 2
            //   viable only on the locked-out run       -> inconclusive (history switch)
            //   neither viable                -> hash: inconclusive; locked to run 2: differ only if the oracle runs
            //                                    agree here, else inconclusive; otherwise order relaxation / unstable /
            //                                    differ against run 1 as below
            const viable1 = !d;
            const viable2 = aligned2 && !d2;
            const accA = viable1 && branch !== "A2";
            const accA2 = viable2 && branch !== "A";
            if (!accA) bij.rollback(m1);
            if (!accA2) bij2.rollback(m2);
            if (accA) {
                outcome = "strict";
                if (!viable2) branch = "A";
            }
            else if (accA2) {
                outcome = "run2";
                branch = "A2";
            }
            else if (viable1 || viable2) {
                outcome = "inconclusive";
                d = { path: "$branch", a: `history locked to oracle run ${branch === "A" ? 1 : 2}`, b: "matches only the other oracle run", inconclusive: true };
                c.branchInconclusive = (c.branchInconclusive ?? 0) + 1;
            }
            else if (d.inconclusive || d2?.inconclusive) outcome = "inconclusive";
            else if (branch === "A2") {
                if (aligned2 && oraclesAgree) {
                    outcome = "differ";
                    d = d2;
                }
                else outcome = "unstable";
            }
            else if (aligned2 && !oraclesAgree) {
                // The oracle runs disagree on this exchange. Accept the candidate only if (1) the oracle requests
                // agree under the run1<->run2 mapping and the candidate request matches under the process mapping,
                // and (2) run 2's response is a permutation of run 1's at exact array paths (under the run1<->run2
                // mapping) and the candidate equals run 1 after reordering only those arrays, with the process
                // mapping. A failed relaxation is a proven difference only if the matching search was exhaustive.
                let paths = null;
                const budget = { attempts: 0 };
                let budgetHit = false;
                if (a.res?.json !== undefined && a2.res?.json !== undefined) {
                    const mAA = bijAA.mark();
                    if (!comparePayload(a.req, a2.req, bijAA, true)) {
                        try {
                            paths = orderPaths(a.res.json, a2.res.json, bijAA, budget);
                        }
                        catch (e) {
                            if (!(e instanceof BudgetExceeded)) throw e;
                            budgetHit = true;
                        }
                    }
                    if (paths && paths.length) {
                        // keep the run1<->run2 bindings the order relation established
                    }
                    else {
                        bijAA.rollback(mAA);
                        paths = null;
                    }
                }
                if (paths) {
                    const m3 = bij.mark();
                    const dq = comparePayload(a.req, b.req, bij, true);
                    if (dq) d = { ...dq, in: "request" };
                    else if (b.res?.json === undefined) d = { path: "$response", a: "json", b: "non-json" };
                    else {
                        try {
                            const reordered = reorderCandidate(a.res.json, b.res.json, paths, bij, budget);
                            d = reordered === undefined
                                ? { path: `${paths.join(",")} (order-relaxed)`, a: "an oracle element", b: "no equal candidate element under the process handle mapping (exhaustive search)" }
                                : diff(a.res.json, reordered, bij);
                        }
                        catch (e) {
                            if (!(e instanceof BudgetExceeded)) throw e;
                            d = { path: `${paths.join(",")} (order-relaxed)`, a: `more than ${MATCH_BUDGET} element comparisons`, b: "matching budget exceeded", inconclusive: true };
                        }
                    }
                    if (d) bij.rollback(m3);
                    else {
                        (c.unorderedPaths ??= new Set()).add(paths.join(","));
                        branch = "A"; // accepted on run 1's handle mapping
                    }
                    outcome = !d ? "unordered" : d.inconclusive ? "inconclusive" : "differ";
                    if (d?.inconclusive) c.budgetInconclusive = (c.budgetInconclusive ?? 0) + 1;
                }
                else {
                    outcome = budgetHit ? "inconclusive" : "unstable";
                    if (budgetHit) {
                        d = { path: "(oracle order relation)", a: `more than ${MATCH_BUDGET} element comparisons`, b: "matching budget exceeded", inconclusive: true };
                        c.budgetInconclusive = (c.budgetInconclusive ?? 0) + 1;
                    }
                }
            }
            else if (!oraclesAgree) {
                // Neither run viable and the oracle runs disagree by kind: the candidate matches no observed variant.
                outcome = "unstable";
            }
            else outcome = "differ";

            const took = Date.now() - started;
            if (took > 1000) slowExchanges.push({ test, process: i, exchange: k, method: a.method, ms: took, outcome });
            if (outcome === "differ") {
                c.differ++;
                if (c.examples.length < 3) c.examples.push({ test, process: i, exchange: k, mode: pa.mode, batched: !!a.batched, diff: truncate(d) });
            }
            else if (outcome === "unstable" || outcome === "inconclusive") {
                c.unverified = (c.unverified ?? 0) + 1;
                if (outcome === "unstable") c.oracleUnstable = (c.oracleUnstable ?? 0) + 1;
                else if (d?.path === "$sha256") c.hashInconclusive = (c.hashInconclusive ?? 0) + 1;
                if ((c.unverifiedExamples ??= []).length < 3) c.unverifiedExamples.push({ test, exchange: k, outcome, diff: truncate(d) });
            }
            else {
                c.equal++;
                if (outcome === "run2") c.matchesOracleRun2 = (c.matchesOracleRun2 ?? 0) + 1;
                if (outcome === "unordered") c.equalUnordered = (c.equalUnordered ?? 0) + 1;
                // The verified exchange is the candidate's: a success answer equal to an oracle run's success answer.
                if (b.kind === "response") {
                    c.okEqual++;
                    c[`okEqual_${pa.mode}`] = (c[`okEqual_${pa.mode}`] ?? 0) + 1;
                    if (outcome === "strict") c.strictEqual = (c.strictEqual ?? 0) + 1;
                }
                if (a.res?.hash) c.hashOnlyEqual = (c.hashOnlyEqual ?? 0) + 1;
            }
            if (b.kind === "response") c.okPairs++;
        }
        // Fail closed: oracle exchanges the candidate never answered (diverged or shorter process) stay unverified.
        for (let j = k; j < ea.length; j++) {
            const c = counter(ea[j].method);
            c.unverified = (c.unverified ?? 0) + 1;
            c.unpairedExchanges = (c.unpairedExchanges ?? 0) + 1;
        }
        // Fail closed symmetrically (N3): candidate exchanges the oracle never made are unverified too; with an
        // identical client, surplus candidate traffic means the server behaved differently.
        for (let j = k; j < eb.length; j++) {
            const c = counter(eb[j].method);
            c.unverified = (c.unverified ?? 0) + 1;
            c.surplusExchanges = (c.surplusExchanges ?? 0) + 1;
        }
        if (ea.length !== eb.length) unpaired.push({ test, index: i, lengths: [ea.length, eb.length] });
        const ca = [...xa.calls].sort();
        const cb = [...xb.calls].sort();
        if (JSON.stringify(ca) !== JSON.stringify(cb)) callDiffs.push({ test, index: i, onlyA: ca.filter(([k, v]) => xb.calls.get(k) !== v).slice(0, 5).map(([k, v]) => `${v}x ${k.slice(0, 160)}`), onlyB: cb.filter(([k, v]) => xa.calls.get(k) !== v).slice(0, 5).map(([k, v]) => `${v}x ${k.slice(0, 160)}`) });
    }
}
for (const [test, listB] of B.byTest) {
    if (A.byTest.has(test) || todoTests.has(test)) continue;
    unpaired.push({ test, missingIn: opts.a });
    for (const pb of listB) {
        for (const e of exchanges(pb, B.treeDir).out.flatMap(expand)) {
            const c = counter(e.method);
            c.unverified = (c.unverified ?? 0) + 1;
            c.surplusExchanges = (c.surplusExchanges ?? 0) + 1;
        }
    }
}

function truncate(d) {
    const s = v => {
        const j = JSON.stringify(v);
        return j && j.length > 300 ? `${j.slice(0, 300)}...` : v;
    };
    return { ...d, a: s(d.a), b: s(d.b) };
}

const proto = fs.readFileSync(path.join(here, "..", "..", "ts-ref", "tsc", "internal", "api", "proto.go"), "utf8");
const methods = [...proto.matchAll(/^\s*Method\w+\s+Method\s*=\s*"([^"]+)"/gm)].map(m => m[1]);
// Methods the candidate rejects as explicitly unsupported (never answered successfully) form their own category.
const unsupportedInB = new Set();
const answeredInB = new Set();
for (const list of B.byTest.values()) {
    for (const proc of list) {
        for (const r of proc.records) {
            if (r.kind === "response" && r.dir === "s2c") answeredInB.add(r.method);
            if (r.kind === "error" && /unsupported|not implemented by tsrs/i.test(r.data ?? "")) unsupportedInB.add(r.method);
        }
    }
}
const rows = methods.map(m => {
    const c = perMethod.get(m);
    let status;
    if (unsupportedInB.has(m) && !answeredInB.has(m)) status = "unsupported";
    else if (!c) status = "unpaired";
    else if (c.differ > 0) status = "differs";
    // Any exchange that could not be verified (oracle disagreement the candidate does not resolve, unattributable
    // hash difference, missing/diverged candidate traffic) keeps the method inconclusive even if others are equal.
    else if ((c.unverified ?? 0) > 0) status = "inconclusive";
    else if (c.okEqual > 0) status = "equal";
    else status = "equal-errors-only";
    return { method: m, status, ...(c ?? {}), unorderedPaths: c?.unorderedPaths ? [...c.unorderedPaths] : undefined };
});
const report = {
    a: opts.a,
    b: opts.b,
    drift: opts.drift ?? null,
    driftAt: opts.driftAt ?? null,
    idKeys: [...ID_KEYS],
    binaryMethods: [...BINARY_METHODS],
    pairedProcesses,
    excludedTodoTests: [...todoTests],
    statusCounts: rows.reduce((acc, r) => ((acc[r.status] = (acc[r.status] ?? 0) + 1), acc), {}),
    unpairedCount: unpaired.length,
    callMultisetDiffs: callDiffs.length,
    slowExchanges,
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
