use super::rangetable::AsRune;

// IsUnicodeIdentifierStart reports whether ch may begin an ECMAScript
// identifier, i.e. whether it has the Unicode ID_Start (or Other_ID_Start)
// property. unicode-id-start is pinned to Unicode 15.1.0 to match Go's tables.
pub fn is_unicode_identifier_start(ch: impl AsRune) -> bool {
    char::from_u32(ch.as_rune() as u32).is_some_and(unicode_id_start::is_id_start)
}

// IsUnicodeIdentifierPart reports whether ch may appear after the first
// character of an ECMAScript identifier, i.e. whether it has the Unicode
// ID_Continue (or Other_ID_Continue) property, which also includes ID_Start.
pub fn is_unicode_identifier_part(ch: impl AsRune) -> bool {
    char::from_u32(ch.as_rune() as u32).is_some_and(unicode_id_start::is_id_continue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_15_1_identifiers() {
        // ID_Start includes characters excluded by XID_Start, and CJK Extension I from Unicode 15.1.
        for ch in ['\u{37A}', '\u{309B}', '\u{2EBF0}', '\u{2EE5D}'] {
            assert!(is_unicode_identifier_start(ch), "U+{:04X}", ch as u32);
            assert!(is_unicode_identifier_part(ch), "U+{:04X}", ch as u32);
        }
        // Join controls and the middle dots added to ID_Continue in Unicode 15.1 cannot start identifiers.
        for ch in ['\u{200C}', '\u{200D}', '\u{30FB}', '\u{FF65}'] {
            assert!(!is_unicode_identifier_start(ch), "U+{:04X}", ch as u32);
            assert!(is_unicode_identifier_part(ch), "U+{:04X}", ch as u32);
        }
        // Unicode 16.0 and 17.0 additions must stay rejected until the TypeScript pin changes.
        for ch in ['\u{1C89}', '\u{88F}'] {
            assert!(!is_unicode_identifier_start(ch), "U+{:04X}", ch as u32);
            assert!(!is_unicode_identifier_part(ch), "U+{:04X}", ch as u32);
        }
    }

    #[test]
    fn invalid_runes_are_not_identifiers() {
        for ch in [i32::MIN, -1, 0xD800, 0xDFFF, 0x110000, i32::MAX] {
            assert!(!is_unicode_identifier_start(ch), "{ch}");
            assert!(!is_unicode_identifier_part(ch), "{ch}");
        }
        assert!(!is_unicode_identifier_start(u32::MAX));
        assert!(!is_unicode_identifier_part(u32::MAX));
    }
}
