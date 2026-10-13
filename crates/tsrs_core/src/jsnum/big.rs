// The subset of Go's math/big used by this package, keeping its parsing and
// intermediate rounding semantics around Rust's arbitrary-precision integers.

use num_bigint::{BigInt, Sign};
use num_traits::{Pow, ToPrimitive};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Int(BigInt);

impl Int {
    pub(crate) fn new(x: i64) -> Int {
        Int(BigInt::from(x))
    }

    // Go's (*big.Int).SetString: base 0 detects prefixes and allows underscores
    // between digits or immediately after a prefix; explicit bases do neither.
    pub(crate) fn set_string(s: &str, base: u32) -> Option<Int> {
        let (negative, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (radix, digits, prefixed) = if base == 0 && body.len() > 1 && body.starts_with('0') {
            match body.as_bytes()[1] {
                b'b' | b'B' => (2, &body[2..], true),
                b'o' | b'O' => (8, &body[2..], true),
                b'x' | b'X' => (16, &body[2..], true),
                _ => (8, &body[1..], true),
            }
        } else {
            (if base == 0 { 10 } else { base }, body, false)
        };
        let mut previous_digit = prefixed;
        for byte in digits.bytes() {
            if byte == b'_' && base == 0 {
                if !previous_digit {
                    return None;
                }
                previous_digit = false;
            } else {
                let digit = match byte {
                    b'0'..=b'9' => u32::from(byte - b'0'),
                    b'a'..=b'z' => u32::from(byte - b'a') + 10,
                    b'A'..=b'Z' => u32::from(byte - b'A') + 10,
                    _ => return None,
                };
                if digit >= radix {
                    return None;
                }
                previous_digit = true;
            }
        }
        if !previous_digit {
            return None;
        }
        // num-bigint rejects a leading underscore even when it follows a Go prefix.
        let digits = digits.strip_prefix('_').unwrap_or(digits);
        let value = BigInt::parse_bytes(digits.as_bytes(), radix)?;
        Some(Int(if negative { -value } else { value }))
    }

    // Go's (*big.Int).Exp(x, y, nil) returns one for a non-positive exponent.
    pub(crate) fn exp(&self, y: &Int) -> Int {
        if y.0.sign() != Sign::Plus {
            return Int::new(1);
        }
        Int(Pow::pow(&self.0, y.0.magnitude()))
    }

    // Round the magnitude before restoring its sign, as big.Float.SetInt does.
    pub(crate) fn round_to_prec(&self, prec: usize) -> Int {
        assert!(prec > 0);
        let magnitude = self.0.magnitude();
        let bits = usize::try_from(magnitude.bits()).unwrap();
        if bits <= prec {
            return self.clone();
        }
        let shift = bits - prec;
        let mut rounded = magnitude >> shift;
        let half = magnitude.bit((shift - 1) as u64);
        let sticky = magnitude
            .trailing_zeros()
            .is_some_and(|zeros| zeros < (shift - 1) as u64);
        if half && (sticky || rounded.bit(0)) {
            rounded += 1u8;
        }
        Int(BigInt::from_biguint(self.0.sign(), rounded << shift))
    }

    pub(crate) fn float64(&self) -> f64 {
        self.0.to_f64().unwrap()
    }

    pub(crate) fn string(&self) -> String {
        self.0.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_preserves_go_prefixes_and_separator_rules() {
        for (text, base, expected) in [
            ("-0", 0, "0"),
            ("+0x_Ff", 0, "255"),
            ("0_17", 0, "15"),
            ("-0b_101", 0, "-5"),
            ("0o_77", 0, "63"),
            ("010", 10, "10"),
            ("FF", 16, "255"),
        ] {
            assert_eq!(Int::set_string(text, base).unwrap().string(), expected);
        }
        for (text, base) in [
            ("", 0),
            ("+", 0),
            ("0x", 0),
            ("08", 0),
            ("_1", 0),
            ("1_", 0),
            ("1__2", 0),
            ("0x__1", 0),
            ("1_2", 10),
            ("0x1", 16),
            ("--1", 0),
            ("１２", 0),
        ] {
            assert!(
                Int::set_string(text, base).is_none(),
                "{text:?}, base {base}"
            );
        }
    }

    #[test]
    fn powers_preserve_non_positive_exponents_and_signs() {
        assert_eq!(Int::new(-3).exp(&Int::new(3)).string(), "-27");
        assert_eq!(Int::new(-3).exp(&Int::new(4)).string(), "81");
        assert_eq!(Int::new(0).exp(&Int::new(0)).string(), "1");
        assert_eq!(Int::new(3).exp(&Int::new(-1)).string(), "1");
    }

    #[test]
    fn intermediate_rounding_preserves_ties_even_and_signs() {
        for (value, expected) in [(9, 8), (11, 12), (13, 12), (15, 16), (-9, -8), (-11, -12)] {
            assert_eq!(Int::new(value).round_to_prec(3), Int::new(expected));
        }
        assert_eq!(Int::new(35).round_to_prec(3), Int::new(32));
        assert_eq!(Int::new(37).round_to_prec(3), Int::new(40));
    }

    #[test]
    fn float_conversion_preserves_rounding_and_overflow() {
        for (text, expected) in [
            ("9007199254740993", 9_007_199_254_740_992.0f64),
            ("9007199254740995", 9_007_199_254_740_996.0),
            ("-9007199254740993", -9_007_199_254_740_992.0),
        ] {
            assert_eq!(
                Int::set_string(text, 10).unwrap().float64().to_bits(),
                expected.to_bits()
            );
        }
        assert_eq!(Int::new(2).exp(&Int::new(1024)).float64(), f64::INFINITY);
        assert_eq!(
            Int::new(-2).exp(&Int::new(1025)).float64(),
            f64::NEG_INFINITY
        );
        assert_eq!(Int::new(0).float64().to_bits(), 0);
    }
}
