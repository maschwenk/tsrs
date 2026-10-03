//! Byte-for-byte compatibility of the Rust encoder with the pinned Go encoder (goldens produced by the real
//! `tsc --api` server, see gen/oracle.mjs), plus index-table and decoder checks on the same inputs.

mod common;

use common::*;
use tsrs_api_codec::*;

#[test]
fn encode_source_file_matches_pinned_go_encoder() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, text) in fixtures() {
        let (region, file) = parse_like_server(&name, &text);
        let (bytes, table) = encode_source_file(file.get()).unwrap();
        let expected = golden(&name);
        if let Some(diff) = first_difference(&expected, &bytes) {
            failures.push(format!("== {name}\n{diff}"));
        }
        assert_eq!(table.len(), dump(&bytes).len() + 1, "{name}: index table covers every record");
        checked += 1;
        drop(region);
    }
    eprintln!("checked {checked} fixtures, {} mismatches", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn build_node_index_table_matches_encode() {
    for (name, text) in fixtures() {
        let (_region, file) = parse_like_server(&name, &text);
        let (_, encoded) = encode_source_file(file.get()).unwrap();
        let built = build_node_index_table(file.get());
        assert_eq!(built.len(), encoded.len(), "{name}");
        for i in 0..encoded.len() as u32 {
            assert_eq!(built.node(i), encoded.node(i), "{name}: index {i}");
            if let Some(n) = encoded.node(i) {
                let idx = encoded.get_index(n);
                assert!(idx != 0 && idx <= i && encoded.node(idx) == Some(n), "{name}: get_index({i}) = {idx}");
                assert_eq!(built.get_index(n), idx, "{name}: get_index (built)");
            }
        }
    }
}

#[test]
fn header_fields() {
    let (_region, file) = parse_like_server("basic.ts", "let x = 1;");
    let (mut bytes, _) = encode_source_file(file.get()).unwrap();
    assert_eq!(bytes[3], PROTOCOL_VERSION);
    set_source_file_id(&mut bytes, 0x0102_0304_0506_0708);
    set_source_file_lease(&mut bytes, 42);
    assert_eq!(&bytes[44..52], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&bytes[52..60], &42u64.to_le_bytes());
    let h = file.hash.get();
    assert_eq!(source_file_hash(file.get()), format!("{:032x}", h));
}

#[test]
fn goldens_cover_every_parser_node_kind() {
    use tsrs_api_codec::decoder::kind_from_u32;
    use tsrs_api_codec::format::*;
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in fixtures() {
        let b = golden(&name);
        let nodes = rd(&b, HEADER_OFFSET_NODES) as usize;
        for i in (nodes + NODE_SIZE..b.len()).step_by(NODE_SIZE) {
            seen.insert(rd(&b, i));
        }
    }
    // Kinds no parser output contains: checker-internal, emit-internal and the lexer-only JsxTextAllWhiteSpaces.
    let not_parsed = ["SyntheticExpression", "SyntaxList", "NotEmittedStatement", "PartiallyEmittedExpression", "SyntheticReferenceExpression", "NotEmittedTypeElement"];
    let missing: Vec<String> = (tsrs_ast::Kind::FirstNode as u32..tsrs_ast::Kind::Count as u32)
        .filter(|k| !seen.contains(k))
        .map(|k| format!("{:?}", kind_from_u32(k).unwrap()))
        .filter(|n| !not_parsed.contains(&n.as_str()))
        .collect();
    eprintln!("goldens contain {} distinct kinds", seen.len());
    assert!(missing.is_empty(), "node kinds without a Go-server golden: {missing:?}");
}

#[test]
fn generated_code_is_fresh() {
    let root = crate_dir().join("../..");
    if !root.join("ts-ref/tools/scripts/tsc/ast.json").exists() {
        eprintln!("SKIPPED: ts-ref checkout not present");
        return;
    }
    let status = std::process::Command::new("node").arg(crate_dir().join("gen/gen-codec.ts")).arg("--check").status();
    match status {
        Ok(s) => assert!(s.success(), "run node crates/tsrs_api_codec/gen/gen-codec.ts"),
        Err(_) => eprintln!("SKIPPED: node not available"),
    }
}

#[test]
fn node_handles_round_trip() {
    let (_region, file) = parse_like_server("basic.ts", "export function foo(a: string) { return a; }");
    let table = build_node_index_table(file.get());
    let mut n = 0;
    for i in 1..table.len() as u32 {
        let Some(node) = table.node(i) else { continue };
        let h = node_handle(&table, file.get(), node);
        let p = parse_node_handle(&h).unwrap();
        assert_eq!((p.index, p.path), (i, "/fixtures/basic.ts"));
        assert_eq!(p.kind, (node.kind() as u32).to_string());
        assert_eq!(resolve_node_index(&table, p.index), Some(node));
        n += 1;
    }
    assert!(n > 10);
    assert_eq!(parse_node_handle("12.80./a/b.c.ts").unwrap().path, "/a/b.c.ts");
    for bad in ["", "1", "1.2", "-1.2./a", "+1.2./a", "x.2./a", "4294967296.1./a"] {
        assert_eq!(parse_node_handle(bad), None, "{bad}");
    }
    assert_eq!(resolve_node_index(&table, 1_000_000), None);
}
