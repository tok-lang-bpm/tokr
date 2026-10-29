//! The standard function library of FEEL (DMN 1.5 §10.3.4).

use std::cmp::Ordering;
use std::rc::Rc;

use super::eval::{Eval, bool_of};
use super::num::Num;
use super::parse::{Names, parse_expression};
use super::regex::Regex;
use super::temporal::{self, AnyDuration, Date, DateTime, DtDuration, Time, YmDuration, Zone};
use super::{Func, Map, Value, compare, equal};
use crate::pydec::Rounding;

/// Signatures: function name and parameter names, in order. A function may have several.
const SIGNATURES: &[(&str, &[&str])] = &[
    ("date", &["from"]),
    ("date", &["year", "month", "day"]),
    ("date and time", &["from"]),
    ("date and time", &["date", "time"]),
    ("time", &["from"]),
    ("time", &["hour", "minute", "second", "offset"]),
    (
        "number",
        &["from", "grouping separator", "decimal separator"],
    ),
    ("string", &["from"]),
    ("duration", &["from"]),
    ("years and months duration", &["from", "to"]),
    ("not", &["negand"]),
    ("substring", &["string", "start position", "length"]),
    ("string length", &["string"]),
    ("upper case", &["string"]),
    ("lower case", &["string"]),
    ("substring before", &["string", "match"]),
    ("substring after", &["string", "match"]),
    ("replace", &["input", "pattern", "replacement", "flags"]),
    ("contains", &["string", "match"]),
    ("starts with", &["string", "match"]),
    ("ends with", &["string", "match"]),
    ("matches", &["input", "pattern", "flags"]),
    ("split", &["string", "delimiter"]),
    ("string join", &["list", "delimiter"]),
    ("list contains", &["list", "element"]),
    ("count", &["list"]),
    ("min", &["list"]),
    ("max", &["list"]),
    ("sum", &["list"]),
    ("mean", &["list"]),
    ("all", &["list"]),
    ("any", &["list"]),
    ("product", &["list"]),
    ("median", &["list"]),
    ("stddev", &["list"]),
    ("mode", &["list"]),
    ("sublist", &["list", "start position", "length"]),
    ("append", &["list", "item"]),
    ("concatenate", &["list"]),
    ("insert before", &["list", "position", "newItem"]),
    ("remove", &["list", "position"]),
    ("reverse", &["list"]),
    ("index of", &["list", "match"]),
    ("union", &["list"]),
    ("distinct values", &["list"]),
    ("flatten", &["list"]),
    ("sort", &["list", "precedes"]),
    ("list replace", &["list", "position", "newItem"]),
    ("list replace", &["list", "match", "newItem"]),
    ("decimal", &["n", "scale"]),
    ("floor", &["n", "scale"]),
    ("ceiling", &["n", "scale"]),
    ("round up", &["n", "scale"]),
    ("round down", &["n", "scale"]),
    ("round half up", &["n", "scale"]),
    ("round half down", &["n", "scale"]),
    ("abs", &["n"]),
    ("modulo", &["dividend", "divisor"]),
    ("sqrt", &["number"]),
    ("log", &["number"]),
    ("exp", &["number"]),
    ("odd", &["number"]),
    ("even", &["number"]),
    ("is", &["value1", "value2"]),
    ("day of year", &["date"]),
    ("day of week", &["date"]),
    ("month of year", &["date"]),
    ("week of year", &["date"]),
    ("last day of month", &["date"]),
    ("before", &["point1", "point2"]),
    ("before", &["point", "range"]),
    ("before", &["range", "point"]),
    ("before", &["range1", "range2"]),
    ("after", &["point1", "point2"]),
    ("after", &["point", "range"]),
    ("after", &["range", "point"]),
    ("after", &["range1", "range2"]),
    ("meets", &["range1", "range2"]),
    ("met by", &["range1", "range2"]),
    ("overlaps", &["range1", "range2"]),
    ("overlaps before", &["range1", "range2"]),
    ("overlaps after", &["range1", "range2"]),
    ("finishes", &["point", "range"]),
    ("finishes", &["range1", "range2"]),
    ("finished by", &["range", "point"]),
    ("finished by", &["range1", "range2"]),
    ("includes", &["range", "point"]),
    ("includes", &["range1", "range2"]),
    ("during", &["point", "range"]),
    ("during", &["range1", "range2"]),
    ("starts", &["point", "range"]),
    ("starts", &["range1", "range2"]),
    ("started by", &["range", "point"]),
    ("started by", &["range1", "range2"]),
    ("coincides", &["point1", "point2"]),
    ("coincides", &["range1", "range2"]),
    ("range", &["from"]),
    ("get value", &["m", "key"]),
    ("get entries", &["m"]),
    ("context", &["entries"]),
    ("context put", &["context", "key", "value"]),
    ("context put", &["context", "keys", "value"]),
    ("context merge", &["contexts"]),
    ("now", &[]),
    ("today", &[]),
];

