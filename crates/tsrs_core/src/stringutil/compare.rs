use super::rangetable::{decode_rune, unicode_simple_fold, unicode_to_lower, Rune};
use std::cmp::Ordering;

/// Go `strings.EqualFold` (Unicode simple case folding).
pub fn equal_fold(a: &str, b: &str) -> bool {
    equal_fold_bytes(a.as_bytes(), b.as_bytes())
}

/// Go `strings.EqualFold` (Go 1.27 strings/strings.go:1185) over bytes, which may be cut mid-rune as Go strings
/// can be; invalid UTF-8 decodes as U+FFFD, one byte at a time.
pub fn equal_fold_bytes(s: &[u8], t: &[u8]) -> bool {
    // ASCII fast path
    let mut i = 0;
    while i < s.len() && i < t.len() {
        let (mut sr, mut tr) = (s[i], t[i]);
        if (sr | tr) >= 0x80 {
            return equal_fold_unicode(&s[i..], &t[i..]);
        }
        i += 1;
        // Easy case.
        if tr == sr {
            continue;
        }
        // Make sr < tr to simplify what follows.
        if tr < sr {
            std::mem::swap(&mut tr, &mut sr);
        }
        // ASCII only, sr/tr must be upper/lower case
        if sr.is_ascii_uppercase() && tr == sr + b'a' - b'A' {
            continue;
        }
        return false;
    }
    // Check if we've exhausted both strings.
    s.len() == t.len()
}

// The `hasUnicode:` half of Go's EqualFold.
fn equal_fold_unicode(mut s: &[u8], mut t: &[u8]) -> bool {
    while !s.is_empty() {
        let (mut sr, size) = decode_rune(s);
        s = &s[size..];
        // If t is exhausted the strings are not equal.
        if t.is_empty() {
            return false;
        }
        let (mut tr, size) = decode_rune(t);
        t = &t[size..];
        // Easy case.
        if tr == sr {
            continue;
        }
        // Make sr < tr to simplify what follows.
        if tr < sr {
            std::mem::swap(&mut tr, &mut sr);
        }
        // Fast check for ASCII.
        if tr < 0x80 {
            // ASCII only, sr/tr must be upper/lower case
            if (b'A' as Rune..=b'Z' as Rune).contains(&sr) && tr == sr + ('a' as Rune - 'A' as Rune) {
                continue;
            }
            return false;
        }
        // General case. SimpleFold(x) returns the next equivalent rune > x or wraps around to smaller values.
        let mut r = unicode_simple_fold(sr);
        while r != sr && r < tr {
            r = unicode_simple_fold(r);
        }
        if r == tr {
            continue;
        }
        return false;
    }
    // First string is empty, so check if the second one is also empty.
    t.is_empty()
}

pub fn equate_string_case_insensitive(a: &str, b: &str) -> bool {
    // !!!
    // return a == b || strings.ToUpper(a) == strings.ToUpper(b)
    equal_fold(a, b)
}

pub fn equate_string_case_sensitive(a: &str, b: &str) -> bool {
    a == b
}

pub fn get_string_equality_comparer(ignore_case: bool) -> fn(&str, &str) -> bool {
    if ignore_case {
        return equate_string_case_insensitive;
    }
    equate_string_case_sensitive
}

pub type Comparison = i32;

pub const COMPARISON_LESS_THAN: Comparison = -1;
pub const COMPARISON_EQUAL: Comparison = 0;
pub const COMPARISON_GREATER_THAN: Comparison = 1;

pub fn compare_strings_case_insensitive(a: &str, b: &str) -> Comparison {
    if a == b {
        return COMPARISON_EQUAL;
    }
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    // ASCII prefix: `unicode_to_lower` of an ASCII rune is its ASCII lowercase. A non-ASCII rune can lower to ASCII
    // (U+212A KELVIN SIGN -> 'k'), so the loop below takes over at the first non-ASCII byte on either side.
    let common = a.iter().zip(b).take_while(|&(&ca, &cb)| ca < 0x80 && cb < 0x80).count();
    for i in 0..common {
        let (lca, lcb) = (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase());
        if lca != lcb {
            return if lca < lcb { COMPARISON_LESS_THAN } else { COMPARISON_GREATER_THAN };
        }
    }
    a = &a[common..];
    b = &b[common..];
    loop {
        let (ca, sa) = decode_rune(a);
        let (cb, sb) = decode_rune(b);
        if sa == 0 {
            if sb == 0 {
                return COMPARISON_EQUAL;
            }
            return COMPARISON_LESS_THAN;
        }
        if sb == 0 {
            return COMPARISON_GREATER_THAN;
        }
        let lca = unicode_to_lower(ca);
        let lcb = unicode_to_lower(cb);
        if lca != lcb {
            if lca < lcb {
                return COMPARISON_LESS_THAN;
            }
            return COMPARISON_GREATER_THAN;
        }
        a = &a[sa..];
        b = &b[sb..];
    }
}

