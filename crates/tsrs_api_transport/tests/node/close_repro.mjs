// Close/cancel on a real API server: the client disconnects while the server is blocked in a
// filesystem callback (it never answers). Speaks the raw wire protocols so the client can vanish at an
// exact point. Reports how long the server takes to exit, its exit code, and whether it wrote anything
// besides protocol frames. Usage: node close_repro.mjs <server-exe> [timeoutMs]
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const exe = process.argv[2];
const timeout = Number(process.argv[3] ?? 20000);

function project() {
    const root = mkdtempSync(join(tmpdir(), "tsrs-close-"));
    const dir = join(root, "p");
    mkdirSync(dir);
    writeFileSync(join(dir, "tsconfig.json"), JSON.stringify({ compilerOptions: { types: [] }, files: ["a.ts"] }));
    writeFileSync(join(dir, "a.ts"), "export const a = 1;\n");
    return { root, config: join(dir, "tsconfig.json") };
}

function tuple(type, method, payload) {
    const m = Buffer.from(method), p = Buffer.from(payload);
    const bin = b => b.length < 256 ? Buffer.concat([Buffer.from([0xc4, b.length]), b])
        : b.length < 65536 ? Buffer.concat([Buffer.from([0xc5, b.length >> 8, b.length & 255]), b])
        : Buffer.concat([Buffer.from([0xc6]), Buffer.from(new Uint32Array([b.length]).buffer).reverse(), b]);
    return Buffer.concat([Buffer.from([0x93, type]), bin(m), bin(p)]);
}

function frame(obj) {
    const body = Buffer.from(JSON.stringify(obj));
    return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]);
}

async function run(mode, how) {
    const { root, config } = project();
    const args = ["--api", "--cwd", root, "--callbacks=readFile", "--useCaseSensitiveFileNames=true"];
    if (mode === "async") args.splice(1, 0, "--async");
    const child = spawn(exe, args, { stdio: ["pipe", "pipe", "pipe"] });
    let out = Buffer.alloc(0), err = "";
    child.stderr.on("data", d => err += d);
    const exited = new Promise(r => child.once("exit", (code, signal) => r({ code, signal })));
    const sawCallback = new Promise(resolve => {
        child.stdout.on("data", d => {
            out = Buffer.concat([out, d]);
            if (out.includes(Buffer.from("readFile"))) resolve();
        });
    });
    const params = JSON.stringify({ openProjects: [config] });
    child.stdin.write(mode === "async" ? frame({ jsonrpc: "2.0", id: 1, method: "createSnapshot", params: JSON.parse(params) }) : tuple(1, "createSnapshot", params));
    const reached = await Promise.race([sawCallback.then(() => true), new Promise(r => setTimeout(() => r(false), timeout))]);
    const start = Date.now();
    if (how === "eof") child.stdin.end();
    else child.stdout.destroy(), child.stdin.end(); // client vanishes entirely
    const result = await Promise.race([exited, new Promise(r => setTimeout(() => r(null), timeout))]);
    const ms = Date.now() - start;
    if (!result) child.kill("SIGKILL");
    rmSync(root, { recursive: true, force: true });
    const firstByteOk = mode === "async" ? out.subarray(0, 15).toString() === "Content-Length:" : out[0] === 0x93;
    return { mode, how, callbackReached: reached, exit: result ? `${result.code ?? result.signal}` : "HANG", ms, stdoutIsProtocol: firstByteOk, stderr: err.trim().split("\n").slice(-1)[0] ?? "" };
}

for (const mode of ["sync", "async"]) {
    for (const how of ["eof", "vanish"]) {
        console.log(JSON.stringify(await run(mode, how)));
    }
}
