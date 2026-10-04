// Go testutil/tsbaseline/sourcemap_baseline.go (`.js.map` baselines) and sourcemap_record_baseline.go
// (`.sourcemap.txt` baselines). The Go functions take a `CompilationResult`; the parts they read are passed in
// through `SourceMapCompilationResult`, which the emit harness fills.

use std::fmt::Write as _;
use tsrs_core::tspath;
use tsrs_core::CompilerOptions;

use crate::baseline::NO_CONTENT;
use crate::harnessutil::{HarnessOptions, TestFile};
use crate::tsbaseline::remove_test_path_prefixes;

pub(crate) struct SourceMapCompilationResult<'a> {
    // result.Maps (in the harness's output order)
    pub(crate) maps: &'a [TestFile],
    // result.DTS.Size()
    pub(crate) dts_count: usize,
    // result.GetNumberOfJSFiles(false /*includeJSON*/)
    pub(crate) number_of_js_files: usize,
    // len(result.Diagnostics)
    pub(crate) diagnostics_count: usize,
    // result.Outputs()
    pub(crate) outputs: &'a [TestFile],
    // result.Inputs()
    pub(crate) inputs: &'a [TestFile],
    // result.Program.GetSourceFile(name).OriginalText()
    pub(crate) get_program_source_text: &'a dyn Fn(&str) -> Option<String>,
    // result.GetSourceMapRecord()
    pub(crate) get_source_map_record: &'a dyn Fn() -> String,
}

// js_emit_baseline.go:149
// TODO(emit/core): shared with the `.js` baseline (js_emit_baseline.go); keep one copy once both have landed.
pub(crate) fn file_output(file: &TestFile, settings: &HarnessOptions) -> String {
    let file_name = if settings.full_emit_paths {
        remove_test_path_prefixes(&file.unit_name, false /*retainTrailingDirectorySeparator*/)
    } else {
        tspath::get_base_file_name(&file.unit_name)
    };
    format!("//// [{}]\r\n{}", file_name, file.content)
}

// sourcemap_baseline.go:18
// Returns the baseline path and content to compare, `Ok(None)` when Go returns without a baseline, and `Err` for
// Go's `t.Fatal`.
pub(crate) fn do_sourcemap_baseline(
    baseline_path: &str,
    options: &CompilerOptions,
    result: &SourceMapCompilationResult,
    harness_settings: &HarnessOptions,
) -> Result<Option<(String, String)>, String> {
    let decl_maps = options.get_are_declaration_maps_enabled();
    if options.inline_source_map.is_true() {
        if !result.maps.is_empty() && !decl_maps {
            return Err("No sourcemap files should be generated if inlineSourceMaps was set.".to_string());
        }
        Ok(None)
    } else if options.source_map.is_true() || decl_maps {
        let mut expected_map_count = 0;
        if options.source_map.is_true() {
            expected_map_count += result.number_of_js_files;
        }
        if decl_maps {
            expected_map_count += result.dts_count;
        }
        if result.maps.len() != expected_map_count {
            return Err("Number of sourcemap files should be same as js files.".to_string());
        }

        let source_map_code = if options.no_emit_on_error.is_true() && result.diagnostics_count != 0 || result.maps.is_empty() {
            NO_CONTENT.to_string()
        } else {
            let mut source_map_code_builder = String::new();
            for source_map in result.maps {
                if !source_map_code_builder.is_empty() {
                    source_map_code_builder.push_str("\r\n");
                }
                source_map_code_builder.push_str(&file_output(source_map, harness_settings));
                if !options.inline_source_map.is_true() {
                    source_map_code_builder.push_str(&create_source_map_preview_link(source_map, result));
                }
            }
            source_map_code_builder
        };

        let mut baseline_path = baseline_path.to_string();
        if tspath::file_extension_is_one_of(&baseline_path, &[tspath::EXTENSION_TS, tspath::EXTENSION_TSX]) {
            baseline_path = tspath::change_extension(&baseline_path, &format!("{}.map", tspath::EXTENSION_JS));
        }

        Ok(Some((baseline_path, source_map_code)))
    } else {
        Ok(None)
    }
}

