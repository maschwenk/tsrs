use std::f64::consts::{E, PI};

use super::*;

pub(super) fn assert_equal_number(got: Number, want: Number) {
    if got.is_nan() || want.is_nan() {
        assert_eq!(got.is_nan(), want.is_nan(), "got: {got}, want: {want}");
    } else {
        assert_eq!(got, want);
    }
}

pub(super) fn number_from_bits(b: u64) -> Number {
    Number(f64::from_bits(b))
}

// evalBinaryOp, evalUnaryOp and the "Node" subtests compare against Node.js
// and are not ported. The benchmarks are not ported either.

fn to_int32_tests() -> Vec<(&'static str, Number, i32)> {
    vec![
        ("0.0", Number(0.0), 0),
        ("-0.0", negative_zero, 0),
        ("NaN", nan(), 0),
        ("+Inf", inf(1), 0),
        ("-Inf", inf(-1), 0),
        ("MaxInt32", Number(i32::MAX as f64), i32::MAX),
        ("MaxInt32+1", Number((i32::MAX as i64 + 1) as f64), i32::MIN),
        ("MinInt32", Number(i32::MIN as f64), i32::MIN),
        ("MinInt32-1", Number((i32::MIN as i64 - 1) as f64), i32::MAX),
        ("MIN_SAFE_INTEGER", MinSafeInteger, 1),
        ("MIN_SAFE_INTEGER-1", MinSafeInteger - Number(1.0), 0),
        ("MIN_SAFE_INTEGER+1", MinSafeInteger + Number(1.0), 2),
        ("MAX_SAFE_INTEGER", MaxSafeInteger, -1),
        ("MAX_SAFE_INTEGER-1", MaxSafeInteger - Number(1.0), -2),
        ("MAX_SAFE_INTEGER+1", MaxSafeInteger + Number(1.0), 0),
        ("-8589934590", Number(-8_589_934_590.0), 2),
        ("0xDEADBEEF", Number(0xDEAD_BEEF_u64 as f64), -559_038_737),
        ("4294967808", Number(4_294_967_808.0), 512),
        ("-0.4", Number(-0.4), 0),
        ("SmallestNonzeroFloat64", Number(f64::from_bits(1)), 0),
        ("-SmallestNonzeroFloat64", Number(-f64::from_bits(1)), 0),
        ("MaxFloat64", Number(f64::MAX), 0),
        ("-MaxFloat64", Number(-f64::MAX), 0),
        ("Largest subnormal number", number_from_bits(0x000F_FFFF_FFFF_FFFF), 0),
        ("Smallest positive normal number", number_from_bits(0x0010_0000_0000_0000), 0),
        ("Largest normal number", Number(f64::MAX), 0),
        ("-Largest normal number", Number(-f64::MAX), 0),
        ("1.0", Number(1.0), 1),
        ("-1.0", Number(-1.0), -1),
        ("1e308", Number(1.0e308), 0),
        ("-1e308", Number(-1.0e308), 0),
        ("math.Pi", Number(PI), 3),
        ("-math.Pi", Number(-PI), -3),
        ("math.E", Number(E), 2),
        ("-math.E", Number(-E), -2),
        ("0.5", Number(0.5), 0),
        ("-0.5", Number(-0.5), 0),
        ("0.49999999999999994", Number(0.499_999_999_999_999_94), 0),
        ("-0.49999999999999994", Number(-0.499_999_999_999_999_94), 0),
        ("0.5000000000000001", Number(0.500_000_000_000_000_1), 0),
        ("-0.5000000000000001", Number(-0.500_000_000_000_000_1), 0),
        ("2^31 + 0.5", Number(2_147_483_648.5), -2_147_483_648),
        ("-2^31 - 0.5", Number(-2_147_483_648.5), -2_147_483_648),
        ("2^40", Number(1_099_511_627_776.0), 0),
        ("-2^40", Number(-1_099_511_627_776.0), 0),
        ("TypeFlagsNarrowable", Number(536_624_127.0), 536_624_127),
    ]
}

#[test]
fn test_to_int32() {
    for (name, input, want) in to_int32_tests() {
        let got = input.to_int32();
        assert_eq!(got, want, "{name} ({})", input.0);
    }
}

