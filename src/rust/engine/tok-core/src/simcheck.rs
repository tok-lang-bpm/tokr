//! Distributions of the simulation statements and the simulation parameter checks
//! (S-codes): `parse_dist`, `parse_assign` and `check_sim` of `sim.py`. The simulator
//! (crate `tok-sim`) samples the distributions parsed here.

use crate::model::Document;
use crate::pyutil::{decimal_value, fmt_g, parse_float, round6_repr, strip};

/// (code, line, element id, arguments)
pub type SimCheck = (
    &'static str,
    usize,
    Option<String>,
    Vec<(&'static str, String)>,
);

/// A label of `cat()`: text, a boolean (`true`, `false`) or a number.
#[derive(Debug, Clone, PartialEq)]
pub enum Label {
    Str(String),
    Bool(bool),
    Num(f64),
}

/// A parsed distribution (`sim.Dist`). `kind` is `duration`, `number`, `bool` or `cat`;
/// `args` are the parameters in seconds or as numbers (the probabilities for `cat`).
#[derive(Debug, Clone, PartialEq)]
pub struct Dist {
    pub name: &'static str,
    pub kind: &'static str,
    pub args: Vec<f64>,
    pub labels: Vec<Label>,
}

fn is_dur_unit(c: char) -> bool {
    matches!(c, 's' | 'm' | 'h' | 'd')
}

/// A digit of Python's regular expressions (`\d`: any Unicode decimal digit).
fn is_digit(c: char) -> bool {
    decimal_value(c).is_some()
}

/// `\d+(?:\.\d+)?`
fn is_plain_number(t: &str) -> bool {
    let (int, frac) = match t.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (t, None),
    };
    !int.is_empty()
        && int.chars().all(is_digit)
        && frac
            .map(|f| !f.is_empty() && f.chars().all(is_digit))
            .unwrap_or(true)
}

/// Python `float()` of a text matched by `-?\d+(?:\.\d+)?`: digits of any script.
fn plain_float(t: &str) -> f64 {
    let ascii: String = t
        .chars()
        .map(|c| match decimal_value(c) {
            Some(d) => char::from(b'0' + d as u8),
            None => c,
        })
        .collect();
    ascii.parse::<f64>().unwrap_or(0.0)
}

/// `_NUM_RE`: `^-?\d+(?:\.\d+)?$`
fn is_signed_number(t: &str) -> bool {
    is_plain_number(t.strip_prefix('-').unwrap_or(t))
}

/// ('duration' | 'number', value) or an error detail.
fn scalar(text: &str) -> Result<(&'static str, f64), String> {
    let t = strip(text);
    if let Some(unit) = t.chars().last()
        && is_dur_unit(unit)
    {
        let num = strip(&t[..t.len() - 1]);
        let num_raw = &t[..t.len() - 1];
        // `^(\d+(?:\.\d+)?)\s*([smhd])$`: whitespace only between number and unit
        if is_plain_number(num)
            && num_raw.trim_end_matches(crate::pyutil::is_space) == num
            && !num.is_empty()
        {
            let mult = match unit {
                's' => 1.0,
                'm' => 60.0,
                'h' => 3600.0,
                _ => 86400.0,
            };
            return Ok(("duration", plain_float(num) * mult));
        }
    }
    if is_signed_number(t) {
        return Ok(("number", plain_float(t)));
    }
    Err(format!("'{t}' is neither a number nor a duration like 30m"))
}

/// `_label_value`: a quoted text, `true`, `false`, a number or the bare text.
fn label_value(text: &str) -> Label {
    let t = strip(text);
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        return Label::Str(chars[1..chars.len() - 1].iter().collect());
    }
    match t {
        "true" => Label::Bool(true),
        "false" => Label::Bool(false),
        _ if is_signed_number(t) => Label::Num(plain_float(t)),
        _ => Label::Str(t.to_string()),
    }
}

/// `_split_args`: split at commas outside parentheses; the parts are stripped.
pub fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i64;
    let mut buf = String::new();
    for c in text.chars() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        }
        if c == ',' && depth == 0 {
            out.push(strip(&buf).to_string());
            buf.clear();
        } else {
            buf.push(c);
        }
    }
    let last = strip(&buf).to_string();
    if !last.is_empty() || !out.is_empty() {
        out.push(last);
    }
    out
}

