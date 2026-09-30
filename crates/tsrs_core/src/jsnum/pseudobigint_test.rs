use super::*;

#[test]
fn test_parse_pseudo_big_int() {
    let mut test_numbers: Vec<Number> = Vec::new();
    for i in 0..1000i64 {
        test_numbers.push(Number(i as f64));
    }
    for bits in 0..53 {
        test_numbers.push(Number((1i64 << bits) as f64));
        test_numbers.push(Number(((1i64 << bits) - 1) as f64));
    }

    // strip base-10 strings
    for test_number in &test_numbers {
        for leading_zeros in 0..10 {
            assert_eq!(
                parse_pseudo_big_int(&("0".repeat(leading_zeros) + &test_number.string() + "n")),
                test_number.string(),
            );
        }
    }

    // parse non-decimal bases (small numbers)
    let cases: Vec<(&str, &str)> = vec![
        // binary
        ("0b0n", "0"),
        ("0b1n", "1"),
        ("0b1010n", "10"),
        ("0b1010_0101n", "165"),
        ("0B1101n", "13"), // uppercase prefix
        // octal
        ("0o0n", "0"),
        ("0o7n", "7"),
        ("0o755n", "493"),
        ("0o7_5_5n", "493"),
        ("0O12n", "10"), // uppercase prefix
        // hex
        ("0x0n", "0"),
        ("0xFn", "15"),
        ("0xFFn", "255"),
        ("0xF_Fn", "255"),
        ("0X1Fn", "31"), // uppercase prefix
    ];

    for (lit, out) in cases {
        let got = parse_pseudo_big_int(lit);
        assert_eq!(got, out, "literal: {lit:?}");
    }

    // can parse large literals
    assert_eq!(parse_pseudo_big_int("123456789012345678901234567890n"), "123456789012345678901234567890");
    assert_eq!(
        parse_pseudo_big_int(
            "0b1100011101110100100001111111101101100001101110011111000001110111001001110001111110000101011010010n"
        ),
        "123456789012345678901234567890"
    );
    assert_eq!(parse_pseudo_big_int("0o143564417755415637016711617605322n"), "123456789012345678901234567890");
    assert_eq!(parse_pseudo_big_int("0x18ee90ff6c373e0ee4e3f0ad2n"), "123456789012345678901234567890");
}
