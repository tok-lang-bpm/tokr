//! The event log of a simulation run in XES (IEEE 1849-2016, eXtensible Event Stream),
//! the exchange format of process-mining tools (ProM, pm4py, bupaR, Disco, Celonis).
//!
//! The document follows the XML schema of IEEE 1849-2016: the elements are in no XML
//! namespace, `xes.version` is the decimal `1.0` (the value OpenXES and ProM write and
//! the schema accepts), and the log declares, in this order, the standard extensions
//! Concept, Time, Lifecycle and Organizational, the global trace and event attributes,
//! the classifiers and the log attributes, followed by one trace per case.
//!
//! - A trace has `concept:name` (the case number), `start`, `end`, `end_event`,
//!   `cycle_time_s` (seconds) and the case variables, typed as XES attributes.
//! - An activity instance gives up to three events with the same `concept:instance` (the
//!   row number of the instance in the event log, from 1): `schedule` when the case
//!   reached the activity, `start` when a performer started the work, and `complete`
//!   when it completed. An instance interrupted by a boundary timer ends with `ati_abort`
//!   (the activity instance was aborted while running) or, when it was interrupted while
//!   waiting for a performer, with `withdraw` (it was withdrawn before it started); these
//!   are the transitions of the standard lifecycle model (`lifecycle:model` `standard`).
//!   The simulator never aborts a whole case on its own, so `pi_abort` does not occur.
//! - Events have `concept:name` (the label of the activity, or its identifier when it
//!   has no label), `concept:instance`, `lifecycle:transition`, `time:timestamp`,
//!   `org:resource` (the resource of `by=`, only when there is one) and `activity` (the
//!   identifier of the activity). Within a trace they are ordered by time; events at the
//!   same time keep the order of the event log and of the lifecycle.
//! - Timestamps are `xs:dateTime` values with milliseconds (truncated, as Python's
//!   `isoformat(timespec="milliseconds")` does) and the UTC offset of `start=` of
//!   `simulate`; a `start=` without an offset gives times without one. An offset that is
//!   not a whole number of minutes or exceeds 14 hours (outside `xs:dateTime`) is
//!   converted to UTC.
//!
//! The output is deterministic: the same result always gives the same bytes.

use std::collections::{HashMap, HashSet};

use tok_core::feel_eval::PyValue;
use tok_core::pytime::DateTime;
use tok_core::pyutil::{PyError, float_repr};

use crate::result::{SimResult, round, write_file};
use crate::sim::shift;

type R<T> = Result<T, PyError>;

/// Value of `xes.version` of the log element.
pub const XES_VERSION: &str = "1.0";

/// The standard extensions the log declares: name, prefix and definition URI.
pub const EXTENSIONS: [(&str, &str, &str); 4] = [
    (
        "Concept",
        "concept",
        "http://www.xes-standard.org/concept.xesext",
    ),
    ("Time", "time", "http://www.xes-standard.org/time.xesext"),
    (
        "Lifecycle",
        "lifecycle",
        "http://www.xes-standard.org/lifecycle.xesext",
    ),
    (
        "Organizational",
        "org",
        "http://www.xes-standard.org/org.xesext",
    ),
];

/// Keys of the trace attributes the simulator writes before the case variables.
pub const TRACE_KEYS: [&str; 5] = ["concept:name", "start", "end", "end_event", "cycle_time_s"];

/// Default value of the global string attributes (the OpenXES convention).
const INVALID: &str = "__INVALID__";
/// Default value of the global date attributes.
const EPOCH: &str = "1970-01-01T00:00:00.000+00:00";

/// A typed XES attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum XesValue {
    Str(String),
    /// an `xs:dateTime` text
    Date(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl XesValue {
    /// The element name of the attribute and the text of its `value`.
    fn parts(&self) -> (&'static str, String) {
        match self {
            XesValue::Str(s) => ("string", s.clone()),
            XesValue::Date(s) => ("date", s.clone()),
            XesValue::Int(i) => ("int", i.to_string()),
            XesValue::Float(f) => ("float", xs_double(*f)),
            XesValue::Bool(b) => ("boolean", b.to_string()),
        }
    }
}

/// An `xs:double` text: Python's `repr` for finite values, `NaN`, `INF` and `-INF`.
fn xs_double(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "INF" } else { "-INF" }.into()
    } else {
        float_repr(f)
    }
}

