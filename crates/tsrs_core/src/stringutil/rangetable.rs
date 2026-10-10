// Equivalent of Go's unicode.RangeTable / unicode.Is, used by the generated tables.

pub type Rune = i32;

/// Anything usable where Go takes a `rune`: `char`, `i32` or `u32`.
pub trait AsRune: Copy {
    fn as_rune(self) -> Rune;

    /// Preserve a decoded character, or validate an integer rune before converting it.
    #[inline]
    fn as_char(self) -> Option<char> {
        char::from_u32(self.as_rune() as u32)
    }
}

impl AsRune for char {
    #[inline]
    fn as_rune(self) -> Rune {
        self as Rune
    }

    #[inline]
    fn as_char(self) -> Option<char> {
        Some(self)
    }
}

impl AsRune for i32 {
    #[inline]
    fn as_rune(self) -> Rune {
        self
    }
}

impl AsRune for u32 {
    #[inline]
    fn as_rune(self) -> Rune {
        self as Rune
    }
}

impl AsRune for u8 {
    #[inline]
    fn as_rune(self) -> Rune {
        self as Rune
    }
}

pub struct Range16 {
    pub lo: u16,
    pub hi: u16,
    pub stride: u16,
}

pub struct Range32 {
    pub lo: u32,
    pub hi: u32,
    pub stride: u32,
}

pub struct RangeTable {
    pub r16: &'static [Range16],
    pub r32: &'static [Range32],
    pub latin_offset: usize,
}

const LINEAR_MAX: usize = 18;

fn is16(ranges: &[Range16], r: u16) -> bool {
    if ranges.len() <= LINEAR_MAX || r <= 0xFF {
        for range in ranges {
            if r < range.lo {
                return false;
            }
            if r <= range.hi {
                return range.stride == 1 || (r - range.lo) % range.stride == 0;
            }
        }
        return false;
    }
    let (mut lo, mut hi) = (0usize, ranges.len());
    while lo < hi {
        let m = lo + (hi - lo) / 2;
        let range = &ranges[m];
        if range.lo <= r && r <= range.hi {
            return range.stride == 1 || (r - range.lo) % range.stride == 0;
        }
        if r < range.lo {
            hi = m;
        } else {
            lo = m + 1;
        }
    }
    false
}

fn is32(ranges: &[Range32], r: u32) -> bool {
    if ranges.len() <= LINEAR_MAX {
        for range in ranges {
            if r < range.lo {
                return false;
            }
            if r <= range.hi {
                return range.stride == 1 || (r - range.lo) % range.stride == 0;
            }
        }
        return false;
    }
    let (mut lo, mut hi) = (0usize, ranges.len());
    while lo < hi {
        let m = lo + (hi - lo) / 2;
        let range = &ranges[m];
        if range.lo <= r && r <= range.hi {
            return range.stride == 1 || (r - range.lo) % range.stride == 0;
        }
        if r < range.lo {
            hi = m;
        } else {
            lo = m + 1;
        }
    }
    false
}

/// Go `unicode.Is`.
pub fn unicode_is(table: &RangeTable, r: Rune) -> bool {
    if r < 0 {
        return false;
    }
    let r16 = table.r16;
    if !r16.is_empty() && r as u32 <= r16[r16.len() - 1].hi as u32 {
        return is16(r16, r as u16);
    }
    let r32 = table.r32;
    if !r32.is_empty() && r as u32 >= r32[0].lo {
        return is32(r32, r as u32);
    }
    false
}

/// Go `unicode.ToLower` via Rust's full lowercase mapping; U+0130, the only rune with a multi-char full lowercase, is
/// special-cased to 'i'.
pub fn unicode_to_lower(r: Rune) -> Rune {
    if r < 0x80 {
        if (b'A' as Rune..=b'Z' as Rune).contains(&r) {
            return r + 32;
        }
        return r;
    }
    let Some(c) = char::from_u32(r as u32) else {
        return r;
    };
    if c == '\u{130}' {
        return 'i' as Rune;
    }
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l as Rune,
        _ => r,
    }
}

