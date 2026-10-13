use std::f64::consts::PI;

use super::jsnum_test::{assert_equal_number, number_from_bits};
use super::ryu_test::ryu_tests;
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct StringTest {
    pub(super) number: Number,
    pub(super) str: &'static str,
}

pub(super) fn st(number: Number, str: &'static str) -> StringTest {
    StringTest { number, str }
}

fn string_tests() -> Vec<StringTest> {
    let mut tests = vec![
        st(nan(), "NaN"),
        st(inf(1), "Infinity"),
        st(inf(-1), "-Infinity"),
        st(Number(0.0), "0"),
        st(negative_zero, "0"),
        st(Number(1.0), "1"),
        st(Number(-1.0), "-1"),
        st(Number(0.3), "0.3"),
        st(Number(-0.3), "-0.3"),
        st(Number(1.5), "1.5"),
        st(Number(-1.5), "-1.5"),
        st(Number(1.0e308), "1e+308"),
        st(Number(-1.0e308), "-1e+308"),
        st(Number(PI), "3.141592653589793"),
        st(Number(-PI), "-3.141592653589793"),
        st(MaxSafeInteger, "9007199254740991"),
        st(MinSafeInteger, "-9007199254740991"),
        st(number_from_bits(0x000F_FFFF_FFFF_FFFF), "2.225073858507201e-308"),
        st(number_from_bits(0x0010_0000_0000_0000), "2.2250738585072014e-308"),
        st(Number(1_234_567.8), "1234567.8"),
        st(Number(19_686_109_595_169_230_000.0), "19686109595169230000"),
        st(Number(123.456), "123.456"),
        st(Number(-123.456), "-123.456"),
        st(Number(444_123.0), "444123"),
        st(Number(-444_123.0), "-444123"),
        st(Number(444_123.789_123_456_8), "444123.7891234568"),
        st(Number(-444_123.789_636_363_6), "-444123.7896363636"),
        st(Number(1.0e21), "1e+21"),
        st(Number(1.0e20), "100000000000000000000"),
    ];
    tests.extend(ryu_tests());
    tests
}

#[test]
fn test_string() {
    for test in string_tests() {
        assert_eq!(test.number.string(), test.str, "{:?}", test.number.0);
    }
}

