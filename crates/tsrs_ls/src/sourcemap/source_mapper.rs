use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_core::goslices;
use tsrs_core::json;
use tsrs_core::stringutil;
use tsrs_core::tspath;

use super::*;

// source_mapper.go:16
pub trait Host {
    fn use_case_sensitive_file_names(&self) -> bool;
    fn get_ecma_line_info(&self, file_name: &str) -> Option<Arc<ECMALineInfo>>;
    fn read_file(&self, file_name: &str) -> Option<String>;
}

// source_mapper.go:23
// Similar to `Mapping`, but position-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedPosition {
    pub(crate) generated_position: i32,
    pub(crate) source_position: i32,
    pub(crate) source_index: SourceIndex,
    pub(crate) name_index: NameIndex,
}

pub(crate) const MISSING_POSITION: i32 = -1;

impl MappedPosition {
    // source_mapper.go:34
    pub(crate) fn is_source_mapped_position(&self) -> bool {
        self.source_index != MISSING_SOURCE && self.source_position != MISSING_POSITION
    }
}

pub type SourceMappedPosition = MappedPosition;

// source_mapper.go:41
// Maps source positions to generated positions and vice versa.
#[derive(Clone, Debug)]
pub struct DocumentPositionMapper {
    use_case_sensitive_file_names: bool,

    source_file_absolute_paths: Vec<String>,
    source_to_source_index_map: FxHashMap<String, SourceIndex>,
    generated_absolute_file_path: String,

    generated_mappings: Vec<MappedPosition>,
    source_mappings: FxHashMap<SourceIndex, Vec<SourceMappedPosition>>,
}

// source_mapper.go:52
pub(crate) fn create_document_position_mapper(host: &dyn Host, source_map: &RawSourceMap, map_path: &str) -> DocumentPositionMapper {
    let map_directory = tspath::get_directory_path(map_path);
    let source_root = if !source_map.source_root.is_empty() {
        tspath::get_normalized_absolute_path(&source_map.source_root, &map_directory)
    } else {
        map_directory.clone()
    };
    let generated_absolute_file_path = tspath::get_normalized_absolute_path(&source_map.file, &map_directory);
    let source_file_absolute_paths: Vec<String> =
        source_map.sources.iter().map(|source| tspath::get_normalized_absolute_path(source, &source_root)).collect();
    let use_case_sensitive_file_names = host.use_case_sensitive_file_names();
    let mut source_to_source_index_map = FxHashMap::with_capacity_and_hasher(source_file_absolute_paths.len(), Default::default());
    for (i, source) in source_file_absolute_paths.iter().enumerate() {
        source_to_source_index_map.insert(tspath::get_canonical_file_name(source, use_case_sensitive_file_names), i as SourceIndex);
    }

    let mut decoded_mappings: Vec<MappedPosition> = Vec::new();
    let mut source_mappings: FxHashMap<SourceIndex, Vec<SourceMappedPosition>> = FxHashMap::default();

    // getDecodedMappings()
    let mut decoder = decode_mappings(&source_map.mappings);
    for mapping in decoder.values() {
        // processMapping()
        let mut generated_position = -1;
        let line_info = host.get_ecma_line_info(&generated_absolute_file_path);
        if let Some(line_info) = &line_info {
            generated_position = tsrs_scanner::compute_position_of_line_and_utf16_character(
                &line_info.line_starts,
                mapping.generated_line,
                mapping.generated_character,
                &line_info.text,
                true, /*allowEdits*/
            );
        }

        let mut source_position = -1;
        if mapping.is_source_mapping() {
            let line_info = host.get_ecma_line_info(&source_file_absolute_paths[mapping.source_index as usize]);
            if let Some(line_info) = &line_info {
                let pos = tsrs_scanner::compute_position_of_line_and_utf16_character(
                    &line_info.line_starts,
                    mapping.source_line,
                    mapping.source_character,
                    &line_info.text,
                    true, /*allowEdits*/
                );
                source_position = pos;
            }
        }

        decoded_mappings.push(MappedPosition {
            generated_position,
            source_index: mapping.source_index,
            source_position,
            name_index: mapping.name_index,
        });
    }
    if decoder.error().is_some() {
        decoded_mappings = Vec::new();
    }

    // getSourceMappings()
    for mapping in &decoded_mappings {
        if !mapping.is_source_mapped_position() {
            continue;
        }
        let source_index = mapping.source_index;
        let list = source_mappings.entry(source_index).or_default();
        list.push(SourceMappedPosition {
            generated_position: mapping.generated_position,
            source_index,
            source_position: mapping.source_position,
            name_index: mapping.name_index,
        });
    }
    for list in source_mappings.values_mut() {
        goslices::sort_func(list, |a, b| {
            assert!(a.source_index == b.source_index, "All source mappings should have the same source index");
            a.source_position - b.source_position
        });
        *list = tsrs_core::deduplicate_sorted(list, |a, b| {
            a.generated_position == b.generated_position && a.source_index == b.source_index && a.source_position == b.source_position
        });
    }

    // getGeneratedMappings()
    let mut generated_mappings = decoded_mappings;
    goslices::sort_func(&mut generated_mappings, |a, b| a.generated_position - b.generated_position);
    let generated_mappings = tsrs_core::deduplicate_sorted(&generated_mappings, |a, b| {
        a.generated_position == b.generated_position && a.source_index == b.source_index && a.source_position == b.source_position
    });

    DocumentPositionMapper {
        use_case_sensitive_file_names,
        source_file_absolute_paths,
        source_to_source_index_map,
        generated_absolute_file_path,
        generated_mappings,
        source_mappings,
    }
}

