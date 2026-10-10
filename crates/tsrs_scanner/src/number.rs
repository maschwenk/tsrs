use tsrs_core::jsnum::Number;

/// Convert the scanner's unsigned, separator-free spelling. Recovery spellings
/// outside this grammar are left to `jsnum::from_string`.
pub(super) fn parse_literal(text: &str) -> Option<Number> {
    let bytes = text.as_bytes();
    let value = match bytes {
        [b'0', b'b' | b'B', digits @ ..] => parse_radix::<1>(digits)?,
        [b'0', b'o' | b'O', digits @ ..] => parse_radix::<3>(digits)?,
        [b'0', b'x' | b'X', digits @ ..] => parse_radix::<4>(digits)?,
        [b'0'..=b'9' | b'.', ..] => parse_decimal(text)?,
        _ => return None,
    };
    Some(Number(value))
}

fn parse_decimal(text: &str) -> Option<f64> {
    // Every decimal integer of at most 19 digits fits in a u64.
    if text.len() <= 19 {
        let mut value = 0_u64;
        for byte in text.bytes() {
            if !byte.is_ascii_digit() {
                return text.parse().ok();
            }
            value = value * 10 + u64::from(byte & 15);
        }
        return Some(value as f64);
    }
    text.parse().ok()
}

fn digit<const BITS: u32>(byte: u8) -> Option<u8> {
    let value = match byte {
        b'0'..=b'9' => byte & 15,
        b'a'..=b'f' | b'A'..=b'F' if BITS == 4 => (byte & 15) + 9,
        _ => return None,
    };
    (u32::from(value) < (1 << BITS)).then_some(value)
}

fn parse_radix<const BITS: u32>(digits: &[u8]) -> Option<f64> {
    if digits.is_empty() {
        return None;
    }
    if digits.len() > (64 / BITS) as usize {
        return parse_radix_slow::<BITS>(digits);
    }
    let mut value = 0_u64;
    for &byte in digits {
        value = (value << BITS) | u64::from(digit::<BITS>(byte)?);
    }
    Some(value as f64)
}

#[cold]
fn parse_radix_slow<const BITS: u32>(digits: &[u8]) -> Option<f64> {
    let mut significand = 0_u64;
    let mut exponent = 0_u32;
    let mut sticky = 0_u8;
    for &byte in digits {
        let value = digit::<BITS>(byte)?;
        if significand < (1 << 54) {
            significand = (significand << BITS) | u64::from(value);
        } else {
            // Keep validating after overflow, without letting an arbitrarily
            // long literal overflow the exponent counter.
            exponent = (exponent + BITS).min(1024);
            sticky |= value;
        }
    }
    if exponent > 1023 {
        return Some(f64::INFINITY);
    }
    // Oxc's conversion retains at least 55 bits: 53 significant bits, a rounding
    // bit, and a sticky bit. The cast rounds once, ties to even; scaling is exact
    // unless it overflows to infinity.
    significand |= u64::from(sticky != 0);
    Some((significand as f64) * f64::from_bits(u64::from(exponent + 1023) << 52))
}

#[cfg(test)]
mod tests {
    use super::parse_literal;
    use tsrs_core::jsnum;

    #[test]
    fn conversion_boundaries() {
        for text in [
            "0",
            "00",
            "08",
            ".0",
            ".5",
            "1.",
            "1e-9999",
            "1e9999",
            "9007199254740991",
            "9007199254740992",
            "9007199254740993",
            "9223372036854775807",
            "9223372036854775808",
            "9999999999999999999",
            "18446744073709551615",
            "18446744073709551616",
            "0xffffffffffffffff",
            "0x10000000000000000",
            "0XABCDEF",
            "0o777777777777777777777",
            "0o1000000000000000000000",
        ] {
            assert_eq!(parse_literal(text).unwrap().0.to_bits(), jsnum::from_string(text).0.to_bits(), "{text}");
        }
        // Integers immediately below, at, and above a halfway rounding point,
        // including the transition to infinity and lengths beyond each fast path.
        for power in [53, 54, 63, 64, 65, 100, 1023, 1024, 1100] {
            for tail in ["", "0", "1", "01", "10", "11"] {
                let binary = format!("1{}1{tail}", "0".repeat(power - 1));
                for (prefix, width) in [("0b", 1), ("0o", 3), ("0x", 4)] {
                    let padded = format!("{}{binary}", "0".repeat((width - binary.len() % width) % width));
                    let digits: String = padded
                        .as_bytes()
                        .chunks(width)
                        .map(|chunk| {
                            let digit = chunk.iter().fold(0, |value, byte| value * 2 + u32::from(byte - b'0'));
                            char::from_digit(digit, 16).unwrap()
                        })
                        .collect();
                    for zeros in [0, 80] {
                        let text = format!("{prefix}{}{digits}", "0".repeat(zeros));
                        assert_eq!(parse_literal(&text).unwrap().0.to_bits(), jsnum::from_string(&text).0.to_bits(), "{text}");
                    }
                }
            }
        }
    }

    #[test]
    fn recovery_spellings_fall_back() {
        for text in ["", ".", "-0", "+1", "NaN", "Infinity", " 1", "1_0", "1e", "1e+", "0x", "0b2", "0o8", "0xg", "1n", "1a", "１"] {
            assert!(parse_literal(text).is_none(), "{text}");
        }
        let invalid_after_overflow = format!("0x{}g", "f".repeat(300));
        assert!(parse_literal(&invalid_after_overflow).is_none());
    }
}
