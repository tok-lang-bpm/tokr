//! Values of the evaluator of language 0.7 for the command line, JSON, Python, R and the
//! simulator, and the entry point they share (`evaluate_values`).

use crate::dmn;
use crate::feel_eval::{self as fe, PyValue};
use crate::feel3::eval::temporal_literal;
use crate::feel3::temporal::DtDuration;
use crate::feel3::{Map, Num, Value};
use crate::json::Json;
use crate::model::Document;
use crate::pyutil::PyError;

/// A value given on the command line: `true`/`false`, a number, a JSON object or list,
/// or else text.
pub fn cli_value(text: &str) -> Value {
    let low = text.to_lowercase();
    if low == "true" || low == "false" {
        return Value::Bool(low == "true");
    }
    if let Some(n) = Num::parse(text) {
        return Value::Num(n);
    }
    let t = text.trim();
    if (t.starts_with('{') || t.starts_with('['))
        && let Some(v) = parse_json(t)
    {
        return v;
    }
    Value::Str(text.to_string())
}

/// A JSON text as a FEEL value (objects are contexts, arrays lists).
pub fn parse_json(text: &str) -> Option<Value> {
    let ch: Vec<char> = text.chars().collect();
    let mut i = 0;
    let v = json_value(&ch, &mut i)?;
    skip(&ch, &mut i);
    (i == ch.len()).then_some(v)
}

fn skip(ch: &[char], i: &mut usize) {
    while *i < ch.len() && ch[*i].is_whitespace() {
        *i += 1;
    }
}

