//! Regular expressions of the FEEL functions `matches`, `replace` and `split` (XML Schema
//! and XPath 2.0 syntax, flags `s`, `m`, `i`, `x`, `q`): a small backtracking matcher.

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<ClassItem>, bool),
    Start,
    End,
    Group(Box<Node>, Option<usize>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat(Box<Node>, usize, Option<usize>, bool),
    Back(usize),
}

#[derive(Debug, Clone)]
enum ClassItem {
    Range(char, char),
    Digit(bool),
    Space(bool),
    Word(bool),
    Category(String, bool),
    /// A subtraction `[a-z-[aeiou]]`: the base items, the subtracted items and whether
    /// the subtracted class is negated.
    Subtract(Vec<ClassItem>, Vec<ClassItem>, bool),
}

/// A match: its start, its end and the spans of the groups.
type Found = (usize, usize, Vec<Option<(usize, usize)>>);

/// A compiled regular expression.
#[derive(Debug, Clone)]
pub struct Regex {
    node: Node,
    groups: usize,
    icase: bool,
    dotall: bool,
    multiline: bool,
}

struct Parser {
    c: Vec<char>,
    i: usize,
    groups: usize,
    extended: bool,
}

fn category(name: &str, c: char) -> Option<bool> {
    let general = |n: &str| -> bool {
        match n {
            "L" => c.is_alphabetic(),
            "Lu" => c.is_uppercase(),
            "Ll" => c.is_lowercase(),
            "N" => c.is_numeric(),
            "Nd" => {
                c.is_ascii_digit() || (c.is_numeric() && c.to_digit(36).is_some_and(|d| d < 10))
            }
            "P" => {
                c.is_ascii_punctuation()
                    || (!c.is_alphanumeric()
                        && !c.is_whitespace()
                        && !c.is_control()
                        && !c.is_ascii())
            }
            "Z" | "Zs" => c.is_whitespace(),
            "C" | "Cc" => c.is_control(),
            "S" => matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~'),
            _ => false,
        }
    };
    if let Some(block) = name.strip_prefix("Is") {
        return Some(match block {
            "BasicLatin" => (c as u32) < 0x80,
            "Latin-1Supplement" => (0x80..0x100).contains(&(c as u32)),
            "Greek" | "GreekandCoptic" => (0x370..0x400).contains(&(c as u32)),
            "Cyrillic" => (0x400..0x500).contains(&(c as u32)),
            _ => return None,
        });
    }
    if ["L", "Lu", "Ll", "N", "Nd", "P", "Z", "Zs", "C", "Cc", "S"].contains(&name) {
        return Some(general(name));
    }
    None
}

