// Synthetic captures for compare-responses.mjs (format of proxy.mjs: <pid>.jsonl start record + <pid>.capture.jsonl).
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
const work = path.join(path.dirname(new URL(import.meta.url).pathname), "..", ".work");
export function run(label, procs) {
    const dir = path.join(work, label, "trace");
    fs.rmSync(path.join(work, label), { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    fs.mkdirSync(path.join(work, label, "tree"), { recursive: true });
    procs.forEach((p, n) => {
        const pid = String(1000 + n);
        fs.writeFileSync(path.join(dir, `${pid}.jsonl`), JSON.stringify({ kind: "start", t: n, mode: p.mode, testFile: "syn.test.ts", test: p.test }) + "\n");
        let seq = 0;
        const lines = p.ex.flatMap(([method, req, res, kind = "response", id]) => {
            const rec = (k, data) => {
                const s = typeof data === "string" ? data : JSON.stringify(data);
                return JSON.stringify({ seq: seq++, kind: k, method, ...(id !== undefined ? { id } : {}), enc: "utf8", data: s, len: Buffer.byteLength(s), sha256: crypto.createHash("sha256").update(s).digest("hex") });
            };
            const env = (body) => p.mode === "async" ? { jsonrpc: "2.0", id, ...(body) } : body;
            return [rec("request", p.mode === "async" ? { jsonrpc: "2.0", id, method, params: req } : req), rec(kind, p.mode === "async" ? (kind === "error" ? { jsonrpc: "2.0", id, error: res } : { jsonrpc: "2.0", id, result: res }) : res)];
        });
        fs.writeFileSync(path.join(dir, `${pid}.capture.jsonl`), lines.join("\n") + "\n");
    });
}
