use std::fmt::Write as _;
use std::path::PathBuf;

use tsrs_api_codec::format::*;
use tsrs_api_codec::decoder::kind_from_u32;
use tsrs_ast::{ExternalModuleIndicatorOptions, SourceFile, SourceFileParseOptions};
use tsrs_core::arena::Region;
use tsrs_core::{tspath, P};

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every fixture (tests/fixtures); each has a golden encoding from the pinned server (gen/oracle.mjs).
pub fn fixtures() -> Vec<(String, String)> {
    let dir = crate_dir().join("tests/fixtures");
    let mut names: Vec<String> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| !n.starts_with('.')).collect();
    names.sort();
    let out: Vec<(String, String)> =
        names.into_iter().map(|n| (n.clone(), String::from_utf8(std::fs::read(dir.join(&n)).unwrap()).unwrap())).collect();
    assert_eq!(out.len(), 12, "fixture count");
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
    let i = nodes + NODE_SIZE;
    let j = 1;
    out.extend(b.get(i..).unwrap_or_default().as_chunks::<NODE_SIZE>().0.iter().enumerate().map(|(index, record)| {
        let j = j + index;
        let kind = rd(record, 0);
        let name = if kind == SYNTAX_KIND_NODE_LIST { "NodeList".to_string() } else { format!("{:?}", kind_from_u32(kind)) };
        format!(
            "#{j} {name} [{}, {}) next={} parent={} data={:#010x} flags={:#x}",
            rd(record, 4),
            rd(record, 8),
            rd(record, 12),
            rd(record, 16),
            rd(record, 20),
            rd(record, 24)
        )
    }));
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
            writeln!(msg, "header {name}: expected {e:#x} actual {a:#x}").unwrap();
        }
    }
    let (de, da) = (dump(expected), dump(actual));
    for (k, (e, a)) in de.iter().zip(da.iter()).enumerate() {
        if e != a {
            write!(msg, "first node difference at record {}:\n  expected {e}\n  actual   {a}\n", k + 1).unwrap();
            let lo = k.saturating_sub(3);
            msg += "  expected context:\n";
            msg.extend(de[lo..(k + 4).min(de.len())].iter().map(|l| format!("    {l}\n")));
            msg += "  actual context:\n";
            msg.extend(da[lo..(k + 4).min(da.len())].iter().map(|l| format!("    {l}\n")));
            return Some(msg);
        }
    }
    if de.len() != da.len() {
        writeln!(msg, "node count: expected {} actual {}", de.len(), da.len()).unwrap();
    } else {
        let pos = expected.iter().zip(actual).position(|(e, a)| e != a).unwrap_or(expected.len().min(actual.len()));
        writeln!(msg, "first differing byte at {pos} (sections: strOff {} strData {} ext {} struct {} nodes {})",
            rd(expected, 24), rd(expected, 28), rd(expected, 32), rd(expected, 36), rd(expected, 40)).unwrap();
    }
    Some(msg)
}

fn string_at(b: &[u8], idx: u32) -> String {
    let (so, sd) = (rd(b, HEADER_OFFSET_STRING_OFFSETS) as usize, rd(b, HEADER_OFFSET_STRING_DATA) as usize);
    let (s, e) = (rd(b, so + idx as usize * 4) as usize, rd(b, so + idx as usize * 4 + 4) as usize);
    format!("{:?}", String::from_utf8_lossy(&b[sd + s..sd + e]))
}

/// Encoding-independent view of a tree: one line per record with its depth, kind, range, flags, child mask /
/// commonData and the *contents* of its strings and extended data (not their table indices). Subtrees rooted
/// at `JSDoc` records are dropped (the decoders attach no JSDoc, as in Go) unless `keep_jsdoc`. The root
/// SourceFile line omits its extended data (structured metadata is not reconstructed by DecodeNodes).
pub fn structural(b: &[u8], keep_jsdoc: bool) -> Vec<String> {
    let nodes = rd(b, HEADER_OFFSET_NODES) as usize;
    let ext = rd(b, HEADER_OFFSET_EXTENDED_DATA) as usize;
    let count = (b.len() - nodes) / NODE_SIZE;
    let field = |i: usize, f: usize| rd(b, nodes + i * NODE_SIZE + f);
    let mut depth = vec![0usize; count];
    let mut out = Vec::new();
    let mut skip_below: Option<usize> = None;
    for i in 1..count {
        let parent = field(i, NODE_OFFSET_PARENT) as usize;
        depth[i] = if i == 1 { 0 } else { depth[parent] + 1 };
        if let Some(d) = skip_below {
            if depth[i] > d {
                continue;
            }
            skip_below = None;
        }
        let kind = field(i, NODE_OFFSET_KIND);
        let data = field(i, NODE_OFFSET_DATA);
        let name = if kind == SYNTAX_KIND_NODE_LIST { "NodeList".to_string() } else { format!("{:?}", kind_from_u32(kind).unwrap()) };
        if name == "JSDoc" && !keep_jsdoc {
            skip_below = Some(depth[i]);
            continue;
        }
        let detail = if kind == SYNTAX_KIND_NODE_LIST {
            format!("len={} trailingComma={}", data, field(i, NODE_OFFSET_FLAGS))
        } else {
            match data & NODE_DATA_TYPE_MASK {
                NODE_DATA_TYPE_STRING => format!("common={} text={}", (data >> 24) & 0x3f, string_at(b, data & NODE_DATA_STRING_INDEX_MASK)),
                NODE_DATA_TYPE_EXTENDED_DATA if i == 1 && name == "SourceFile" => "sourceFile".to_string(),
                NODE_DATA_TYPE_EXTENDED_DATA => {
                    let off = ext + (data & NODE_DATA_STRING_INDEX_MASK) as usize;
                    if name.starts_with("Template") {
                        format!("text={} raw={} flags={}", string_at(b, rd(b, off)), string_at(b, rd(b, off + 4)), rd(b, off + 8))
                    } else {
                        format!("text={} flags={}", string_at(b, rd(b, off)), rd(b, off + 4))
                    }
                }
                _ => format!("data={data:#010x}"),
            }
        };
        out.push(format!(
            "{}{name} [{}, {}) flags={:#x} {detail}",
            "  ".repeat(depth[i]),
            field(i, NODE_OFFSET_POS),
            field(i, NODE_OFFSET_END),
            if kind == SYNTAX_KIND_NODE_LIST { 0 } else { field(i, NODE_OFFSET_FLAGS) },
        ));
    }
    out
}

pub fn diff_lines(expected: &[String], actual: &[String]) -> Option<String> {
    if let Some(result) = expected
        .iter()
        .zip(actual)
        .enumerate()
        .find_map(|(k, (e, a))| if e != a { Some(Some(format!("line {k}:\n  expected {e}\n  actual   {a}"))) } else { None })
    {
        return result;
    }
    (expected.len() != actual.len()).then(|| format!("line count: expected {} actual {}", expected.len(), actual.len()))
}