fn from_string_tests() -> Vec<StringTest> {
    vec![
        st(nan(), "    NaN"),
        st(inf(1), "Infinity    "),
        st(inf(-1), "    -Infinity"),
        st(Number(1.0), "1."),
        st(Number(1.0), "1.0   "),
        st(Number(1.0), "+1"),
        st(Number(1.0), "+1."),
        st(Number(1.0), "+1.0"),
        st(nan(), "whoops"),
        st(Number(0.0), ""),
        st(Number(0.0), "0"),
        st(Number(0.0), "0."),
        st(Number(0.0), "0.0"),
        st(Number(0.0), "0.0000"),
        st(Number(0.0), ".0000"),
        st(negative_zero, "-0"),
        st(negative_zero, "-0."),
        st(negative_zero, "-0.0"),
        st(negative_zero, "-.0"),
        st(nan(), "."),
        st(nan(), "e"),
        st(nan(), ".e"),
        st(nan(), "+"),
        st(nan(), "-"),
        st(nan(), "１２"),
        st(nan(), "١٢"),
        st(nan(), "0xé"),
        st(nan(), "1\u{2028}2"),
        st(nan(), "1e٢"),
        st(Number(12.5), "\u{FEFF}\u{00A0}12.5\u{2028}\u{2029}"),
        st(Number(100.0), "1E2"),
        st(Number(0.0), "0X0"),
        st(nan(), "e0"),
        st(nan(), "E0"),
        st(nan(), "1e"),
        st(nan(), "1e+"),
        st(nan(), "1e-"),
        st(Number(1.0), "1e+0"),
        st(nan(), "++0"),
        st(nan(), "0_0"),
        st(inf(1), "1e1000"),
        st(inf(-1), "-1e1000"),
        st(Number(0.0), ".0e0"),
        st(nan(), "0e++0"),
        st(Number(10.0), "0XA"),
        st(Number(0b1010u64 as f64), "0b1010"),
        st(Number(0b1010u64 as f64), "0B1010"),
        st(Number(0o12u64 as f64), "0o12"),
        st(Number(0o12u64 as f64), "0O12"),
        st(Number(0x1234_5678_9abc_def0_u64 as f64), "0x123456789abcdef0"),
        st(Number(0x1234_5678_9abc_def0_u64 as f64), "0X123456789ABCDEF0"),
        st(Number(18_446_744_073_709_552_000.0), "0X10000000000000000"),
        st(Number(18_446_744_073_709_597_000.0), "0X1000000000000A801"),
        st(nan(), "0B0.0"),
        st(Number(1.231_235_345_083_403e+91), "12312353450834030486384068034683603046834603806830644850340602384608368034634603680348603864"),
        st(nan(), "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX8OOOOOOOOOOOOOOOOOOO"),
        st(inf(1), "+Infinity"),
        st(Number(1234.56), "  \t1234.56  "),
        st(nan(), "\u{200b}"),
        st(Number(0.0), " "),
        st(Number(0.0), "\n"),
        st(Number(0.0), "\r"),
        st(Number(0.0), "\r\n"),
        st(Number(0.0), "\u{2028}"),
        st(Number(0.0), "\u{2029}"),
        st(Number(0.0), "\t"),
        st(Number(0.0), "\u{000B}"),
        st(Number(0.0), "\u{000C}"),
        st(Number(0.0), "\u{FEFF}"),
        st(Number(0.0), "\u{00A0}"),
        st(Number(10_000_000_000_000_000_000.0), "010000000000000000000"),
        st(nan(), "0x1.fffffffffffffp1023"), // Make sure Go's extended float syntax doesn't work.
        st(nan(), "0X_1FFFP-16"),
        st(nan(), "1_000"), // NumberToString doesn't handle underscores.
        st(Number(0.0), "0x0"),
        st(Number(0.0), "0X0"),
        st(nan(), "0xOOPS"),
        st(Number(0x00AB_CDEF_u64 as f64), "0xABCDEF"),
        st(Number(0x00AB_CDEF_u64 as f64), "0xABCDEF"),
        st(Number(0.0), "0o0"),
        st(Number(0.0), "0O0"),
        st(nan(), "0o8"),
        st(nan(), "0O8"),
        st(Number(0o12345u64 as f64), "0o12345"),
        st(Number(0o12345u64 as f64), "0O12345"),
        st(Number(0.0), "0b0"),
        st(Number(0.0), "0B0"),
        st(nan(), "0b2"),
        st(nan(), "0b2"),
        st(Number(0b10101u64 as f64), "0b10101"),
        st(Number(0b10101u64 as f64), "0B10101"),
        st(nan(), "1.f"),
        st(nan(), "1.e"),
        st(nan(), "1.0ef"),
        st(nan(), "1.0e"),
        st(nan(), ".f"),
        st(nan(), ".e"),
        st(nan(), ".0ef"),
        st(nan(), ".0e"),
        st(nan(), "a.f"),
        st(nan(), "a.e"),
        st(nan(), "a.0ef"),
        st(nan(), "a.0e"),
    ]
}

#[test]
fn test_from_string() {
    // stringTests
    for test in string_tests() {
        assert_equal_number(from_string(test.str), test.number);
        assert_equal_number(from_string(&(test.str.to_string() + " ")), test.number);
        assert_equal_number(from_string(&(" ".to_string() + test.str)), test.number);
    }

    // fromStringTests
    for test in from_string_tests() {
        assert_equal_number(from_string(test.str), test.number);
    }
}

#[test]
fn test_string_roundtrip() {
    for test in string_tests() {
        assert_eq!(from_string(test.str).string(), test.str);
    }
}

#[test]
fn cut_any_preserves_unicode_cutsets() {
    assert_eq!(cut_any("é1E2e3", "eE"), ("é1", "2e3", true));
    assert_eq!(cut_any("é😀1", "😀é"), ("", "😀1", true));
    assert_eq!(cut_any("a😀b", "😀"), ("a", "b", true));
    assert_eq!(cut_any("aébE2", "éE"), ("a", "bE2", true));
    assert_eq!(cut_any("é😀", "eE"), ("é😀", "", false));
    assert_eq!(cut_any("é😀", ""), ("é😀", "", false));
}

// TestStringJS, FuzzStringJS, FuzzFromStringJS and getStringResultsFromJS
// compare against Node.js and are not ported.
