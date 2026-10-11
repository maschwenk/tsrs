import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { getPlatformPackageName } from "../tsrs/lib/platform.js";

// Override registry references with local tarballs, but install only the main package so pnpm applies os/cpu/libc
// filtering to its optional dependencies. Rebase manifest paths: release artifacts can move between runners.
export function installConsumer(dist, dir, env) {
    const manifest = JSON.parse(fs.readFileSync(path.join(dist, "packages.json"), "utf8"));
    const main = manifest.packages.find(pkg => pkg.name === "tsrs");
    const platforms = manifest.packages.filter(pkg => pkg.name.startsWith("@ts-rs/") && pkg.name !== "@ts-rs/wasm");
    const tarball = pkg => {
        if (!pkg?.tarball) throw new Error("packages.json has no tarballs; run npm/build.mjs with --pack");
        const file = path.join(dist, path.basename(pkg.tarball));
        if (!fs.existsSync(file)) throw new Error(`missing tarball: ${file}`);
        return file;
    };
    const expected = getPlatformPackageName();
    if (!platforms.some(pkg => pkg.name === expected)) throw new Error(`packages.json has no package for this host: ${expected}`);
    fs.writeFileSync(path.join(dir, "pnpm-workspace.yaml"), [
        "supportedArchitectures:",
        "  os: [current]",
        "  cpu: [current]",
        "  libc: [current]",
        "overrides:",
        ...platforms.map(pkg => `  ${JSON.stringify(pkg.name)}: ${JSON.stringify(`file:${tarball(pkg)}`)}`),
        "",
    ].join("\n"));
    execFileSync("pnpm", ["add", "--offline", tarball(main)], { cwd: dir, stdio: "inherit", env, shell: process.platform === "win32" });

    const require = createRequire(fs.realpathSync(path.join(dir, "node_modules", "tsrs", "package.json")));
    for (const pkg of platforms) {
        let installed = false;
        try {
            require.resolve(`${pkg.name}/package.json`);
            installed = true;
        }
        catch {
            // Nonmatching optional dependencies should not be installed.
        }
        if (installed !== (pkg.name === expected)) throw new Error(`optional dependency filtering: ${pkg.name} ${installed ? "installed unexpectedly" : "was not installed"}`);
    }
    console.log(`optional dependency selected: ${expected}`);
}