/// Functions whose parameters take single values: a list of one is its element.
const SCALAR: &[&str] = &[
    "date",
    "date and time",
    "time",
    "number",
    "duration",
    "years and months duration",
    "not",
    "substring",
    "string length",
    "upper case",
    "lower case",
    "substring before",
    "substring after",
    "replace",
    "contains",
    "starts with",
    "ends with",
    "matches",
    "split",
    "decimal",
    "floor",
    "ceiling",
    "round up",
    "round down",
    "round half up",
    "round half down",
    "abs",
    "modulo",
    "sqrt",
    "log",
    "exp",
    "odd",
    "even",
    "day of year",
    "day of week",
    "month of year",
    "week of year",
    "last day of month",
];

/// The built-in function with this name.
pub fn canonical(name: &str) -> Option<&'static str> {
    SIGNATURES.iter().find(|(n, _)| *n == name).map(|(n, _)| *n)
}

/// Positional arguments of a call with named arguments.
pub fn by_name(name: &str, named: &[(String, Value)]) -> Option<Vec<Value>> {
    'sig: for (n, params) in SIGNATURES {
        if *n != name {
            continue;
        }
        for (k, _) in named {
            if !params.contains(&k.as_str()) {
                continue 'sig;
            }
        }
        let mut args: Vec<Value> = params
            .iter()
            .map(|p| {
                named
                    .iter()
                    .find(|(k, _)| k == p)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            })
            .collect();
        if name == "context put"
            && named
                .iter()
                .any(|(k, v)| k == "key" && matches!(v, Value::List(_)))
        {
            return None;
        }
        // trailing optional parameters that were not given are absent, not null
        while args.len() > 1 && arity(name, args.len() - 1) {
            let p = params[args.len() - 1];
            if named.iter().any(|(k, _)| k == p) {
                break;
            }
            args.pop();
        }
        return Some(args);
    }
    None
}

fn num(v: &Value) -> Option<&Num> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn s(v: &Value) -> Option<&str> {
    match v {
        Value::Str(x) => Some(x),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    num(v).and_then(Num::to_i64)
}

fn chars(x: &str) -> Vec<char> {
    x.chars().collect()
}

/// The elements of a variadic list function: one list argument or several values.
fn items(args: &[Value]) -> Option<Rc<Vec<Value>>> {
    match args {
        [] => None,
        [Value::List(l)] => Some(l.clone()),
        [Value::Null] => None,
        _ => Some(Rc::new(args.to_vec())),
    }
}

fn numbers(l: &[Value]) -> Option<Vec<Num>> {
    l.iter().map(|v| num(v).cloned()).collect()
}

fn round_with(args: &[Value], mode: Rounding, default_scale: bool) -> Value {
    let Some(n) = args.first().and_then(num) else {
        return Value::Null;
    };
    let scale = match args.get(1) {
        Some(v) => match num(v).and_then(Num::trunc_i64) {
            Some(s) => s,
            None => return Value::Null,
        },
        None if default_scale => 0,
        None => return Value::Null,
    };
    Value::num(n.round(scale, mode))
}

fn date_of(v: &Value) -> Option<Date> {
    match v {
        Value::Date(d) => Some(*d),
        Value::DateTime(d) => Some(d.date),
        _ => None,
    }
}

fn sort_values(ev: &Eval, list: &[Value], precedes: Option<&Func>) -> Option<Vec<Value>> {
    let mut out: Vec<Value> = list.to_vec();
    let mut failed = false;
    // insertion into a sorted vector keeps the sort stable and calls the function O(n²)
    // at most, which is fine for decision models
    let mut sorted: Vec<Value> = Vec::with_capacity(out.len());
    for v in out.drain(..) {
        let mut pos = sorted.len();
        while pos > 0 {
            let before = match precedes {
                Some(f) => match bool_of(&ev.call(f, vec![v.clone(), sorted[pos - 1].clone()])) {
                    Some(b) => b,
                    None => {
                        failed = true;
                        false
                    }
                },
                None => match compare(&v, &sorted[pos - 1]) {
                    Some(o) => o == Ordering::Less,
                    None => {
                        failed = true;
                        false
                    }
                },
            };
            if !before {
                break;
            }
            pos -= 1;
        }
        sorted.insert(pos, v);
    }
    (!failed).then_some(sorted)
}

fn flatten_into(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::List(l) => l.iter().for_each(|x| flatten_into(x, out)),
        other => out.push(other.clone()),
    }
}

fn distinct(l: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for v in l {
        if !out.iter().any(|x| equal(x, v) == Some(true)) {
            out.push(v.clone());
        }
    }
    out
}

