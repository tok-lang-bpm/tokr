//! Tokenizer and parser of FEEL at DMN conformance level 3 (DMN 1.5 §10.3.1), used by
//! documents of language 0.7. Names may contain spaces and the additional name symbols
//! `. / - ' + *`; they are resolved against the names known in the model (DMN 1.5
//! §10.3.1.2), the longest known name winning.

use std::collections::HashSet;
use std::rc::Rc;

use super::num::Num;
use crate::feel::{FeelError, MAX_DEPTH, TOO_DEEP};
use crate::pyutil::repr_char;

type R<T> = Result<T, FeelError>;

fn syntax<T>(msg: impl Into<String>) -> R<T> {
    Err(FeelError::Syntax(msg.into()))
}

/// A FEEL type of `instance of`, of parameters and of item definitions.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Any,
    Null,
    Number,
    String,
    Boolean,
    Date,
    Time,
    DateTime,
    DtDuration,
    YmDuration,
    List(Box<Type>),
    Context(Vec<(String, Type)>),
    Range(Box<Type>),
    Function(Vec<Type>, Box<Type>),
    /// A type defined in the model (an item definition).
    Named(String),
}

impl Type {
    /// The built-in type with this name (`number`, `date and time`, `list` …).
    pub fn builtin(name: &str) -> Option<Type> {
        Some(match name {
            "Any" | "any" => Type::Any,
            "Null" => Type::Null,
            "number" => Type::Number,
            "string" => Type::String,
            "boolean" => Type::Boolean,
            "date" => Type::Date,
            "time" => Type::Time,
            "date and time" | "dateTime" => Type::DateTime,
            "days and time duration" | "dayTimeDuration" => Type::DtDuration,
            "years and months duration" | "yearMonthDuration" => Type::YmDuration,
            "list" => Type::List(Box::new(Type::Any)),
            "context" => Type::Context(Vec::new()),
            "range" => Type::Range(Box::new(Type::Any)),
            "function" => Type::Function(Vec::new(), Box::new(Type::Any)),
            _ => return None,
        })
    }

    /// A type from a `typeRef` text: a built-in type, a type expression or a model type.
    pub fn from_ref(text: &str) -> Type {
        let t = text.trim();
        let t = t.strip_prefix("feel:").unwrap_or(t);
        if let Some(b) = Type::builtin(t) {
            return b;
        }
        if let Ok(ty) = parse_type(t, &Names::default()) {
            return ty;
        }
        Type::Named(t.to_string())
    }
}

