#!/usr/bin/env python3
"""Scenarios for tsrs-oracle-modulespecifiers (one JSON object per line on stdout).

  python3 tools/oracle/modulespecifiers/gen.py > crates/tsrs_compiler/testdata/modulespecifiers_oracle/scenarios.jsonl
  bin/tsrs-oracle-modulespecifiers < .../scenarios.jsonl > .../expected.jsonl
"""
import json

ESM = 99
CJS = 1


def req(frm, to, ending="", mode=0, pref=""):
    return {"from": frm, "to": to, "ending": ending, "mode": mode, "pref": pref}


def all_prefs(frm, to, ending="", mode=0):
    return [req(frm, to, ending, mode, pref) for pref in ["", "shortest", "relative", "non-relative"]]


def scenario(name, files, requests, symlinks=None, config="/project/tsconfig.json", cwd="/project", case_sensitive=True):
    return {
        "name": name,
        "cwd": cwd,
        "config": config,
        "caseSensitive": case_sensitive,
        "files": files,
        "symlinks": symlinks or {},
        "requests": requests,
    }


def tsconfig(**options):
    return json.dumps({"compilerOptions": options})


out = []

# Plain relative specifiers, index elision, endings.
relative_files = {
    "/project/tsconfig.json": tsconfig(strict=True),
    "/project/src/a.ts": "export const a = 1;",
    "/project/src/sub/b.ts": "export const b = 1;",
    "/project/src/dir/index.ts": "export const d = 1;",
    "/project/src/dir.ts": "export const clash = 1;",
    "/project/src/other/index.ts": "export const o = 1;",
    "/project/src/decl.d.ts": "export declare const x: number;",
    "/project/src/m.mts": "export const m = 1;",
    "/project/src/c.cts": "export const c = 1;",
    "/project/src/dm.d.mts": "export declare const dm: number;",
    "/project/lib/c.ts": "export const c = 1;",
    "/project/src/comp.tsx": "export const Comp = 1;",
}
out.append(scenario("relative", relative_files, [
    req("/project/src/a.ts", "/project/src/sub/b.ts"),
    req("/project/src/sub/b.ts", "/project/src/a.ts"),
    req("/project/src/a.ts", "/project/lib/c.ts"),
    req("/project/src/a.ts", "/project/src/dir/index.ts"),
    req("/project/src/a.ts", "/project/src/other/index.ts"),
    req("/project/src/a.ts", "/project/src/other/index.ts", "js"),
    req("/project/src/a.ts", "/project/src/decl.d.ts"),
    req("/project/src/a.ts", "/project/src/decl.d.ts", "js"),
    req("/project/src/a.ts", "/project/src/m.mts"),
    req("/project/src/a.ts", "/project/src/c.cts"),
    req("/project/src/a.ts", "/project/src/dm.d.mts"),
    req("/project/src/a.ts", "/project/src/comp.tsx", "js"),
    req("/project/src/a.ts", "/project/src/sub/b.ts", "", ESM),
]))

# nodenext: ESM files need extensions; existing import is reused.
out.append(scenario("nodenext", {
    "/project/tsconfig.json": tsconfig(module="nodenext", strict=True),
    "/project/package.json": json.dumps({"name": "app", "type": "module"}),
    "/project/src/a.ts": "import { b } from './sub/b.js'; export const a = b;",
    "/project/src/sub/b.ts": "export const b = 1;",
    "/project/src/dir/index.ts": "export const d = 1;",
    "/project/src/c.cts": "export const c = 1;",
}, [
    req("/project/src/a.ts", "/project/src/sub/b.ts"),
    req("/project/src/a.ts", "/project/src/dir/index.ts"),
    req("/project/src/a.ts", "/project/src/dir/index.ts", "", CJS),
    req("/project/src/c.cts", "/project/src/dir/index.ts"),
    req("/project/src/sub/b.ts", "/project/src/a.ts"),
]))

