import { spawn } from "node:child_process";
import getExePath from "./getExePath.js";

let exe;
try {
    exe = getExePath();
}
catch (e) {
    console.error(`tsrs: ${e.message}`);
    process.exit(1);
}
const args = process.argv.slice(2);

// Node >= 22.15 / 23.11: replace this process with the binary, so exit codes and signals need no forwarding.
if (process.platform !== "win32" && typeof process.execve === "function") {
    try {
        process.execve(exe, [exe, ...args]);
    }
    catch {
        // Not permitted here (e.g. worker or sandbox); fall back to a child process.
    }
}

const child = spawn(exe, args, { stdio: "inherit" });

const forwarded = ["SIGINT", "SIGTERM", "SIGHUP", "SIGQUIT", "SIGBREAK"];
const handlers = new Map();
for (const signal of forwarded) {
    const handler = () => child.kill(signal);
    try {
        process.on(signal, handler);
        handlers.set(signal, handler);
    }
    catch {
        // Signal not supported on this platform.
    }
}

child.on("error", e => {
    console.error(`tsrs: failed to run ${exe}: ${e.message}`);
    process.exit(1);
});

child.on("exit", (code, signal) => {
    for (const [s, handler] of handlers) process.off(s, handler);
    if (signal) {
        // Die the same way the binary did, so callers see the signal rather than an exit code.
        process.kill(process.pid, signal);
        return;
    }
    process.exit(code ?? 1);
});
