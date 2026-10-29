//! Evaluation of the FEEL subset: expressions, unary tests and the conversion of results
//! to plain values. Mirrors the evaluator of `feel.py` (`evaluate`, `test`, `eval_text`,
//! `to_python`), including its number semantics (`decimal.Decimal`), its dates and
//! durations (`datetime`) and the exceptions it lets through.

use std::cmp::Ordering;

use indexmap::IndexMap;

use crate::feel::{self, Ast, FeelError, Positive, Unary};
use crate::json::Json;
use crate::pydec::{Decimal, Rounding, is_invalid_operation};
use crate::pytime::{Date, DateTime, Duration};
use crate::pyutil::{PyError, decimal_value, parse_int, repr_str, strip};

use Value as V;

/// A FEEL value.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(Decimal),
    Str(String),
    Date(Date),
    DateTime(DateTime),
    Duration(Duration),
    List(Vec<Value>),
    /// A structured value (a Python dict); its fields are read with `a.b`.
    Context(IndexMap<String, Value>),
}

/// Variables of an evaluation.
pub type Context = IndexMap<String, Value>;

type R<T> = Result<T, PyError>;

/// A FEEL parse error as the Python exception `feel.py` raises.
pub fn feel_error(e: FeelError) -> PyError {
    match e {
        FeelError::Syntax(m) => PyError::new("tok.feel.FeelSyntaxError", m),
        FeelError::Unsupported(m) => PyError::new("tok.feel.FeelUnsupported", m),
    }
}

fn tri(v: Option<bool>) -> Value {
    match v {
        Some(b) => Value::Bool(b),
        None => Value::Null,
    }
}

fn same_type(a: &Value, b: &Value) -> bool {
    if matches!(a, V::Bool(_)) || matches!(b, V::Bool(_)) {
        return matches!(a, V::Bool(_)) && matches!(b, V::Bool(_));
    }
    matches!(
        (a, b),
        (V::Num(_), V::Num(_))
            | (V::Str(_), V::Str(_))
            | (V::Date(_) | V::DateTime(_), V::Date(_) | V::DateTime(_))
            | (V::List(_), V::List(_))
    )
}

/// FEEL `=`: `null = null` is true, `null = x` is false, values of different types give null.
pub fn equal(a: &Value, b: &Value) -> R<Option<bool>> {
    if matches!(a, V::Null) || matches!(b, V::Null) {
        return Ok(Some(matches!(a, V::Null) && matches!(b, V::Null)));
    }
    if !same_type(a, b) {
        return Ok(None);
    }
    Ok(Some(match (a, b) {
        (V::List(x), V::List(y)) => {
            if x.len() != y.len() {
                return Ok(Some(false));
            }
            for (p, q) in x.iter().zip(y) {
                if equal(p, q)? != Some(true) {
                    return Ok(Some(false));
                }
            }
            true
        }
        (V::Num(x), V::Num(y)) => x.py_eq(y)?,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Date(x), V::Date(y)) => x == y,
        (V::DateTime(x), V::DateTime(y)) => x.py_eq(y),
        _ => false,
    }))
}

fn order(a: &Value, b: &Value) -> R<Ordering> {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x.py_cmp(y),
        (V::Str(x), V::Str(y)) => Ok(x.cmp(y)),
        (V::Date(x), V::Date(y)) => Ok(x.cmp(y)),
        (V::DateTime(x), V::DateTime(y)) => x.py_cmp(y),
        _ => Err(PyError::new(
            "TypeError",
            "can't compare datetime.datetime to datetime.date",
        )),
    }
}

/// A comparison `op` (`=`, `!=`, `<`, `<=`, `>`, `>=`); null when it is undefined.
pub fn compare(op: &str, a: &Value, b: &Value) -> R<Option<bool>> {
    match op {
        "=" => return equal(a, b),
        "!=" => return Ok(equal(a, b)?.map(|r| !r)),
        _ => {}
    }
    if matches!(a, Value::Null)
        || matches!(b, Value::Null)
        || !same_type(a, b)
        || matches!(a, Value::Bool(_) | Value::List(_))
    {
        return Ok(None);
    }
    let ord = order(a, b)?;
    Ok(match op {
        "<" => Some(ord == Ordering::Less),
        "<=" => Some(ord != Ordering::Greater),
        ">" => Some(ord == Ordering::Greater),
        ">=" => Some(ord != Ordering::Less),
        _ => None,
    })
}

