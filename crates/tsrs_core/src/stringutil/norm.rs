// Go standard-library / golang.org/x/text equivalents used by the language service (ls/lsutil/organizeimports.go):
// `unicode.Is(unicode.Mn, r)`, `unicode.IsUpper(r)` and `norm.NFD.String(s)`. Tables are generated from the Go
// sources by tools/oracle/unicodenorm (norm_generated.rs).

use super::norm_generated::{CANONICAL_COMBINING_CLASS, NFD_DECOMPOSITIONS, UNICODE_MN, UNICODE_UPPER};
use super::rangetable::{unicode_is, Rune};

/// Go `unicode.Is(unicode.Mn, r)`.
pub fn unicode_is_mn(r: Rune) -> bool {
    unicode_is(&UNICODE_MN, r)
}

/// Go `unicode.IsUpper(r)`.
pub fn unicode_is_upper(r: Rune) -> bool {
    unicode_is(&UNICODE_UPPER, r)
}

fn canonical_combining_class(c: char) -> u8 {
    match CANONICAL_COMBINING_CLASS.binary_search_by_key(&(c as u32), |&(r, _)| r) {
        Ok(i) => CANONICAL_COMBINING_CLASS[i].1,
        Err(_) => 0,
    }
}

const HANGUL_S_BASE: u32 = 0xAC00;
const HANGUL_L_BASE: u32 = 0x1100;
const HANGUL_V_BASE: u32 = 0x1161;
const HANGUL_T_BASE: u32 = 0x11A7;
const HANGUL_T_COUNT: u32 = 28;
const HANGUL_N_COUNT: u32 = 21 * HANGUL_T_COUNT;
const HANGUL_S_COUNT: u32 = 19 * HANGUL_N_COUNT;

/// Go `norm.NFD.String(s)`: canonical decomposition followed by canonical ordering of combining marks.
/// x/text additionally inserts U+034F COMBINING GRAPHEME JOINER after 30 consecutive non-starters (the
/// Stream-Safe Text Format); that is not reproduced.
pub fn norm_nfd_string(s: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        if (HANGUL_S_BASE..HANGUL_S_BASE + HANGUL_S_COUNT).contains(&cp) {
            let s_index = cp - HANGUL_S_BASE;
            out.push(char::from_u32(HANGUL_L_BASE + s_index / HANGUL_N_COUNT).unwrap());
            out.push(char::from_u32(HANGUL_V_BASE + (s_index % HANGUL_N_COUNT) / HANGUL_T_COUNT).unwrap());
            let t = s_index % HANGUL_T_COUNT;
            if t != 0 {
                out.push(char::from_u32(HANGUL_T_BASE + t).unwrap());
            }
            continue;
        }
        match NFD_DECOMPOSITIONS.binary_search_by_key(&cp, |&(r, _)| r) {
            Ok(i) => out.extend(NFD_DECOMPOSITIONS[i].1.chars()),
            Err(_) => out.push(c),
        }
    }
    // Canonical ordering: stable sort of every maximal run of non-starters by combining class.
    let mut i = 0;
    while i < out.len() {
        if canonical_combining_class(out[i]) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < out.len() && canonical_combining_class(out[i]) != 0 {
            i += 1;
        }
        out[start..i].sort_by_key(|&c| canonical_combining_class(c));
    }
    out.into_iter().collect()
}
