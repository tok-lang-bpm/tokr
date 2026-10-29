//! Number and text formatting with the exact semantics of Python's `%`-formatting,
//! `round()` and `str.capitalize()` used by `bpmn.py`.

/// `"%.1f" % x`.
#[must_use]
pub fn f1(x: f64) -> String {
    format!("{x:.1}")
}

/// `"%.0f" % x`.
#[must_use]
pub fn f0(x: f64) -> String {
    format!("{x:.0}")
}

/// `str(int(round(x)))` (round half to even).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn round_str(x: f64) -> String {
    let r = x.round_ties_even() as i64;
    r.to_string()
}

/// `str.capitalize()`: first character upper case, the rest lower case.
#[must_use]
pub fn capitalize(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    if let Some(c) = chars.next() {
        out.extend(c.to_uppercase());
    }
    for c in chars {
        out.extend(c.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_rounding() {
        assert_eq!(f1(0.25), "0.2");
        assert_eq!(f1(0.35), "0.3");
        assert_eq!(f1(-0.04), "-0.0");
        assert_eq!(f0(152.5), "152");
        assert_eq!(f0(-0.5), "-0");
        assert_eq!(round_str(2.5), "2");
        assert_eq!(round_str(-0.5), "0");
        assert_eq!(round_str(3.5), "4");
    }

    #[test]
    fn capitalize_like_python() {
        assert_eq!(
            capitalize("bramka wykluczająca (XOR)"),
            "Bramka wykluczająca (xor)"
        );
        assert_eq!(capitalize("zadanie usługowe"), "Zadanie usługowe");
    }
}
