use tsrs_core::UTF16Offset;

use super::*;

// decoder.go:10
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mapping {
    pub generated_line: i32,
    pub generated_character: UTF16Offset,
    pub source_index: SourceIndex,
    pub source_line: i32,
    pub source_character: UTF16Offset,
    pub name_index: NameIndex,
}

impl Mapping {
    // decoder.go:19
    pub fn equals(&self, other: &Mapping) -> bool {
        std::ptr::eq(self, other)
            || self.generated_line == other.generated_line
                && self.generated_character == other.generated_character
                && self.source_index == other.source_index
                && self.source_line == other.source_line
                && self.source_character == other.source_character
                && self.name_index == other.name_index
    }

    // decoder.go:28
    pub fn is_source_mapping(&self) -> bool {
        self.source_index != MISSING_SOURCE && self.source_line != MISSING_LINE_OR_COLUMN && self.source_character != MISSING_UTF16_COLUMN
    }
}

pub const MISSING_SOURCE: SourceIndex = -1;
pub const MISSING_NAME: NameIndex = -1;
pub const MISSING_LINE_OR_COLUMN: i32 = -1;
pub const MISSING_UTF16_COLUMN: UTF16Offset = -1;

// decoder.go:41
// Go allocates the returned mappings in a per-decoder arena (`mappingArena`); Mapping is a small value type here.
#[derive(Clone, Debug, Default)]
pub struct MappingsDecoder {
    mappings: String,
    done: bool,
    pos: usize,
    generated_line: i32,
    generated_character: UTF16Offset,
    source_index: SourceIndex,
    source_line: i32,
    source_character: UTF16Offset,
    name_index: NameIndex,
    error: Option<String>,
}

// decoder.go:55
pub fn decode_mappings(mappings: &str) -> MappingsDecoder {
    MappingsDecoder { mappings: mappings.to_string(), ..Default::default() }
}

impl MappingsDecoder {
    // decoder.go:59
    pub fn mappings_string(&self) -> &str {
        &self.mappings
    }

    // decoder.go:63
    pub fn pos(&self) -> usize {
        self.pos
    }

    // decoder.go:67
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // decoder.go:71
    pub fn state(&self) -> Mapping {
        self.capture_mapping(true /*hasSource*/, true /*hasName*/)
    }

