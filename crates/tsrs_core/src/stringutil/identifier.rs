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
