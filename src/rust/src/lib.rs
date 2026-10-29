//! R bindings of the tok engine (package `tokr`).
//!
//! The functions are internal: the R functions in `R/tok.R` check their arguments, read
//! files and shape the results (data frames, files, the viewer). Every function takes the
//! text of a document and returns plain strings, vectors or lists.

use extendr_api::prelude::*;

use tok_core::check::{BpmnIssue, BpmnRules};
use tok_core::diagnostics::{Diag, format_diag};
use tok_core::feel_eval::{Context, PyValue, Value};
use tok_core::pydec::Decimal;
use tok_core::pytime::DateTime;
use tok_core::{Document, export, forms, import, printer};
use tok_sim::sim::shift;
use tok_sim::{Param, PyError};

type Res<T> = std::result::Result<T, Error>;

/// BPMN rules from the `tok-bpmn` crate (as in the command line).
struct Rules;

impl BpmnRules for Rules {
    fn validate(&self, xml: &str) -> Vec<BpmnIssue> {
        tok_bpmn::validate(xml)
            .unwrap_or_default()
            .into_iter()
            .map(|i| BpmnIssue {
                level: match i.level {
                    tok_bpmn::Level::Error => "błąd",
                    tok_bpmn::Level::Warning => "ostrzeżenie",
                    tok_bpmn::Level::Note => "uwaga",
                }
                .to_string(),
                code: i.code,
                id: i.id,
                message: i.message,
            })
            .collect()
    }
}

fn fail(message: impl Into<String>) -> Error {
    Error::Other(message.into())
}