impl ClassItem {
    fn matches(&self, c: char, icase: bool) -> bool {
        match self {
            ClassItem::Range(a, b) => {
                if (*a..=*b).contains(&c) {
                    return true;
                }
                if icase {
                    let lc = c.to_lowercase().next().unwrap_or(c);
                    let uc = c.to_uppercase().next().unwrap_or(c);
                    return (*a..=*b).contains(&lc) || (*a..=*b).contains(&uc);
                }
                false
            }
            ClassItem::Digit(neg) => c.is_ascii_digit() != *neg,
            ClassItem::Space(neg) => matches!(c, ' ' | '\t' | '\n' | '\r') != *neg,
            ClassItem::Word(neg) => {
                let w = c.is_alphanumeric() || c == '_' || (!c.is_ascii() && !c.is_whitespace());
                w != *neg
            }
            ClassItem::Category(n, neg) => category(n, c).unwrap_or(false) != *neg,
            ClassItem::Subtract(base, sub, sneg) => {
                base.iter().any(|x| x.matches(c, icase))
                    && sub.iter().any(|x| x.matches(c, icase)) == *sneg
            }
        }
    }
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn skip_space(&mut self) {
        if self.extended {
            while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
                self.i += 1;
            }
        }
    }
    fn alt(&mut self) -> Option<Node> {
        let mut branches = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.i += 1;
            branches.push(self.concat()?);
        }
        Some(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Alt(branches)
        })
    }
    fn concat(&mut self) -> Option<Node> {
        let mut items = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {}
            }
            let atom = self.atom()?;
            self.skip_space();
            let atom = self.quantifier(atom)?;
            items.push(atom);
        }
        Some(Node::Concat(items))
    }
    fn quantifier(&mut self, atom: Node) -> Option<Node> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, None)
            }
            Some('+') => {
                self.i += 1;
                (1, None)
            }
            Some('?') => {
                self.i += 1;
                (0, Some(1))
            }
            Some('{') => {
                let start = self.i;
                self.i += 1;
                let mut a = String::new();
                while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                    a.push(d);
                    self.i += 1;
                }
                let min: usize = a.parse().ok()?;
                let max = if self.peek() == Some(',') {
                    self.i += 1;
                    let mut b = String::new();
                    while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                        b.push(d);
                        self.i += 1;
                    }
                    if b.is_empty() {
                        None
                    } else {
                        Some(b.parse().ok()?)
                    }
                } else {
                    Some(min)
                };
                if self.peek() != Some('}') {
                    self.i = start;
                    return None;
                }
                self.i += 1;
                if max.is_some_and(|m| m < min) {
                    return None;
                }
                (min, max)
            }
            _ => return Some(atom),
        };
        let lazy = if self.peek() == Some('?') {
            self.i += 1;
            true
        } else {
            false
        };
        if matches!(self.peek(), Some('*' | '+' | '{')) {
            return None;
        }
        Some(Node::Repeat(Box::new(atom), min, max, lazy))
    }
    fn escape(&mut self) -> Option<ClassItem> {
        let e = self.peek()?;
        self.i += 1;
        Some(match e {
            'd' => ClassItem::Digit(false),
            'D' => ClassItem::Digit(true),
            's' => ClassItem::Space(false),
            'S' => ClassItem::Space(true),
            'w' => ClassItem::Word(false),
            'W' => ClassItem::Word(true),
            'i' => ClassItem::Category("L".into(), false),
            'c' => ClassItem::Word(false),
            'p' | 'P' => {
                if self.peek() != Some('{') {
                    return None;
                }
                self.i += 1;
                let mut n = String::new();
                while let Some(x) = self.peek() {
                    self.i += 1;
                    if x == '}' {
                        break;
                    }
                    n.push(x);
                }
                category(&n, 'a')?;
                ClassItem::Category(n, e == 'P')
            }
            'n' => ClassItem::Range('\n', '\n'),
            'r' => ClassItem::Range('\r', '\r'),
            't' => ClassItem::Range('\t', '\t'),
            c if "\\|.-^?*+{}()[]$".contains(c) => ClassItem::Range(c, c),
            _ => return None,
        })
    }
    fn atom(&mut self) -> Option<Node> {
        let c = self.peek()?;
        self.i += 1;
        Some(match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => {
                let capture =
                    if self.c.get(self.i) == Some(&'?') && self.c.get(self.i + 1) == Some(&':') {
                        self.i += 2;
                        None
                    } else {
                        self.groups += 1;
                        Some(self.groups)
                    };
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return None;
                }
                self.i += 1;
                Node::Group(Box::new(inner), capture)
            }
            '[' => self.class()?,
            '\\' => {
                if let Some(d) = self.peek().filter(|d| d.is_ascii_digit() && *d != '0') {
                    self.i += 1;
                    let n = d.to_digit(10)? as usize;
                    if n > self.groups {
                        return None;
                    }
                    return Some(Node::Back(n));
                }
                match self.escape()? {
                    ClassItem::Range(a, b) if a == b => Node::Char(a),
                    item => Node::Class(vec![item], false),
                }
            }
            '*' | '+' | '?' | '{' | ')' | ']' => return None,
            c => Node::Char(c),
        })
    }
    fn class(&mut self) -> Option<Node> {
        let (items, neg) = self.class_items()?;
        Some(Node::Class(items, neg))
    }
    fn class_items(&mut self) -> Option<(Vec<ClassItem>, bool)> {
        let neg = if self.peek() == Some('^') {
            self.i += 1;
            true
        } else {
            false
        };
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek()?;
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            if c == '-' && self.c.get(self.i + 1) == Some(&'[') {
                self.i += 2;
                let (sub, sneg) = self.class_items()?;
                if self.peek() != Some(']') {
                    return None;
                }
                self.i += 1;
                let base = std::mem::take(&mut items);
                return Some((vec![ClassItem::Subtract(base, sub, sneg)], neg));
            }
            self.i += 1;
            let lo = if c == '\\' {
                match self.escape()? {
                    ClassItem::Range(a, _) => a,
                    other => {
                        items.push(other);
                        continue;
                    }
                }
            } else {
                c
            };
            if self.peek() == Some('-')
                && self
                    .c
                    .get(self.i + 1)
                    .is_some_and(|x| *x != ']' && *x != '[')
            {
                self.i += 1;
                let mut hi = self.peek()?;
                self.i += 1;
                if hi == '\\' {
                    match self.escape()? {
                        ClassItem::Range(a, _) => hi = a,
                        _ => return None,
                    }
                }
                if hi < lo {
                    return None;
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
        Some((items, neg))
    }
}

struct Matcher<'a> {
    re: &'a Regex,
    s: &'a [char],
    caps: Vec<Option<(usize, usize)>>,
    steps: usize,
}