/// A FEEL expression.
#[derive(Debug, Clone)]
pub enum Expr {
    Null,
    Bool(bool),
    Num(Num),
    Str(String),
    /// `@"…"` literal.
    At(String),
    Name(String),
    List(Vec<Expr>),
    Context(Vec<(String, Expr)>),
    /// A range literal `[a..b]`; an absent end comes from a unary comparison.
    Range(Option<Box<Expr>>, bool, Option<Box<Expr>>, bool),
    /// `< e`, `<= e`, `> e`, `>= e`, `= e`, `!= e` as a test or a range.
    Compare1(&'static str, Box<Expr>),
    Path(Box<Expr>, String),
    Filter(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<(Option<String>, Expr)>),
    Neg(Box<Expr>),
    Arith(char, Box<Expr>, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    Between(Box<Expr>, Box<Expr>, Box<Expr>),
    In(Box<Expr>, Vec<Expr>),
    InstanceOf(Box<Expr>, Type),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    For(Vec<(String, Expr, Option<Expr>)>, Box<Expr>),
    Quantified(bool, Vec<(String, Expr)>, Box<Expr>),
    Function(Vec<(String, Option<Type>)>, Rc<Expr>),
}

/// Unary tests of a decision table input cell or of `allowedValues`.
#[derive(Debug, Clone)]
pub enum Tests {
    Any,
    Not(Vec<Expr>),
    Some(Vec<Expr>),
}

/// Names that may contain spaces or additional symbols, for name resolution.
#[derive(Debug, Clone, Default)]
pub struct Names {
    exact: HashSet<String>,
    prefixes: HashSet<String>,
}

/// Names of the built-in functions and named parameters that contain spaces.
pub const BUILTIN_NAMES: &[&str] = &[
    "date and time",
    "years and months duration",
    "days and time duration",
    "string length",
    "upper case",
    "lower case",
    "substring before",
    "substring after",
    "starts with",
    "ends with",
    "string join",
    "list contains",
    "insert before",
    "index of",
    "distinct values",
    "list replace",
    "round up",
    "round down",
    "round half up",
    "round half down",
    "day of year",
    "day of week",
    "month of year",
    "week of year",
    "last day of month",
    "overlaps before",
    "overlaps after",
    "finished by",
    "started by",
    "met by",
    "get value",
    "get entries",
    "context put",
    "context merge",
    "start position",
    "grouping separator",
    "decimal separator",
    "start included",
    "end included",
    "time offset",
    "position",
    "new item",
    "insert",
    "match",
    "case sensitive",
];

fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut space = false;
    for c in name.trim().chars() {
        if is_space(c) {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

impl Names {
    /// The names of the built-in functions.
    pub fn builtins() -> Names {
        let mut n = Names::default();
        for b in BUILTIN_NAMES {
            n.add(b);
        }
        n
    }

    pub fn add(&mut self, name: &str) {
        let n = normalize(name);
        if n.is_empty() {
            return;
        }
        let chars: Vec<char> = n.chars().collect();
        for k in 1..=chars.len() {
            self.prefixes.insert(chars[..k].iter().collect());
        }
        self.exact.insert(n);
    }

    fn known(&self, n: &str) -> bool {
        self.exact.contains(n)
    }

    /// Whether `name` (its words separated by any white space) is one of the names.
    pub fn contains(&self, name: &str) -> bool {
        self.known(&normalize(name))
    }

    fn prefix(&self, n: &str) -> bool {
        self.prefixes.contains(n)
    }
}

pub fn is_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\u{0B}'
            | '\u{0C}'
            | '\u{A0}'
            | '\u{FEFF}'
            | '\u{1680}'
            | '\u{180E}'
            | '\u{2000}'..='\u{200B}' | '\u{2028}' | '\u{2029}' | '\u{205F}' | '\u{3000}'
    )
}

fn name_start(c: char) -> bool {
    c == '?' || c == '_' || c.is_alphabetic() || ('\u{10000}'..='\u{EFFFF}').contains(&c)
}

fn name_part(c: char) -> bool {
    name_start(c)
        || c.is_alphanumeric()
        || c == '\u{B7}'
        || ('\u{300}'..='\u{36F}').contains(&c)
        || ('\u{203F}'..='\u{2040}').contains(&c)
}

#[derive(Debug, Clone, PartialEq)]
enum K {
    Num,
    Str,
    Name,
    Op,
    End,
}

#[derive(Debug, Clone)]
struct Tok {
    k: K,
    text: String,
    /// White space (or a comment) before the token.
    sp: bool,
    /// Position of the token in the text (characters).
    start: usize,
    end: usize,
}

fn tokenize(text: &str) -> R<Vec<Tok>> {
    let ch: Vec<char> = text.chars().collect();
    let n = ch.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut sp = false;
    let s = |a: usize, b: usize| -> String { ch[a..b].iter().collect() };
    while i < n {
        let c = ch[i];
        if is_space(c) {
            sp = true;
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            sp = true;
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            let mut j = i + 2;
            while j + 1 < n && !(ch[j] == '*' && ch[j + 1] == '/') {
                j += 1;
            }
            if j + 1 >= n {
                return syntax("unterminated comment");
            }
            i = j + 2;
            sp = true;
            continue;
        }
        let st = i;
        let tok = |k: K, t: String, sp: bool| Tok {
            k,
            text: t,
            sp,
            start: 0,
            end: 0,
        };
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && ch[i + 1].is_ascii_digit()) {
            let mut j = i;
            while j < n && ch[j].is_ascii_digit() {
                j += 1;
            }
            if j + 1 < n && ch[j] == '.' && ch[j + 1].is_ascii_digit() {
                j += 1;
                while j < n && ch[j].is_ascii_digit() {
                    j += 1;
                }
            }
            if j < n && (ch[j] == 'e' || ch[j] == 'E') {
                let mut k = j + 1;
                if k < n && (ch[k] == '+' || ch[k] == '-') {
                    k += 1;
                }
                if k < n && ch[k].is_ascii_digit() {
                    while k < n && ch[k].is_ascii_digit() {
                        k += 1;
                    }
                    j = k;
                }
            }
            out.push(tok(K::Num, s(i, j), sp));
            i = j;
        } else if c == '"' {
            let mut j = i + 1;
            let mut buf = String::new();
            loop {
                if j >= n {
                    return syntax("unterminated string");
                }
                let x = ch[j];
                if x == '\\' && j + 1 < n {
                    let e = ch[j + 1];
                    match e {
                        'n' => buf.push('\n'),
                        't' => buf.push('\t'),
                        'r' => buf.push('\r'),
                        'b' => buf.push('\u{8}'),
                        'f' => buf.push('\u{C}'),
                        '"' => buf.push('"'),
                        '\'' => buf.push('\''),
                        '\\' => buf.push('\\'),
                        'u' | 'U' => {
                            let len = if e == 'u' { 4 } else { 6 };
                            let hex: String =
                                ch[(j + 2).min(n)..(j + 2 + len).min(n)].iter().collect();
                            let Some(code) = u32::from_str_radix(&hex, 16)
                                .ok()
                                .filter(|_| hex.len() == len)
                            else {
                                return syntax("malformed escape in a string");
                            };
                            // a surrogate pair 💩
                            if (0xD800..0xDC00).contains(&code)
                                && j + 2 + len + 6 <= n
                                && ch[j + 2 + len] == '\\'
                                && ch[j + 3 + len] == 'u'
                            {
                                let low: String = ch[j + 4 + len..j + 8 + len].iter().collect();
                                if let Ok(lo) = u32::from_str_radix(&low, 16)
                                    && (0xDC00..0xE000).contains(&lo)
                                {
                                    let cp = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                                    if let Some(c) = char::from_u32(cp) {
                                        buf.push(c);
                                    }
                                    j += 2 + len + 6;
                                    continue;
                                }
                            }
                            match char::from_u32(code) {
                                Some(c) => buf.push(c),
                                None => return syntax("malformed escape in a string"),
                            }
                            j += 2 + len;
                            continue;
                        }
                        other => {
                            buf.push('\\');
                            buf.push(other);
                        }
                    }
                    j += 2;
                } else if x == '"' {
                    break;
                } else {
                    buf.push(x);
                    j += 1;
                }
            }
            out.push(tok(K::Str, buf, sp));
            i = j + 1;
        } else if name_start(c) {
            let mut j = i + 1;
            while j < n && name_part(ch[j]) {
                j += 1;
            }
            out.push(tok(K::Name, s(i, j), sp));
            i = j;
        } else {
            let two = if i + 1 < n {
                s(i, i + 2)
            } else {
                String::new()
            };
            if ["**", "..", "<=", ">=", "!=", "->"].contains(&two.as_str()) {
                out.push(tok(K::Op, two, sp));
                i += 2;
            } else if "+-*/=<>()[]{},.:@'".contains(c) {
                out.push(tok(K::Op, c.to_string(), sp));
                i += 1;
            } else {
                return syntax(format!("unexpected character {}", repr_char(c)));
            }
        }
        if let Some(t) = out.last_mut() {
            t.start = st;
            t.end = i;
        }
        sp = false;
    }
    out.push(Tok {
        k: K::End,
        text: String::new(),
        sp,
        start: n,
        end: n,
    });
    Ok(out)
}

const KEYWORDS: &[&str] = &[
    "if",
    "then",
    "else",
    "for",
    "return",
    "in",
    "some",
    "every",
    "satisfies",
    "and",
    "or",
    "between",
    "instance",
    "of",
    "true",
    "false",
    "null",
    "function",
    "external",
];

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    names: &'a Names,
    local: Vec<String>,
    /// Parsing the high end of a range: `[` may close it (`[1..10[`).
    range_high: bool,
    /// The names read, with their positions: (start, end, name, a key of a context).
    spans: Vec<(usize, usize, String, bool)>,
    /// Nesting of the expression parsed: a bound on the height of its tree.
    depth: usize,
}

