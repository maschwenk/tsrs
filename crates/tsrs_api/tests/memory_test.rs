// Memory ownership tests. RSS is process-wide, so these tests serialize on MEMORY and live in their own test
// binary (other test files are separate processes).
// The sampler requires glibc's malloc_trim and Linux /proc; other platforms cannot run these RSS assertions.
#![cfg(all(target_os = "linux", target_env = "gnu"))]

mod common;
use common::*;
use std::sync::Mutex;
use tsrs_core::json::{self, Value};

static MEMORY: Mutex<()> = Mutex::new(());

fn arr(v: &Value) -> &Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => panic!("not an array: {}", json::marshal(v).unwrap()),
    }
}

fn create_program(s: &tsrs_api::Session, roots: &[String], options: &str) -> (u64, String) {
    let roots: Vec<String> = roots.iter().map(|r| quote(r)).collect();
    let r = call(s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{options}}}]}}", roots.join(",")));
    let snapshot = match get(&r, "snapshot") {
        Value::Number(n) => *n as u64,
        v => panic!("{v:?}"),
    };
    (snapshot, str_of(get(&r, "operation.createdPrograms.0")).to_string())
}

unsafe extern "C" {
    fn malloc_trim(pad: usize) -> i32;
}

/// Resident memory after returning free heap pages to the OS (test binaries use the system allocator, whose
/// free lists would otherwise look like growth).
fn rss_kib() -> u64 {
    // SAFETY: glibc `malloc_trim` has no preconditions.
    unsafe { malloc_trim(0) };
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

/// Program, checker and emit allocations are owned by the snapshot's regions: creating, checking, emitting
/// and releasing snapshots repeatedly must not accumulate them.
#[test]
fn released_snapshots_free_program_and_emit_memory() {
    let _g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("snaploop");
    let body = "export function f(a: string): number { return a.length; }\nexport class C { x = 1; m() { return this.x; } }\n".repeat(30);
    let a = dir.write("a.ts", &body);
    let s = session(&dir.dir(), false);
    let n: usize = std::env::var("TSRS_API_STRESS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let round = |s: &tsrs_api::Session| {
        let (snap, project) = create_program(s, &[a.clone()], "{\"declaration\":true,\"noLib\":true}");
        let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
        call(s, "getSemanticDiagnostics", &format!("{{{sp}}}"));
        let out = call(s, "emitToString", &format!("{{{sp}}}"));
        assert_eq!(arr(get(&out, "outputFiles")).len(), 2);
        call(s, "release", &format!("{{\"snapshot\":{snap}}}"));
    };
    for _ in 0..5 {
        round(&s);
    }
    let before = rss_kib();
    for _ in 0..n {
        round(&s);
    }
    let grown = rss_kib().saturating_sub(before);
    eprintln!("snapshot+emit+release x{n}: rss grew {grown} KiB");
    assert!(grown < (n as u64) * 256, "rss grew {grown} KiB over {n} rounds");
}

/// createSnapshot + release with the default libs (parity repro: ~3 MiB per cycle retained before the shared
/// program data of a full build was freed with its base region).
#[test]
fn create_and_release_snapshot_cycles_do_not_accumulate() {
    let _g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("snapcycles");
    let cfg = dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true } }"#);
    dir.write("a.ts", "export const a: number = 1;\n");
    let s = session(&dir.dir(), false);
    let mut prev: Option<u64> = None;
    let mut cycle = |s: &tsrs_api::Session| {
        let r = call(s, "createSnapshot", &format!("{{\"openProjects\":[{}]}}", quote(&cfg)));
        let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
        if let Some(p) = prev.replace(snap) {
            call(s, "release", &format!("{{\"snapshot\":{p}}}"));
        }
    };
    for _ in 0..20 {
        cycle(&s);
    }
    let before = rss_kib();
    for _ in 0..60 {
        cycle(&s);
    }
    let grown = rss_kib().saturating_sub(before);
    eprintln!("createSnapshot+release x60: rss grew {grown} KiB");
    assert!(grown < 40 * 1024, "rss grew {grown} KiB over 60 cycles");
}