pub fn compare_strings_case_sensitive(a: &str, b: &str) -> Comparison {
    match a.as_bytes().cmp(b.as_bytes()) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

pub fn get_string_comparer(ignore_case: bool) -> fn(&str, &str) -> Comparison {
    if ignore_case {
        return compare_strings_case_insensitive;
    }
    compare_strings_case_sensitive
}

pub fn has_prefix(s: &str, prefix: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        return s.as_bytes().starts_with(prefix.as_bytes());
    }
    if prefix.len() > s.len() {
        return false;
    }
    equal_fold_bytes(&s.as_bytes()[..prefix.len()], prefix.as_bytes())
}

pub fn has_suffix(s: &str, suffix: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        return s.as_bytes().ends_with(suffix.as_bytes());
    }
    if suffix.len() > s.len() {
        return false;
    }
    equal_fold_bytes(&s.as_bytes()[s.len() - suffix.len()..], suffix.as_bytes())
}

pub fn has_prefix_and_suffix_without_overlap(s: &str, prefix: &str, suffix: &str, case_sensitive: bool) -> bool {
    if prefix.len() + suffix.len() > s.len() {
        return false;
    }

    has_prefix(s, prefix, case_sensitive) && has_suffix(s, suffix, case_sensitive)
}

pub fn compare_strings_case_insensitive_then_sensitive(a: &str, b: &str) -> Comparison {
    let cmp = compare_strings_case_insensitive(a, b);
    if cmp != COMPARISON_EQUAL {
        return cmp;
    }
    compare_strings_case_sensitive(a, b)
}

// CompareStringsCaseInsensitiveEslintCompatible performs a case-insensitive comparison
// using toLowerCase() instead of toUpperCase() for ESLint compatibility.
pub fn compare_strings_case_insensitive_eslint_compatible(a: &str, b: &str) -> Comparison {
    if a == b {
        return COMPARISON_EQUAL;
    }
    let a = go_strings_to_lower(a);
    let b = go_strings_to_lower(b);
    compare_strings_case_sensitive(&a, &b)
}

/// Go `strings.ToLower` (per-rune simple lowercase mapping).
pub fn go_strings_to_lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let (r, size) = decode_rune(&bytes[i..]);
        super::rangetable::push_rune(&mut out, unicode_to_lower(r));
        i += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are Go 1.27's strings.EqualFold. On these pairs a lowercase/uppercase comparison disagrees with
    // Go's SimpleFold orbit walk; an exhaustive comparison over all case-related rune pairs finds no others.
    #[test]
    fn test_equal_fold_simple_fold_orbits() {
        for (a, b) in [("I", "\u{130}"), ("i", "\u{130}"), ("I", "\u{131}"), ("i", "\u{131}")] {
            assert!(!equal_fold(a, b), "{a} {b}");
            assert!(!equal_fold(b, a), "{b} {a}");
        }
        for (a, b) in [("\u{390}", "\u{1FD3}"), ("\u{3B0}", "\u{1FE3}"), ("\u{3D1}", "\u{3F4}"), ("\u{FB05}", "\u{FB06}")] {
            assert!(equal_fold(a, b), "{a} {b}");
            assert!(equal_fold(b, a), "{b} {a}");
            assert!(equal_fold(&format!("ab{a}c"), &format!("AB{b}C")), "{a} {b}");
        }
        assert!(equal_fold("k", "\u{212A}"));
        assert!(equal_fold("\u{1F80}", "\u{1F88}"));
        assert!(equal_fold("Stra\u{DF}e", "STRA\u{1E9E}E"));
        assert!(!equal_fold("abc", "ab"));
    }

    // has_prefix / has_suffix compare byte slices that can end mid-rune, as Go's do.
    #[test]
    fn test_has_suffix_ignore_case_cut_rune() {
        assert!(has_suffix("x\u{3F4}.TS", "\u{3D1}.ts", false));
        assert!(!has_suffix("x\u{131}.TS", "i.ts", false));
        assert!(has_prefix("\u{3B8}", "\u{3D1}", false));
        // The 2-byte prefix of U+1FD3 is invalid UTF-8, which never folds to U+0390.
        assert!(!has_prefix("\u{1FD3}x", "\u{390}", false));
    }
}
