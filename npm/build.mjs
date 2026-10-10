#!/usr/bin/env node
// Assembles the npm packages for tsrs, the way TypeScript 7 ships its native compiler: one main package (Node launcher,
// `bin/tsrs`) whose `optionalDependencies` name one package per platform, each holding only that platform's binary.
//
//   node npm/build.mjs --binary aarch64-apple-darwin=target/release/tsrs --pack
//   node npm/build.mjs --artifacts dir/ --pack      # dir/<target triple>/tsrs[.exe], as the release workflow lays out
//   node npm/build.mjs --wasm npm/tsrs-wasm/tsrs.wasm --pack   # the WebAssembly package (tools/wasm/build.sh)
//   node npm/build.mjs --print-version          # also --print-typescript-commit, --check-tag <tag>
//   node npm/build.mjs --sdk-only               # just compile the JS API (npm/tsrs/src -> npm/tsrs/dist)
//
// The main package also carries TypeScript 7's unstable JS API (`tsrs/unstable/sync`, `/async`, `/ast`, ...),
// copied from microsoft/TypeScript packages/typescript by npm/sdk/sync-upstream.mjs. It is compiled here with the
// build-only compiler in npm/package.json (`npm ci --prefix npm` first; `--tsc <path>` to use another TypeScript 7 tsc).
//
// The package name comes from npm/tsrs/package.json; platform packages are `@ts-rs/<os>-<cpu>`. The version is
// `<workspace version>-ts<[workspace.metadata.typescript] version>` from the workspace Cargo.toml, for every package
// including `@ts-rs/wasm` (npm/tsrs-wasm). The main package lists exactly the platforms given here, so a
// release never points at a platform package that was not published.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const npmDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(npmDir, "..");

// Rust target triple -> Node's process.platform / process.arch.
const TARGETS = {
    "aarch64-apple-darwin": { os: "darwin", cpu: "arm64" },
    "x86_64-apple-darwin": { os: "darwin", cpu: "x64" },
    "x86_64-unknown-linux-gnu": { os: "linux", cpu: "x64", libc: "glibc" },
    "aarch64-unknown-linux-gnu": { os: "linux", cpu: "arm64", libc: "glibc" },
    "x86_64-pc-windows-msvc": { os: "win32", cpu: "x64" },
};

function usage(message) {
    if (message) console.error(`error: ${message}\n`);
    console.error(
        "usage: node npm/build.mjs [--binary <triple>=<path>]... [--artifacts <dir>] [--out <dir>] [--pack] [--tsc <path>]\n" +
            "                          [--wasm <tsrs.wasm>] [--sdk-only]\n" +
            "                          [--check-tag <git tag>] [--print-version | --print-typescript-commit]\n" +
            `triples: ${Object.keys(TARGETS).join(", ")}`,
    );
    process.exit(message ? 2 : 0);
}

function parseArgs(argv) {
    const opts = { binaries: new Map(), out: path.join(npmDir, "dist"), pack: false, sdkOnly: false, tsc: undefined };
    for (let i = 0; i < argv.length; i++) {
        const arg = argv[i];
        const value = () => argv[++i] ?? usage(`${arg} needs a value`);
        switch (arg) {
            case "--binary": {
                const [triple, file] = value().split(/=(.*)/s);
                if (!TARGETS[triple]) usage(`unknown target triple ${triple}`);
                if (!file) usage(`--binary needs <triple>=<path>`);
                opts.binaries.set(triple, path.resolve(file));
                break;
            }
            case "--artifacts": {
                const dir = path.resolve(value());
                for (const triple of Object.keys(TARGETS)) {
                    for (const exe of ["tsrs", "tsrs.exe"]) {
                        const file = path.join(dir, triple, exe);
                        if (fs.existsSync(file)) opts.binaries.set(triple, file);
                    }
                }
                break;
            }
            case "--wasm":
                opts.wasm = path.resolve(value());
                break;
            case "--out":
                opts.out = path.resolve(value());
                break;
            case "--pack":
                opts.pack = true;
                break;
            case "--sdk-only":
                opts.sdkOnly = true;
                break;
            case "--tsc":
                opts.tsc = path.resolve(value());
                break;
            case "--check-tag":
                opts.checkTag = value();
                break;
            case "--print-version":
                opts.print = "version";
                break;
            case "--print-typescript-commit":
                opts.print = "typescriptCommit";
                break;
            case "-h":
            case "--help":
                usage();
            default:
                usage(`unknown argument ${arg}`);
        }
    }
    return opts;
}