/// Each transpile frees its scratch region and program: repeated calls stay correct and memory stays flat.
#[test]
fn repeated_transpile_reclaims_scratch_memory() {
    let _g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("tmloop");
    let s = session(&dir.dir(), false);
    let n: usize = std::env::var("TSRS_API_STRESS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let body = "export function f(a: string): number { return a.length; }\n".repeat(40);
    let req_js = format!("{{\"input\":{},\"options\":{{}}}}", quote(&body));
    let first_js = call(&s, "transpileModule", &req_js);
    let first_dts = call(&s, "transpileDeclaration", &req_js);
    for _ in 0..10 {
        call(&s, "transpileModule", &req_js);
        call(&s, "transpileDeclaration", &req_js);
    }
    let before = rss_kib();
    for _ in 0..n {
        assert_eq!(call(&s, "transpileModule", &req_js), first_js);
        assert_eq!(call(&s, "transpileDeclaration", &req_js), first_dts);
    }
    let grown = rss_kib().saturating_sub(before);
    eprintln!("transpile x{n}: rss grew {grown} KiB");
    // Before scratch regions each call retained ~530 KiB, then ~42 KiB while the compiler checker pool's
    // checkers were leaked; now ~5 KiB (allocator retention).
    assert!(grown < (n as u64) * 24, "rss grew {grown} KiB over {n} iterations");
}

/// Config requests build their parsed command lines in a scratch region that is freed after the response is
/// serialized.
#[test]
fn repeated_config_parsing_reclaims_memory() {
    let _g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("cfgloop");
    let cfg = dir.write("tsconfig.json", &format!("{{ \"compilerOptions\": {{ \"strict\": true, \"paths\": {{ {} }} }}, \"include\": [\"src\"] }}", (0..200).map(|i| format!("\"p{i}/*\": [\"src/p{i}/*\"]")).collect::<Vec<_>>().join(",")));
    for i in 0..50 {
        dir.write(&format!("src/f{i}.ts"), "export {};\n");
    }
    let s = session(&dir.dir(), false);
    let n: usize = std::env::var("TSRS_API_STRESS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let req = format!("{{\"file\":{}}}", quote(&cfg));
    let first = call(&s, "parseConfigFile", &req);
    for _ in 0..10 {
        call(&s, "parseConfigFile", &req);
    }
    let before = rss_kib();
    for _ in 0..n {
        assert_eq!(call(&s, "parseConfigFile", &req), first);
        call(&s, "readConfigFile", &req);
    }
    let grown = rss_kib().saturating_sub(before);
    eprintln!("parseConfigFile+readConfigFile x{n}: rss grew {grown} KiB");
    assert!(grown < (n as u64) * 8, "rss grew {grown} KiB over {n} iterations");
}

/// A createSnapshot whose program build unwinds (moduleResolution with no named kind reaches the module resolver's
/// `Unexpected moduleResolution` panic, answered as a client error) must release what the aborted clone took: the
/// parse-cache references of its files (codec d9be067 review: ~7.9 MiB retained per failure with changing content)
/// and the file loader's project reference mapper and hosts. A retained snapshot sharing a file stays intact.
#[test]
fn failed_program_builds_release_their_files() {
    let _g = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("failedbuilds");
    let a = dir.write("a.ts", "export const a = 1;\n");
    let s = session(&dir.dir(), false);
    let (keep, keep_project) = create_program(&s, &[a.clone()], "{\"noLib\":true}");
    let root = format!("{}/root.ts", dir.dir());
    let body = |i: usize| {
        format!("import {{ a }} from './a';\nexport const v{i} = a;\n")
            + &(0..1500).map(|k| format!("export declare function f{i}_{k}(x: {{ p{k}: string; q: number[] }}): Promise<Map<string, number>>;\n")).collect::<String>()
    };
    let fail = |i: usize| {
        std::fs::write(&root, body(i)).unwrap();
        let e = call_err(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"moduleResolution":-1,"noLib":true}}}}]}}"#, quote(&root)));
        assert_eq!(e, "api: client error: unsupported moduleResolution value -1 (not a ModuleResolutionKind)");
    };
    for i in 0..5 {
        fail(i);
    }
    let before = rss_kib();
    let n = 40;
    for i in 5..5 + n {
        fail(i);
    }
    let grown = rss_kib().saturating_sub(before);
    eprintln!("failed createSnapshot x{n} (changing ~150 KB root): rss grew {grown} KiB");
    assert!(grown < (n as u64) * 64, "rss grew {grown} KiB over {n} failed builds");
    // The retained snapshot's copy of a.ts (also acquired and rolled back by every failed build) is still alive.
    let sf = call(&s, "getSourceFile", &format!(r#"{{"snapshot":{keep},"project":{},"file":{}}}"#, quote(&keep_project), quote(&a)));
    assert!(!matches!(sf, Value::Null));
    let (next, _) = create_program(&s, &[a.clone()], "{\"noLib\":true}");
    call(&s, "release", &format!("{{\"snapshot\":{next}}}"));
}
