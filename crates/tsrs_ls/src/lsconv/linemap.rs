use std::sync::Arc;

use tsrs_core::goslices;
use tsrs_core::stringutil;
use tsrs_core::TextPos;

// linemap.go:12
pub type LSPLineStarts = Vec<TextPos>;

// linemap.go:14
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LSPLineMap {
    pub line_starts: LSPLineStarts,
    pub ascii_only: bool, // TODO(jakebailey): collect ascii-only info per line
}

// linemap.go:19
pub fn compute_lsp_line_starts(text: &str) -> Arc<LSPLineMap> {
    // This is like core.ComputeLineStarts, but only considers "\n", "\r", and "\r\n" as line breaks,
    // and reports when the text is ASCII-only.
    let bytes = text.as_bytes();
    let mut line_starts: Vec<TextPos> = Vec::with_capacity(memchr_count(bytes) + 1);
    let mut ascii_only = true;

    let text_len = bytes.len() as TextPos;
    let mut pos: TextPos = 0;
    let mut line_start: TextPos = 0;
    while pos < text_len {
        let b = bytes[pos as usize];
        if b < 0x80 {
            pos += 1;
            match b {
                b'\r' | b'\n' => {
                    if b == b'\r' && pos < text_len && bytes[pos as usize] == b'\n' {
                        pos += 1;
                    }
                    line_starts.push(line_start);
                    line_start = pos;
                }
                _ => {}
            }
        } else {
            let (_, size) = stringutil::decode_rune(&bytes[pos as usize..]);
            pos += size as TextPos;
            ascii_only = false;
        }
    }
    line_starts.push(line_start);

    Arc::new(LSPLineMap { line_starts, ascii_only })
}

fn memchr_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}

impl LSPLineMap {
    // linemap.go:56
    pub fn compute_index_of_line_start(&self, target_pos: TextPos) -> usize {
        // port of computeLineOfPosition(lineStarts: readonly number[], position: number, lowerBound?: number): number {
        let (mut line_number, ok) = goslices::binary_search_func(&self.line_starts, &target_pos, |p, t| (*p).cmp(t) as i32);
        if !ok && line_number > 0 {
            // If the actual position was not found, the binary search returns where the target line start would be inserted
            // if the target was in the slice.
            // e.g. if the line starts at [5, 10, 23, 80] and the position requested was 20
            // then the search will return (3, false).
            //
            // We want the index of the previous line start, so we subtract 1.
            line_number -= 1;
        }
        line_number
    }
}