#[test]
fn test_bitwise_not() {
    let tests: Vec<(Number, Number)> = vec![
        // Original pairs: ~(-2147483649) == ~(2147483647)
        (Number(-2_147_483_649.0), Number(-2_147_483_648.0)),
        (Number(2_147_483_647.0), Number(-2_147_483_648.0)),
        // Original pairs: ~(-4294967296) == ~(0)
        (Number(-4_294_967_296.0), Number(-1.0)),
        (Number(0.0), Number(-1.0)),
        // Original pairs: ~(2147483648) == ~(-2147483648)
        (Number(2_147_483_648.0), Number(2_147_483_647.0)),
        (Number(-2_147_483_648.0), Number(2_147_483_647.0)),
        // Original pairs: ~(4294967296) == ~(0)
        (Number(4_294_967_296.0), Number(-1.0)),
    ];

    for (x, want) in tests {
        let got = x.bitwise_not();
        assert_equal_number(got, want);
    }
}

#[test]
fn test_bitwise_and() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(0.0), Number(0.0), Number(0.0)),
        (Number(0.0), Number(1.0), Number(0.0)),
        (Number(1.0), Number(0.0), Number(0.0)),
        (Number(1.0), Number(1.0), Number(1.0)),
    ];

    for (x, y, want) in tests {
        let got = x.bitwise_and(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_bitwise_or() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(0.0), Number(0.0), Number(0.0)),
        (Number(0.0), Number(1.0), Number(1.0)),
        (Number(1.0), Number(0.0), Number(1.0)),
        (Number(1.0), Number(1.0), Number(1.0)),
    ];

    for (x, y, want) in tests {
        let got = x.bitwise_or(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_bitwise_xor() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(0.0), Number(0.0), Number(0.0)),
        (Number(0.0), Number(1.0), Number(1.0)),
        (Number(1.0), Number(0.0), Number(1.0)),
        (Number(1.0), Number(1.0), Number(0.0)),
    ];

    for (x, y, want) in tests {
        let got = x.bitwise_xor(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_signed_right_shift() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(1.0), Number(0.0), Number(1.0)),
        (Number(1.0), Number(1.0), Number(0.0)),
        (Number(1.0), Number(2.0), Number(0.0)),
        (Number(1.0), Number(31.0), Number(0.0)),
        (Number(1.0), Number(32.0), Number(1.0)),
        (Number(-4.0), Number(0.0), Number(-4.0)),
        (Number(-4.0), Number(1.0), Number(-2.0)),
        (Number(-4.0), Number(2.0), Number(-1.0)),
        (Number(-4.0), Number(3.0), Number(-1.0)),
        (Number(-4.0), Number(4.0), Number(-1.0)),
        (Number(-4.0), Number(31.0), Number(-1.0)),
        (Number(-4.0), Number(32.0), Number(-4.0)),
        (Number(-4.0), Number(33.0), Number(-2.0)),
    ];

    for (x, y, want) in tests {
        let got = x.signed_right_shift(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_unsigned_right_shift() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(1.0), Number(0.0), Number(1.0)),
        (Number(1.0), Number(1.0), Number(0.0)),
        (Number(1.0), Number(2.0), Number(0.0)),
        (Number(1.0), Number(31.0), Number(0.0)),
        (Number(1.0), Number(32.0), Number(1.0)),
        (Number(-4.0), Number(0.0), Number(4_294_967_292.0)),
        (Number(-4.0), Number(1.0), Number(2_147_483_646.0)),
        (Number(-4.0), Number(2.0), Number(1_073_741_823.0)),
        (Number(-4.0), Number(3.0), Number(536_870_911.0)),
        (Number(-4.0), Number(4.0), Number(268_435_455.0)),
        (Number(-4.0), Number(31.0), Number(1.0)),
        (Number(-4.0), Number(32.0), Number(4_294_967_292.0)),
        (Number(-4.0), Number(33.0), Number(2_147_483_646.0)),
    ];

    for (x, y, want) in tests {
        let got = x.unsigned_right_shift(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_left_shift() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(1.0), Number(0.0), Number(1.0)),
        (Number(1.0), Number(1.0), Number(2.0)),
        (Number(1.0), Number(2.0), Number(4.0)),
        (Number(1.0), Number(31.0), Number(-2_147_483_648.0)),
        (Number(1.0), Number(32.0), Number(1.0)),
        (Number(-4.0), Number(0.0), Number(-4.0)),
        (Number(-4.0), Number(1.0), Number(-8.0)),
        (Number(-4.0), Number(2.0), Number(-16.0)),
        (Number(-4.0), Number(3.0), Number(-32.0)),
        (Number(-4.0), Number(31.0), Number(0.0)),
        (Number(-4.0), Number(32.0), Number(-4.0)),
    ];

    for (x, y, want) in tests {
        let got = x.left_shift(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_remainder() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (nan(), Number(1.0), nan()),
        (Number(1.0), nan(), nan()),
        (inf(1), Number(1.0), nan()),
        (inf(-1), Number(1.0), nan()),
        (Number(123.0), inf(1), Number(123.0)),
        (Number(123.0), inf(-1), Number(123.0)),
        (Number(123.0), Number(0.0), nan()),
        (Number(123.0), negative_zero, nan()),
        (Number(0.0), Number(123.0), Number(0.0)),
        (negative_zero, Number(123.0), negative_zero),
        // Normal cases
        (Number(10.0), Number(3.0), Number(1.0)),
        (Number(-10.0), Number(3.0), Number(-1.0)),
        (Number(10.0), Number(-3.0), Number(1.0)),
        (Number(-10.0), Number(-3.0), Number(-1.0)),
        (Number(5.5), Number(2.0), Number(1.5)),
        (Number(-5.5), Number(2.0), Number(-1.5)),
        (Number(1.0), Number(0.5), Number(0.0)),
        (Number(-1.0), Number(0.5), negative_zero),
        (Number(1.5), Number(1.0), Number(0.5)),
        (Number(-1.5), Number(1.0), Number(-0.5)),
        // Edge cases that prove the bug in the manual formula:
        // The manual formula n - d*(n/d).trunc() accumulates floating-point
        // rounding errors that IEEE 754 fmod (math.Mod) avoids.
        (Number(7.0), Number(0.1), Number(7.0 % 0.1)),
        (Number(7.0), Number(0.2), Number(7.0 % 0.2)),
        (Number(7.0), Number(0.3), Number(7.0 % 0.3)),
        (Number(100.0), Number(0.3), Number(100.0 % 0.3)),
    ];

    for (x, y, want) in tests {
        let got = x.remainder(y);
        assert_equal_number(got, want);
    }
}

