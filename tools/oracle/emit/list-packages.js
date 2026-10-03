// Lists the monorepo packages whose `build` script runs tsc, with the tsconfig it uses and the effective options
// that matter for emit (from `tsgo --showConfig`). Usage: node list-packages.js <repo> <tsgo>  -> TSV on stdout:
// dir  config  buildMode  target  module  jsx  experimentalDecorators  emitDecoratorMetadata  declarationDir
const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const [repo, tsgo] = process.argv.slice(2);
const files = execFileSync("git", ["-C", repo, "ls-files", "*package.json"], { encoding: "utf8" }).split("\n").filter(f => f && !f.includes("node_modules"));
for (const f of files) {
    let pkg;
    try { pkg = JSON.parse(fs.readFileSync(path.join(repo, f), "utf8")); } catch { continue; }
    const build = (pkg.scripts || {}).build || "";
    const m = /(?:^|[\s&;])tsc((?:\s+[^&;|]*)?)$/.exec(build.split("&&").map(s => s.trim()).filter(s => /(^|\s)tsc(\s|$)/.test(s)).pop() || "");
    if (!m) continue;
    const args = m[1].trim().split(/\s+/).filter(Boolean);
    const dir = path.dirname(f);
    let config = "tsconfig.json";
    const pi = args.findIndex(a => a === "-p" || a === "--project");
    if (pi >= 0) config = args[pi + 1];
    const buildMode = args.includes("-b") || args.includes("--build");
    if (buildMode) { const rest = args.filter(a => !a.startsWith("-")); if (rest.length) config = rest[0]; }
    if (!config.endsWith(".json")) config = path.join(config, "tsconfig.json");
    let opts = {};
    try {
        opts = JSON.parse(execFileSync(tsgo, ["--showConfig", "-p", path.join(repo, dir, config)], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] })).compilerOptions || {};
    } catch { opts = { error: true }; }
    console.log([dir, config, buildMode ? "b" : "-", opts.target || "", opts.module || "", opts.jsx || "", opts.experimentalDecorators ? 1 : 0, opts.emitDecoratorMetadata ? 1 : 0, opts.declarationDir || ""].map(v => v === "" ? "-" : v).join("\t"));
}
