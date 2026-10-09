// sourcemap has no Go tests besides generator_test.go; these cover the decoder, the data-URL parsing and the
// position mapping end to end.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_core::tspath::ComparePathsOptions;

use super::*;

struct TestHost {
    files: FxHashMap<String, String>,
}

impl Host for TestHost {
    fn use_case_sensitive_file_names(&self) -> bool {
        true
    }
    fn get_ecma_line_info(&self, file_name: &str) -> Option<Arc<ECMALineInfo>> {
        let text = self.files.get(file_name)?;
        Some(create_ecma_line_info(text.clone(), tsrs_core::compute_ecma_line_starts(text)))
    }
    fn read_file(&self, file_name: &str) -> Option<String> {
        self.files.get(file_name).cloned()
    }
}

#[test]
fn decoder_reads_what_the_generator_writes() {
    let mut r#gen = new_generator("main.js", "", "/", ComparePathsOptions::default());
    let s = r#gen.add_source("/main.ts");
    let n = r#gen.add_name("foo");
    r#gen.add_source_mapping(0, 4, s, 1, 2).unwrap();
    r#gen.add_named_source_mapping(0, 40, s, 0, 0, n).unwrap();
    r#gen.add_generated_mapping(2, 1).unwrap();
    let raw = r#gen.raw_source_map();

    let mut decoder = decode_mappings(&raw.mappings);
    let mappings: Vec<Mapping> = decoder.values().collect();
    assert_eq!(decoder.error(), None);
    let m = |gl, gc, si, sl, sc, ni| Mapping {
        generated_line: gl,
        generated_character: gc,
        source_index: si,
        source_line: sl,
        source_character: sc,
        name_index: ni,
    };
    assert_eq!(
        mappings,
        vec![m(0, 4, 0, 1, 2, MISSING_NAME), m(0, 40, 0, 0, 0, 0), m(2, 1, MISSING_SOURCE, MISSING_LINE_OR_COLUMN, MISSING_UTF16_COLUMN, MISSING_NAME)]
    );
    assert!(mappings[0].is_source_mapping());
    assert!(!mappings[2].is_source_mapping());
}

#[test]
fn decoder_errors_stop_iteration() {
    for (mappings, error) in [
        ("A!", "Invalid character in VLQ"),
        ("g", "Error in decoding base64VLQFormatDecode, past the mapping string"),
        ("AA", "Unsupported Format: No entries after sourceIndex"),
        ("AAA", "Unsupported Format: No entries after sourceLine"),
        ("AAAAAA", "Unsupported Error Format: Entries after nameIndex"),
        ("D", "Invalid generatedCharacter found"),
    ] {
        let mut decoder = decode_mappings(mappings);
        let _: Vec<Mapping> = decoder.values().collect();
        assert_eq!(decoder.error(), Some(error), "{mappings}");
        assert!(decoder.next().is_none());
    }
}

#[test]
fn base64_url_parsing() {
    assert_eq!(try_parse_base64_url("main.js.map"), ("", false));
    assert_eq!(try_parse_base64_url("data:text/plain,x"), ("", true));
    assert_eq!(try_parse_base64_url("data:application/json;base64,eyJ9"), ("eyJ9", true));
    assert_eq!(try_parse_base64_url("data:application/json;charset=UTF-8;base64,eyJ9"), ("eyJ9", true));
    assert_eq!(try_parse_base64_url("data:application/json;charset=latin1;base64,eyJ9"), ("", true));
    assert_eq!(try_parse_base64_url("data:application/json;base64,ey J9"), ("", true));
}

#[test]
fn base64_std_decoding() {
    assert_eq!(base64_std_decode_string("eyJ9"), Some(b"{\"}".to_vec()));
    assert_eq!(base64_std_decode_string("QQ=="), Some(b"A".to_vec()));
    assert_eq!(base64_std_decode_string("QUI="), Some(b"AB".to_vec()));
    assert_eq!(base64_std_decode_string("QR=="), Some(b"A".to_vec()));
    assert_eq!(base64_std_decode_string("QQ"), None);
    assert_eq!(base64_std_decode_string("QQ=A"), None);
    assert_eq!(base64_std_decode_string("Q==="), None);
    assert_eq!(base64_std_decode_string("QQ==QQ=="), None);
}

fn test_host(map_comment: &str, map_file: Option<&str>) -> TestHost {
    let mut files = FxHashMap::default();
    files.insert("/src/main.ts".to_string(), "let x: number = 1;\nlet y = x;\n".to_string());
    files.insert("/out/main.js".to_string(), format!("let x = 1;\nlet y = x;\n{map_comment}\n"));
    if let Some(map_file) = map_file {
        files.insert("/out/main.js.map".to_string(), map_file.to_string());
    }
    TestHost { files }
}

fn generate_map() -> Generator {
    let mut r#gen = new_generator("main.js", "", "/out", ComparePathsOptions { use_case_sensitive_file_names: true, current_directory: "/".to_string() });
    let s = r#gen.add_source("/src/main.ts");
    r#gen.add_source_mapping(0, 0, s, 0, 0).unwrap();
    r#gen.add_source_mapping(0, 4, s, 0, 4).unwrap();
    r#gen.add_source_mapping(1, 0, s, 1, 0).unwrap();
    r#gen.add_source_mapping(1, 8, s, 1, 8).unwrap();
    r#gen
}

#[test]
fn document_position_mapper_from_map_file() {
    let map = generate_map().string();
    assert!(map.contains(r#""sources":["../src/main.ts"]"#), "{map}");
    let host = test_host("//# sourceMappingURL=main.js.map", Some(&map));
    let mapper = get_document_position_mapper(&host, "/out/main.js").unwrap();

    // "y" in main.js (line 1, char 4) -> closest mapping at or after it is (1, 8) -> main.ts line 1 char 8.
    let pos = mapper.get_source_position(&DocumentPosition { file_name: "/out/main.js".to_string(), pos: 15 }).unwrap();
    assert_eq!(pos, DocumentPosition { file_name: "/src/main.ts".to_string(), pos: 19 + 8 });

    let pos = mapper.get_generated_position(&DocumentPosition { file_name: "/src/main.ts".to_string(), pos: 4 }).unwrap();
    assert_eq!(pos, DocumentPosition { file_name: "/out/main.js".to_string(), pos: 4 });

    assert!(mapper.get_generated_position(&DocumentPosition { file_name: "/src/other.ts".to_string(), pos: 0 }).is_none());
    assert!(mapper.get_source_position(&DocumentPosition { file_name: "/out/main.js".to_string(), pos: 1000 }).is_none());
}

#[test]
fn document_position_mapper_from_data_url() {
    let url = generate_map().base64_data_url();
    let host = test_host(&format!("//# sourceMappingURL={url}"), None);
    let mapper = get_document_position_mapper(&host, "/out/main.js").unwrap();
    let pos = mapper.get_generated_position(&DocumentPosition { file_name: "/src/main.ts".to_string(), pos: 19 }).unwrap();
    assert_eq!(pos, DocumentPosition { file_name: "/out/main.js".to_string(), pos: 11 });
}

#[test]
fn document_position_mapper_rejects_inlined_sources() {
    let mut r#gen = generate_map();
    r#gen.set_source_content(0, "let x: number = 1;\n").unwrap();
    let host = test_host("", Some(&r#gen.string()));
    assert!(get_document_position_mapper(&host, "/out/main.js").is_none());
}
