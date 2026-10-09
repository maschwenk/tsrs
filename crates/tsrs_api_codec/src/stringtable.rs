//! Port of the pinned `tsc/internal/api/encoder/stringtable.go`.

use tsrs_ast::Kind;
use tsrs_core::TextPos;

/// String offsets + string data sections. Strings that equal the slice of file text at their node's position are
/// stored as a range of the file text; everything else is appended after it. All offsets are UTF-8 (WTF-8) byte
/// offsets into the string data section, exactly as the pinned Go encoder writes them.
pub struct StringTable<'a> {
    file_text: &'a [u8],
    other_strings: Vec<u8>,
    /// pos/end pairs
    offsets: Vec<u32>,
}

impl<'a> StringTable<'a> {
    pub fn new(file_text: &'a str, string_count: usize) -> StringTable<'a> {
        StringTable { file_text: file_text.as_bytes(), other_strings: Vec::new(), offsets: Vec::with_capacity(string_count * 2) }
    }

    /// Go `stringTable.add`. `text` may be WTF-8 (lone surrogates encoded by `encode_js_string_rune`); it is
    /// copied byte for byte.
    pub fn add(&mut self, text: &str, kind: Kind, pos: TextPos, end: TextPos) -> u32 {
        let index = self.offsets.len() as u32;
        if kind == Kind::SourceFile {
            self.offsets.push(pos as u32);
            self.offsets.push(end as u32);
            return index;
        }
        let text = text.as_bytes();
        let length = text.len() as i64;
        let (pos, end) = (pos as i64, end as i64);
        if end - pos > 0 && end <= self.file_text.len() as i64 {
            // pos includes leading trivia, but we can usually infer the actual start of the
            // string from the kind and end
            let end_offset = matches!(kind, Kind::StringLiteral | Kind::TemplateTail | Kind::NoSubstitutionTemplateLiteral) as i64;
            let end = end - end_offset;
            let start = end - length;
            // Go would panic on a negative start (it never happens for parsed nodes); fall back to appending.
            if start >= 0 && &self.file_text[start as usize..end as usize] == text {
                self.offsets.push(start as u32);
                self.offsets.push(end as u32);
                return index;
            }
        }
        // no exact match, so we need to add it to the string table
        let offset = self.file_text.len() + self.other_strings.len();
        self.other_strings.extend_from_slice(text);
        self.offsets.push(offset as u32);
        self.offsets.push((offset + text.len()) as u32);
        index
    }

    /// Number of uint32 entries in the string offsets section.
    pub fn offsets_len(&self) -> usize {
        self.offsets.len()
    }

    pub fn string_length(&self) -> usize {
        self.file_text.len() + self.other_strings.len()
    }

    pub fn encoded_length(&self) -> usize {
        self.offsets.len() * 4 + self.string_length()
    }

    /// Appends the string offsets section followed by the string data section.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.reserve(self.encoded_length());
        for &o in &self.offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend_from_slice(self.file_text);
        out.extend_from_slice(&self.other_strings);
    }
}
