#!/usr/bin/env node
// Assembles the npm packages for tsrs, the way TypeScript 7 ships its native compiler: one main package (Node launcher,
// `bin/tsrs`) whose `optionalDependencies` name one package per platform, each holding only that platform's binary.
//
//   node npm/build.mjs --binary aarch64-apple-darwin=target/release/tsrs --pack
//   node npm/build.mjs --artifacts dir/ --pack      # dir/<target triple>/tsrs[.exe], as the release workflow lays out
//   node npm/build.mjs --print-version          # also --print-typescript-commit, --check-tag <tag>
//
// The package name comes from npm/tsrs/package.json; platform packages are `<name>-<os>-<cpu>`. The version is
// `<workspace version>-ts<[workspace.metadata.typescript] version>` from the workspace Cargo.toml. The main package
// lists exactly the platforms given here, so a release never points at a platform package that was not published.

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
        "usage: node npm/build.mjs [--binary <triple>=<path>]... [--artifacts <dir>] [--out <dir>] [--pack]\n" +
            "                          [--check-tag <git tag>] [--print-version | --print-typescript-commit]\n" +
            `triples: ${Object.keys(TARGETS).join(", ")}`,
    );
    process.exit(message ? 2 : 0);
}

function parseArgs(argv) {
    const opts = { binaries: new Map(), out: path.join(npmDir, "dist"), pack: false };
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
            case "--out":
                opts.out = path.resolve(value());
                break;
            case "--pack":
                opts.pack = true;
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

function npmPack(dir, out) {
    const stdout = execFileSync("npm", ["pack", "--json", "--pack-destination", out], {
        cwd: dir,
        encoding: "utf8",
        shell: process.platform === "win32",
    });
    const [info] = JSON.parse(stdout);
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
    if (opts.binaries.size === 0) usage("no binaries given (--binary or --artifacts)");

    const template = JSON.parse(fs.readFileSync(path.join(npmDir, "tsrs", "package.json"), "utf8"));
    const name = template.name;
    const baseName = name.split("/").pop();
    const license = fs.readFileSync(path.join(repoRoot, "LICENSE"));
    const notice = fs.readFileSync(path.join(npmDir, "NOTICE.txt"));

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
        const platformName = `${name}-${os}-${cpu}`;
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

    const mainDir = path.join(opts.out, baseName);
    copyDir(path.join(npmDir, "tsrs"), mainDir);
    fs.chmodSync(path.join(mainDir, "bin", "tsrs"), 0o755);
    fs.writeFileSync(path.join(mainDir, "LICENSE"), license);
    fs.writeFileSync(path.join(mainDir, "NOTICE.txt"), notice);
    writeJson(path.join(mainDir, "package.json"), {
        ...template,
        version,
        tsrs: { typescriptVersion: versions.typescriptVersion, typescriptCommit: versions.typescriptCommit },
        optionalDependencies,
    });
    // Platform packages first: they must be published before the main package that depends on them.
    packageDirs.push(mainDir);

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
