//! FEEL subset: tokenizer and recursive-descent parser used for checking (F001, F002)
//! and for overlap analysis of decision tables. Mirrors `feel.py` (evaluation comes later).

use std::collections::HashMap;

use crate::pyutil::{repr_char, strip};

/// A FEEL error: `F002` (syntax) or `F001` (valid FEEL outside the subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeelError {
    Syntax(String),
    Unsupported(String),
}

impl FeelError {
    pub fn code(&self) -> &'static str {
        match self {
            FeelError::Syntax(_) => "F002",
            FeelError::Unsupported(_) => "F001",
        }
    }
    pub fn detail(&self) -> &str {
        match self {
            FeelError::Syntax(s) | FeelError::Unsupported(s) => s,
        }
    }
}

type R<T> = Result<T, FeelError>;

/// The deepest nesting of an expression: parentheses, lists, calls and the other nested
/// parts, and the operators of a chain such as `a + b + c`, each count one level.
pub const MAX_DEPTH: usize = 160;

/// The detail of the syntax error (`F002`) of an expression nested deeper.
pub const TOO_DEEP: &str = "the expression is nested too deeply";

fn syntax<T>(msg: impl Into<String>) -> R<T> {
    Err(FeelError::Syntax(msg.into()))
}
fn unsupported<T>(msg: impl Into<String>) -> R<T> {
    Err(FeelError::Unsupported(msg.into()))
}

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "and", "or", "between", "in", "true", "false", "null",
];
const UNSUPPORTED_WORDS: &[&str] = &[
    "for",
    "some",
    "every",
    "function",
    "instance",
    "satisfies",
    "return",
];
/// (name, min args, max args)
const FUNCTIONS: &[(&str, usize, usize)] = &[
    ("string length", 1, 1),
    ("upper case", 1, 1),
    ("lower case", 1, 1),
    ("contains", 2, 2),
    ("starts with", 2, 2),
    ("ends with", 2, 2),
    ("substring", 2, 3),
    ("abs", 1, 1),
    ("floor", 1, 1),
    ("ceiling", 1, 1),
    ("round half up", 2, 2),
    ("min", 1, 99),
    ("max", 1, 99),
    ("sum", 1, 99),
    ("count", 1, 1),
    ("string", 1, 1),
    ("number", 1, 1),
    ("date", 1, 1),
    ("not", 1, 1),
    // temporal helpers used by schedules of user tasks (language 0.3)
    ("now", 0, 0),
    ("today", 0, 0),
    ("duration", 1, 1),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum T {
    Num(String),
    Str(String),
    Name(String),
    Op(String),
    End,
}

impl T {
    fn value(&self) -> Option<&str> {
        match self {
            T::Num(s) | T::Str(s) | T::Name(s) | T::Op(s) => Some(s),
            T::End => None,
        }
    }
}

fn show(t: &T) -> String {
    match t.value() {
        None => "end of expression".into(),
        Some(v) => format!("'{v}'"),
    }
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric())
}

