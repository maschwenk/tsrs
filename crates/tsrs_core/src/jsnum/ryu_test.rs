use super::jsnum_test::number_from_bits;
use super::string_test::{st, StringTest};
use super::*;

// Copyright 2018 Ulf Adams
//
// The contents of this file may be used under the terms of the Apache License,
// Version 2.0.
//
//    (See accompanying file LICENSE-Apache or copy at
//     http://www.apache.org/licenses/LICENSE-2.0)
//
// Alternatively, the contents of this file may be used under the terms of
// the Boost Software License, Version 1.0.
//    (See accompanying file LICENSE-Boost or copy at
//     https://www.boost.org/LICENSE_1_0.txt)
//
// Unless required by applicable law or agreed to in writing, this software
// is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.

// Copied from https://github.com/ulfjack/ryu/blob/1264a946ba66eab320e927bfd2362e0c8580c42f/ryu/tests/d2s_test.cc
// Modified to fit Number::toString's output.

fn ieee_parts2_double(sign: bool, ieee_exponent: u32, ieee_mantissa: u64) -> Number {
    if ieee_exponent > 2047 {
        panic!("ieeeExponent > 2047");
    }
    if ieee_mantissa > max_mantissa {
        panic!("ieeeMantissa > maxMantissa");
    }
    let sign_bit: u64 = if sign { 1 } else { 0 };
    number_from_bits((sign_bit << 63) | ((ieee_exponent as u64) << 52) | ieee_mantissa)
}

const max_mantissa: u64 = (1 << 53) - 1;

