//! printNode parity: the pinned Go server's `handlePrintNode` (DecodeNodes + printer, golden `print/*.go.txt`
//! or `*.go.err`) against `decode_nodes` + `tsrs_printer` on the same bytes. Inputs: the Go server's own
//! SourceFile encodings, the pinned JS client encoder's encodings of the same trees, and client-factory
//! synthesized trees (all positions -1, i.e. 0xFFFFFFFF on the wire) encoded by the client's encodeNode.

mod common;

use common::*;
use tsrs_api_codec::*;
use tsrs_ast::Kind;
use tsrs_printer::{new_printer, PrintHandlers, PrinterOptions};

/// Rust side of Go `handlePrintNode`: `Ok(text)`, or the error / panic message.
fn print_node(bytes: &[u8]) -> Result<String, String> {
    let decoded = decode_nodes(bytes).map_err(|e| format!("decode: {e}"))?;
    let root = decoded.root();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = decoded.region().enter();
        let sf = (root.kind() == Kind::SourceFile).then(|| root.as_source_file_p());
        new_printer(PrinterOptions::default(), PrintHandlers::default(), None).emit(root, sf)
    }));
    r.map_err(|p| format!("panic: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()))
}

#[test]
fn print_node_matches_pinned_go_server() {
    let dir = crate_dir().join("tests/golden/print");
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, _) in fixtures() {
        cases.push((name.clone(), golden(&name)));
        cases.push((format!("{name}.client"), client_golden(&name)));
    }
    for e in std::fs::read_dir(&dir).unwrap() {
        let f = e.unwrap().file_name().into_string().unwrap();
        if let Some(base) = f.strip_suffix(".client.bin") {
            cases.push((base.to_string(), std::fs::read(dir.join(&f)).unwrap()));
        }
    }
    cases.sort();
    let (mut printed, mut errors, mut failures) = (0, 0, Vec::new());
    for (case, bytes) in &cases {
        let rust = print_node(bytes);
        let go_txt = std::fs::read_to_string(dir.join(format!("{case}.go.txt")));
        match (go_txt, rust) {
            (Ok(go), Ok(rust)) if go == rust => printed += 1,
            (Ok(go), rust) => failures.push(format!("== {case}: go printed\n{go}\n-- rust\n{rust:?}")),
            (Err(_), Err(rust_err)) => {
                let go_err = std::fs::read_to_string(dir.join(format!("{case}.go.err"))).unwrap();
                eprintln!("{case}: both fail; go {:?}, rust {rust_err:?}", go_err.trim());
                // Go formats kinds as `KindX` (Kind.String), tsrs as `X` (Debug); the failure is the same.
                let go_msg = go_err.trim().trim_start_matches("panic: ").replace(": Kind", ": ");
                let rust_msg = rust_err.trim_start_matches("decode: ").trim_start_matches("panic: ");
                if go_msg.contains(rust_msg) || rust_msg.contains(go_msg.as_str()) {
                    errors += 1;
                } else {
                    failures.push(format!("== {case}: different failures: go {go_err:?} rust {rust_err:?}"));
                }
            }
            (Err(_), Ok(rust)) => failures.push(format!("== {case}: go failed but rust printed\n{rust}")),
        }
    }
    eprintln!("printNode parity: {} cases, {printed} identical texts, {errors} identical failures, {} mismatches", cases.len(), failures.len());
    assert!(cases.len() >= 26);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