fn number_from_text(text: &str, group: &Value, dec: &Value) -> Value {
    let g = match group {
        Value::Null => None,
        Value::Str(x) if [" ", ",", "."].contains(&x.as_str()) => Some(x.as_str()),
        _ => return Value::Null,
    };
    let d = match dec {
        Value::Null => None,
        Value::Str(x) if [",", "."].contains(&x.as_str()) => Some(x.as_str()),
        _ => return Value::Null,
    };
    if g.is_some() && g == d {
        return Value::Null;
    }
    let mut t = text.to_string();
    if let Some(g) = g {
        t = t.replace(g, "");
    }
    if let Some(d) = d {
        t = t.replace(d, ".");
    }
    Value::num(Num::parse(&t))
}

fn time_from_parts(args: &[Value]) -> Value {
    let (Some(h), Some(m), Some(sec)) = (int(&args[0]), int(&args[1]), args.get(2).and_then(num))
    else {
        return Value::Null;
    };
    let whole = match sec.floor().and_then(|f| f.to_i64()) {
        Some(w) => w,
        None => return Value::Null,
    };
    let frac = sec
        .sub(&Num::int(whole))
        .and_then(|f| f.mul(&Num::int(1_000_000_000)))
        .and_then(|f| f.round(0, Rounding::HalfEven))
        .and_then(|f| f.to_i64())
        .unwrap_or(0);
    let zone = match args.get(3) {
        None | Some(Value::Null) => Zone::None,
        Some(Value::Dt(d)) => {
            let secs = d.0 / 1_000_000_000;
            if d.0 % 1_000_000_000 != 0 || secs.abs() > 14 * 3600 {
                return Value::Null;
            }
            Zone::Offset(secs as i32)
        }
        _ => return Value::Null,
    };
    Time::new(h, m, whole, i128::from(frac), zone)
        .map(Value::Time)
        .unwrap_or(Value::Null)
}

/// Range ends for the range functions: a point is a closed range of one value.
struct Ends {
    low: Option<Value>,
    lc: bool,
    high: Option<Value>,
    hc: bool,
}

fn ends(v: &Value) -> Option<(Ends, bool)> {
    match v {
        Value::Null => None,
        Value::Range(r) => Some((
            Ends {
                low: r.low.clone(),
                lc: r.low_closed,
                high: r.high.clone(),
                hc: r.high_closed,
            },
            true,
        )),
        Value::List(_) | Value::Context(_) | Value::Func(_) => None,
        p => Some((
            Ends {
                low: Some(p.clone()),
                lc: true,
                high: Some(p.clone()),
                hc: true,
            },
            false,
        )),
    }
}

/// Compare range ends; an absent low end is -∞ and an absent high end +∞.
fn cmp_end(a: &Option<Value>, a_low: bool, b: &Option<Value>, b_low: bool) -> Option<Ordering> {
    match (a, b) {
        (Some(x), Some(y)) => compare(x, y),
        (None, None) => Some(if a_low == b_low {
            Ordering::Equal
        } else if a_low {
            Ordering::Less
        } else {
            Ordering::Greater
        }),
        (None, Some(_)) => Some(if a_low {
            Ordering::Less
        } else {
            Ordering::Greater
        }),
        (Some(_), None) => Some(if b_low {
            Ordering::Greater
        } else {
            Ordering::Less
        }),
    }
}

