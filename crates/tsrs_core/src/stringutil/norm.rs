// Go's Unicode categories stay on the pinned tables; ICU supplies canonical normalization.

use icu_normalizer::DecomposingNormalizerBorrowed;

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
    // The NFD-specific constructor keeps compatibility-decomposition tables out of the binary.
    DecomposingNormalizerBorrowed::new_nfd().normalize(s).into_owned()
}

#[cfg(test)]
mod tests {
    use unicode_normalization::UnicodeNormalization;

    use super::*;

    #[test]
    fn normalization_uses_the_pinned_unicode_version() {
        assert_eq!(unicode_normalization::UNICODE_VERSION, (17, 0, 0));
    }

    #[test]
    fn normalization_matches_pinned_tables_for_every_scalar() {
        let mut buf = [0; 4];
        for c in (0..=0x0010_ffff).filter_map(char::from_u32) {
            let s = c.encode_utf8(&mut buf);
            assert_eq!(norm_nfd_string(s), s.nfd().collect::<String>(), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn normalization_matches_pinned_combining_sequences() {
        let marks: Vec<char> = (0..=0x0010_ffff).filter_map(char::from_u32).filter(|&c| unicode_is_mn(c as u32)).collect();
        let mut seed = 0x1234_5678_u64;
        for case in 0..20_000 {
            let mut text = if case % 3 == 0 { String::new() } else { "Åé각".to_owned() };
            for _ in 0..case % 130 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                text.push(marks[seed as usize % marks.len()]);
            }
            assert_eq!(norm_nfd_string(&text), text.nfd().collect::<String>(), "{text:?}");
        }
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
        assert_eq!(norm_nfd_string("ﬁ①"), "ﬁ①");
    }

    #[test]
    fn normalization_preserves_long_nonstarter_sequences() {
        let text = format!("a{}", "\u{301}".repeat(31));
        assert_eq!(norm_nfd_string(&text), text);
    }
}
