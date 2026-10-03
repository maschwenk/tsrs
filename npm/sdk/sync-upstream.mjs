#!/usr/bin/env node
// Copies the TypeScript 7 JS/TS API SDK (packages/typescript in microsoft/TypeScript) into npm/tsrs, from a checkout
// of exactly the commit pinned in Cargo.toml's [workspace.metadata.typescript]. The copied files are committed, so
// building the npm package needs no network or upstream checkout; rerun this after bumping the pinned commit.
//
//   node npm/sdk/sync-upstream.mjs [--ts-ref <dir>]   # copy (default checkout: ./ts-ref)
//   node npm/sdk/sync-upstream.mjs --check            # verify the committed copy matches the checkout
//
// Source files are copied byte for byte, except files listed in PATCHES, which get a deliberate tsrs patch from
// npm/sdk/patches applied (strictly: every hunk must match the pinned original exactly). Test files get the small, listed edits in editTest (package name, the
// places upstream hard-codes its repo build of the server, and the compiler fixture root). npm/tsrs/UPSTREAM.json records the commit and a
// sha256 of every copied file.

import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const npmDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(npmDir, "..");
const pkgDir = path.join(npmDir, "tsrs");
const UPSTREAM_PKG = "packages/typescript";
const UPSTREAM_NAME = "@typescript/typescript";

// Directories copied verbatim (relative to packages/typescript).
const VERBATIM_DIRS = ["src", "vendor"];
// Single files copied verbatim. lib/getExePath.d.ts types tsrs's own lib/getExePath.js, which has the same signature.
const VERBATIM_FILES = ["tsconfig.base.json", "tsconfig.json", "tsconfig.dev.json", "lib/getExePath.d.ts"];
// Deliberate tsrs changes to upstream sources, documented in each patch header and in npm/README.md.
const PATCHES = {
    "src/api/async/client.ts": "async-client-connection-loss.patch",
    "vendor/vscode-jsonrpc/lib/common/connection.js": "vscode-jsonrpc-send-request-write-error.patch",
};

// Test files copied with the editTest edits applied. Benchmarks are left out (they need tinybench and TypeScript 5),
// except ast.bench.ts, which ast.test.ts imports.
const TEST_FILES = [
    "test/testUtils.ts",
    "test/diagnosticFormatter.test.ts",
    "test/encoder.test.ts",
    "test/proto.test.ts",
    "test/scanner.test.ts",
    "test/spanMap.test.ts",
    "test/wtf8.test.ts",
    "test/sync/api.testUtils.ts",
    "test/sync/api.test.ts",
    "test/sync/api-generators.test.ts",
    "test/sync/ast.test.ts",
    "test/sync/ast.bench.ts",
    "test/sync/astnav.test.ts",
    "test/sync/shutdown.test.ts",
    "test/async/api.testUtils.ts",
    "test/async/api.test.ts",
    "test/async/astnav.test.ts",
];

function parseArgs(argv) {
    const opts = { tsRef: path.join(repoRoot, "ts-ref"), check: false };
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === "--ts-ref") opts.tsRef = path.resolve(argv[++i]);
        else if (argv[i] === "--check") opts.check = true;
        else throw new Error(`unknown argument ${argv[i]}`);
    }
    return opts;
}

function pinnedCommit() {
    const text = fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8");
    const table = text.split(/^\[workspace\.metadata\.typescript\]$/m)[1];
    const commit = table?.match(/^commit\s*=\s*"([0-9a-f]{40})"/m)?.[1];
    if (!commit) throw new Error("Cargo.toml has no [workspace.metadata.typescript] commit");
    return commit;
}

