// Whole-program API diagnostics share a worker pool and one diagnostics checker. Merged interfaces must neither
// schedule nested pool work while holding that checker nor lose errors in single-file requests to a CLI check slot.

#[expect(dead_code, reason = "This target uses only part of the shared integration helper module.")]
mod common;
use common::*;
use tsrs_core::json::Value;

#[test]
fn merged_interface_diagnostics_finish_and_report_in_each_declaring_file() {
    let (done, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let dir = TempDir::new("merged");
        let mut roots = vec![quote(&dir.write(
            "base.ts",
            "interface Left { kind: 'l' }\ninterface Right { kind: 'r' }\ninterface Base { tag: string }\ninterface Indexed { [key: string]: number }\n",
        ))];
        let mut declaring_files = Vec::new();
        for i in 0..48 {
            let text = if i % 4 == 0 {
                format!("interface Merged extends Left, Right {{ p{i}: number }}\ninterface Bad extends Base {{ tag: number }}\ninterface IndexTest extends Indexed {{ item{i}: string }}\n")
            } else {
                format!("export const v{i}: number = {i};\n")
            };
            let path = dir.write(&format!("f{i}.ts"), &text);
            if i % 4 == 0 {
                declaring_files.push(path.clone());
            }
            roots.push(quote(&path));
        }
        let s = session(&dir.dir(), false);
        for round in 0..12 {
            let r = call(&s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{\"skipLibCheck\":true}}}}]}}", roots.join(",")));
            let snapshot = match get(&r, "snapshot") {
                Value::Number(n) => *n as u64,
                v => panic!("{v:?}"),
            };
            let sp = format!("\"snapshot\":{snapshot},\"project\":{}", quote(str_of(get(&r, "operation.createdPrograms.0"))));
            if round % 3 == 0 {
                assert_eq!(call(&s, "getGlobalDiagnostics", &format!("{{{sp}}}")), Value::Array(vec![]));
            } else if round % 3 == 1 {
                let last = declaring_files.last().unwrap();
                let selected = call(&s, "getSemanticDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(last)));
                assert_eq!(diagnostic_sites(&selected), expected_sites(std::slice::from_ref(last)));
            }
            let all = call(&s, "getSemanticDiagnostics", &format!("{{{sp}}}"));
            assert_eq!(diagnostic_sites(&all), expected_sites(&declaring_files), "round {round}");
            for file in declaring_files.iter().rev() {
                let selected = call(&s, "getSemanticDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(file)));
                assert_eq!(diagnostic_sites(&selected), expected_sites(std::slice::from_ref(file)));
            }
            assert_eq!(call(&s, "getGlobalDiagnostics", &format!("{{{sp}}}")), Value::Array(vec![]));
            assert_eq!(call(&s, "release", &format!("{{\"snapshot\":{snapshot}}}")), Value::Bool(true));
        }
        done.send(()).unwrap();
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => worker.join().unwrap(),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("merged-interface API diagnostics did not finish within 30 seconds"),
    }
}

fn diagnostic_sites(diagnostics: &Value) -> Vec<(u32, String)> {
    let Value::Array(diagnostics) = diagnostics else { panic!("not an array") };
    let mut sites: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            let Value::Number(code) = get(d, "code") else { panic!("not a diagnostic code") };
            (*code as u32, str_of(get(d, "fileName")).to_string())
        })
        .collect();
    sites.sort();
    sites
}

fn expected_sites(files: &[String]) -> Vec<(u32, String)> {
    let mut sites: Vec<_> = files.iter().flat_map(|file| [2320, 2430, 2411].map(|code| (code, file.clone()))).collect();
    sites.sort();
    sites
}
