//! Helpers that reproduce Python string and number behaviour used by the reference
//! implementation (whitespace rules, `repr` of characters and floats, `%g`).

/// Python `str.isspace()` for one character.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python `str.rstrip()`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// Python `str.split()` without arguments.
pub fn split_ws(s: &str) -> Vec<&str> {
    s.split(is_space).filter(|p| !p.is_empty()).collect()
}

/// Python `str.isprintable()` for one character (approximation sufficient for repr).
fn printable(c: char) -> bool {
    !(c.is_control()
        || matches!(c, '\u{ad}' | '\u{2028}' | '\u{2029}' | '\u{feff}')
        || (c.is_whitespace() && c != ' '))
}

/// A Python exception raised by the reference implementation, kept as data: `kind` is the
/// qualified class name as a traceback prints it (`decimal.InvalidOperation`, `ValueError`,
/// `tok.dmn.DecisionError`), `message` is `str(exception)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyError {
    pub kind: String,
    pub message: String,
}

impl PyError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        PyError {
            kind: kind.into(),
            message: message.into(),
        }
    }

    /// Unqualified class name (`InvalidOperation` for `decimal.InvalidOperation`).
    pub fn class_name(&self) -> &str {
        self.kind.rsplit('.').next().unwrap_or(&self.kind)
    }
}

impl std::fmt::Display for PyError {
    /// The last line of a Python traceback: `kind: message`, or `kind` alone.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.kind)
        } else {
            write!(f, "{}: {}", self.kind, self.message)
        }
    }
}

impl std::error::Error for PyError {}

/// Code points of the digit zero of every Unicode decimal-digit run (category Nd,
/// Unicode 14.0 as in Python 3.11); each run holds the digits 0 to 9 in order.
const ND_ZEROS: [u32; 66] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66,
    0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
    0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650,
    0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x16A60, 0x16AC0, 0x16B50,
    0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E950, 0x1FBF0,
];

/// Python `unicodedata.decimal(c, None)`: the value of a decimal digit of any script.
pub fn decimal_value(c: char) -> Option<u32> {
    let n = c as u32;
    let k = ND_ZEROS.partition_point(|z| *z <= n);
    if k == 0 {
        return None;
    }
    let off = n - ND_ZEROS[k - 1];
    (off < 10).then_some(off)
}

