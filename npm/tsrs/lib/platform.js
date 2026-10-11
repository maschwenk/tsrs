import childProcess from "node:child_process";
import fs from "node:fs";

// Rust triples and npm's os/cpu/libc fields. Keep package construction and binary lookup on the same naming scheme.
export const TARGETS = {
    "aarch64-apple-darwin": { os: "darwin", cpu: "arm64" },
    "x86_64-apple-darwin": { os: "darwin", cpu: "x64" },
    "x86_64-unknown-linux-gnu": { os: "linux", cpu: "x64", libc: "glibc" },
    "aarch64-unknown-linux-gnu": { os: "linux", cpu: "arm64", libc: "glibc" },
    "x86_64-unknown-linux-musl": { os: "linux", cpu: "x64", libc: "musl" },
    "aarch64-unknown-linux-musl": { os: "linux", cpu: "arm64", libc: "musl" },
    "x86_64-pc-windows-msvc": { os: "win32", cpu: "x64" },
};

export function platformName({ os, cpu, libc }) {
    return `${os}-${cpu}${libc ? `-${libc === "glibc" ? "gnu" : libc}` : ""}`;
}

// The NAPI-RS loader used by Oxc probes ldd, then Node's report, then a child process. Unknown hosts default to GNU.
function isMusl() {
    try {
        return fs.readFileSync("/usr/bin/ldd", "utf8").includes("musl");
    }
    catch {
        // ldd is not always installed at /usr/bin/ldd.
    }

    if (typeof process.report?.getReport === "function") {
        const excludeNetwork = process.report.excludeNetwork;
        try {
            process.report.excludeNetwork = true;
            const report = process.report.getReport();
            if (report?.header?.glibcVersionRuntime) return false;
            if (report?.sharedObjects?.some(file => file.includes("libc.musl-") || file.includes("ld-musl-"))) return true;
        }
        catch {
            // A disabled report or one with no libc evidence leaves the final ldd probe.
        }
        finally {
            process.report.excludeNetwork = excludeNetwork;
        }
    }

    try {
        return childProcess.execFileSync("ldd", ["--version"], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).includes("musl");
    }
    catch (error) {
        // musl's ldd can print its version to stderr and exit nonzero.
        return `${error.stdout ?? ""}${error.stderr ?? ""}`.includes("musl");
    }
}

export function getPlatformPackageName(os = process.platform, cpu = process.arch) {
    const libc = os === "linux" ? (isMusl() ? "musl" : "glibc") : undefined;
    return `@ts-rs/${platformName({ os, cpu, libc })}`;
}
