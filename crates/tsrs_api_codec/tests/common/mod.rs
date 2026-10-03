#![allow(dead_code)]

use std::path::PathBuf;

use tsrs_api_codec::format::*;
use tsrs_api_codec::decoder::kind_from_u32;
use tsrs_ast::{ExternalModuleIndicatorOptions, SourceFile, SourceFileParseOptions};
use tsrs_core::arena::Region;
use tsrs_core::{tspath, P};

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Fixture names with a golden encoding from the pinned server (tests/golden/manifest.json).
pub fn fixtures() -> Vec<(String, String)> {
    let manifest = std::fs::read_to_string(crate_dir().join("tests/golden/manifest.json")).unwrap();
    let mut out = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix('"') {
            if let Some((name, tail)) = rest.split_once("\": {").filter(|(n, _)| *n != "files") {
                let _ = tail;
                let bytes = std::fs::read(crate_dir().join("tests/fixtures").join(name)).unwrap();
                out.push((name.to_string(), String::from_utf8(bytes).unwrap()));
            }
        }
    }
    assert!(out.len() >= 9, "manifest lists {} fixtures", out.len());
    out
}

pub fn golden(name: &str) -> Vec<u8> {
    std::fs::read(crate_dir().join("tests/golden").join(format!("{name}.bin"))).unwrap()
}

pub fn client_golden(name: &str) -> Vec<u8> {
    std::fs::read(crate_dir().join("tests/golden").join(format!("{name}.client.bin"))).unwrap()
}

/// Parses and binds `text` the way the pinned server's `createSourceFile` does (snapshothost.AcquireSourceFile →
/// parse cache: ParseSourceFile, Hash = xxh3-128 of the content, BindSourceFile), in a region the caller owns.
pub fn parse_like_server(name: &str, text: &str) -> (Region, P<SourceFile>) {
    let file_name = format!("/fixtures/{name}");
    let region = Region::new(1 << 16);
    let file = {
        let _scope = region.enter();
        let opts = SourceFileParseOptions {
            file_name: file_name.clone(),
            path: tspath::Path(file_name.as_str().into()),
            external_module_indicator_options: ExternalModuleIndicatorOptions::default(),
        };
        let kind = tsrs_core::ensure_script_kind_from_file_name(&file_name);
        let file = tsrs_parser::parse_source_file(opts, text, kind);
        file.hash.set(xxhash_rust::xxh3::xxh3_128(text.as_bytes()));
        tsrs_binder::bind_source_file(file);
        file
    };
    (region, file)
}

pub fn rd(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// One line per node record, like the pinned encoder_test.go `formatEncodedSourceFile` plus data/flags.
pub fn dump(b: &[u8]) -> Vec<String> {
    let nodes = rd(b, HEADER_OFFSET_NODES) as usize;
    let mut out = Vec::new();
    let mut i = nodes + NODE_SIZE;
    let mut j = 1;
    while i + NODE_SIZE <= b.len() {
        let kind = rd(b, i);
        let name = if kind == SYNTAX_KIND_NODE_LIST { "NodeList".to_string() } else { format!("{:?}", kind_from_u32(kind)) };
        out.push(format!(
            "#{j} {name} [{}, {}) next={} parent={} data={:#010x} flags={:#x}",
            rd(b, i + 4),
            rd(b, i + 8),
            rd(b, i + 12),
            rd(b, i + 16),
            rd(b, i + 20),
            rd(b, i + 24)
        ));
        i += NODE_SIZE;
        j += 1;
    }
    out
}

/// Describes the first difference between two encodings (None when identical).
pub fn first_difference(expected: &[u8], actual: &[u8]) -> Option<String> {
    if expected == actual {
        return None;
    }
    let mut msg = format!("lengths: expected {} actual {}\n", expected.len(), actual.len());
    for (name, off) in [
        ("metadata", HEADER_OFFSET_METADATA),
        ("hash0", HEADER_OFFSET_HASH_LO0),
        ("hash1", HEADER_OFFSET_HASH_LO1),
        ("hash2", HEADER_OFFSET_HASH_HI0),
        ("hash3", HEADER_OFFSET_HASH_HI1),
        ("parseOptions", HEADER_OFFSET_PARSE_OPTIONS),
        ("stringOffsets", HEADER_OFFSET_STRING_OFFSETS),
        ("stringData", HEADER_OFFSET_STRING_DATA),
        ("extendedData", HEADER_OFFSET_EXTENDED_DATA),
        ("structuredData", HEADER_OFFSET_STRUCTURED_DATA),
        ("nodes", HEADER_OFFSET_NODES),
    ] {
        let (e, a) = (rd(expected, off), rd(actual, off));
        if e != a {
            msg += &format!("header {name}: expected {e:#x} actual {a:#x}\n");
        }
    }
    let (de, da) = (dump(expected), dump(actual));
    for (k, (e, a)) in de.iter().zip(da.iter()).enumerate() {
        if e != a {
            msg += &format!("first node difference at record {}:\n  expected {e}\n  actual   {a}\n", k + 1);
            let lo = k.saturating_sub(3);
            msg += "  expected context:\n";
            for l in &de[lo..(k + 4).min(de.len())] {
                msg += &format!("    {l}\n");
            }
            msg += "  actual context:\n";
            for l in &da[lo..(k + 4).min(da.len())] {
                msg += &format!("    {l}\n");
            }
            return Some(msg);
        }
    }
    if de.len() != da.len() {
        msg += &format!("node count: expected {} actual {}\n", de.len(), da.len());
    } else {
        let pos = expected.iter().zip(actual).position(|(e, a)| e != a).unwrap_or(expected.len().min(actual.len()));
        msg += &format!("first differing byte at {pos} (sections: strOff {} strData {} ext {} struct {} nodes {})\n",
            rd(expected, 24), rd(expected, 28), rd(expected, 32), rd(expected, 36), rd(expected, 40));
    }
    Some(msg)
}
