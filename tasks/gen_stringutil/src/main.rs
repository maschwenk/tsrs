use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const UNICODE_VERSION: &str = "15.1.0";
const INPUTS: [(&str, &str); 3] = [
    (
        "UnicodeData.txt",
        "2fc713e6a31a87c4850a37fe2caffa4218180fadb5de86b43a143ddb4581fb86",
    ),
    (
        "SpecialCasing.txt",
        "55a477efd933a52cd27e6a9bf70265bb2d8814af31aab07767abc8eb421f27ef",
    ),
    (
        "DerivedCoreProperties.txt",
        "f55d0db69123431a7317868725b1fcbf1eab6b265d756d1bd7f0f6d9f9ee108b",
    ),
];

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    strings: [String; 3],
    condition: &'static str,
}

impl Entry {
    fn identity(key: u32) -> Self {
        let text = char::from_u32(key).unwrap().to_string();
        Self {
            strings: [text.clone(), text, String::new()],
            condition: "None",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Range {
    lo: u32,
    hi: u32,
    stride: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct RangeTable {
    r16: Vec<Range>,
    r32: Vec<Range>,
    latin_offset: usize,
}

#[derive(Clone, Copy)]
struct Span {
    offset: u16,
    length: u8,
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut data_dir = root.join("target/unicode").join(UNICODE_VERSION);
    let mut check = false;
    let mut go_path = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--check" {
            check = true;
        } else if arg == "--verify-go" {
            go_path = Some(PathBuf::from(
                args.next().ok_or("--verify-go needs a path")?,
            ));
        } else if arg == "--ucd-dir" {
            data_dir = PathBuf::from(args.next().ok_or("--ucd-dir needs a path")?);
        } else {
            return Err(format!("unknown argument: {}", arg.to_string_lossy()).into());
        }
    }

    let unicode_data = load_input(&data_dir, INPUTS[0])?;
    let special_casing = load_input(&data_dir, INPUTS[1])?;
    let properties = load_input(&data_dir, INPUTS[2])?;
    let entries = build_entries(&unicode_data, &special_casing);
    let cased = range_table(&properties, "Cased");
    let case_ignorable = range_table(&properties, "Case_Ignorable");
    if let Some(path) = &go_path {
        verify_go(
            &fs::read_to_string(path)?,
            &entries,
            &cased,
            &case_ignorable,
        );
    }
    let (output, pool_size) = render(&entries, &cased, &case_ignorable);
    let destination = root.join("crates/tsrs_core/src/stringutil/js_case_generated.rs");
    if check {
        if fs::read_to_string(destination)? != output {
            return Err("stale casing table: run cargo run --locked -p tsrs_gen_stringutil".into());
        }
    } else {
        fs::write(destination, output)?;
    }
    println!(
        "{} casing entries; {pool_size} pooled UTF-8 bytes{}",
        entries.len(),
        if go_path.is_some() {
            "; all mappings and ranges match Go"
        } else {
            ""
        }
    );
    Ok(())
}

fn load_input(directory: &Path, (name, digest): (&str, &str)) -> Result<String> {
    let path = directory.join(name);
    if !path.exists() {
        fs::create_dir_all(directory)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let status = Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--retry",
                "3",
            ])
            .arg(format!(
                "https://www.unicode.org/Public/{UNICODE_VERSION}/ucd/{name}"
            ))
            .arg("--output")
            .arg(&temporary)
            .status()?;
        let result = if status.success() {
            verify_digest(&fs::read(&temporary)?, digest, name)
                .and_then(|()| Ok(fs::rename(&temporary, &path)?))
        } else {
            Err(format!("could not download {name}: {status}").into())
        };
        // A failed download must not become a cached generator input.
        if temporary.exists() {
            fs::remove_file(temporary)?;
        }
        result?;
    }
    let bytes = fs::read(path)?;
    verify_digest(&bytes, digest, name)?;
    Ok(String::from_utf8(bytes)?)
}

fn verify_digest(bytes: &[u8], expected: &str, name: &str) -> Result<()> {
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err(format!("{name}: SHA-256 mismatch for Unicode {UNICODE_VERSION}").into());
    }
    Ok(())
}

fn hex(text: &str) -> u32 {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).unwrap()
}

fn mapping(text: &str) -> String {
    text.split_whitespace()
        .map(|cp| char::from_u32(hex(cp)).unwrap())
        .collect()
}

