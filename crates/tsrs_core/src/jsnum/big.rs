// The subset of Go's math/big used by this package: arbitrary-precision
// integers with Go's SetString/String/Exp semantics and correctly rounded
// (round-to-nearest-even) conversion to float64.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Int {
    neg: bool,
    // Little-endian 32-bit limbs with no most-significant zero limbs.
    abs: Vec<u32>,
}

fn norm(z: &mut Vec<u32>) {
    while z.last() == Some(&0) {
        z.pop();
    }
}

fn mul_add_ww(z: &mut Vec<u32>, y: u32, r: u32) {
    let mut carry = r as u64;
    for limb in z.iter_mut() {
        let t = (*limb as u64) * (y as u64) + carry;
        *limb = t as u32;
        carry = t >> 32;
    }
    if carry != 0 {
        z.push(carry as u32);
    }
    norm(z);
}

fn div_w(z: &mut [u32], y: u32) -> u32 {
    let mut r: u64 = 0;
    for limb in z.iter_mut().rev() {
        let t = (r << 32) | (*limb as u64);
        *limb = (t / y as u64) as u32;
        r = t % y as u64;
    }
    r as u32
}

fn mul(x: &[u32], y: &[u32]) -> Vec<u32> {
    if x.is_empty() || y.is_empty() {
        return Vec::new();
    }
    let mut z = vec![0u32; x.len() + y.len()];
    for (i, &xi) in x.iter().enumerate() {
        let mut carry: u64 = 0;
        for (j, &yj) in y.iter().enumerate() {
            let t = (xi as u64) * (yj as u64) + (z[i + j] as u64) + carry;
            z[i + j] = t as u32;
            carry = t >> 32;
        }
        z[i + y.len()] = carry as u32;
    }
    norm(&mut z);
    z
}

