//! Python `decimal.Decimal` in the default context (precision 28, or another precision set with
//! [`with_precision`]): rounding ROUND_HALF_EVEN,
//! Emax 999999, Emin -999999, with the signals InvalidOperation, DivisionByZero and Overflow
//! trapped (raised as exceptions). The algorithms follow CPython's `_pydecimal`, so values,
//! exponents (visible in `str` and `repr`) and raised exceptions match the reference
//! implementation of FEEL, which computes with `Decimal`.

use std::cmp::Ordering;

use crate::pyutil::{PyError, decimal_value, is_space};

/// Precision of Python's default context.
const DEFAULT_PREC: i64 = 28;
const EMAX: i64 = 999_999;
const EMIN: i64 = -999_999;

thread_local! {
    /// Precision of the arithmetic on this thread: 28 digits (Python's default context)
    /// unless [`with_precision`] runs a computation with another one.
    static PRECISION: std::cell::Cell<i64> = const { std::cell::Cell::new(DEFAULT_PREC) };
}

/// Run `f` with the arithmetic of this thread rounding to `digits` significant digits
/// (34 for FEEL of language 0.7, as DMN requires); the previous precision is restored.
pub fn with_precision<T>(digits: u32, f: impl FnOnce() -> T) -> T {
    let old = PRECISION.with(|p| p.replace(i64::from(digits)));
    let out = f();
    PRECISION.with(|p| p.set(old));
    out
}

/// The current precision (significant digits).
pub fn precision() -> u32 {
    PRECISION.with(|p| p.get()) as u32
}

fn prec() -> i64 {
    PRECISION.with(|p| p.get())
}
fn etiny() -> i64 {
    EMIN - prec() + 1
}
fn etop() -> i64 {
    EMAX - prec() + 1
}
/// Largest exponent accepted by the constructor (libmpdec `MPD_MAX_EMAX` on 64-bit systems).
const MAX_EMAX: i128 = 999_999_999_999_999_999;
/// Smallest exponent accepted by the constructor (libmpdec `MPD_MIN_ETINY`).
const MIN_ETINY: i128 = -999_999_999_999_999_999 - (999_999_999_999_999_999 - 1);
/// Integers longer than this are refused with `MemoryError` instead of being built.
const MAX_DIGITS: i64 = 100_000_000;

/// Rounding modes used by FEEL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    HalfEven,
    HalfUp,
    Floor,
    Ceiling,
    Down,
    /// Away from zero (ROUND_UP).
    Up,
    /// To nearest, ties towards zero (ROUND_HALF_DOWN).
    HalfDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Finite,
    Inf,
    NaN,
    SNaN,
}

/// A decimal number: `(-1)^sign * coefficient * 10^exp`, or an infinity or a NaN.
#[derive(Debug, Clone)]
pub struct Decimal {
    sign: bool,
    /// Coefficient digits (values 0 to 9), most significant first, without leading zeros
    /// (`[0]` for zero). For a NaN: the diagnostic payload (possibly empty).
    int: Vec<u8>,
    exp: i64,
    kind: Kind,
}

fn invalid() -> PyError {
    PyError::new(
        "decimal.InvalidOperation",
        "[<class 'decimal.InvalidOperation'>]",
    )
}

fn conversion_syntax() -> PyError {
    PyError::new(
        "decimal.InvalidOperation",
        "[<class 'decimal.ConversionSyntax'>]",
    )
}

fn overflow() -> PyError {
    PyError::new("decimal.Overflow", "[<class 'decimal.Overflow'>]")
}

fn memory_error() -> PyError {
    PyError::new("MemoryError", "")
}

/// True when `e` is `decimal.InvalidOperation` (the error FEEL arithmetic turns into null).
pub fn is_invalid_operation(e: &PyError) -> bool {
    e.kind == "decimal.InvalidOperation"
}

// ------------------------------------------------------------ coefficient arithmetic

fn trim(mut v: Vec<u8>) -> Vec<u8> {
    let nz = v
        .iter()
        .position(|d| *d != 0)
        .unwrap_or(v.len().saturating_sub(1));
    if nz > 0 {
        v.drain(..nz);
    }
    if v.is_empty() {
        v.push(0);
    }
    v
}

fn cmp_mag(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn add_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    let n = a.len().max(b.len());
    let mut out = vec![0u8; n + 1];
    let mut carry = 0u8;
    for k in 0..n {
        let x = if k < a.len() { a[a.len() - 1 - k] } else { 0 };
        let y = if k < b.len() { b[b.len() - 1 - k] } else { 0 };
        let s = x + y + carry;
        out[n - k] = s % 10;
        carry = s / 10;
    }
    out[0] = carry;
    trim(out)
}