// sourcemap_baseline.go:72
pub(crate) fn create_source_map_preview_link(source_map: &TestFile, result: &SourceMapCompilationResult) -> String {
    let sourcemap_json = match tsrs_core::json::unmarshal(&source_map.content).and_then(|v| tsrs_sourcemap::RawSourceMap::from_json(&v)) {
        Ok(sourcemap_json) => sourcemap_json,
        Err(err) => panic!("{}", err),
    };

    let output_js_file = result.outputs.iter().find(|td| td.unit_name.ends_with(&sourcemap_json.file));

    // !!! Strada uses a fallible approach to associating inputs and outputs derived from a source map output. The
    // !!! commented logic below should be used after the Strada migration is complete:

    ////inputsAndOutputs := result.GetInputsAndOutputsForFile(sourceMap.UnitName)
    ////outputJSFile := inputsAndOutputs.Js

    let Some(output_js_file) = output_js_file else {
        return String::new();
    };

    let source_tds: Vec<Option<TestFile>> = sourcemap_json
        .sources
        .iter()
        .map(|s| {
            let source_file = result.inputs.iter().find(|td| td.unit_name.ends_with(s.as_str()));
            if let Some(source_file) = source_file {
                if let Some(text) = (result.get_program_source_text)(&source_file.unit_name) {
                    return Some(TestFile { unit_name: source_file.unit_name.clone(), content: text });
                }
            }
            source_file.cloned()
        })
        .collect();
    if source_tds.iter().any(Option::is_none) {
        return String::new();
    }

    let mut hash = String::new();
    hash.push_str("\n//// https://sokra.github.io/source-map-visualization#base64,");
    hash.push_str(&base64_encode_chunk(&output_js_file.content));
    hash.push(',');
    hash.push_str(&base64_encode_chunk(&source_map.content));
    for td in source_tds.iter().flatten() {
        hash.push(',');
        hash.push_str(&base64_encode_chunk(&td.content));
    }
    hash.push('\n');
    hash
}

// sourcemap_baseline.go:124
fn base64_encode_chunk(s: &str) -> String {
    let s = query_escape(s);
    let s = match query_unescape(&s) {
        Ok(s) => s,
        Err(err) => panic!("{}", err),
    };
    base64_std_encode(&s)
}

// Go net/url QueryEscape (encodeQueryComponent mode).
fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.' || c == b'~' {
            out.push(c as char);
        } else if c == b' ' {
            out.push('+');
        } else {
            out.push('%');
            let _ = write!(out, "{:02X}", c);
        }
    }
    out
}

// Go net/url QueryUnescape.
fn query_unescape(s: &str) -> Result<Vec<u8>, String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                if i + 2 >= b.len() {
                    return Err(format!("invalid URL escape {:?}", &s[i..]));
                }
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                    _ => return Err(format!("invalid URL escape {:?}", &s[i..i + 3])),
                }
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

// Go encoding/base64 StdEncoding.EncodeToString.
fn base64_std_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let v = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(v >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(v >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(v >> 6 & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(v & 0x3f) as usize] as char } else { '=' });
    }
    out
}

// sourcemap_record_baseline.go:12
// Returns the baseline path and content to compare.
pub(crate) fn do_sourcemap_record_baseline(baseline_path: &str, options: &CompilerOptions, result: &SourceMapCompilationResult) -> (String, String) {
    let mut actual = NO_CONTENT.to_string();
    if options.source_map.is_true() || options.inline_source_map.is_true() || options.declaration_map.is_true() {
        let record = remove_test_path_prefixes(&(result.get_source_map_record)(), false /*retainTrailingDirectorySeparator*/);
        if !(options.no_emit_on_error.is_true() && result.diagnostics_count > 0) && !record.is_empty() {
            actual = record;
        }
    }

    let mut baseline_path = baseline_path.to_string();
    if tspath::file_extension_is_one_of(&baseline_path, &[tspath::EXTENSION_TS, tspath::EXTENSION_TSX]) {
        baseline_path = tspath::change_extension(&baseline_path, ".sourcemap.txt");
    }

    (baseline_path, actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_round_trip_is_identity() {
        for s in ["a b+c%d", "héllo\r\n\t~", ""] {
            assert_eq!(query_unescape(&query_escape(s)).unwrap(), s.as_bytes());
        }
    }
}
