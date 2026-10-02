// test_parser against Go's fourslash.ParseTestData (tools/oracle/fourslash-parser): files, markers (positions, LSP
// positions, names, object data), named markers, ranges (with their markers), symlinks and global options.

use std::panic::{self, AssertUnwindSafe};

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::{self, Value};

use crate::test_parser::{parse_test_data, TestData};
use crate::testing::T;

fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v {
        Value::Object(o) => o.get(name).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => panic!("not a string: {v:?}"),
    }
}

fn n(v: &Value) -> i64 {
    match v {
        Value::Number(n) => *n as i64,
        _ => panic!("not a number: {v:?}"),
    }
}

fn arr(v: &Value) -> &[Value] {
    match v {
        Value::Array(a) => a,
        _ => &[],
    }
}

fn string_map(v: &Value) -> OrderedMap<String, String> {
    match v {
        Value::Object(o) => o.iter().map(|(k, v)| (k.clone(), s(v))).collect(),
        _ => OrderedMap::default(),
    }
}

fn pos(v: &Value) -> (i64, i64) {
    (n(field(v, "line")), n(field(v, "character")))
}

fn check_case(expected: &Value) -> Result<(), String> {
    let test = s(field(expected, "test"));
    let content = s(field(expected, "content"));
    let file_name = s(field(expected, "fileName"));
    let t = T::new(&test, "");
    let r = panic::catch_unwind(AssertUnwindSafe(|| parse_test_data(&t, &content, &file_name)));
    let want_error = matches!(field(expected, "error"), Value::Bool(true));
    let td: TestData = match r {
        Ok(td) => td,
        Err(_) => {
            if want_error && t.failed() {
                return Ok(());
            }
            return Err(format!("{test}: Rust failed ({:?}), Go did not", t.logs()));
        }
    };
    if want_error {
        return Err(format!("{test}: Go failed, Rust did not"));
    }

    let files: Vec<(String, String)> = td.files.iter().map(|f| (f.file_name.clone(), f.content.clone())).collect();
    let want_files: Vec<(String, String)> = arr(field(expected, "files")).iter().map(|f| (s(field(f, "fileName")), s(field(f, "content")))).collect();
    if files != want_files {
        return Err(format!("{test}: files differ:\n  rust {files:?}\n  go   {want_files:?}"));
    }

    let want_markers = arr(field(expected, "markers"));
    if td.markers.len() != want_markers.len() {
        return Err(format!("{test}: {} markers, Go has {}", td.markers.len(), want_markers.len()));
    }
    for (m, w) in td.markers.iter().zip(want_markers) {
        let got = (m.file_name.clone(), m.position as i64, (m.ls_position.line as i64, m.ls_position.character as i64), m.name.clone());
        let name = match field(w, "name") {
            Value::Null => None,
            v => Some(s(v)),
        };
        let want = (s(field(w, "fileName")), n(field(w, "position")), pos(field(w, "ls")), name);
        if got != want {
            return Err(format!("{test}: marker differs:\n  rust {got:?}\n  go   {want:?}"));
        }
        let got_data = if m.data.is_empty() { Value::Null } else { Value::Object(m.data.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()) };
        if got_data != *field(w, "data") {
            return Err(format!("{test}: marker data differs:\n  rust {got_data:?}\n  go   {:?}", field(w, "data")));
        }
    }

    let mut names: Vec<String> = td.marker_positions.keys().cloned().collect();
    let mut want_names: Vec<String> = arr(field(expected, "markerNames")).iter().map(s).collect();
    names.sort();
    want_names.sort();
    if names != want_names {
        return Err(format!("{test}: marker names differ:\n  rust {names:?}\n  go   {want_names:?}"));
    }

    let want_ranges = arr(field(expected, "ranges"));
    if td.ranges.len() != want_ranges.len() {
        return Err(format!("{test}: {} ranges, Go has {}", td.ranges.len(), want_ranges.len()));
    }
    for (r, w) in td.ranges.iter().zip(want_ranges) {
        let marker_index = match &r.marker {
            None => -1,
            Some(m) => td.markers.iter().position(|x| std::sync::Arc::ptr_eq(x, m)).map(|i| i as i64).unwrap_or(-2),
        };
        let got = (
            r.file_name.clone(),
            r.range.pos() as i64,
            r.range.end() as i64,
            (r.ls_range.start.line as i64, r.ls_range.start.character as i64),
            (r.ls_range.end.line as i64, r.ls_range.end.character as i64),
            marker_index,
        );
        let want = (s(field(w, "fileName")), n(field(w, "pos")), n(field(w, "end")), pos(field(w, "start")), pos(field(w, "stop")), n(field(w, "marker")));
        if got != want {
            return Err(format!("{test}: range differs:\n  rust {got:?}\n  go   {want:?}"));
        }
    }

    if td.symlinks != string_map(field(expected, "symlinks")) {
        return Err(format!("{test}: symlinks differ: {:?} vs {:?}", td.symlinks, field(expected, "symlinks")));
    }
    if td.global_options != string_map(field(expected, "globalOptions")) {
        return Err(format!("{test}: global options differ: {:?} vs {:?}", td.global_options, field(expected, "globalOptions")));
    }
    Ok(())
}

#[test]
fn test_parser_oracle() {
    crate::runner::install_panic_hook();
    let path = std::env::var("TSRS_FOURSLASH_PARSER_ORACLE")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/parser_oracle_sample.jsonl").to_string());
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut failures = Vec::new();
    let mut count = 0;
    for line in data.lines() {
        let v = json::unmarshal(line).expect("oracle line");
        count += 1;
        if let Err(e) = check_case(&v) {
            failures.push(e);
        }
    }
    assert!(count > 0);
    if !failures.is_empty() {
        panic!("{} of {} parser cases differ from Go:\n{}", failures.len(), count, failures.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
    }
    eprintln!("{count} parser cases match Go");
}

// Unit cases from the Go parser's behavior (positions after stripped markers and ranges).
#[test]
fn test_parser_markers_and_ranges() {
    let t = T::new("TestSample", "");
    let td = parse_test_data(&t, "// @Filename: /a.ts\nlet [|/*m*/x|] = 1;\n/*n*/y", "sample.ts");
    assert_eq!(td.files.len(), 1);
    assert_eq!(td.files[0].content, "let x = 1;\ny");
    let m = &td.marker_positions["m"];
    assert_eq!(m.position, 4);
    let nm = &td.marker_positions["n"];
    assert_eq!((nm.ls_position.line, nm.ls_position.character), (1, 0));
    assert_eq!(td.ranges.len(), 1);
    assert_eq!((td.ranges[0].range.pos(), td.ranges[0].range.end()), (4, 5));
    assert!(std::sync::Arc::ptr_eq(td.ranges[0].marker.as_ref().unwrap(), m));
}