fn build_entries(unicode_data: &str, special_casing: &str) -> BTreeMap<u32, Entry> {
    let mut entries = BTreeMap::new();
    for line in unicode_data.lines() {
        let fields: Vec<_> = line.split(';').collect();
        assert_eq!(fields.len(), 15);
        if fields[12].is_empty() && fields[13].is_empty() {
            continue;
        }
        let key = hex(fields[0]);
        let mut entry = Entry::identity(key);
        for (index, field) in [(0, fields[13]), (1, fields[12])] {
            if !field.is_empty() {
                entry.strings[index] = mapping(field);
            }
        }
        assert!(entries.insert(key, entry).is_none());
    }
    for line in special_casing.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split(';').map(str::trim).collect();
        // ECMAScript default casing excludes locale-specific rules.
        if !matches!(fields[4], "" | "Final_Sigma") {
            continue;
        }
        let key = hex(fields[0]);
        let entry = entries.entry(key).or_insert_with(|| Entry::identity(key));
        if fields[4] == "Final_Sigma" {
            entry.strings[2] = mapping(fields[1]);
            entry.condition = "FinalSigma";
        } else {
            entry.strings[0] = mapping(fields[1]);
            entry.strings[1] = mapping(fields[3]);
        }
    }
    entries
}

// Match the upstream stride grouping, splitting BMP and astral code points first.
fn stride_ranges(points: &[u32]) -> Vec<Range> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < points.len() {
        let lo = points[i];
        let stride = points.get(i + 1).map_or(1, |next| next - lo);
        let mut hi = lo;
        i += 1;
        while i < points.len() && points[i] == hi + stride {
            hi = points[i];
            i += 1;
        }
        ranges.push(Range { lo, hi, stride });
    }
    ranges
}

fn range_table(properties: &str, property: &str) -> RangeTable {
    let mut points = BTreeSet::new();
    for line in properties.lines() {
        let line = line.split('#').next().unwrap();
        let mut fields = line.split(';').map(str::trim);
        let range = fields.next().unwrap();
        if fields.next() != Some(property) {
            continue;
        }
        let (lo, hi) = range.split_once("..").unwrap_or((range, range));
        points.extend(hex(lo)..=hex(hi));
    }
    assert!(!points.is_empty(), "missing {property} property");
    let points: Vec<_> = points.into_iter().collect();
    let split = points.partition_point(|&point| point <= 0xFFFF);
    let r16 = stride_ranges(&points[..split]);
    let r32 = stride_ranges(&points[split..]);
    for range in &r16 {
        assert!(
            u16::try_from(range.lo).is_ok()
                && u16::try_from(range.hi).is_ok()
                && u16::try_from(range.stride).is_ok()
        );
    }
    let expanded = r16
        .iter()
        .chain(&r32)
        .flat_map(|r| (r.lo..=r.hi).step_by(r.stride as usize));
    assert!(expanded.eq(points), "property ranges did not round-trip");
    let latin_offset = r16.iter().take_while(|r| r.hi <= 0xFF).count();
    RangeTable {
        r16,
        r32,
        latin_offset,
    }
}

// The Go file is an optional independent oracle, never an input to generation.
fn verify_go(
    go: &str,
    entries: &BTreeMap<u32, Entry>,
    cased: &RangeTable,
    case_ignorable: &RangeTable,
) {
    let mut count = 0;
    let mut previous = None;
    for line in go
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("0x"))
    {
        let (key, fields) = line.split_once(':').unwrap();
        let key = hex(key);
        assert!(
            previous.is_none_or(|previous| previous < key),
            "Go mappings must be sorted"
        );
        previous = Some(key);
        let fields = fields
            .trim()
            .strip_prefix('{')
            .unwrap()
            .strip_suffix("},")
            .unwrap();
        let mut strings = [String::new(), String::new(), String::new()];
        let mut condition = None;
        for field in fields.split(", ") {
            let (name, value) = field.split_once(": ").unwrap();
            match name {
                "lower" => strings[0] = go_string(value),
                "upper" => strings[1] = go_string(value),
                "conditionalLower" => strings[2] = go_string(value),
                "condition" => {
                    condition = Some(match value {
                        "specialCasingConditionNone" => "None",
                        "specialCasingConditionFinalSigma" => "FinalSigma",
                        _ => panic!("unsupported Go casing condition: {value}"),
                    });
                }
                _ => panic!("unknown Go casing field: {name}"),
            }
        }
        let original = Entry {
            strings,
            condition: condition.unwrap(),
        };
        assert_eq!(
            entries.get(&key),
            Some(&original),
            "Unicode mapping differs from Go at U+{key:04X}"
        );
        count += 1;
    }
    assert_eq!(
        entries.len(),
        count,
        "Unicode mapping count differs from Go"
    );
    for (name, table) in [
        ("unicodeCasedRanges", cased),
        ("unicodeCaseIgnorableRanges", case_ignorable),
    ] {
        let start = format!("var {name} = &unicode.RangeTable{{");
        let body = go.split_once(start.as_str()).unwrap().1;
        let mut original = RangeTable {
            r16: Vec::new(),
            r32: Vec::new(),
            latin_offset: 0,
        };
        let mut astral = false;
        for line in body.lines() {
            if line == "}" || line == "}\r" {
                break;
            }
            let line = line.trim();
            if line.starts_with("R32:") {
                astral = true;
            } else if let Some(offset) = line.strip_prefix("LatinOffset: ") {
                original.latin_offset = offset.trim_end_matches(',').parse().unwrap();
            } else if let Some(range) = line.strip_prefix('{') {
                let fields: Vec<_> = range.trim_end_matches("},").split(", ").collect();
                assert_eq!(fields.len(), 3);
                let range = Range {
                    lo: hex(fields[0]),
                    hi: hex(fields[1]),
                    stride: fields[2].parse().unwrap(),
                };
                if astral {
                    &mut original.r32
                } else {
                    &mut original.r16
                }
                .push(range);
            }
        }
        assert_eq!(table, &original, "{name}");
    }
}

