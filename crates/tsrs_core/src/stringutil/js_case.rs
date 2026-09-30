use super::js_case_generated::{
    SpecialCasingCondition, SpecialCasingMapping, SPECIAL_CASING_MAPPINGS, UNICODE_CASED_RANGES, UNICODE_CASE_IGNORABLE_RANGES,
};
use super::rangetable::{push_rune, unicode_is, Rune};
use super::util::{decode_js_string_rune_bytes, encode_js_string_rune, is_surrogate};

fn special_casing_mapping(r: Rune) -> Option<&'static SpecialCasingMapping> {
    SPECIAL_CASING_MAPPINGS.binary_search_by_key(&r, |(k, _)| *k).ok().map(|i| &SPECIAL_CASING_MAPPINGS[i].1)
}

/// Appends raw bytes that are known to be (possibly WTF-8) slices of a string.
fn push_bytes(builder: &mut String, bytes: &[u8]) {
    // SAFETY: callers pass whole encoded runes taken from a string (or WTF-8 sentinels), matching Go byte strings.
    unsafe { builder.as_mut_vec().extend_from_slice(bytes) }
}

pub fn to_lower_js(str: &str) -> String {
    if let Some(ascii) = to_lower_ascii(str) {
        return ascii;
    }

    let bytes = str.as_bytes();
    let mut builder = String::with_capacity(str.len());
    // casedBefore tracks whether the most recent non-Case_Ignorable code point is
    // "cased", which is the backward half of the Final_Sigma context. We
    // accumulate it as we stream so we never have to scan (or decode) backwards.
    let mut cased_before = false;
    let mut i = 0;
    while i < bytes.len() {
        let (r, size) = decode_js_string_rune_bytes(&bytes[i..]);
        i += size;
        if is_surrogate(r) {
            // A lone surrogate has no case mapping; preserve it verbatim, matching
            // String.prototype.toLowerCase. EncodeJSStringRune restores the sentinel
            // bytes because WriteRune would re-encode the surrogate as U+FFFD.
            push_bytes(&mut builder, encode_js_string_rune(r).as_bytes());
        } else if let Some(mapping) = special_casing_mapping(r) {
            if mapping.condition == SpecialCasingCondition::FinalSigma && is_final_sigma_context(cased_before, bytes, i) {
                builder.push_str(mapping.conditional_lower);
            } else {
                builder.push_str(mapping.lower);
            }
        } else {
            push_rune(&mut builder, r);
        }
        if !is_unicode_case_ignorable(r) {
            cased_before = is_sigma_cased(r);
        }
    }
    builder
}

pub fn to_upper_js(str: &str) -> String {
    if let Some(ascii) = to_upper_ascii(str) {
        return ascii;
    }

    let bytes = str.as_bytes();
    let mut builder = String::with_capacity(str.len());
    let mut i = 0;
    while i < bytes.len() {
        let (r, size) = decode_js_string_rune_bytes(&bytes[i..]);
        if is_surrogate(r) {
            // A lone surrogate has no case mapping; preserve it verbatim, matching
            // String.prototype.toUpperCase. Copy the sentinel bytes directly because
            // WriteRune would re-encode the surrogate as U+FFFD.
            push_bytes(&mut builder, &bytes[i..i + size]);
        } else if let Some(mapping) = special_casing_mapping(r) {
            builder.push_str(mapping.upper);
        } else {
            push_rune(&mut builder, r);
        }
        i += size;
    }

    builder
}

fn to_lower_ascii(str: &str) -> Option<String> {
    let mut needs_mapping = false;
    for &ch in str.as_bytes() {
        if ch >= 0x80 {
            return None;
        }
        needs_mapping = needs_mapping || ch.is_ascii_uppercase();
    }
    if !needs_mapping {
        return Some(str.to_string());
    }
    Some(str.to_ascii_lowercase())
}

fn to_upper_ascii(str: &str) -> Option<String> {
    let mut needs_mapping = false;
    for &ch in str.as_bytes() {
        if ch >= 0x80 {
            return None;
        }
        needs_mapping = needs_mapping || ch.is_ascii_lowercase();
    }
    if !needs_mapping {
        return Some(str.to_string());
    }
    Some(str.to_ascii_uppercase())
}

