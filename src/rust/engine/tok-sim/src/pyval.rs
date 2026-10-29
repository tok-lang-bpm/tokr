//! Python semantics of the plain values the simulator keeps in case variables
//! (`feel.to_python` results and distribution samples): truth value, `==`, ordering
//! comparisons and the arithmetic operators, with the exceptions Python raises.
//!
//! Integers are exact within the range of `i128`; integers beyond it (possible only as
//! results of FEEL arithmetic with more than 38 digits) take part in arithmetic as floats.

use std::cmp::Ordering;

use tok_core::feel_eval::{Context, PyValue, Value};
use tok_core::pydec::Decimal;
use tok_core::pytime::Duration;
use tok_core::pyutil::{PyError, float_repr};

type R<T> = Result<T, PyError>;

/// Python's class name of a value, as error messages print it.
pub fn type_name(v: &PyValue) -> &'static str {
    match v {
        PyValue::None => "NoneType",
        PyValue::Bool(_) => "bool",
        PyValue::Int(_) => "int",
        PyValue::Float(_) => "float",
        PyValue::Str(_) => "str",
        PyValue::Duration(_) => "datetime.timedelta",
        PyValue::List(_) => "list",
        PyValue::Dict(_) => "dict",
    }
}

/// `bool(v)`.
pub fn truthy(v: &PyValue) -> bool {
    match v {
        PyValue::None => false,
        PyValue::Bool(b) => *b,
        PyValue::Int(t) => !t.trim_start_matches('-').trim_start_matches('0').is_empty(),
        PyValue::Float(f) => *f != 0.0,
        PyValue::Str(s) => !s.is_empty(),
        PyValue::Duration(d) => d.total_micros() != 0,
        PyValue::List(items) => !items.is_empty(),
        PyValue::Dict(pairs) => !pairs.is_empty(),
    }
}

/// `v == w`.
pub fn eq(v: &PyValue, w: &PyValue) -> bool {
    v.py_eq(w)
}

/// `feel.to_feel`: a plain value as a FEEL value (floats through their `repr`).
pub fn to_feel(v: &PyValue) -> R<Value> {
    Ok(match v {
        PyValue::None => Value::Null,
        PyValue::Bool(b) => Value::Bool(*b),
        PyValue::Int(t) => Value::Num(Decimal::parse(t)?),
        PyValue::Float(f) => Value::Num(Decimal::parse(&float_repr(*f))?),
        PyValue::Str(s) => Value::Str(s.clone()),
        PyValue::Duration(d) => Value::Duration(*d),
        PyValue::List(items) => Value::List(items.iter().map(to_feel).collect::<R<_>>()?),
        PyValue::Dict(pairs) => Value::Context(
            pairs
                .iter()
                .map(|(k, x)| Ok((k.clone(), to_feel(x)?)))
                .collect::<R<_>>()?,
        ),
    })
}

/// A context of FEEL values from plain variables.
pub fn feel_context(vars: &indexmap::IndexMap<String, PyValue>) -> R<Context> {
    vars.iter()
        .map(|(k, v)| Ok((k.clone(), to_feel(v)?)))
        .collect()
}

/// A number of Python's numeric tower: bool and int as integers, float.
#[derive(Debug, Clone, Copy)]
enum Num {
    I(i128),
    F(f64),
}

fn num_of(v: &PyValue) -> Option<Num> {
    match v {
        PyValue::Bool(b) => Some(Num::I(i128::from(*b))),
        PyValue::Int(t) => Some(match t.parse::<i128>() {
            Ok(i) => Num::I(i),
            Err(_) => Num::F(t.parse::<f64>().unwrap_or(f64::NAN)),
        }),
        PyValue::Float(f) => Some(Num::F(*f)),
        _ => None,
    }
}

fn int_value(i: i128) -> PyValue {
    PyValue::Int(i.to_string())
}

/// An integer operand of sequence repetition (`int` or `bool`).
fn index_of(v: &PyValue) -> Option<i128> {
    match num_of(v) {
        Some(Num::I(i)) => Some(i),
        _ => None,
    }
}

/// Exact comparison of an integer with a float (`None` when the float is NaN).
fn cmp_int_float(i: i128, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    const LIMIT: f64 = 1.7e38;
    if f >= LIMIT {
        return Some(Ordering::Less);
    }
    if f <= -LIMIT {
        return Some(Ordering::Greater);
    }
    let fl = f.floor();
    match i.cmp(&(fl as i128)) {
        Ordering::Equal if f > fl => Some(Ordering::Less),
        o => Some(o),
    }
}

