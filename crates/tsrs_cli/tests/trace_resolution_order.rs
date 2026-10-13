// `--traceResolution` says whether a package.json lookup was served "according to earlier cached lookups", so its
// text depends on the order in which files resolve. tsrs loads traced programs in the order of Go's single-threaded
// work group (last queued first, core/workgroup.go), at any checker count. Here `index.ts` imports `./a` then `./b`,
// which both import `pkg`: Go resolves `b.ts` first, so the lookup in `a.ts`'s (earlier) trace is the cached one.
// Expected output from tsgo 7.1.0-dev.20260930.4 (the pinned commit) with `--singleThreaded`.

use std::path::PathBuf;
use std::process::Command;

const EXPECTED: &str = r"======== Resolving module './a' from '<DIR>/index.ts'. ========
Explicitly specified module resolution kind: 'Bundler'.
Resolving in CJS mode with conditions 'import', 'types'.
Loading module as file / folder, candidate module location '<DIR>/a', target file types: TypeScript, JavaScript, Declaration, JSON.
File '<DIR>/a.ts' exists - use it as a name resolution result.
======== Module name './a' was successfully resolved to '<DIR>/a.ts'. ========
======== Resolving module './b' from '<DIR>/index.ts'. ========
Explicitly specified module resolution kind: 'Bundler'.
Resolving in CJS mode with conditions 'import', 'types'.
Loading module as file / folder, candidate module location '<DIR>/b', target file types: TypeScript, JavaScript, Declaration, JSON.
File '<DIR>/b.ts' exists - use it as a name resolution result.
======== Module name './b' was successfully resolved to '<DIR>/b.ts'. ========
======== Resolving module 'pkg' from '<DIR>/a.ts'. ========
Explicitly specified module resolution kind: 'Bundler'.
Resolving in CJS mode with conditions 'import', 'types'.
File '<DIR>/package.json' exists according to earlier cached lookups.
Loading module 'pkg' from 'node_modules' folder, target file types: TypeScript, JavaScript, Declaration, JSON.
Searching all ancestor node_modules directories for preferred extensions: TypeScript, Declaration.
File '<DIR>/node_modules/pkg/package.json' exists according to earlier cached lookups.
File '<DIR>/node_modules/pkg.ts' does not exist.
File '<DIR>/node_modules/pkg.tsx' does not exist.
File '<DIR>/node_modules/pkg.d.ts' does not exist.
'package.json' does not have a 'typesVersions' field.
'package.json' does not have a 'typings' field.
'package.json' has 'types' field 'index.d.ts' that references '<DIR>/node_modules/pkg/index.d.ts'.
File '<DIR>/node_modules/pkg/index.d.ts' exists - use it as a name resolution result.
'package.json' does not have a 'peerDependencies' field.
Resolving real path for '<DIR>/node_modules/pkg/index.d.ts', result '<DIR>/node_modules/pkg/index.d.ts'.
======== Module name 'pkg' was successfully resolved to '<DIR>/node_modules/pkg/index.d.ts' with Package ID 'pkg/index.d.ts@1.0.0'. ========
======== Resolving module 'pkg' from '<DIR>/b.ts'. ========
Explicitly specified module resolution kind: 'Bundler'.
Resolving in CJS mode with conditions 'import', 'types'.
File '<DIR>/package.json' exists according to earlier cached lookups.
Loading module 'pkg' from 'node_modules' folder, target file types: TypeScript, JavaScript, Declaration, JSON.
Searching all ancestor node_modules directories for preferred extensions: TypeScript, Declaration.
Found 'package.json' at '<DIR>/node_modules/pkg/package.json'.
File '<DIR>/node_modules/pkg.ts' does not exist.
File '<DIR>/node_modules/pkg.tsx' does not exist.
File '<DIR>/node_modules/pkg.d.ts' does not exist.
'package.json' does not have a 'typesVersions' field.
'package.json' does not have a 'typings' field.
'package.json' has 'types' field 'index.d.ts' that references '<DIR>/node_modules/pkg/index.d.ts'.
File '<DIR>/node_modules/pkg/index.d.ts' exists - use it as a name resolution result.
'package.json' does not have a 'peerDependencies' field.
Resolving real path for '<DIR>/node_modules/pkg/index.d.ts', result '<DIR>/node_modules/pkg/index.d.ts'.
======== Module name 'pkg' was successfully resolved to '<DIR>/node_modules/pkg/index.d.ts' with Package ID 'pkg/index.d.ts@1.0.0'. ========
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-trace-order-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    let files = [
        ("tsconfig.json", r#"{"compilerOptions":{"noEmit":true,"module":"esnext","moduleResolution":"bundler","types":[]},"files":["index.ts"]}"#),
        // Ends the package.json lookups of the importing files' directories here, wherever the scratch directory is.
        ("package.json", r#"{"name":"root","private":true}"#),
        ("index.ts", "import \"./a\";\nimport \"./b\";\n"),
        ("a.ts", "import { p } from \"pkg\"; export const a = p;\n"),
        ("b.ts", "import { p } from \"pkg\"; export const b = p;\n"),
        ("node_modules/pkg/package.json", r#"{"name":"pkg","version":"1.0.0","types":"index.d.ts"}"#),
        ("node_modules/pkg/index.d.ts", "export declare const p: number;\n"),
    ];
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

#[test]
fn traces_follow_go_single_threaded_order_at_any_checker_count() {
    for (i, extra) in [&["--singleThreaded"][..], &["--checkers", "4"][..]].into_iter().enumerate() {
        let dir = scratch(&i.to_string());
        let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
            .args(["-p", ".", "--traceResolution"])
            .args(extra)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{extra:?}: exit {:?}", out.status.code());
        let canonical = std::fs::canonicalize(&dir).unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap().replace(canonical.to_str().unwrap(), "<DIR>").replace(dir.to_str().unwrap(), "<DIR>");
        assert_eq!(stdout, EXPECTED, "{extra:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
