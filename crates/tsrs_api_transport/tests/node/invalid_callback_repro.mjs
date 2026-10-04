// Invalid readFile callback answers during a real createSnapshot (raw sync MessagePack client, so the
// client can send what the pinned client would refuse to). Prints the server's answer to the request.
//   node invalid_callback_repro.mjs <server-exe>
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const exe = process.argv[2];
const answers = {
    badKind: [2, '{"kind":"bogus"}'],
    numberValue: [2, '{"kind":"value","value":5}'],
    missingKind: [2, '{"value":"x"}'],
    loneSurrogate: [2, '{"kind":"value","value":"x\\ud800"}'],
    duplicateKind: [2, '{"kind":"value","kind":"missing"}'],
    malformed: [2, '{"kind":'],
    errorSentinel: [2, '{"kind":"error"}'],
    callError: [3, "Error: boom"],
};
const bin = b => b.length < 256 ? Buffer.concat([Buffer.from([0xc4, b.length]), b]) : Buffer.concat([Buffer.from([0xc5, b.length >> 8, b.length & 255]), b]);
const tuple = (t, m, p) => Buffer.concat([Buffer.from([0x93, t]), bin(Buffer.from(m)), bin(Buffer.from(p))]);

async function run(name) {
    const root = mkdtempSync(join(tmpdir(), "tsrs-badcb-"));
    const p = join(root, "p");
    mkdirSync(p);
    writeFileSync(join(p, "tsconfig.json"), JSON.stringify({ compilerOptions: { types: [] }, files: ["a.ts", "b.ts"] }));
    writeFileSync(join(p, "a.ts"), "export const a = 1;\n");
    writeFileSync(join(p, "b.ts"), "export const b = 2;\n");
    const child = spawn(exe, ["--api", "--cwd", root, "--callbacks=readFile", "--useCaseSensitiveFileNames=true"], { stdio: ["pipe", "pipe", "pipe"] });
    let buf = Buffer.alloc(0);
    let stderr = "";
    child.stderr.on("data", d => (stderr += d));
    const exited = new Promise(resolve => child.once("exit", (code, signal) => resolve(`EXIT ${code ?? signal}: ${stderr.split("\n").find(l => l.trim()) ?? ""}`.slice(0, 220))));
    const result = new Promise(resolve => {
        child.stdout.on("data", d => {
            buf = Buffer.concat([buf, d]);
            for (;;) {
                // parse one tuple if complete
                if (buf.length < 2) return;
                let i = 2;
                const rb = () => { const m = buf[i++]; let n; if (m === 0xc4) n = buf[i++]; else if (m === 0xc5) { n = buf.readUInt16BE(i); i += 2; } else { n = buf.readUInt32BE(i); i += 4; } if (i + n > buf.length) throw 0; const b = buf.subarray(i, i + n); i += n; return b; };
                let t = buf[1], m, pl;
                try { m = rb().toString(); pl = rb().toString(); } catch { return; }
                buf = buf.subarray(i);
                if (t === 6) {
                    const path = JSON.parse(pl);
                    const [rt, ans] = path.endsWith("/b.ts") ? answers[name] : [2, '{"kind":"useOS"}'];
                    child.stdin.write(tuple(rt, m, ans));
                } else {
                    resolve(`${t === 4 ? "Response" : "Error"}: ${pl.split("\n")[0].slice(0, 220)}`);
                }
            }
        });
    });
    child.stdin.write(tuple(1, "createSnapshot", JSON.stringify({ openProjects: [join(p, "tsconfig.json")] })));
    const r = await Promise.race([result, exited, new Promise(res => setTimeout(() => res("HANG (no response, process alive after 15 s)"), 15000))]);
    child.kill("SIGKILL");
    rmSync(root, { recursive: true, force: true });
    return r;
}
const out = {};
for (const name of Object.keys(answers)) out[name] = await run(name);
console.log(JSON.stringify(out, null, 1));
