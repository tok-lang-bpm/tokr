//! The simulation model of a document as data, for simulators outside this crate (the
//! native simulator of the Julia package reads it through the C interface).
//!
//! The description holds what the simulator of this crate reads from a document: the
//! parameters of `simulate`, the resources and case variables, every flow node with its
//! attributes and parsed distributions, the timers resolved to seconds, and the sequence
//! flows in document order. Distributions are objects `{"name", "kind", "args",
//! "labels"}` with the arguments in seconds for durations, as [`Dist`] holds them.

use tok_core::Document;
use tok_core::json::{Json, obj};
use tok_core::model::{Node, TRIGGERS};
use tok_core::pytime::DateTime;
use tok_core::pyutil::{PyError, parse_float};
use tok_core::simcheck::{Dist, Label, parse_assign, parse_dist};

use crate::sim::{
    DEFAULT_ARRIVAL, DEFAULT_CASES, DEFAULT_START, Param, dist_error, int_of_float, int_text,
    iso_duration, main_scope, start_of,
};

type R<T> = Result<T, PyError>;

/// A distribution as JSON: `{"name", "kind", "args", "labels"}`.
pub fn dist_json(d: &Dist) -> Json {
    obj(vec![
        ("name", Json::Str(d.name.to_string())),
        ("kind", Json::Str(d.kind.to_string())),
        (
            "args",
            Json::Arr(d.args.iter().map(|a| Json::Float(*a)).collect()),
        ),
        (
            "labels",
            Json::Arr(
                d.labels
                    .iter()
                    .map(|l| match l {
                        Label::Str(s) => Json::Str(s.clone()),
                        Label::Bool(b) => Json::Bool(*b),
                        Label::Num(x) => Json::Float(*x),
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Parse a distribution text and describe it.
///
/// # Errors
/// `tok.sim.DistError` when the text is not a distribution.
pub fn describe_dist(text: &str) -> R<Json> {
    parse_dist(text).map(|d| dist_json(&d)).map_err(dist_error)
}

fn opt_dist(text: Option<&str>) -> R<Json> {
    match text {
        Some(t) => describe_dist(t),
        None => Ok(Json::Null),
    }
}

fn opt_str(s: Option<&str>) -> Json {
    s.map(|t| Json::Str(t.to_string())).unwrap_or(Json::Null)
}

/// A timer value: `{"kind": "duration", "seconds"}` for a duration or a cycle (its first
/// period), `{"kind": "date", "seconds"}` for a date-time (seconds from the simulation
/// start; the wait is the difference to the current time, at least 0), `{"kind":
/// "invalid", "text"}` for a value the simulator does not understand (it warns and does
/// not wait), and `{"kind": "error", "message"}` for a date-time that cannot be compared
/// with the simulation start (one with a UTC offset and one without).
fn timer_json(value: &str, t0: &DateTime) -> Json {
    let v = tok_core::pyutil::strip(value);
    let duration = if v.starts_with('R') && v.contains('/') {
        Some(iso_duration(v.rsplit('/').next().unwrap_or("")))
    } else {
        iso_duration(v).map(Some)
    };
    match duration {
        Some(Some(s)) => obj(vec![
            ("kind", Json::Str("duration".into())),
            ("seconds", Json::Float(s)),
        ]),
        Some(None) => obj(vec![
            ("kind", Json::Str("invalid".into())),
            ("text", Json::Str(value.to_string())),
        ]),
        None => match DateTime::fromisoformat(v) {
            Err(_) => obj(vec![
                ("kind", Json::Str("invalid".into())),
                ("text", Json::Str(value.to_string())),
            ]),
            Ok(target) => match target.sub_datetime(t0) {
                Ok(d) => obj(vec![
                    ("kind", Json::Str("date".into())),
                    ("seconds", Json::Float(d.total_seconds())),
                ]),
                Err(e) => obj(vec![
                    ("kind", Json::Str("error".into())),
                    ("message", Json::Str(e.to_string())),
                ]),
            },
        },
    }
}

fn node_json(doc: &Document, n: &Node, t0: &DateTime) -> R<Json> {
    let b = &n.base;
    let attrs = Json::Obj(
        b.attrs
            .iter()
            .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
            .collect(),
    );
    // number of triggers of each kind (several only on multiple events of language 0.5)
    let triggers = Json::Obj(
        TRIGGERS
            .iter()
            .filter_map(|k| {
                let n = b.values(k).len();
                (n > 0).then(|| (k.to_string(), Json::Int(n as i64)))
            })
            .collect(),
    );
    let assign = match b.attr("assign") {
        Some(text) => Json::Arr(
            parse_assign(text)
                .map_err(dist_error)?
                .iter()
                .map(|(name, d)| {
                    obj(vec![
                        ("name", Json::Str(name.clone())),
                        ("dist", dist_json(d)),
                    ])
                })
                .collect(),
        ),
        None => Json::Null,
    };
    let timer = match b.attr("timer") {
        Some(t) => timer_json(t, t0),
        None => Json::Null,
    };
    Ok(obj(vec![
        ("id", Json::Str(b.id().to_string())),
        ("kind", Json::Str(b.kind.clone())),
        ("label", opt_str(b.label.as_deref())),
        ("scope", opt_str(n.scope.as_deref())),
        (
            "children",
            Json::Arr(n.children.iter().map(|c| Json::Str(c.clone())).collect()),
        ),
        ("attrs", attrs),
        (
            "flags",
            Json::Arr(b.flags.iter().map(|f| Json::Str(f.clone())).collect()),
        ),
        ("triggers", triggers),
        ("duration", opt_dist(b.attr("duration"))?),
        ("delay", opt_dist(b.attr("delay"))?),
        ("assign", assign),
        ("timer", timer),
        ("start", opt_str(start_of(doc, b.id()).as_deref())),
    ]))
}

/// The simulation model of a checked document. `cases` and `seed` override the
/// attributes of `simulate`, as in [`crate::simulate`].
///
/// # Errors
/// The errors of [`crate::Simulator::new`] for invalid parameters, and `tok.sim.DistError`
/// for an invalid distribution.
pub fn describe(doc: &Document, cases: Option<Param>, seed: Option<Param>) -> R<Json> {
    let attrs = doc.simulate.as_ref().map(|s| &s.attrs);
    let attr = |k: &str| attrs.and_then(|a| a.get(k)).cloned();
    let cases = cases.unwrap_or_else(|| match attr("cases") {
        Some(t) => Param::Text(t),
        None => Param::Int(DEFAULT_CASES.to_string()),
    });
    let seed = seed.unwrap_or_else(|| match attr("seed") {
        Some(t) => Param::Text(t),
        None => Param::Int("1".into()),
    });
    let n_cases = int_of_float(&cases)?;
    let seed = int_of_float(&seed)?;
    let start = attr("start").unwrap_or_else(|| DEFAULT_START.into());
    let t0 = DateTime::fromisoformat(&start)?;
    let arrival = describe_dist(&attr("arrival").unwrap_or_else(|| DEFAULT_ARRIVAL.into()))?;
    let mut variables = Vec::new();
    for v in &doc.variables {
        variables.push(obj(vec![
            ("name", Json::Str(v.id().to_string())),
            ("dist", describe_dist(v.label.as_deref().unwrap_or(""))?),
        ]));
    }
    let mut resources = Vec::new();
    for r in doc.resources.values() {
        let count = match r.attrs.get("count") {
            Some(t) => int_of_float(&Param::Text(t.clone()))?,
            None => 1.0,
        };
        resources.push(obj(vec![
            ("id", Json::Str(r.id().to_string())),
            ("label", opt_str(r.label.as_deref())),
            ("count", Json::Num(int_text(count))),
        ]));
    }
    let mut nodes = Vec::new();
    for n in doc.nodes.values() {
        nodes.push(node_json(doc, n, &t0)?);
    }
    let flows = doc
        .flows
        .iter()
        .filter(|f| doc.nodes.contains_key(&f.source) && doc.nodes.contains_key(&f.target))
        .map(|f| {
            obj(vec![
                ("id", opt_str(f.base.id.as_deref())),
                ("source", Json::Str(f.source.clone())),
                ("target", Json::Str(f.target.clone())),
                ("when", opt_str(f.base.attr("when"))),
                (
                    "p",
                    f.base
                        .attr("p")
                        .map(|p| Json::Float(parse_float(p).unwrap_or(f64::NAN)))
                        .unwrap_or(Json::Null),
                ),
                ("default", Json::Bool(f.base.has("default"))),
            ])
        })
        .collect();
    let scope = main_scope(doc);
    Ok(obj(vec![
        ("language", opt_str(doc.version.as_deref())),
        ("lang05", Json::Bool(doc.lang_05())),
        ("lang07", Json::Bool(doc.lang_07())),
        ("process", Json::Str(scope.clone())),
        ("start_event", opt_str(start_of(doc, &scope).as_deref())),
        ("start", Json::Str(start)),
        ("cases", Json::Num(int_text(n_cases))),
        ("seed", Json::Num(int_text(seed))),
        ("arrival", arrival),
        ("variables", Json::Arr(variables)),
        ("resources", Json::Arr(resources)),
        ("nodes", Json::Arr(nodes)),
        ("flows", Json::Arr(flows)),
    ]))
}
