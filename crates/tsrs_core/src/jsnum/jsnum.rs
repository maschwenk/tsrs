use std::hash::{Hash, Hasher};
use std::ops::{Add, Div, Mul, Neg, Sub};

use super::big;
use super::math;

pub const MaxSafeInteger: Number = Number(((1u64 << 53) - 1) as f64);
pub const MinSafeInteger: Number = Number(-MaxSafeInteger.0);

// Number represents a JS-like number.
//
// All operations that can be performed directly on this type
// (e.g., conversion, arithmetic, etc.) behave as they would in JavaScript,
// but any other operation should use this type's methods,
// not the "math" package and conversions.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Number(pub f64);

impl Eq for Number {}

impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let f = if self.0 == 0.0 { 0.0 } else { self.0 };
        f.to_bits().hash(state);
    }
}

impl Add for Number {
    type Output = Number;
    fn add(self, rhs: Number) -> Number {
        Number(self.0 + rhs.0)
    }
}

impl Sub for Number {
    type Output = Number;
    fn sub(self, rhs: Number) -> Number {
        Number(self.0 - rhs.0)
    }
}

impl Mul for Number {
    type Output = Number;
    fn mul(self, rhs: Number) -> Number {
        Number(self.0 * rhs.0)
    }
}

impl Div for Number {
    type Output = Number;
    fn div(self, rhs: Number) -> Number {
        Number(self.0 / rhs.0)
    }
}

impl Neg for Number {
    type Output = Number;
    fn neg(self) -> Number {
        Number(-self.0)
    }
}

pub fn nan() -> Number {
    Number(f64::from_bits(0x7FF8000000000001))
}

impl Number {
    pub fn is_nan(self) -> bool {
        self.0.is_nan()
    }
}

pub fn inf(sign: i32) -> Number {
    if sign >= 0 {
        Number(f64::INFINITY)
    } else {
        Number(f64::NEG_INFINITY)
    }
}

impl Number {
    pub fn is_inf(self) -> bool {
        self.0.is_infinite()
    }
}

pub(crate) fn is_non_finite(x: f64) -> bool {
    // This is equivalent to checking `math.IsNaN(x) || math.IsInf(x, 0)` in one operation.
    const mask: u64 = 0x7FF0000000000000;
    x.to_bits() & mask == mask
}

impl Number {
    // https://tc39.es/ecma262/2024/multipage/abstract-operations.html#sec-touint32
    pub(crate) fn to_uint32(self) -> u32 {
        // The only difference between ToUint32 and ToInt32 is the interpretation of the bits.
        self.to_int32() as u32
    }

    // https://tc39.es/ecma262/2024/multipage/abstract-operations.html#sec-toint32
    pub(crate) fn to_int32(self) -> i32 {
        let mut x = self.0;

        // Fast path: if the number is in the range (-2^31, 2^32), i.e. an SMI,
        // then we don't need to do any special mapping.
        let smi = x as i32;
        if smi as f64 == x {
            return smi;
        }

        // 2. If number is not finite or number is either +0𝔽 or -0𝔽, return +0𝔽.
        // Zero was covered by the test above.
        if is_non_finite(x) {
            return 0;
        }

        // Let int be truncate(ℝ(number)).
        x = x.trunc();
        // Let int32bit be int modulo 2**32.
        x %= (1u64 << 32) as f64;
        // If int32bit ≥ 2**31, return 𝔽(int32bit - 2**32); otherwise return 𝔽(int32bit).
        x as i64 as i32
    }

    pub(crate) fn to_shift_count(self) -> u32 {
        self.to_uint32() & 31
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-signedRightShift
    pub fn signed_right_shift(self, y: Number) -> Number {
        Number((self.to_int32() >> y.to_shift_count()) as f64)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-unsignedRightShift
    pub fn unsigned_right_shift(self, y: Number) -> Number {
        Number((self.to_uint32() >> y.to_shift_count()) as f64)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-leftShift
    pub fn left_shift(self, y: Number) -> Number {
        Number((self.to_int32() << y.to_shift_count()) as f64)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-bitwiseNOT
    pub fn bitwise_not(self) -> Number {
        Number((!self.to_int32()) as f64)
    }

    // The below are implemented by https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numberbitwiseop.

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-bitwiseOR
    pub fn bitwise_or(self, y: Number) -> Number {
        Number((self.to_int32() | y.to_int32()) as f64)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-bitwiseAND
    pub fn bitwise_and(self, y: Number) -> Number {
        Number((self.to_int32() & y.to_int32()) as f64)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-bitwiseXOR
    pub fn bitwise_xor(self, y: Number) -> Number {
        Number((self.to_int32() ^ y.to_int32()) as f64)
    }

    pub(crate) fn trunc(self) -> Number {
        Number(self.0.trunc())
    }

    pub fn floor(self) -> Number {
        Number(self.0.floor())
    }

    pub fn abs(self) -> Number {
        Number(self.0.abs())
    }
}

pub(crate) const negative_zero: Number = Number(-0.0);

impl Number {
    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-remainder
    pub fn remainder(self, d: Number) -> Number {
        let n = self;
        if n.is_nan() || d.is_nan() {
            return nan();
        } else if n.is_inf() {
            return nan();
        } else if d.is_inf() {
            return n;
        } else if d == Number(0.0) {
            return nan();
        } else if n == Number(0.0) {
            return n;
        }
        Number(n.0 % d.0)
    }

    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-exponentiate
    pub fn exponentiate(self, exponent: Number) -> Number {
        let base = self;
        if (base == Number(1.0) || base == Number(-1.0)) && exponent.is_inf() {
            return nan();
        } else if base == Number(1.0) && exponent.is_nan() {
            return nan();
        }

        let b = base.0;
        let e = exponent.0;

        // For integer base ** integer exponent where the result exceeds 53 bits,
        // math.Pow can be off by multiple ULPs vs JS engines. Use exact big.Int
        // arithmetic and IEEE 754 round-to-nearest-even conversion instead.
        // The ES spec (§6.1.6.1.3) says exponentiate returns an
        // "implementation-approximated" value, so engines are allowed to differ.
        // This won't exactly match every engine (V8's fdlibm-compiled pow can
        // round halfway ties differently), but will always be within 1 ULP
        // (unit in the last place, i.e. the least significant bit of the result).
        if b >= i64::MIN as f64
            && b <= i64::MAX as f64
            && b == b.trunc()
            && e >= 0.0
            && e <= i64::MAX as f64
            && e == e.trunc()
            && !e.is_infinite()
        {
            let magnitude = e * math::log2(b.abs());
            if magnitude > 53.0 && magnitude <= math::log2(f64::MAX) {
                let ri = big::Int::new(b as i64).exp(&big::Int::new(e as i64));
                let result = ri.round_to_prec(256).float64();
                return Number(result);
            }
        }

        Number(math::pow(b, e))
    }
}
