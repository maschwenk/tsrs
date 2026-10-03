// Additional comparator controls (parity lane), next to the codec lane's scen.mjs / mk.mjs (kept verbatim).
// Same capture format as proxy.mjs. Each scenario writes syn-a / syn-a2 / syn-b under tools/node-api/.work.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { run } from "./mk.mjs";

const work = path.join(path.dirname(new URL(import.meta.url).pathname), "..", ".work");
const P = { snapshot: 1, project: "p" };
const sym = (id, name) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0 });
const scen = process.argv[2];

/** Raw writer for records the codec's mk.mjs does not produce (sync base64 / truncated payloads). */
function runRaw(label, procs) {
    const dir = path.join(work, label, "trace");
    fs.rmSync(path.join(work, label), { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    fs.mkdirSync(path.join(work, label, "tree"), { recursive: true });
    procs.forEach((p, n) => {
        const pid = String(2000 + n);
        fs.writeFileSync(path.join(dir, `${pid}.jsonl`), JSON.stringify({ kind: "start", t: n, mode: "sync", testFile: "syn.test.ts", test: p.test }) + "\n");
        let seq = 0;
        const lines = p.recs.map(r => {
            const rec = { seq: seq++, kind: r.kind, method: r.method, len: r.len ?? 0, sha256: r.sha256 ?? crypto.createHash("sha256").update(r.data ?? "").digest("hex"), enc: r.enc };
            if (r.enc !== "truncated") rec.data = r.data;
            return JSON.stringify(rec);
        });
        fs.writeFileSync(path.join(dir, `${pid}.capture.jsonl`), lines.join("\n") + "\n");
    });
}
const req = (method, params) => ({ kind: "request", method, enc: "utf8", data: JSON.stringify(params) });
const res = (method, value) => ({ kind: "response", method, enc: "utf8", data: JSON.stringify(value) });

if (scen === "ce6" || scen === "ce6-ctl") {
    // A user property literally named "__k@1" must be compared exactly (no counter-suffix renaming).
    const q = ["getPropertiesOfType", { ...P, type: 3 }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q, [sym(1, "__k@1")]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q, [sym(5, scen === "ce6" ? "__k@2" : "__k@1")]]] }]);
}
if (scen === "ce11-ctl") {
    // A unique ES symbol's escaped name embeds a server counter: "__@iterator@3" vs "__@iterator@9" is a renaming.
    const q = ["getPropertiesOfType", { ...P, type: 3 }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q, [sym(1, "__@iterator@3")]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q, [sym(5, "__@iterator@9")]]] }]);
}
if (scen === "ce7") {
    // Candidate process missing for one of two processes: the method must not be reported as verified equal.
    const ex = [["getTypeAtPosition", { ...P, file: "/f.ts", position: 3 }, { id: 7, flags: 1, value: null }]];
    run("syn-a", [{ mode: "sync", test: "t", ex }, { mode: "sync", test: "t2", ex }]);
    run("syn-b", [{ mode: "sync", test: "t", ex }]);
}
if (scen === "ce8") {
    // A >4 MiB payload whose hash differs (could be ids or real content) next to one equal exchange: inconclusive.
    const ok = [req("getSourceFileNames", P), res("getSourceFileNames", ["/a.ts"])];
    const big = h => [req("getSourceFile", { ...P, file: "/a.ts" }), { kind: "response", method: "getSourceFile", enc: "truncated", len: 5 << 20, sha256: h }];
    runRaw("syn-a", [{ test: "t", recs: [...ok, ...big("aa"), ...big("cc")] }]);
    runRaw("syn-b", [{ test: "t", recs: [...ok, ...big("aa"), ...big("dd")] }]);
}
if (scen === "ce9") {
    // Sync binary (msgpack bin) AST payloads, identical in both runs: base for the --drift binary control.
    const bin = Buffer.from("binary-ast-payload-0123456789abcdef").toString("base64");
    const rec = [req("getSourceFile", { ...P, file: "/a.ts" }), { kind: "response", method: "getSourceFile", enc: "base64", data: bin, len: 35 }];
    runRaw("syn-a", [{ test: "t", recs: rec }]);
    runRaw("syn-b", [{ test: "t", recs: rec }]);
}
if (scen === "ce10-ctl") {
    // Oracle runs disagree on the order of a 3-element result; the candidate returns a third order of the same
    // elements (same handles via the process bijection): accepted only as equal-unordered.
    const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
    const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...q2, [sym(1, "x"), sym(2, "y"), sym(3, "z")]]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...q2, [sym(3, "z"), sym(1, "x"), sym(2, "y")]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")], [...q2, [sym(6, "y"), sym(7, "z"), sym(5, "x")]]] }]);
}
if (scen === "n1c" || scen === "n1c-ctl") {
    // Mirror of the codec's n1b: oracle run 1 errored in exchange 1 (run 2 answered), the candidate matches run 2
    // and must be locked to it; in exchange 2 the candidate answers with run 1's value ("tx", correct per run 2: "ty").
    const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
    const tos = id => ["getTypeOfSymbol", { ...P, symbol: { kind: 1, id } }];
    const T = (id, v) => ({ id, flags: 128, value: v });
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, "boom", "error"], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, sym(2, "x")], [...tos(2), T(8, "ty")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")], [...tos(5), T(9, scen === "n1c" ? "tx" : "ty")]] }]);
}
