use super::identifier_parts_generated::{UNICODE_ESNEXT_IDENTIFIER_PART, UNICODE_ESNEXT_IDENTIFIER_START};
use super::rangetable::{unicode_is, AsRune};

// IsUnicodeIdentifierStart reports whether ch may begin an ECMAScript
// identifier, i.e. whether it has the Unicode ID_Start (or Other_ID_Start)
// property. The range table is generated; see generate-unicode-data.mts.
pub fn is_unicode_identifier_start(ch: impl AsRune) -> bool {
    unicode_is(&UNICODE_ESNEXT_IDENTIFIER_START, ch.as_rune())
}

// IsUnicodeIdentifierPart reports whether ch may appear after the first
// character of an ECMAScript identifier, i.e. whether it has the Unicode
// ID_Continue (or Other_ID_Continue) property, which also includes ID_Start.
pub fn is_unicode_identifier_part(ch: impl AsRune) -> bool {
    unicode_is(&UNICODE_ESNEXT_IDENTIFIER_PART, ch.as_rune())
}
