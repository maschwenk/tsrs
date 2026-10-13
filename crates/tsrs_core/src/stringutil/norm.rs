// Go's Unicode categories stay on the pinned tables; NFD uses matching Rust tables.

use unicode_normalization::UnicodeNormalization;

use super::norm_generated::{UNICODE_MN, UNICODE_UPPER};
use super::rangetable::{Rune, unicode_is};

/// Go `unicode.Is(unicode.Mn, r)`.
pub fn unicode_is_mn(r: Rune) -> bool {
    unicode_is(&UNICODE_MN, r)
}

/// Go `unicode.IsUpper(r)`.
pub fn unicode_is_upper(r: Rune) -> bool {
    unicode_is(&UNICODE_UPPER, r)
}

/// Canonical decomposition with canonical ordering of combining marks.
/// Keep the existing omission of x/text's Stream-Safe Text Format joiners.
pub fn norm_nfd_string(s: &str) -> String {
    s.nfd().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_uses_the_pinned_unicode_version() {
        assert_eq!(unicode_normalization::UNICODE_VERSION, (17, 0, 0));
    }

    #[test]
    fn normalization_preserves_canonical_order_and_hangul() {
        assert_eq!(
            norm_nfd_string("Åé각"),
            "A\u{30a}e\u{301}\u{1100}\u{1161}\u{11a8}"
        );
        assert_eq!(norm_nfd_string("a\u{301}\u{327}"), "a\u{327}\u{301}");
        assert_eq!(norm_nfd_string("\u{301}\u{327}"), "\u{327}\u{301}");
        assert_eq!(norm_nfd_string("a\u{301}\u{300}"), "a\u{301}\u{300}");
    }

    #[test]
    fn normalization_preserves_long_nonstarter_sequences() {
        let text = format!("a{}", "\u{301}".repeat(31));
        assert_eq!(norm_nfd_string(&text), text);
    }
}
