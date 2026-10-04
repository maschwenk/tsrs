mod common;
use common::*;
use std::sync::{Arc, Mutex};
use tsrs_api::{ApiResult, ClientConn};
use tsrs_core::json::{self, Value};

#[test]
fn resolve_module_name_standalone_and_static() {
    let dir = TempDir::new("mr");
    dir.write("node_modules/pkg/package.json", r#"{"name":"pkg","version":"1.2.3","types":"index.d.ts"}"#);
    let idx = dir.write("node_modules/pkg/index.d.ts", "export declare const v: number;");
    let alias = dir.write("src/real.ts", "export const real = 1;");
    let s = session(&dir.dir(), false);
    let id = call(&s, "createModuleResolver", r#"{"compilerOptions":{"moduleResolution":100,"module":99}}"#);
    let r = call(&s, "resolveModuleName", &format!("{{\"resolver\":{},\"moduleName\":\"pkg\",\"containingDirectory\":{}}}", json::marshal(&id).unwrap(), quote(&dir.path("src"))));
    assert_eq!(str_of(get(&r, "resolvedModule.resolvedFileName")), idx);
    assert_eq!(str_of(get(&r, "resolvedModule.extension")), ".d.ts");
    assert_eq!(get(&r, "resolvedModule.isExternalLibraryImport"), &Value::Bool(true));
    assert_eq!(str_of(get(&r, "resolvedModule.packageId.version")), "1.2.3");
    let r = call(&s, "resolveModuleName", &format!("{{\"resolver\":{},\"moduleName\":\"missing\",\"containingDirectory\":{}}}", json::marshal(&id).unwrap(), quote(&dir.dir())));
    assert_eq!(get(&r, "resolvedModule"), &Value::Null);

    // Static entries feed createProgram: "virtual" resolves to src/real.ts without any file named virtual.
    let main = dir.write("src/main.ts", "import { real } from 'virtual';\nexport const x: string = real;\n");
    let sid = call(&s, "createModuleResolver", &format!(
        "{{\"compilerOptions\":{{\"moduleResolution\":100,\"module\":99}},\"moduleResolutions\":{{\"fallback\":\"resolve\",\"entries\":[{{\"moduleName\":\"virtual\",\"result\":{{\"resolvedFileName\":{}}}}}]}}}}",
        quote(&alias)
    ));
    let r = call(&s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{\"module\":99,\"moduleResolution\":100,\"strict\":true}},\"options\":{{\"moduleResolver\":{}}}}}]}}", quote(&main), json::marshal(&sid).unwrap()));
    let snap = json::marshal(get(&r, "snapshot")).unwrap();
    let project = str_of(get(&r, "operation.createdPrograms.0")).to_string();
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let names = call(&s, "getSourceFileNames", &format!("{{{sp}}}"));
    assert!(json::marshal(&names).unwrap().contains(&alias));
    let d = call(&s, "getSemanticDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&main)));
    assert_eq!(get(&d, "0.code"), &Value::Number(2322.0), "{}", json::marshal(&d).unwrap());

    let e = call_err(&s, "createModuleResolver", r#"{"compilerOptions":{},"moduleResolutions":{"fallback":"maybe","entries":[]}}"#);
    assert!(e.contains("invalid module resolution fallback"), "{e}");
    call(&s, "releaseModuleResolver", &format!("{{\"resolver\":{}}}", json::marshal(&sid).unwrap()));
    let e = call_err(&s, "releaseModuleResolver", &format!("{{\"resolver\":{}}}", json::marshal(&sid).unwrap()));
    assert!(e.contains("not found"), "{e}");
}

struct FakeConn {
    target: String,
    calls: Mutex<Vec<(String, String)>>,
}

impl ClientConn for FakeConn {
    fn call(&self, method: &str, params: &str) -> ApiResult<String> {
        self.calls.lock().unwrap().push((method.to_string(), params.to_string()));
        if params.contains("\"moduleName\":\"cb\"") {
            Ok(format!("{{\"resolvedFileName\":{}}}", quote(&self.target)))
        } else {
            Ok("null".to_string())
        }
    }
}

#[test]
fn callback_module_resolver_calls_client() {
    let dir = TempDir::new("mrcb");
    let target = dir.write("lib/target.ts", "export const t = 1;");
    let main = dir.write("main.ts", "import { t } from 'cb';\nexport const y = t;\n");
    let s = session(&dir.dir(), false);
    // Without a connection, callback resolvers cannot be used for programs.
    let id = call(&s, "createModuleResolver", r#"{"compilerOptions":{"moduleResolution":100,"module":99},"resolveModuleNameCallback":"resolveCb"}"#);
    let req = format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{\"module\":99,\"moduleResolution\":100}},\"options\":{{\"moduleResolver\":{}}}}}]}}", quote(&main), json::marshal(&id).unwrap());
    let e = call_err(&s, "createSnapshot", &req);
    assert!(e.contains("API connection is not initialized"), "{e}");

    let conn = Arc::new(FakeConn { target: target.clone(), calls: Mutex::new(Vec::new()) });
    s.set_connection(conn.clone());
    let r = call(&s, "createSnapshot", &req);
    let snap = json::marshal(get(&r, "snapshot")).unwrap();
    let project = str_of(get(&r, "operation.createdPrograms.0")).to_string();
    let names = call(&s, "getSourceFileNames", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)));
    assert!(json::marshal(&names).unwrap().contains(&target));
    let calls = conn.calls.lock().unwrap().clone();
    let cb = calls.iter().find(|(_, p)| p.contains("\"moduleName\":\"cb\"")).expect("callback for cb");
    assert_eq!(cb.0, "resolveCb");
    assert!(cb.1.contains("\"inProgressSnapshot\":"), "{}", cb.1);

    let r = call(&s, "resolveModuleName", &format!("{{\"resolver\":{},\"moduleName\":\"cb\",\"containingDirectory\":{},\"snapshot\":{snap}}}", json::marshal(&id).unwrap(), quote(&dir.dir())));
    assert_eq!(str_of(get(&r, "resolvedModule.resolvedFileName")), target);
    let last = conn.calls.lock().unwrap().last().unwrap().1.clone();
    assert!(last.contains(&format!("\"snapshot\":{snap}")), "{last}");
}