# allowImportingTsExtensions.
out.append(scenario("ts-extensions", {
    "/project/tsconfig.json": tsconfig(module="preserve", allowImportingTsExtensions=True, noEmit=True),
    "/project/src/a.ts": "import { b } from './b.ts'; export const a = b;",
    "/project/src/b.ts": "export const b = 1;",
    "/project/src/c.ts": "export const c = 1;",
    "/project/src/d.d.ts": "export declare const d: number;",
}, [
    req("/project/src/a.ts", "/project/src/c.ts"),
    req("/project/src/a.ts", "/project/src/d.d.ts"),
    req("/project/src/c.ts", "/project/src/b.ts"),
    req("/project/src/c.ts", "/project/src/b.ts", "js"),
]))

# paths / baseUrl.
out.append(scenario("paths", {
    "/project/tsconfig.json": json.dumps({"compilerOptions": {
        "baseUrl": ".",
        "paths": {"@lib/*": ["lib/*"], "@root": ["src/index.ts"], "@dist/*": ["dist/*.d.ts"], "~/*": ["./src/*"]},
    }}),
    "/project/src/index.ts": "export const i = 1;",
    "/project/src/deep/nested/a.ts": "export const a = 1;",
    "/project/src/deep/nested/sib.ts": "export const s = 1;",
    "/project/lib/util.ts": "export const u = 1;",
    "/project/lib/pkg/index.ts": "export const p = 1;",
    "/project/dist/haha.d.ts": "export declare const h: number;",
    "/project/other/x.ts": "export const x = 1;",
}, [
    *all_prefs("/project/src/deep/nested/a.ts", "/project/lib/util.ts"),
    *all_prefs("/project/src/deep/nested/a.ts", "/project/lib/pkg/index.ts"),
    *all_prefs("/project/src/deep/nested/a.ts", "/project/src/index.ts"),
    *all_prefs("/project/src/deep/nested/a.ts", "/project/dist/haha.d.ts"),
    *all_prefs("/project/src/deep/nested/a.ts", "/project/src/deep/nested/sib.ts"),
    *all_prefs("/project/src/deep/nested/a.ts", "/project/other/x.ts"),
    *all_prefs("/project/lib/util.ts", "/project/src/deep/nested/a.ts"),
]))

# paths without baseUrl, tsconfig in a subdirectory (project-relative boundary).
out.append(scenario("paths-project-relative", {
    "/project/app/tsconfig.json": json.dumps({"compilerOptions": {"paths": {"@shared/*": ["../shared/*"], "@app/*": ["./*"]}}, "include": ["**/*", "../shared/**/*"]}),
    "/project/app/src/main.ts": "export const m = 1;",
    "/project/app/src/feature/f.ts": "export const f = 1;",
    "/project/shared/s.ts": "export const s = 1;",
}, [
    *all_prefs("/project/app/src/main.ts", "/project/shared/s.ts"),
    *all_prefs("/project/app/src/main.ts", "/project/app/src/feature/f.ts"),
    *all_prefs("/project/shared/s.ts", "/project/app/src/main.ts"),
], config="/project/app/tsconfig.json", cwd="/project/app"))

# rootDirs.
out.append(scenario("rootDirs", {
    "/project/tsconfig.json": tsconfig(rootDirs=["src", "generated"]),
    "/project/src/views/view.ts": "export const v = 1;",
    "/project/generated/views/template.ts": "export const t = 1;",
}, [
    req("/project/src/views/view.ts", "/project/generated/views/template.ts"),
    req("/project/generated/views/template.ts", "/project/src/views/view.ts"),
]))