fn in_range(
    v: &Value,
    low_closed: bool,
    low: &Ast,
    high: &Ast,
    high_closed: bool,
    ctx: &Context,
) -> R<Option<bool>> {
    let lo = evaluate(low, ctx)?;
    let hi = evaluate(high, ctx)?;
    let a = compare(if low_closed { ">=" } else { ">" }, v, &lo)?;
    let b = compare(if high_closed { "<=" } else { "<" }, v, &hi)?;
    Ok(match (a, b) {
        (Some(a), Some(b)) => Some(a && b),
        _ => None,
    })
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Any element of `items` equal to `v` (stops at the first match, like Python's `any`).
fn any_equal(v: &Value, items: &[Value]) -> R<bool> {
    for x in items {
        if equal(v, x)? == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Evaluate an expression against a context of variables.
pub fn evaluate(node: &Ast, ctx: &Context) -> R<Value> {
    Ok(match node {
        Ast::Num(t) => Value::Num(Decimal::parse(t)?),
        Ast::Str(s) => Value::Str(s.clone()),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Null => Value::Null,
        Ast::Name(n) => ctx.get(n).cloned().unwrap_or(Value::Null),
        Ast::Path(base, field) => match evaluate(base, ctx)? {
            Value::Context(m) => m.get(field).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        },
        Ast::List(items) => Value::List(items.iter().map(|x| evaluate(x, ctx)).collect::<R<_>>()?),
        Ast::Neg(x) => match evaluate(x, ctx)? {
            Value::Num(d) => Value::Num(d.neg()?),
            _ => Value::Null,
        },
        Ast::Bin(op, a, b) => {
            let a = evaluate(a, ctx)?;
            let b = evaluate(b, ctx)?;
            if matches!(op.as_str(), "=" | "!=" | "<" | "<=" | ">" | ">=") {
                tri(compare(op, &a, &b)?)
            } else {
                arith(op, &a, &b)?
            }
        }
        Ast::And(a, b) => {
            let a = as_bool(&evaluate(a, ctx)?);
            let b = as_bool(&evaluate(b, ctx)?);
            if a == Some(false) || b == Some(false) {
                Value::Bool(false)
            } else if a == Some(true) && b == Some(true) {
                Value::Bool(true)
            } else {
                Value::Null
            }
        }
        Ast::Or(a, b) => {
            let a = as_bool(&evaluate(a, ctx)?);
            let b = as_bool(&evaluate(b, ctx)?);
            if a == Some(true) || b == Some(true) {
                Value::Bool(true)
            } else if a == Some(false) && b == Some(false) {
                Value::Bool(false)
            } else {
                Value::Null
            }
        }
        Ast::Not(x) => match evaluate(x, ctx)? {
            Value::Bool(b) => Value::Bool(!b),
            _ => Value::Null,
        },
        Ast::If(c, a, b) => {
            if matches!(evaluate(c, ctx)?, Value::Bool(true)) {
                evaluate(a, ctx)?
            } else {
                evaluate(b, ctx)?
            }
        }
        Ast::Between(x, low, high) => {
            let v = evaluate(x, ctx)?;
            let lo = evaluate(low, ctx)?;
            let a = compare(">=", &v, &lo)?;
            let hi = evaluate(high, ctx)?;
            let b = compare("<=", &v, &hi)?;
            match (a, b) {
                (Some(a), Some(b)) => Value::Bool(a && b),
                _ => Value::Null,
            }
        }
        Ast::In(x, target) => {
            let v = evaluate(x, ctx)?;
            if let Ast::Range(lc, lo, hi, hc) = target.as_ref() {
                tri(in_range(&v, *lc, lo, hi, *hc, ctx)?)
            } else {
                match evaluate(target, ctx)? {
                    Value::List(items) => Value::Bool(any_equal(&v, &items)?),
                    tv => tri(equal(&v, &tv)?),
                }
            }
        }
        Ast::Range(..) => {
            return Err(feel_error(FeelError::Syntax(
                "an interval is not a value".into(),
            )));
        }
        Ast::Call(f, args) => {
            let args = args
                .iter()
                .map(|a| evaluate(a, ctx))
                .collect::<R<Vec<_>>>()?;
            call(f, args)?
        }
    })
}

fn arith(op: &str, a: &Value, b: &Value) -> R<Value> {
    if matches!(a, V::Null) || matches!(b, V::Null) {
        return Ok(V::Null);
    }
    if let (V::Str(x), V::Str(y), "+") = (a, b, op) {
        return Ok(V::Str(format!("{x}{y}")));
    }
    if let V::Duration(d) = b
        && matches!(a, V::Date(_) | V::DateTime(_) | V::Duration(_))
        && (op == "+" || op == "-")
    {
        let plus = op == "+";
        return Ok(match a {
            V::Date(x) => V::Date(if plus { x.add(d)? } else { x.sub(d)? }),
            V::DateTime(x) => V::DateTime(if plus { x.add(d)? } else { x.sub(d)? }),
            V::Duration(x) => V::Duration(if plus { x.add(d)? } else { x.sub(d)? }),
            _ => V::Null,
        });
    }
    if let V::Duration(d) = a
        && op == "+"
    {
        match b {
            V::Date(x) => return Ok(V::Date(x.add(d)?)),
            V::DateTime(x) => return Ok(V::DateTime(x.add(d)?)),
            _ => {}
        }
    }
    let (V::Num(x), V::Num(y)) = (a, b) else {
        return Ok(V::Null);
    };
    let res = match op {
        "+" => x.add(y),
        "-" => x.sub(y),
        "*" => x.mul(y),
        "/" => match y.py_eq(&Decimal::zero()) {
            Ok(true) => return Ok(V::Null),
            Ok(false) => x.div(y),
            Err(e) => Err(e),
        },
        _ => return Ok(V::Null),
    };
    match res {
        Ok(v) => Ok(V::Num(v)),
        Err(e) if is_invalid_operation(&e) => Ok(V::Null),
        Err(e) => Err(e),
    }
}

/// `feel.number_to_string`: `12.50` → `12.5`, `3.0` → `3`, `1E+2` → `100`.
pub fn number_to_string(v: &Decimal) -> R<String> {
    if v.is_integral()? {
        return Ok(v
            .quantize(&Decimal::from_i128(1), Rounding::HalfEven)?
            .to_sci_string());
    }
    v.normalize()?.format_f()
}

/// Python `repr()` of a FEEL value (as `str()` of a list shows its items).
pub fn repr_value(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Num(d) => d.repr(),
        Value::Str(s) => repr_str(s),
        Value::Date(d) => d.repr(),
        Value::DateTime(d) => d.repr(),
        Value::Duration(d) => d.repr(),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(repr_value).collect::<Vec<_>>().join(", ")
        ),
        Value::Context(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, x)| format!("{}: {}", repr_str(k), repr_value(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Python slicing `s[start:stop]` of a sequence of `n` characters.
fn py_slice(chars: &[char], start: i128, stop: Option<i128>) -> String {
    let n = chars.len() as i128;
    let norm = |i: i128| {
        if i < 0 {
            i.saturating_add(n).max(0)
        } else {
            i.min(n)
        }
    };
    let a = norm(start);
    let b = stop.map(norm).unwrap_or(n);
    if b <= a {
        return String::new();
    }
    chars[a as usize..b as usize].iter().collect()
}

/// `feel.parse_duration`: an ISO 8601 days-and-time duration (`PnDTnHnMnS`), or `None`.
pub fn parse_duration(text: &str) -> R<Option<Duration>> {
    let t = strip(text);
    if matches!(t, "P" | "PT" | "-P" | "-PT") {
        return Ok(None);
    }
    let ch: Vec<char> = t.chars().collect();
    let mut i = 0;
    let neg = ch.first() == Some(&'-');
    if neg {
        i += 1;
    }
    if ch.get(i) != Some(&'P') {
        return Ok(None);
    }
    i += 1;
    let digits = |i: &mut usize| -> String {
        let mut s = String::new();
        while let Some(d) = ch.get(*i).and_then(|c| decimal_value(*c)) {
            s.push(char::from(b'0' + d as u8));
            *i += 1;
        }
        s
    };
    // an optional group `(\d+)X`; the position is restored when it does not match
    let group = |i: &mut usize, unit: char, fraction: bool| -> Option<String> {
        let save = *i;
        let mut s = digits(i);
        if s.is_empty() {
            *i = save;
            return None;
        }
        if fraction && ch.get(*i) == Some(&'.') {
            let mut j = *i + 1;
            let f = digits(&mut j);
            if !f.is_empty() {
                s = format!("{s}.{f}");
                *i = j;
            }
        }
        if ch.get(*i) == Some(&unit) {
            *i += 1;
            Some(s)
        } else {
            *i = save;
            None
        }
    };
    let days = group(&mut i, 'D', false);
    let (mut hours, mut minutes, mut seconds) = (None, None, None);
    if ch.get(i) == Some(&'T') {
        i += 1;
        hours = group(&mut i, 'H', false);
        minutes = group(&mut i, 'M', false);
        seconds = group(&mut i, 'S', true);
    }
    if i != ch.len() {
        return Ok(None);
    }
    let f = |x: Option<String>| x.and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let td = Duration::from_parts(f(days), f(hours), f(minutes), f(seconds))?;
    Ok(Some(if neg { td.neg()? } else { td }))
}

fn today() -> Date {
    DateTime::now_utc().date
}

fn call(f: &str, mut args: Vec<Value>) -> R<Value> {
    if matches!(f, "min" | "max" | "sum")
        && args.len() == 1
        && matches!(args[0], V::List(_))
        && let Some(V::List(items)) = args.pop()
    {
        args = items;
    }
    let first = args.first().cloned().unwrap_or(V::Null);
    Ok(match f {
        "string length" => match &first {
            V::Str(s) => V::Num(Decimal::from_i128(s.chars().count() as i128)),
            _ => V::Null,
        },
        "upper case" => match &first {
            V::Str(s) => V::Str(s.to_uppercase()),
            _ => V::Null,
        },
        "lower case" => match &first {
            V::Str(s) => V::Str(s.to_lowercase()),
            _ => V::Null,
        },
        "contains" | "starts with" | "ends with" => match (&first, args.get(1)) {
            (V::Str(s), Some(V::Str(x))) => V::Bool(match f {
                "contains" => s.contains(x.as_str()),
                "starts with" => s.starts_with(x.as_str()),
                _ => s.ends_with(x.as_str()),
            }),
            _ => V::Null,
        },
        "substring" => {
            let (V::Str(s), Some(V::Num(start))) = (&first, args.get(1)) else {
                return Ok(V::Null);
            };
            let st = start.to_i128()?;
            let chars: Vec<char> = s.chars().collect();
            let idx = if st > 0 {
                st - 1
            } else {
                (chars.len() as i128).saturating_add(st)
            };
            if args.len() == 3 {
                let Some(V::Num(n)) = args.get(2) else {
                    return Ok(V::Null);
                };
                let n = n.to_i128()?;
                V::Str(py_slice(&chars, idx, Some(idx.saturating_add(n))))
            } else {
                V::Str(py_slice(&chars, idx, None))
            }
        }
        "abs" => match &first {
            V::Num(d) => V::Num(d.abs()?),
            _ => V::Null,
        },
        "floor" => match &first {
            V::Num(d) => V::Num(d.to_integral_value(Rounding::Floor)?),
            _ => V::Null,
        },
        "ceiling" => match &first {
            V::Num(d) => V::Num(d.to_integral_value(Rounding::Ceiling)?),
            _ => V::Null,
        },
        "round half up" => match (&first, args.get(1)) {
            (V::Num(n), Some(V::Num(scale))) => {
                let places = scale.to_i128()?;
                V::Num(n.quantize_scaleb(places.saturating_neg(), Rounding::HalfUp)?)
            }
            _ => V::Null,
        },
        "min" | "max" => {
            if args.is_empty() || !args.iter().all(|a| matches!(a, V::Num(_))) {
                return Ok(V::Null);
            }
            let want = if f == "min" {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            let mut best = 0;
            for k in 1..args.len() {
                if let (V::Num(item), V::Num(cur)) = (&args[k], &args[best])
                    && item.py_cmp(cur)? == want
                {
                    best = k;
                }
            }
            args.swap_remove(best)
        }
        "sum" => {
            if !args.iter().all(|a| matches!(a, V::Num(_))) {
                return Ok(V::Null);
            }
            let mut total = Decimal::zero();
            for a in &args {
                if let V::Num(d) = a {
                    total = total.add(d)?;
                }
            }
            V::Num(total)
        }
        "count" => match &first {
            V::List(items) => V::Num(Decimal::from_i128(items.len() as i128)),
            _ => V::Null,
        },
        "string" => match &first {
            V::Null => V::Null,
            V::Bool(b) => V::Str(if *b { "true" } else { "false" }.into()),
            V::Num(d) => V::Str(number_to_string(d)?),
            V::Date(d) => V::Str(d.isoformat()),
            V::DateTime(d) => V::Str(d.isoformat()),
            V::Str(s) => V::Str(s.clone()),
            V::Duration(d) => V::Str(d.to_str()),
            v @ (V::List(_) | V::Context(_)) => V::Str(repr_value(v)),
        },
        "number" => match &first {
            V::Str(s) => match Decimal::parse(strip(s)) {
                Ok(d) => V::Num(d),
                Err(e) if is_invalid_operation(&e) => V::Null,
                Err(e) => return Err(e),
            },
            _ => V::Null,
        },
        "now" => V::DateTime(DateTime::now_utc()),
        "today" => V::Date(today()),
        "duration" => match &first {
            V::Str(s) => match parse_duration(s)? {
                Some(d) => V::Duration(d),
                None => V::Null,
            },
            _ => V::Null,
        },
        "date" => match &first {
            V::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                if chars.len() != 10 {
                    return Ok(V::Null);
                }
                let part = |a: usize, b: usize| parse_int(&chars[a..b].iter().collect::<String>());
                match (part(0, 4), part(5, 7), part(8, 10)) {
                    (Ok(y), Ok(m), Ok(d)) => match crate::pytime::Date::new(y, m, d) {
                        Ok(d) => V::Date(d),
                        Err(_) => V::Null,
                    },
                    _ => V::Null,
                }
            }
            _ => V::Null,
        },
        _ => V::Null,
    })
}

/// Apply parsed unary tests to a value: true, false or null.
pub fn test(unary: &Unary, value: &Value, ctx: &Context) -> R<Option<bool>> {
    let (tests, negated) = match unary {
        Unary::Any => return Ok(Some(true)),
        Unary::Tests(t) => (t, false),
        Unary::NotTests(t) => (t, true),
    };
    let results = tests
        .iter()
        .map(|p| positive(p, value, ctx))
        .collect::<R<Vec<_>>>()?;
    let hit = results.contains(&Some(true));
    if negated {
        if hit {
            return Ok(Some(false));
        }
        if results.contains(&None) {
            return Ok(None);
        }
        return Ok(Some(true));
    }
    Ok(Some(hit))
}

fn positive(p: &Positive, value: &Value, ctx: &Context) -> R<Option<bool>> {
    match p {
        Positive::Cmp(op, node) => compare(op, value, &evaluate(node, ctx)?),
        Positive::Range(lc, lo, hi, hc) => in_range(value, *lc, lo, hi, *hc, ctx),
        Positive::Eq(node) => {
            let target = evaluate(node, ctx)?;
            match (&target, value) {
                (Value::List(items), v) if !matches!(v, Value::List(_)) => {
                    Ok(Some(any_equal(value, items)?))
                }
                _ => equal(value, &target),
            }
        }
    }
}

/// Parse and evaluate an expression in one call.
pub fn eval_text(text: &str, ctx: &Context) -> R<Value> {
    let node = feel::parse_expr(text).map_err(feel_error)?;
    evaluate(&node, ctx)
}

/// A plain value as `feel.to_python` returns it (numbers become int or float, dates
/// ISO text); durations stay durations.
#[derive(Debug, Clone)]
pub enum PyValue {
    None,
    Bool(bool),
    /// An integer of any size, as decimal digits.
    Int(String),
    Float(f64),
    Str(String),
    Duration(Duration),
    List(Vec<PyValue>),
    Dict(Vec<(String, PyValue)>),
}

/// `feel.to_python`.
pub fn to_python(v: &Value) -> R<PyValue> {
    Ok(match v {
        Value::Null => PyValue::None,
        Value::Bool(b) => PyValue::Bool(*b),
        Value::Num(d) => {
            if d.is_integral()? {
                PyValue::Int(d.int_text()?)
            } else {
                PyValue::Float(d.to_f64()?)
            }
        }
        Value::Str(s) => PyValue::Str(s.clone()),
        Value::Date(d) => PyValue::Str(d.isoformat()),
        Value::DateTime(d) => PyValue::Str(d.isoformat()),
        Value::Duration(d) => PyValue::Duration(*d),
        Value::List(items) => PyValue::List(items.iter().map(to_python).collect::<R<_>>()?),
        Value::Context(m) => PyValue::Dict(
            m.iter()
                .map(|(k, x)| Ok((k.clone(), to_python(x)?)))
                .collect::<R<_>>()?,
        ),
    })
}

/// Python's default limit of digits when an int is converted to text (`sys.int_info`).
const INT_MAX_STR_DIGITS: usize = 4300;

/// Exact integer digits of an integral float.
fn float_int_text(f: f64) -> String {
    if f == 0.0 {
        return "0".into();
    }
    format!("{f:.0}")
}

impl PyValue {
    /// `json.dumps(value, ensure_ascii=False, default=str)` as a JSON tree.
    /// Integers longer than 4300 digits raise `ValueError`, as Python's int-to-text limit.
    pub fn to_json(&self) -> R<Json> {
        Ok(match self {
            PyValue::None => Json::Null,
            PyValue::Bool(b) => Json::Bool(*b),
            PyValue::Int(t) => {
                if t.trim_start_matches('-').len() > INT_MAX_STR_DIGITS {
                    return Err(PyError::new(
                        "ValueError",
                        format!(
                            "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion; \
                             use sys.set_int_max_str_digits() to increase the limit"
                        ),
                    ));
                }
                Json::Num(t.clone())
            }
            PyValue::Float(f) => Json::Float(*f),
            PyValue::Str(s) => Json::Str(s.clone()),
            PyValue::Duration(d) => Json::Str(d.to_str()),
            PyValue::List(items) => {
                Json::Arr(items.iter().map(PyValue::to_json).collect::<R<_>>()?)
            }
            PyValue::Dict(pairs) => Json::Obj(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), v.to_json()?)))
                    .collect::<R<_>>()?,
            ),
        })
    }

    /// The numeric value of a bool, int or float, for Python's cross-type `==`.
    fn number_text(&self) -> Option<String> {
        match self {
            PyValue::Bool(b) => Some(if *b { "1" } else { "0" }.into()),
            PyValue::Int(t) => Some(t.clone()),
            PyValue::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(float_int_text(*f)),
            _ => None,
        }
    }

    /// Python `==` (`True == 1 == 1.0`, dicts compared without regard to order).
    pub fn py_eq(&self, other: &PyValue) -> bool {
        use PyValue::*;
        match (self, other) {
            (None, None) => true,
            (Str(a), Str(b)) => a == b,
            (Duration(a), Duration(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (List(a), List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y)),
            (Dict(a), Dict(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v.py_eq(v2)))
            }
            (Bool(_) | Int(_) | Float(_), Bool(_) | Int(_) | Float(_)) => {
                match (self.number_text(), other.number_text()) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(text: &str) -> String {
        match eval_text(text, &Context::new()) {
            Ok(v) => repr_value(&v),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn expressions() {
        assert_eq!(ev("0.1 + 0.2"), "Decimal('0.3')");
        assert_eq!(ev("1 / 3"), "Decimal('0.3333333333333333333333333333')");
        assert_eq!(ev("1 / 0"), "None");
        assert_eq!(ev("string(12.50)"), "'12.5'");
        assert_eq!(ev("\"a\" + \"b\""), "'ab'");
        assert_eq!(ev("round half up(2.345, 2)"), "Decimal('2.35')");
        assert_eq!(ev("substring(\"abcdef\", -2)"), "'ef'");
        assert_eq!(ev("5 in [1..10]"), "True");
        assert_eq!(ev("null = null"), "True");
        assert_eq!(ev("1 < \"a\""), "None");
        assert_eq!(
            ev("date(\"2027-01-15\") + duration(\"P1D\")"),
            "datetime.date(2027, 1, 16)"
        );
        assert_eq!(ev("string([1, \"a\"])"), "\"[Decimal('1'), 'a']\"");
        assert_eq!(
            ev("x ** 2"),
            "tok.feel.FeelUnsupported: exponentiation '**'"
        );
    }

    #[test]
    fn unary_tests() {
        let u = feel::parse_unary_tests("not(\"a\", \"b\")").expect("cell");
        let ctx = Context::new();
        assert_eq!(test(&u, &Value::Str("c".into()), &ctx), Ok(Some(true)));
        assert_eq!(test(&u, &Value::Num(Decimal::zero()), &ctx), Ok(None));
    }
}
