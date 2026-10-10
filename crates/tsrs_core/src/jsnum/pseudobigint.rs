use std::cmp::Ordering;
use std::fmt;

use super::big;
use crate::owned_text::TextView;

// PseudoBigInt represents a JS-like bigint. The zero state of the struct represents the value 0.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PseudoBigInt {
    pub negative: bool,              // true if the value is a non-zero negative number.
    pub base10_value: TextView, // The absolute value in base 10 with no leading zeros. The value zero is represented as an empty string.
}

pub fn new_pseudo_big_int(value: &str, negative: bool) -> PseudoBigInt {
    let value = value.trim_start_matches('0');
    PseudoBigInt { negative: negative && !value.is_empty(), base10_value: value.into() }
}

impl PseudoBigInt {
    pub fn string(&self) -> String {
        if self.base10_value.is_empty() {
            return "0".to_string();
        }
        if self.negative {
            return "-".to_string() + &self.base10_value;
        }
        self.base10_value.to_string()
    }

    pub fn sign(&self) -> i32 {
        if self.base10_value.is_empty() {
            return 0;
        }
        if self.negative {
            return -1;
        }
        1
    }

    pub fn compare(&self, other: &PseudoBigInt) -> i32 {
        let c = self.sign().cmp(&other.sign()) as i32;
        if c != 0 {
            return c;
        }
        let mut c = self.base10_value.len().cmp(&other.base10_value.len());
        if c == Ordering::Equal {
            c = self.base10_value.cmp(&other.base10_value);
        }
        let mut c = c as i32;
        if self.negative {
            c = -c;
        }
        c
    }
}

impl fmt::Display for PseudoBigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

pub fn parse_valid_big_int(text: &str) -> PseudoBigInt {
    let (text, negative) = match text.strip_prefix('-') {
        Some(rest) => (rest, true),
        None => (text, false),
    };
    new_pseudo_big_int(&parse_pseudo_big_int(text), negative)
}

pub fn parse_pseudo_big_int(string_value: &str) -> String {
    let mut string_value = string_value.strip_suffix('n').unwrap_or(string_value);
    let mut b1: u8 = 0;
    if string_value.len() > 1 {
        b1 = string_value.as_bytes()[1];
    }
    match b1 {
        b'b' | b'B' | b'o' | b'O' | b'x' | b'X' => {
            // Not decimal.
        }
        _ => {
            string_value = string_value.trim_start_matches('0');
            if string_value.is_empty() {
                return "0".to_string();
            }
            return string_value.to_string();
        }
    }
    let Some(bi) = big::Int::set_string(string_value, 0) else {
        panic!("Failed to parse big int: {string_value:?}");
    };
    bi.string() // !!!
}
