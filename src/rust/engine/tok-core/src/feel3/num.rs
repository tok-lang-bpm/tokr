//! FEEL numbers of language 0.7: decimal numbers with 34 significant digits and rounding
//! half even (IEEE 754 decimal128, DMN 1.5 §10.3.2.3.1). Every operation that is not
//! defined (division by zero, a square root of a negative number, an overflow) gives
//! `None`, which the evaluator turns into `null`.

use std::cmp::Ordering;

use crate::pydec::{Decimal, Rounding, with_precision};

/// Significant digits of FEEL numbers.
pub const DIGITS: u32 = 34;
/// Working precision of the transcendental functions before the result is rounded.
const WORK: u32 = 44;

/// A finite FEEL number.
#[derive(Debug, Clone)]
pub struct Num(Decimal);

impl PartialEq for Num {
    fn eq(&self, o: &Num) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Num {}

impl PartialOrd for Num {
    fn partial_cmp(&self, o: &Num) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Num {
    /// Numbers are finite, so they are totally ordered by value.
    fn cmp(&self, o: &Num) -> Ordering {
        self.0.py_cmp(&o.0).unwrap_or(Ordering::Equal)
    }
}

fn finite(d: Decimal) -> Option<Num> {
    d.is_finite().then_some(Num(d))
}

fn at<T>(digits: u32, f: impl FnOnce() -> T) -> T {
    with_precision(digits, f)
}

impl Num {
    pub fn zero() -> Num {
        Num(Decimal::zero())
    }

    pub fn int(n: i64) -> Num {
        Num(Decimal::from_i128(i128::from(n)))
    }

    pub fn from_i128(n: i128) -> Option<Num> {
        at(DIGITS, || {
            Decimal::from_i128(n)
                .add(&Decimal::zero())
                .ok()
                .and_then(finite)
        })
    }

    /// A number from decimal text (`12`, `-0.5`, `1.23e4`), rounded to 34 digits.
    pub fn parse(text: &str) -> Option<Num> {
        let t = text.trim();
        let b = t.as_bytes();
        let body = t.strip_prefix(['-', '+']).unwrap_or(t);
        if body.is_empty()
            || !body
                .bytes()
                .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
            || b.contains(&b'_')
        {
            return None;
        }
        let d = Decimal::parse(t).ok()?;
        let (neg, digits, exp) = d.parts()?;
        at(DIGITS, || {
            Decimal::from_parts(neg, digits.to_vec(), exp)
                .ok()
                .and_then(finite)
        })
    }

    /// From a float (used only for values coming from other programs).
    pub fn from_f64(f: f64) -> Option<Num> {
        if !f.is_finite() {
            return None;
        }
        Num::parse(&format!("{f}"))
    }

    pub fn decimal(&self) -> &Decimal {
        &self.0
    }

    fn op(
        &self,
        other: &Num,
        f: impl FnOnce(&Decimal, &Decimal) -> Option<Decimal>,
    ) -> Option<Num> {
        at(DIGITS, || f(&self.0, &other.0).and_then(finite))
    }

    pub fn add(&self, o: &Num) -> Option<Num> {
        self.op(o, |a, b| a.add(b).ok())
    }
    pub fn sub(&self, o: &Num) -> Option<Num> {
        self.op(o, |a, b| a.sub(b).ok())
    }
    pub fn mul(&self, o: &Num) -> Option<Num> {
        self.op(o, |a, b| a.mul(b).ok())
    }
    pub fn div(&self, o: &Num) -> Option<Num> {
        if o.is_zero() {
            return None;
        }
        self.op(o, |a, b| a.div(b).ok())
    }
    pub fn neg(&self) -> Num {
        at(DIGITS, || {
            self.0
                .neg()
                .ok()
                .and_then(finite)
                .unwrap_or_else(|| self.clone())
        })
    }
    pub fn abs(&self) -> Num {
        if self.is_negative() {
            self.neg()
        } else {
            self.clone()
        }
    }
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
    pub fn is_negative(&self) -> bool {
        self.0.is_negative() && !self.0.is_zero()
    }
    pub fn is_integer(&self) -> bool {
        self.0.is_integral().unwrap_or(false)
    }
    /// The value as an integer, when it is one and fits.
    pub fn to_i64(&self) -> Option<i64> {
        if !self.is_integer() {
            return None;
        }
        if self.0.magnitude() > 18 {
            return None;
        }
        self.0.to_i128().ok().and_then(|v| i64::try_from(v).ok())
    }
    /// Truncated towards zero to an integer that fits.
    pub fn trunc_i64(&self) -> Option<i64> {
        if self.0.magnitude() > 18 {
            return None;
        }
        self.0.to_i128().ok().and_then(|v| i64::try_from(v).ok())
    }
    pub fn to_f64(&self) -> f64 {
        self.0.to_f64().unwrap_or(f64::NAN)
    }