# node_modules packages: types/main, exports (conditions, patterns, blocked), @types, typesVersions.
pkg_files = {
    "/project/tsconfig.json": tsconfig(module="nodenext", strict=True),
    "/project/package.json": json.dumps({"name": "app", "type": "module"}),
    "/project/src/main.ts": "\n".join([
        "import 'plain';",
        "import 'exp';",
        "import 'exp/sub';",
        "import 'exp/features/one';",
        "import 'lodash';",
        "import '@scope/tool';",
        "import 'tv';",
    ]),
    "/project/node_modules/plain/package.json": json.dumps({"name": "plain", "types": "./lib/index.d.ts"}),
    "/project/node_modules/plain/lib/index.d.ts": "export declare const p: number;",
    "/project/node_modules/plain/lib/extra.d.ts": "export declare const e: number;",
    "/project/node_modules/exp/package.json": json.dumps({
        "name": "exp",
        "exports": {
            ".": {"types": "./dist/index.d.ts", "default": "./dist/index.js"},
            "./sub": {"import": {"types": "./dist/sub.d.mts"}, "require": {"types": "./dist/sub.d.cts"}},
            "./features/*": {"types": "./dist/features/*.d.ts"},
            "./internal/*": None,
        },
    }),
    "/project/node_modules/exp/dist/index.d.ts": "export declare const e: number;",
    "/project/node_modules/exp/dist/sub.d.mts": "export declare const s: number;",
    "/project/node_modules/exp/dist/sub.d.cts": "export declare const s: number;",
    "/project/node_modules/exp/dist/features/one.d.ts": "export declare const one: number;",
    "/project/node_modules/exp/dist/private.d.ts": "export declare const priv: number;",
    "/project/node_modules/@types/lodash/package.json": json.dumps({"name": "@types/lodash", "types": "index.d.ts"}),
    "/project/node_modules/@types/lodash/index.d.ts": "export declare const l: number;",
    "/project/node_modules/@types/lodash/fp.d.ts": "export declare const fp: number;",
    "/project/node_modules/@scope/tool/package.json": json.dumps({"name": "@scope/tool", "main": "./out/main.js"}),
    "/project/node_modules/@scope/tool/out/main.d.ts": "export declare const t: number;",
    "/project/node_modules/@scope/tool/out/helpers/index.d.ts": "export declare const h: number;",
    "/project/node_modules/tv/package.json": json.dumps({"name": "tv", "types": "index.d.ts", "typesVersions": {"*": {"*": ["ts/*"]}}}),
    "/project/node_modules/tv/ts/index.d.ts": "export declare const tv: number;",
    "/project/node_modules/tv/ts/more.d.ts": "export declare const more: number;",
    "/project/node_modules/noPkgJson/index.d.ts": "export declare const n: number;",
    "/project/node_modules/noPkgJson/other.d.ts": "export declare const o: number;",
}
pkg_requests = [
    req("/project/src/main.ts", "/project/node_modules/plain/lib/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/plain/lib/extra.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/sub.d.mts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/sub.d.cts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/features/one.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/private.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/@types/lodash/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/@types/lodash/fp.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/@scope/tool/out/main.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/@scope/tool/out/helpers/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/tv/ts/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/tv/ts/more.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/noPkgJson/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/noPkgJson/other.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/exp/dist/index.d.ts", "", CJS),
]
out.append(scenario("node_modules-nodenext", pkg_files, pkg_requests))
pkg_files_node10 = dict(pkg_files)
pkg_files_node10["/project/tsconfig.json"] = tsconfig(module="commonjs", moduleResolution="node10")
pkg_files_node10["/project/package.json"] = json.dumps({"name": "app"})
out.append(scenario("node_modules-node10", pkg_files_node10, pkg_requests))
pkg_files_bundler = dict(pkg_files)
pkg_files_bundler["/project/tsconfig.json"] = tsconfig(module="esnext", moduleResolution="bundler")
out.append(scenario("node_modules-bundler", pkg_files_bundler, pkg_requests))

# Nested node_modules not reachable from the importing file.
out.append(scenario("sibling-node_modules", {
    "/project/tsconfig.json": tsconfig(strict=True),
    "/project/a/main.ts": "export {};",
    "/project/b/node_modules/dep/index.d.ts": "export declare const d: number;",
    "/project/b/node_modules/dep/package.json": json.dumps({"name": "dep"}),
}, [
    req("/project/a/main.ts", "/project/b/node_modules/dep/index.d.ts"),
]))

