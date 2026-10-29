//! Step numbers in element names (course convention, `--kroki`/`--opis`).
//!
//! Equivalent of the regular expression used by `bpmn.py`:
//! `^\s*(\d+(?:\.\d+)?[a-z]?)(?:\s*[–-]\s*(\d+[a-z]?))?(?:[.):]|\s)`,
//! including its backtracking order (digits are ASCII, see PARITY.md).

use std::cmp::Ordering;
use std::collections::HashSet;

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

fn is_lower(c: char) -> bool {
    c.is_ascii_lowercase()
}

/// Ends of a run of digits starting at `i`, longest first (greedy with backtracking).
fn digit_ends(ch: &[char], i: usize) -> Vec<usize> {
    let mut j = i;
    while j < ch.len() && is_digit(ch[j]) {
        j += 1;
    }
    ((i + 1)..=j).rev().collect()
}

/// Optional lower-case letter at `p`: positions to try, greedy first.
fn letter_opts(ch: &[char], p: usize) -> Vec<usize> {
    if p < ch.len() && is_lower(ch[p]) {
        vec![p + 1, p]
    } else {
        vec![p]
    }
}

fn final_ok(ch: &[char], p: usize) -> bool {
    p < ch.len() && (matches!(ch[p], '.' | ')' | ':') || ch[p].is_whitespace())
}

/// Returns (group 1, group 2) of the first match, or `None`.
pub fn step_match(name: &str) -> Option<(String, Option<String>)> {
    let ch: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < ch.len() && ch[i].is_whitespace() {
        i += 1;
    }
    for p in digit_ends(&ch, i) {
        // optional fraction, greedy first
        let mut after_frac = Vec::new();
        if p < ch.len() && ch[p] == '.' {
            for q in digit_ends(&ch, p + 1) {
                after_frac.push(q);
            }
        }
        after_frac.push(p);
        for x in after_frac {
            for y in letter_opts(&ch, x) {
                let g1: String = ch[i..y].iter().collect();
                // optional range part, tried first
                let mut z = y;
                while z < ch.len() && ch[z].is_whitespace() {
                    z += 1;
                }
                if z < ch.len() && (ch[z] == '\u{2013}' || ch[z] == '-') {
                    let mut w = z + 1;
                    while w < ch.len() && ch[w].is_whitespace() {
                        w += 1;
                    }
                    for v in digit_ends(&ch, w) {
                        for u in letter_opts(&ch, v) {
                            if final_ok(&ch, u) {
                                return Some((g1, Some(ch[w..u].iter().collect())));
                            }
                        }
                    }
                }
                if final_ok(&ch, y) {
                    return Some((g1, None));
                }
            }
        }
    }
    None
}

/// `_step_numbers(name)`.
#[must_use]
pub fn step_numbers(name: &str) -> HashSet<String> {
    let Some((a, b)) = step_match(name) else {
        return HashSet::new();
    };
    if let Some(b) = b
        && let (Ok(x), Ok(y)) = (a.parse::<u64>(), b.parse::<u64>())
        && a.chars().all(is_digit)
        && b.chars().all(is_digit)
        && x < y
        && y <= x + 50
    {
        return (x..=y).map(|k| k.to_string()).collect();
    }
    std::iter::once(a).collect()
}

/// Part of a sort key of a step number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPart {
    Num(u64),
    Str(String),
}

impl PartialOrd for KeyPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for KeyPart {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (KeyPart::Num(a), KeyPart::Num(b)) => a.cmp(b),
            (KeyPart::Str(a), KeyPart::Str(b)) => a.cmp(b),
            (KeyPart::Num(_), KeyPart::Str(_)) => Ordering::Less,
            (KeyPart::Str(_), KeyPart::Num(_)) => Ordering::Greater,
        }
    }
}

/// `_step_key(k)`: digit runs as numbers, letter runs as text.
#[must_use]
pub fn step_key(k: &str) -> Vec<KeyPart> {
    let mut out = Vec::new();
    let ch: Vec<char> = k.chars().collect();
    let mut i = 0;
    while i < ch.len() {
        if is_digit(ch[i]) {
            let s = i;
            while i < ch.len() && is_digit(ch[i]) {
                i += 1;
            }
            let txt: String = ch[s..i].iter().collect();
            out.push(KeyPart::Num(txt.parse().unwrap_or(u64::MAX)));
        } else if is_lower(ch[i]) {
            let s = i;
            while i < ch.len() && is_lower(ch[i]) {
                i += 1;
            }
            out.push(KeyPart::Str(ch[s..i].iter().collect()));
        } else {
            i += 1;
        }
    }
    out
}

/// `read_opis`: step numbers from the first column of Markdown tables.
#[must_use]
pub fn read_opis(text: &str) -> Vec<String> {
    let mut steps: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let inner = line.trim_matches('|');
        let first = inner.split('|').next().unwrap_or("").trim();
        if is_step_cell(first) {
            let v = first.to_lowercase();
            if !steps.contains(&v) {
                steps.push(v);
            }
        }
    }
    steps
}

fn is_step_cell(c: &str) -> bool {
    let ch: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < ch.len() && is_digit(ch[i]) {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    if i < ch.len() && ch[i] == '.' {
        let s = i + 1;
        let mut j = s;
        while j < ch.len() && is_digit(ch[j]) {
            j += 1;
        }
        if j > s {
            i = j;
        }
    }
    if i < ch.len() && is_lower(ch[i]) {
        i += 1;
    }
    i == ch.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_python() {
        assert_eq!(step_match("12. Zapytanie").map(|x| x.0), Some("12".into()));
        assert_eq!(step_match("5.1. Badanie").map(|x| x.0), Some("5.1".into()));
        assert_eq!(
            step_match("5a. Interwencja").map(|x| x.0),
            Some("5a".into())
        );
        assert_eq!(step_match("1.5Foo").map(|x| x.0), Some("1".into()));
        assert_eq!(step_match("12"), None);
        assert_eq!(
            step_match("31–33 Koniec"),
            Some(("31".into(), Some("33".into())))
        );
        assert_eq!(step_numbers("31–33 Koniec").len(), 3);
    }
}