    /// Rounded to `scale` digits after the point with the given rounding.
    pub fn round(&self, scale: i64, mode: Rounding) -> Option<Num> {
        if !(-6111..=6176).contains(&scale) {
            return None;
        }
        // enough digits for the integral part of the result
        let digits = (self.0.magnitude() + 1 + scale.max(0)).max(1) as u32 + 2;
        let r = at(digits.max(DIGITS), || {
            self.0.quantize_scaleb(i128::from(-scale), mode).ok()
        })?;
        let (neg, d, e) = r.parts()?;
        let d = d.to_vec();
        at(DIGITS, || {
            Decimal::from_parts(neg, d, e).ok().and_then(finite)
        })
    }

    pub fn floor(&self) -> Option<Num> {
        self.round(0, Rounding::Floor)
    }
    pub fn ceiling(&self) -> Option<Num> {
        self.round(0, Rounding::Ceiling)
    }

    /// `modulo(a, b)`: `a - b * floor(a / b)`.
    pub fn modulo(&self, o: &Num) -> Option<Num> {
        if o.is_zero() {
            return None;
        }
        let q = at(WORK, || self.0.div(&o.0).ok())?;
        let q = at(WORK, || q.to_integral_value(Rounding::Floor).ok())?;
        let p = at(WORK, || o.0.mul(&q).ok())?;
        at(DIGITS, || self.0.sub(&p).ok().and_then(finite))
    }

    /// `self ** e`; exact for integral exponents, through `exp` and `log` otherwise.
    pub fn pow(&self, e: &Num) -> Option<Num> {
        if let Some(n) = e.to_i64()
            && n.unsigned_abs() <= 999_999_999
        {
            let mut result = Decimal::from_i128(1);
            let mut base = self.0.clone();
            let mut k = n.unsigned_abs();
            let r = at(WORK, || {
                while k > 0 {
                    if k & 1 == 1 {
                        result = result.mul(&base).ok()?;
                    }
                    k >>= 1;
                    if k > 0 {
                        base = base.mul(&base).ok()?;
                    }
                }
                if n < 0 {
                    if result.is_zero() {
                        return None;
                    }
                    Decimal::from_i128(1).div(&result).ok()
                } else {
                    Some(result)
                }
            })?;
            return Num::rounded(r);
        }
        if self.is_negative() {
            return None;
        }
        if self.is_zero() {
            return if e.is_negative() {
                None
            } else {
                Some(Num::zero())
            };
        }
        let l = ln_work(&self.0)?;
        let x = at(WORK, || l.mul(&e.0).ok())?;
        Num::rounded(exp_work(&x)?)
    }

    fn rounded(d: Decimal) -> Option<Num> {
        let (neg, digits, e) = d.parts()?;
        let digits = digits.to_vec();
        at(DIGITS, || {
            Decimal::from_parts(neg, digits, e).ok().and_then(finite)
        })
    }

    pub fn sqrt(&self) -> Option<Num> {
        if self.is_negative() {
            return None;
        }
        if self.is_zero() {
            return Some(Num::zero());
        }
        let x = &self.0;
        let guess = self.to_f64().sqrt();
        let mut y = if guess.is_finite() && guess > 0.0 {
            Decimal::parse(&format!("{guess:e}")).ok()?
        } else {
            x.clone()
        };
        let two = Decimal::from_i128(2);
        let r = at(WORK, || {
            for _ in 0..200 {
                let next = y.add(&x.div(&y).ok()?).ok()?.div(&two).ok()?;
                if next.py_eq(&y).unwrap_or(false) {
                    break;
                }
                y = next;
            }
            Some(y)
        })?;
        Num::rounded(r)
    }

    /// Natural logarithm (`log`).
    pub fn ln(&self) -> Option<Num> {
        if self.is_negative() || self.is_zero() {
            return None;
        }
        Num::rounded(ln_work(&self.0)?)
    }

    pub fn exp(&self) -> Option<Num> {
        Num::rounded(exp_work(&self.0)?)
    }