fn json_string(ch: &[char], i: &mut usize) -> Option<String> {
    if ch.get(*i) != Some(&'"') {
        return None;
    }
    *i += 1;
    let mut out = String::new();
    while *i < ch.len() {
        let c = ch[*i];
        *i += 1;
        match c {
            '"' => return Some(out),
            '\\' => {
                let e = *ch.get(*i)?;
                *i += 1;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let hex: String = ch.get(*i..*i + 4)?.iter().collect();
                        *i += 4;
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    None
}

fn json_value(ch: &[char], i: &mut usize) -> Option<Value> {
    skip(ch, i);
    match ch.get(*i)? {
        '{' => {
            *i += 1;
            let mut m = Map::new();
            skip(ch, i);
            if ch.get(*i) == Some(&'}') {
                *i += 1;
                return Some(Value::context(m));
            }
            loop {
                skip(ch, i);
                let k = json_string(ch, i)?;
                skip(ch, i);
                if ch.get(*i) != Some(&':') {
                    return None;
                }
                *i += 1;
                let v = json_value(ch, i)?;
                m.insert(k, v);
                skip(ch, i);
                match ch.get(*i)? {
                    ',' => *i += 1,
                    '}' => {
                        *i += 1;
                        return Some(Value::context(m));
                    }
                    _ => return None,
                }
            }
        }
        '[' => {
            *i += 1;
            let mut l = Vec::new();
            skip(ch, i);
            if ch.get(*i) == Some(&']') {
                *i += 1;
                return Some(Value::list(l));
            }
            loop {
                l.push(json_value(ch, i)?);
                skip(ch, i);
                match ch.get(*i)? {
                    ',' => *i += 1,
                    ']' => {
                        *i += 1;
                        return Some(Value::list(l));
                    }
                    _ => return None,
                }
            }
        }
        '"' => json_string(ch, i).map(Value::Str),
        _ => {
            let start = *i;
            while *i < ch.len() && !matches!(ch[*i], ',' | '}' | ']') && !ch[*i].is_whitespace() {
                *i += 1;
            }
            let word: String = ch[start..*i].iter().collect();
            match word.as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                "null" => Some(Value::Null),
                w => Num::parse(w).map(Value::Num),
            }
        }
    }
}

/// The JSON form of a result: numbers as numbers, temporal values as their FEEL text,
/// ranges in FEEL notation, functions as `null`.
pub fn to_json(v: &Value) -> Json {
    match v {
        Value::Null | Value::Func(_) => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Num(n) => Json::Num(n.to_text()),
        Value::Str(s) => Json::Str(s.clone()),
        Value::List(items) => Json::Arr(items.iter().map(to_json).collect()),
        Value::Context(m) => Json::Obj(m.iter().map(|(k, x)| (k.clone(), to_json(x))).collect()),
        other => Json::Str(other.text()),
    }
}

/// A value of the FEEL subset (inputs of Python, R and the simulator) in language 0.7.
pub fn from_subset(v: &fe::Value) -> Value {
    match v {
        fe::Value::Null => Value::Null,
        fe::Value::Bool(b) => Value::Bool(*b),
        fe::Value::Num(d) => Value::num(Num::parse(&d.to_sci_string())),
        fe::Value::Str(s) => Value::Str(s.clone()),
        fe::Value::Date(d) => temporal_literal(&d.isoformat()),
        fe::Value::DateTime(d) => temporal_literal(&d.isoformat()),
        fe::Value::Duration(d) => Value::Dt(DtDuration(d.total_micros() * 1000)),
        fe::Value::List(items) => Value::list(items.iter().map(from_subset).collect()),
        fe::Value::Context(m) => {
            Value::context(m.iter().map(|(k, x)| (k.clone(), from_subset(x))).collect())
        }
    }
}

/// A result as a plain value for Python, R and the simulator: integral numbers as
/// integers, other numbers as floats, temporal values as text (days-and-time durations
/// as durations), ranges as text, functions as `None`.
pub fn to_plain(v: &Value) -> PyValue {
    match v {
        Value::Null | Value::Func(_) => PyValue::None,
        Value::Bool(b) => PyValue::Bool(*b),
        Value::Num(n) => {
            if n.is_integer() {
                PyValue::Int(n.to_text())
            } else {
                PyValue::Float(n.to_f64())
            }
        }
        Value::Str(s) => PyValue::Str(s.clone()),
        Value::Dt(d) => match crate::pytime::Duration::from_micros(d.0 / 1000) {
            Ok(x) if d.0 % 1000 == 0 => PyValue::Duration(x),
            _ => PyValue::Str(d.text()),
        },
        Value::List(items) => PyValue::List(items.iter().map(to_plain).collect()),
        Value::Context(m) => {
            PyValue::Dict(m.iter().map(|(k, x)| (k.clone(), to_plain(x))).collect())
        }
        other => PyValue::Str(other.text()),
    }
}

/// A plain value (of the simulator) as a value of language 0.7.
pub fn from_plain(v: &PyValue) -> Value {
    match v {
        PyValue::None => Value::Null,
        PyValue::Bool(b) => Value::Bool(*b),
        PyValue::Int(t) => Value::num(Num::parse(t)),
        PyValue::Float(f) => Value::num(Num::from_f64(*f)),
        PyValue::Str(s) => Value::Str(s.clone()),
        PyValue::Duration(d) => Value::Dt(DtDuration(d.total_micros() * 1000)),
        PyValue::List(items) => Value::list(items.iter().map(from_plain).collect()),
        PyValue::Dict(pairs) => Value::context(
            pairs
                .iter()
                .map(|(k, x)| (k.clone(), from_plain(x)))
                .collect(),
        ),
    }
}

fn decision_error(problems: &[String]) -> PyError {
    PyError::new("tok.dmn.DecisionError", problems.join("; "))
}

/// Evaluate a decision for values of the FEEL subset, as `tok eval`, Python and R do:
/// in a 0.7 document with its requirements (the result is the DMN value of the
/// decision), in earlier documents the decision table alone, as before.
pub fn evaluate_values(
    doc: &Document,
    decision: &str,
    ctx: &fe::Context,
) -> Result<PyValue, PyError> {
    if !doc.lang_07() {
        let Some(dec) = doc.decisions.get(decision) else {
            return Ok(PyValue::None);
        };
        return dmn::evaluate_decision(dec, ctx);
    }
    let inputs: Map = ctx
        .iter()
        .map(|(k, v)| (k.clone(), from_subset(v)))
        .collect();
    let (v, problems) = super::eval::evaluate(doc, decision, &inputs);
    if !problems.is_empty() {
        return Err(decision_error(&problems));
    }
    Ok(to_plain(&v))
}

/// Evaluate a decision of a 0.7 document for plain values (the simulator).
pub fn evaluate_plain(
    doc: &Document,
    decision: &str,
    vars: &indexmap::IndexMap<String, PyValue>,
) -> Result<PyValue, PyError> {
    let inputs: Map = vars
        .iter()
        .map(|(k, v)| (k.clone(), from_plain(v)))
        .collect();
    let (v, problems) = super::eval::evaluate(doc, decision, &inputs);
    if !problems.is_empty() {
        return Err(decision_error(&problems));
    }
    Ok(to_plain(&v))
}