/// Text escaped for a double-quoted XML attribute value. Characters that XML 1.0 does not
/// allow (control characters other than tab, line feed and carriage return, and the
/// non-characters U+FFFE and U+FFFF) become U+FFFD.
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// A time as `xs:dateTime` with milliseconds and the offset of the time, if any.
pub fn xs_datetime(t: &DateTime) -> R<String> {
    const MAX_OFFSET_US: i128 = 14 * 3600 * 1_000_000;
    let (t, zone) = match t.offset {
        None => (*t, String::new()),
        Some(off) => {
            let us = off.total_micros();
            if us % 60_000_000 == 0 && us.abs() <= MAX_OFFSET_US {
                let minutes = us / 60_000_000;
                let sign = if minutes < 0 { '-' } else { '+' };
                let m = minutes.abs();
                (*t, format!("{sign}{:02}:{:02}", m / 60, m % 60))
            } else {
                (t.sub(&off)?, "+00:00".to_string())
            }
        }
    };
    Ok(format!(
        "{}T{:02}:{:02}:{:02}.{:03}{zone}",
        t.date.isoformat(),
        t.hour,
        t.minute,
        t.second,
        t.microsecond / 1000
    ))
}

/// A string that is exactly an ISO date (`2027-01-04`) or date-time
/// (`2027-01-04T08:00:00`, with seconds and an optional fraction and offset), as FEEL
/// dates and date-times are stored in case variables, as an `xs:dateTime` text.
fn iso_text_date(s: &str) -> Option<String> {
    if s.len() < 10 {
        return None;
    }
    let t = DateTime::fromisoformat(s).ok()?;
    if s == t.date.isoformat() || s == t.isoformat() {
        xs_datetime(&t).ok()
    } else {
        None
    }
}

/// A case variable as an XES value: booleans, integers (as `int` when they fit in 64
/// bits, otherwise as text), floats, durations (as seconds), ISO dates and date-times
/// (as `date`), other text, and lists and contexts as compact JSON text. `None` (a
/// variable without a value) gives no attribute.
pub fn variable_value(v: &PyValue) -> R<Option<XesValue>> {
    Ok(Some(match v {
        PyValue::None => return Ok(None),
        PyValue::Bool(b) => XesValue::Bool(*b),
        PyValue::Int(t) => match t.parse::<i64>() {
            Ok(i) => XesValue::Int(i),
            Err(_) => XesValue::Str(t.clone()),
        },
        PyValue::Float(f) => XesValue::Float(*f),
        PyValue::Str(s) => match iso_text_date(s) {
            Some(d) => XesValue::Date(d),
            None => XesValue::Str(s.clone()),
        },
        PyValue::Duration(d) => XesValue::Float(d.total_seconds()),
        PyValue::List(_) | PyValue::Dict(_) => XesValue::Str(v.to_json()?.compact()),
    }))
}

/// An attribute key (`xs:Name`) for a variable name: letters, digits, `_`, `-` and `.`
/// are kept, any other character (including `:`, which XES reserves for extension
/// prefixes) becomes `_`, and a name that does not start with a letter or `_` gets a
/// leading `_`.
pub fn attribute_key(name: &str) -> String {
    let mut key: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !key.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        key.insert(0, '_');
    }
    key
}

/// The key of a case variable that is unique among the keys already used in the trace:
/// a name taken by a fixed trace attribute gets the prefix `variable_`, any remaining
/// clash a suffix `_2`, `_3`, ….
fn unique_key(name: &str, used: &mut HashSet<String>) -> String {
    let mut key = attribute_key(name);
    if used.contains(&key) {
        key = format!("variable_{key}");
    }
    let base = key.clone();
    let mut n = 2;
    while used.contains(&key) {
        key = format!("{base}_{n}");
        n += 1;
    }
    used.insert(key.clone());
    key
}

struct Writer {
    out: String,
}

impl Writer {
    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..depth {
            self.out.push('\t');
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn attr(&mut self, depth: usize, key: &str, value: &XesValue) {
        let (tag, text) = value.parts();
        self.line(
            depth,
            &format!(
                "<{tag} key=\"{}\" value=\"{}\"/>",
                escape_attr(key),
                escape_attr(&text)
            ),
        );
    }
}

/// One event of an activity instance.
struct Event {
    time: f64,
    row: usize,
    phase: u8,
    transition: &'static str,
}

impl SimResult {
    /// The `xs:dateTime` text of a simulated instant.
    fn xes_time(&self, s: f64) -> R<String> {
        xs_datetime(&shift(&self.outcome.t0, s)?)
    }

