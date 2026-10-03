//! Rust-encoded bytes decoded by the real pinned JavaScript client. Needs Node.js and an installed
//! typescript@7.1.0-dev.20260930.4 package: `TSRS_CODEC_ORACLE=<dir>/node_modules/typescript cargo test -p
//! tsrs_api_codec --test node_client`. Without TSRS_CODEC_ORACLE the test reports that it was skipped.

mod common;

use std::process::Command;

use common::*;
use tsrs_api_codec::*;
use tsrs_ast::*;
use tsrs_core::stringutil::{decode_js_string_rune_bytes, encode_js_string_rune};
use tsrs_core::{alloc_str, P};

fn json_quote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::from("\"");
    let mut i = 0;
    while i < b.len() {
        let (r, size) = decode_js_string_rune_bytes(&b[i..]);
        match r {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            r if r < 0x20 || (0xD800..=0xDFFF).contains(&r) => out.push_str(&format!("\\u{:04x}", r)),
            r => out.push(char::from_u32(r as u32).unwrap()),
        }
        i += size.max(1);
    }
    out.push('"');
    out
}

fn expected_lines(node: P<Node>, depth: usize, out: &mut Vec<String>) {
    let k = node.kind();
    let text = match k {
        Kind::NumericLiteral | Kind::BigIntLiteral | Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral | Kind::TemplateHead
        | Kind::TemplateMiddle | Kind::TemplateTail | Kind::Identifier | Kind::PrivateIdentifier => format!(" text={}", json_quote(node.text())),
        _ => String::new(),
    };
    out.push(format!("{}{} {} {}{text}", " ".repeat(depth), k as u32, node.pos(), node.end()));
    node.for_each_child(&mut |c| {
        expected_lines(c, depth + 1, out);
        false
    });
}

/// A synthesized tree like the checker's `typeToTypeNode` output (no positions, factory-made, WTF-8 text).
fn synthesized(f: &NodeFactory) -> P<Node> {
    let lone = alloc_str(&format!("a{}b", encode_js_string_rune(0xD800)));
    let str_lit = f.new_literal_type_node(f.new_string_literal(lone, TokenFlags::None));
    let reference = f.new_type_reference_node(f.new_identifier("Map"), Some(f.new_node_list(vec![f.new_keyword_type_node(Kind::StringKeyword), str_lit])));
    let param = f.new_parameter_declaration(None, None, f.new_identifier("x"), Some(f.new_token(Kind::QuestionToken)), Some(f.new_keyword_type_node(Kind::NumberKeyword)), None);
    let fn_type = f.new_function_type_node(None, Some(f.new_node_list(vec![param])), Some(f.new_type_operator_node(Kind::ReadonlyKeyword, f.new_array_type_node(f.new_keyword_type_node(Kind::BooleanKeyword)))));
    let num = f.new_literal_type_node(f.new_numeric_literal("42", TokenFlags::None));
    f.new_union_type_node(f.new_node_list(vec![reference, fn_type, num]))
}

#[test]
fn rust_encodings_decode_in_the_pinned_js_client() {
    let Ok(oracle) = std::env::var("TSRS_CODEC_ORACLE") else {
        eprintln!("SKIPPED: set TSRS_CODEC_ORACLE to an installed typescript@7.1.0-dev.20260930.4 package directory");
        return;
    };
    let out_dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("codec_node_client");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    for (name, text) in fixtures() {
        let (_region, file) = parse_like_server(&name, &text);
        let (bytes, _) = encode_source_file(file.get()).unwrap();
        std::fs::write(out_dir.join(format!("{name}.rust.bin")), bytes).unwrap();
    }
    let region = tsrs_core::arena::Region::new(1 << 14);
    {
        let _scope = region.enter();
        let root = synthesized(&NodeFactory::default());
        let (bytes, _) = encode_node(root, None).unwrap();
        let mut lines = Vec::new();
        expected_lines(root, 0, &mut lines);
        std::fs::write(out_dir.join("synthetic.union.rust.bin"), bytes).unwrap();
        std::fs::write(out_dir.join("synthetic.union.expected.txt"), lines.join("\n") + "\n").unwrap();
    }
    let script = crate_dir().join("tests/node_client.mjs");
    let output = Command::new("node").arg(script).arg(&oracle).arg(&out_dir).arg(crate_dir().join("tests/golden")).output().expect("run node");
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "node client comparison failed");
    assert!(stdout.contains(" 0 mismatches"));
}