# Symlinked monorepo package (pnpm/workspaces style).
out.append(scenario("symlinked-package", {
    "/project/tsconfig.json": json.dumps({"compilerOptions": {"module": "nodenext"}, "include": ["app/**/*"]}),
    "/project/package.json": json.dumps({"name": "root", "dependencies": {"lib": "workspace:*", "other": "workspace:*"}}),
    "/project/app/main.ts": "import { l } from 'lib'; export const m = l;",
    "/project/packages/lib/package.json": json.dumps({"name": "lib", "types": "./src/index.ts", "exports": {".": "./src/index.ts", "./util": "./src/util.ts"}}),
    "/project/packages/lib/src/index.ts": "export const l = 1;",
    "/project/packages/lib/src/util.ts": "export const u = 1;",
    "/project/packages/lib/src/hidden.ts": "export const h = 1;",
    "/project/packages/other/package.json": json.dumps({"name": "other", "types": "./index.d.ts"}),
    "/project/packages/other/index.d.ts": "export declare const o: number;",
}, [
    req("/project/app/main.ts", "/project/packages/lib/src/index.ts"),
    req("/project/app/main.ts", "/project/packages/lib/src/util.ts"),
    req("/project/app/main.ts", "/project/packages/lib/src/hidden.ts"),
    req("/project/app/main.ts", "/project/packages/other/index.d.ts"),
    req("/project/packages/lib/src/index.ts", "/project/packages/lib/src/util.ts"),
    *all_prefs("/project/app/main.ts", "/project/packages/lib/src/hidden.ts"),
], symlinks={
    "/project/node_modules/lib": "/project/packages/lib",
    "/project/node_modules/other": "/project/packages/other",
}))

# package.json "imports".
out.append(scenario("package-imports", {
    "/project/tsconfig.json": tsconfig(module="nodenext", outDir="dist", rootDir="src"),
    "/project/package.json": json.dumps({"name": "app", "type": "module", "imports": {
        "#utils/*": "./dist/utils/*.js",
        "#config": {"types": "./src/config.ts", "default": "./dist/config.js"},
        "#dir/": "./dist/dir/",
    }}),
    "/project/src/main.ts": "export {};",
    "/project/src/utils/strings.ts": "export const s = 1;",
    "/project/src/config.ts": "export const c = 1;",
    "/project/src/dir/x.ts": "export const x = 1;",
    "/project/src/plain.ts": "export const p = 1;",
}, [
    *all_prefs("/project/src/main.ts", "/project/src/utils/strings.ts"),
    *all_prefs("/project/src/main.ts", "/project/src/config.ts"),
    *all_prefs("/project/src/main.ts", "/project/src/dir/x.ts"),
    *all_prefs("/project/src/main.ts", "/project/src/plain.ts"),
]))

# Case-insensitive file system.
out.append(scenario("case-insensitive", {
    "/Project/tsconfig.json": tsconfig(strict=True),
    "/Project/Src/A.ts": "export const a = 1;",
    "/Project/Src/Sub/B.ts": "export const b = 1;",
}, [
    req("/Project/Src/A.ts", "/Project/Src/Sub/B.ts"),
    req("/project/src/a.ts", "/project/src/sub/b.ts"),
], config="/Project/tsconfig.json", cwd="/Project", case_sensitive=False))

# JSON modules and non-JS declaration files.
out.append(scenario("json-and-arbitrary", {
    "/project/tsconfig.json": tsconfig(resolveJsonModule=True, allowArbitraryExtensions=True, module="esnext", moduleResolution="bundler"),
    "/project/src/a.ts": "export {};",
    "/project/src/data.json": "{}",
    "/project/src/styles.d.css.ts": "export declare const s: string;",
}, [
    req("/project/src/a.ts", "/project/src/data.json"),
    req("/project/src/a.ts", "/project/src/styles.d.css.ts"),
]))

# pnpm layout: realpaths under node_modules/.pnpm are ignored paths, the symlink is preferred.
out.append(scenario("pnpm", {
    "/project/tsconfig.json": tsconfig(module="nodenext"),
    "/project/package.json": json.dumps({"name": "app", "type": "module", "dependencies": {"pkg": "1.0.0"}}),
    "/project/src/main.ts": "import 'pkg';",
    "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/package.json": json.dumps({"name": "pkg", "version": "1.0.0", "exports": {".": "./index.js", "./feature": "./feature.js"}}),
    "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/index.d.ts": "export declare const p: number;",
    "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/feature.d.ts": "export declare const f: number;",
    "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/internal.d.ts": "export declare const i: number;",
}, [
    req("/project/src/main.ts", "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/feature.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg/internal.d.ts"),
], symlinks={
    "/project/node_modules/pkg": "/project/node_modules/.pnpm/pkg@1.0.0/node_modules/pkg",
}))

