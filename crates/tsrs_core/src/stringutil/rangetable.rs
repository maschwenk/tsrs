// Equivalent of Go's unicode.RangeTable / unicode.Is, used by the generated tables.

pub type Rune = i32;

/// Anything usable where Go takes a `rune`: `char`, `i32` or `u32`.
pub trait AsRune: Copy {
    fn as_rune(self) -> Rune;
}

impl AsRune for char {
    #[inline]
    fn as_rune(self) -> Rune {
        self as Rune
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

/// Approximates Go `unicode.ToUpper` with Rust's full uppercase mapping, keeping the rune when that mapping is not a
/// single char. Differs from Go's simple mapping for runes whose full uppercase is multi-char but that have a simple
/// mapping (e.g. U+1F80: Go gives U+1F88, this returns it unchanged). The Unicode version is the Rust toolchain's.
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
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u as Rune,
        _ => r,
    }
}

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
