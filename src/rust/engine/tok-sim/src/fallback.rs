//! Expressions and decisions as `sim.py` evaluates them: FEEL first (`tok-core`), and
//! when FEEL raises any error, the small built-in evaluator `_Fallback` and the
//! built-in decision table subset of `eval_decision`.

use indexmap::IndexMap;

use tok_core::Document;
use tok_core::dmn;
use tok_core::feel;
use tok_core::feel_eval::{self as fe, PyValue};
use tok_core::pyutil::{PyError, decimal_value, is_space, strip};
use tok_core::simcheck::split_args;

use crate::pyval;

type R<T> = Result<T, PyError>;

/// Case variables (or a multi-instance context): name → plain value.
pub type Vars = IndexMap<String, PyValue>;

fn value_error(msg: impl Into<String>) -> PyError {
    PyError::new("ValueError", msg)
}

thread_local! {
    /// The simulated document declares language 0.7.
    static FEEL07: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While alive, expressions are evaluated as FEEL of language 0.7 (when `on`).
pub struct Feel07(bool);

impl Feel07 {
    pub fn set(on: bool) -> Feel07 {
        Feel07(FEEL07.with(|f| f.replace(on)))
    }
}

impl Drop for Feel07 {
    fn drop(&mut self) {
        FEEL07.with(|f| f.set(self.0));
    }
}

/// An expression of a 0.7 document: FEEL at conformance level 3; an expression that
/// cannot be evaluated is `null`.
fn feel07_value(text: &str, ctx: &Vars) -> R<PyValue> {
    use tok_core::feel3::{Env, Eval, Names, Types, parse_expression};
    let names = Names::builtins();
    let e = parse_expression(text, &names)
        .map_err(|e| PyError::new("tok.feel.FeelSyntaxError", e.detail().to_string()))?;
    let vars = ctx
        .iter()
        .map(|(k, v)| (k.clone(), tok_core::drg::values::from_plain(v)))
        .collect();
    let types = Types::default();
    let v = Eval::new(&types).eval(&e, &Env::root(vars));
    Ok(tok_core::drg::values::to_plain(&v))
}

/// Evaluate an expression (`sim.eval_expr`): FEEL, then the built-in subset.
pub fn eval_expr(text: &str, ctx: &Vars) -> R<PyValue> {
    if FEEL07.with(|f| f.get()) {
        return feel07_value(text, ctx);
    }
    if let Ok(v) = feel_value(text, ctx) {
        return Ok(v);
    }
    Fallback::new(text)?.parse(ctx)
}

fn feel_value(text: &str, ctx: &Vars) -> R<PyValue> {
    let node = feel::parse_expr(text).map_err(fe::feel_error)?;
    let ctx = pyval::feel_context(ctx)?;
    fe::to_python(&fe::evaluate(&node, &ctx)?)
}

/// Evaluate a decision table (`sim.eval_decision`): `tok.dmn`, or when it raises, the
/// built-in subset (hit policies unique, first, any, priority and collect).
pub fn eval_decision(doc: &Document, decision_id: &str, ctx: &Vars) -> R<PyValue> {
    let Some(dec) = doc.decisions.get(decision_id) else {
        return Ok(PyValue::None);
    };
    if doc.lang_07() {
        // language 0.7: the decision with what it requires; the case variables are the input data
        return tok_core::drg::values::evaluate_plain(doc, decision_id, ctx);
    }
    if let Ok(v) = pyval::feel_context(ctx).and_then(|c| dmn::evaluate_decision(dec, &c)) {
        return Ok(v);
    }
    let hit = dec.hit();
    let mut matches = Vec::new();
    for rule in &dec.rules {
        let mut ok = true;
        for ((expr, _), cell) in dec.inputs.iter().zip(&rule.inputs) {
            let value = eval_expr(expr, ctx)?;
            if !fallback_test(cell, &value, ctx)? {
                ok = false;
                break;
            }
        }
        if ok {
            let mut out: Vec<(String, PyValue)> = Vec::new();
            for ((name, _), cell) in dec.outputs.iter().zip(&rule.outputs) {
                let v = eval_expr(cell, ctx)?;
                match out.iter_mut().find(|(k, _)| k == name) {
                    Some(slot) => slot.1 = v,
                    None => out.push((name.clone(), v)),
                }
            }
            if out.len() == 1 {
                let first = &dec.outputs[0].0;
                let v = out
                    .iter()
                    .find(|(k, _)| k == first)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(PyValue::None);
                matches.push(v);
            } else {
                matches.push(PyValue::Dict(out));
            }
            if ["unique", "first", "any", "priority"].contains(&hit) {
                break;
            }
        }
    }
    if hit == "collect" {
        return Ok(PyValue::List(matches));
    }
    Ok(matches.into_iter().next().unwrap_or(PyValue::None))
}

/// `^([\[\(\]])(.+)\.\.(.+)([\]\)\[])$`: (opening bracket, low, high, closing bracket).
fn range_cell(p: &str) -> Option<(char, &str, &str, char)> {
    let first = p.chars().next()?;
    let last = p.chars().next_back()?;
    if !matches!(first, '[' | '(' | ']') || !matches!(last, ']' | ')' | '[') || p.len() < 2 {
        return None;
    }
    let middle = &p[first.len_utf8()..p.len() - last.len_utf8()];
    if middle.contains('\n') {
        return None;
    }
    // the low part is greedy: the last `..` that leaves a non-empty high part
    let mut best = None;
    let bytes = middle.as_bytes();
    for i in 1..middle.len() {
        if bytes[i] == b'.' && bytes.get(i + 1) == Some(&b'.') && i + 2 < middle.len() {
            best = Some(i);
        }
    }
    let i = best?;
    Some((first, &middle[..i], &middle[i + 2..], last))
}

/// `^(<=|>=|<|>|!=|=)\s*(.+)$`: (operator, operand).
fn comparison_cell(p: &str) -> Option<(&'static str, &str)> {
    let op = ["<=", ">=", "<", ">", "!=", "="]
        .into_iter()
        .find(|op| p.starts_with(op))?;
    let rest = p[op.len()..].trim_start_matches(is_space);
    if rest.is_empty() || rest.contains('\n') {
        return None;
    }
    Some((op, rest))
}

/// `_fallback_test`: does a value pass the unary tests of a cell?
fn fallback_test(cell: &str, value: &PyValue, ctx: &Vars) -> R<bool> {
    let c = strip(cell);
    if c == "-" {
        return Ok(true);
    }
    for part in split_args(c) {
        let p = strip(&part);
        if let Some((open, low, high, close)) = range_cell(p) {
            let lo = eval_expr(low, ctx)?;
            let hi = eval_expr(high, ctx)?;
            let lo_ok = pyval::compare(if open == '[' { ">=" } else { ">" }, value, &lo)?;
            let hi_ok = pyval::compare(if close == ']' { "<=" } else { "<" }, value, &hi)?;
            if !matches!(value, PyValue::None) && lo_ok && hi_ok {
                return Ok(true);
            }
            continue;
        }
        if let Some((op, rest)) = comparison_cell(p) {
            let other = eval_expr(rest, ctx)?;
            if fallback_compare(op, value, &other) {
                return Ok(true);
            }
            continue;
        }
        if pyval::eq(value, &eval_expr(p, ctx)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `_Fallback.compare`: comparisons that never raise.
fn fallback_compare(op: &str, a: &PyValue, b: &PyValue) -> bool {
    match op {
        "=" => pyval::eq(a, b),
        "!=" => !pyval::eq(a, b),
        _ => {
            if matches!(a, PyValue::None) || matches!(b, PyValue::None) {
                return false;
            }
            pyval::compare(op, a, b).unwrap_or(false)
        }
    }
}

/// A token of `_Fallback`.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Op(&'static str),
    Name(String),
}

impl Tok {
    /// The token's value as `self.take()[1]` gives it, as a dict key.
    fn key(&self) -> Option<&str> {
        match self {
            Tok::Num(_) => None,
            Tok::Str(s) | Tok::Name(s) => Some(s),
            Tok::Op(o) => Some(o),
        }
    }
}

const OPS2: [&str; 3] = ["<=", ">=", "!="];
const OPS1: [&str; 11] = ["-", "+", "*", "/", "(", ")", "=", "<", ">", ".", ","];

/// `_Fallback`: literals, names (`a.b`), `+ - * /`, comparisons, `and`, `or`, `not()`
/// and parentheses over plain values.
struct Fallback {
    toks: Vec<Tok>,
    i: usize,
}

/// `\d+(?:\.\d+)?` at the start of `s`: the matched length in bytes.
fn match_number(s: &str) -> Option<usize> {
    let digits = |t: &str| -> usize {
        t.char_indices()
            .find(|(_, c)| decimal_value(*c).is_none())
            .map(|(i, _)| i)
            .unwrap_or(t.len())
    };
    let n = digits(s);
    if n == 0 {
        return None;
    }
    if let Some(rest) = s[n..].strip_prefix('.') {
        let m = digits(rest);
        if m > 0 {
            return Some(n + 1 + m);
        }
    }
    Some(n)
}

/// `"(?:[^"\\]|\\.)*"` at the start of `s` (`.` does not match a newline).
fn match_string(s: &str) -> Option<usize> {
    let mut it = s.char_indices();
    if it.next()?.1 != '"' {
        return None;
    }
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return Some(i + 1),
            '\\' => match it.next() {
                Some((_, n)) if n != '\n' => {}
                _ => return None,
            },
            _ => {}
        }
    }
    None
}

fn match_name(s: &str) -> Option<usize> {
    let mut it = s.char_indices();
    let (_, c) = it.next()?;
    if !(c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    Some(
        it.find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
            .map(|(i, _)| i)
            .unwrap_or(s.len()),
    )
}

fn number_value(t: &str) -> f64 {
    let ascii: String = t
        .chars()
        .map(|c| match decimal_value(c) {
            Some(d) => char::from(b'0' + d as u8),
            None => c,
        })
        .collect();
    ascii.parse::<f64>().unwrap_or(f64::NAN)
}

impl Fallback {
    fn new(text: &str) -> R<Self> {
        let text = strip(text);
        let mut toks = Vec::new();
        let mut pos = 0;
        let err = || value_error(format!("cannot read expression: {text}"));
        while pos < text.len() {
            let rest = &text[pos..];
            let ws = rest.len() - rest.trim_start_matches(is_space).len();
            let s = &rest[ws..];
            if let Some(n) = match_number(s) {
                toks.push(Tok::Num(number_value(&s[..n])));
                pos += ws + n;
            } else if let Some(n) = match_string(s) {
                let body = &s[1..n - 1];
                toks.push(Tok::Str(body.replace("\\\"", "\"").replace("\\\\", "\\")));
                pos += ws + n;
            } else if let Some(op) = OPS2.into_iter().chain(OPS1).find(|op| s.starts_with(op)) {
                toks.push(Tok::Op(op));
                pos += ws + op.len();
            } else if let Some(n) = match_name(s) {
                toks.push(Tok::Name(s[..n].to_string()));
                pos += ws + n;
            } else {
                return Err(err());
            }
        }
        Ok(Fallback { toks, i: 0 })
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn take(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }

    fn at_name(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Tok::Name(n)) if n == name)
    }

    fn parse(&mut self, ctx: &Vars) -> R<PyValue> {
        let v = self.or(ctx)?;
        if self.i != self.toks.len() {
            return Err(value_error("unexpected token"));
        }
        Ok(v)
    }

    fn or(&mut self, ctx: &Vars) -> R<PyValue> {
        let mut v = self.and(ctx)?;
        while self.at_name("or") {
            self.take();
            let r = self.and(ctx)?;
            v = PyValue::Bool(pyval::truthy(&v) || pyval::truthy(&r));
        }
        Ok(v)
    }

    fn and(&mut self, ctx: &Vars) -> R<PyValue> {
        let mut v = self.cmp(ctx)?;
        while self.at_name("and") {
            self.take();
            let r = self.cmp(ctx)?;
            v = PyValue::Bool(pyval::truthy(&v) && pyval::truthy(&r));
        }
        Ok(v)
    }

    fn cmp(&mut self, ctx: &Vars) -> R<PyValue> {
        let v = self.add(ctx)?;
        if let Some(Tok::Op(op)) = self.peek()
            && ["=", "!=", "<", "<=", ">", ">="].contains(op)
        {
            let op = *op;
            self.take();
            let r = self.add(ctx)?;
            return Ok(PyValue::Bool(fallback_compare(op, &v, &r)));
        }
        Ok(v)
    }

    fn add(&mut self, ctx: &Vars) -> R<PyValue> {
        let mut v = self.mul(ctx)?;
        while self.at_op("+") || self.at_op("-") {
            let op = if self.at_op("+") { "+" } else { "-" };
            self.take();
            let r = self.mul(ctx)?;
            v = pyval::binary(op, &v, &r)?;
        }
        Ok(v)
    }

    fn mul(&mut self, ctx: &Vars) -> R<PyValue> {
        let mut v = self.unary(ctx)?;
        while self.at_op("*") || self.at_op("/") {
            let op = if self.at_op("*") { "*" } else { "/" };
            self.take();
            let r = self.unary(ctx)?;
            v = pyval::binary(op, &v, &r)?;
        }
        Ok(v)
    }

    fn unary(&mut self, ctx: &Vars) -> R<PyValue> {
        if self.at_op("-") {
            self.take();
            let v = self.unary(ctx)?;
            return pyval::negate(&v);
        }
        self.primary(ctx)
    }

    fn primary(&mut self, ctx: &Vars) -> R<PyValue> {
        match self.take() {
            Some(Tok::Num(x)) => Ok(PyValue::Float(x)),
            Some(Tok::Str(s)) => Ok(PyValue::Str(s)),
            Some(Tok::Op("(")) => {
                let r = self.or(ctx)?;
                self.take();
                Ok(r)
            }
            Some(Tok::Name(v)) => {
                match v.as_str() {
                    "true" => return Ok(PyValue::Bool(true)),
                    "false" => return Ok(PyValue::Bool(false)),
                    "null" => return Ok(PyValue::None),
                    "not" if self.at_op("(") => {
                        self.take();
                        let r = self.or(ctx)?;
                        self.take();
                        return Ok(PyValue::Bool(!pyval::truthy(&r)));
                    }
                    _ => {}
                }
                let mut val = ctx.get(&v).cloned().unwrap_or(PyValue::None);
                while self.at_op(".") {
                    self.take();
                    let key = self.take();
                    val = match (&val, key.as_ref().and_then(Tok::key)) {
                        (PyValue::Dict(pairs), Some(k)) => pairs
                            .iter()
                            .find(|(name, _)| name == k)
                            .map(|(_, x)| x.clone())
                            .unwrap_or(PyValue::None),
                        _ => PyValue::None,
                    };
                }
                Ok(val)
            }
            _ => Err(value_error("unexpected token")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, vars: &[(&str, PyValue)]) -> String {
        let ctx: Vars = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        match Fallback::new(text).and_then(|mut f| f.parse(&ctx)) {
            Ok(v) => format!("{v:?}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn fallback_subset() {
        assert_eq!(run("1 + 2 * 3", &[]), "Float(7.0)");
        assert_eq!(run("x ** 2", &[]), "ValueError: unexpected token");
        assert_eq!(
            run("{a: 1}", &[]),
            "ValueError: cannot read expression: {a: 1}"
        );
        assert_eq!(
            run("for + 1", &[("for", PyValue::Int("2".into()))]),
            "Float(3.0)"
        );
        assert_eq!(
            run("s + 1", &[("s", PyValue::Str("a".into()))]),
            "TypeError: can only concatenate str (not \"float\") to str"
        );
        assert_eq!(run("not(a) and \"x\" = \"x\"", &[]), "Bool(true)");
        assert_eq!(run("(1", &[]), "ValueError: unexpected token");
        assert_eq!(
            run(
                "o.k",
                &[("o", PyValue::Dict(vec![("k".into(), PyValue::Bool(true))]))]
            ),
            "Bool(true)"
        );
    }

    #[test]
    fn cells() {
        assert_eq!(range_cell("[1..5]"), Some(('[', "1", "5", ']')));
        assert_eq!(range_cell("]1...5["), Some((']', "1.", "5", '[')));
        assert_eq!(range_cell("[1..]"), None);
        assert_eq!(comparison_cell(">= 5"), Some((">=", "5")));
        assert_eq!(comparison_cell("<"), None);
    }
}
