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
