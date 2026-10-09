use tsrs_core::tspath::ComparePathsOptions;

use super::*;

fn new_gen() -> Generator {
    new_generator("main.js", "/", "/", ComparePathsOptions::default())
}

fn raw(sources: &[&str], mappings: &str, names: &[&str], sources_content: Option<Vec<Option<&str>>>) -> RawSourceMap {
    RawSourceMap {
        version: 3,
        file: "main.js".to_string(),
        source_root: "/".to_string(),
        sources: sources.iter().map(|s| s.to_string()).collect(),
        mappings: mappings.to_string(),
        names: names.iter().map(|s| s.to_string()).collect(),
        sources_content: sources_content.map(|v| v.into_iter().map(|s| s.map(str::to_string)).collect()),
    }
}

fn err(r: Result<(), String>) -> String {
    r.unwrap_err()
}

// generator_test.go:10
#[test]
fn test_source_map_generator_empty() {
    let mut r#gen = new_gen();
    assert_eq!(r#gen.raw_source_map(), raw(&[], "", &[], None));
}

// generator_test.go:25
#[test]
fn test_source_map_generator_empty_serialized() {
    let mut r#gen = new_gen();
    let actual = r#gen.string();
    let expected = r#"{"version":3,"file":"main.js","sourceRoot":"/","sources":[],"names":[],"mappings":""}"#;
    assert_eq!(actual, expected);
}

// generator_test.go:33
#[test]
fn test_source_map_generator_add_source() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    let source_map = r#gen.raw_source_map();
    assert_eq!(source_index, 0);
    assert_eq!(source_map, raw(&["main.ts"], "", &[], None));
}

// generator_test.go:50
#[test]
fn test_source_map_generator_set_source_content() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.set_source_content(source_index, "foo").unwrap();
    let source_map = r#gen.raw_source_map();
    assert_eq!(source_index, 0);
    assert_eq!(source_map, raw(&["main.ts"], "", &[], Some(vec![Some("foo")])));
}

// generator_test.go:69
#[test]
fn test_source_map_generator_set_source_content_for_second_source_only() {
    let mut r#gen = new_gen();
    r#gen.add_source("/skipped.ts");
    let source_index = r#gen.add_source("/main.ts");
    r#gen.set_source_content(source_index, "foo").unwrap();
    let source_map = r#gen.raw_source_map();
    assert_eq!(source_index, 1);
    assert_eq!(source_map, raw(&["skipped.ts", "main.ts"], "", &[], Some(vec![None, Some("foo")])));
}

// generator_test.go:89
#[test]
fn test_source_map_generator_set_source_content_source_index_out_of_range() {
    let mut r#gen = new_gen();
    assert_eq!(err(r#gen.set_source_content(-1, "")), "sourceIndex is out of range");
    assert_eq!(err(r#gen.set_source_content(0, "")), "sourceIndex is out of range");
}

// generator_test.go:96
#[test]
fn test_source_map_generator_set_source_content_for_second_source_only_serialized() {
    let mut r#gen = new_gen();
    r#gen.add_source("/skipped.ts");
    let source_index = r#gen.add_source("/main.ts");
    r#gen.set_source_content(source_index, "foo").unwrap();
    let actual = r#gen.string();
    let expected = r#"{"version":3,"file":"main.js","sourceRoot":"/","sources":["skipped.ts","main.ts"],"names":[],"mappings":"","sourcesContent":[null,"foo"]}"#;
    assert_eq!(actual, expected);
}

// generator_test.go:108
#[test]
fn test_source_map_generator_add_name() {
    let mut r#gen = new_gen();
    let name_index = r#gen.add_name("foo");
    let source_map = r#gen.raw_source_map();
    assert_eq!(name_index, 0);
    assert_eq!(source_map, raw(&[], "", &["foo"], None));
}

// generator_test.go:125
#[test]
fn test_source_map_generator_add_generated_mapping() {
    let mut r#gen = new_gen();
    r#gen.add_generated_mapping(0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&[], "A", &[], None));
}

// generator_test.go:141
#[test]
fn test_source_map_generator_add_generated_mapping_replaces_pending_source_mapping() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    r#gen.add_generated_mapping(0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map().mappings, "A");
}

// generator_test.go:151
#[test]
fn test_source_map_generator_add_generated_mapping_is_not_replaced_by_source_mapping() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_generated_mapping(0, 0).unwrap();
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map().mappings, "A");
}

// generator_test.go:161
#[test]
fn test_source_map_generator_add_generated_mapping_on_second_line_only() {
    let mut r#gen = new_gen();
    r#gen.add_generated_mapping(1, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&[], ";A", &[], None));
}

// generator_test.go:177
#[test]
fn test_source_map_generator_add_source_mapping() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAA", &[], None));
}

// generator_test.go:194
#[test]
fn test_source_map_generator_add_source_mapping_next_generated_character() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    r#gen.add_source_mapping(0, 1, source_index, 0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAA,CAAA", &[], None));
}

// generator_test.go:212
#[test]
fn test_source_map_generator_add_source_mapping_next_generated_and_source_character() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    r#gen.add_source_mapping(0, 1, source_index, 0, 1).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAA,CAAC", &[], None));
}

// generator_test.go:230
#[test]
fn test_source_map_generator_add_source_mapping_next_generated_line() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    r#gen.add_source_mapping(1, 0, source_index, 0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAA;AAAA", &[], None));
}