// source_mapper.go:164
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DocumentPosition {
    pub file_name: String,
    pub pos: i32,
}

// Go's methods accept a nil receiver and return nil; callers holding an Option<DocumentPositionMapper> take that path
// themselves.
impl DocumentPositionMapper {
    // source_mapper.go:169
    pub fn get_source_position(&self, loc: &DocumentPosition) -> Option<DocumentPosition> {
        if self.generated_mappings.is_empty() {
            return None;
        }

        let (target_index, _) = goslices::binary_search_func(&self.generated_mappings, &loc.pos, |m, pos| m.generated_position - pos);

        if target_index >= self.generated_mappings.len() {
            return None;
        }

        let mapping = &self.generated_mappings[target_index];
        if !mapping.is_source_mapped_position() {
            return None;
        }

        // Closest position
        Some(DocumentPosition {
            file_name: self.source_file_absolute_paths[mapping.source_index as usize].clone(),
            pos: mapping.source_position,
        })
    }

    // source_mapper.go:197
    pub fn get_generated_position(&self, loc: &DocumentPosition) -> Option<DocumentPosition> {
        let &source_index = self
            .source_to_source_index_map
            .get(&tspath::get_canonical_file_name(&loc.file_name, self.use_case_sensitive_file_names))?;
        if source_index < 0 || source_index as usize >= self.source_mappings.len() {
            return None;
        }
        let source_mappings: &[SourceMappedPosition] = self.source_mappings.get(&source_index).map(|v| v.as_slice()).unwrap_or(&[]);
        let (target_index, _) = goslices::binary_search_func(source_mappings, &loc.pos, |m, pos| m.source_position - pos);

        if target_index >= source_mappings.len() {
            return None;
        }

        let mapping = &source_mappings[target_index];
        if mapping.source_index != source_index {
            return None;
        }

        // Closest position
        Some(DocumentPosition { file_name: self.generated_absolute_file_path.clone(), pos: mapping.generated_position })
    }
}

// source_mapper.go:229
pub fn get_document_position_mapper(host: &dyn Host, generated_file_name: &str) -> Option<DocumentPositionMapper> {
    let mut map_file_name = try_get_source_mapping_url_(host, generated_file_name);
    if !map_file_name.is_empty() {
        let (base64_object, matched) = try_parse_base64_url(&map_file_name);
        if matched {
            if !base64_object.is_empty() {
                if let Some(decoded) = base64_std_decode_string(base64_object) {
                    // Go converts the bytes to a string and json.Unmarshal rejects invalid UTF-8, so invalid UTF-8
                    // ends in the same nil result.
                    return match String::from_utf8(decoded) {
                        Ok(decoded) => convert_document_to_source_mapper(host, &decoded, generated_file_name),
                        Err(_) => None,
                    };
                }
            }
            // Not a data URL we can parse, skip it
            map_file_name = String::new();
        }
    }

    let mut possible_map_locations: Vec<String> = Vec::new();
    if !map_file_name.is_empty() {
        possible_map_locations.push(map_file_name);
    }
    possible_map_locations.push(format!("{generated_file_name}.map"));
    for location in &possible_map_locations {
        let map_file_name = tspath::get_normalized_absolute_path(location, &tspath::get_directory_path(generated_file_name));
        if let Some(map_file_contents) = host.read_file(&map_file_name) {
            return convert_document_to_source_mapper(host, &map_file_contents, &map_file_name);
        }
    }
    None
}