// Minimal reader for the two tables we need from the workspace Cargo.toml.
function readCargoVersions() {
    const text = fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8");
    const tables = {};
    let table = "";
    for (const raw of text.split("\n")) {
        const line = raw.replace(/#.*$/, "").trim();
        const header = line.match(/^\[([^\]]+)\]$/);
        if (header) {
            table = header[1];
            continue;
        }
        const kv = line.match(/^([\w-]+)\s*=\s*"([^"]*)"$/);
        if (kv) (tables[table] ??= {})[kv[1]] = kv[2];
    }
    const crateVersion = tables["workspace.package"]?.version;
    const ts = tables["workspace.metadata.typescript"];
    if (!crateVersion || !ts?.version || !ts?.commit) {
        throw new Error("Cargo.toml needs [workspace.package] version and [workspace.metadata.typescript] version/commit");
    }
    return { crateVersion, version: `${crateVersion}-ts${ts.version}`, typescriptVersion: ts.version, typescriptCommit: ts.commit };
}

function writeJson(file, value) {
    fs.writeFileSync(file, JSON.stringify(value, undefined, 4) + "\n");
}

function copyDir(src, dst) {
    fs.mkdirSync(dst, { recursive: true });
    for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
        const from = path.join(src, entry.name);
        const to = path.join(dst, entry.name);
        if (entry.isDirectory()) copyDir(from, to);
        else fs.copyFileSync(from, to);
    }
}

// Compiles npm/tsrs/src (the TypeScript 7 JS API) into npm/tsrs/dist with `tsc -b`, like upstream's
// packages/typescript build. The output dir must stay npm/tsrs/dist: the `#enums/*` import map points its types at
// dist, which tsc maps back to the sources of the project being built.
function buildSdk(tscPath) {
    const pkgDir = path.join(npmDir, "tsrs");
    const tsc = tscPath ?? path.join(npmDir, "node_modules", "typescript", "bin", "tsc");
    if (!fs.existsSync(tsc)) {
        throw new Error(`the JS API build compiler is not installed (${path.relative(process.cwd(), tsc)}); run \`npm ci --prefix npm\` or pass --tsc`);
    }
    fs.rmSync(path.join(pkgDir, "dist"), { recursive: true, force: true });
    fs.rmSync(path.join(pkgDir, "tsconfig.tsbuildinfo"), { force: true });
    const isJs = /\.[cm]?js$/.test(tsc);
    execFileSync(isJs ? process.execPath : tsc, [...(isJs ? [tsc] : []), "-b", path.join(pkgDir, "tsconfig.json")], { stdio: "inherit" });

    // Like upstream: published declarations may only import relative paths or the package's own `#` imports.
    const errors = [];
    const walk = dir => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const file = path.join(dir, entry.name);
            if (entry.isDirectory()) walk(file);
            else if (entry.name.endsWith(".d.ts")) {
                for (const [i, line] of fs.readFileSync(file, "utf8").split("\n").entries()) {
                    const specs = [
                        line.match(/(?:import|export)\s.*?\sfrom\s+["']([^"']+)["']/)?.[1],
                        ...[...line.matchAll(/import\(["']([^"']+)["']\)/g)].map(m => m[1]),
                    ];
                    for (const spec of specs) {
                        if (spec && !spec.startsWith(".") && !spec.startsWith("#") && !spec.startsWith("node:")) {
                            errors.push(`${path.relative(pkgDir, file)}:${i + 1}: external import "${spec}"`);
                        }
                    }
                }
            }
        }
    };
    walk(path.join(pkgDir, "dist"));
    if (errors.length) throw new Error(`external imports in the JS API declarations:\n  ${errors.join("\n  ")}`);
}