    // decoder.go:75
    pub fn values(&mut self) -> impl Iterator<Item = Mapping> + '_ {
        std::iter::from_fn(move || self.next())
    }

    // decoder.go:85
    // Go returns (value, done); None is done.
    pub fn next(&mut self) -> Option<Mapping> {
        while !self.done && self.pos < self.mappings.len() {
            let ch = self.mappings.as_bytes()[self.pos];
            if ch == b';' {
                // new line
                self.generated_line += 1;
                self.generated_character = 0;
                self.pos += 1;
                continue;
            }

            if ch == b',' {
                // Next entry is on same line - no action needed
                self.pos += 1;
                continue;
            }

            let mut has_source = false;
            let mut has_name = false;
            self.generated_character = self.generated_character.wrapping_add(self.base64_vlq_format_decode() as UTF16Offset);
            if self.has_reported_error() {
                return self.stop_iterating();
            }
            if self.generated_character < 0 {
                return self.set_error_and_stop_iterating("Invalid generatedCharacter found");
            }

            if !self.is_source_mapping_segment_end() {
                has_source = true;

                self.source_index = self.source_index.wrapping_add(self.base64_vlq_format_decode() as SourceIndex);
                if self.has_reported_error() {
                    return self.stop_iterating();
                }
                if self.source_index < 0 {
                    return self.set_error_and_stop_iterating("Invalid sourceIndex found");
                }
                if self.is_source_mapping_segment_end() {
                    return self.set_error_and_stop_iterating("Unsupported Format: No entries after sourceIndex");
                }

                self.source_line = self.source_line.wrapping_add(self.base64_vlq_format_decode() as i32);
                if self.has_reported_error() {
                    return self.stop_iterating();
                }
                if self.source_line < 0 {
                    return self.set_error_and_stop_iterating("Invalid sourceLine found");
                }
                if self.is_source_mapping_segment_end() {
                    return self.set_error_and_stop_iterating("Unsupported Format: No entries after sourceLine");
                }

                self.source_character = self.source_character.wrapping_add(self.base64_vlq_format_decode() as UTF16Offset);
                if self.has_reported_error() {
                    return self.stop_iterating();
                }
                if self.source_character < 0 {
                    return self.set_error_and_stop_iterating("Invalid sourceCharacter found");
                }

                if !self.is_source_mapping_segment_end() {
                    has_name = true;
                    self.name_index = self.name_index.wrapping_add(self.base64_vlq_format_decode() as NameIndex);
                    if self.has_reported_error() {
                        return self.stop_iterating();
                    }
                    if self.name_index < 0 {
                        return self.set_error_and_stop_iterating("Invalid nameIndex found");
                    }

                    if !self.is_source_mapping_segment_end() {
                        return self.set_error_and_stop_iterating("Unsupported Error Format: Entries after nameIndex");
                    }
                }
            }

            return Some(self.capture_mapping(has_source, has_name));
        }

        self.stop_iterating()
    }

    // decoder.go:167
    fn capture_mapping(&self, has_source: bool, has_name: bool) -> Mapping {
        Mapping {
            generated_line: self.generated_line,
            generated_character: self.generated_character,
            source_index: if has_source { self.source_index } else { MISSING_SOURCE },
            source_line: if has_source { self.source_line } else { MISSING_LINE_OR_COLUMN },
            source_character: if has_source { self.source_character } else { MISSING_UTF16_COLUMN },
            name_index: if has_name { self.name_index } else { MISSING_NAME },
        }
    }

    // decoder.go:178
    fn stop_iterating(&mut self) -> Option<Mapping> {
        self.done = true;
        None
    }

    // decoder.go:183
    fn set_error(&mut self, err: &str) {
        self.error = Some(err.to_string());
    }

    // decoder.go:187
    fn set_error_and_stop_iterating(&mut self, err: &str) -> Option<Mapping> {
        self.set_error(err);
        self.stop_iterating()
    }

    // decoder.go:192
    fn has_reported_error(&self) -> bool {
        self.error.is_some()
    }

    // decoder.go:196
    fn is_source_mapping_segment_end(&self) -> bool {
        self.pos == self.mappings.len() || self.mappings.as_bytes()[self.pos] == b',' || self.mappings.as_bytes()[self.pos] == b';'
    }

    // decoder.go:200
    // Returns Go's (64-bit) int.
    fn base64_vlq_format_decode(&mut self) -> i64 {
        let mut more_digits = true;
        let mut shift_count: u32 = 0;
        let mut value: i64 = 0;
        while more_digits {
            if self.pos >= self.mappings.len() {
                self.set_error("Error in decoding base64VLQFormatDecode, past the mapping string");
                return -1;
            }

            // 6 digit number
            let current_byte = base64_format_decode(self.mappings.as_bytes()[self.pos]);
            if current_byte == -1 {
                self.set_error("Invalid character in VLQ");
                return -1;
            }

            // If msb is set, we still have more bits to continue
            more_digits = (current_byte & 32) != 0;

            // least significant 5 bits are the next msbs in the final value.
            // (A Go shift by 64 or more yields 0.)
            value |= ((current_byte & 31) as i64).checked_shl(shift_count).unwrap_or(0);
            shift_count = shift_count.saturating_add(5);
            self.pos += 1;
        }

        // Least significant bit if 1 represents negative and rest of the msb is actual absolute value
        if (value & 1) == 0 {
            // + number
            value >>= 1;
        } else {
            // - number
            value >>= 1;
            value = value.wrapping_neg();
        }

        value
    }
}

// decoder.go:238
pub(crate) fn base64_format_decode(ch: u8) -> i32 {
    match ch {
        b'A'..=b'Z' => (ch - b'A') as i32,
        b'a'..=b'z' => (ch - b'a' + 26) as i32,
        b'0'..=b'9' => (ch - b'0' + 52) as i32,
        b'+' => 62,
        b'/' => 63,
        _ => -1,
    }
}