/// Python `int(text)` in base 10: surrounding whitespace, a sign, digits of any script and
/// single underscores between digits. Errors as `ValueError`.
pub fn parse_int(text: &str) -> Result<i128, PyError> {
    let err = || {
        PyError::new(
            "ValueError",
            format!("invalid literal for int() with base 10: {}", repr_str(text)),
        )
    };
    let t = strip(text);
    let (neg, body) = match t.chars().next() {
        Some('-') => (true, &t[1..]),
        Some('+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut value: i128 = 0;
    let mut prev_digit = false;
    let mut any = false;
    let chars: Vec<char> = body.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == '_' {
            let next_digit = chars.get(i + 1).and_then(|n| decimal_value(*n)).is_some();
            if !prev_digit || !next_digit {
                return Err(err());
            }
            prev_digit = false;
            continue;
        }
        let d = decimal_value(*c).ok_or_else(err)?;
        value = value.saturating_mul(10).saturating_add(d as i128);
        prev_digit = true;
        any = true;
    }
    if !any {
        return Err(err());
    }
    Ok(if neg { -value } else { value })
}

/// Python `repr()` of a string: single quotes unless the text contains `'` and no `"`.
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if printable(c) => out.push(c),
            c => {
                let n = c as u32;
                if n < 0x100 {
                    out.push_str(&format!("\\x{n:02x}"));
                } else if n < 0x10000 {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    out.push_str(&format!("\\U{n:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Python `repr()` of a one-character string, e.g. `'x'`, `"'"`, `'\n'`.
pub fn repr_char(c: char) -> String {
    let body = match c {
        '\\' => "\\\\".to_string(),
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        '\'' => return "\"'\"".to_string(),
        c if printable(c) => c.to_string(),
        c => {
            let n = c as u32;
            if n < 0x100 {
                format!("\\x{n:02x}")
            } else if n < 0x10000 {
                format!("\\u{n:04x}")
            } else {
                format!("\\U{n:08x}")
            }
        }
    };
    format!("'{body}'")
}

/// Mantissa digits and decimal exponent of the shortest round-trip representation.
fn shortest(x: f64) -> (bool, String, i32) {
    let s = format!("{:e}", x.abs());
    let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let exp: i32 = exp.parse().unwrap_or(0);
    (x.is_sign_negative(), digits, exp)
}

/// Python `repr(float)`.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let (neg, digits, exp) = shortest(x);
    let sign = if neg { "-" } else { "" };
    let n = digits.len() as i32;
    if (-4..16).contains(&exp) {
        let body = if exp >= 0 {
            if exp + 1 >= n {
                format!("{}{}.0", digits, "0".repeat((exp + 1 - n) as usize))
            } else {
                let (a, b) = digits.split_at((exp + 1) as usize);
                format!("{a}.{b}")
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{body}")
    } else {
        let mant = if n > 1 {
            format!("{}.{}", &digits[..1], &digits[1..])
        } else {
            digits.clone()
        };
        let es = if exp < 0 { "-" } else { "+" };
        format!("{sign}{mant}e{es}{:02}", exp.abs())
    }
}

/// Python `"%g" % x` (precision 6).
pub fn fmt_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let s = format!("{:.5e}", x);
    let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..6).contains(&exp) {
        let prec = (5 - exp).max(0) as usize;
        let f = format!("{:.*}", prec, x);
        if f.contains('.') {
            f.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            f
        }
    } else {
        let m = if mant.contains('.') {
            mant.trim_end_matches('0').trim_end_matches('.')
        } else {
            mant
        };
        let es = if exp < 0 { "-" } else { "+" };
        format!("{m}e{es}{:02}", exp.abs())
    }
}

/// Python `float(text)` (accepts surrounding whitespace, underscores between digits, inf, nan).
pub fn parse_float(text: &str) -> Option<f64> {
    let t = strip(text);
    if t.is_empty() {
        return None;
    }
    let cleaned: String = if t.contains('_') {
        let chars: Vec<char> = t.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == '_' {
                let ok = i > 0
                    && i + 1 < chars.len()
                    && chars[i - 1].is_ascii_digit()
                    && chars[i + 1].is_ascii_digit();
                if !ok {
                    return None;
                }
            }
        }
        t.replace('_', "")
    } else {
        t.to_string()
    };
    let low = cleaned.to_ascii_lowercase();
    let body = low.trim_start_matches(['+', '-']);
    if matches!(body, "inf" | "infinity" | "nan") {
        return cleaned.parse::<f64>().ok().or_else(|| {
            let neg = low.starts_with('-');
            let v = if body == "nan" {
                f64::NAN
            } else {
                f64::INFINITY
            };
            Some(if neg { -v } else { v })
        });
    }
    if !body
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | '+' | '-'))
    {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

/// Python `round(x, 6)` followed by `repr`.
pub fn round6_repr(x: f64) -> String {
    let s = format!("{x:.6}");
    float_repr(s.parse::<f64>().unwrap_or(x))
}

/// ASCII folding of Python `unicodedata.normalize("NFKD", t).encode("ascii", "ignore")`
/// for Latin letters (Latin-1 Supplement, Latin Extended-A) and common compatibility forms.
pub fn ascii_fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        if let Some(s) = fold_char(c) {
            out.push_str(s);
        }
    }
    out
}

fn fold_char(c: char) -> Option<&'static str> {
    let n = c as u32;
    let s = match n {
        0xC0..=0xC5 => "A",
        0xC7 => "C",
        0xC8..=0xCB => "E",
        0xCC..=0xCF => "I",
        0xD1 => "N",
        0xD2..=0xD6 => "O",
        0xD9..=0xDC => "U",
        0xDD => "Y",
        0xE0..=0xE5 => "a",
        0xE7 => "c",
        0xE8..=0xEB => "e",
        0xEC..=0xEF => "i",
        0xF1 => "n",
        0xF2..=0xF6 => "o",
        0xF9..=0xFC => "u",
        0xFD | 0xFF => "y",
        0xAA => "a",
        0xBA => "o",
        0xB2 => "2",
        0xB3 => "3",
        0xB9 => "1",
        0xBC => "14",
        0xBD => "12",
        0xBE => "34",
        0x100 | 0x102 | 0x104 => "A",
        0x101 | 0x103 | 0x105 => "a",
        0x106 | 0x108 | 0x10A | 0x10C => "C",
        0x107 | 0x109 | 0x10B | 0x10D => "c",
        0x10E => "D",
        0x10F => "d",
        0x112 | 0x114 | 0x116 | 0x118 | 0x11A => "E",
        0x113 | 0x115 | 0x117 | 0x119 | 0x11B => "e",
        0x11C | 0x11E | 0x120 | 0x122 => "G",
        0x11D | 0x11F | 0x121 | 0x123 => "g",
        0x124 => "H",
        0x125 => "h",
        0x128 | 0x12A | 0x12C | 0x12E | 0x130 => "I",
        0x129 | 0x12B | 0x12D | 0x12F => "i",
        0x132 => "IJ",
        0x133 => "ij",
        0x134 => "J",
        0x135 => "j",
        0x136 => "K",
        0x137 => "k",
        0x139 | 0x13B | 0x13D | 0x13F => "L",
        0x13A | 0x13C | 0x13E | 0x140 => "l",
        0x143 | 0x145 | 0x147 => "N",
        0x144 | 0x146 | 0x148 | 0x149 => "n",
        0x14C | 0x14E | 0x150 => "O",
        0x14D | 0x14F | 0x151 => "o",
        0x154 | 0x156 | 0x158 => "R",
        0x155 | 0x157 | 0x159 => "r",
        0x15A | 0x15C | 0x15E | 0x160 => "S",
        0x15B | 0x15D | 0x15F | 0x161 | 0x17F => "s",
        0x162 | 0x164 => "T",
        0x163 | 0x165 => "t",
        0x168 | 0x16A | 0x16C | 0x16E | 0x170 | 0x172 => "U",
        0x169 | 0x16B | 0x16D | 0x16F | 0x171 | 0x173 => "u",
        0x174 => "W",
        0x175 => "w",
        0x176 | 0x178 => "Y",
        0x177 => "y",
        0x179 | 0x17B | 0x17D => "Z",
        0x17A | 0x17C | 0x17E => "z",
        0xFB00 => "ff",
        0xFB01 => "fi",
        0xFB02 => "fl",
        0xFB03 => "ffi",
        0xFB04 => "ffl",
        0xFB05 | 0xFB06 => "st",
        0x2070 => "0",
        0x2074 => "4",
        0x2075 => "5",
        0x2076 => "6",
        0x2077 => "7",
        0x2078 => "8",
        0x2079 => "9",
        0x2080..=0x2089 => return digit((n - 0x2080) as usize),
        0xFF10..=0xFF19 => return digit((n - 0xFF10) as usize),
        0xFF21..=0xFF3A => return letter_upper((n - 0xFF21) as usize),
        0xFF41..=0xFF5A => return letter_lower((n - 0xFF41) as usize),
        0x3000 | 0x2002..=0x200A | 0xA0 => " ",
        _ => return None,
    };
    Some(s)
}

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const UPPER: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];
const LOWER: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];

fn digit(i: usize) -> Option<&'static str> {
    DIGITS.get(i).copied()
}
fn letter_upper(i: usize) -> Option<&'static str> {
    UPPER.get(i).copied()
}
fn letter_lower(i: usize) -> Option<&'static str> {
    LOWER.get(i).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats() {
        assert_eq!(float_repr(1.0), "1.0");
        assert_eq!(float_repr(0.9), "0.9");
        assert_eq!(float_repr(1e-7), "1e-07");
        assert_eq!(float_repr(1.5e16), "1.5e+16");
        assert_eq!(float_repr(123456.5), "123456.5");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(fmt_g(1.1), "1.1");
        assert_eq!(fmt_g(0.9000001), "0.9");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(2.0), "2");
        assert_eq!(round6_repr(1.0999999999), "1.1");
    }

    #[test]
    fn reprs() {
        assert_eq!(repr_char('x'), "'x'");
        assert_eq!(repr_char('\''), "\"'\"");
        assert_eq!(repr_char('\u{1}'), "'\\x01'");
        assert_eq!(ascii_fold("Zamrożenie budżetu ąę"), "Zamrozenie budzetu ae");
    }
}