fn go_string(literal: &str) -> String {
    let mut remaining = literal
        .strip_prefix('"')
        .unwrap()
        .strip_suffix('"')
        .unwrap();
    let mut output = String::new();
    while !remaining.is_empty() {
        let digits = if remaining.starts_with("\\u") { 4 } else { 8 };
        assert!(remaining.starts_with("\\u") || remaining.starts_with("\\U"));
        output.push(char::from_u32(hex(&remaining[2..2 + digits])).unwrap());
        remaining = &remaining[2 + digits..];
    }
    output
}

fn intern(text: &str, strings: &mut BTreeMap<String, Span>, pool: &mut String) -> Span {
    if let Some(span) = strings.get(text) {
        return *span;
    }
    let span = Span {
        offset: u16::try_from(pool.len()).expect("casing offset exceeds u16"),
        length: u8::try_from(text.len()).expect("casing length exceeds u8"),
    };
    strings.insert(text.to_owned(), span);
    pool.push_str(text);
    assert!(
        pool.len() <= usize::from(u16::MAX) + 1,
        "casing pool exceeds u16 address space"
    );
    span
}

fn render(
    entries: &BTreeMap<u32, Entry>,
    cased: &RangeTable,
    case_ignorable: &RangeTable,
) -> (String, usize) {
    let mut strings = BTreeMap::new();
    let mut pool = String::new();
    intern("", &mut strings, &mut pool);
    let records: Vec<_> = entries
        .iter()
        .map(|(key, entry)| {
            (
                *key,
                entry,
                entry
                    .strings
                    .each_ref()
                    .map(|text| intern(text, &mut strings, &mut pool)),
            )
        })
        .collect();
    for (_, entry, spans) in &records {
        for (text, span) in entry.strings.iter().zip(spans) {
            let start = usize::from(span.offset);
            let end = start + usize::from(span.length);
            assert_eq!(
                pool.get(start..end),
                Some(text.as_str()),
                "casing string did not round-trip"
            );
        }
    }
    let mut output = format!(
        "// Code generated by tasks/gen_stringutil from Unicode {UNICODE_VERSION} UCD. DO NOT EDIT.\n\n"
    );
    output.push_str(include_str!("header.rs.txt"));
    writeln!(
        output,
        "// {} distinct strings, {} bytes; generation validates every slice.",
        strings.len(),
        pool.len()
    )
    .unwrap();
    output.push_str("pub(crate) static CASING_STRINGS: &str = concat!(\n");
    let points: Vec<_> = pool.chars().collect();
    for chunk in points.chunks(16) {
        output.push_str("    \"");
        for &ch in chunk {
            write!(output, "\\u{{{:X}}}", u32::from(ch)).unwrap();
        }
        output.push_str("\",\n");
    }
    output
        .push_str(");\n\n// Sorted by code point; looked up with binary search (Go uses a map).\n");
    output.push_str(
        "pub(crate) static SPECIAL_CASING_MAPPINGS: &[(u32, SpecialCasingMapping)] = &[\n",
    );
    for (key, entry, spans) in records {
        writeln!(output, "    (0x{key:X}, SpecialCasingMapping {{ offsets: [{}, {}, {}], lengths: [{}, {}, {}], condition: SpecialCasingCondition::{} }}),",
            spans[0].offset, spans[1].offset, spans[2].offset, spans[0].length, spans[1].length, spans[2].length, entry.condition).unwrap();
    }
    output.push_str("];\n\n");
    render_ranges(&mut output, "UNICODE_CASED_RANGES", cased);
    render_ranges(&mut output, "UNICODE_CASE_IGNORABLE_RANGES", case_ignorable);
    (output, pool.len())
}

fn render_ranges(output: &mut String, name: &str, table: &RangeTable) {
    for (kind, ranges, rust_type) in [
        ("R16", &table.r16, "Range16"),
        ("R32", &table.r32, "Range32"),
    ] {
        writeln!(output, "static {name}_{kind}: &[{rust_type}] = &[").unwrap();
        for Range { lo, hi, stride } in ranges {
            writeln!(
                output,
                "    {rust_type} {{ lo: 0x{lo:X}, hi: 0x{hi:X}, stride: {stride} }},"
            )
            .unwrap();
        }
        output.push_str("];\n");
    }
    writeln!(output, "pub(crate) static {name}: RangeTable = RangeTable {{ r16: {name}_R16, r32: {name}_R32, latin_offset: {} }};\n", table.latin_offset).unwrap();
}