fn cmp_num(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::I(x), Num::I(y)) => Some(x.cmp(&y)),
        (Num::F(x), Num::F(y)) => x.partial_cmp(&y),
        (Num::I(x), Num::F(y)) => cmp_int_float(x, y),
        (Num::F(x), Num::I(y)) => cmp_int_float(y, x).map(Ordering::reverse),
    }
}

fn apply(op: &str, o: Option<Ordering>) -> bool {
    match (op, o) {
        (_, None) => false,
        ("<", Some(o)) => o == Ordering::Less,
        ("<=", Some(o)) => o != Ordering::Greater,
        (">", Some(o)) => o == Ordering::Greater,
        (">=", Some(o)) => o != Ordering::Less,
        _ => false,
    }
}

/// `a OP b` for `<`, `<=`, `>`, `>=`; `TypeError` for values without an order.
pub fn compare(op: &str, a: &PyValue, b: &PyValue) -> R<bool> {
    if let (Some(x), Some(y)) = (num_of(a), num_of(b)) {
        return Ok(apply(op, cmp_num(x, y)));
    }
    match (a, b) {
        (PyValue::Str(x), PyValue::Str(y)) => Ok(apply(op, Some(x.cmp(y)))),
        (PyValue::Duration(x), PyValue::Duration(y)) => {
            Ok(apply(op, Some(x.total_micros().cmp(&y.total_micros()))))
        }
        (PyValue::List(x), PyValue::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                if !eq(p, q) {
                    return compare(op, p, q);
                }
            }
            Ok(apply(op, Some(x.len().cmp(&y.len()))))
        }
        _ => Err(PyError::new(
            "TypeError",
            format!(
                "'{op}' not supported between instances of '{}' and '{}'",
                type_name(a),
                type_name(b)
            ),
        )),
    }
}

fn unsupported(op: &str, a: &PyValue, b: &PyValue) -> PyError {
    PyError::new(
        "TypeError",
        format!(
            "unsupported operand type(s) for {op}: '{}' and '{}'",
            type_name(a),
            type_name(b)
        ),
    )
}

fn float_value(f: f64) -> PyValue {
    PyValue::Float(f)
}

fn as_f64(n: Num) -> f64 {
    match n {
        Num::I(i) => i as f64,
        Num::F(f) => f,
    }
}

/// Integer arithmetic of `+`, `-`, `*`, falling back to floats beyond `i128`.
fn int_op(op: &str, x: i128, y: i128) -> PyValue {
    let r = match op {
        "+" => x.checked_add(y),
        "-" => x.checked_sub(y),
        _ => x.checked_mul(y),
    };
    match r {
        Some(v) => int_value(v),
        None => {
            let (a, b) = (x as f64, y as f64);
            float_value(match op {
                "+" => a + b,
                "-" => a - b,
                _ => a * b,
            })
        }
    }
}

fn num_op(op: &str, a: Num, b: Num) -> R<PyValue> {
    if op == "/" {
        let both_int = matches!((a, b), (Num::I(_), Num::I(_)));
        let d = as_f64(b);
        if d == 0.0 {
            let msg = if both_int {
                "division by zero"
            } else {
                "float division by zero"
            };
            return Err(PyError::new("ZeroDivisionError", msg));
        }
        return Ok(float_value(as_f64(a) / d));
    }
    if let (Num::I(x), Num::I(y)) = (a, b) {
        return Ok(int_op(op, x, y));
    }
    let (x, y) = (as_f64(a), as_f64(b));
    Ok(float_value(match op {
        "+" => x + y,
        "-" => x - y,
        _ => x * y,
    }))
}

fn repeat_str(s: &str, n: i128) -> R<PyValue> {
    if n <= 0 || s.is_empty() {
        return Ok(PyValue::Str(String::new()));
    }
    if (s.len() as i128).saturating_mul(n) > i128::from(u32::MAX) {
        return Err(PyError::new("MemoryError", ""));
    }
    Ok(PyValue::Str(s.repeat(n as usize)))
}

fn repeat_list(items: &[PyValue], n: i128) -> R<PyValue> {
    if n <= 0 || items.is_empty() {
        return Ok(PyValue::List(Vec::new()));
    }
    if (items.len() as i128).saturating_mul(n) > i128::from(u32::MAX) {
        return Err(PyError::new("MemoryError", ""));
    }
    let mut out = Vec::with_capacity(items.len() * n as usize);
    for _ in 0..n {
        out.extend(items.iter().cloned());
    }
    Ok(PyValue::List(out))
}

fn duration_op(op: &str, x: &Duration, y: &Duration) -> R<PyValue> {
    Ok(PyValue::Duration(if op == "+" {
        x.add(y)?
    } else {
        x.sub(y)?
    }))
}