/// `^([a-z]+)\s*\((.*)\)$` (dot matches newlines).
fn call(t: &str) -> Option<(&str, &str)> {
    let name_end = t.find(|c: char| !c.is_ascii_lowercase()).unwrap_or(t.len());
    if name_end == 0 {
        return None;
    }
    let rest = t[name_end..].trim_start_matches(crate::pyutil::is_space);
    let body = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some((&t[..name_end], body))
}

/// The name of a distribution with its number of arguments.
fn arity(name: &str) -> Option<(&'static str, usize)> {
    Some(match name {
        "fixed" => ("fixed", 1),
        "exp" => ("exp", 1),
        "bern" => ("bern", 1),
        "unif" => ("unif", 2),
        "norm" => ("norm", 2),
        "lnorm" => ("lnorm", 2),
        "tri" => ("tri", 3),
        _ => return None,
    })
}

/// Parse a distribution (`sim.parse_dist`): a duration (`30m`), a number, or a call such
/// as `tri(5m, 10m, 20m)`. The error is the detail text of `DistError`.
pub fn parse_dist(text: &str) -> Result<Dist, String> {
    if strip(text).is_empty() {
        return Err("empty distribution".into());
    }
    let t = strip(text);
    let Some((name, body)) = call(t) else {
        let (kind, v) = scalar(t)?;
        return Ok(Dist {
            name: "fixed",
            kind,
            args: vec![v],
            labels: Vec::new(),
        });
    };
    let args = split_args(body);
    if name == "cat" {
        if args.is_empty() {
            return Err("cat() needs at least one label=probability".into());
        }
        let mut labels = Vec::new();
        let mut probs = Vec::new();
        for a in &args {
            let Some((label, p)) = a.rsplit_once('=') else {
                return Err(format!(
                    "cat() arguments must be label=probability, got '{a}'"
                ));
            };
            let (kind, pv) = scalar(p)?;
            if kind != "number" || !(0.0..=1.0).contains(&pv) {
                return Err(format!(
                    "probability of '{}' must be a number between 0 and 1",
                    strip(label)
                ));
            }
            labels.push(label_value(label));
            probs.push(pv);
        }
        let total = probs.iter().fold(0.0, |acc, p| acc + p);
        if (total - 1.0).abs() > 1e-6 {
            return Err(format!(
                "cat() probabilities sum to {}, not 1",
                fmt_g(total)
            ));
        }
        return Ok(Dist {
            name: "cat",
            kind: "cat",
            args: probs,
            labels,
        });
    }
    let Some((name, n)) = arity(name) else {
        return Err(format!("unknown distribution '{name}'"));
    };
    if args.len() != n {
        return Err(format!(
            "{name}() takes {n} argument(s), got {}",
            args.len()
        ));
    }
    let mut parsed = Vec::new();
    for a in &args {
        parsed.push(scalar(a)?);
    }
    let kind = parsed[0].0;
    if parsed.iter().any(|p| p.0 != kind) {
        return Err(format!("mix of durations and numbers in {name}()"));
    }
    let v: Vec<f64> = parsed.iter().map(|p| p.1).collect();
    if name == "bern" {
        if kind != "number" || !(0.0..=1.0).contains(&v[0]) {
            return Err("bern(p) needs a probability between 0 and 1".into());
        }
        return Ok(Dist {
            name,
            kind: "bool",
            args: v,
            labels: Vec::new(),
        });
    }
    if name == "unif" && v[0] > v[1] {
        return Err("unif(a, b) needs a <= b".into());
    }
    if name == "tri" && !(v[0] <= v[1] && v[1] <= v[2]) {
        return Err("tri(a, mode, b) needs a <= mode <= b".into());
    }
    if (name == "norm" || name == "lnorm") && v[1] < 0.0 {
        return Err("standard deviation must not be negative".into());
    }
    if (name == "lnorm" || name == "exp") && v[0] <= 0.0 {
        return Err(format!("{name}() needs a positive mean"));
    }
    Ok(Dist {
        name,
        kind,
        args: v,
        labels: Vec::new(),
    })
}