/// Go `unicode.ToUpper` (simple uppercase mapping) via Rust's full uppercase mapping, keeping the rune when that
/// mapping is not a single char. The only runes whose full uppercase is multi-char but whose simple uppercase is
/// another rune are the 27 Greek letters with ypogegrammeni, which map to their titlecase form (U+1F80 -> U+1F88,
/// U+1FB3 -> U+1FBC); they are special-cased. Checked exhaustively against Go 1.27 (Unicode 17.0.0, the same version
/// as the pinned Rust toolchain's `char::UNICODE_VERSION`).
pub fn unicode_to_upper(r: Rune) -> Rune {
    if r < 0x80 {
        if (b'a' as Rune..=b'z' as Rune).contains(&r) {
            return r - 32;
        }
        return r;
    }
    let Some(c) = char::from_u32(r as u32) else {
        return r;
    };
    match r {
        0x1F80..=0x1F87 | 0x1F90..=0x1F97 | 0x1FA0..=0x1FA7 => return r + 8,
        0x1FB3 | 0x1FC3 | 0x1FF3 => return r + 9,
        _ => {}
    }
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u as Rune,
        _ => r,
    }
}

/// Go `unicode.SimpleFold` (Go 1.27 unicode/letter.go:354): the next rune after `r` in its simple case folding orbit,
/// wrapping around to the smallest. Go first consults its `asciiFold` table for ASCII; that table agrees with
/// `CASE_ORBIT` (K, S, k, s) and the ToLower/ToUpper fallback, so it is not copied.
pub fn unicode_simple_fold(r: Rune) -> Rune {
    if !(0..=0x10FFFF).contains(&r) {
        return r;
    }
    if let Ok(i) = CASE_ORBIT.binary_search_by_key(&r, |&(from, _)| from as Rune) {
        return CASE_ORBIT[i].1 as Rune;
    }
    // No folding specified. This is a one- or two-element equivalence class containing rune and ToLower(rune) and
    // ToUpper(rune) if they are different from rune.
    let l = unicode_to_lower(r);
    if l != r {
        return l;
    }
    unicode_to_upper(r)
}

/// Go `unicode.caseOrbit` (unicode/tables.go:9816, Unicode 17.0.0): the orbits of more than two runes, plus U+0130 and
/// U+0131, which fold only to themselves.
#[rustfmt::skip]
static CASE_ORBIT: [(u16, u16); 94] = [
    (0x004B, 0x006B), (0x0053, 0x0073), (0x006B, 0x212A), (0x0073, 0x017F), (0x00B5, 0x039C), (0x00C5, 0x00E5),
    (0x00DF, 0x1E9E), (0x00E5, 0x212B), (0x0130, 0x0130), (0x0131, 0x0131), (0x017F, 0x0053), (0x01C4, 0x01C5),
    (0x01C5, 0x01C6), (0x01C6, 0x01C4), (0x01C7, 0x01C8), (0x01C8, 0x01C9), (0x01C9, 0x01C7), (0x01CA, 0x01CB),
    (0x01CB, 0x01CC), (0x01CC, 0x01CA), (0x01F1, 0x01F2), (0x01F2, 0x01F3), (0x01F3, 0x01F1), (0x0345, 0x0399),
    (0x0390, 0x1FD3), (0x0392, 0x03B2), (0x0395, 0x03B5), (0x0398, 0x03B8), (0x0399, 0x03B9), (0x039A, 0x03BA),
    (0x039C, 0x03BC), (0x03A0, 0x03C0), (0x03A1, 0x03C1), (0x03A3, 0x03C2), (0x03A6, 0x03C6), (0x03A9, 0x03C9),
    (0x03B0, 0x1FE3), (0x03B2, 0x03D0), (0x03B5, 0x03F5), (0x03B8, 0x03D1), (0x03B9, 0x1FBE), (0x03BA, 0x03F0),
    (0x03BC, 0x00B5), (0x03C0, 0x03D6), (0x03C1, 0x03F1), (0x03C2, 0x03C3), (0x03C3, 0x03A3), (0x03C6, 0x03D5),
    (0x03C9, 0x2126), (0x03D0, 0x0392), (0x03D1, 0x03F4), (0x03D5, 0x03A6), (0x03D6, 0x03A0), (0x03F0, 0x039A),
    (0x03F1, 0x03A1), (0x03F4, 0x0398), (0x03F5, 0x0395), (0x0412, 0x0432), (0x0414, 0x0434), (0x041E, 0x043E),
    (0x0421, 0x0441), (0x0422, 0x0442), (0x042A, 0x044A), (0x0432, 0x1C80), (0x0434, 0x1C81), (0x043E, 0x1C82),
    (0x0441, 0x1C83), (0x0442, 0x1C84), (0x044A, 0x1C86), (0x0462, 0x0463), (0x0463, 0x1C87), (0x1C80, 0x0412),
    (0x1C81, 0x0414), (0x1C82, 0x041E), (0x1C83, 0x0421), (0x1C84, 0x1C85), (0x1C85, 0x0422), (0x1C86, 0x042A),
    (0x1C87, 0x0462), (0x1C88, 0xA64A), (0x1E60, 0x1E61), (0x1E61, 0x1E9B), (0x1E9B, 0x1E60), (0x1E9E, 0x00DF),
    (0x1FBE, 0x0345), (0x1FD3, 0x0390), (0x1FE3, 0x03B0), (0x2126, 0x03A9), (0x212A, 0x004B), (0x212B, 0x00C5),
    (0xA64A, 0xA64B), (0xA64B, 0x1C88), (0xFB05, 0xFB06), (0xFB06, 0xFB05),
];