pub(super) fn ryu_tests() -> Vec<StringTest> {
    vec![
        st(Number(2.2250738585072014e-308), "2.2250738585072014e-308"),
        st(number_from_bits(0x7fefffffffffffff), "1.7976931348623157e+308"),
        st(number_from_bits(1), "5e-324"),
        st(Number(2.98023223876953125e-8), "2.9802322387695312e-8"),
        st(Number(-2.109808898695963e16), "-21098088986959630"),
        st(Number(4.940656e-318), "4.940656e-318"),
        st(Number(1.18575755e-316), "1.18575755e-316"),
        st(Number(2.989102097996e-312), "2.989102097996e-312"),
        st(Number(9.0608011534336e15), "9060801153433600"),
        st(Number(4.708356024711512e18), "4708356024711512000"),
        st(Number(9.409340012568248e18), "9409340012568248000"),
        st(Number(1.2345678), "1.2345678"),
        st(number_from_bits(0x4830F0CF064DD592), "5.764607523034235e+39"),
        st(number_from_bits(0x4840F0CF064DD592), "1.152921504606847e+40"),
        st(number_from_bits(0x4850F0CF064DD592), "2.305843009213694e+40"),
        st(Number(1.2), "1.2"),
        st(Number(1.23), "1.23"),
        st(Number(1.234), "1.234"),
        st(Number(1.2345), "1.2345"),
        st(Number(1.23456), "1.23456"),
        st(Number(1.234567), "1.234567"),
        st(Number(1.2345678), "1.2345678"),
        st(Number(1.23456789), "1.23456789"),
        st(Number(1.234567895), "1.234567895"),
        st(Number(1.2345678901), "1.2345678901"),
        st(Number(1.23456789012), "1.23456789012"),
        st(Number(1.234567890123), "1.234567890123"),
        st(Number(1.2345678901234), "1.2345678901234"),
        st(Number(1.23456789012345), "1.23456789012345"),
        st(Number(1.234567890123456), "1.234567890123456"),
        st(Number(1.2345678901234567), "1.2345678901234567"),
        st(Number(4.294967294), "4.294967294"),
        st(Number(4.294967295), "4.294967295"),
        st(Number(4.294967296), "4.294967296"),
        st(Number(4.294967297), "4.294967297"),
        st(Number(4.294967298), "4.294967298"),
        st(ieee_parts2_double(false, 4, 0), "1.7800590868057611e-307"),
        st(ieee_parts2_double(false, 6, max_mantissa), "2.8480945388892175e-306"),
        st(ieee_parts2_double(false, 41, 0), "2.446494580089078e-296"),
        st(ieee_parts2_double(false, 40, max_mantissa), "4.8929891601781557e-296"),
        st(ieee_parts2_double(false, 1077, 0), "18014398509481984"),
        st(ieee_parts2_double(false, 1076, max_mantissa), "36028797018963964"),
        st(ieee_parts2_double(false, 307, 0), "2.900835519859558e-216"),
        st(ieee_parts2_double(false, 306, max_mantissa), "5.801671039719115e-216"),
        st(ieee_parts2_double(false, 934, 0x000FA7161A4D6E0C), "3.196104012172126e-27"),
        st(Number(9007199254740991.0), "9007199254740991"),
        st(Number(9007199254740992.0), "9007199254740992"),
        st(Number(1.0e+0), "1"),
        st(Number(1.2e+1), "12"),
        st(Number(1.23e+2), "123"),
        st(Number(1.234e+3), "1234"),
        st(Number(1.2345e+4), "12345"),
        st(Number(1.23456e+5), "123456"),
        st(Number(1.234567e+6), "1234567"),
        st(Number(1.2345678e+7), "12345678"),
        st(Number(1.23456789e+8), "123456789"),
        st(Number(1.23456789e+9), "1234567890"),
        st(Number(1.234567895e+9), "1234567895"),
        st(Number(1.2345678901e+10), "12345678901"),
        st(Number(1.23456789012e+11), "123456789012"),
        st(Number(1.234567890123e+12), "1234567890123"),
        st(Number(1.2345678901234e+13), "12345678901234"),
        st(Number(1.23456789012345e+14), "123456789012345"),
        st(Number(1.234567890123456e+15), "1234567890123456"),
        st(Number(1.0e+0), "1"),
        st(Number(1.0e+1), "10"),
        st(Number(1.0e+2), "100"),
        st(Number(1.0e+3), "1000"),
        st(Number(1.0e+4), "10000"),
        st(Number(1.0e+5), "100000"),
        st(Number(1.0e+6), "1000000"),
        st(Number(1.0e+7), "10000000"),
        st(Number(1.0e+8), "100000000"),
        st(Number(1.0e+9), "1000000000"),
        st(Number(1.0e+10), "10000000000"),
        st(Number(1.0e+11), "100000000000"),
        st(Number(1.0e+12), "1000000000000"),
        st(Number(1.0e+13), "10000000000000"),
        st(Number(1.0e+14), "100000000000000"),
        st(Number(1.0e+15), "1000000000000000"),
        st(Number(1000000000000001.0), "1000000000000001"),
        st(Number(1000000000000010.0), "1000000000000010"),
        st(Number(1000000000000100.0), "1000000000000100"),
        st(Number(1000000000001000.0), "1000000000001000"),
        st(Number(1000000000010000.0), "1000000000010000"),
        st(Number(1000000000100000.0), "1000000000100000"),
        st(Number(1000000001000000.0), "1000000001000000"),
        st(Number(1000000010000000.0), "1000000010000000"),
        st(Number(1000000100000000.0), "1000000100000000"),
        st(Number(1000001000000000.0), "1000001000000000"),
        st(Number(1000010000000000.0), "1000010000000000"),
        st(Number(1000100000000000.0), "1000100000000000"),
        st(Number(1001000000000000.0), "1001000000000000"),
        st(Number(1010000000000000.0), "1010000000000000"),
        st(Number(1100000000000000.0), "1100000000000000"),
        st(Number(8.0), "8"),
        st(Number(64.0), "64"),
        st(Number(512.0), "512"),
        st(Number(8192.0), "8192"),
        st(Number(65536.0), "65536"),
        st(Number(524288.0), "524288"),
        st(Number(8388608.0), "8388608"),
        st(Number(67108864.0), "67108864"),
        st(Number(536870912.0), "536870912"),
        st(Number(8589934592.0), "8589934592"),
        st(Number(68719476736.0), "68719476736"),
        st(Number(549755813888.0), "549755813888"),
        st(Number(8796093022208.0), "8796093022208"),
        st(Number(70368744177664.0), "70368744177664"),
        st(Number(562949953421312.0), "562949953421312"),
        st(Number(9007199254740992.0), "9007199254740992"),
        st(Number(8.0e+3), "8000"),
        st(Number(64.0e+3), "64000"),
        st(Number(512.0e+3), "512000"),
        st(Number(8192.0e+3), "8192000"),
        st(Number(65536.0e+3), "65536000"),
        st(Number(524288.0e+3), "524288000"),
        st(Number(8388608.0e+3), "8388608000"),
        st(Number(67108864.0e+3), "67108864000"),
        st(Number(536870912.0e+3), "536870912000"),
        st(Number(8589934592.0e+3), "8589934592000"),
        st(Number(68719476736.0e+3), "68719476736000"),
        st(Number(549755813888.0e+3), "549755813888000"),
        st(Number(8796093022208.0e+3), "8796093022208000"),
    ]
}
