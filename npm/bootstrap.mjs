#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const [version, ...extra] = process.argv.slice(2);
if (!/^0\.0\.0-bootstrap\.(0|[1-9]\d*)$/.test(version ?? "") || extra.length) {
    throw new Error("usage: node npm/bootstrap.mjs 0.0.0-bootstrap.<number>");
}

const root = fileURLToPath(new URL("../", import.meta.url));
const { repository, license } = JSON.parse(fs.readFileSync(path.join(root, "npm/tsrs/package.json"), "utf8"));
const out = fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-bootstrap-"));
const packages = [];
for (const platform of ["darwin-arm64", "linux-x64-gnu", "linux-arm64-gnu", "linux-x64-musl", "linux-arm64-musl", "wasm"]) {
    const name = `@tsrs/${platform}`;
    const dir = path.join(out, platform);
    fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify({
        name,
        version,
        description: `Placeholder for ${name}; the compiler will be available in a future release`,
        license,
        repository,
        files: ["README.md", "LICENSE"],
        publishConfig: { access: "public", tag: "bootstrap" },
    }, undefined, 4) + "\n");
    fs.writeFileSync(path.join(dir, "README.md"),
        `# ${name}\n\nPlaceholder for upcoming tsrs releases. This package contains no compiler binaries or runtime code.\n`);
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(dir, "LICENSE"));
    const packed = JSON.parse(execFileSync("pnpm", ["pack", "--json", "--pack-destination", out], {
        cwd: dir,
        encoding: "utf8",
    }));
    packages.push({ name, tarball: path.resolve(out, packed.filename) });
}
console.log(JSON.stringify({ version, packages }, undefined, 2));