fn tokenize(text: &str) -> R<Vec<T>> {
    let ch: Vec<char> = text.chars().collect();
    let n = ch.len();
    let mut toks = Vec::new();
    let mut i = 0;
    let s = |a: usize, b: usize| -> String { ch[a..b].iter().collect() };
    while i < n {
        let c = ch[i];
        if matches!(c, ' ' | '\t' | '\r' | '\n') {
            i += 1;
        } else if is_digit(c) || (c == '.' && i + 1 < n && is_digit(ch[i + 1])) {
            let mut j = i;
            while j < n && is_digit(ch[j]) {
                j += 1;
            }
            if j < n && ch[j] == '.' && j + 1 < n && is_digit(ch[j + 1]) {
                j += 1;
                while j < n && is_digit(ch[j]) {
                    j += 1;
                }
            }
            toks.push(T::Num(s(i, j)));
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
                    match ch[j + 1] {
                        'n' => buf.push('\n'),
                        't' => buf.push('\t'),
                        '"' => buf.push('"'),
                        '\\' => buf.push('\\'),
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
            toks.push(T::Str(buf));
            i = j + 1;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < n && (ch[j].is_alphanumeric() || ch[j] == '_') {
                j += 1;
            }
            toks.push(T::Name(s(i, j)));
            i = j;
        } else if c == '*' && i + 1 < n && ch[i + 1] == '*' {
            return unsupported("exponentiation '**'");
        } else if i + 1 < n && ["..", "<=", ">=", "!="].contains(&s(i, i + 2).as_str()) {
            toks.push(T::Op(s(i, i + 2)));
            i += 2;
        } else if "+-*/=<>()[],.".contains(c) {
            toks.push(T::Op(c.to_string()));
            i += 1;
        } else if "{}:?@;".contains(c) {
            return unsupported(format!(
                "'{c}' (contexts, filters and temporal literals are not in the subset)"
            ));
        } else {
            return syntax(format!("unexpected character {}", repr_char(c)));
        }
    }
    toks.push(T::End);
    Ok(toks)
}

/// Parsed FEEL expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Num(String),
    Str(String),
    Bool(bool),
    Null,
    Name(String),
    Path(Box<Ast>, String),
    List(Vec<Ast>),
    Neg(Box<Ast>),
    Bin(String, Box<Ast>, Box<Ast>),
    And(Box<Ast>, Box<Ast>),
    Or(Box<Ast>, Box<Ast>),
    Not(Box<Ast>),
    If(Box<Ast>, Box<Ast>, Box<Ast>),
    Between(Box<Ast>, Box<Ast>, Box<Ast>),
    In(Box<Ast>, Box<Ast>),
    Call(String, Vec<Ast>),
    Range(bool, Box<Ast>, Box<Ast>, bool),
}

/// One positive unary test.
#[derive(Debug, Clone, PartialEq)]
pub enum Positive {
    Cmp(String, Ast),
    Range(bool, Ast, Ast, bool),
    Eq(Ast),
}

/// Parsed unary tests of a decision table input cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Unary {
    Any,
    Tests(Vec<Positive>),
    NotTests(Vec<Positive>),
}

struct P {
    toks: Vec<T>,
    i: usize,
    in_interval: bool,
    /// Nesting of the expression parsed: a bound on the height of its tree.
    depth: usize,
    /// The targets of `in` parsed so far, by position: a target is parsed once more after
    /// its first expression to see whether it is an interval, so without them a nested
    /// `in [ … ]` would be parsed a number of times exponential in its depth.
    targets: HashMap<(usize, bool), (R<Ast>, usize)>,
}

