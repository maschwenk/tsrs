use super::rangetable::{decode_rune, unicode_to_lower, unicode_to_upper, Rune};
use std::cmp::Ordering;

/// Go `strings.EqualFold` (Unicode simple case folding).
pub fn equal_fold(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        if a.is_empty() || b.is_empty() {
            return a.is_empty() && b.is_empty();
        }
        let (ra, sa) = decode_rune(a);
        let (rb, sb) = decode_rune(b);
        a = &a[sa..];
        b = &b[sb..];
        if ra == rb {
            continue;
        }
        if !fold_equal_rune(ra, rb) {
            return false;
        }
    }
}

fn fold_equal_rune(ra: Rune, rb: Rune) -> bool {
    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
    if hi < 0x80 {
        return (b'A' as Rune..=b'Z' as Rune).contains(&lo) && hi == lo + ('a' as Rune - 'A' as Rune);
    }
    unicode_to_lower(ra) == unicode_to_lower(rb) || unicode_to_upper(ra) == unicode_to_upper(rb)
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

fn equal_fold_bytes(a: &[u8], b: &[u8]) -> bool {
    let (mut a, mut b) = (a, b);
    loop {
        if a.is_empty() || b.is_empty() {
            return a.is_empty() && b.is_empty();
        }
        let (ra, sa) = decode_rune(a);
        let (rb, sb) = decode_rune(b);
        a = &a[sa..];
        b = &b[sb..];
        if ra != rb && !fold_equal_rune(ra, rb) {
            return false;
        }
    }
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
