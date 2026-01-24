use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integer {
    sign: i8,        // -1, 0, +1
    limbs: Vec<u64>, // little-endian base 2^64
}

impl Integer {
    pub fn zero() -> Self {
        Self {
            sign: 0,
            limbs: Vec::new(),
        }
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
        if self.limbs.is_empty() {
            self.sign = 0;
        }
    }

    fn abs_cmp(&self, other: &Self) -> Ordering {
        match self.limbs.len().cmp(&other.limbs.len()) {
            Ordering::Equal => {}
            o => return o,
        }
        for (a, b) in self.limbs.iter().rev().zip(other.limbs.iter().rev()) {
            match a.cmp(b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }

    fn add_abs(a: &[u64], b: &[u64]) -> Vec<u64> {
        let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
        let mut carry: u128 = 0;
        let n = a.len().max(b.len());
        for i in 0..n {
            let aa = *a.get(i).unwrap_or(&0) as u128;
            let bb = *b.get(i).unwrap_or(&0) as u128;
            let s = aa + bb + carry;
            out.push(s as u64);
            carry = s >> 64;
        }
        if carry != 0 {
            out.push(carry as u64);
        }
        out
    }

    fn sub_abs(a: &[u64], b: &[u64]) -> Vec<u64> {
        // assumes |a| >= |b|
        let mut out = Vec::with_capacity(a.len());
        let mut borrow: u128 = 0;
        for (i, &a0) in a.iter().enumerate() {
            let aa = a0 as u128;
            let bb = *b.get(i).unwrap_or(&0) as u128;
            let sub = bb + borrow;
            if aa >= sub {
                out.push((aa - sub) as u64);
                borrow = 0;
            } else {
                out.push(((1u128 << 64) + aa - sub) as u64);
                borrow = 1;
            }
        }
        while out.last() == Some(&0) {
            out.pop();
        }
        out
    }

    fn mul_abs(a: &[u64], b: &[u64]) -> Vec<u64> {
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }
        let mut out = vec![0u64; a.len() + b.len()];
        for (i, &ai) in a.iter().enumerate() {
            let mut carry: u128 = 0;
            for (j, &bj) in b.iter().enumerate() {
                let idx = i + j;
                let acc = out[idx] as u128 + (ai as u128) * (bj as u128) + carry;
                out[idx] = acc as u64;
                carry = acc >> 64;
            }
            let idx = i + b.len();
            let acc = out[idx] as u128 + carry;
            out[idx] = acc as u64;
        }
        while out.last() == Some(&0) {
            out.pop();
        }
        out
    }

    fn shl_bits(limbs: &[u64], s: u32) -> Vec<u64> {
        if limbs.is_empty() {
            return Vec::new();
        }
        if s == 0 {
            return limbs.to_vec();
        }
        let mut out = Vec::with_capacity(limbs.len() + 1);
        let mut carry: u64 = 0;
        for &x in limbs {
            out.push((x << s) | carry);
            carry = x >> (64 - s);
        }
        if carry != 0 {
            out.push(carry);
        }
        out
    }

    fn shr_bits(mut limbs: Vec<u64>, s: u32) -> Vec<u64> {
        if limbs.is_empty() {
            return limbs;
        }
        if s == 0 {
            return limbs;
        }
        let mut carry: u64 = 0;
        for x in limbs.iter_mut().rev() {
            let new_carry = *x << (64 - s);
            *x = (*x >> s) | carry;
            carry = new_carry;
        }
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        limbs
    }

    fn div_mod_u64_abs(u: &[u64], d: u64) -> (Vec<u64>, u64) {
        debug_assert!(d != 0);
        let mut q = Vec::with_capacity(u.len());
        let mut rem: u128 = 0;
        for &x in u.iter().rev() {
            let acc = (rem << 64) + x as u128;
            let qq = acc / d as u128;
            rem = acc % d as u128;
            q.push(qq as u64);
        }
        q.reverse();
        while q.last() == Some(&0) {
            q.pop();
        }
        (q, rem as u64)
    }

    fn div_mod_small_abs(&self, d: u64) -> (Self, u64) {
        debug_assert!(self.sign >= 0);
        debug_assert!(d != 0);
        if self.sign == 0 {
            return (Self::zero(), 0);
        }

        let (q, rem) = Self::div_mod_u64_abs(&self.limbs, d);
        let mut qn = Self {
            sign: if q.is_empty() { 0 } else { 1 },
            limbs: q,
        };
        qn.normalize();
        (qn, rem)
    }

    fn div_mod_abs(u: &[u64], v: &[u64]) -> (Vec<u64>, Vec<u64>) {
        // Knuth division (Algorithm D), base 2^64.
        debug_assert!(!v.is_empty());
        if u.is_empty() {
            return (Vec::new(), Vec::new());
        }
        if u.len() < v.len() {
            return (Vec::new(), u.to_vec());
        }
        if v.len() == 1 {
            let (q, rem) = Self::div_mod_u64_abs(u, v[0]);
            let r = if rem == 0 { Vec::new() } else { vec![rem] };
            return (q, r);
        }

        let n = v.len();
        let m = u.len() - n;
        let s = v[n - 1].leading_zeros();

        let vn = Self::shl_bits(v, s);
        let mut un = Self::shl_bits(u, s);
        un.push(0);

        let base: u128 = 1u128 << 64;
        let v1 = vn[n - 1] as u128;
        let v2 = vn[n - 2] as u128;

        let mut q = vec![0u64; m + 1];

        for j in (0..=m).rev() {
            let ujn = un[j + n] as u128;
            let ujn1 = un[j + n - 1] as u128;
            let ujn2 = un[j + n - 2] as u128;

            let mut qhat = ((ujn << 64) + ujn1) / v1;
            let mut rhat = ((ujn << 64) + ujn1) % v1;
            if qhat == base {
                qhat -= 1;
                rhat += v1;
            }
            while qhat * v2 > (rhat << 64) + ujn2 {
                qhat -= 1;
                rhat += v1;
                if rhat >= base {
                    break;
                }
            }

            let mut borrow: u128 = 0;
            for i in 0..n {
                let p = qhat * (vn[i] as u128);
                let sub = (p & (base - 1)) + borrow;
                let uval = un[j + i] as u128;
                if uval >= sub {
                    un[j + i] = (uval - sub) as u64;
                    borrow = p >> 64;
                } else {
                    un[j + i] = (base + uval - sub) as u64;
                    borrow = (p >> 64) + 1;
                }
            }

            let uval = un[j + n] as u128;
            let negative = if uval >= borrow {
                un[j + n] = (uval - borrow) as u64;
                false
            } else {
                un[j + n] = (base + uval - borrow) as u64;
                true
            };

            let mut qhat_u64 = qhat as u64;
            if negative {
                qhat_u64 = qhat_u64.wrapping_sub(1);
                let mut carry: u128 = 0;
                for i in 0..n {
                    let acc = (un[j + i] as u128) + (vn[i] as u128) + carry;
                    un[j + i] = acc as u64;
                    carry = acc >> 64;
                }
                let acc = (un[j + n] as u128) + carry;
                un[j + n] = acc as u64;
            }

            q[j] = qhat_u64;
        }

        while q.last() == Some(&0) {
            q.pop();
        }

        let mut r = un[..n].to_vec();
        r = Self::shr_bits(r, s);
        while r.last() == Some(&0) {
            r.pop();
        }

        (q, r)
    }

    fn abs(&self) -> Self {
        let mut out = self.clone();
        if out.sign < 0 {
            out.sign = 1;
        }
        out
    }

    pub fn is_zero(&self) -> bool {
        self.sign == 0
    }

    pub fn in_i32_range(&self) -> bool {
        let min = Integer::from(i32::MIN as i64);
        let max = Integer::from(i32::MAX as i64);
        self >= &min && self <= &max
    }

    pub fn in_i64_range(&self) -> bool {
        let min = Integer::from(i64::MIN);
        let max = Integer::from(i64::MAX);
        self >= &min && self <= &max
    }

    pub fn into_i32_range_checked(self) -> i32 {
        self.to_string()
            .parse::<i32>()
            .expect("range-checked Integer should parse to i32")
    }

    pub fn into_i64_range_checked(self) -> i64 {
        self.to_string()
            .parse::<i64>()
            .expect("range-checked Integer should parse to i64")
    }
}

impl From<i64> for Integer {
    fn from(n: i64) -> Self {
        if n == 0 {
            return Self::zero();
        }
        let sign = if n < 0 { -1 } else { 1 };
        let mag: u64 = if n < 0 {
            (-(n as i128)) as u64
        } else {
            n as u64
        };
        Self {
            sign,
            limbs: vec![mag],
        }
    }
}

impl Neg for Integer {
    type Output = Self;
    fn neg(mut self) -> Self {
        if self.sign != 0 {
            self.sign = -self.sign;
        }
        self
    }
}

impl Add for Integer {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        if self.sign == 0 {
            return other;
        }
        if other.sign == 0 {
            return self;
        }
        if self.sign == other.sign {
            let mut out = Self {
                sign: self.sign,
                limbs: Self::add_abs(&self.limbs, &other.limbs),
            };
            out.normalize();
            return out;
        }
        match self.abs_cmp(&other) {
            Ordering::Equal => Self::zero(),
            Ordering::Greater => {
                let mut out = Self {
                    sign: self.sign,
                    limbs: Self::sub_abs(&self.limbs, &other.limbs),
                };
                out.normalize();
                out
            }
            Ordering::Less => {
                let mut out = Self {
                    sign: other.sign,
                    limbs: Self::sub_abs(&other.limbs, &self.limbs),
                };
                out.normalize();
                out
            }
        }
    }
}

