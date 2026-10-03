//! Decoder (Go `DecodeNodes`) on bytes from the pinned Go server and from the pinned JavaScript client encoder,
//! checked by re-encoding the decoded tree and comparing structure, and robustness on malformed input.

mod common;

use common::*;
use tsrs_api_codec::*;
use tsrs_ast::*;

fn roundtrip(name: &str, bytes: &[u8], what: &str) -> Result<(), String> {
    let suffix = if what == "js client" { ".client" } else { "" };
    let go_err = crate_dir().join(format!("tests/golden/reencode/{name}{suffix}.goreencoded.err"));
    if let Ok(go_err) = std::fs::read_to_string(go_err) {
        // The pinned Go decoder fails on this input; the Rust decoder must fail the same way.
        let rust = decode_source_file(bytes).err().map(|e| e.to_string());
        return if rust.as_deref() == Some(go_err.trim().trim_start_matches("panic: ")) {
            Ok(())
        } else {
            Err(format!("{name} ({what}): go failed with {go_err:?}, rust {rust:?}"))
        };
    }
    let decoded = decode_source_file(bytes).map_err(|e| format!("{name} ({what}): decode failed: {e}"))?;
    let root = decoded.root();
    let sf = root.as_source_file();
    assert_eq!(sf.file_name(), format!("/fixtures/{name}"), "{name}");
    assert_eq!(&*sf.path().0, format!("/fixtures/{name}").as_str(), "{name}");
    let original = std::fs::read(crate_dir().join("tests/fixtures").join(name)).unwrap();
    // The JS client's RemoteSourceFile.text drops a leading BOM, so its re-encoding does too.
    let original = if what == "js client" { original.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&original).to_vec() } else { original };
    assert_eq!(sf.text().as_bytes(), &original[..], "{name}: text");
    let expected_kind = tsrs_core::ensure_script_kind_from_file_name(name);
    assert_eq!(sf.script_kind(), expected_kind, "{name}: scriptKind");
    let (re, _) = encode_node(root, None).map_err(|e| format!("{name}: re-encode failed: {e}"))?;
    if let Some(d) = diff_lines(&structural(bytes, false), &structural(&re, false)) {
        return Err(format!("{name} ({what}): {d}"));
    }
    Ok(())
}