const STEP_LIMIT: usize = 2_000_000;

impl Matcher<'_> {
    fn eq(&self, a: char, b: char) -> bool {
        a == b
            || (self.re.icase
                && (a.to_lowercase().eq(b.to_lowercase()) || a.to_uppercase().eq(b.to_uppercase())))
    }

    /// Match `node` at `i`, then the continuation `k`.
    fn m(&mut self, node: &Node, i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            return false;
        }
        match node {
            Node::Char(c) => i < self.s.len() && self.eq(self.s[i], *c) && k(self, i + 1),
            Node::Any => {
                i < self.s.len()
                    && (self.re.dotall || !matches!(self.s[i], '\n' | '\r'))
                    && k(self, i + 1)
            }
            Node::Class(items, neg) => {
                i < self.s.len()
                    && items.iter().any(|x| x.matches(self.s[i], self.re.icase)) != *neg
                    && k(self, i + 1)
            }
            Node::Start => (i == 0 || (self.re.multiline && self.s[i - 1] == '\n')) && k(self, i),
            Node::End => {
                (i == self.s.len() || (self.re.multiline && self.s[i] == '\n')) && k(self, i)
            }
            Node::Back(n) => {
                let Some((a, b)) = self.caps.get(*n).copied().flatten() else {
                    return k(self, i);
                };
                let len = b - a;
                if i + len > self.s.len() {
                    return false;
                }
                for j in 0..len {
                    if !self.eq(self.s[a + j], self.s[i + j]) {
                        return false;
                    }
                }
                k(self, i + len)
            }
            Node::Group(inner, cap) => {
                let cap = *cap;
                let old = cap.and_then(|c| self.caps[c]);
                let ok = self.m(inner, i, &mut |me: &mut Self, j| {
                    if let Some(c) = cap {
                        let prev = me.caps[c];
                        me.caps[c] = Some((i, j));
                        if k(me, j) {
                            return true;
                        }
                        me.caps[c] = prev;
                        false
                    } else {
                        k(me, j)
                    }
                });
                if !ok && let Some(c) = cap {
                    self.caps[c] = old;
                }
                ok
            }
            Node::Concat(items) => self.seq(items, 0, i, k),
            Node::Alt(branches) => {
                for b in branches {
                    if self.m(b, i, k) {
                        return true;
                    }
                }
                false
            }
            Node::Repeat(inner, min, max, lazy) => {
                self.rep(inner, *min, *max, *lazy, 0, i, usize::MAX, k)
            }
        }
    }

    fn seq(
        &mut self,
        items: &[Node],
        idx: usize,
        i: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> bool,
    ) -> bool {
        if idx == items.len() {
            return k(self, i);
        }
        let (first, _) = items.split_at(idx + 1);
        let node = &first[idx];
        self.m(node, i, &mut |me: &mut Self, j| {
            me.seq(items, idx + 1, j, k)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn rep(
        &mut self,
        inner: &Node,
        min: usize,
        max: Option<usize>,
        lazy: bool,
        count: usize,
        i: usize,
        last: usize,
        k: &mut dyn FnMut(&mut Self, usize) -> bool,
    ) -> bool {
        let can_more = max.is_none_or(|m| count < m) && i != last;
        if count >= min && lazy && k(self, i) {
            return true;
        }
        if can_more || count < min {
            let ok = self.m(inner, i, &mut |me: &mut Self, j| {
                if j == i && count >= min {
                    return false;
                }
                me.rep(inner, min, max, lazy, count + 1, j, i, k)
            });
            if ok {
                return true;
            }
        }
        count >= min && !lazy && k(self, i)
    }
}

impl Regex {
    /// Compile a pattern with XPath flags; `None` for a malformed pattern or flag.
    pub fn new(pattern: &str, flags: &str) -> Option<Regex> {
        if !flags.chars().all(|f| "smixq".contains(f)) {
            return None;
        }
        let quote = flags.contains('q');
        let mut c: Vec<char> = pattern.chars().collect();
        if flags.contains('x') && !quote {
            let mut out = Vec::with_capacity(c.len());
            let mut depth = 0usize;
            let mut k = 0;
            while k < c.len() {
                let ch = c[k];
                if ch == '\\' && k + 1 < c.len() && depth > 0 {
                    out.push(ch);
                    out.push(c[k + 1]);
                    k += 2;
                    continue;
                }
                match ch {
                    '[' => depth += 1,
                    ']' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                if depth > 0 || !matches!(ch, ' ' | '\t' | '\n' | '\r') {
                    out.push(ch);
                }
                k += 1;
            }
            c = out;
        }
        let (node, groups) = if quote {
            (Node::Concat(c.iter().map(|x| Node::Char(*x)).collect()), 0)
        } else {
            let mut p = Parser {
                c,
                i: 0,
                groups: 0,
                extended: false,
            };
            let node = p.alt()?;
            if p.i != p.c.len() {
                return None;
            }
            (node, p.groups)
        };
        Some(Regex {
            node,
            groups,
            icase: flags.contains('i'),
            dotall: flags.contains('s'),
            multiline: flags.contains('m'),
        })
    }

    /// The first match at or after `from`: (start, end, captures).
    fn find_at(&self, s: &[char], from: usize) -> Option<Found> {
        for start in from..=s.len() {
            let mut m = Matcher {
                re: self,
                s,
                caps: vec![None; self.groups + 1],
                steps: 0,
            };
            let mut end = None;
            let ok = m.m(&self.node, start, &mut |_, j| {
                end = Some(j);
                true
            });
            if m.steps > STEP_LIMIT {
                return None;
            }
            if ok && let Some(e) = end {
                return Some((start, e, m.caps));
            }
        }
        None
    }

    pub fn is_match(&self, text: &str) -> bool {
        let s: Vec<char> = text.chars().collect();
        self.find_at(&s, 0).is_some()
    }

    /// `replace`: every non-overlapping match replaced, `$N` naming a group.
    pub fn replace(&self, text: &str, replacement: &str) -> Option<String> {
        let s: Vec<char> = text.chars().collect();
        let r: Vec<char> = replacement.chars().collect();
        let mut out = String::new();
        let mut pos = 0;
        while let Some((a, b, caps)) = self.find_at(&s, pos) {
            if a == b && self.find_at(&[], 0).is_some() && s.is_empty() {
                return None;
            }
            out.extend(&s[pos..a]);
            let mut j = 0;
            while j < r.len() {
                match r[j] {
                    '\\' if j + 1 < r.len() && (r[j + 1] == '$' || r[j + 1] == '\\') => {
                        out.push(r[j + 1]);
                        j += 2;
                    }
                    '\\' => return None,
                    '$' => {
                        let mut k = j + 1;
                        let mut n = 0usize;
                        let mut any = false;
                        while k < r.len() && r[k].is_ascii_digit() {
                            let d = r[k].to_digit(10)? as usize;
                            if any && n * 10 + d > self.groups {
                                break;
                            }
                            n = n * 10 + d;
                            any = true;
                            k += 1;
                        }
                        if !any {
                            return None;
                        }
                        if n == 0 {
                            out.extend(&s[a..b]);
                        } else if let Some(Some((x, y))) = caps.get(n) {
                            out.extend(&s[*x..*y]);
                        }
                        j = k;
                    }
                    c => {
                        out.push(c);
                        j += 1;
                    }
                }
            }
            if b == a {
                if a < s.len() {
                    out.push(s[a]);
                }
                pos = a + 1;
                if pos > s.len() {
                    break;
                }
            } else {
                pos = b;
            }
        }
        if pos <= s.len() {
            out.extend(&s[pos.min(s.len())..]);
        }
        Some(out)
    }

    /// `split`: the parts between the matches.
    pub fn split(&self, text: &str) -> Option<Vec<String>> {
        let s: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut pos = 0;
        let mut from = 0;
        while let Some((a, b, _)) = self.find_at(&s, from) {
            if a == b {
                if a >= s.len() {
                    break;
                }
                from = a + 1;
                continue;
            }
            out.push(s[pos..a].iter().collect());
            pos = b;
            from = b;
        }
        out.push(s[pos..].iter().collect());
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        let m = |p: &str, f: &str, t: &str| Regex::new(p, f).map(|r| r.is_match(t));
        assert_eq!(m("^fo*b", "", "foobar"), Some(true));
        assert_eq!(m("^FO*B", "i", "foobar"), Some(true));
        assert_eq!(m("[0-9]{3}-[0-9]{2}", "", "tel 123-45"), Some(true));
        assert_eq!(m("^(ab)+$", "", "ababab"), Some(true));
        assert_eq!(m("^(ab)+$", "", "ababa"), Some(false));
        assert_eq!(m("(", "", "x"), None);
        let r = |p: &str, t: &str, x: &str| Regex::new(p, "").and_then(|r| r.replace(t, x));
        assert_eq!(r("(a)(b)", "abcab", "$2$1"), Some("bacba".into()));
        assert_eq!(r("[aeiou]", "banana", "_"), Some("b_n_n_".into()));
        assert_eq!(
            Regex::new(",\\s*", "").and_then(|r| r.split("a, b,c")),
            Some(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(m("[a-z-[aeiou]]+", "", "xyz"), Some(true));
        assert_eq!(m("^[a-z-[aeiou]]+$", "", "xaz"), Some(false));
    }
}