// isFinalSigmaContext reports whether a sigma at the current position is in
// Final_Sigma context: it is preceded by a cased code point and not followed by
// one. casedBefore carries the backward half (tracked incrementally by the
// caller so we never scan backwards); afterOffset is the byte offset just past
// the sigma, from which we scan forward.
fn is_final_sigma_context(cased_before: bool, str: &[u8], after_offset: usize) -> bool {
    cased_before && !has_sigma_cased_after(str, after_offset)
}

fn has_sigma_cased_after(str: &[u8], start: usize) -> bool {
    let mut i = start;
    while i < str.len() {
        let (r, size) = decode_js_string_rune_bytes(&str[i..]);
        i += size;
        if is_unicode_case_ignorable(r) {
            continue;
        }
        return is_sigma_cased(r);
    }
    false
}

fn is_sigma_cased(r: Rune) -> bool {
    unicode_is(&UNICODE_CASED_RANGES, r)
}

fn is_unicode_case_ignorable(r: Rune) -> bool {
    unicode_is(&UNICODE_CASE_IGNORABLE_RANGES, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_js_casing() {
        let e = |r: i32| encode_js_string_rune(r);
        let tests: Vec<(&str, String, String)> = vec![
            ("ascii lowercase", to_lower_js("HELLO"), "hello".into()),
            ("ascii uppercase", to_upper_js("hello"), "HELLO".into()),
            ("lowercase dotted i", to_lower_js("İSPANYOL"), "i̇spanyol".into()),
            ("lowercase lone sigma", to_lower_js("Σ"), "σ".into()),
            ("lowercase final sigma", to_lower_js("ΟΣ"), "ος".into()),
            ("lowercase non-sigma greek", to_lower_js("Ω"), "ω".into()),
            ("uppercase sharp s", to_upper_js("ßfoo"), "SSFOO".into()),
            ("uppercase non-ascii simple mapping", to_upper_js("ω"), "Ω".into()),
            ("uppercase ligature", to_upper_js("ﬁoo"), "FIOO".into()),
            ("capitalize-style uppercase", to_upper_js("ß") + "foo", "SSfoo".into()),
            ("uncapitalize-style lowercase", to_lower_js("İ") + "foo", "i̇foo".into()),
            ("final sigma after lowercase letter without uppercase mapping", to_lower_js("ʕΣ"), "ʕς".into()),
            ("sigma after modifier letter", to_lower_js("ʰΣ"), "ʰσ".into()),
            ("sigma after case ignorable ypogegrammeni", to_lower_js("ͅΣ"), "ͅσ".into()),
            ("final sigma after feminine ordinal indicator", to_lower_js("ªΣ"), "ªς".into()),
            ("final sigma after masculine ordinal indicator", to_lower_js("ºΣ"), "ºς".into()),
            ("final sigma after roman numeral", to_lower_js("ⅠΣ"), "ⅰς".into()),
            ("sigma after uppercase property added after unicode 15", to_lower_js("\u{1C89}Σ"), "\u{1C89}σ".into()),
            ("sigma after uppercase property skewed", to_lower_js("\u{A7CB}Σ"), "\u{A7CB}σ".into()),
            ("sigma before immediate latin letter", to_lower_js("ΣA"), "σa".into()),
            ("sigma before immediate roman numeral letter", to_lower_js("ΣⅠ"), "σⅰ".into()),
            ("sigma before case ignorable then latin letter", to_lower_js("ΣͅA"), "σͅa".into()),
            ("uppercase lone surrogate", to_upper_js(&e(0xD800)), e(0xD800)),
            ("lowercase lone surrogate", to_lower_js(&("A".to_string() + &e(0xD800) + "B")), "a".to_string() + &e(0xD800) + "b"),
            ("uppercase lone low surrogate with text", to_upper_js(&(e(0xDC00) + "x")), e(0xDC00) + "X"),
            ("lowercase lone surrogate before sigma", to_lower_js(&(e(0xD800) + "Σ")), e(0xD800) + "σ"),
        ];
        for (name, got, want) in tests {
            assert!(got.as_bytes() == want.as_bytes(), "{name}: got {:?}, want {:?}", got.as_bytes(), want.as_bytes());
        }
    }
}