// source_mapper.go:257
pub(crate) fn convert_document_to_source_mapper(host: &dyn Host, contents: &str, map_file_name: &str) -> Option<DocumentPositionMapper> {
    let source_map = try_parse_raw_source_map(contents);
    let source_map = match source_map {
        Some(source_map) if !source_map.sources.is_empty() && !source_map.file.is_empty() && !source_map.mappings.is_empty() => source_map,
        // invalid map
        _ => return None,
    };

    // Don't support source maps that contain inlined sources
    if source_map.sources_content.as_deref().unwrap_or(&[]).iter().any(|s| s.is_some()) {
        return None;
    }

    Some(create_document_position_mapper(host, &source_map, map_file_name))
}

// source_mapper.go:272
pub(crate) fn try_parse_raw_source_map(contents: &str) -> Option<RawSourceMap> {
    let source_map = json::unmarshal(contents).and_then(|v| RawSourceMap::from_json(&v)).ok()?;
    if source_map.version != 3 {
        return None;
    }
    Some(source_map)
}

// source_mapper.go:284
// Go `tryGetSourceMappingURL`; the trailing `_` keeps it apart from util.go's exported `TryGetSourceMappingURL`.
pub(crate) fn try_get_source_mapping_url_(host: &dyn Host, file_name: &str) -> String {
    let line_info = host.get_ecma_line_info(file_name);
    try_get_source_mapping_url(line_info.as_deref())
}

// source_mapper.go:290
// Equivalent to /^data:(?:application\/json;(?:charset=[uU][tT][fF]-8;)?base64,([A-Za-z0-9+/=]+)$)?/
pub(crate) fn try_parse_base64_url(url: &str) -> (&str, bool) {
    let Some(url) = url.strip_prefix("data:") else {
        return ("", false);
    };
    let Some(mut url) = url.strip_prefix("application/json;") else {
        return ("", true);
    };
    if let Some(rest) = url.strip_prefix("charset=") {
        // Go slices `url[:len("utf-8;")]` without a length check (and panics on a shorter string).
        if !rest.as_bytes()[.."utf-8;".len()].eq_ignore_ascii_case(b"utf-8;") {
            return ("", true);
        }
        url = &rest["utf-8;".len()..];
    }
    let Some(url) = url.strip_prefix("base64,") else {
        return ("", true);
    };
    for r in url.chars() {
        if !(stringutil::is_ascii_letter(r) || stringutil::is_digit(r) || r == '+' || r == '/' || r == '=') {
            return ("", true);
        }
    }
    (url, true)
}

// Go encoding/base64 StdEncoding.DecodeString for input already restricted to [A-Za-z0-9+/=] (so no newlines to
// skip): padded quanta of four, '=' only as "xx==" or "xxx=" in the last quantum, nonzero trailing bits accepted.
fn base64_std_decode_string(s: &str) -> Option<Vec<u8>> {
    let src = s.as_bytes();
    if src.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(src.len() / 4 * 3);
    let quanta = src.len() / 4;
    for (q, chunk) in src.chunks(4).enumerate() {
        let is_last = q + 1 == quanta;
        let padding = match (chunk[2], chunk[3]) {
            (b'=', b'=') => 2,
            (_, b'=') => 1,
            _ => 0,
        };
        if padding > 0 && !is_last {
            return None;
        }
        let mut v: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let d = if i >= 4 - padding {
                0
            } else {
                let d = base64_format_decode(c);
                if d < 0 {
                    return None;
                }
                d as u32
            };
            v = (v << 6) | d;
        }
        out.push((v >> 16) as u8);
        if padding < 2 {
            out.push((v >> 8) as u8);
        }
        if padding < 1 {
            out.push(v as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "source_mapper_test.rs"]
mod source_mapper_test;