fn range_fn(name: &str, a: &Value, b: &Value) -> Option<bool> {
    use Ordering::*;
    let (x, xr) = ends(a)?;
    let (y, yr) = ends(b)?;
    // ends of different types are not comparable
    for (p, q) in [(&x.low, &y.low), (&x.high, &y.high), (&x.low, &y.high)] {
        if let (Some(p), Some(q)) = (p, q)
            && compare(p, q).is_none()
        {
            return None;
        }
    }
    let lo_lo = cmp_end(&x.low, true, &y.low, true)?;
    let hi_hi = cmp_end(&x.high, false, &y.high, false)?;
    let hi_lo = cmp_end(&x.high, false, &y.low, true)?;
    let lo_hi = cmp_end(&x.low, true, &y.high, false)?;
    Some(match name {
        "before" => hi_lo == Less || (hi_lo == Equal && !(x.hc && y.lc)),
        "after" => lo_hi == Greater || (lo_hi == Equal && !(x.lc && y.hc)),
        "meets" => xr && yr && x.hc && y.lc && hi_lo == Equal,
        "met by" => xr && yr && x.lc && y.hc && lo_hi == Equal,
        "overlaps" => {
            (hi_lo == Greater || (hi_lo == Equal && x.hc && y.lc))
                && (lo_hi == Less || (lo_hi == Equal && x.lc && y.hc))
        }
        "overlaps before" => {
            (lo_lo == Less || (lo_lo == Equal && x.lc && !y.lc))
                && (hi_lo == Greater || (hi_lo == Equal && x.hc && y.lc))
                && (hi_hi == Less || (hi_hi == Equal && (!x.hc || y.hc)))
        }
        "overlaps after" => {
            (lo_lo == Greater || (lo_lo == Equal && !x.lc && y.lc))
                && (lo_hi == Less || (lo_hi == Equal && x.lc && y.hc))
                && (hi_hi == Greater || (hi_hi == Equal && (x.hc || !y.hc)))
        }
        "finishes" => {
            if !xr {
                y.hc && hi_hi == Equal
            } else {
                x.hc == y.hc
                    && hi_hi == Equal
                    && (lo_lo == Greater || (lo_lo == Equal && (!x.lc || y.lc)))
            }
        }
        "finished by" => {
            if !yr {
                x.hc && hi_hi == Equal
            } else {
                x.hc == y.hc
                    && hi_hi == Equal
                    && (lo_lo == Less || (lo_lo == Equal && (x.lc || !y.lc)))
            }
        }
        "includes" => {
            if !yr {
                (lo_lo == Less && hi_hi == Greater)
                    || (lo_lo == Equal && x.lc)
                    || (hi_hi == Equal && x.hc)
            } else {
                (lo_lo == Less || (lo_lo == Equal && (x.lc || !y.lc)))
                    && (hi_hi == Greater || (hi_hi == Equal && (x.hc || !y.hc)))
            }
        }
        "during" => {
            if !xr {
                (lo_lo == Greater && hi_hi == Less)
                    || (lo_lo == Equal && y.lc)
                    || (hi_hi == Equal && y.hc)
            } else {
                (lo_lo == Greater || (lo_lo == Equal && (!x.lc || y.lc)))
                    && (hi_hi == Less || (hi_hi == Equal && (!x.hc || y.hc)))
            }
        }
        "starts" => {
            if !xr {
                y.lc && lo_lo == Equal
            } else {
                x.lc == y.lc
                    && lo_lo == Equal
                    && (hi_hi == Less || (hi_hi == Equal && (!x.hc || y.hc)))
            }
        }
        "started by" => {
            if !yr {
                x.lc && lo_lo == Equal
            } else {
                x.lc == y.lc
                    && lo_lo == Equal
                    && (hi_hi == Greater || (hi_hi == Equal && (x.hc || !y.hc)))
            }
        }
        "coincides" => {
            if xr != yr {
                return None;
            }
            lo_lo == Equal && hi_hi == Equal && x.lc == y.lc && x.hc == y.hc
        }
        _ => return None,
    })
}

/// Call the built-in function `name` with positional arguments.
pub fn call(ev: &Eval, name: &str, args: Vec<Value>) -> Value {
    // a parameter that takes a list accepts a single value as a list of one; other
    // parameters take the element of a list of one (DMN 1.5 §10.3.2.9.4)
    let lists: &[usize] = match name {
        "string join" | "list contains" | "count" | "sublist" | "append" | "insert before"
        | "remove" | "reverse" | "index of" | "distinct values" | "flatten" | "sort"
        | "list replace" | "context merge" => &[0],
        "concatenate" | "union" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        "context" => &[0],
        _ => &[],
    };
    let variadic = [
        "min", "max", "sum", "mean", "all", "any", "product", "median", "stddev", "mode",
    ];
    let args: Vec<Value> = args
        .into_iter()
        .enumerate()
        .map(|(k, v)| {
            if lists.contains(&k) {
                match v {
                    Value::List(_) | Value::Null => v,
                    Value::Context(_) if name == "context" => v,
                    other => Value::list(vec![other]),
                }
            } else if variadic.contains(&name) || !SCALAR.contains(&name) {
                v
            } else {
                match v {
                    Value::List(l) if l.len() == 1 && !matches!(name, "list contains") => {
                        l[0].clone()
                    }
                    other => other,
                }
            }
        })
        .collect();
    call_inner(ev, name, &args).unwrap_or(Value::Null)
}

fn arity(name: &str, n: usize) -> bool {
    let variadic = [
        "min",
        "max",
        "sum",
        "mean",
        "all",
        "any",
        "product",
        "median",
        "stddev",
        "mode",
        "concatenate",
        "union",
        "append",
    ];
    if variadic.contains(&name) {
        return n >= 1;
    }
    SIGNATURES.iter().any(|(f, params)| {
        *f == name && {
            let max = params.len();
            let min = match name {
                "substring" | "sublist" | "replace" | "matches" | "number" | "string join"
                | "floor" | "ceiling" | "sort" | "time" => max - 1,
                _ => max,
            };
            (min..=max).contains(&n)
        }
    })
}

