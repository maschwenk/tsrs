use super::rangetable::AsRune;

// IsUnicodeIdentifierStart reports whether ch may begin an ECMAScript
// identifier, i.e. whether it has the Unicode ID_Start (or Other_ID_Start)
// property. unicode-id-start is pinned to Unicode 15.1.0 to match Go's tables.
#[inline]
pub fn is_unicode_identifier_start(ch: impl AsRune) -> bool {
    ch.as_char().is_some_and(|ch| {
        if ch.is_ascii() { ch.is_ascii_alphabetic() } else { unicode_id_start::is_id_start_unicode(ch) }
    })
}

// IsUnicodeIdentifierPart reports whether ch may appear after the first
// character of an ECMAScript identifier, i.e. whether it has the Unicode
// ID_Continue (or Other_ID_Continue) property, which also includes ID_Start.
#[inline]
pub fn is_unicode_identifier_part(ch: impl AsRune) -> bool {
    ch.as_char().is_some_and(|ch| {
        if ch.is_ascii() { ch.is_ascii_alphanumeric() || ch == '_' } else { unicode_id_start::is_id_continue_unicode(ch) }
    })
}

#[cfg(test)]
mod tests {
    use super::{is_unicode_identifier_part, is_unicode_identifier_start};

    #[test]
    fn identifier_properties_match_pinned_unicode() {
        for rune in 0..=0x110000 {
            let ch = char::from_u32(rune);
            assert_eq!(is_unicode_identifier_start(rune), ch.is_some_and(unicode_id_start::is_id_start), "start {rune:#x}");
            assert_eq!(is_unicode_identifier_part(rune), ch.is_some_and(unicode_id_start::is_id_continue), "part {rune:#x}");
        }
        for rune in [i32::MIN, -1, i32::MAX] {
            assert!(!is_unicode_identifier_start(rune));
            assert!(!is_unicode_identifier_part(rune));
        }
    }
}