# Duplicate packages (same name@version) in two places: redirect targets.
dup_pkg = json.dumps({"name": "dup", "version": "1.2.3", "types": "index.d.ts"})
out.append(scenario("duplicate-packages", {
    "/project/tsconfig.json": tsconfig(strict=True),
    "/project/src/main.ts": "import 'a'; import 'b';",
    "/project/node_modules/a/package.json": json.dumps({"name": "a", "types": "index.d.ts"}),
    "/project/node_modules/a/index.d.ts": "export * from 'dup';",
    "/project/node_modules/a/node_modules/dup/package.json": dup_pkg,
    "/project/node_modules/a/node_modules/dup/index.d.ts": "export declare class D { private x; }",
    "/project/node_modules/b/package.json": json.dumps({"name": "b", "types": "index.d.ts"}),
    "/project/node_modules/b/index.d.ts": "export * from 'dup';",
    "/project/node_modules/b/node_modules/dup/package.json": dup_pkg,
    "/project/node_modules/b/node_modules/dup/index.d.ts": "export declare class D { private x; }",
}, [
    req("/project/src/main.ts", "/project/node_modules/a/node_modules/dup/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/b/node_modules/dup/index.d.ts"),
    req("/project/node_modules/a/index.d.ts", "/project/node_modules/a/node_modules/dup/index.d.ts"),
]))

# More exports shapes: directory mapping, arrays, versioned types, custom conditions, self-reference.
out.append(scenario("exports-shapes", {
    "/project/tsconfig.json": tsconfig(module="nodenext", customConditions=["custom"]),
    "/project/package.json": json.dumps({"name": "self", "type": "module", "exports": {"./lib/*": "./src/lib/*.ts"}}),
    "/project/src/main.ts": "import 'dirs';",
    "/project/src/lib/x.ts": "export const x = 1;",
    "/project/node_modules/dirs/package.json": json.dumps({"name": "dirs", "exports": {
        ".": [{"types@>=4.0": "./ts4/index.d.ts"}, "./fallback.d.ts"],
        "./d/": "./dist/d/",
        "./c": {"custom": "./custom.d.ts", "default": "./c.d.ts"},
        "./arr": ["./nope.d.ts", {"types": "./arr.d.ts"}],
    }}),
    "/project/node_modules/dirs/ts4/index.d.ts": "export declare const a: number;",
    "/project/node_modules/dirs/fallback.d.ts": "export declare const f: number;",
    "/project/node_modules/dirs/dist/d/deep/file.d.ts": "export declare const d: number;",
    "/project/node_modules/dirs/custom.d.ts": "export declare const c: number;",
    "/project/node_modules/dirs/c.d.ts": "export declare const c: number;",
    "/project/node_modules/dirs/arr.d.ts": "export declare const r: number;",
}, [
    req("/project/src/main.ts", "/project/node_modules/dirs/ts4/index.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/dirs/fallback.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/dirs/dist/d/deep/file.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/dirs/custom.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/dirs/c.d.ts"),
    req("/project/src/main.ts", "/project/node_modules/dirs/arr.d.ts"),
    *all_prefs("/project/src/main.ts", "/project/src/lib/x.ts"),
]))

# Symlinked package importing from itself through its own symlink must stay relative.
out.append(scenario("symlink-own-package", {
    "/project/tsconfig.json": json.dumps({"compilerOptions": {"module": "commonjs"}, "include": ["packages/**/*"]}),
    "/project/packages/app/src/a.ts": "import 'lib';",
    "/project/packages/lib/package.json": json.dumps({"name": "lib", "types": "index.ts"}),
    "/project/packages/lib/index.ts": "export * from './src/b';",
    "/project/packages/lib/src/b.ts": "export const b = 1;",
    "/project/packages/lib/src/c.ts": "export const c = 1;",
}, [
    req("/project/packages/lib/src/b.ts", "/project/packages/lib/src/c.ts"),
    req("/project/packages/app/src/a.ts", "/project/packages/lib/src/c.ts"),
    req("/project/packages/app/src/a.ts", "/project/packages/lib/index.ts"),
], symlinks={
    "/project/packages/app/node_modules/lib": "/project/packages/lib",
}))

for s in out:
    print(json.dumps(s))