/// Go `utf8.DecodeRuneInString` over raw bytes: returns (RuneError, 1) for invalid
/// encodings (including surrogate encodings) and (RuneError, 0) for empty input.
pub fn decode_rune(s: &[u8]) -> (Rune, usize) {
    const RUNE_ERROR: Rune = 0xFFFD;
    if s.is_empty() {
        return (RUNE_ERROR, 0);
    }
    let b0 = s[0];
    if b0 < 0x80 {
        return (b0 as Rune, 1);
    }
    let cont = |i: usize, lo: u8, hi: u8| -> Option<u32> {
        let b = *s.get(i)?;
        if b < lo || b > hi {
            None
        } else {
            Some((b & 0x3F) as u32)
        }
    };
    if (0xC2..=0xDF).contains(&b0) {
        if let Some(c1) = cont(1, 0x80, 0xBF) {
            return ((((b0 & 0x1F) as u32) << 6 | c1) as Rune, 2);
        }
        return (RUNE_ERROR, 1);
    }
    if (0xE0..=0xEF).contains(&b0) {
        let (lo, hi) = match b0 {
            0xE0 => (0xA0, 0xBF),
            0xED => (0x80, 0x9F),
            _ => (0x80, 0xBF),
        };
        if let (Some(c1), Some(c2)) = (cont(1, lo, hi), cont(2, 0x80, 0xBF)) {
            return ((((b0 & 0x0F) as u32) << 12 | c1 << 6 | c2) as Rune, 3);
        }
        return (RUNE_ERROR, 1);
    }
    if (0xF0..=0xF4).contains(&b0) {
        let (lo, hi) = match b0 {
            0xF0 => (0x90, 0xBF),
            0xF4 => (0x80, 0x8F),
            _ => (0x80, 0xBF),
        };
        if let (Some(c1), Some(c2), Some(c3)) = (cont(1, lo, hi), cont(2, 0x80, 0xBF), cont(3, 0x80, 0xBF)) {
            return ((((b0 & 0x07) as u32) << 18 | c1 << 12 | c2 << 6 | c3) as Rune, 4);
        }
        return (RUNE_ERROR, 1);
    }
    (RUNE_ERROR, 1)
}

/// Go `utf8.DecodeLastRuneInString` over raw bytes.
pub fn decode_last_rune(s: &[u8]) -> (Rune, usize) {
    if s.is_empty() {
        return (0xFFFD, 0);
    }
    let end = s.len();
    let lim = end.saturating_sub(4);
    let mut start = end - 1;
    if s[start] < 0x80 {
        return (s[start] as Rune, 1);
    }
    while start > lim && (s[start] & 0xC0) == 0x80 {
        start -= 1;
    }
    let (r, size) = decode_rune(&s[start..end]);
    if start + size != end {
        return (0xFFFD, 1);
    }
    (r, size)
}