/// `a - b` for `a >= b`.
fn sub_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = a.to_vec();
    let mut borrow = 0i8;
    for k in 0..a.len() {
        let i = a.len() - 1 - k;
        let y = if k < b.len() {
            b[b.len() - 1 - k] as i8
        } else {
            0
        };
        let mut d = a[i] as i8 - y - borrow;
        if d < 0 {
            d += 10;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out[i] = d as u8;
    }
    trim(out)
}

fn mul_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut acc = vec![0u64; a.len() + b.len()];
    for (i, x) in a.iter().rev().enumerate() {
        if *x == 0 {
            continue;
        }
        for (j, y) in b.iter().rev().enumerate() {
            acc[i + j] += (*x as u64) * (*y as u64);
        }
        // keep the accumulators small
        let mut carry = 0u64;
        for v in acc.iter_mut() {
            let s = *v + carry;
            *v = s % 10;
            carry = s / 10;
        }
    }
    let mut carry = 0u64;
    for v in acc.iter_mut() {
        let s = *v + carry;
        *v = s % 10;
        carry = s / 10;
    }
    trim(acc.iter().rev().map(|d| *d as u8).collect())
}

/// `(a // b, a % b)` for `b > 0`.
fn divmod_mag(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut q = Vec::with_capacity(a.len());
    let mut r: Vec<u8> = vec![0];
    for d in a {
        r.push(*d);
        r = trim(r);
        let mut k = 0u8;
        while cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            k += 1;
        }
        q.push(k);
    }
    (trim(q), r)
}

fn shift(a: &[u8], k: i64) -> Vec<u8> {
    let mut v = a.to_vec();
    if !(v.len() == 1 && v[0] == 0) {
        v.extend(std::iter::repeat_n(0u8, k.max(0) as usize));
    }
    v
}

fn is_zero_mag(a: &[u8]) -> bool {
    a.len() == 1 && a[0] == 0
}

fn last_digit(a: &[u8]) -> u8 {
    a.last().copied().unwrap_or(0)
}

fn digits_text(a: &[u8]) -> String {
    a.iter().map(|d| char::from(b'0' + d)).collect()
}

impl Decimal {
    fn finite(sign: bool, int: Vec<u8>, exp: i64) -> Self {
        Decimal {
            sign,
            int: trim(int),
            exp,
            kind: Kind::Finite,
        }
    }

    fn infinity(sign: bool) -> Self {
        Decimal {
            sign,
            int: vec![0],
            exp: 0,
            kind: Kind::Inf,
        }
    }

    /// `Decimal(0)`.
    pub fn zero() -> Self {
        Decimal::finite(false, vec![0], 0)
    }

    /// `Decimal(n)` for an integer.
    pub fn from_i128(n: i128) -> Self {
        let digits = n
            .unsigned_abs()
            .to_string()
            .bytes()
            .map(|b| b - b'0')
            .collect();
        Decimal::finite(n < 0, digits, 0)
    }