fn is_var_name(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse `assign="a=bern(0.8); b=cat(x=0.5, y=0.5)"` into (name, distribution) pairs.
pub fn parse_assign(text: &str) -> Result<Vec<(String, Dist)>, String> {
    let mut out = Vec::new();
    for part in text.split(';') {
        let part = strip(part);
        if part.is_empty() {
            continue;
        }
        let Some((name, dist)) = part.split_once('=') else {
            return Err(format!("assign needs NAME=DIST, got '{part}'"));
        };
        let name = strip(name);
        if !is_var_name(name) {
            return Err(format!("'{name}' is not a variable name"));
        }
        out.push((name.to_string(), parse_dist(dist)?));
    }
    Ok(out)
}

fn dist_check(out: &mut Vec<SimCheck>, text: &str, line: usize, id: Option<&str>, assign: bool) {
    let res = if assign {
        parse_assign(text).map(|_| ())
    } else {
        parse_dist(text).map(|_| ())
    };
    if let Err(detail) = res {
        out.push((
            "S001",
            line,
            id.map(str::to_string),
            vec![("text", text.to_string()), ("detail", detail)],
        ));
    }
}

fn prob(text: &str) -> Option<f64> {
    parse_float(text)
}

/// Simulation diagnostics (`sim.check_sim`).
pub fn check_sim(doc: &Document) -> Vec<SimCheck> {
    let mut out = Vec::new();
    if let Some(s) = &doc.simulate
        && let Some(a) = s.attrs.get("arrival")
    {
        dist_check(&mut out, a, s.line, None, false);
    }
    for v in &doc.variables {
        dist_check(
            &mut out,
            v.label.as_deref().unwrap_or(""),
            v.line,
            v.id.as_deref(),
            false,
        );
    }
    for n in doc.nodes.values() {
        for key in ["duration", "delay"] {
            if let Some(t) = n.base.attrs.get(key) {
                dist_check(&mut out, t, n.base.line, n.base.id.as_deref(), false);
            }
        }
        if let Some(t) = n.base.attrs.get("assign") {
            dist_check(&mut out, t, n.base.line, n.base.id.as_deref(), true);
        }
        if let Some(by) = n.base.attrs.get("by")
            && !doc.resources.contains_key(by)
        {
            out.push((
                "R002",
                n.base.line,
                n.base.id.clone(),
                vec![("what", "resource".into()), ("id", by.clone())],
            ));
        }
    }
    for m in &doc.messages {
        if let Some(t) = m.base.attrs.get("delay") {
            dist_check(&mut out, t, m.base.line, m.base.id.as_deref(), false);
        }
    }
    let mut outgoing: indexmap::IndexMap<&str, Vec<&crate::model::Flow>> = Default::default();
    for f in &doc.flows {
        outgoing.entry(f.source.as_str()).or_default().push(f);
        if let Some(p) = f.base.attrs.get("p") {
            let ok = prob(p).map(|x| (0.0..=1.0).contains(&x)).unwrap_or(false);
            if !ok {
                out.push((
                    "S001",
                    f.base.line,
                    f.base.id.clone(),
                    vec![
                        ("text", format!("p={p}")),
                        ("detail", "a probability must be between 0 and 1".into()),
                    ],
                ));
            }
        }
    }
    // the gateways of choreographies are not simulated (language 0.6)
    for n in doc.nodes.values().filter(|n| n.choreography.is_none()) {
        let kind = n.base.kind.as_str();
        if kind != "xor" && kind != "or" {
            continue;
        }
        let flows = outgoing.get(n.base.id()).cloned().unwrap_or_default();
        let ps: Vec<_> = flows
            .iter()
            .filter(|f| f.base.attrs.contains_key("p"))
            .collect();
        if kind == "xor" && !ps.is_empty() {
            let mut total = Some(0.0);
            for f in &ps {
                match prob(&f.base.attrs["p"]) {
                    Some(x) => total = total.map(|t| t + x),
                    None => {
                        total = None;
                        break;
                    }
                }
            }
            if let Some(t) = total
                && (t - 1.0).abs() > 1e-6
            {
                out.push((
                    "S002",
                    n.base.line,
                    n.base.id.clone(),
                    vec![("id", n.base.id().to_string()), ("total", round6_repr(t))],
                ));
            }
        }
        if doc.simulate.is_some()
            && flows.len() >= 2
            && ps.is_empty()
            && !flows.iter().any(|f| f.base.attrs.contains_key("when"))
        {
            out.push((
                "S003",
                n.base.line,
                n.base.id.clone(),
                vec![("id", n.base.id().to_string())],
            ));
        }
    }
    out
}