    /// The event log as an XES document (IEEE 1849-2016); see the module documentation.
    pub fn to_xes(&self) -> R<String> {
        let o = &self.outcome;
        let mut w = Writer {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"),
        };
        w.line(0, &format!("<log xes.version=\"{XES_VERSION}\">"));
        for (name, prefix, uri) in EXTENSIONS {
            w.line(
                1,
                &format!("<extension name=\"{name}\" prefix=\"{prefix}\" uri=\"{uri}\"/>"),
            );
        }
        w.line(1, "<global scope=\"trace\">");
        w.attr(2, "concept:name", &XesValue::Str(INVALID.into()));
        w.attr(2, "start", &XesValue::Date(EPOCH.into()));
        w.line(1, "</global>");
        w.line(1, "<global scope=\"event\">");
        w.attr(2, "concept:name", &XesValue::Str(INVALID.into()));
        w.attr(2, "concept:instance", &XesValue::Str(INVALID.into()));
        w.attr(2, "lifecycle:transition", &XesValue::Str("complete".into()));
        w.attr(2, "time:timestamp", &XesValue::Date(EPOCH.into()));
        w.attr(2, "activity", &XesValue::Str(INVALID.into()));
        w.line(1, "</global>");
        w.line(1, "<classifier name=\"Activity\" keys=\"concept:name\"/>");
        w.line(
            1,
            "<classifier name=\"Activity_lifecycle\" keys=\"concept:name lifecycle:transition\"/>",
        );
        w.line(1, "<classifier name=\"Activity_id\" keys=\"activity\"/>");
        w.attr(1, "lifecycle:model", &XesValue::Str("standard".into()));
        w.attr(
            1,
            "seed",
            &match o.seed.parse::<i64>() {
                Ok(i) => XesValue::Int(i),
                Err(_) => XesValue::Str(o.seed.clone()),
            },
        );

        let mut rows_of: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, r) in o.rows.iter().enumerate() {
            rows_of.entry(r.case).or_default().push(i);
        }
        for c in &o.cases {
            w.line(1, "<trace>");
            w.attr(2, "concept:name", &XesValue::Str(c.id.to_string()));
            w.attr(2, "start", &XesValue::Date(self.xes_time(c.start)?));
            if let Some(end) = c.end {
                w.attr(2, "end", &XesValue::Date(self.xes_time(end)?));
            }
            if let Some(e) = &c.end_event {
                w.attr(2, "end_event", &XesValue::Str(e.clone()));
            }
            if let Some(end) = c.end {
                w.attr(2, "cycle_time_s", &XesValue::Float(round(end - c.start, 3)));
            }
            let mut used: HashSet<String> = TRACE_KEYS.iter().map(|k| k.to_string()).collect();
            for (name, v) in &c.vars {
                if let Some(value) = variable_value(v)? {
                    let key = unique_key(name, &mut used);
                    w.attr(2, &key, &value);
                }
            }
            let mut events: Vec<Event> = Vec::new();
            for &i in rows_of.get(&c.id).map(Vec::as_slice).unwrap_or_default() {
                let r = &o.rows[i];
                events.push(Event {
                    time: r.enqueue,
                    row: i,
                    phase: 0,
                    transition: "schedule",
                });
                if let Some(start) = r.start {
                    events.push(Event {
                        time: start,
                        row: i,
                        phase: 1,
                        transition: "start",
                    });
                }
                if let Some(end) = r.end {
                    let transition = match (r.status, r.start) {
                        ("completed", _) => "complete",
                        (_, Some(_)) => "ati_abort",
                        (_, None) => "withdraw",
                    };
                    events.push(Event {
                        time: end,
                        row: i,
                        phase: 2,
                        transition,
                    });
                }
            }
            events.sort_by(|a, b| {
                a.time
                    .total_cmp(&b.time)
                    .then(a.row.cmp(&b.row))
                    .then(a.phase.cmp(&b.phase))
            });
            for e in &events {
                let r = &o.rows[e.row];
                let name = if r.label.is_empty() {
                    &r.activity
                } else {
                    &r.label
                };
                w.line(2, "<event>");
                w.attr(3, "concept:name", &XesValue::Str(name.clone()));
                w.attr(
                    3,
                    "concept:instance",
                    &XesValue::Str((e.row + 1).to_string()),
                );
                w.attr(
                    3,
                    "lifecycle:transition",
                    &XesValue::Str(e.transition.into()),
                );
                w.attr(3, "time:timestamp", &XesValue::Date(self.xes_time(e.time)?));
                if !r.resource.is_empty() {
                    w.attr(3, "org:resource", &XesValue::Str(r.resource.clone()));
                }
                w.attr(3, "activity", &XesValue::Str(r.activity.clone()));
                w.line(2, "</event>");
            }
            w.line(1, "</trace>");
        }
        w.line(0, "</log>");
        Ok(w.out)
    }