/// Appends the UTF-8 encoding of `r` (Go `strings.Builder.WriteRune`; invalid runes become U+FFFD).
pub fn push_rune(b: &mut String, r: Rune) {
    b.push(char::from_u32(r as u32).unwrap_or('\u{FFFD}'));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are Go 1.27's unicode.ToUpper / unicode.ToLower (Unicode 17.0.0). An exhaustive comparison over
    // every rune found these 27 runes as the only difference between the simple mapping and Rust's full mapping.
    #[test]
    fn test_unicode_to_upper_simple_mapping() {
        for (lo, hi) in [(0x1F80, 0x1F87), (0x1F90, 0x1F97), (0x1FA0, 0x1FA7)] {
            for r in lo..=hi {
                assert_eq!(unicode_to_upper(r), r + 8, "U+{r:04X}");
            }
        }
        assert_eq!(unicode_to_upper(0x1FB3), 0x1FBC);
        assert_eq!(unicode_to_upper(0x1FC3), 0x1FCC);
        assert_eq!(unicode_to_upper(0x1FF3), 0x1FFC);
        // No simple uppercase: unchanged.
        for r in [0xDF, 0x149, 0x1F0, 0x1F50, 0x1F88, 0x1FB2, 0x1FB4, 0x1FBC, 0xFB00] {
            assert_eq!(unicode_to_upper(r), r, "U+{r:04X}");
        }
        assert_eq!(unicode_to_upper(0x1C5), 0x1C4);
        assert_eq!(unicode_to_upper('a' as Rune), 'A' as Rune);
        assert_eq!(unicode_to_upper(0xFF), 0x178);
        assert_eq!(unicode_to_upper(-1), -1);
        assert_eq!(unicode_to_upper(0xD800), 0xD800);
    }

    // The special cases in unicode_to_upper / unicode_to_lower were derived for Unicode 17.0.0. A toolchain with
    // another Unicode version needs the exhaustive comparison against Go's unicode.ToUpper / ToLower re-run.
    #[test]
    fn test_unicode_version_of_case_mappings() {
        assert_eq!(char::UNICODE_VERSION, (17, 0, 0));
    }

    // Expected values are Go 1.27's unicode.SimpleFold, which an exhaustive comparison over every rune matches.
    #[test]
    fn test_unicode_simple_fold() {
        let orbit = |start: Rune| {
            let mut out = vec![start];
            let mut r = unicode_simple_fold(start);
            while r != start {
                out.push(r);
                r = unicode_simple_fold(r);
            }
            out
        };
        assert_eq!(orbit('K' as Rune), ['K' as Rune, 'k' as Rune, 0x212A]);
        assert_eq!(orbit(0x398), [0x398, 0x3B8, 0x3D1, 0x3F4]);
        assert_eq!(orbit(0xFB05), [0xFB05, 0xFB06]);
        assert_eq!(orbit(0x390), [0x390, 0x1FD3]);
        assert_eq!(orbit('i' as Rune), ['i' as Rune, 'I' as Rune]);
        // U+0130 and U+0131 fold to themselves, though ToLower(U+0130) is 'i' and ToUpper(U+0131) is 'I'.
        assert_eq!(unicode_simple_fold(0x130), 0x130);
        assert_eq!(unicode_simple_fold(0x131), 0x131);
        // The ToLower / ToUpper fallback, with ToUpper's simple mapping.
        assert_eq!(unicode_simple_fold(0x1F80), 0x1F88);
        assert_eq!(unicode_simple_fold(0x1F88), 0x1F80);
        assert_eq!(unicode_simple_fold('1' as Rune), '1' as Rune);
        assert_eq!(unicode_simple_fold(-2), -2);
    }

    #[test]
    fn test_unicode_to_lower_simple_mapping() {
        assert_eq!(unicode_to_lower(0x130), 'i' as Rune);
        assert_eq!(unicode_to_lower(0x1F88), 0x1F80);
        assert_eq!(unicode_to_lower(0x1FBC), 0x1FB3);
        assert_eq!(unicode_to_lower(0x1C5), 0x1C6);
        assert_eq!(unicode_to_lower('A' as Rune), 'a' as Rune);
    }
}