// The test edits, each required to match exactly the given number of times so upstream drift fails loudly.
function editTest(rel, text, packageName) {
    const edits = [];
    const count = (text.match(new RegExp(`["']${UPSTREAM_NAME}(/|["'])`, "g")) ?? []).length;
    if (count > 0) edits.push([new RegExp(`(["'])${UPSTREAM_NAME}(?=/|["'])`, "g"), `$1${packageName}`, count]);
    if (rel === "test/sync/astnav.test.ts" || rel === "test/async/astnav.test.ts") {
        // Upstream reads compiler fixtures from its repo root; here they come from the ignored pinned checkout.
        edits.push([/^const repoRoot = resolve\(import\.meta\.dirname!, "\.\.", "\.\.", "\.\.", "\.\."\);$/gm, `const repoRoot = resolve(import.meta.dirname!, "..", "..", "..", "..", "ts-ref");`, 1]);
    }
    if (rel === "test/sync/ast.test.ts" || rel === "test/sync/ast.bench.ts") {
        // Upstream points at its repo build (built/local/tsc); use the package's own binary resolution instead.
        edits.push([/^\s*tsserverPath: fileURLToPath\(new URL\(`\.\.\/\.\.\/\.\.\/\.\.\/built\/local\/tsc[^\n]*\n/gm, "", 1]);
    }
    for (const [pattern, replacement, expected] of edits) {
        const found = (text.match(pattern) ?? []).length;
        if (found !== expected) throw new Error(`${rel}: expected ${expected} match(es) of ${pattern}, found ${found}`);
        text = text.replace(pattern, replacement);
    }
    return text;
}

// Applies a unified diff to `text`. Every hunk's context and removed lines must match exactly at the stated
// line, so an upstream change under a patch fails loudly instead of being merged silently. Lines keep their
// original endings (the patch files are byte-exact against the pinned sources, CRLF included).
function applyPatch(rel, text, patchText) {
    const lines = text.split("\n");
    const patchLines = patchText.split("\n");
    let i = patchLines.findIndex(l => l.startsWith("--- "));
    if (i < 0 || !patchLines[i + 1]?.startsWith("+++ ")) throw new Error(`${rel}: patch has no ---/+++ header`);
    i += 2;
    let offset = 0;
    let hunks = 0;
    while (i < patchLines.length) {
        const header = patchLines[i].match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
        if (!header) {
            if (patchLines[i] === "") {
                i++;
                continue;
            }
            throw new Error(`${rel}: unexpected patch line ${i + 1}: ${JSON.stringify(patchLines[i])}`);
        }
        i++;
        const oldLines = [], newLines = [];
        while (i < patchLines.length && !patchLines[i].startsWith("@@")) {
            const line = patchLines[i++];
            if (line.startsWith(" ")) oldLines.push(line.slice(1)), newLines.push(line.slice(1));
            else if (line.startsWith("-")) oldLines.push(line.slice(1));
            else if (line.startsWith("+")) newLines.push(line.slice(1));
            else if (line.startsWith("\\")) continue;
            else if (line === "" && i === patchLines.length) break;
            else throw new Error(`${rel}: malformed hunk line ${i}: ${JSON.stringify(line)}`);
        }
        const start = Number(header[1]) - 1 + offset;
        const actual = lines.slice(start, start + oldLines.length);
        if (actual.length !== oldLines.length || actual.some((l, k) => l !== oldLines[k])) {
            throw new Error(`${rel}: hunk ${hunks + 1} (@@ -${header[1]}) does not match the pinned source`);
        }
        lines.splice(start, oldLines.length, ...newLines);
        offset += newLines.length - oldLines.length;
        hunks++;
    }
    if (hunks === 0) throw new Error(`${rel}: patch has no hunks`);
    return lines.join("\n");
}

function listFiles(dir, base = dir) {
    const out = [];
    for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name < b.name ? -1 : 1)) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) out.push(...listFiles(full, base));
        else if (entry.isFile()) out.push(path.relative(base, full).split(path.sep).join("/"));
    }
    return out;
}

function sha256(data) {
    return crypto.createHash("sha256").update(data).digest("hex");
}

function main() {
    const opts = parseArgs(process.argv.slice(2));
    const commit = pinnedCommit();
    const head = execFileSync("git", ["-C", opts.tsRef, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    if (head !== commit) throw new Error(`${opts.tsRef} is at ${head}, but Cargo.toml pins ${commit}`);
    const status = execFileSync("git", ["-C", opts.tsRef, "status", "--porcelain", "--", UPSTREAM_PKG], { encoding: "utf8" });
    if (status.trim()) throw new Error(`${opts.tsRef}/${UPSTREAM_PKG} has local changes:\n${status}`);

    const packageName = JSON.parse(fs.readFileSync(path.join(pkgDir, "package.json"), "utf8")).name;
    const upstreamDir = path.join(opts.tsRef, UPSTREAM_PKG);

    /** @type {Map<string, Buffer>} relative output path -> contents */
    const outputs = new Map();
    const patched = {};
    for (const dir of VERBATIM_DIRS) {
        for (const rel of listFiles(path.join(upstreamDir, dir))) {
            const file = `${dir}/${rel}`;
            let data = fs.readFileSync(path.join(upstreamDir, dir, rel));
            if (PATCHES[file]) {
                const patchFile = path.join(npmDir, "sdk", "patches", PATCHES[file]);
                const patchText = fs.readFileSync(patchFile, "utf8");
                const original = data;
                data = Buffer.from(applyPatch(file, original.toString("utf8"), patchText));
                patched[file] = { upstreamSha256: sha256(original), patch: `npm/sdk/patches/${PATCHES[file]}`, patchSha256: sha256(patchText) };
            }
            outputs.set(file, data);
        }
    }
    for (const file of Object.keys(PATCHES)) if (!patched[file]) throw new Error(`patched file ${file} is not in the pinned sources`);
    for (const rel of VERBATIM_FILES) outputs.set(rel, fs.readFileSync(path.join(upstreamDir, rel)));
    for (const rel of TEST_FILES) {
        const text = fs.readFileSync(path.join(upstreamDir, rel), "utf8");
        outputs.set(rel, Buffer.from(editTest(rel, text, packageName)));
    }

    const manifest = {
        comment: "Generated by npm/sdk/sync-upstream.mjs. Copied from microsoft/TypeScript packages/typescript (Apache-2.0).",
        repository: "https://github.com/microsoft/TypeScript",
        commit,
        path: UPSTREAM_PKG,
        editedFiles: TEST_FILES,
        patchedFiles: patched,
        files: Object.fromEntries([...outputs].sort(([a], [b]) => a < b ? -1 : 1).map(([rel, data]) => [rel, sha256(data)])),
    };
    const manifestText = JSON.stringify(manifest, undefined, 4) + "\n";

    const managedRoots = [...VERBATIM_DIRS, "test"];
    const existing = new Set(managedRoots.flatMap(root => fs.existsSync(path.join(pkgDir, root)) ? listFiles(path.join(pkgDir, root)).map(r => `${root}/${r}`) : []));
    // Files under test/ that tsrs adds itself are not upstream-managed.
    const ownTests = new Set([...existing].filter(rel => rel.startsWith("test/tsrs/")));

    if (opts.check) {
        const problems = [];
        for (const [rel, data] of outputs) {
            const file = path.join(pkgDir, rel);
            if (!fs.existsSync(file)) problems.push(`missing ${rel}`);
            else if (!fs.readFileSync(file).equals(data)) problems.push(`differs ${rel}`);
        }
        for (const rel of existing) if (!outputs.has(rel) && !ownTests.has(rel)) problems.push(`extra ${rel}`);
        const manifestFile = path.join(pkgDir, "UPSTREAM.json");
        if (!fs.existsSync(manifestFile) || fs.readFileSync(manifestFile, "utf8") !== manifestText) problems.push("differs UPSTREAM.json");
        if (problems.length) throw new Error(`npm/tsrs is out of sync with ${commit}:\n  ${problems.join("\n  ")}`);
        console.log(`npm/tsrs matches microsoft/TypeScript@${commit.slice(0, 12)} (${outputs.size} files)`);
        return;
    }

    for (const rel of existing) if (!outputs.has(rel) && !ownTests.has(rel)) fs.rmSync(path.join(pkgDir, rel));
    for (const [rel, data] of outputs) {
        const file = path.join(pkgDir, rel);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, data);
    }
    fs.writeFileSync(path.join(pkgDir, "UPSTREAM.json"), manifestText);
    console.log(`copied ${outputs.size} files from microsoft/TypeScript@${commit.slice(0, 12)}`);
}

try {
    main();
}
catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
}
