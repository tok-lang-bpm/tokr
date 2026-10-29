//! A small exact decimal number, enough for FEEL literals in decision table analysis
//! (Python uses `decimal.Decimal`).

use std::cmp::Ordering;

/// `mantissa / 10^scale`
#[derive(Debug, Clone, Copy)]
pub struct Dec {
    mantissa: i128,
    scale: u32,
}

impl Dec {
    pub fn int(v: i128) -> Self {
        Dec {
            mantissa: v,
            scale: 0,
        }
    }

    /// Parse digits with an optional fraction (`12`, `1.50`, `.5`); `None` when out of range.
    pub fn parse(text: &str) -> Option<Self> {
        let (neg, body) = match text.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, text),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let digits = format!("{int}{frac}");
        let digits = if digits.is_empty() {
            "0".to_string()
        } else {
            digits
        };
        let m: i128 = digits.parse().ok()?;
        Some(Dec {
            mantissa: if neg { -m } else { m },
            scale: frac.len() as u32,
        })
    }

    fn rescale(self, scale: u32) -> Option<i128> {
        let f = 10i128.checked_pow(scale - self.scale)?;
        self.mantissa.checked_mul(f)
    }

    fn align(a: Dec, b: Dec) -> Option<(i128, i128, u32)> {
        let s = a.scale.max(b.scale);
        Some((a.rescale(s)?, b.rescale(s)?, s))
    }

    pub fn negate(self) -> Self {
        Dec {
            mantissa: -self.mantissa,
            scale: self.scale,
        }
    }

    pub fn plus(self, other: Dec) -> Option<Dec> {
        let (a, b, s) = Self::align(self, other)?;
        Some(Dec {
            mantissa: a.checked_add(b)?,
            scale: s,
        })
    }

    pub fn minus(self, other: Dec) -> Option<Dec> {
        self.plus(other.negate())
    }

    pub fn half(self) -> Option<Dec> {
        if self.mantissa % 2 == 0 {
            Some(Dec {
                mantissa: self.mantissa / 2,
                scale: self.scale,
            })
        } else {
            Some(Dec {
                mantissa: self.mantissa.checked_mul(5)?,
                scale: self.scale + 1,
            })
        }
    }

    pub fn cmp_dec(&self, other: &Dec) -> Ordering {
        match Self::align(*self, *other) {
            Some((a, b, _)) => a.cmp(&b),
            None => self
                .to_f64()
                .partial_cmp(&other.to_f64())
                .unwrap_or(Ordering::Equal),
        }
    }

    fn to_f64(self) -> f64 {
        self.mantissa as f64 / 10f64.powi(self.scale as i32)
    }

    /// `feel.number_to_string`: integral values without decimals, others normalized.
    pub fn to_text(self) -> String {
        let neg = self.mantissa < 0;
        let digits = self.mantissa.unsigned_abs().to_string();
        let s = self.scale as usize;
        let (int, frac) = if digits.len() > s {
            (
                digits[..digits.len() - s].to_string(),
                digits[digits.len() - s..].to_string(),
            )
        } else {
            (
                "0".to_string(),
                format!("{}{}", "0".repeat(s - digits.len()), digits),
            )
        };
        let frac = frac.trim_end_matches('0');
        let sign = if neg && (int != "0" || !frac.is_empty()) {
            "-"
        } else {
            ""
        };
        if frac.is_empty() {
            format!("{sign}{int}")
        } else {
            format!("{sign}{int}.{frac}")
        }
    }
}

impl PartialEq for Dec {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_dec(other) == Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals() {
        let a = Dec::parse("150").expect("dec");
        let b = Dec::parse("150.01").expect("dec");
        assert_eq!(
            a.plus(b).and_then(Dec::half).map(Dec::to_text).as_deref(),
            Some("150.005")
        );
        assert_eq!(Dec::parse("1.50").map(Dec::to_text).as_deref(), Some("1.5"));
        assert_eq!(Dec::parse(".5").map(Dec::to_text).as_deref(), Some("0.5"));
        assert_eq!(
            Dec::parse("18")
                .and_then(|d| d.minus(Dec::int(1)))
                .map(Dec::to_text)
                .as_deref(),
            Some("17")
        );
        assert!(a.cmp_dec(&b) == Ordering::Less);
    }
}