fn call_inner(ev: &Eval, name: &str, a: &[Value]) -> Option<Value> {
    use Value as V;
    if !arity(name, a.len()) {
        return None;
    }
    let arg = |k: usize| a.get(k).unwrap_or(&V::Null);
    Some(match name {
        // conversion
        "date" => match a {
            [V::Str(t)] => V::Date(Date::parse(t)?),
            [V::Date(d)] => V::Date(*d),
            [V::DateTime(d)] => V::Date(d.date),
            [y, m, d] => V::Date(Date::new(int(y)?, int(m)?, int(d)?)?),
            _ => return None,
        },
        "date and time" => match a {
            [V::Str(t)] => V::DateTime(DateTime::parse(t)?),
            [V::DateTime(d)] => V::DateTime(d.clone()),
            [d, V::Time(t)] => V::DateTime(DateTime {
                date: date_of(d)?,
                time: t.clone(),
            }),
            _ => return None,
        },
        "time" => match a {
            [V::Str(t)] => V::Time(Time::parse(t)?),
            [V::Time(t)] => V::Time(t.clone()),
            [V::DateTime(d)] => V::Time(d.time.clone()),
            [V::Date(_)] => V::Time(Time {
                nanos: 0,
                zone: Zone::Offset(0),
            }),
            [_, _, _] | [_, _, _, _] => time_from_parts(a),
            _ => return None,
        },
        "number" => {
            let t = s(arg(0))?;
            number_from_text(t, arg(1), arg(2))
        }
        "string" => match arg(0) {
            V::Null => V::Null,
            V::Str(x) => V::Str(x.clone()),
            other => V::Str(other.text()),
        },
        "duration" => match temporal::parse_duration(s(arg(0))?)? {
            AnyDuration::Dt(d) => V::Dt(d),
            AnyDuration::Ym(d) => V::Ym(d),
        },
        "years and months duration" => {
            let (f, t) = (arg(0), arg(1));
            let (df, dt) = (date_of(f)?, date_of(t)?);
            V::Ym(YmDuration(temporal::months_between(&df, &dt)))
        }
        // boolean
        "not" => V::Bool(!bool_of(arg(0))?),
        // string
        "substring" => {
            let c = chars(s(arg(0))?);
            let start = num(arg(1))?.round(0, Rounding::HalfEven)?.to_i64()?;
            let len = c.len() as i64;
            let from = if start > 0 { start - 1 } else { len + start };
            if start == 0 {
                return None;
            }
            let from = from.max(0);
            let count = match a.get(2) {
                Some(l) => num(l)?.trunc_i64()?.max(0),
                None => len,
            };
            let to = (from + count).min(len);
            if from >= len {
                return Some(V::str(""));
            }
            V::Str(c[from as usize..to.max(from) as usize].iter().collect())
        }
        "string length" => V::int(s(arg(0))?.chars().count() as i64),
        "upper case" => V::Str(s(arg(0))?.to_uppercase()),
        "lower case" => V::Str(s(arg(0))?.to_lowercase()),
        "substring before" => {
            let (x, m) = (s(arg(0))?, s(arg(1))?);
            V::Str(x.find(m).map(|i| x[..i].to_string()).unwrap_or_default())
        }
        "substring after" => {
            let (x, m) = (s(arg(0))?, s(arg(1))?);
            if m.is_empty() {
                return Some(V::str(x));
            }
            V::Str(
                x.find(m)
                    .map(|i| x[i + m.len()..].to_string())
                    .unwrap_or_default(),
            )
        }
        "replace" => {
            let (x, p, r) = (s(arg(0))?, s(arg(1))?, s(arg(2))?);
            let flags = match a.get(3) {
                Some(V::Str(f)) => f.as_str(),
                Some(V::Null) | None => "",
                _ => return None,
            };
            let re = Regex::new(p, flags)?;
            V::Str(re.replace(x, r)?)
        }
        "contains" => V::Bool(s(arg(0))?.contains(s(arg(1))?)),
        "starts with" => V::Bool(s(arg(0))?.starts_with(s(arg(1))?)),
        "ends with" => V::Bool(s(arg(0))?.ends_with(s(arg(1))?)),
        "matches" => {
            let (x, p) = (s(arg(0))?, s(arg(1))?);
            let flags = match a.get(2) {
                Some(V::Str(f)) => f.as_str(),
                Some(V::Null) | None => "",
                _ => return None,
            };
            V::Bool(Regex::new(p, flags)?.is_match(x))
        }
        "split" => {
            let (x, d) = (s(arg(0))?, s(arg(1))?);
            let re = Regex::new(d, "")?;
            V::list(re.split(x)?.into_iter().map(V::Str).collect())
        }
        "string join" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            let sep = match a.get(1) {
                Some(V::Str(x)) => x.clone(),
                Some(V::Null) | None => String::new(),
                _ => return None,
            };
            let mut parts = Vec::new();
            for v in l.iter() {
                match v {
                    V::Str(x) => parts.push(x.clone()),
                    V::Null => {}
                    _ => return None,
                }
            }
            V::Str(parts.join(&sep))
        }
        // lists
        "list contains" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            V::Bool(l.iter().any(|x| equal(x, arg(1)) == Some(true)))
        }
        "count" => match arg(0) {
            V::List(l) => V::int(l.len() as i64),
            _ => return None,
        },
        "min" | "max" => {
            let l = items(a)?;
            if l.is_empty() {
                return None;
            }
            let mut best = l[0].clone();
            for v in l.iter().skip(1) {
                let o = compare(v, &best)?;
                if (name == "min" && o == Ordering::Less)
                    || (name == "max" && o == Ordering::Greater)
                {
                    best = v.clone();
                }
            }
            compare(&best, &best)?;
            best
        }
        "sum" => {
            let l = items(a)?;
            if l.is_empty() {
                return None;
            }
            let ns = numbers(&l)?;
            let mut t = Num::zero();
            for n in &ns {
                t = t.add(n)?;
            }
            V::Num(t)
        }
        "product" => {
            let l = items(a)?;
            if l.is_empty() {
                return None;
            }
            let ns = numbers(&l)?;
            let mut t = Num::int(1);
            for n in &ns {
                t = t.mul(n)?;
            }
            V::Num(t)
        }
        "mean" => {
            let l = items(a)?;
            if l.is_empty() {
                return None;
            }
            let ns = numbers(&l)?;
            let mut t = Num::zero();
            for n in &ns {
                t = t.add(n)?;
            }
            V::Num(t.div(&Num::int(ns.len() as i64))?)
        }
        "median" => {
            let l = items(a)?;
            if l.is_empty() {
                return None;
            }
            let mut ns = numbers(&l)?;
            ns.sort();
            let k = ns.len();
            if k % 2 == 1 {
                V::Num(ns[k / 2].clone())
            } else {
                V::Num(ns[k / 2 - 1].add(&ns[k / 2])?.div(&Num::int(2))?)
            }
        }
        "stddev" => {
            let l = items(a)?;
            let ns = numbers(&l)?;
            if ns.len() < 2 {
                return None;
            }
            let mut t = Num::zero();
            for n in &ns {
                t = t.add(n)?;
            }
            let mean = t.div(&Num::int(ns.len() as i64))?;
            let mut sq = Num::zero();
            for n in &ns {
                let d = n.sub(&mean)?;
                sq = sq.add(&d.mul(&d)?)?;
            }
            V::Num(sq.div(&Num::int(ns.len() as i64 - 1))?.sqrt()?)
        }
        "mode" => {
            let l = items(a)?;
            if l.is_empty() {
                return Some(V::list(Vec::new()));
            }
            let mut counts: Vec<(Value, usize)> = Vec::new();
            for v in l.iter() {
                compare(v, v)?;
                match counts.iter_mut().find(|(x, _)| equal(x, v) == Some(true)) {
                    Some(c) => c.1 += 1,
                    None => counts.push((v.clone(), 1)),
                }
            }
            let top = counts.iter().map(|c| c.1).max().unwrap_or(0);
            let modes: Vec<Value> = counts
                .into_iter()
                .filter(|c| c.1 == top)
                .map(|c| c.0)
                .collect();
            V::list(sort_values(ev, &modes, None)?)
        }
        "all" | "any" => {
            let l = items(a)?;
            let mut unknown = false;
            for v in l.iter() {
                match bool_of(v) {
                    Some(b) if b != (name == "all") => return Some(V::Bool(b)),
                    Some(_) => {}
                    None => unknown = true,
                }
            }
            if unknown {
                return None;
            }
            V::Bool(name == "all")
        }
        "sublist" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            let start = int(arg(1))?;
            let len = l.len() as i64;
            let from = if start > 0 { start - 1 } else { len + start };
            if start == 0 || from < 0 || from >= len {
                return None;
            }
            let count = match a.get(1 + 1) {
                Some(v) => int(v)?,
                None => len - from,
            };
            if count < 0 || from + count > len {
                return None;
            }
            V::list(l[from as usize..(from + count) as usize].to_vec())
        }
        "append" => {
            let mut l = match arg(0) {
                V::List(l) => l.as_ref().clone(),
                _ => return None,
            };
            l.extend(a[1..].iter().cloned());
            V::list(l)
        }
        "concatenate" => {
            let mut out = Vec::new();
            for v in a {
                match v {
                    V::List(l) => out.extend(l.iter().cloned()),
                    _ => return None,
                }
            }
            V::list(out)
        }
        "insert before" => {
            let mut l = match arg(0) {
                V::List(l) => l.as_ref().clone(),
                _ => return None,
            };
            let p = int(arg(1))?;
            let len = l.len() as i64;
            let idx = if p > 0 { p - 1 } else { len + p };
            if p == 0 || idx < 0 || idx > len {
                return None;
            }
            l.insert(idx as usize, arg(2).clone());
            V::list(l)
        }
        "remove" => {
            let mut l = match arg(0) {
                V::List(l) => l.as_ref().clone(),
                _ => return None,
            };
            let p = int(arg(1))?;
            let len = l.len() as i64;
            let idx = if p > 0 { p - 1 } else { len + p };
            if p == 0 || idx < 0 || idx >= len {
                return None;
            }
            l.remove(idx as usize);
            V::list(l)
        }
        "reverse" => match arg(0) {
            V::List(l) => V::list(l.iter().rev().cloned().collect()),
            _ => return None,
        },
        "index of" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            V::list(
                l.iter()
                    .enumerate()
                    .filter(|(_, x)| equal(x, arg(1)) == Some(true))
                    .map(|(i, _)| V::int(i as i64 + 1))
                    .collect(),
            )
        }
        "union" => {
            let mut out = Vec::new();
            for v in a {
                match v {
                    V::List(l) => out.extend(l.iter().cloned()),
                    _ => return None,
                }
            }
            V::list(distinct(&out))
        }
        "distinct values" => match arg(0) {
            V::List(l) => V::list(distinct(l)),
            _ => return None,
        },
        "flatten" => match arg(0) {
            V::List(_) => {
                let mut out = Vec::new();
                flatten_into(arg(0), &mut out);
                V::list(out)
            }
            _ => return None,
        },
        "sort" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            match a.get(1) {
                Some(V::Func(f)) => V::list(sort_values(ev, &l, Some(f))?),
                Some(V::Null) | None => V::list(sort_values(ev, &l, None)?),
                _ => return None,
            }
        }
        "list replace" => {
            let mut l = match arg(0) {
                V::List(l) => l.as_ref().clone(),
                _ => return None,
            };
            match arg(1) {
                V::Num(_) => {
                    let p = num(arg(1))?.trunc_i64()?;
                    let len = l.len() as i64;
                    let idx = if p > 0 { p - 1 } else { len + p };
                    if p == 0 || idx < 0 || idx >= len {
                        return None;
                    }
                    l[idx as usize] = arg(2).clone();
                }
                V::Func(f) => {
                    if let Func::Lambda { params, .. } | Func::Boxed { params, .. } = f.as_ref()
                        && params.len() != 2
                    {
                        return None;
                    }
                    for x in l.iter_mut() {
                        match bool_of(&ev.call(f, vec![x.clone(), arg(2).clone()])) {
                            Some(true) => *x = arg(2).clone(),
                            Some(false) => {}
                            None => return None,
                        }
                    }
                }
                _ => return None,
            }
            V::list(l)
        }
        // numbers
        "decimal" => {
            let n = num(arg(0))?;
            V::Num(n.round(num(arg(1))?.trunc_i64()?, Rounding::HalfEven)?)
        }
        "floor" => round_with(a, Rounding::Floor, true),
        "ceiling" => round_with(a, Rounding::Ceiling, true),
        "round up" => round_with(a, Rounding::Up, false),
        "round down" => round_with(a, Rounding::Down, false),
        "round half up" => round_with(a, Rounding::HalfUp, false),
        "round half down" => round_with(a, Rounding::HalfDown, false),
        "abs" => match arg(0) {
            V::Num(n) => V::Num(n.abs()),
            V::Dt(d) => V::Dt(DtDuration(d.0.abs())),
            V::Ym(d) => V::Ym(YmDuration(d.0.abs())),
            _ => return None,
        },
        "modulo" => V::Num(num(arg(0))?.modulo(num(arg(1))?)?),
        "sqrt" => V::Num(num(arg(0))?.sqrt()?),
        "log" => V::Num(num(arg(0))?.ln()?),
        "exp" => V::Num(num(arg(0))?.exp()?),
        "odd" | "even" => {
            let n = num(arg(0))?;
            if !n.is_integer() {
                return None;
            }
            let r = n.modulo(&Num::int(2))?;
            V::Bool(r.is_zero() == (name == "even"))
        }
        // temporal
        "is" => {
            let (x, y) = (arg(0), arg(1));
            V::Bool(match (x, y) {
                (V::Time(p), V::Time(q)) => p == q,
                (V::DateTime(p), V::DateTime(q)) => p == q,
                _ => x.kind() == y.kind() && equal(x, y) == Some(true),
            })
        }
        "day of year" => V::int(date_of(arg(0))?.day_of_year()),
        "day of week" => {
            let names = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            V::str(names[(date_of(arg(0))?.weekday() - 1) as usize])
        }
        "month of year" => {
            let names = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            V::str(names[(date_of(arg(0))?.month - 1) as usize])
        }
        "week of year" => V::int(date_of(arg(0))?.week_of_year()),
        "last day of month" => {
            let d = date_of(arg(0))?;
            V::Date(Date::new(
                d.year,
                i64::from(d.month),
                i64::from(temporal::days_in_month(d.year, d.month)),
            )?)
        }
        "now" => V::DateTime(ev.now.clone()),
        "today" => V::Date(ev.now.date),
        // ranges
        "before" | "after" | "meets" | "met by" | "overlaps" | "overlaps before"
        | "overlaps after" | "finishes" | "finished by" | "includes" | "during" | "starts"
        | "started by" | "coincides" => V::Bool(range_fn(name, arg(0), arg(1))?),
        "range" => {
            let text = s(arg(0))?;
            let e = parse_expression(text, &Names::builtins()).ok()?;
            // the ends of the text of a range are literals: numbers, strings, temporal
            // literals and the conversion functions applied to a string
            let literal = |x: &super::Expr| -> bool {
                use super::Expr as E;
                match x {
                    E::Num(_) | E::Str(_) | E::At(_) => true,
                    E::Neg(n) => matches!(n.as_ref(), E::Num(_)),
                    E::Call(f, args) => {
                        matches!(f.as_ref(), E::Name(n) if ["date", "time", "date and time", "duration"].contains(&n.as_str()))
                            && args.len() == 1
                            && matches!(args[0].1, E::Str(_))
                    }
                    _ => false,
                }
            };
            match &e {
                super::Expr::Range(lo, _, hi, _) => {
                    if ![lo, hi].into_iter().flatten().all(|x| literal(x)) {
                        return None;
                    }
                }
                _ => return None,
            }
            let v = ev.eval(&e, &super::Env::root(Map::new()));
            let V::Range(r) = &v else {
                return None;
            };
            // the ends of a range literal are literals
            for end in [&r.low, &r.high].into_iter().flatten() {
                if matches!(end, V::List(_) | V::Context(_) | V::Func(_) | V::Range(_)) {
                    return None;
                }
            }
            if let (Some(lo), Some(hi)) = (&r.low, &r.high)
                && compare(lo, hi) == Some(Ordering::Greater)
            {
                return None;
            }
            v
        }
        // contexts
        "get value" => match (arg(0), arg(1)) {
            (V::Context(m), V::Str(k)) => m.get(k.as_str()).cloned().unwrap_or(V::Null),
            (V::Context(_), V::List(keys)) => {
                let mut cur = arg(0).clone();
                for k in keys.iter() {
                    cur = match (&cur, k) {
                        (V::Context(m), V::Str(k)) => m.get(k.as_str()).cloned().unwrap_or(V::Null),
                        _ => return None,
                    };
                }
                cur
            }
            _ => return None,
        },
        "get entries" => match arg(0) {
            V::Context(m) => V::list(
                m.iter()
                    .map(|(k, v)| {
                        let mut e = Map::new();
                        e.insert("key".into(), V::Str(k.clone()));
                        e.insert("value".into(), v.clone());
                        V::context(e)
                    })
                    .collect(),
            ),
            _ => return None,
        },
        "context" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                V::Context(_) => Rc::new(vec![arg(0).clone()]),
                _ => return None,
            };
            let mut m = Map::new();
            for e in l.iter() {
                let V::Context(e) = e else {
                    return None;
                };
                let (Some(V::Str(k)), Some(v)) = (e.get("key"), e.get("value")) else {
                    return None;
                };
                if m.contains_key(k.as_str()) {
                    return None;
                }
                m.insert(k.clone(), v.clone());
            }
            V::context(m)
        }
        "context put" => {
            let keys: Vec<String> = match arg(1) {
                V::Str(k) => vec![k.clone()],
                V::List(l) => {
                    let mut ks = Vec::new();
                    for k in l.iter() {
                        ks.push(s(k)?.to_string());
                    }
                    if ks.is_empty() {
                        return None;
                    }
                    ks
                }
                _ => return None,
            };
            put(arg(0), &keys, arg(2))?
        }
        "context merge" => {
            let l = match arg(0) {
                V::List(l) => l.clone(),
                _ => return None,
            };
            let mut m = Map::new();
            for c in l.iter() {
                let V::Context(c) = c else {
                    return None;
                };
                for (k, v) in c.iter() {
                    m.insert(k.clone(), v.clone());
                }
            }
            V::context(m)
        }
        _ => return None,
    })
}

fn put(ctx: &Value, keys: &[String], value: &Value) -> Option<Value> {
    let Value::Context(m) = ctx else {
        return None;
    };
    let mut m = m.as_ref().clone();
    if keys.len() == 1 {
        m.insert(keys[0].clone(), value.clone());
    } else {
        let inner = m
            .get(&keys[0])
            .cloned()
            .unwrap_or_else(|| Value::context(Map::new()));
        let v = put(&inner, &keys[1..], value)?;
        m.insert(keys[0].clone(), v);
    }
    Some(Value::context(m))
}