/// `a OP b` for `+`, `-`, `*`, `/`.
pub fn binary(op: &str, a: &PyValue, b: &PyValue) -> R<PyValue> {
    if let (Some(x), Some(y)) = (num_of(a), num_of(b)) {
        return num_op(op, x, y);
    }
    match op {
        "+" => match (a, b) {
            (PyValue::Str(x), PyValue::Str(y)) => Ok(PyValue::Str(format!("{x}{y}"))),
            (PyValue::Str(_), _) => Err(PyError::new(
                "TypeError",
                format!("can only concatenate str (not \"{}\") to str", type_name(b)),
            )),
            (PyValue::List(x), PyValue::List(y)) => {
                Ok(PyValue::List(x.iter().chain(y).cloned().collect()))
            }
            (PyValue::List(_), _) => Err(PyError::new(
                "TypeError",
                format!(
                    "can only concatenate list (not \"{}\") to list",
                    type_name(b)
                ),
            )),
            (PyValue::Duration(x), PyValue::Duration(y)) => duration_op(op, x, y),
            _ => Err(unsupported(op, a, b)),
        },
        "-" => match (a, b) {
            (PyValue::Duration(x), PyValue::Duration(y)) => duration_op(op, x, y),
            _ => Err(unsupported(op, a, b)),
        },
        "*" => {
            let seq = |s: &PyValue, n: &PyValue| -> Option<R<PyValue>> {
                match s {
                    PyValue::Str(_) | PyValue::List(_) => Some(match index_of(n) {
                        Some(k) => match s {
                            PyValue::Str(t) => repeat_str(t, k),
                            PyValue::List(items) => repeat_list(items, k),
                            _ => Err(unsupported(op, a, b)),
                        },
                        None => Err(PyError::new(
                            "TypeError",
                            format!(
                                "can't multiply sequence by non-int of type '{}'",
                                type_name(n)
                            ),
                        )),
                    }),
                    _ => None,
                }
            };
            if let Some(r) = seq(a, b) {
                return r;
            }
            if let Some(r) = seq(b, a) {
                return r;
            }
            Err(unsupported(op, a, b))
        }
        _ => Err(unsupported(op, a, b)),
    }
}

/// `-v`.
pub fn negate(v: &PyValue) -> R<PyValue> {
    match num_of(v) {
        Some(Num::I(i)) => Ok(int_value(-i)),
        Some(Num::F(f)) => Ok(float_value(-f)),
        None => match v {
            PyValue::Duration(d) => Ok(PyValue::Duration(d.neg()?)),
            _ => Err(PyError::new(
                "TypeError",
                format!("bad operand type for unary -: '{}'", type_name(v)),
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &str) -> PyValue {
        PyValue::Str(t.into())
    }

    fn err(r: R<impl std::fmt::Debug>) -> String {
        r.expect_err("an error").message
    }

    #[test]
    fn python_operators() {
        let one = PyValue::Int("1".into());
        let half = PyValue::Float(0.5);
        assert!(matches!(binary("+", &one, &half), Ok(PyValue::Float(x)) if x == 1.5));
        assert!(matches!(binary("+", &PyValue::Bool(true), &one), Ok(PyValue::Int(t)) if t == "2"));
        assert_eq!(
            err(binary("+", &s("a"), &half)),
            "can only concatenate str (not \"float\") to str"
        );
        assert_eq!(
            err(binary("-", &s("a"), &half)),
            "unsupported operand type(s) for -: 'str' and 'float'"
        );
        assert_eq!(
            err(binary("*", &half, &s("a"))),
            "can't multiply sequence by non-int of type 'float'"
        );
        assert!(
            matches!(binary("*", &s("ab"), &PyValue::Int("2".into())), Ok(PyValue::Str(t)) if t == "abab")
        );
        assert_eq!(
            err(binary("/", &one, &PyValue::Int("0".into()))),
            "division by zero"
        );
        assert_eq!(
            err(binary("/", &half, &PyValue::Int("0".into()))),
            "float division by zero"
        );
        assert_eq!(
            err(compare("<", &s("a"), &one)),
            "'<' not supported between instances of 'str' and 'int'"
        );
        assert_eq!(compare("<", &one, &PyValue::Float(1.5)), Ok(true));
        assert_eq!(compare(">=", &PyValue::Float(f64::NAN), &one), Ok(false));
        assert_eq!(
            err(negate(&PyValue::None)),
            "bad operand type for unary -: 'NoneType'"
        );
        assert!(truthy(&PyValue::Float(f64::NAN)));
        assert!(!truthy(&PyValue::Int("0".into())));
    }
}
