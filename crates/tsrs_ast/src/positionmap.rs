use tsrs_core::{text_pos_from_len, TextPos, SYNTHETIC_POSITION};

// PositionMap provides bidirectional mapping between UTF-8 byte offsets (used by Go)
// and UTF-16 code unit offsets (used by JavaScript/TypeScript).
//
// For ASCII-only text, the two are identical. For text containing non-ASCII characters,
// the offsets diverge because multi-byte UTF-8 sequences map to different numbers of
// UTF-16 code units:
//   - U+0000..U+007F:   1 byte  in UTF-8, 1 code unit  in UTF-16
//   - U+0080..U+07FF:   2 bytes in UTF-8, 1 code unit  in UTF-16
//   - U+0800..U+FFFF:   3 bytes in UTF-8, 1 code unit  in UTF-16
//   - U+10000..U+10FFFF: 4 bytes in UTF-8, 2 code units in UTF-16 (surrogate pair)
#[derive(Clone, Debug, Default)]
pub struct PositionMap {
    // asciiOnly is true if the text contains only ASCII characters,
    // meaning UTF-8 byte offsets and UTF-16 code unit offsets are identical.
    ascii_only: bool,
    // For each multi-byte character, we store:
    //   - the UTF-8 byte offset of the character
    //   - the cumulative delta (utf8Offset - utf16Offset) at that character
    // This allows O(log n) conversion in either direction.
    entries: Vec<PositionMapEntry>,
}
#[derive(Clone, Copy, Debug)]
struct PositionMapEntry {
    utf8_pos: TextPos, // UTF-8 byte offset AFTER this multi-byte character
    delta: u32,        // cumulative (utf8 - utf16) offset difference after this character
}

// Rust strings are always valid UTF-8, so the lone-surrogate sentinel that Go's
// stringutil.DecodeJSStringRune special-cases cannot occur here; decoding by `char` is equivalent.
pub fn compute_position_map(text: &str) -> PositionMap {
    text_pos_from_len(text.len());
    let mut pm = PositionMap::default();
    let mut delta = 0u32;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            i += 1;
            continue;
        }
        let r = text[i..].chars().next().unwrap();
        let size = r.len_utf8();
        let utf16_size = if r as u32 >= 0x10000 { 2 } else { 1 };
        delta += u32::try_from(size).unwrap() - utf16_size;
        pm.entries.push(PositionMapEntry {
            utf8_pos: TextPos::try_from(i + size).expect("source length exceeds TextPos"),
            delta,
        });
        i += size;
    }
    pm.ascii_only = pm.entries.is_empty();
    pm
}

impl PositionMap {
    pub fn is_ascii_only(&self) -> bool {
        self.ascii_only
    }

    pub fn utf8_to_utf16(&self, utf8_offset: TextPos) -> u32 {
        if utf8_offset == SYNTHETIC_POSITION {
            return SYNTHETIC_POSITION;
        }
        if self.ascii_only {
            return utf8_offset;
        }
        // Binary search: find the last entry where utf8Pos <= utf8Offset
        let (mut lo, mut hi) = (0usize, self.entries.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.entries[mid].utf8_pos <= utf8_offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            // Before any multi-byte character
            return utf8_offset;
        }
        utf8_offset - self.entries[lo - 1].delta
    }

    pub fn utf16_to_utf8(&self, utf16_offset: u32) -> TextPos {
        if utf16_offset == SYNTHETIC_POSITION {
            return SYNTHETIC_POSITION;
        }
        if self.ascii_only {
            return utf16_offset;
        }
        // We need the last entry where (utf8Pos - delta) <= utf16Offset.
        // (utf8Pos - delta) is the UTF-16 offset of that entry's character.
        let (mut lo, mut hi) = (0usize, self.entries.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let utf16_pos = self.entries[mid].utf8_pos - self.entries[mid].delta;
            if utf16_pos <= utf16_offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return utf16_offset;
        }
        utf16_offset + self.entries[lo - 1].delta
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_map_ascii() {
        let text = "const x = 1;";
        let pm = compute_position_map(text);
        assert!(pm.is_ascii_only());
        for i in 0..=text.len() as u32 {
            assert_eq!(pm.utf8_to_utf16(i), i);
            assert_eq!(pm.utf16_to_utf8(i), i);
        }
    }

    #[test]
    fn position_map_supports_offsets_above_i32_max() {
        let utf8_pos = i32::MAX as u32 + 2;
        let pm = PositionMap {
            ascii_only: false,
            entries: vec![PositionMapEntry { utf8_pos, delta: 1 }],
        };
        assert_eq!(pm.utf8_to_utf16(utf8_pos), utf8_pos - 1);
        assert_eq!(pm.utf16_to_utf8(utf8_pos - 1), utf8_pos);
        assert_eq!(pm.utf8_to_utf16(SYNTHETIC_POSITION), SYNTHETIC_POSITION);
        assert_eq!(pm.utf16_to_utf8(SYNTHETIC_POSITION), SYNTHETIC_POSITION);
    }

    #[test]
    fn position_map_two_byte() {
        let text = "const café = 1;\nconst x = 2;";
        let pm = compute_position_map(text);
        assert!(!pm.is_ascii_only());
        for i in 0..10 {
            assert_eq!(pm.utf8_to_utf16(i), i);
        }
        assert_eq!(pm.utf8_to_utf16(9), 9);
        assert_eq!(pm.utf8_to_utf16(11), 10);
        let x_utf8 = text.rfind('x').unwrap() as u32;
        assert_eq!(pm.utf8_to_utf16(x_utf8), x_utf8 - 1);
        assert_eq!(pm.utf16_to_utf8(x_utf8 - 1), x_utf8);
    }

    #[test]
    fn position_map_four_byte() {
        let text = "const a = \"🎉\";\nconst b = 2;";
        let pm = compute_position_map(text);
        assert!(!pm.is_ascii_only());
        let b_utf8 = text.rfind('b').unwrap() as u32;
        let b_utf16 = b_utf8 - 2;
        assert_eq!(pm.utf8_to_utf16(b_utf8), b_utf16);
        assert_eq!(pm.utf16_to_utf8(b_utf16), b_utf8);
    }

    #[test]
    fn position_map_multiple_non_ascii() {
        let pm = compute_position_map("à🎉x");
        for (utf8, utf16) in [(0, 0), (2, 1), (6, 3), (7, 4)] {
            assert_eq!(pm.utf8_to_utf16(utf8), utf16);
            assert_eq!(pm.utf16_to_utf8(utf16), utf8);
        }
    }

    #[test]
    fn position_map_roundtrip() {
        let text = "let café = \"🎉\"; // naïve";
        let pm = compute_position_map(text);
        let utf16_len = pm.utf8_to_utf16(text.len() as u32);
        for i in 0..=utf16_len {
            let utf8_pos = pm.utf16_to_utf8(i);
            assert_eq!(pm.utf8_to_utf16(utf8_pos), i);
        }
    }
}