    /// FEEL text of the number: no exponent, no trailing zeros (`string(1.10) = "1.1"`).
    pub fn to_text(&self) -> String {
        let n = at(DIGITS, || self.0.normalize().ok()).unwrap_or_else(|| self.0.clone());
        let text = n.format_f().unwrap_or_else(|_| n.to_sci_string());
        if text == "-0" { "0".into() } else { text }
    }
}

/// ln 2 at the working precision: 2·atanh(1/3).
fn ln2() -> Option<Decimal> {
    atanh_series(&at(WORK, || {
        Decimal::from_i128(1).div(&Decimal::from_i128(3)).ok()
    })?)
}

/// 2·atanh(z) = 2·(z + z³/3 + z⁵/5 + …) at the working precision.
fn atanh_series(z: &Decimal) -> Option<Decimal> {
    at(WORK, || {
        let z2 = z.mul(z).ok()?;
        let mut term = z.clone();
        let mut sum = z.clone();
        let mut k: i128 = 1;
        for _ in 0..2000 {
            term = term.mul(&z2).ok()?;
            k += 2;
            let add = term.div(&Decimal::from_i128(k)).ok()?;
            let next = sum.add(&add).ok()?;
            if next.py_eq(&sum).unwrap_or(false) {
                break;
            }
            sum = next;
        }
        sum.mul(&Decimal::from_i128(2)).ok()
    })
}

/// ln x (x > 0) at the working precision.
fn ln_work(x: &Decimal) -> Option<Decimal> {
    at(WORK, || {
        let one = Decimal::from_i128(1);
        let two = Decimal::from_i128(2);
        let mut m = x.clone();
        let mut k: i128 = 0;
        // bring m into [0.75, 1.5) by powers of two
        let low = Decimal::parse("0.75").ok()?;
        let high = Decimal::parse("1.5").ok()?;
        let mut guard = 0;
        while m.py_cmp(&high).ok()? != Ordering::Less && guard < 100_000 {
            m = m.div(&two).ok()?;
            k += 1;
            guard += 1;
        }
        while m.py_cmp(&low).ok()? == Ordering::Less && guard < 100_000 {
            m = m.mul(&two).ok()?;
            k -= 1;
            guard += 1;
        }
        let z = m.sub(&one).ok()?.div(&m.add(&one).ok()?).ok()?;
        let lm = atanh_series(&z)?;
        let l2 = ln2()?;
        lm.add(&l2.mul(&Decimal::from_i128(k)).ok()?).ok()
    })
}

/// e^x at the working precision.
fn exp_work(x: &Decimal) -> Option<Decimal> {
    at(WORK, || {
        let l2 = ln2()?;
        // x = n·ln 2 + r, |r| <= ln 2 / 2
        let n = x
            .div(&l2)
            .ok()?
            .to_integral_value(Rounding::HalfEven)
            .ok()?;
        let ni = n.to_i128().ok()?;
        if ni.abs() > 100_000 {
            return None;
        }
        let r = x.sub(&l2.mul(&n).ok()?).ok()?;
        let mut term = Decimal::from_i128(1);
        let mut sum = Decimal::from_i128(1);
        for k in 1..400 {
            term = term.mul(&r).ok()?.div(&Decimal::from_i128(k)).ok()?;
            let next = sum.add(&term).ok()?;
            if next.py_eq(&sum).unwrap_or(false) {
                break;
            }
            sum = next;
        }
        let mut p = Decimal::from_i128(1);
        let mut b = Decimal::from_i128(2);
        let mut e = ni.unsigned_abs();
        while e > 0 {
            if e & 1 == 1 {
                p = p.mul(&b).ok()?;
            }
            e >>= 1;
            if e > 0 {
                b = b.mul(&b).ok()?;
            }
        }
        if ni < 0 {
            sum.div(&p).ok()
        } else {
            sum.mul(&p).ok()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Num {
        Num::parse(s).unwrap_or_else(Num::zero)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(
            n("0.1").add(&n("0.2")).map(|x| x.to_text()),
            Some("0.3".into())
        );
        assert_eq!(
            n("1").div(&n("3")).map(|x| x.to_text()),
            Some("0.3333333333333333333333333333333333".into())
        );
        assert_eq!(n("1.23e4").to_text(), "12300");
        assert_eq!(
            n("10").modulo(&n("-3")).map(|x| x.to_text()),
            Some("-2".into())
        );
        assert_eq!(
            n("-10").modulo(&n("3")).map(|x| x.to_text()),
            Some("2".into())
        );
        assert_eq!(
            n("2").pow(&n("10")).map(|x| x.to_text()),
            Some("1024".into())
        );
        assert_eq!(n("16").sqrt().map(|x| x.to_text()), Some("4".into()));
        assert_eq!(
            n("2").sqrt().map(|x| x.to_text()),
            Some("1.414213562373095048801688724209698".into())
        );
        assert_eq!(
            n("10").ln().map(|x| x.to_text()),
            Some("2.302585092994045684017991454684364".into())
        );
        assert_eq!(
            n("1").exp().map(|x| x.to_text()),
            Some("2.718281828459045235360287471352662".into())
        );
        assert_eq!(
            n("5.5").round(0, Rounding::HalfEven).map(|x| x.to_text()),
            Some("6".into())
        );
        assert_eq!(
            n("-5.5").round(0, Rounding::HalfDown).map(|x| x.to_text()),
            Some("-5".into())
        );
        assert_eq!(
            n("5.1").round(0, Rounding::Up).map(|x| x.to_text()),
            Some("6".into())
        );
    }
}