// The `@typescript/source` export/import conditions point at .ts sources, which are not published.
function stripSourceConditions(value) {
    if (Array.isArray(value)) return value.map(stripSourceConditions);
    if (value === null || typeof value !== "object") return value;
    return Object.fromEntries(
        Object.entries(value).filter(([key]) => key !== "@typescript/source").map(([key, v]) => [key, stripSourceConditions(v)]),
    );
}

// `@ts-rs/wasm`: npm/tsrs-wasm's published files, the module tools/wasm/build.sh wrote, LICENSE and NOTICE,
// at the native packages' version (the module's `--version` prints the same, from crates/tsrs_execute/build.rs).
function stageWasm(module, out, versions, license, notice) {
    const srcDir = path.join(npmDir, "tsrs-wasm");
    const template = JSON.parse(fs.readFileSync(path.join(srcDir, "package.json"), "utf8"));
    const bytes = fs.readFileSync(module);
    if (bytes.subarray(0, 4).toString("latin1") !== "\0asm") throw new Error(`not a WebAssembly module: ${module}`);
    const dir = path.join(out, template.name.split("/").pop());
    fs.mkdirSync(dir, { recursive: true });
    for (const entry of ["README.md", ...template.files]) {
        if (entry === "tsrs.wasm" || entry === "NOTICE.txt") continue;
        // A missing file throws: the package would install but fail to load.
        const from = path.join(srcDir, entry);
        if (fs.statSync(from).isDirectory()) copyDir(from, path.join(dir, entry));
        else fs.copyFileSync(from, path.join(dir, entry));
    }
    for (const bin of Object.values(template.bin ?? {})) fs.chmodSync(path.join(dir, bin), 0o755);
    fs.writeFileSync(path.join(dir, "tsrs.wasm"), bytes);
    fs.writeFileSync(path.join(dir, "LICENSE"), license);
    fs.writeFileSync(path.join(dir, "NOTICE.txt"), notice);
    writeJson(path.join(dir, "package.json"), {
        ...template,
        version: versions.version,
        tsrs: { typescriptVersion: versions.typescriptVersion, typescriptCommit: versions.typescriptCommit },
    });
    return dir;
}

function npmPack(dir, out) {
    const stdout = execFileSync("npm", ["pack", "--json", "--pack-destination", out], {
        cwd: dir,
        encoding: "utf8",
        shell: process.platform === "win32",
    });
    // npm 12 keys the results by package name; earlier versions return an array.
    const [info] = Object.values(JSON.parse(stdout));
    return path.join(out, info.filename);
}