impl P {
    fn peek(&self, k: usize) -> &T {
        let idx = (self.i + k).min(self.toks.len() - 1);
        &self.toks[idx]
    }
    fn next(&mut self) -> T {
        let idx = self.i.min(self.toks.len() - 1);
        self.i += 1;
        self.toks[idx].clone()
    }
    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(0), T::Op(o) if o == op)
    }
    fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(0), T::Name(o) if o == w)
    }
    fn expect_op(&mut self, op: &str) -> R<()> {
        let t = self.next();
        if !matches!(&t, T::Op(o) if o == op) {
            return syntax(format!("expected '{op}', found {}", show(&t)));
        }
        Ok(())
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        let t = self.next();
        if !matches!(&t, T::Name(o) if o == w) {
            return syntax(format!("expected '{w}', found {}", show(&t)));
        }
        Ok(())
    }
    fn expect_end(&self) -> R<()> {
        if *self.peek(0) != T::End {
            return syntax(format!("unexpected {}", show(self.peek(0))));
        }
        Ok(())
    }

    /// Parse a nested part of the expression; the nesting is bounded, so that the
    /// parser and the evaluation stay within the stack.
    fn nested(&mut self, part: fn(&mut Self) -> R<Ast>) -> R<Ast> {
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

    fn expr(&mut self) -> R<Ast> {
        self.nested(Self::expr_body)
    }

    fn expr_body(&mut self) -> R<Ast> {
        if self.at_word("if") {
            self.next();
            let c = self.expr()?;
            self.expect_word("then")?;
            let a = self.expr()?;
            self.expect_word("else")?;
            let b = self.expr()?;
            return Ok(Ast::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        if let T::Name(w) = self.peek(0)
            && UNSUPPORTED_WORDS.contains(&w.as_str())
        {
            return unsupported(format!("'{w}' expressions"));
        }
        self.disjunction()
    }

    fn disjunction(&mut self) -> R<Ast> {
        let base = self.depth;
        let mut left = self.conjunction()?;
        while self.at_word("or") {
            self.deeper()?;
            self.next();
            left = Ast::Or(Box::new(left), Box::new(self.conjunction()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn conjunction(&mut self) -> R<Ast> {
        let base = self.depth;
        let mut left = self.comparison()?;
        while self.at_word("and") {
            self.deeper()?;
            self.next();
            left = Ast::And(Box::new(left), Box::new(self.comparison()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn comparison(&mut self) -> R<Ast> {
        let left = self.additive()?;
        let t = self.peek(0).clone();
        if let T::Op(o) = &t
            && ["=", "!=", "<", "<=", ">", ">="].contains(&o.as_str())
        {
            self.next();
            return Ok(Ast::Bin(
                o.clone(),
                Box::new(left),
                Box::new(self.additive()?),
            ));
        }
        if self.at_word("between") {
            self.next();
            let low = self.additive()?;
            self.expect_word("and")?;
            let high = self.additive()?;
            return Ok(Ast::Between(Box::new(left), Box::new(low), Box::new(high)));
        }
        if self.at_word("in") {
            self.next();
            let target = self.in_target()?;
            return Ok(Ast::In(Box::new(left), Box::new(target)));
        }
        if matches!(&t, T::Name(w) if w == "instance") {
            return unsupported("'instance of'");
        }
        Ok(left)
    }

    fn in_target(&mut self) -> R<Ast> {
        let key = (self.i, self.in_interval);
        if let Some((r, end)) = self.targets.get(&key) {
            self.i = *end;
            return r.clone();
        }
        let r = self.in_target_parsed();
        self.targets.insert(key, (r.clone(), self.i));
        r
    }

    fn in_target_parsed(&mut self) -> R<Ast> {
        if self.at_op("(") || self.at_op("]") {
            return self.interval();
        }
        if self.at_op("[") {
            let (save, depth) = (self.i, self.depth);
            self.next();
            self.expr()?;
            if self.at_op("..") {
                (self.i, self.depth) = (save, depth);
                return self.interval();
            }
            (self.i, self.depth) = (save, depth);
        }
        self.additive()
    }

    fn interval(&mut self) -> R<Ast> {
        let t = self.next();
        let low_closed = match &t {
            T::Op(o) if o == "[" => true,
            T::Op(o) if o == "(" || o == "]" => false,
            _ => return syntax("expected an interval"),
        };
        let low = self.additive()?;
        self.expect_op("..")?;
        self.in_interval = true;
        let high = self.additive();
        self.in_interval = false;
        let high = high?;
        let t = self.next();
        let high_closed = match &t {
            T::Op(o) if o == "]" => true,
            T::Op(o) if o == ")" || o == "[" => false,
            _ => return syntax("expected ']' or ')' closing the interval"),
        };
        Ok(Ast::Range(
            low_closed,
            Box::new(low),
            Box::new(high),
            high_closed,
        ))
    }

    fn additive(&mut self) -> R<Ast> {
        let base = self.depth;
        let mut left = self.multiplicative()?;
        while self.at_op("+") || self.at_op("-") {
            self.deeper()?;
            let op = self.next().value().unwrap_or_default().to_string();
            left = Ast::Bin(op, Box::new(left), Box::new(self.multiplicative()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn multiplicative(&mut self) -> R<Ast> {
        let base = self.depth;
        let mut left = self.unary()?;
        while self.at_op("*") || self.at_op("/") {
            self.deeper()?;
            let op = self.next().value().unwrap_or_default().to_string();
            left = Ast::Bin(op, Box::new(left), Box::new(self.unary()?));
        }
        self.depth = base;
        Ok(left)
    }

    fn unary(&mut self) -> R<Ast> {
        self.nested(Self::unary_body)
    }

    fn unary_body(&mut self) -> R<Ast> {
        if self.at_op("-") {
            self.next();
            return Ok(Ast::Neg(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R<Ast> {
        let base = self.depth;
        let mut node = self.primary()?;
        while self.at_op(".") {
            self.next();
            self.deeper()?;
            let t = self.next();
            let T::Name(name) = t else {
                return syntax("expected a name after '.'");
            };
            node = Ast::Path(Box::new(node), name);
        }
        if self.at_op("[") && !self.in_interval {
            return unsupported("filter expressions");
        }
        self.depth = base;
        Ok(node)
    }

    fn primary(&mut self) -> R<Ast> {
        let t = self.peek(0).clone();
        match &t {
            T::Num(v) => {
                self.next();
                return Ok(Ast::Num(v.clone()));
            }
            T::Str(v) => {
                self.next();
                return Ok(Ast::Str(v.clone()));
            }
            T::Op(o) if o == "(" => {
                self.next();
                let e = self.expr()?;
                self.expect_op(")")?;
                return Ok(e);
            }
            T::Op(o) if o == "[" => {
                self.next();
                let mut items = Vec::new();
                if !self.at_op("]") {
                    items.push(self.expr()?);
                    if self.at_op("..") {
                        return syntax("an interval is allowed only after 'in' or as a unary test");
                    }
                    while self.at_op(",") {
                        self.next();
                        items.push(self.expr()?);
                    }
                }
                self.expect_op("]")?;
                return Ok(Ast::List(items));
            }
            T::Name(w) => {
                if let Some(fname) = self.function_name() {
                    return self.call(&fname);
                }
                self.next();
                return match w.as_str() {
                    "true" => Ok(Ast::Bool(true)),
                    "false" => Ok(Ast::Bool(false)),
                    "null" => Ok(Ast::Null),
                    w if UNSUPPORTED_WORDS.contains(&w) => {
                        unsupported(format!("'{w}' expressions"))
                    }
                    w if KEYWORDS.contains(&w) => syntax(format!("unexpected keyword '{w}'")),
                    w => {
                        if self.at_op("(") {
                            return unsupported(format!("function '{w}'"));
                        }
                        Ok(Ast::Name(w.to_string()))
                    }
                };
            }
            _ => {}
        }
        syntax(format!("unexpected {}", show(&t)))
    }

    fn function_name(&self) -> Option<String> {
        let mut multi: Vec<Vec<&str>> = FUNCTIONS
            .iter()
            .filter(|f| f.0.contains(' '))
            .map(|f| f.0.split(' ').collect())
            .collect();
        multi.sort_by_key(|w| std::cmp::Reverse(w.len()));
        for words in &multi {
            let ok = words
                .iter()
                .enumerate()
                .all(|(k, w)| matches!(self.peek(k), T::Name(n) if n == w));
            if ok && matches!(self.peek(words.len()), T::Op(o) if o == "(") {
                return Some(words.join(" "));
            }
        }
        if let Some(v) = self.peek(0).value()
            && FUNCTIONS.iter().any(|f| f.0 == v && !v.contains(' '))
            && matches!(self.peek(1), T::Op(o) if o == "(")
        {
            return Some(v.to_string());
        }
        None
    }

    fn call(&mut self, fname: &str) -> R<Ast> {
        self.i += fname.split(' ').count();
        self.expect_op("(")?;
        let mut args = Vec::new();
        if !self.at_op(")") {
            args.push(self.expr()?);
            while self.at_op(",") {
                self.next();
                args.push(self.expr()?);
            }
        }
        self.expect_op(")")?;
        let (lo, hi) = FUNCTIONS
            .iter()
            .find(|f| f.0 == fname)
            .map(|f| (f.1, f.2))
            .unwrap_or((0, 99));
        if args.len() < lo || args.len() > hi {
            let range = if lo == hi {
                lo.to_string()
            } else {
                format!("{lo}-{hi}")
            };
            return syntax(format!("{fname}() takes {range} arguments"));
        }
        if fname == "not" {
            return Ok(Ast::Not(Box::new(args.remove(0))));
        }
        Ok(Ast::Call(fname.to_string(), args))
    }

    fn unary_tests(&mut self) -> R<Unary> {
        if self.at_op("-") && *self.peek(1) == T::End {
            self.next();
            return Ok(Unary::Any);
        }
        if self.at_word("not") && matches!(self.peek(1), T::Op(o) if o == "(") {
            let (save, depth) = (self.i, self.depth);
            self.next();
            self.next();
            let tests = self.positive_list()?;
            if self.at_op(")") && *self.peek(1) == T::End {
                self.next();
                return Ok(Unary::NotTests(tests));
            }
            (self.i, self.depth) = (save, depth);
        }
        Ok(Unary::Tests(self.positive_list()?))
    }

    fn positive_list(&mut self) -> R<Vec<Positive>> {
        let mut tests = vec![self.positive()?];
        while self.at_op(",") {
            self.next();
            tests.push(self.positive()?);
        }
        Ok(tests)
    }

    fn positive(&mut self) -> R<Positive> {
        let t = self.peek(0).clone();
        if let T::Op(o) = &t {
            if ["<", "<=", ">", ">=", "=", "!="].contains(&o.as_str()) {
                self.next();
                return Ok(Positive::Cmp(o.clone(), self.additive()?));
            }
            if o == "(" || o == "]" || o == "[" {
                let (save, depth) = (self.i, self.depth);
                match self.interval() {
                    Ok(Ast::Range(a, lo, hi, b)) => return Ok(Positive::Range(a, *lo, *hi, b)),
                    Ok(_) => {}
                    Err(FeelError::Syntax(_)) => (self.i, self.depth) = (save, depth),
                    Err(e) => return Err(e),
                }
                self.in_interval = false;
            }
        }
        Ok(Positive::Eq(self.additive()?))
    }
}

fn parser(text: &str) -> R<P> {
    Ok(P {
        toks: tokenize(text)?,
        i: 0,
        in_interval: false,
        depth: 0,
        targets: HashMap::new(),
    })
}

/// Parse a FEEL expression.
pub fn parse_expr(text: &str) -> R<Ast> {
    if strip(text).is_empty() {
        return syntax("empty expression");
    }
    let mut p = parser(text)?;
    let node = p.expr()?;
    p.expect_end()?;
    Ok(node)
}

/// Parse unary tests (a decision table input cell).
pub fn parse_unary_tests(text: &str) -> R<Unary> {
    if strip(text).is_empty() {
        return syntax("empty cell; use '-' for any value");
    }
    let mut p = parser(text)?;
    let node = p.unary_tests()?;
    p.expect_end()?;
    Ok(node)
}

/// `None` when the expression is in the subset, else (code, detail).
pub fn check_expr(text: &str) -> Option<(&'static str, String)> {
    parse_expr(text)
        .err()
        .map(|e| (e.code(), e.detail().to_string()))
}

pub fn check_unary(text: &str) -> Option<(&'static str, String)> {
    parse_unary_tests(text)
        .err()
        .map(|e| (e.code(), e.detail().to_string()))
}

fn pl_token(text: &str) -> String {
    let t = strip(text);
    if t == "end of expression" {
        return "koniec wyrażenia".into();
    }
    let ch: Vec<char> = t.chars().collect();
    if ch.len() >= 2 && ch[0] == ch[ch.len() - 1] && (ch[0] == '\'' || ch[0] == '"') {
        let inner: String = ch[1..ch.len() - 1].iter().collect();
        return format!("„{inner}”");
    }
    t.to_string()
}

/// Polish version of an F001/F002 detail (`feel.detail_pl`).
pub fn detail_pl(text: &str) -> String {
    let fixed: &[(&str, &str)] = &[
        ("unterminated string", "niezamknięty tekst"),
        ("exponentiation '**'", "potęgowanie „**”"),
        ("'instance of'", "„instance of”"),
        ("expected an interval", "oczekiwano przedziału"),
        (
            "expected ']' or ')' closing the interval",
            "oczekiwano „]” albo „)” zamykającego przedział",
        ),
        ("expected a name after '.'", "oczekiwano nazwy po „.”"),
        ("filter expressions", "wyrażenia filtrujące"),
        (
            "an interval is allowed only after 'in' or as a unary test",
            "przedział jest dozwolony tylko po „in” albo jako test jednoargumentowy",
        ),
        ("empty expression", "puste wyrażenie"),
        (
            "empty cell; use '-' for any value",
            "pusta komórka; dla dowolnej wartości wpisz „-”",
        ),
        ("an interval is not a value", "przedział nie jest wartością"),
        (TOO_DEEP, "wyrażenie jest zbyt głęboko zagnieżdżone"),
    ];
    if let Some((_, pl)) = fixed.iter().find(|f| f.0 == text) {
        return pl.to_string();
    }
    // Python `.` does not match a newline; patterns only apply to single-line details
    let line_ok = |s: &str| !s.is_empty() && !s.contains('\n');
    let ctx_suffix = "' (contexts, filters and temporal literals are not in the subset)";
    if let Some(rest) = text.strip_prefix('\'')
        && let Some(inner) = rest.strip_suffix(ctx_suffix)
        && !inner.contains('\n')
    {
        return format!("„{inner}” (konteksty, filtry i literały czasowe nie należą do podzbioru)");
    }
    if let Some(g) = text.strip_prefix("unexpected character ")
        && line_ok(g)
    {
        return format!("nieoczekiwany znak {}", pl_token(g));
    }
    if let Some(rest) = text.strip_prefix("expected ")
        && let Some(first) = rest.chars().next()
        && let Some(pos) = rest[first.len_utf8()..]
            .find(", found ")
            .map(|p| p + first.len_utf8())
    {
        let (a, b) = (&rest[..pos], &rest[pos + 8..]);
        if line_ok(a) && line_ok(b) {
            return format!("oczekiwano {}, znaleziono {}", pl_token(a), pl_token(b));
        }
    }
    if let Some(g) = text.strip_prefix("unexpected keyword ")
        && line_ok(g)
    {
        return format!("nieoczekiwane słowo kluczowe {}", pl_token(g));
    }
    if text == "unexpected end of expression" {
        return "nieoczekiwany koniec wyrażenia".into();
    }
    if let Some(g) = text.strip_prefix("unexpected ")
        && line_ok(g)
    {
        return format!("nieoczekiwany element {}", pl_token(g));
    }
    if let Some(rest) = text.strip_prefix('\'')
        && let Some(inner) = rest.strip_suffix("' expressions")
        && !inner.contains('\n')
    {
        return format!("wyrażenia „{inner}”");
    }
    if let Some(g) = text.strip_prefix("function ")
        && line_ok(g)
    {
        return format!("funkcja {}", pl_token(g));
    }
    if let Some(stem) = text.strip_suffix(" arguments")
        && !text.contains('\n')
    {
        // greedy (.+)\(\) takes (.+): split at the last "() takes " that leaves both parts non-empty
        let mut idx = stem.rfind("() takes ");
        while let Some(i) = idx {
            let (a, b) = (&stem[..i], &stem[i + 9..]);
            if !a.is_empty() && !b.is_empty() {
                return format!("{a}(): liczba argumentów: {b}");
            }
            idx = stem[..i].rfind("() takes ");
        }
    }
    if let Some(g) = text.strip_prefix("unknown node ")
        && line_ok(g)
    {
        return format!("nieznany węzeł {g}");
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset() {
        assert_eq!(check_expr("cena + 12.5"), None);
        assert_eq!(check_expr("tryb = \"kierownik\""), None);
        assert_eq!(
            check_expr("a > "),
            Some(("F002", "unexpected end of expression".into()))
        );
        assert_eq!(check_expr("for x in y return x").map(|r| r.0), Some("F001"));
        assert_eq!(check_unary("[18..65]"), None);
        assert_eq!(check_unary("not(\"x\")"), None);
        assert_eq!(check_unary("-"), None);
        assert_eq!(check_unary("< 18, > 65"), None);
        assert_eq!(detail_pl("unexpected '>'"), "nieoczekiwany element „>”");
        assert_eq!(
            detail_pl("expected ')', found end of expression"),
            "oczekiwano „)”, znaleziono koniec wyrażenia"
        );
        assert_eq!(
            detail_pl("substring() takes 2-3 arguments"),
            "substring(): liczba argumentów: 2-3"
        );
    }
}