    /// `Decimal(text)`: whitespace around, underscores anywhere, digits of any script,
    /// exponents, `Infinity`, `NaN` and `sNaN` with payloads. A malformed text raises
    /// `InvalidOperation` (ConversionSyntax).
    pub fn parse(text: &str) -> Result<Self, PyError> {
        let mut ascii = String::with_capacity(text.len());
        for c in text.chars() {
            if c == '_' {
                continue;
            }
            if c != '\0' && c.is_ascii() {
                ascii.push(c);
            } else if is_space(c) {
                ascii.push(' ');
            } else if let Some(d) = decimal_value(c) {
                ascii.push(char::from(b'0' + d as u8));
            } else {
                return Err(conversion_syntax());
            }
        }
        let s = ascii.trim_matches(is_space);
        let (sign, body) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        let low = body.to_ascii_lowercase();
        if low == "inf" || low == "infinity" {
            return Ok(Decimal::infinity(sign));
        }
        for (prefix, kind) in [("snan", Kind::SNaN), ("nan", Kind::NaN)] {
            if let Some(payload) = low.strip_prefix(prefix) {
                if !payload.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(conversion_syntax());
                }
                let digits: Vec<u8> = payload
                    .bytes()
                    .map(|b| b - b'0')
                    .skip_while(|d| *d == 0)
                    .collect();
                return Ok(Decimal {
                    sign,
                    int: digits,
                    exp: 0,
                    kind,
                });
            }
        }
        let b = body.as_bytes();
        let mut i = 0;
        let mut digits = Vec::new();
        while i < b.len() && b[i].is_ascii_digit() {
            digits.push(b[i] - b'0');
            i += 1;
        }
        let mut frac = 0i128;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                digits.push(b[i] - b'0');
                frac += 1;
                i += 1;
            }
        }
        if digits.is_empty() {
            return Err(conversion_syntax());
        }
        let mut e: i128 = 0;
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            let neg = match b.get(i) {
                Some(b'-') => {
                    i += 1;
                    true
                }
                Some(b'+') => {
                    i += 1;
                    false
                }
                _ => false,
            };
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                e = e.saturating_mul(10).saturating_add((b[i] - b'0') as i128);
                i += 1;
            }
            if i == start {
                return Err(conversion_syntax());
            }
            if neg {
                e = -e;
            }
        }
        if i != b.len() {
            return Err(conversion_syntax());
        }
        let int = trim(digits);
        let exp = e - frac;
        let adjusted = exp + int.len() as i128 - 1;
        if adjusted > MAX_EMAX || exp < MIN_ETINY {
            return Err(invalid());
        }
        Ok(Decimal::finite(sign, int, exp as i64))
    }

    pub fn is_nan(&self) -> bool {
        matches!(self.kind, Kind::NaN | Kind::SNaN)
    }

    pub fn is_infinite(&self) -> bool {
        self.kind == Kind::Inf
    }

    fn is_special(&self) -> bool {
        self.kind != Kind::Finite
    }

    /// Finite and equal to zero (either sign).
    pub fn is_zero(&self) -> bool {
        self.kind == Kind::Finite && is_zero_mag(&self.int)
    }

    pub fn is_negative(&self) -> bool {
        self.sign
    }

    fn adjusted(&self) -> i64 {
        self.exp + self.int.len() as i64 - 1
    }

    /// Exponent of the most significant digit (`adjusted()`).
    pub fn magnitude(&self) -> i64 {
        self.adjusted()
    }

    /// A finite number `(-1)^negative * digits * 10^exp`, rounded to the current precision.
    pub fn from_parts(negative: bool, digits: Vec<u8>, exp: i64) -> Result<Self, PyError> {
        Decimal::finite(negative, digits, exp).fix()
    }

    /// Sign, coefficient digits (most significant first) and exponent of a finite number.
    pub fn parts(&self) -> Option<(bool, &[u8], i64)> {
        (self.kind == Kind::Finite).then_some((self.sign, self.int.as_slice(), self.exp))
    }

    /// Finite (neither an infinity nor a NaN).
    pub fn is_finite(&self) -> bool {
        self.kind == Kind::Finite
    }

    fn nan_level(&self) -> u8 {
        match self.kind {
            Kind::NaN => 1,
            Kind::SNaN => 2,
            _ => 0,
        }
    }

    fn inf_level(&self) -> i8 {
        match (self.kind, self.sign) {
            (Kind::Inf, false) => 1,
            (Kind::Inf, true) => -1,
            _ => 0,
        }
    }

    fn copy_negate(&self) -> Self {
        let mut d = self.clone();
        d.sign = !d.sign;
        d
    }

    fn copy_abs(&self) -> Self {
        let mut d = self.clone();
        d.sign = false;
        d
    }

    /// `_check_nans`: a signaling NaN raises, a quiet NaN is the result.
    fn check_nans(&self, other: Option<&Decimal>) -> Result<Option<Decimal>, PyError> {
        let a = self.nan_level();
        let b = other.map(Decimal::nan_level).unwrap_or(0);
        if a == 2 || b == 2 {
            return Err(invalid());
        }
        if a == 1 {
            return Ok(Some(self.fix_nan()));
        }
        if let (1, Some(o)) = (b, other) {
            return Ok(Some(o.fix_nan()));
        }
        Ok(None)
    }

    fn fix_nan(&self) -> Self {
        let mut d = self.clone();
        let max = prec() as usize;
        if d.int.len() > max {
            let cut = d.int.len() - max;
            d.int = d.int[cut..]
                .iter()
                .copied()
                .skip_while(|x| *x == 0)
                .collect();
        }
        d
    }

    /// `_fix`: round to the precision and check the exponent limits.
    fn fix(&self) -> Result<Decimal, PyError> {
        match self.kind {
            Kind::NaN | Kind::SNaN => return Ok(self.fix_nan()),
            Kind::Inf => return Ok(self.clone()),
            Kind::Finite => {}
        }
        if self.is_zero() {
            let new_exp = self.exp.clamp(etiny(), EMAX);
            return Ok(Decimal::finite(self.sign, vec![0], new_exp));
        }
        let mut exp_min = self.int.len() as i64 + self.exp - prec();
        if exp_min > etop() {
            return Err(overflow());
        }
        if exp_min < etiny() {
            exp_min = etiny();
        }
        if self.exp < exp_min {
            let mut digits = self.int.len() as i64 + self.exp - exp_min;
            let mut src = self.clone();
            if digits < 0 {
                src = Decimal::finite(self.sign, vec![1], exp_min - 1);
                digits = 0;
            }
            let digits = digits as usize;
            let changed = src.round_fn(Rounding::HalfEven, digits);
            let mut coeff = if digits == 0 {
                vec![0]
            } else {
                src.int[..digits].to_vec()
            };
            if changed > 0 {
                coeff = add_mag(&coeff, &[1]);
                if coeff.len() as i64 > prec() {
                    coeff.pop();
                    exp_min += 1;
                }
            }
            if exp_min > etop() {
                return Err(overflow());
            }
            return Ok(Decimal::finite(self.sign, coeff, exp_min));
        }
        Ok(self.clone())
    }

    /// The rounding functions of `_pydecimal` (`_round_half_even` …): 1 to increment the
    /// kept digits, -1 when nonzero digits were dropped, 0 when the result is exact.
    fn round_fn(&self, rounding: Rounding, prec: usize) -> i32 {
        let all_zeros = |from: usize| {
            self.int
                .get(from..)
                .is_none_or(|r| r.iter().all(|d| *d == 0))
        };
        let down = if all_zeros(prec) { 0 } else { -1 };
        let half_up = || {
            if self.int[prec] >= 5 {
                1
            } else if all_zeros(prec) {
                0
            } else {
                -1
            }
        };
        match rounding {
            Rounding::Down => down,
            Rounding::Up => -down,
            Rounding::HalfUp => half_up(),
            Rounding::HalfDown => {
                if self.int[prec] == 5 && all_zeros(prec + 1) {
                    -1
                } else {
                    half_up()
                }
            }
            Rounding::HalfEven => {
                let exact_half = self.int[prec] == 5 && all_zeros(prec + 1);
                if exact_half && (prec == 0 || self.int[prec - 1].is_multiple_of(2)) {
                    -1
                } else {
                    half_up()
                }
            }
            Rounding::Ceiling => {
                if self.sign {
                    down
                } else {
                    -down
                }
            }
            Rounding::Floor => {
                if self.sign {
                    -down
                } else {
                    down
                }
            }
        }
    }

    /// `_rescale`: the same value (rounded) with the given exponent.
    fn rescale(&self, exp: i64, rounding: Rounding) -> Result<Decimal, PyError> {
        if self.is_special() {
            return Ok(self.clone());
        }
        if self.is_zero() {
            return Ok(Decimal::finite(self.sign, vec![0], exp));
        }
        if self.exp >= exp {
            if self.exp - exp > MAX_DIGITS {
                return Err(memory_error());
            }
            return Ok(Decimal::finite(
                self.sign,
                shift(&self.int, self.exp - exp),
                exp,
            ));
        }
        let mut digits = self.int.len() as i64 + self.exp - exp;
        let mut src = self.clone();
        if digits < 0 {
            src = Decimal::finite(self.sign, vec![1], exp - 1);
            digits = 0;
        }
        let digits = digits as usize;
        let changed = src.round_fn(rounding, digits);
        let mut coeff = if digits == 0 {
            vec![0]
        } else {
            src.int[..digits].to_vec()
        };
        if changed == 1 {
            coeff = add_mag(&coeff, &[1]);
        }
        Ok(Decimal::finite(self.sign, coeff, exp))
    }

    // ------------------------------------------------------------------ arithmetic

    /// `self + other`.
    pub fn add(&self, other: &Decimal) -> Result<Decimal, PyError> {
        if self.is_special() || other.is_special() {
            if let Some(n) = self.check_nans(Some(other))? {
                return Ok(n);
            }
            if self.is_infinite() {
                if self.sign != other.sign && other.is_infinite() {
                    return Err(invalid());
                }
                return Ok(self.clone());
            }
            if other.is_infinite() {
                return Ok(other.clone());
            }
        }
        let exp = self.exp.min(other.exp);
        if self.is_zero() && other.is_zero() {
            let sign = self.sign && other.sign;
            return Decimal::finite(sign, vec![0], exp).fix();
        }
        if self.is_zero() {
            let exp = exp.max(other.exp - prec() - 1);
            return other.rescale(exp, Rounding::HalfEven)?.fix();
        }
        if other.is_zero() {
            let exp = exp.max(self.exp - prec() - 1);
            return self.rescale(exp, Rounding::HalfEven)?.fix();
        }
        // _normalize: align the exponents, replacing a negligible operand by 10**exp
        let (mut s1, mut i1, mut e1) = (self.sign, self.int.clone(), self.exp);
        let (mut s2, mut i2, mut e2) = (other.sign, other.int.clone(), other.exp);
        {
            let swap = e1 < e2;
            let (tmp_i, tmp_e, oth_i, oth_e) = if swap {
                (&mut i2, &mut e2, &mut i1, &mut e1)
            } else {
                (&mut i1, &mut e1, &mut i2, &mut e2)
            };
            let tmp_len = tmp_i.len() as i64;
            let other_len = oth_i.len() as i64;
            let lim = *tmp_e + (-1i64).min(tmp_len - prec() - 2);
            if other_len + *oth_e - 1 < lim {
                *oth_i = vec![1];
                *oth_e = lim;
            }
            *tmp_i = shift(tmp_i, *tmp_e - *oth_e);
            *tmp_e = *oth_e;
        }
        let result_sign;
        if s1 != s2 {
            match cmp_mag(&i1, &i2) {
                Ordering::Equal => {
                    return Decimal::finite(false, vec![0], exp).fix();
                }
                Ordering::Less => {
                    std::mem::swap(&mut i1, &mut i2);
                    std::mem::swap(&mut s1, &mut s2);
                    std::mem::swap(&mut e1, &mut e2);
                }
                Ordering::Greater => {}
            }
            if s1 {
                result_sign = true;
                std::mem::swap(&mut s1, &mut s2);
            } else {
                result_sign = false;
            }
        } else if s1 {
            result_sign = true;
            s2 = false;
        } else {
            result_sign = false;
        }
        let int = if !s2 {
            add_mag(&i1, &i2)
        } else {
            sub_mag(&i1, &i2)
        };
        Decimal::finite(result_sign, int, e1).fix()
    }

    /// `self - other`.
    pub fn sub(&self, other: &Decimal) -> Result<Decimal, PyError> {
        if (self.is_special() || other.is_special())
            && let Some(n) = self.check_nans(Some(other))?
        {
            return Ok(n);
        }
        self.add(&other.copy_negate())
    }

    /// `self * other`.
    pub fn mul(&self, other: &Decimal) -> Result<Decimal, PyError> {
        let sign = self.sign ^ other.sign;
        if self.is_special() || other.is_special() {
            if let Some(n) = self.check_nans(Some(other))? {
                return Ok(n);
            }
            if self.is_infinite() {
                if other.is_zero() {
                    return Err(invalid());
                }
                return Ok(Decimal::infinity(sign));
            }
            if other.is_infinite() {
                if self.is_zero() {
                    return Err(invalid());
                }
                return Ok(Decimal::infinity(sign));
            }
        }
        let exp = self.exp + other.exp;
        if self.is_zero() || other.is_zero() {
            return Decimal::finite(sign, vec![0], exp).fix();
        }
        Decimal::finite(sign, mul_mag(&self.int, &other.int), exp).fix()
    }

    /// `self / other`.
    pub fn div(&self, other: &Decimal) -> Result<Decimal, PyError> {
        let sign = self.sign ^ other.sign;
        if self.is_special() || other.is_special() {
            if let Some(n) = self.check_nans(Some(other))? {
                return Ok(n);
            }
            if self.is_infinite() && other.is_infinite() {
                return Err(invalid());
            }
            if self.is_infinite() {
                return Ok(Decimal::infinity(sign));
            }
            if other.is_infinite() {
                return Ok(Decimal::finite(sign, vec![0], etiny()));
            }
        }
        if other.is_zero() {
            if self.is_zero() {
                return Err(PyError::new(
                    "decimal.InvalidOperation",
                    "[<class 'decimal.DivisionUndefined'>]",
                ));
            }
            return Err(PyError::new(
                "decimal.DivisionByZero",
                "[<class 'decimal.DivisionByZero'>]",
            ));
        }
        let (coeff, exp) = if self.is_zero() {
            (vec![0], self.exp - other.exp)
        } else {
            let sh = other.int.len() as i64 - self.int.len() as i64 + prec() + 1;
            let mut exp = self.exp - other.exp - sh;
            let (mut coeff, rem) = if sh >= 0 {
                divmod_mag(&shift(&self.int, sh), &other.int)
            } else {
                divmod_mag(&self.int, &shift(&other.int, -sh))
            };
            if !is_zero_mag(&rem) {
                if last_digit(&coeff).is_multiple_of(5) {
                    coeff = add_mag(&coeff, &[1]);
                }
            } else {
                let ideal = self.exp - other.exp;
                while exp < ideal && last_digit(&coeff) == 0 && !is_zero_mag(&coeff) {
                    coeff.pop();
                    exp += 1;
                }
            }
            (coeff, exp)
        };
        Decimal::finite(sign, coeff, exp).fix()
    }

    /// `-self`.
    pub fn neg(&self) -> Result<Decimal, PyError> {
        if self.is_special()
            && let Some(n) = self.check_nans(None)?
        {
            return Ok(n);
        }
        if self.is_zero() {
            return self.copy_abs().fix();
        }
        self.copy_negate().fix()
    }

    /// `+self`.
    fn pos(&self) -> Result<Decimal, PyError> {
        if self.is_special()
            && let Some(n) = self.check_nans(None)?
        {
            return Ok(n);
        }
        if self.is_zero() {
            return self.copy_abs().fix();
        }
        self.fix()
    }

    /// `abs(self)`.
    pub fn abs(&self) -> Result<Decimal, PyError> {
        if self.is_special()
            && let Some(n) = self.check_nans(None)?
        {
            return Ok(n);
        }
        if self.sign { self.neg() } else { self.pos() }
    }

    // ----------------------------------------------------------------- comparison

    /// `_cmp` for values that are not NaN.
    fn cmp_value(&self, other: &Decimal) -> Ordering {
        if self.is_special() || other.is_special() {
            return self.inf_level().cmp(&other.inf_level());
        }
        let sgn = |neg: bool| {
            if neg {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        };
        if self.is_zero() {
            if other.is_zero() {
                return Ordering::Equal;
            }
            return sgn(other.sign).reverse();
        }
        if other.is_zero() {
            return sgn(self.sign);
        }
        if other.sign && !self.sign {
            return Ordering::Greater;
        }
        if self.sign && !other.sign {
            return Ordering::Less;
        }
        let (a, b) = (self.adjusted(), other.adjusted());
        let mag = if a == b {
            let pa = shift(&self.int, self.exp - other.exp);
            let pb = shift(&other.int, other.exp - self.exp);
            pa.cmp(&pb)
        } else {
            a.cmp(&b)
        };
        if self.sign { mag.reverse() } else { mag }
    }

    /// `self == other`: false with a quiet NaN, `InvalidOperation` with a signaling NaN.
    pub fn py_eq(&self, other: &Decimal) -> Result<bool, PyError> {
        if self.kind == Kind::SNaN || other.kind == Kind::SNaN {
            return Err(invalid());
        }
        if self.is_nan() || other.is_nan() {
            return Ok(false);
        }
        Ok(self.cmp_value(other) == Ordering::Equal)
    }

    /// Ordering for `<`, `<=`, `>`, `>=`: any NaN raises `InvalidOperation`.
    pub fn py_cmp(&self, other: &Decimal) -> Result<Ordering, PyError> {
        if self.is_nan() || other.is_nan() {
            return Err(invalid());
        }
        Ok(self.cmp_value(other))
    }

    // ------------------------------------------------------- integral values, quantize

    /// `to_integral_value(rounding)`.
    pub fn to_integral_value(&self, rounding: Rounding) -> Result<Decimal, PyError> {
        if self.is_special() {
            if let Some(n) = self.check_nans(None)? {
                return Ok(n);
            }
            return Ok(self.clone());
        }
        if self.exp >= 0 {
            return Ok(self.clone());
        }
        self.rescale(0, rounding)
    }

    /// `self == self.to_integral_value()`.
    pub fn is_integral(&self) -> Result<bool, PyError> {
        let i = self.to_integral_value(Rounding::HalfEven)?;
        self.py_eq(&i)
    }

    /// `quantize(Decimal(1).scaleb(-places), rounding)` (`places` already negated by the caller
    /// is passed as the scaleb argument).
    pub fn quantize_scaleb(&self, scaleb: i128, rounding: Rounding) -> Result<Decimal, PyError> {
        let lim = 2 * (EMAX + prec()) as i128;
        if !(-lim..=lim).contains(&scaleb) {
            return Err(invalid());
        }
        let target = Decimal::finite(false, vec![1], scaleb as i64).fix()?;
        self.quantize(&target, rounding)
    }

    /// `quantize(exp, rounding)`.
    pub fn quantize(&self, exp: &Decimal, rounding: Rounding) -> Result<Decimal, PyError> {
        if self.is_special() || exp.is_special() {
            if let Some(n) = self.check_nans(Some(exp))? {
                return Ok(n);
            }
            if exp.is_infinite() || self.is_infinite() {
                if exp.is_infinite() && self.is_infinite() {
                    return Ok(self.clone());
                }
                return Err(invalid());
            }
        }
        if !(etiny()..=EMAX).contains(&exp.exp) {
            return Err(invalid());
        }
        if self.is_zero() {
            return Decimal::finite(self.sign, vec![0], exp.exp).fix();
        }
        let adj = self.adjusted();
        if adj > EMAX || adj - exp.exp + 1 > prec() {
            return Err(invalid());
        }
        let ans = self.rescale(exp.exp, rounding)?;
        if ans.adjusted() > EMAX || ans.int.len() as i64 > prec() {
            return Err(invalid());
        }
        ans.fix()
    }

    /// `normalize()`: strip trailing zeros.
    pub fn normalize(&self) -> Result<Decimal, PyError> {
        if self.is_special()
            && let Some(n) = self.check_nans(None)?
        {
            return Ok(n);
        }
        let dup = self.fix()?;
        if dup.is_infinite() {
            return Ok(dup);
        }
        if dup.is_zero() {
            return Ok(Decimal::finite(dup.sign, vec![0], 0));
        }
        let mut end = dup.int.len();
        let mut exp = dup.exp;
        while dup.int[end - 1] == 0 && exp < EMAX {
            exp += 1;
            end -= 1;
        }
        Ok(Decimal::finite(dup.sign, dup.int[..end].to_vec(), exp))
    }

    // ----------------------------------------------------------------- conversions

    /// `str(self)`: scientific notation as Python prints it.
    pub fn to_sci_string(&self) -> String {
        let sign = if self.sign { "-" } else { "" };
        match self.kind {
            Kind::Inf => return format!("{sign}Infinity"),
            Kind::NaN => return format!("{sign}NaN{}", digits_text(&self.int)),
            Kind::SNaN => return format!("{sign}sNaN{}", digits_text(&self.int)),
            Kind::Finite => {}
        }
        let digits = digits_text(&self.int);
        let n = digits.len() as i64;
        let leftdigits = self.exp + n;
        let dotplace = if self.exp <= 0 && leftdigits > -6 {
            leftdigits
        } else {
            1
        };
        let (intpart, fracpart) = if dotplace <= 0 {
            (
                "0".to_string(),
                format!(".{}{digits}", "0".repeat((-dotplace) as usize)),
            )
        } else if dotplace >= n {
            (
                format!("{digits}{}", "0".repeat((dotplace - n) as usize)),
                String::new(),
            )
        } else {
            (
                digits[..dotplace as usize].to_string(),
                format!(".{}", &digits[dotplace as usize..]),
            )
        };
        let exp = if leftdigits == dotplace {
            String::new()
        } else {
            format!("E{:+}", leftdigits - dotplace)
        };
        format!("{sign}{intpart}{fracpart}{exp}")
    }

    /// `repr(self)`: `Decimal('1.50')`.
    pub fn repr(&self) -> String {
        format!("Decimal('{}')", self.to_sci_string())
    }

    /// `format(self, "f")`.
    pub fn format_f(&self) -> Result<String, PyError> {
        let sign = if self.sign { "-" } else { "" };
        if self.is_special() {
            return Ok(format!("{sign}{}", self.copy_abs().to_sci_string()));
        }
        let mut d = self.clone();
        if d.is_zero() && d.exp > 0 {
            d = d.rescale(0, Rounding::HalfEven)?;
        }
        let digits = digits_text(&d.int);
        let n = digits.len() as i64;
        let dotplace = d.exp + n;
        if dotplace.abs() > MAX_DIGITS {
            return Err(memory_error());
        }
        let (intpart, fracpart) = if dotplace < 0 {
            (
                "0".to_string(),
                format!("{}{digits}", "0".repeat((-dotplace) as usize)),
            )
        } else if dotplace > n {
            (
                format!("{digits}{}", "0".repeat((dotplace - n) as usize)),
                String::new(),
            )
        } else {
            let ip = &digits[..dotplace as usize];
            (
                if ip.is_empty() { "0" } else { ip }.to_string(),
                digits[dotplace as usize..].to_string(),
            )
        };
        if fracpart.is_empty() {
            Ok(format!("{sign}{intpart}"))
        } else {
            Ok(format!("{sign}{intpart}.{fracpart}"))
        }
    }

    /// `float(self)`.
    pub fn to_f64(&self) -> Result<f64, PyError> {
        match self.kind {
            Kind::SNaN => Err(PyError::new(
                "ValueError",
                "cannot convert signaling NaN to float",
            )),
            Kind::NaN => Ok(if self.sign { -f64::NAN } else { f64::NAN }),
            Kind::Inf => Ok(if self.sign {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }),
            Kind::Finite => {
                let sign = if self.sign { "-" } else { "" };
                let text = format!("{sign}{}e{}", digits_text(&self.int), self.exp);
                Ok(text.parse::<f64>().unwrap_or(f64::NAN))
            }
        }
    }

    fn int_check(&self) -> Result<(), PyError> {
        match self.kind {
            Kind::NaN | Kind::SNaN => {
                Err(PyError::new("ValueError", "cannot convert NaN to integer"))
            }
            Kind::Inf => Err(PyError::new(
                "OverflowError",
                "cannot convert Infinity to integer",
            )),
            Kind::Finite => Ok(()),
        }
    }

    /// `int(self)` (truncation towards zero), saturated to the range of `i128`.
    pub fn to_i128(&self) -> Result<i128, PyError> {
        self.int_check()?;
        let keep = self.int.len() as i64 + self.exp.min(0);
        if keep <= 0 {
            return Ok(0);
        }
        let mut v: i128 = 0;
        for d in &self.int[..keep as usize] {
            v = v.saturating_mul(10).saturating_add(*d as i128);
        }
        if self.exp > 0 {
            for _ in 0..self.exp.min(40) {
                v = v.saturating_mul(10);
            }
        }
        Ok(if self.sign { -v } else { v })
    }

    /// `str(int(self))`: the integer digits of any size.
    pub fn int_text(&self) -> Result<String, PyError> {
        self.int_check()?;
        let keep = self.int.len() as i64 + self.exp.min(0);
        if keep <= 0 {
            return Ok("0".into());
        }
        if self.exp > MAX_DIGITS {
            return Err(memory_error());
        }
        let mut digits = digits_text(&self.int[..keep as usize]);
        if self.exp > 0 && digits != "0" {
            digits.push_str(&"0".repeat(self.exp as usize));
        }
        if self.sign && digits != "0" {
            digits.insert(0, '-');
        }
        Ok(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).expect("decimal")
    }

    #[test]
    fn parse_and_print() {
        assert_eq!(d("1e3").to_sci_string(), "1E+3");
        assert_eq!(d(" 1_000 ").to_sci_string(), "1000");
        assert_eq!(d("-0.0").to_sci_string(), "-0.0");
        assert_eq!(d("0E-3").to_sci_string(), "0.000");
        assert_eq!(d("NaN0012").to_sci_string(), "NaN12");
        assert_eq!(d("-inf").to_sci_string(), "-Infinity");
        assert_eq!(d("\u{663}").to_sci_string(), "3");
        assert_eq!(d("0.0000001").to_sci_string(), "1E-7");
        assert!(Decimal::parse("1e").is_err());
        assert!(Decimal::parse("1 2").is_err());
    }

    #[test]
    fn arithmetic() {
        assert_eq!(
            d("1").div(&d("3")).map(|x| x.to_sci_string()),
            Ok("0.3333333333333333333333333333".into())
        );
        assert_eq!(
            d("1E+2").div(&d("5")).map(|x| x.to_sci_string()),
            Ok("2E+1".into())
        );
        assert_eq!(
            d("1.20").mul(&d("2")).map(|x| x.to_sci_string()),
            Ok("2.40".into())
        );
        assert_eq!(
            d("1E+2").add(&d("1.0")).map(|x| x.to_sci_string()),
            Ok("101.0".into())
        );
        assert_eq!(
            d("0.1").add(&d("0.2")).map(|x| x.to_sci_string()),
            Ok("0.3".into())
        );
        assert_eq!(
            d("0").mul(&d("-1")).map(|x| x.to_sci_string()),
            Ok("-0".into())
        );
        assert_eq!(d("0").neg().map(|x| x.to_sci_string()), Ok("0".into()));
        assert_eq!(
            d("1E+999999")
                .mul(&d("10"))
                .map(|x| x.to_sci_string())
                .map_err(|e| e.kind),
            Err("decimal.Overflow".into())
        );
        assert!(d("NaN").py_cmp(&d("1")).is_err());
        assert_eq!(d("NaN").py_eq(&d("1")), Ok(false));
        assert_eq!(
            d("2.345")
                .quantize(&d("0.01"), Rounding::HalfUp)
                .map(|x| x.to_sci_string()),
            Ok("2.35".into())
        );
        assert_eq!(
            d("-0.5")
                .to_integral_value(Rounding::Ceiling)
                .map(|x| x.to_sci_string()),
            Ok("-0".into())
        );
        assert_eq!(
            d("12.50").normalize().and_then(|x| x.format_f()),
            Ok("12.5".into())
        );
    }
}