/// Universal newlines, like reading a text file in Python or R.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// An error listing the errors among the diagnostics.
fn errors(diags: &[Diag], lang: &str) -> Error {
    fail(
        diags
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format_diag(d, lang, None))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn parse_ok(text: &str, lang: &str) -> Res<Document> {
    let (doc, diags) = tok_core::parse(&normalize(text));
    if diags.iter().any(Diag::is_error) {
        return Err(errors(&diags, lang));
    }
    Ok(doc)
}

fn load_ok(text: &str, lang: &str) -> Res<Document> {
    let (doc, diags) = tok_core::load(&normalize(text), "full", Some(&Rules));
    if diags.iter().any(Diag::is_error) {
        return Err(errors(&diags, lang));
    }
    Ok(doc)
}

fn bpmn_with_layout(doc: &Document) -> Res<String> {
    tok_bpmn::layout(&export::export_xml(doc, None)).map_err(|e| fail(e.to_string()))
}

fn svg_of(doc: &Document, title: Option<&str>) -> Res<String> {
    tok_bpmn::render_svg(&export::export_xml(doc, None), title).map_err(|e| fail(e.to_string()))
}

/// Version of the engine.
#[extendr]
fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Diagnostics as a list of equal-length columns: code, severity, line, id, message.
#[extendr]
fn engine_check(text: &str, lang: &str, level: &str, bpmn: bool) -> List {
    let (_, diags) = tok_core::load(
        &normalize(text),
        level,
        if bpmn { Some(&Rules) } else { None },
    );
    let code: Vec<String> = diags.iter().map(|d| d.code.clone()).collect();
    let severity: Vec<String> = diags
        .iter()
        .map(|d| d.severity().as_str().to_string())
        .collect();
    let line: Vec<i32> = diags
        .iter()
        .map(|d| i32::try_from(d.line).unwrap_or(i32::MAX))
        .collect();
    let id: Vec<Option<String>> = diags.iter().map(|d| d.id.clone()).collect();
    let message: Vec<String> = diags.iter().map(|d| d.message(lang)).collect();
    list!(
        code = code,
        severity = severity,
        line = line,
        id = id,
        message = message
    )
}

/// Canonical text (`tok fmt`).
#[extendr]
fn engine_fmt(text: &str, lang: &str) -> Res<String> {
    Ok(printer::fmt(&parse_ok(text, lang)?))
}

/// BPMN XML with diagram layout (`tok bpmn`).
#[extendr]
fn engine_bpmn(text: &str, lang: &str) -> Res<String> {
    bpmn_with_layout(&load_ok(text, lang)?)
}

/// SVG picture (`tok svg`); an empty title draws none.
#[extendr]
fn engine_svg(text: &str, title: &str, lang: &str) -> Res<String> {
    let doc = load_ok(text, lang)?;
    svg_of(&doc, (!title.is_empty()).then_some(title))
}

/// Every file `tok export` writes, as a named list file name → content.
#[extendr]
fn engine_export(text: &str, name: &str, lang: &str) -> Res<List> {
    let doc = load_ok(text, lang)?;
    let mut names = vec![format!("{name}.bpmn")];
    let mut files = vec![bpmn_with_layout(&doc)?];
    let svg_name = format!("{name}.svg");
    files.push(svg_of(&doc, Some(&svg_name))?);
    names.push(svg_name);
    for (file, content) in tok_core::drg::export::decision_files(&doc) {
        names.push(file);
        files.push(content);
    }
    for form in doc.forms.values() {
        names.push(format!("{}.form", form.base.id()));
        files.push(forms::export_form(form));
    }
    List::from_names_and_values(names, files)
}

/// BPMN 2.0 XML to tok (`tok import`): list(text, warnings).
#[extendr]
fn engine_import(xml: &str, keep_ids: bool) -> Res<List> {
    let (doc, warnings) = import::import_bpmn(xml, keep_ids).map_err(|e| fail(e.to_string()))?;
    Ok(list!(text = printer::fmt(&doc), warnings = warnings))
}

/// One R value as a FEEL value: logical, integer, double and character vectors (a vector
/// longer than one is a list), lists (named: a context), `NULL` and `NA` (null).
fn value_from_r(x: &Robj) -> Res<Value> {
    if x.is_null() {
        return Ok(Value::Null);
    }
    if let Some(list) = x.as_list() {
        return if list.names().is_some() {
            let mut ctx = Context::new();
            for (k, v) in list.iter() {
                ctx.insert(k.to_string(), value_from_r(&v)?);
            }
            Ok(Value::Context(ctx))
        } else {
            list.values()
                .map(|v| value_from_r(&v))
                .collect::<Res<_>>()
                .map(Value::List)
        };
    }
    let items: Vec<Value> = if let Some(v) = x.as_logical_slice() {
        v.iter()
            .map(|b| {
                if b.is_na() {
                    Value::Null
                } else {
                    Value::Bool(b.is_true())
                }
            })
            .collect()
    } else if let Some(v) = x.as_integer_slice() {
        v.iter()
            .map(|i| {
                if i.is_na() {
                    Value::Null
                } else {
                    Value::Num(Decimal::from_i128(i128::from(*i)))
                }
            })
            .collect()
    } else if let Some(v) = x.as_real_slice() {
        v.iter()
            .map(|f| {
                if f.is_na() {
                    return Ok(Value::Null);
                }
                Decimal::parse(&format!("{f}"))
                    .map(Value::Num)
                    .map_err(|e| fail(e.to_string()))
            })
            .collect::<Res<_>>()?
    } else if let Some(v) = x.as_str_iter() {
        v.map(|s| {
            if s.is_na() {
                Value::Null
            } else {
                Value::Str(s.to_string())
            }
        })
        .collect()
    } else {
        return Err(fail(format!(
            "values must be logical, numeric, character or lists, not {:?}",
            x.rtype()
        )));
    };
    Ok(if items.len() == 1 {
        items.into_iter().next().unwrap_or(Value::Null)
    } else {
        Value::List(items)
    })
}

/// A result of a decision as an R value: numbers as doubles, durations as seconds,
/// lists as lists and contexts as named lists.
fn value_to_r(v: &PyValue) -> Robj {
    match v {
        PyValue::None => ().into(),
        PyValue::Bool(b) => (*b).into(),
        PyValue::Int(digits) => digits.parse::<f64>().unwrap_or(f64::NAN).into(),
        PyValue::Float(f) => (*f).into(),
        PyValue::Str(s) => s.as_str().into(),
        PyValue::Duration(d) => {
            ((d.days * 86_400 + d.seconds) as f64 + d.microseconds as f64 / 1e6).into()
        }
        PyValue::List(items) => List::from_values(items.iter().map(value_to_r)).into(),
        PyValue::Dict(pairs) => List::from_names_and_values(
            pairs.iter().map(|(k, _)| k.as_str()),
            pairs.iter().map(|(_, v)| value_to_r(v)),
        )
        .map(Robj::from)
        .unwrap_or_else(|_| ().into()),
    }
}

/// Evaluate a decision table (`tok eval`) with the inputs in a named list.
#[extendr]
fn engine_eval(text: &str, decision: &str, values: List, lang: &str) -> Res<Robj> {
    let (doc, diags) = tok_core::load(&normalize(text), "full", None);
    if diags.iter().any(Diag::is_error) {
        return Err(errors(&diags, lang));
    }
    if !doc.decisions.contains_key(decision) {
        let known: Vec<&str> = doc.decisions.keys().map(String::as_str).collect();
        return Err(fail(format!(
            "unknown decision '{decision}'; the model has: {}",
            known.join(", ")
        )));
    }
    let mut ctx = Context::new();
    for (k, v) in values.iter() {
        ctx.insert(k.to_string(), value_from_r(&v)?);
    }
    // language 0.7: the decision with what it requires; earlier documents: the table alone
    tok_core::drg::values::evaluate_values(&doc, decision, &ctx)
        .map(|r| value_to_r(&r))
        .map_err(|e| fail(e.message))
}

/// `cases` or `seed` of the simulation: `NULL` keeps the value of `simulate`; an integer,
/// a double or a string is passed on as Python would pass it to `int(float(value))`.
fn param_from_r(x: &Robj, name: &str) -> Res<Option<Param>> {
    if x.is_null() {
        return Ok(None);
    }
    if x.len() == 1 {
        if let Some(i) = x.as_integer() {
            return Ok(Some(Param::Int(i.to_string())));
        }
        if let Some(f) = x.as_real() {
            return Ok(Some(Param::Float(f)));
        }
        if let Some(s) = x.as_str() {
            return Ok(Some(Param::Text(s.to_string())));
        }
    }
    Err(fail(format!("`{name}` must be a single number")))
}

/// Seconds since 1970-01-01 UTC of a time; a time without an offset is taken as UTC.
fn epoch(t: &DateTime) -> f64 {
    const UNIX_ORDINAL: i128 = 719_163;
    let local = (i128::from(t.date.ordinal()) - UNIX_ORDINAL) * 86_400_000_000
        + ((i128::from(t.hour) * 60 + i128::from(t.minute)) * 60 + i128::from(t.second))
            * 1_000_000
        + i128::from(t.microsecond);
    let utc = local - t.offset.map(|o| o.total_micros()).unwrap_or(0);
    utc as f64 / 1e6
}

/// A simulated instant (seconds from the start of the simulation) as seconds since
/// 1970-01-01 UTC, through the same calendar arithmetic as the times of the event log.
fn instant(t0: &DateTime, s: Option<f64>) -> Res<Option<f64>> {
    s.map(|s| shift(t0, s).map(|t| epoch(&t)))
        .transpose()
        .map_err(sim_fail)
}

/// An exception of the simulator, as the last line of the traceback `tok sim` prints.
fn sim_fail(e: PyError) -> Error {
    fail(e.to_string())
}

fn r_int(n: u64) -> Option<i32> {
    i32::try_from(n).ok()
}

/// Run the simulation (`tok sim`). The result is a list: `report` (the text `tok sim`
/// prints, in `lang`), `seed`, `horizon_s`, `warnings`, the indicators `started`,
/// `finished`, `by_end_event`, `cycle_time`, `activities` and `resources`, the event log
/// `log` and the table `cases`; tables are lists of equal-length columns, times are
/// seconds since 1970-01-01 UTC.
#[extendr]
fn engine_sim(text: &str, cases: Robj, seed: Robj, lang: &str) -> Res<List> {
    let doc = load_ok(text, lang)?;
    let (cases, seed) = (param_from_r(&cases, "cases")?, param_from_r(&seed, "seed")?);
    let res = tok_sim::simulate(&doc, cases, seed).map_err(sim_fail)?;
    let kpi = res.kpi();
    let o = &res.outcome;

    let by_end_event = list!(
        end_event = kpi
            .by_end_event
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        cases = kpi
            .by_end_event
            .iter()
            .map(|(_, n)| r_int(*n as u64))
            .collect::<Vec<_>>()
    );
    let cycle_time = list!(
        mean_s = kpi.mean_s,
        median_s = kpi.median_s,
        p90_s = kpi.p90_s,
        mean = kpi.mean.as_str(),
        median = kpi.median.as_str(),
        p90 = kpi.p90.as_str()
    );
    let acts = &kpi.activities;
    let activities = list!(
        activity = acts.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        label = acts
            .iter()
            .map(|(_, a)| a.label.as_str())
            .collect::<Vec<_>>(),
        count = acts.iter().map(|(_, a)| r_int(a.count)).collect::<Vec<_>>(),
        mean_wait_s = acts.iter().map(|(_, a)| a.mean_wait_s).collect::<Vec<_>>(),
        mean_processing_s = acts
            .iter()
            .map(|(_, a)| a.mean_processing_s)
            .collect::<Vec<_>>(),
        mean_wait = acts
            .iter()
            .map(|(_, a)| a.mean_wait.as_str())
            .collect::<Vec<_>>(),
        mean_processing = acts
            .iter()
            .map(|(_, a)| a.mean_processing.as_str())
            .collect::<Vec<_>>()
    );
    let ress = &kpi.resources;
    let resources = list!(
        resource = ress.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        label = ress
            .iter()
            .map(|(_, r)| r.label.as_str())
            .collect::<Vec<_>>(),
        count = ress.iter().map(|(_, r)| r.count).collect::<Vec<_>>(),
        utilization = ress.iter().map(|(_, r)| r.utilization).collect::<Vec<_>>()
    );

    let rows = &o.rows;
    let mut enqueue = Vec::with_capacity(rows.len());
    let mut start = Vec::with_capacity(rows.len());
    let mut end = Vec::with_capacity(rows.len());
    for r in rows {
        enqueue.push(instant(&o.t0, Some(r.enqueue))?);
        start.push(instant(&o.t0, r.start)?);
        end.push(instant(&o.t0, r.end)?);
    }
    let log = list!(
        case = rows.iter().map(|r| r_int(r.case)).collect::<Vec<_>>(),
        activity = rows.iter().map(|r| r.activity.as_str()).collect::<Vec<_>>(),
        label = rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        resource = rows.iter().map(|r| r.resource.as_str()).collect::<Vec<_>>(),
        enqueue = enqueue,
        start = start,
        end = end,
        status = rows.iter().map(|r| r.status).collect::<Vec<_>>()
    );

    let cs = &o.cases;
    let mut case_start = Vec::with_capacity(cs.len());
    let mut case_end = Vec::with_capacity(cs.len());
    for c in cs {
        case_start.push(instant(&o.t0, Some(c.start))?);
        case_end.push(instant(&o.t0, c.end)?);
    }
    let variables = List::from_values(cs.iter().map(|c| {
        List::from_names_and_values(
            c.vars.iter().map(|(k, _)| k.as_str()),
            c.vars.iter().map(|(_, v)| value_to_r(v)),
        )
        .map(Robj::from)
        .unwrap_or_else(|_| ().into())
    }));
    let cases = list!(
        case = cs.iter().map(|c| r_int(c.id)).collect::<Vec<_>>(),
        start = case_start,
        end = case_end,
        end_event = cs.iter().map(|c| c.end_event.clone()).collect::<Vec<_>>(),
        cycle_time_s = cs
            .iter()
            .map(|c| c.end.map(|e| tok_sim::result::round(e - c.start, 3)))
            .collect::<Vec<_>>(),
        variables = variables
    );

    Ok(list!(
        report = kpi.format(lang),
        seed = kpi.seed.as_str(),
        horizon_s = kpi.horizon_s,
        warnings = kpi.warnings.clone(),
        started = r_int(kpi.started as u64),
        finished = r_int(kpi.finished as u64),
        by_end_event = by_end_event,
        cycle_time = cycle_time,
        activities = activities,
        resources = resources,
        log = log,
        cases = cases
    ))
}

/// Run the simulation (`tok sim`) and return its event log as an XES document
/// (`tok sim --xes`).
#[extendr]
fn engine_sim_xes(text: &str, cases: Robj, seed: Robj, lang: &str) -> Res<String> {
    let doc = load_ok(text, lang)?;
    let (cases, seed) = (param_from_r(&cases, "cases")?, param_from_r(&seed, "seed")?);
    tok_sim::simulate(&doc, cases, seed)
        .and_then(|res| res.to_xes())
        .map_err(sim_fail)
}

extendr_module! {
    mod tokr;
    fn engine_version;
    fn engine_check;
    fn engine_fmt;
    fn engine_bpmn;
    fn engine_svg;
    fn engine_export;
    fn engine_import;
    fn engine_eval;
    fn engine_sim;
    fn engine_sim_xes;
}