#[test]
fn test_exponentiate() {
    let tests: Vec<(Number, Number, Number)> = vec![
        (Number(2.0), Number(3.0), Number(8.0)),
        (inf(1), Number(3.0), inf(1)),
        (inf(1), Number(-5.0), Number(0.0)),
        (inf(-1), Number(3.0), inf(-1)),
        (inf(-1), Number(4.0), inf(1)),
        (inf(-1), Number(-3.0), negative_zero),
        (inf(-1), Number(-4.0), Number(0.0)),
        (Number(0.0), Number(3.0), Number(0.0)),
        (Number(0.0), Number(-10.0), inf(1)),
        (negative_zero, Number(3.0), negative_zero),
        (negative_zero, Number(4.0), Number(0.0)),
        (negative_zero, Number(-3.0), inf(-1)),
        (negative_zero, Number(-4.0), inf(1)),
        (Number(3.0), inf(1), inf(1)),
        (Number(-3.0), inf(1), inf(1)),
        (Number(3.0), inf(-1), Number(0.0)),
        (Number(-3.0), inf(-1), Number(0.0)),
        (nan(), Number(3.0), nan()),
        (Number(1.0), inf(1), nan()),
        (Number(1.0), inf(-1), nan()),
        (Number(-1.0), inf(1), nan()),
        (Number(-1.0), inf(-1), nan()),
        (Number(1.0), nan(), nan()),
        // Cases where math.Pow diverges from V8 by >1 ULP.
        // Expected values are the correctly-rounded IEEE 754 results
        // computed via exact integer arithmetic (big.Int).
        // Cross-engine testing (V8, SpiderMonkey, QuickJS, XS via jsvu)
        // confirmed these match the majority of JS engines.
        (Number(10.0), Number(308.0), number_from_bits(0x7fe1_ccf3_85eb_c8a0)),
        (Number(5.0), Number(210.0), number_from_bits(0x5e68_557f_3132_6bbb)),
        (Number(10.0), Number(200.0), number_from_bits(0x6974_e718_d7d7_625a)),
    ];

    for (x, y, want) in tests {
        let got = x.exponentiate(y);
        assert_equal_number(got, want);
    }
}
