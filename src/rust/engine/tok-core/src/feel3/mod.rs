//! FEEL at DMN conformance level 3 (DMN 1.5 chapter 10), the expression language of
//! documents of language 0.7: values, parser, evaluator and the standard function
//! library. Documents of earlier versions keep the FEEL subset of `feel` and
//! `feel_eval`, whose results do not change.

pub mod builtins;
pub mod eval;
pub mod num;
pub mod parse;
pub mod regex;
pub mod temporal;

use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

use indexmap::IndexMap;

pub use eval::{Env, Eval, TypeDef, Types};
pub use num::Num;
pub use parse::{Expr, Names, Tests, Type, parse_expression, parse_tests, parse_type};
use temporal::{Date, DateTime, DtDuration, Time, YmDuration};

/// Entries of a context value, in order.
pub type Map = IndexMap<String, Value>;

/// A FEEL value.
#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(Num),
    Str(String),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Dt(DtDuration),
    Ym(YmDuration),
    List(Rc<Vec<Value>>),
    Context(Rc<Map>),
    Range(Rc<Range>),
    Func(Rc<Func>),
}

/// A range value: an absent end is unbounded (`< 10`).
#[derive(Debug, Clone)]
pub struct Range {
    pub low: Option<Value>,
    pub low_closed: bool,
    pub high: Option<Value>,
    pub high_closed: bool,
}

/// A parameter of a function.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
}

/// The body of a function defined by a boxed expression (a business knowledge model or a
/// function definition of a context entry).
pub trait Body {
    /// Evaluate the body with the arguments bound in `env`.
    fn call(&self, ev: &Eval, env: &Rc<Env>) -> Value;
}

/// A function value.
pub enum Func {
    Builtin(&'static str),
    Lambda {
        params: Vec<Param>,
        body: Rc<Expr>,
        env: Rc<Env>,
    },
    Boxed {
        params: Vec<Param>,
        body: Rc<dyn Body>,
        env: Rc<Env>,
        /// The declared result type.
        result: Option<Type>,
    },
}

impl fmt::Debug for Func {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Func::Builtin(n) => write!(f, "builtin {n}"),
            Func::Lambda { params, .. } | Func::Boxed { params, .. } => {
                let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                write!(f, "function({})", names.join(", "))
            }
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.text())
    }
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }
    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Rc::new(items))
    }
    pub fn context(m: Map) -> Value {
        Value::Context(Rc::new(m))
    }
    pub fn int(n: i64) -> Value {
        Value::Num(Num::int(n))
    }
    pub fn num(n: Option<Num>) -> Value {
        n.map(Value::Num).unwrap_or(Value::Null)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The name of the type of the value.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "Null",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Date(_) => "date",
            Value::Time(_) => "time",
            Value::DateTime(_) => "date and time",
            Value::Dt(_) => "days and time duration",
            Value::Ym(_) => "years and months duration",
            Value::List(_) => "list",
            Value::Context(_) => "context",
            Value::Range(_) => "range",
            Value::Func(_) => "function",
        }
    }

    /// The text form of the value (`string(x)` for scalars; lists and contexts in FEEL).
    pub fn text(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.to_text(),
            Value::Str(s) => s.clone(),
            Value::Date(d) => d.text(),
            Value::Time(t) => t.text(),
            Value::DateTime(d) => d.text(),
            Value::Dt(d) => d.text(),
            Value::Ym(d) => d.text(),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(Value::literal).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Context(m) => {
                let parts: Vec<String> = m
                    .iter()
                    .map(|(k, v)| format!("{}: {}", context_key(k), v.literal()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            Value::Range(r) => {
                let end = |v: &Option<Value>| v.as_ref().map(Value::literal).unwrap_or_default();
                match (&r.low, &r.high) {
                    (None, Some(_)) => format!(
                        "{} {}",
                        if r.high_closed { "<=" } else { "<" },
                        end(&r.high)
                    ),
                    (Some(_), None) => {
                        format!("{} {}", if r.low_closed { ">=" } else { ">" }, end(&r.low))
                    }
                    _ => format!(
                        "{}{}..{}{}",
                        if r.low_closed { "[" } else { "(" },
                        end(&r.low),
                        end(&r.high),
                        if r.high_closed { "]" } else { ")" }
                    ),
                }
            }
            Value::Func(f) => format!("{f:?}"),
        }
    }

    /// The value written as a FEEL literal (strings quoted, temporals with `@`).
    pub fn literal(&self) -> String {
        match self {
            Value::Str(s) => quote(s),
            Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::Dt(_) | Value::Ym(_) => {
                format!("@{}", quote(&self.text()))
            }
            _ => self.text(),
        }
    }
}

fn context_key(k: &str) -> String {
    let plain = !k.is_empty()
        && k.chars().enumerate().all(|(i, c)| {
            if i == 0 {
                c.is_alphabetic() || c == '_'
            } else {
                c.is_alphanumeric() || c == '_'
            }
        });
    if plain { k.to_string() } else { quote(k) }
}

/// A FEEL string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// FEEL equality (`=`): `None` when the values cannot be compared (different types).
pub fn equal(a: &Value, b: &Value) -> Option<bool> {
    use Value as V;
    Some(match (a, b) {
        (V::Null, V::Null) => true,
        (V::Null, _) | (_, V::Null) => false,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Num(x), V::Num(y)) => x.eq(y),
        (V::Str(x), V::Str(y)) => x == y,
        (V::Date(x), V::Date(y)) => x == y,
        (V::Time(x), V::Time(y)) => x.equals(y),
        (V::DateTime(x), V::DateTime(y)) => x.equals(y),
        (V::Dt(x), V::Dt(y)) => x == y,
        (V::Ym(x), V::Ym(y)) => x == y,
        (V::List(x), V::List(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            for (p, q) in x.iter().zip(y.iter()) {
                if equal(p, q) != Some(true) {
                    return Some(false);
                }
            }
            true
        }
        (V::Context(x), V::Context(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            for (k, v) in x.iter() {
                match y.get(k) {
                    Some(w) if equal(v, w) == Some(true) => {}
                    _ => return Some(false),
                }
            }
            true
        }
        (V::Range(x), V::Range(y)) => {
            let end = |p: &Option<Value>, q: &Option<Value>| match (p, q) {
                (None, None) => true,
                (Some(p), Some(q)) => equal(p, q) == Some(true),
                _ => false,
            };
            end(&x.low, &y.low)
                && end(&x.high, &y.high)
                && x.low_closed == y.low_closed
                && x.high_closed == y.high_closed
        }
        (V::Func(x), V::Func(y)) => Rc::ptr_eq(x, y),
        _ => return None,
    })
}

/// FEEL ordering of two values of the same ordered type.
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => Some(x.cmp(y)),
        (V::Str(x), V::Str(y)) => Some(x.cmp(y)),
        (V::Date(x), V::Date(y)) => Some(x.days().cmp(&y.days())),
        (V::Time(x), V::Time(y)) => x.compare(y),
        (V::DateTime(x), V::DateTime(y)) => x.compare(y),
        (V::Dt(x), V::Dt(y)) => Some(x.cmp(y)),
        (V::Ym(x), V::Ym(y)) => Some(x.cmp(y)),
        _ => None,
    }
}