// generator_test.go:248
#[test]
fn test_source_map_generator_add_source_mapping_previous_source_character() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 1).unwrap();
    r#gen.add_source_mapping(0, 1, source_index, 0, 0).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAC,CAAD", &[], None));
}

// generator_test.go:266
#[test]
fn test_source_map_generator_add_named_source_mapping() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    let name_index = r#gen.add_name("foo");
    r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, name_index).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAAA", &["foo"], None));
}

// generator_test.go:284
#[test]
fn test_source_map_generator_add_named_source_mapping_with_previous_name() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    let name_index1 = r#gen.add_name("foo");
    let name_index2 = r#gen.add_name("bar");
    r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, name_index2).unwrap();
    r#gen.add_named_source_mapping(0, 1, source_index, 0, 0, name_index1).unwrap();
    assert_eq!(r#gen.raw_source_map(), raw(&["main.ts"], "AAAAC,CAAAD", &["foo", "bar"], None));
}

// generator_test.go:304
#[test]
fn test_source_map_generator_add_generated_mapping_generated_line_cannot_backtrack() {
    let mut r#gen = new_gen();
    r#gen.add_generated_mapping(1, 0).unwrap();
    assert_eq!(err(r#gen.add_generated_mapping(0, 0)), "generatedLine cannot backtrack");
}

// generator_test.go:311
#[test]
fn test_source_map_generator_add_generated_mapping_generated_character_cannot_be_negative() {
    let mut r#gen = new_gen();
    r#gen.add_generated_mapping(0, 0).unwrap();
    assert_eq!(err(r#gen.add_generated_mapping(0, -1)), "generatedCharacter cannot be negative");
}

// generator_test.go:318
#[test]
fn test_source_map_generator_add_source_mapping_generated_line_cannot_backtrack() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(1, 0, source_index, 0, 0).unwrap();
    assert_eq!(err(r#gen.add_source_mapping(0, 0, source_index, 0, 0)), "generatedLine cannot backtrack");
}

// generator_test.go:326
#[test]
fn test_source_map_generator_add_source_mapping_generated_character_cannot_be_negative() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    r#gen.add_source_mapping(0, 0, source_index, 0, 0).unwrap();
    assert_eq!(err(r#gen.add_source_mapping(0, -1, source_index, 0, 0)), "generatedCharacter cannot be negative");
}

// generator_test.go:334
#[test]
fn test_source_map_generator_add_source_mapping_source_index_is_out_of_range() {
    let mut r#gen = new_gen();
    assert_eq!(err(r#gen.add_source_mapping(0, 0, -1, 0, 0)), "sourceIndex is out of range");
    assert_eq!(err(r#gen.add_source_mapping(0, 0, 0, 0, 0)), "sourceIndex is out of range");
}

// generator_test.go:341
#[test]
fn test_source_map_generator_add_source_mapping_source_line_cannot_be_negative() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    assert_eq!(err(r#gen.add_source_mapping(0, 0, source_index, -1, 0)), "sourceLine cannot be negative");
}

// generator_test.go:348
#[test]
fn test_source_map_generator_add_source_mapping_source_character_cannot_be_negative() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    assert_eq!(err(r#gen.add_source_mapping(0, 0, source_index, 0, -1)), "sourceCharacter cannot be negative");
}

// generator_test.go:355
#[test]
fn test_source_map_generator_add_named_source_mapping_generated_line_cannot_backtrack() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    let name_index = r#gen.add_name("foo");
    r#gen.add_named_source_mapping(1, 0, source_index, 0, 0, name_index).unwrap();
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, name_index)), "generatedLine cannot backtrack");
}

// generator_test.go:364
#[test]
fn test_source_map_generator_add_named_source_mapping_generated_character_cannot_be_negative() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    let name_index = r#gen.add_name("foo");
    r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, name_index).unwrap();
    assert_eq!(err(r#gen.add_named_source_mapping(0, -1, source_index, 0, 0, name_index)), "generatedCharacter cannot be negative");
}

// generator_test.go:373
#[test]
fn test_source_map_generator_add_named_source_mapping_source_index_is_out_of_range() {
    let mut r#gen = new_gen();
    let name_index = r#gen.add_name("foo");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, -1, 0, 0, name_index)), "sourceIndex is out of range");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, 0, 0, 0, name_index)), "sourceIndex is out of range");
}

// generator_test.go:381
#[test]
fn test_source_map_generator_add_named_source_mapping_source_line_cannot_be_negative() {
    let mut r#gen = new_gen();
    let name_index = r#gen.add_name("foo");
    let source_index = r#gen.add_source("/main.ts");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, source_index, -1, 0, name_index)), "sourceLine cannot be negative");
}

// generator_test.go:389
#[test]
fn test_source_map_generator_add_named_source_mapping_source_character_cannot_be_negative() {
    let mut r#gen = new_gen();
    let name_index = r#gen.add_name("foo");
    let source_index = r#gen.add_source("/main.ts");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, source_index, 0, -1, name_index)), "sourceCharacter cannot be negative");
}

// generator_test.go:397
#[test]
fn test_source_map_generator_add_named_source_mapping_name_index_is_out_of_range() {
    let mut r#gen = new_gen();
    let source_index = r#gen.add_source("/main.ts");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, -1)), "nameIndex is out of range");
    assert_eq!(err(r#gen.add_named_source_mapping(0, 0, source_index, 0, 0, 0)), "nameIndex is out of range");
}