fn show(t: &Tok) -> String {
    if t.k == K::End {
        "end of expression".into()
    } else {
        format!("'{}'", t.text)
    }
}

impl P<'_> {
    fn peek(&self, k: usize) -> &Tok {
        &self.t[(self.i + k).min(self.t.len() - 1)]
    }
    fn next(&mut self) -> Tok {
        let t = self.peek(0).clone();
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn op(&self, o: &str) -> bool {
        let t = self.peek(0);
        t.k == K::Op && t.text == o
    }
    fn word(&self, w: &str) -> bool {
        let t = self.peek(0);
        t.k == K::Name && t.text == w
    }
    fn word_at(&self, k: usize, w: &str) -> bool {
        let t = self.peek(k);
        t.k == K::Name && t.text == w
    }
    fn expect_op(&mut self, o: &str) -> R<()> {
        if self.op(o) {
            self.next();
            return Ok(());
        }
        syntax(format!("expected '{o}', found {}", show(self.peek(0))))
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.word(w) {
            self.next();
            return Ok(());
        }
        syntax(format!("expected '{w}', found {}", show(self.peek(0))))
    }
    /// Parse a nested part of the expression; the nesting is bounded, so that the
    /// parser and everything that walks the tree (evaluation, the names of an
    /// expression) stays within the stack.
    fn nested<T>(&mut self, part: fn(&mut Self) -> R<T>) -> R<T> {
        let saved = self.depth;
        self.deeper()?;
        let r = part(self);
        self.depth = saved;
        r
    }

    /// One more level of the tree: a nested part, or an operator applied to what was
    /// parsed so far.
    fn deeper(&mut self) -> R<()> {
        if self.depth >= MAX_DEPTH {
            return syntax(TOO_DEEP);
        }
        self.depth += 1;
        Ok(())
    }

    fn at_end(&self) -> bool {
        self.peek(0).k == K::End
    }

    fn known(&self, n: &str) -> bool {
        self.names.known(n) || self.local.iter().any(|l| l == n)
    }
    fn prefix(&self, n: &str) -> bool {
        self.names.prefix(n) || self.local.iter().any(|l| l.starts_with(n))
    }

    /// The longest known name starting at the current token (or the token itself).
    fn name(&mut self) -> R<String> {
        let first = self.peek(0).clone();
        if first.k != K::Name {
            return syntax(format!("expected a name, found {}", show(&first)));
        }
        let mut joined = first.text.clone();
        let mut best = (0usize, joined.clone());
        let mut k = 1;
        loop {
            let t = self.peek(k);
            let joinable = match t.k {
                K::Name | K::Num => true,
                K::Op => [".", "/", "-", "'", "+", "*"].contains(&t.text.as_str()),
                _ => false,
            };
            if !joinable {
                break;
            }
            let cand = format!("{joined}{}{}", if t.sp { " " } else { "" }, t.text);
            if !self.prefix(&cand) {
                break;
            }
            joined = cand;
            if self.known(&joined) {
                best = (k, joined.clone());
            }
            k += 1;
        }
        let end = self.peek(best.0).end;
        self.spans.push((first.start, end, best.1.clone(), false));
        self.i += best.0 + 1;
        Ok(best.1)
    }

    /// A name up to `stop` (a key of a context, a parameter name, a bound variable):
    /// every token before it, joined.
    fn name_until(&mut self, stop: &[&str]) -> R<String> {
        let mut out = String::new();
        loop {
            let t = self.peek(0).clone();
            let is_stop = match t.k {
                K::Op => stop.contains(&t.text.as_str()),
                K::Name => stop.contains(&t.text.as_str()) && !out.is_empty(),
                K::End => true,
                _ => false,
            };
            if is_stop {
                break;
            }
            if !matches!(t.k, K::Name | K::Num | K::Op)
                || (t.k == K::Op && ![".", "/", "-", "'", "+", "*"].contains(&t.text.as_str()))
            {
                return syntax(format!("unexpected {}", show(&t)));
            }
            if !out.is_empty() && t.sp {
                out.push(' ');
            }
            out.push_str(&t.text);
            self.next();
        }
        if out.is_empty() {
            return syntax(format!("expected a name, found {}", show(self.peek(0))));
        }
        Ok(out)
    }

    fn expression(&mut self) -> R<Expr> {
        self.nested(Self::expression_body)
    }

    fn expression_body(&mut self) -> R<Expr> {
        if self.word("if") {
            self.next();
            let c = self.expression()?;
            self.expect_word("then")?;
            let a = self.expression()?;
            self.expect_word("else")?;
            let b = self.expression()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        if self.word("for") && self.peek(1).k == K::Name {
            self.next();
            let mut ctx = Vec::new();
            let saved = self.local.len();
            loop {
                let name = self.name_until(&["in"])?;
                self.expect_word("in")?;
                let from = self.expression()?;
                let to = if self.op("..") {
                    self.next();
                    Some(self.expression()?)
                } else {
                    None
                };
                self.local.push(name.clone());
                ctx.push((name, from, to));
                if self.op(",") {
                    self.next();
                    continue;
                }
                break;
            }
            self.expect_word("return")?;
            self.local.push("partial".into());
            let body = self.expression()?;
            self.local.truncate(saved);
            return Ok(Expr::For(ctx, Box::new(body)));
        }
        if (self.word("some") || self.word("every")) && self.peek(1).k == K::Name {
            let every = self.next().text == "every";
            let mut ctx = Vec::new();
            let saved = self.local.len();
            loop {
                let name = self.name_until(&["in"])?;
                self.expect_word("in")?;
                let list = self.expression()?;
                self.local.push(name.clone());
                ctx.push((name, list));
                if self.op(",") {
                    self.next();
                    continue;
                }
                break;
            }
            self.expect_word("satisfies")?;
            let body = self.expression()?;
            self.local.truncate(saved);
            return Ok(Expr::Quantified(every, ctx, Box::new(body)));
        }
        self.disjunction()
    }

    fn disjunction(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.conjunction()?;
        while self.word("or") {
            self.deeper()?;
            self.next();
            left = Expr::Or(Box::new(left), Box::new(self.conjunction()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn conjunction(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.comparison()?;
        while self.word("and") {
            self.deeper()?;
            self.next();
            left = Expr::And(Box::new(left), Box::new(self.comparison()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn comparison(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.additive()?;
        loop {
            let t = self.peek(0).clone();
            if t.k == K::Op
                && let Some(op) = cmp_op(&t.text)
            {
                self.next();
                self.deeper()?;
                left = Expr::Cmp(op, Box::new(left), Box::new(self.additive()?));
                continue;
            }
            if self.word("between") {
                self.next();
                self.deeper()?;
                let lo = self.additive()?;
                self.expect_word("and")?;
                let hi = self.additive()?;
                left = Expr::Between(Box::new(left), Box::new(lo), Box::new(hi));
                continue;
            }
            if self.word("in") {
                self.next();
                self.deeper()?;
                let tests = self.in_target()?;
                left = Expr::In(Box::new(left), tests);
                continue;
            }
            if self.word("instance") && self.word_at(1, "of") {
                self.next();
                self.next();
                self.deeper()?;
                let ty = self.type_expr()?;
                left = Expr::InstanceOf(Box::new(left), ty);
                continue;
            }
            self.depth = base;
            return Ok(left);
        }
    }

    fn in_target(&mut self) -> R<Vec<Expr>> {
        if self.op("(") {
            let (save, depth) = (self.i, self.depth);
            self.next();
            let attempt = (|| -> R<Vec<Expr>> {
                let mut tests = vec![self.positive_test()?];
                while self.op(",") {
                    self.next();
                    tests.push(self.positive_test()?);
                }
                self.expect_op(")")?;
                Ok(tests)
            })();
            if let Ok(tests) = attempt {
                return Ok(tests);
            }
            (self.i, self.depth) = (save, depth);
            return Ok(vec![self.positive_test()?]);
        }
        Ok(vec![self.positive_test()?])
    }

    /// A positive unary test: a unary comparison, or an expression.
    fn positive_test(&mut self) -> R<Expr> {
        let t = self.peek(0).clone();
        if t.k == K::Op
            && let Some(op) = cmp_op(&t.text)
        {
            self.next();
            return Ok(Expr::Compare1(op, Box::new(self.additive()?)));
        }
        self.expression()
    }

    fn additive(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.multiplicative()?;
        while self.op("+") || self.op("-") {
            self.deeper()?;
            let op = self.next().text.chars().next().unwrap_or('+');
            left = Expr::Arith(op, Box::new(left), Box::new(self.multiplicative()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn multiplicative(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.power()?;
        while self.op("*") || self.op("/") {
            self.deeper()?;
            let op = self.next().text.chars().next().unwrap_or('*');
            left = Expr::Arith(op, Box::new(left), Box::new(self.power()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn power(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut left = self.unary()?;
        while self.op("**") {
            self.deeper()?;
            self.next();
            left = Expr::Pow(Box::new(left), Box::new(self.unary()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn unary(&mut self) -> R<Expr> {
        self.nested(Self::unary_body)
    }

    fn unary_body(&mut self) -> R<Expr> {
        if self.op("-") {
            self.next();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.op("+") {
            self.next();
            return self.unary();
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R<Expr> {
        let base = self.depth;
        let mut e = self.primary()?;
        loop {
            if self.op(".") {
                self.next();
                self.deeper()?;
                let name = if self.peek(0).k == K::Str {
                    self.next().text
                } else {
                    self.name()?
                };
                e = Expr::Path(Box::new(e), name);
            } else if self.op("[") && !(self.range_high && self.closes_range()) {
                self.next();
                self.deeper()?;
                let outer = self.range_high;
                self.range_high = false;
                let f = self.expression();
                self.range_high = outer;
                let f = f?;
                self.expect_op("]")?;
                e = Expr::Filter(Box::new(e), Box::new(f));
            } else if self.op("(") {
                self.next();
                self.deeper()?;
                let args = self.arguments()?;
                e = Expr::Call(Box::new(e), args);
            } else {
                self.depth = base;
                return Ok(e);
            }
        }
    }

    /// At `[` in the high end of a range: the bracket closes the range when what follows
    /// cannot start a filter expression.
    fn closes_range(&self) -> bool {
        let t = self.peek(1);
        match t.k {
            K::End => true,
            K::Op => [
                ".", ",", ")", "]", "=", "!=", "<", ">", "<=", ">=", "*", "/", "**", "+",
            ]
            .contains(&t.text.as_str()),
            K::Name => [
                "and",
                "or",
                "then",
                "else",
                "return",
                "satisfies",
                "between",
                "instance",
                "in",
            ]
            .contains(&t.text.as_str()),
            _ => false,
        }
    }

    fn arguments(&mut self) -> R<Vec<(Option<String>, Expr)>> {
        let mut args = Vec::new();
        if self.op(")") {
            self.next();
            return Ok(args);
        }
        loop {
            // a named argument: name tokens followed by ':'
            let mut k = 0;
            let mut named = false;
            while matches!(self.peek(k).k, K::Name | K::Num)
                || (self.peek(k).k == K::Op
                    && ["-", "'"].contains(&self.peek(k).text.as_str())
                    && k > 0)
            {
                k += 1;
            }
            if k > 0
                && self.peek(0).k == K::Name
                && self.peek(k).k == K::Op
                && self.peek(k).text == ":"
            {
                named = true;
            }
            if named {
                let mut name = String::new();
                for _ in 0..k {
                    let t = self.next();
                    if !name.is_empty() && t.sp {
                        name.push(' ');
                    }
                    name.push_str(&t.text);
                }
                self.next();
                args.push((Some(name), self.expression()?));
            } else {
                args.push((None, self.expression()?));
            }
            if self.op(",") {
                self.next();
                continue;
            }
            self.expect_op(")")?;
            return Ok(args);
        }
    }

    fn range_end(&mut self, low: Expr, low_closed: bool) -> R<Expr> {
        self.expect_op("..")?;
        let outer = self.range_high;
        self.range_high = true;
        let high = self.expression();
        self.range_high = outer;
        let high = high?;
        let t = self.next();
        let high_closed = match (t.k, t.text.as_str()) {
            (K::Op, "]") => true,
            (K::Op, ")") | (K::Op, "[") => false,
            _ => return syntax("expected ']' or ')' closing the interval"),
        };
        Ok(Expr::Range(
            Some(Box::new(low)),
            low_closed,
            Some(Box::new(high)),
            high_closed,
        ))
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.peek(0).clone();
        match t.k {
            K::Num => {
                self.next();
                match Num::parse(&t.text) {
                    Some(n) => Ok(Expr::Num(n)),
                    None => syntax(format!("malformed number '{}'", t.text)),
                }
            }
            K::Str => {
                self.next();
                Ok(Expr::Str(t.text))
            }
            K::End => syntax("unexpected end of expression"),
            K::Op => match t.text.as_str() {
                "(" => {
                    self.next();
                    let e = self.expression()?;
                    if self.op("..") {
                        return self.range_end(e, false);
                    }
                    self.expect_op(")")?;
                    Ok(e)
                }
                "]" => {
                    self.next();
                    let e = self.expression()?;
                    self.range_end(e, false)
                }
                "[" => {
                    self.next();
                    if self.op("]") {
                        self.next();
                        return Ok(Expr::List(Vec::new()));
                    }
                    let first = self.expression()?;
                    if self.op("..") {
                        return self.range_end(first, true);
                    }
                    let mut items = vec![first];
                    while self.op(",") {
                        self.next();
                        items.push(self.expression()?);
                    }
                    self.expect_op("]")?;
                    Ok(Expr::List(items))
                }
                "{" => {
                    self.next();
                    let mut entries: Vec<(String, Expr)> = Vec::new();
                    let saved = self.local.len();
                    if !self.op("}") {
                        loop {
                            let key = if self.peek(0).k == K::Str {
                                self.next().text
                            } else {
                                let start = self.peek(0).start;
                                let k = self.name_until(&[":"])?;
                                let end = self.t[self.i.saturating_sub(1)].end;
                                self.spans.push((start, end, k.clone(), true));
                                k
                            };
                            self.expect_op(":")?;
                            let v = self.expression()?;
                            if entries.iter().any(|(k, _)| *k == key) {
                                return syntax(format!("duplicate context entry '{key}'"));
                            }
                            self.local.push(key.clone());
                            entries.push((key, v));
                            if self.op(",") {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    self.local.truncate(saved);
                    self.expect_op("}")?;
                    Ok(Expr::Context(entries))
                }
                "@" => {
                    self.next();
                    let s = self.next();
                    if s.k != K::Str {
                        return syntax("expected a string after '@'");
                    }
                    Ok(Expr::At(s.text))
                }
                "<" | "<=" | ">" | ">=" => {
                    self.next();
                    let op = cmp_op(&t.text).unwrap_or("<");
                    Ok(Expr::Compare1(op, Box::new(self.additive()?)))
                }
                _ => syntax(format!("unexpected {}", show(&t))),
            },
            K::Name => match t.text.as_str() {
                "true" => {
                    self.next();
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.next();
                    Ok(Expr::Bool(false))
                }
                "null" => {
                    self.next();
                    Ok(Expr::Null)
                }
                "if" => self.expression(),
                // an iteration or a quantifier: `expression` reads it when a name
                // follows; otherwise the keyword is out of place (below)
                "for" | "some" | "every" if self.peek(1).k == K::Name => self.expression(),
                "function" if self.peek(1).k == K::Op && self.peek(1).text == "(" => {
                    self.next();
                    self.next();
                    let mut params = Vec::new();
                    if !self.op(")") {
                        loop {
                            let name = self.name_until(&[":", ",", ")"])?;
                            let ty = if self.op(":") {
                                self.next();
                                Some(self.type_expr()?)
                            } else {
                                None
                            };
                            params.push((name, ty));
                            if self.op(",") {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_op(")")?;
                    if self.word("external") {
                        return Err(FeelError::Unsupported(
                            "externally defined functions ('external')".into(),
                        ));
                    }
                    let saved = self.local.len();
                    self.local.extend(params.iter().map(|(n, _)| n.clone()));
                    let body = self.expression()?;
                    self.local.truncate(saved);
                    Ok(Expr::Function(params, Rc::new(body)))
                }
                w if KEYWORDS.contains(&w) && !self.known(w) => {
                    syntax(format!("unexpected keyword '{w}'"))
                }
                _ => Ok(Expr::Name(self.name()?)),
            },
        }
    }

    fn type_expr(&mut self) -> R<Type> {
        self.nested(Self::type_expr_body)
    }

    fn type_expr_body(&mut self) -> R<Type> {
        let t = self.peek(0).clone();
        if t.k != K::Name {
            return syntax(format!("expected a type, found {}", show(&t)));
        }
        if self.peek(1).k == K::Op && self.peek(1).text == "<" {
            let head = t.text.clone();
            match head.as_str() {
                "list" | "range" => {
                    self.next();
                    self.next();
                    let inner = self.type_expr()?;
                    self.expect_op(">")?;
                    return Ok(if head == "list" {
                        Type::List(Box::new(inner))
                    } else {
                        Type::Range(Box::new(inner))
                    });
                }
                "context" => {
                    self.next();
                    self.next();
                    let mut fields = Vec::new();
                    if !self.op(">") {
                        loop {
                            let name = if self.peek(0).k == K::Str {
                                self.next().text
                            } else {
                                self.name_until(&[":"])?
                            };
                            self.expect_op(":")?;
                            fields.push((name, self.type_expr()?));
                            if self.op(",") {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_op(">")?;
                    return Ok(Type::Context(fields));
                }
                "function" => {
                    self.next();
                    self.next();
                    let mut params = Vec::new();
                    if !self.op(">") {
                        loop {
                            params.push(self.type_expr()?);
                            if self.op(",") {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_op(">")?;
                    self.expect_op("->")?;
                    let out = self.type_expr()?;
                    return Ok(Type::Function(params, Box::new(out)));
                }
                _ => {}
            }
        }
        // the longest built-in or known type name
        for cand in [
            "years and months duration",
            "days and time duration",
            "date and time",
        ] {
            let words: Vec<&str> = cand.split(' ').collect();
            if words.iter().enumerate().all(|(k, w)| self.word_at(k, w)) {
                self.i += words.len();
                return Ok(Type::builtin(cand).unwrap_or(Type::Any));
            }
        }
        let name = self.name()?;
        Ok(Type::builtin(&name).unwrap_or(Type::Named(name)))
    }

    fn unary_tests(&mut self) -> R<Tests> {
        if self.op("-") && self.peek(1).k == K::End {
            self.next();
            return Ok(Tests::Any);
        }
        if self.word("not") && self.peek(1).k == K::Op && self.peek(1).text == "(" {
            let (save, depth) = (self.i, self.depth);
            self.next();
            self.next();
            let attempt = (|| -> R<Vec<Expr>> {
                let mut tests = vec![self.positive_test()?];
                while self.op(",") {
                    self.next();
                    tests.push(self.positive_test()?);
                }
                self.expect_op(")")?;
                Ok(tests)
            })();
            if let Ok(tests) = attempt
                && self.at_end()
            {
                return Ok(Tests::Not(tests));
            }
            (self.i, self.depth) = (save, depth);
        }
        let mut tests = vec![self.positive_test()?];
        while self.op(",") {
            self.next();
            tests.push(self.positive_test()?);
        }
        Ok(Tests::Some(tests))
    }
}

fn cmp_op(t: &str) -> Option<&'static str> {
    Some(match t {
        "=" => "=",
        "!=" => "!=",
        "<" => "<",
        "<=" => "<=",
        ">" => ">",
        ">=" => ">=",
        _ => return None,
    })
}

fn parser<'a>(text: &str, names: &'a Names) -> R<P<'a>> {
    Ok(P {
        t: tokenize(text)?,
        i: 0,
        names,
        local: Vec::new(),
        range_high: false,
        spans: Vec::new(),
        depth: 0,
    })
}

/// Parse a FEEL expression.
pub fn parse_expression(text: &str, names: &Names) -> R<Expr> {
    if text.trim_matches(is_space).is_empty() {
        return syntax("empty expression");
    }
    let mut p = parser(text, names)?;
    no_comparison_after_comparison(&p.t)?;
    let e = p.expression()?;
    if !p.at_end() {
        return syntax(format!("unexpected {}", show(p.peek(0))));
    }
    Ok(e)
}

/// `a >= > 18`: in an expression a comparison operator cannot follow another one (the unary
/// comparison `> 18` is a primary of its own, but nothing is compared with it). Checked on
/// the tokens before parsing, so that the recursive descent (which runs deeply nested
/// expressions at the limit of the stack) is unchanged. The angle brackets of a type
/// (`list<list<number>>`, `context<a: number>`, `range<…>`, `function<…>`) are skipped.
fn no_comparison_after_comparison(t: &[Tok]) -> R<()> {
    let is_cmp = |x: &Tok| x.k == K::Op && cmp_op(&x.text).is_some();
    let mut angle = 0usize;
    for (i, x) in t.iter().enumerate() {
        if x.k == K::Op
            && x.text == "<"
            && i > 0
            && t[i - 1].k == K::Name
            && matches!(
                t[i - 1].text.as_str(),
                "list" | "context" | "range" | "function"
            )
        {
            angle += 1;
            continue;
        }
        if angle > 0 {
            if x.k == K::Op && x.text == ">" {
                angle -= 1;
            }
            continue;
        }
        if is_cmp(x) && t.get(i + 1).is_some_and(is_cmp) {
            return syntax(format!("unexpected {}", show(&t[i + 1])));
        }
    }
    Ok(())
}

/// Parse unary tests (a decision table input cell, allowed values).
pub fn parse_tests(text: &str, names: &Names) -> R<Tests> {
    if text.trim_matches(is_space).is_empty() {
        return syntax("empty cell; use '-' for any value");
    }
    let mut p = parser(text, names)?;
    let t = p.unary_tests()?;
    if !p.at_end() {
        return syntax(format!("unexpected {}", show(p.peek(0))));
    }
    Ok(t)
}

/// Parse a type expression (`number`, `list<string>`, `context<a: number>`).
pub fn parse_type(text: &str, names: &Names) -> R<Type> {
    let mut p = parser(text, names)?;
    let t = p.type_expr()?;
    if !p.at_end() {
        return syntax(format!("unexpected {}", show(p.peek(0))));
    }
    Ok(t)
}

/// The names an expression reads (for requirement checks): names that are not bound
/// inside it.
pub fn free_names(e: &Expr, out: &mut Vec<String>) {
    fn go(e: &Expr, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match e {
            Expr::Name(n) => {
                if !bound.contains(n) && !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Expr::Null | Expr::Bool(_) | Expr::Num(_) | Expr::Str(_) | Expr::At(_) => {}
            Expr::List(items) => items.iter().for_each(|x| go(x, bound, out)),
            Expr::Context(entries) => {
                let n = bound.len();
                for (k, v) in entries {
                    go(v, bound, out);
                    bound.push(k.clone());
                }
                bound.truncate(n);
            }
            Expr::Range(a, _, b, _) => {
                for x in [a, b].into_iter().flatten() {
                    go(x, bound, out);
                }
            }
            Expr::Compare1(_, x) | Expr::Neg(x) | Expr::InstanceOf(x, _) => go(x, bound, out),
            Expr::Path(x, _) => go(x, bound, out),
            Expr::Filter(x, f) => {
                go(x, bound, out);
                // names inside a filter may be fields of the elements
                let mut inner = Vec::new();
                go(f, bound, &mut inner);
                for n in inner {
                    if n != "item" && !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
            Expr::Call(f, args) => {
                go(f, bound, out);
                args.iter().for_each(|(_, a)| go(a, bound, out));
            }
            Expr::Arith(_, a, b)
            | Expr::Pow(a, b)
            | Expr::Cmp(_, a, b)
            | Expr::And(a, b)
            | Expr::Or(a, b) => {
                go(a, bound, out);
                go(b, bound, out);
            }
            Expr::Between(a, b, c) | Expr::If(a, b, c) => {
                go(a, bound, out);
                go(b, bound, out);
                go(c, bound, out);
            }
            Expr::In(a, tests) => {
                go(a, bound, out);
                tests.iter().for_each(|t| go(t, bound, out));
            }
            Expr::For(ctx, body) => {
                let n = bound.len();
                for (name, from, to) in ctx {
                    go(from, bound, out);
                    if let Some(t) = to {
                        go(t, bound, out);
                    }
                    bound.push(name.clone());
                }
                bound.push("partial".into());
                go(body, bound, out);
                bound.truncate(n);
            }
            Expr::Quantified(_, ctx, body) => {
                let n = bound.len();
                for (name, list) in ctx {
                    go(list, bound, out);
                    bound.push(name.clone());
                }
                go(body, bound, out);
                bound.truncate(n);
            }
            Expr::Function(params, body) => {
                let n = bound.len();
                bound.extend(params.iter().map(|(p, _)| p.clone()));
                go(body, bound, out);
                bound.truncate(n);
            }
        }
    }
    let mut bound = vec!["?".to_string()];
    go(e, &mut bound, out);
}

/// Whether Camunda's FEEL reads `name` as a name without quoting: a letter or `_`, then
/// letters, digits and `_`.
fn plain_name(name: &str) -> bool {
    let mut c = name.chars();
    matches!(c.next(), Some(f) if f.is_alphabetic() || f == '_')
        && c.all(|x| x.is_alphanumeric() || x == '_')
}

/// A FEEL text for Camunda 8 (feel-scala): every name the parser resolves that is not a
/// plain name (spaces, `-`, `+`, `/`, `*`, `'`, dots inside the name) is written in
/// backticks, and such a key of a context as a string; built-in functions, strings and
/// the rest of the text stay as written. A text that does not parse is returned as is.
pub fn camunda_text(text: &str, names: &Names, tests: bool) -> String {
    let Ok(mut p) = parser(text, names) else {
        return text.to_string();
    };
    let ok = if tests {
        p.unary_tests().is_ok() && p.at_end()
    } else {
        p.expression().is_ok() && p.at_end()
    };
    if !ok {
        return text.to_string();
    }
    let mut spans: Vec<(usize, usize, String, bool)> = p
        .spans
        .into_iter()
        .filter(|(_, _, n, _)| {
            !plain_name(n)
                && n != "?"
                && !BUILTIN_NAMES.contains(&n.as_str())
                && super::builtins::canonical(n).is_none()
        })
        .collect();
    spans.sort_by_key(|x| x.0);
    spans.dedup_by_key(|x| x.0);
    let ch: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut pos = 0;
    for (a, b, name, key) in spans {
        if a < pos || b > ch.len() {
            continue;
        }
        out.extend(&ch[pos..a]);
        if key {
            out.push_str(&super::quote(&name));
        } else {
            out.push('`');
            out.push_str(&name);
            out.push('`');
        }
        pos = b;
    }
    out.extend(&ch[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str, names: &Names) -> bool {
        parse_expression(text, names).is_ok()
    }

    #[test]
    fn names_with_spaces() {
        let mut names = Names::builtins();
        names.add("Ocena ryzyka");
        names.add("dni-zwloki");
        let e = parse_expression("Ocena ryzyka + 1", &names);
        assert!(matches!(e, Ok(Expr::Arith('+', _, _))));
        let e = parse_expression("dni-zwloki * 2", &names);
        assert!(
            matches!(e, Ok(Expr::Arith('*', ref a, _)) if matches!(a.as_ref(), Expr::Name(n) if n == "dni-zwloki"))
        );
        assert!(ok("date and time(\"2027-01-01T10:00:00\")", &names));
        assert!(ok("for x in [1, 2], y in 1..3 return x + y", &names));
        assert!(ok("some x in l satisfies x > 2", &names));
        assert!(ok("{a: 1, \"b c\": a + 1}.a", &names));
        assert!(ok("l[item > 1][1]", &names));
        assert!(ok("x instance of list<number>", &names));
        assert!(ok("x in (1, 2, > 5)", &names));
        assert!(ok("(function(a, b) a * b)(2, 3)", &names));
        assert!(ok("substring(string: \"abc\", start position: 2)", &names));
        assert!(ok("x between 1 and 10 // comment", &names));
        assert!(ok("@\"P1D\" * 2", &names));
        assert!(parse_tests("not(\"a\", \"b\")", &names).is_ok());
        assert!(parse_tests("[1..10), > 20", &names).is_ok());
        assert!(parse_expression("1 +", &names).is_err());
    }
}