impl Sub for Integer {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + (-other)
    }
}

impl Mul for Integer {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        if self.sign == 0 || other.sign == 0 {
            return Self::zero();
        }
        let mut out = Self {
            sign: self.sign * other.sign,
            limbs: Self::mul_abs(&self.limbs, &other.limbs),
        };
        out.normalize();
        out
    }
}

impl Div for Integer {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        assert!(other.sign != 0, "division by zero");
        if self.sign == 0 {
            return Self::zero();
        }
        let a = self.abs();
        let b = other.abs();
        let (q, _) = Self::div_mod_abs(&a.limbs, &b.limbs);
        let mut out = Self {
            sign: self.sign * other.sign,
            limbs: q,
        };
        out.normalize();
        out
    }
}

impl Ord for Integer {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.sign.cmp(&other.sign) {
            Ordering::Equal => {}
            o => return o,
        }
        if self.sign == 0 {
            return Ordering::Equal;
        }
        let c = self.abs_cmp(other);
        if self.sign > 0 {
            c
        } else {
            c.reverse()
        }
    }
}

impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sign == 0 {
            return write!(f, "0");
        }
        if self.sign < 0 {
            write!(f, "-")?;
        }
        let mut x = self.abs();
        let mut digits = Vec::new();
        while !x.is_zero() {
            let (q, r) = x.div_mod_small_abs(10);
            digits.push((b'0' + (r as u8)) as char);
            x = q;
        }
        for ch in digits.iter().rev() {
            write!(f, "{ch}")?;
        }
        Ok(())
    }
}