function main() {
    const opts = parseArgs(process.argv.slice(2));
    const versions = readCargoVersions();
    const { version } = versions;

    if (opts.checkTag !== undefined) {
        const tag = opts.checkTag.replace(/^refs\/tags\//, "");
        if (tag !== `v${version}` && tag !== `v${versions.crateVersion}`) {
            throw new Error(`git tag ${tag} does not match the package version: expected v${versions.crateVersion} or v${version}`);
        }
    }
    if (opts.print) {
        console.log(versions[opts.print]);
        return;
    }
    if (opts.checkTag !== undefined && opts.binaries.size === 0 && !opts.wasm) return;
    if (opts.sdkOnly) {
        buildSdk(opts.tsc);
        return;
    }
    if (opts.binaries.size === 0 && !opts.wasm) usage("no binaries given (--binary, --artifacts or --wasm)");
    if (opts.binaries.size > 0) buildSdk(opts.tsc);

    const template = JSON.parse(fs.readFileSync(path.join(npmDir, "tsrs", "package.json"), "utf8"));
    const name = template.name;
    const baseName = name.split("/").pop();
    const license = fs.readFileSync(path.join(repoRoot, "LICENSE"));
    const notice = fs.readFileSync(path.join(repoRoot, "NOTICE"));

    fs.rmSync(opts.out, { recursive: true, force: true });
    fs.mkdirSync(opts.out, { recursive: true });

    const common = {
        version,
        license: template.license,
        repository: template.repository,
        engines: template.engines,
        publishConfig: template.publishConfig,
    };

    const packageDirs = [];
    const optionalDependencies = {};
    for (const [triple, binary] of [...opts.binaries].sort()) {
        if (!fs.existsSync(binary)) throw new Error(`binary for ${triple} not found: ${binary}`);
        const { os, cpu, libc } = TARGETS[triple];
        const platformName = `@ts-rs/${os}-${cpu}`;
        const dir = path.join(opts.out, `${baseName}-${os}-${cpu}`);
        fs.mkdirSync(dir, { recursive: true });

        const exeName = os === "win32" ? "tsrs.exe" : "tsrs";
        fs.copyFileSync(binary, path.join(dir, exeName));
        fs.chmodSync(path.join(dir, exeName), 0o755);
        fs.writeFileSync(path.join(dir, "LICENSE"), license);
        fs.writeFileSync(path.join(dir, "NOTICE.txt"), notice);
        const readme = fs.readFileSync(path.join(npmDir, "platform", "README.md"), "utf8")
            .replaceAll("{{name}}", platformName)
            .replaceAll("{{mainName}}", name)
            .replaceAll("{{os}}", os)
            .replaceAll("{{cpu}}", cpu)
            .replaceAll("{{version}}", version)
            .replaceAll("{{commit}}", versions.typescriptCommit.slice(0, 12));
        fs.writeFileSync(path.join(dir, "README.md"), readme);
        writeJson(path.join(dir, "package.json"), {
            name: platformName,
            ...common,
            description: `The ${os}-${cpu} binary for ${name}`,
            preferUnplugged: true,
            files: [exeName, "NOTICE.txt"],
            exports: { "./package.json": "./package.json" },
            os: [os],
            cpu: [cpu],
            ...(libc ? { libc: [libc] } : {}),
        });
        optionalDependencies[platformName] = version;
        packageDirs.push(dir);
    }

    if (opts.binaries.size > 0) {
        const mainDir = path.join(opts.out, baseName);
        // Only what the package's `files` publishes (plus package.json/README); the .ts sources and tests stay behind.
        fs.mkdirSync(mainDir, { recursive: true });
        for (const entry of ["README.md", "UPSTREAM.json", ...template.files]) {
            const from = path.join(npmDir, "tsrs", entry);
            if (!fs.existsSync(from)) continue;
            if (fs.statSync(from).isDirectory()) copyDir(from, path.join(mainDir, entry));
            else fs.copyFileSync(from, path.join(mainDir, entry));
        }
        fs.chmodSync(path.join(mainDir, "bin", "tsrs"), 0o755);
        fs.writeFileSync(path.join(mainDir, "LICENSE"), license);
        fs.writeFileSync(path.join(mainDir, "NOTICE.txt"), notice);
        writeJson(path.join(mainDir, "package.json"), {
            ...stripSourceConditions(template),
            version,
            tsrs: { typescriptVersion: versions.typescriptVersion, typescriptCommit: versions.typescriptCommit },
            optionalDependencies,
        });
        // Platform packages first: they must be published before the main package that depends on them.
        packageDirs.push(mainDir);
    }
    // The WebAssembly package depends on nothing, so it goes last.
    if (opts.wasm) packageDirs.push(stageWasm(opts.wasm, opts.out, versions, license, notice));

    console.log(`version ${version}`);
    const manifest = [];
    for (const dir of packageDirs) {
        const entry = { name: JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8")).name, dir };
        if (opts.pack) entry.tarball = npmPack(dir, opts.out);
        console.log(`${opts.pack ? "packed" : "staged"} ${path.relative(process.cwd(), entry.tarball ?? dir)}`);
        manifest.push(entry);
    }
    // Publish order for the release workflow.
    writeJson(path.join(opts.out, "packages.json"), { version, packages: manifest });
}

try {
    main();
}
catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
}
