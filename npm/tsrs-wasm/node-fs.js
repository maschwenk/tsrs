// The real file system as a HostFileSystem for core.js, matching native tsrs's osvfs: write errors are Rust's
// `io::Error` texts ("<strerror> (os error N)"), realpath is the kernel's on Linux and a symlink walk elsewhere,
// writes create missing parent directories, remove is recursive and ignores a missing path.

import fs from "node:fs";
import path from "node:path";
import { NotFoundError } from "./core.js";

const STRERROR = {
    EACCES: "Permission denied",
    ENOENT: "No such file or directory",
    ENOTDIR: "Not a directory",
    EISDIR: "Is a directory",
    EEXIST: "File exists",
    ENOSPC: "No space left on device",
    EROFS: "Read-only file system",
    ELOOP: "Too many levels of symbolic links",
    ENAMETOOLONG: "File name too long",
    EPERM: "Operation not permitted",
    ENOTEMPTY: "Directory not empty",
    EBUSY: "Resource busy",
};

/** Rust's `io::Error` display for a Node error. */
export function osErrorText(e) {
    if (typeof e?.errno === "number" && STRERROR[e.code]) {
        return `${STRERROR[e.code]} (os error ${-e.errno})`;
    }
    return String(e?.message ?? e);
}

function rethrow(e) {
    throw new Error(osErrorText(e));
}

const realpath = process.platform === "linux" ? fs.realpathSync.native : fs.realpathSync;

function write(p, data, append) {
    if (append) fs.appendFileSync(p, data);
    else fs.writeFileSync(p, data);
}

/** Whether the file system holding Node's executable is case-insensitive (osvfs probes its own executable). */
export function isCaseInsensitive() {
    const exe = process.execPath;
    const swapped = [...exe].map((c) => (c === c.toUpperCase() ? c.toLowerCase() : c.toUpperCase())).join("");
    return swapped !== exe && fs.existsSync(swapped);
}

export function nodeFileSystem() {
    return {
        readFile(p) {
            try {
                return fs.readFileSync(p);
            } catch (e) {
                if (e.code === "ENOENT") throw new NotFoundError(p);
                rethrow(e);
            }
        },
        stat(p) {
            let s;
            try {
                s = fs.statSync(p, { bigint: true, throwIfNoEntry: false });
            } catch (e) {
                rethrow(e);
            }
            if (!s) throw new NotFoundError(p);
            return { kind: s.isDirectory() ? "d" : s.isFile() ? "f" : "o", size: s.size, mtimeNs: s.mtimeNs };
        },
        readDir(p) {
            let entries;
            try {
                entries = fs.readdirSync(p, { withFileTypes: true });
            } catch (e) {
                if (e.code === "ENOENT") throw new NotFoundError(p);
                rethrow(e);
            }
            return entries.map((d) => [d.isDirectory() ? "d" : d.isFile() ? "f" : d.isSymbolicLink() ? "l" : "o", d.name]);
        },
        realpath(p) {
            return realpath(p);
        },
        writeFile(p, data, append) {
            try {
                write(p, data, append);
                return;
            } catch {
                // As osvfs: create the directory, then write again; errors come from those two steps.
            }
            try {
                fs.mkdirSync(path.dirname(p), { recursive: true });
                write(p, data, append);
            } catch (e) {
                rethrow(e);
            }
        },
        remove(p) {
            let s;
            try {
                s = fs.lstatSync(p, { throwIfNoEntry: false });
                if (!s) return;
                // unlink for a symlink: rmSync throws EISDIR on a symlink to a directory (Node 24).
                if (s.isDirectory()) fs.rmSync(p, { recursive: true, force: true });
                else fs.unlinkSync(p);
            } catch (e) {
                rethrow(e);
            }
        },
        chtimes(p, atimeNs, mtimeNs) {
            try {
                fs.utimesSync(p, Number(atimeNs) / 1e9, Number(mtimeNs) / 1e9);
            } catch (e) {
                rethrow(e);
            }
        },
    };
}