#[test]
fn decode_go_server_encodings() {
    let mut errors = Vec::new();
    let fixtures = fixtures();
    for (name, _) in &fixtures {
        if let Err(e) = roundtrip(name, &golden(name), "go server") {
            errors.push(e);
        }
    }
    eprintln!("decoded {} go-server encodings, {} failures", fixtures.len(), errors.len());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn decode_js_client_encodings() {
    let mut errors = Vec::new();
    let fixtures = fixtures();
    for (name, _) in &fixtures {
        if let Err(e) = roundtrip(name, &client_golden(name), "js client") {
            errors.push(e);
        }
    }
    eprintln!("decoded {} js-client encodings, {} failures", fixtures.len(), errors.len());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn decoded_tree_shape() {
    let decoded = decode_source_file(&golden("basic.ts")).unwrap();
    let sf = decoded.root().as_source_file();
    let stmts = sf.statements.nodes();
    assert_eq!(stmts.iter().map(|s| s.kind()).collect::<Vec<_>>(), [Kind::ImportDeclaration, Kind::FunctionDeclaration, Kind::ExpressionStatement]);
    let f = stmts[1].as_function_declaration();
    assert_eq!(f.name().unwrap().text(), "foo");
    assert!(stmts[1].modifier_flags().intersects(ModifierFlags::Export));
    assert_eq!(f.type_parameters().unwrap().nodes().len(), 2);
    assert_eq!(f.parameters().unwrap().nodes().len(), 2);
    let import = stmts[0].as_import_declaration();
    assert_eq!(import.module_specifier.text(), "bar");
    assert_eq!(sf.end_of_file_token.kind(), Kind::EndOfFile);
    // Positions are the wire (UTF-16) positions, as in Go's decoder.
    assert_eq!((stmts[1].pos(), stmts[1].end()), (26, 82));
}

#[test]
fn decode_non_source_file_node() {
    // A subtree encoding (Go EncodeNode / client encodeNode): the root keeps its own kind.
    let (_region, file) = parse_like_server("x.ts", "type T = { a: string; b?: number[] } | `x${1}`;");
    let alias = file.statements.nodes()[0];
    let ty = alias.as_type_alias_declaration().type_().unwrap();
    let (bytes, table) = encode_node(ty, Some(file.get())).unwrap();
    assert_eq!(&bytes[4..24], &[0u8; 20], "non-SourceFile encodings have no hash or parse options");
    assert_eq!(table.node(1), Some(ty));
    let decoded = decode_nodes(&bytes).unwrap();
    assert_eq!(decoded.root().kind(), Kind::UnionType);
    let (re, _) = encode_node(decoded.root(), None).unwrap();
    assert_eq!(structural(&bytes, true), structural(&re, true));
    assert!(decode_source_file(&bytes).is_err());
}

#[test]
fn decode_rejects_malformed_input_without_panicking() {
    assert!(matches!(decode_nodes(&[]), Err(DecodeError::TooShort(0))));
    let good = golden("complex.ts");
    let mut bad_version = good.clone();
    bad_version[3] = 8;
    assert!(matches!(decode_nodes(&bad_version), Err(DecodeError::UnsupportedVersion(8))));
    let mut bad_offsets = good.clone();
    bad_offsets[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(decode_nodes(&bad_offsets), Err(DecodeError::InvalidHeader(_))));

    // Every truncation and a deterministic set of byte/word corruptions: the decoder must return (Ok or Err),
    // never panic or loop.
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut cases = 0;
    let mut errors = 0;
    for name in ["basic.ts", "complex.ts", "jsx_component.tsx", "jsdoc_checked.js", "unicode_text.ts"] {
        let good = golden(name);
        let mut inputs: Vec<Vec<u8>> = (0..good.len()).step_by(7).map(|n| good[..n].to_vec()).collect();
        for _ in 0..1500 {
            let mut b = good.clone();
            let at = (next() as usize) % b.len();
            if next() % 2 == 0 {
                b[at] ^= 1 << (next() % 8);
            } else {
                let at = at & !3;
                if at + 4 <= b.len() {
                    let v = [0u32, 1, 2, u32::MAX, 0x7fff_ffff, (next() % 4096) as u32][(next() % 6) as usize];
                    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            inputs.push(b);
        }
        for input in inputs {
            cases += 1;
            let r = std::panic::catch_unwind(|| decode_nodes(&input).map(|d| d.root().kind()));
            match r {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => errors += 1,
                Err(_) => panic!("{name}: decoder panicked on a malformed input of {} bytes", input.len()),
            }
        }
    }
    eprintln!("malformed inputs: {cases} cases, {errors} rejected, none panicked");
    assert!(errors > cases / 10);
}

/// Go `DecodeNodes` followed by `EncodeNode(node, nil)` (`EncodeSourceFile` for a SourceFile root), run by the
/// pinned sources (gen/goprobe.sh, tests/golden/reencode), against the same in Rust, byte for byte. Covers the
/// server's SourceFile encodings and every client-encoded print input, incl. synthesized trees whose wire
/// positions are 0xFFFFFFFF (Go decodes them to int 4294967295, Rust to -1; both write 0xFFFFFFFF back).
#[test]
fn decode_reencode_matches_pinned_go() {
    let dir = crate_dir().join("tests/golden/reencode");
    let (mut identical, mut both_failed, mut failures) = (0, 0, Vec::new());
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    for f in &names {
        let (base, go) = if let Some(b) = f.strip_suffix(".goreencoded.bin") {
            (b, Ok(std::fs::read(dir.join(f)).unwrap()))
        } else if let Some(b) = f.strip_suffix(".goreencoded.err") {
            (b, Err(std::fs::read_to_string(dir.join(f)).unwrap()))
        } else {
            continue;
        };
        let input_path = [crate_dir().join(format!("tests/golden/{base}.bin")), crate_dir().join(format!("tests/golden/print/{base}.bin"))]
            .into_iter()
            .find(|p| p.exists())
            .unwrap();
        let input = std::fs::read(input_path).unwrap();
        let rust = decode_nodes(&input).map_err(|e| e.to_string()).and_then(|d| {
            let _scope = d.region().enter();
            let root = d.root();
            let r = if root.kind() == Kind::SourceFile { encode_source_file(root.as_source_file()) } else { encode_node(root, None) };
            r.map(|(b, _)| b).map_err(|e| e.to_string())
        });
        match (go, rust) {
            (Ok(go), Ok(rust)) => match first_difference(&go, &rust) {
                None => identical += 1,
                Some(d) => failures.push(format!("== {base}\n{d}")),
            },
            (Err(go), Err(rust)) if go.trim().trim_start_matches("panic: ") == rust => both_failed += 1,
            (go, rust) => failures.push(format!("== {base}: go {:?} rust {:?}", go.map(|b| b.len()), rust.map(|b| b.len()))),
        }
    }
    eprintln!("decode+re-encode vs pinned Go: {} cases, {identical} byte-identical, {both_failed} identical failures, {} mismatches", identical + both_failed + failures.len(), failures.len());
    assert!(identical + both_failed >= 30);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