    /// Write the XES event log to a file (UTF-8, `\n` line endings).
    pub fn write_xes(&self, path: &str) -> R<()> {
        write_file(path, &self.to_xes()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_values_are_escaped() {
        assert_eq!(
            escape_attr("a<b>&\"c\"\t\n\r\u{1}"),
            "a&lt;b&gt;&amp;&quot;c&quot;&#9;&#10;&#13;\u{fffd}"
        );
        assert_eq!(escape_attr("Archiwiści 'x'"), "Archiwiści 'x'");
    }

    #[test]
    fn datetimes_have_milliseconds_and_offsets() {
        let t = DateTime::fromisoformat("2027-01-04T08:00:01.123987").expect("time");
        assert_eq!(xs_datetime(&t).expect("text"), "2027-01-04T08:00:01.123");
        let t = DateTime::fromisoformat("2027-05-03T08:00+02:00").expect("time");
        assert_eq!(
            xs_datetime(&t).expect("text"),
            "2027-05-03T08:00:00.000+02:00"
        );
        let t = DateTime::fromisoformat("2027-05-03T08:00Z").expect("time");
        assert_eq!(
            xs_datetime(&t).expect("text"),
            "2027-05-03T08:00:00.000+00:00"
        );
        let t = DateTime::fromisoformat("2027-05-03T08:00-03:30").expect("time");
        assert_eq!(
            xs_datetime(&t).expect("text"),
            "2027-05-03T08:00:00.000-03:30"
        );
        // offsets outside xs:dateTime are converted to UTC
        let t = DateTime::fromisoformat("2027-05-03T08:00+15:00").expect("time");
        assert_eq!(
            xs_datetime(&t).expect("text"),
            "2027-05-02T17:00:00.000+00:00"
        );
        let t = DateTime::fromisoformat("2027-05-03T08:00+01:00:30").expect("time");
        assert_eq!(
            xs_datetime(&t).expect("text"),
            "2027-05-03T06:59:30.000+00:00"
        );
    }

    #[test]
    fn variables_have_xes_types() {
        let v = |p: PyValue| variable_value(&p).expect("value");
        assert_eq!(v(PyValue::None), None);
        assert_eq!(v(PyValue::Bool(true)), Some(XesValue::Bool(true)));
        assert_eq!(v(PyValue::Int("42".into())), Some(XesValue::Int(42)));
        assert_eq!(
            v(PyValue::Int("123456789012345678901234".into())),
            Some(XesValue::Str("123456789012345678901234".into()))
        );
        assert_eq!(v(PyValue::Float(2.5)), Some(XesValue::Float(2.5)));
        assert_eq!(
            v(PyValue::Str("e-book".into())),
            Some(XesValue::Str("e-book".into()))
        );
        assert_eq!(
            v(PyValue::Str("2027-01-04".into())),
            Some(XesValue::Date("2027-01-04T00:00:00.000".into()))
        );
        assert_eq!(
            v(PyValue::Str("2027-01-04T10:30:00+01:00".into())),
            Some(XesValue::Date("2027-01-04T10:30:00.000+01:00".into()))
        );
        assert_eq!(
            v(PyValue::Str("20270104".into())),
            Some(XesValue::Str("20270104".into()))
        );
        let d = tok_core::pytime::Duration::from_seconds(5400.5).expect("duration");
        assert_eq!(v(PyValue::Duration(d)), Some(XesValue::Float(5400.5)));
        assert_eq!(
            v(PyValue::List(vec![
                PyValue::Int("1".into()),
                PyValue::Str("a".into())
            ])),
            Some(XesValue::Str("[1, \"a\"]".into()))
        );
        assert_eq!(XesValue::Float(f64::NAN).parts().1, "NaN");
        assert_eq!(XesValue::Float(f64::NEG_INFINITY).parts().1, "-INF");
        assert_eq!(XesValue::Float(1e22).parts().1, "1e+22");
    }

    #[test]
    fn keys_are_xml_names_and_unique() {
        assert_eq!(attribute_key("cena"), "cena");
        assert_eq!(attribute_key("zgoda.klienta"), "zgoda.klienta");
        assert_eq!(attribute_key("a b:c"), "a_b_c");
        assert_eq!(attribute_key("1st"), "_1st");
        assert_eq!(attribute_key(""), "_");
        let mut used: HashSet<String> = TRACE_KEYS.iter().map(|k| k.to_string()).collect();
        assert_eq!(unique_key("end", &mut used), "variable_end");
        assert_eq!(unique_key("a b", &mut used), "a_b");
        assert_eq!(unique_key("a_b", &mut used), "variable_a_b");
        assert_eq!(unique_key("a:b", &mut used), "variable_a_b_2");
    }
}