pub fn int_from_i64(n: i64) -> Integer {
    Integer::from(n)
}

pub fn parse_integer(s: &str) -> Result<Integer, &'static str> {
    let s = s.trim();
    if s.is_empty() {
        return Err("invalid integer");
    }
    let (sign, digits) = match s.as_bytes()[0] {
        b'+' => (1i8, &s[1..]),
        b'-' => (-1i8, &s[1..]),
        _ => (1i8, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return Err("invalid integer");
    }
    let mut x = Integer::zero();
    for b in digits.bytes() {
        let d = (b - b'0') as u64;
        // x = x*10 + d
        // mul_small
        if x.sign != 0 {
            let mut carry: u128 = 0;
            for limb in x.limbs.iter_mut() {
                let prod = (*limb as u128) * 10u128 + carry;
                *limb = prod as u64;
                carry = prod >> 64;
            }
            if carry != 0 {
                x.limbs.push(carry as u64);
            }
        }
        // add_small
        if x.sign == 0 {
            if d != 0 {
                x.sign = 1;
                x.limbs.push(d);
            }
        } else {
            let mut carry: u128 = d as u128;
            for limb in x.limbs.iter_mut() {
                if carry == 0 {
                    break;
                }
                let sum = (*limb as u128) + carry;
                *limb = sum as u64;
                carry = sum >> 64;
            }
            if carry != 0 {
                x.limbs.push(carry as u64);
            }
        }
    }
    if x.sign != 0 {
        x.sign *= sign;
    }
    x.normalize();
    Ok(x)
}

pub fn is_zero(n: &Integer) -> bool {
    n.is_zero()
}

pub fn in_i32_range(n: &Integer) -> bool {
    n.in_i32_range()
}

pub fn in_i64_range(n: &Integer) -> bool {
    n.in_i64_range()
}

pub fn to_i32_range_checked(n: Integer) -> i32 {
    n.into_i32_range_checked()
}

pub fn to_i64_range_checked(n: Integer) -> i64 {
    n.into_i64_range_checked()
}
