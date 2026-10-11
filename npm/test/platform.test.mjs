import assert from "node:assert/strict";
import childProcess from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { getPlatformPackageName } from "../tsrs/lib/platform.js";

function probes(t, { ldd, report, version } = {}) {
    const readFileSync = fs.readFileSync;
    t.mock.method(fs, "readFileSync", (file, ...args) => {
        if (file !== "/usr/bin/ldd") return readFileSync(file, ...args);
        if (ldd === undefined) throw new Error("ldd missing");
        return ldd;
    });
    t.mock.method(process.report, "getReport", () => {
        if (report instanceof Error) throw report;
        return report;
    });
    t.mock.method(childProcess, "execFileSync", (command, args) => {
        assert.equal(command, "ldd");
        assert.deepEqual(args, ["--version"]);
        if (version instanceof Error) throw version;
        if (version === undefined) throw new Error("ldd unavailable");
        return version;
    });
}

for (const [name, options, suffix] of [
    ["GNU ldd file", { ldd: "#!/bin/bash\n# GNU libc" }, "gnu"],
    ["musl ldd file", { ldd: "#!/bin/sh\n/lib/ld-musl-x86_64.so.1" }, "musl"],
    ["GNU report", { report: { header: { glibcVersionRuntime: "2.35" } } }, "gnu"],
    ["musl report", { report: { sharedObjects: ["/lib/libc.musl-aarch64.so.1"] } }, "musl"],
    ["musl loader in report", { report: { sharedObjects: ["/lib/ld-musl-x86_64.so.1"] } }, "musl"],
    ["inconclusive report, GNU ldd version", { report: { header: {}, sharedObjects: [] }, version: "ldd (GNU libc) 2.35" }, "gnu"],
    ["disabled report, musl ldd version", { report: new Error("disabled"), version: "musl libc" }, "musl"],
    ["nonzero ldd with musl on stderr", { version: Object.assign(new Error("exit 1"), { stderr: "musl libc\nVersion 1.2.5" }) }, "musl"],
    ["unavailable probes", {}, "gnu"],
]) {
    test(`Linux libc selection: ${name}`, t => {
        const excludeNetwork = process.report.excludeNetwork;
        probes(t, options);
        for (const cpu of ["x64", "arm64"]) assert.equal(getPlatformPackageName("linux", cpu), `@ts-rs/linux-${cpu}-${suffix}`);
        assert.equal(process.report.excludeNetwork, excludeNetwork);
    });
}

test("non-Linux packages do not probe libc", t => {
    const probe = t.mock.method(fs, "readFileSync", () => { throw new Error("unexpected libc probe"); });
    assert.equal(getPlatformPackageName("darwin", "arm64"), "@ts-rs/darwin-arm64");
    assert.equal(getPlatformPackageName("win32", "x64"), "@ts-rs/win32-x64");
    assert.equal(probe.mock.callCount(), 0);
});

function resolverFixture(t, cpu = "x64") {
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-resolver-")));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    fs.cpSync(new URL("../tsrs/lib", import.meta.url), path.join(root, "lib"), { recursive: true });
    fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({
        name: "tsrs", type: "module", optionalDependencies: {
            [`@ts-rs/linux-${cpu}-gnu`]: "1.0.0",
            [`@ts-rs/linux-${cpu}-musl`]: "1.0.0",
        },
    }));
    return {
        root,
        install(suffix, binary = true) {
            const dir = path.join(root, "node_modules", "@ts-rs", `linux-${cpu}-${suffix}`);
            fs.mkdirSync(dir, { recursive: true });
            fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify({ name: `@ts-rs/linux-${cpu}-${suffix}` }));
            if (binary) fs.writeFileSync(path.join(dir, "tsrs"), suffix);
            return path.join(dir, "tsrs");
        },
        run(suffix, override = "", arch = cpu) {
            return childProcess.spawnSync(process.env.TSRS_TEST_NODE ?? process.execPath, ["--input-type=module", "-e", `
                import fs from "node:fs";
                Object.defineProperty(process, "platform", { value: "linux" });
                Object.defineProperty(process, "arch", { value: ${JSON.stringify(arch)} });
                const read = fs.readFileSync;
                fs.readFileSync = (file, ...args) => file === "/usr/bin/ldd" ? ${JSON.stringify(suffix === "musl" ? "musl" : "GNU libc")} : read(file, ...args);
                const { default: getExePath } = await import(${JSON.stringify(pathToFileURL(path.join(root, "lib", "getExePath.js")).href)});
                try { console.log(getExePath()); }
                catch (error) { console.error(error.message); process.exitCode = 1; }
            `], { encoding: "utf8", env: { ...process.env, TSRS_BINARY: override } });
        },
    };
}

test("the resolver selects the matching libc when both packages are present", t => {
    for (const cpu of ["x64", "arm64"]) {
        const fixture = resolverFixture(t, cpu);
        const gnu = fixture.install("gnu");
        const musl = fixture.install("musl");
        for (const [suffix, expected] of [["gnu", gnu], ["musl", musl]]) {
            const result = fixture.run(suffix);
            assert.equal(result.status, 0, result.stderr);
            assert.equal(result.stdout.trim(), expected);
        }
    }
});

test("a missing musl dependency does not silently select an installed GNU package", t => {
    const fixture = resolverFixture(t);
    fixture.install("gnu");
    const result = fixture.run("musl");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Cannot find the optional dependency @ts-rs\/linux-x64-musl/);
    assert.match(result.stderr, /was not installed/);
});

test("an unsupported architecture reports the supported package names", t => {
    const result = resolverFixture(t).run("gnu", "", "riscv64");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /This platform is not supported/);
    assert.match(result.stderr, /linux-x64-gnu, linux-x64-musl/);
});

test("an installed package without a binary is reported", t => {
    const fixture = resolverFixture(t);
    fixture.install("musl", false);
    const result = fixture.run("musl");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /linux-x64-musl is installed .* but has no tsrs/);
});

test("TSRS_BINARY takes precedence over platform packages and reports missing overrides", t => {
    const fixture = resolverFixture(t);
    const binary = path.join(fixture.root, "custom-tsrs");
    fs.writeFileSync(binary, "local binary");
    const result = fixture.run("musl", binary, "unsupported");
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.trim(), binary);
    const missing = fixture.run("musl", `${binary}-missing`);
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /TSRS_BINARY is set .* which does not exist/);
});
