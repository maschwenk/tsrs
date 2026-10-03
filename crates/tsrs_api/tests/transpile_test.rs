mod common;
use common::*;
use tsrs_core::json::Value;

#[test]
fn transpile_module_strips_types() {
    let dir = TempDir::new("tm");
    let s = session(&dir.dir(), false);
    let r = call(&s, "transpileModule", r#"{"input":"export const x: number = 1;\nenum E { A }\n","options":{"compilerOptions":{"target":9,"module":99}}}"#);
    let out = str_of(get(&r, "outputText"));
    assert!(out.contains("export const x = 1;"), "{out}");
    assert!(out.contains("var E;"), "{out}");
    assert!(matches!(r, Value::Object(ref o) if !o.contains_key("diagnostics")));

    // Syntax errors only reported with reportDiagnostics.
    let r = call(&s, "transpileModule", r#"{"input":"let = ;","options":{"reportDiagnostics":true}}"#);
    assert!(matches!(get(&r, "diagnostics"), Value::Array(a) if !a.is_empty()));
    assert_eq!(str_of(get(&r, "diagnostics.0.fileName")), "/module.ts");

    let r = call(&s, "transpileModule", r#"{"input":"const a = 1;","options":{"compilerOptions":{"sourceMap":true},"fileName":"x.ts"}}"#);
    assert!(str_of(get(&r, "sourceMapText")).contains("\"sources\":[\"x.ts\"]"), "{:?}", get(&r, "sourceMapText"));
}

#[test]
fn transpile_declaration_and_from_file() {
    let dir = TempDir::new("td");
    let f = dir.write("lib.ts", "export function f(a: string): number { return a.length; }\nexport const y = Symbol.for('y');\n");
    let s = session(&dir.dir(), false);
    let r = call(&s, "transpileDeclarationFromFile", &format!("{{\"fileName\":{},\"options\":{{}}}}", quote(&f)));
    let out = str_of(get(&r, "outputText"));
    assert!(out.contains("export declare function f(a: string): number;"), "{out}");
    let r = call(&s, "transpileModuleFromFile", &format!("{{\"fileName\":{},\"options\":{{}}}}", quote(&f)));
    assert!(str_of(get(&r, "outputText")).contains("return a.length"));
    // Upstream test/sync/api.test.ts "transpileDeclaration" expectations.
    let r = call(&s, "transpileDeclaration", r#"{"input":"export const x: number = 1;","options":{}}"#);
    assert_eq!(str_of(get(&r, "outputText")), "export declare const x: number;\n");
    let r = call(&s, "transpileDeclaration", r#"{"input":"export const x: number = 1;","options":{"fileName":"C:/Users/me/project/input.ts"}}"#);
    assert_eq!(str_of(get(&r, "outputText")), "export declare const x: number;\n");
    let e = call_err(&s, "transpileModuleFromFile", &format!("{{\"fileName\":{}}}", quote(&dir.path("nope.ts"))));
    assert!(e.contains("could not read file"), "{e}");
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

/// Each transpile frees its scratch region and program: repeated calls stay correct and memory stays flat.
#[test]
fn repeated_transpile_reclaims_scratch_memory() {
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
    // Before scratch regions each call retained ~530 KiB; the remainder (~25 KiB/call, heap state of the
    // compiler/emitter outside arenas) is a documented gap. Guard against regressions to the old behavior.
    assert!(grown < (n as u64) * 128, "rss grew {grown} KiB over {n} iterations");
}