fn bit_len(x: &[u32]) -> usize {
    match x.last() {
        None => 0,
        Some(&top) => (x.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
    }
}

fn bit(x: &[u32], i: usize) -> bool {
    let w = i / 32;
    w < x.len() && (x[w] >> (i % 32)) & 1 == 1
}

// Reports whether any of the bits below position i are set.
fn sticky(x: &[u32], i: usize) -> bool {
    let w = i / 32;
    if x[..w.min(x.len())].iter().any(|&l| l != 0) {
        return true;
    }
    let b = i % 32;
    w < x.len() && b != 0 && x[w] & ((1u32 << b) - 1) != 0
}

fn shr(x: &[u32], s: usize) -> Vec<u32> {
    let ws = s / 32;
    let bs = s % 32;
    if ws >= x.len() {
        return Vec::new();
    }
    let mut z = Vec::with_capacity(x.len() - ws);
    for i in ws..x.len() {
        let lo = x[i] >> bs;
        let hi = if bs != 0 && i + 1 < x.len() { x[i + 1] << (32 - bs) } else { 0 };
        z.push(lo | hi);
    }
    norm(&mut z);
    z
}

fn shl(x: &[u32], s: usize) -> Vec<u32> {
    if x.is_empty() {
        return Vec::new();
    }
    let ws = s / 32;
    let bs = s % 32;
    let mut z = vec![0u32; ws];
    let mut carry = 0u32;
    for &l in x {
        z.push((l << bs) | carry);
        carry = if bs != 0 { l >> (32 - bs) } else { 0 };
    }
    if carry != 0 {
        z.push(carry);
    }
    norm(&mut z);
    z
}

fn low64(x: &[u32]) -> u64 {
    let lo = x.first().copied().unwrap_or(0) as u64;
    let hi = x.get(1).copied().unwrap_or(0) as u64;
    (hi << 32) | lo
}

impl Int {
    pub(crate) fn new(x: i64) -> Int {
        let u = x.unsigned_abs();
        let mut abs = vec![u as u32, (u >> 32) as u32];
        norm(&mut abs);
        Int { neg: x < 0, abs }
    }

    // Go's (*big.Int).SetString: an optional sign, then digits in the given base.
    // For base 0 the base is chosen by the prefix (0b, 0o, 0x, or 0 for octal)
    // and underscores may separate digits. The entire string must be consumed.
    pub(crate) fn set_string(s: &str, base: u32) -> Option<Int> {
        let bytes = s.as_bytes();
        let mut pos = 0usize;
        let read_byte = |pos: &mut usize| -> Option<u8> {
            if *pos < bytes.len() {
                *pos += 1;
                Some(bytes[*pos - 1])
            } else {
                None
            }
        };

        // scanSign
        let neg = match read_byte(&mut pos) {
            None => return None,
            Some(b'-') => true,
            Some(b'+') => false,
            Some(_) => {
                pos -= 1;
                false
            }
        };

        // nat.scan
        let mut prev = b'.';
        let mut inval_sep = false;
        let mut ch = read_byte(&mut pos);
        let mut b = base;
        let mut prefix = 0u8;
        let mut count = 0usize;
        if base == 0 {
            b = 10;
            if ch == Some(b'0') {
                prev = b'0';
                count = 1;
                ch = read_byte(&mut pos);
                if let Some(c) = ch {
                    match c {
                        b'b' | b'B' => {
                            b = 2;
                            prefix = b'b';
                        }
                        b'o' | b'O' => {
                            b = 8;
                            prefix = b'o';
                        }
                        b'x' | b'X' => {
                            b = 16;
                            prefix = b'x';
                        }
                        _ => {
                            b = 8;
                            prefix = b'0';
                        }
                    }
                    if prefix != 0 {
                        count = 0;
                        if prefix != b'0' {
                            ch = read_byte(&mut pos);
                        }
                    }
                }
            }
        }

        let mut z: Vec<u32> = Vec::new();
        while let Some(c) = ch {
            if c == b'_' && base == 0 {
                if prev != b'0' {
                    inval_sep = true;
                }
                prev = b'_';
            } else {
                let d1 = match c {
                    b'0'..=b'9' => (c - b'0') as u32,
                    b'a'..=b'z' => (c - b'a' + 10) as u32,
                    b'A'..=b'Z' => (c - b'A' + 10) as u32,
                    _ => u32::MAX,
                };
                if d1 >= b {
                    pos -= 1;
                    break;
                }
                prev = b'0';
                count += 1;
                mul_add_ww(&mut z, b, d1);
            }
            ch = read_byte(&mut pos);
        }

        let mut err = inval_sep || prev == b'_';
        if count == 0 {
            if prefix == b'0' {
                z.clear();
            } else {
                err = true;
            }
        }
        if err {
            return None;
        }

        if pos != bytes.len() {
            return None;
        }
        Some(Int { neg: !z.is_empty() && neg, abs: z })
    }

    // Go's (*big.Int).Exp(x, y, nil).
    pub(crate) fn exp(&self, y: &Int) -> Int {
        if y.neg || y.abs.is_empty() {
            return Int::new(1);
        }
        let mut result: Vec<u32> = vec![1];
        let mut base = self.abs.clone();
        let n = bit_len(&y.abs);
        for i in 0..n {
            if bit(&y.abs, i) {
                result = mul(&result, &base);
            }
            if i + 1 < n {
                base = mul(&base, &base);
            }
        }
        let odd = bit(&y.abs, 0);
        Int { neg: self.neg && odd && !result.is_empty(), abs: result }
    }

    pub(crate) fn mul(&self, y: &Int) -> Int {
        let abs = mul(&self.abs, &y.abs);
        Int { neg: self.neg != y.neg && !abs.is_empty(), abs }
    }

    pub(crate) fn lsh(&self, n: usize) -> Int {
        Int { neg: self.neg, abs: shl(&self.abs, n) }
    }

    // Rounds to prec significant bits (round-to-nearest-even), like
    // new(big.Float).SetPrec(prec).SetInt(x).
    pub(crate) fn round_to_prec(&self, prec: usize) -> Int {
        let n = bit_len(&self.abs);
        if n <= prec {
            return self.clone();
        }
        let s = n - prec;
        let mut q = shr(&self.abs, s);
        let half = bit(&self.abs, s - 1);
        if half && (sticky(&self.abs, s - 1) || q[0] & 1 == 1) {
            mul_add_ww(&mut q, 1, 1);
        }
        Int { neg: self.neg, abs: shl(&q, s) }
    }

    // Go's (*big.Int).Float64, dropping the accuracy result.
    pub(crate) fn float64(&self) -> f64 {
        let n = bit_len(&self.abs);
        if n == 0 {
            return 0.0;
        }
        let f = if n <= 64 {
            low64(&self.abs) as f64
        } else if n > 1024 {
            f64::INFINITY
        } else {
            let s = n - 64;
            let mut top = low64(&shr(&self.abs, s));
            if sticky(&self.abs, s) {
                top |= 1;
            }
            (top as f64) * 2f64.powi(s as i32)
        };
        if self.neg {
            -f
        } else {
            f
        }
    }

    pub(crate) fn string(&self) -> String {
        if self.abs.is_empty() {
            return "0".to_string();
        }
        let mut q = self.abs.clone();
        let mut chunks = Vec::new();
        while !q.is_empty() {
            chunks.push(div_w(&mut q, 1_000_000_000));
            norm(&mut q);
        }
        let mut s = String::new();
        if self.neg {
            s.push('-');
        }
        let mut iter = chunks.iter().rev();
        if let Some(first) = iter.next() {
            s.push_str(&first.to_string());
        }
        for chunk in iter {
            s.push_str(&format!("{chunk:09}"));
        }
        s
    }
}
