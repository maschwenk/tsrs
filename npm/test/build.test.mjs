import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

function fixture(t) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-packaging-"));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    const copy = (from, to = from) => fs.cpSync(new URL(`../${from}`, import.meta.url), path.join(root, "npm", to), { recursive: true });
    copy("build.mjs");
    copy("platform");
    copy("tsrs/package.json");
    copy("tsrs/lib");
    copy("tsrs/bin");
    copy("tsrs-wasm");
    fs.copyFileSync(new URL("../../Cargo.toml", import.meta.url), path.join(root, "Cargo.toml"));
    fs.writeFileSync(path.join(root, "LICENSE"), "fixture license");
    fs.writeFileSync(path.join(root, "NOTICE"), "fixture notice");
    // Package metadata does not depend on the SDK compiler: keep this fixture independent of installed tools.
    fs.writeFileSync(path.join(root, "compiler.mjs"), `
        import fs from "node:fs";
        import path from "node:path";
        fs.mkdirSync(path.join(path.dirname(process.argv.at(-1)), "dist"), { recursive: true });
    `);
    return {
        root,
        artifact(triple) {
            const dir = path.join(root, "artifacts", triple);
            fs.mkdirSync(dir, { recursive: true });
            fs.writeFileSync(path.join(dir, "tsrs"), triple);
        },
        run(...args) {
            return spawnSync(process.execPath, ["npm/build.mjs", "--tsc", path.join(root, "compiler.mjs"), "--out", path.join(root, "dist"), ...args], { cwd: root, encoding: "utf8" });
        },
        json(name) {
            return JSON.parse(fs.readFileSync(path.join(root, "dist", name), "utf8"));
        },
    };
}

test("all release packages coexist with distinct names, binaries and libc metadata", t => {
    const f = fixture(t);
    const platforms = [
        ["aarch64-apple-darwin", "darwin-arm64", "darwin", "arm64", undefined],
        ["x86_64-unknown-linux-gnu", "linux-x64-gnu", "linux", "x64", "glibc"],
        ["aarch64-unknown-linux-gnu", "linux-arm64-gnu", "linux", "arm64", "glibc"],
        ["x86_64-unknown-linux-musl", "linux-x64-musl", "linux", "x64", "musl"],
        ["aarch64-unknown-linux-musl", "linux-arm64-musl", "linux", "arm64", "musl"],
    ];
    for (const [triple] of platforms) f.artifact(triple);
    const wasm = path.join(f.root, "module.wasm");
    fs.writeFileSync(wasm, Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]));
    const result = f.run("--artifacts", path.join(f.root, "artifacts"), "--wasm", wasm);
    assert.equal(result.status, 0, result.stderr);
    const manifest = f.json("packages.json");
    const main = f.json("tsrs/package.json");
    assert.equal(manifest.packages.length, 7);
    assert.equal(manifest.packages.at(-2).name, "tsrs");
    assert.equal(manifest.packages.at(-1).name, "@ts-rs/wasm");
    assert.equal(new Set(manifest.packages.map(pkg => pkg.dir)).size, 7);
    assert.deepEqual(Object.keys(main.optionalDependencies).sort(), platforms.map(([, name]) => `@ts-rs/${name}`).sort());
    for (const [triple, name, os, cpu, libc] of platforms) {
        const pkg = f.json(`tsrs-${name}/package.json`);
        assert.equal(pkg.name, `@ts-rs/${name}`);
        assert.equal(pkg.version, manifest.version);
        assert.equal(main.optionalDependencies[pkg.name], manifest.version);
        assert.deepEqual(pkg.os, [os]);
        assert.deepEqual(pkg.cpu, [cpu]);
        assert.deepEqual(pkg.libc, libc ? [libc] : undefined);
        assert.equal(fs.readFileSync(path.join(f.root, "dist", `tsrs-${name}`, "tsrs"), "utf8"), triple);
        const readme = fs.readFileSync(path.join(f.root, "dist", `tsrs-${name}`, "README.md"), "utf8");
        assert.ok(readme.includes(`\`${name}\``));
        assert.ok(!readme.includes("{{"));
    }
    assert.equal(f.json("wasm/package.json").version, manifest.version);
});

test("a partial build depends only on supplied targets and rejects a missing binary", t => {
    const f = fixture(t);
    f.artifact("x86_64-unknown-linux-musl");
    const result = f.run("--artifacts", path.join(f.root, "artifacts"));
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(Object.keys(f.json("tsrs/package.json").optionalDependencies), ["@ts-rs/linux-x64-musl"]);
    const missing = f.run("--binary", `aarch64-unknown-linux-musl=${path.join(f.root, "missing")}`);
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /binary for aarch64-unknown-linux-musl not found/);
});
