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
