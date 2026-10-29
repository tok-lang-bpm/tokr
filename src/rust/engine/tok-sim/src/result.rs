//! Results of a simulation (`sim.Result`): the event log, the cases, the indicators
//! (`kpi()`), their JSON and CSV forms and the plain-text report (`format_kpi`), with
//! Python's number formatting.

use tok_core::feel_eval::PyValue;
use tok_core::json::{Json, obj};
use tok_core::pyutil::{PyError, repr_str};

use crate::sim::{Outcome, Row, int_text, shift};

type R<T> = Result<T, PyError>;

/// Python `"%.{prec}f" % x` (and `format(x, ".{prec}f")`): round half to even on the
/// exact binary value, `nan`, `inf` and `-inf` spelled as Python does.
pub fn fixed(x: f64, prec: usize) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    format!("{x:.prec$}")
}

/// Python `round(x, ndigits)` of a float.
pub fn round(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    fixed(x, ndigits).parse::<f64>().unwrap_or(x)
}

/// `sim.human`: a short duration such as `45 s`, `12.5 min`, `3.2 h`, `1.8 d`.
pub fn human(seconds: Option<f64>) -> String {
    let Some(s) = seconds else {
        return String::new();
    };
    if s >= 86400.0 {
        format!("{} d", fixed(s / 86400.0, 1))
    } else if s >= 3600.0 {
        format!("{} h", fixed(s / 3600.0, 1))
    } else if s >= 60.0 {
        format!("{} min", fixed(s / 60.0, 1))
    } else {
        format!("{} s", fixed(s, 0))
    }
}

/// Python's `sum()` of floats (3.12 and newer): the first value, then Neumaier's
/// compensated summation of the others.
pub fn py_sum(values: &[f64]) -> f64 {
    let Some((first, rest)) = values.split_first() else {
        return 0.0;
    };
    let mut f = 0.0 + first;
    let mut c = 0.0f64;
    for &x in rest {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// Sort with Python's `<` only (a merge sort; it never panics on NaN).
fn py_sorted(values: &[f64]) -> Vec<f64> {
    if values.len() <= 1 {
        return values.to_vec();
    }
    let (a, b) = values.split_at(values.len() / 2);
    let (a, b) = (py_sorted(a), py_sorted(b));
    let mut out = Vec::with_capacity(values.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if b[j] < a[i] {
            out.push(b[j]);
            j += 1;
        } else {
            out.push(a[i]);
            i += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// `sim._quantile`.
fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let v = py_sorted(values);
    let k = ((q * v.len() as f64).ceil() as i64 - 1).max(0) as usize;
    v.get(k).copied()
}

/// Indicators of one activity.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityKpi {
    pub label: String,
    pub count: u64,
    pub mean_wait_s: f64,
    pub mean_processing_s: f64,
    pub mean_wait: String,
    pub mean_processing: String,
}

/// Indicators of one resource.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceKpi {
    pub label: String,
    /// an integral float (`int(float(count))`)
    pub count: f64,
    pub utilization: f64,
}

/// The indicators of `Result.kpi()`, in its key order.
#[derive(Debug, Clone, PartialEq)]
pub struct Kpi {
    pub seed: String,
    pub started: usize,
    pub finished: usize,
    pub by_end_event: Vec<(String, usize)>,
    pub mean_s: Option<f64>,
    pub median_s: Option<f64>,
    pub p90_s: Option<f64>,
    pub mean: String,
    pub median: String,
    pub p90: String,
    pub activities: Vec<(String, ActivityKpi)>,
    pub resources: Vec<(String, ResourceKpi)>,
    pub horizon_s: f64,
    pub warnings: Vec<String>,
}

fn opt_float(x: Option<f64>) -> Json {
    x.map(Json::Float).unwrap_or(Json::Null)
}

impl Kpi {
    /// The dict of `Result.kpi()`.
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("seed", Json::Num(self.seed.clone())),
            (
                "cases",
                obj(vec![
                    ("started", Json::Int(self.started as i64)),
                    ("finished", Json::Int(self.finished as i64)),
                    (
                        "by_end_event",
                        Json::Obj(
                            self.by_end_event
                                .iter()
                                .map(|(k, v)| (k.clone(), Json::Int(*v as i64)))
                                .collect(),
                        ),
                    ),
                ]),
            ),
            (
                "cycle_time",
                obj(vec![
                    ("mean_s", opt_float(self.mean_s)),
                    ("median_s", opt_float(self.median_s)),
                    ("p90_s", opt_float(self.p90_s)),
                    ("mean", Json::Str(self.mean.clone())),
                    ("median", Json::Str(self.median.clone())),
                    ("p90", Json::Str(self.p90.clone())),
                ]),
            ),
            (
                "activities",
                Json::Obj(
                    self.activities
                        .iter()
                        .map(|(k, a)| {
                            (
                                k.clone(),
                                obj(vec![
                                    ("label", Json::Str(a.label.clone())),
                                    ("count", Json::Int(a.count as i64)),
                                    ("mean_wait_s", Json::Float(a.mean_wait_s)),
                                    ("mean_processing_s", Json::Float(a.mean_processing_s)),
                                    ("mean_wait", Json::Str(a.mean_wait.clone())),
                                    ("mean_processing", Json::Str(a.mean_processing.clone())),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "resources",
                Json::Obj(
                    self.resources
                        .iter()
                        .map(|(k, r)| {
                            (
                                k.clone(),
                                obj(vec![
                                    ("label", Json::Str(r.label.clone())),
                                    ("count", Json::Num(int_text(r.count))),
                                    ("utilization", Json::Float(r.utilization)),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
            ("horizon_s", Json::Float(self.horizon_s)),
            (
                "warnings",
                Json::Arr(self.warnings.iter().cloned().map(Json::Str).collect()),
            ),
        ])
    }

    /// `sim.format_kpi`: the plain-text report in English (`en`) or Polish (`pl`).
    pub fn format(&self, lang: &str) -> String {
        let pl = lang == "pl";
        let t = |en: &'static str, p: &'static str| if pl { p } else { en };
        let mut out: Vec<String> = Vec::new();
        out.push(if pl {
            format!(
                "Symulacja (ziarno {}): rozpoczęte sprawy {}, zakończone {}",
                self.seed, self.started, self.finished
            )
        } else {
            format!(
                "Simulation (seed {}): {} cases started, {} finished",
                self.seed, self.started, self.finished
            )
        });
        out.push(String::new());
        out.push(t("End events", "Zakończenia").into());
        for (k, v) in &self.by_end_event {
            let share = 100.0 * *v as f64 / self.finished.max(1) as f64;
            out.push(format!("  {k:<30} {v:>6}  {:>5}%", fixed(share, 1)));
        }
        out.push(String::new());
        out.push(format!(
            "{} {}, {} {}, {} {}",
            t("Cycle time: mean", "Czas cyklu: średnio"),
            self.mean,
            t("median", "mediana"),
            self.median,
            t("90th percentile", "90. percentyl"),
            self.p90
        ));
        out.push(String::new());
        out.push(format!(
            "{}  ({})",
            t("Activities", "Czynności"),
            t(
                "count, mean wait, mean processing",
                "liczba, średnie oczekiwanie, średnie wykonanie"
            )
        ));
        for (k, a) in &self.activities {
            let name = if a.label.is_empty() { k } else { &a.label };
            let name: String = name.chars().take(30).collect();
            out.push(format!(
                "  {name:<30} {:>6}  {:>10}  {:>10}",
                a.count, a.mean_wait, a.mean_processing
            ));
        }
        if !self.resources.is_empty() {
            out.push(String::new());
            out.push(t("Resources (utilization)", "Zasoby (wykorzystanie)").into());
            for (k, r) in &self.resources {
                let name = if r.label.is_empty() { k } else { &r.label };
                let name: String = name.chars().take(30).collect();
                out.push(format!(
                    "  {name:<30} x{:<3} {:>5}%",
                    int_text(r.count),
                    fixed(100.0 * r.utilization, 1)
                ));
            }
        }
        for w in &self.warnings {
            out.push(format!("{}{w}", t("warning: ", "ostrzeżenie: ")));
        }
        out.join("\n") + "\n"
    }
}

/// One row of the event log with ISO timestamps (`Result.log`).
#[derive(Debug, Clone, PartialEq)]
pub struct LogRow {
    pub case: u64,
    pub activity: String,
    pub label: String,
    pub resource: String,
    pub enqueue: String,
    pub start: String,
    pub end: String,
    pub status: String,
}

/// One case (`Result.cases`).
#[derive(Debug, Clone)]
pub struct CaseRow {
    pub case: u64,
    pub start: String,
    pub end: String,
    pub end_event: String,
    pub cycle_time_s: Option<f64>,
    pub variables: Vec<(String, PyValue)>,
}

impl CaseRow {
    fn variables_json(&self) -> R<Json> {
        Ok(Json::Obj(
            self.variables
                .iter()
                .map(|(k, v)| Ok((k.clone(), v.to_json()?)))
                .collect::<R<_>>()?,
        ))
    }

    pub fn to_json(&self) -> R<Json> {
        Ok(obj(vec![
            ("case", Json::Int(self.case as i64)),
            ("start", Json::Str(self.start.clone())),
            ("end", Json::Str(self.end.clone())),
            ("end_event", Json::Str(self.end_event.clone())),
            ("cycle_time_s", opt_float(self.cycle_time_s)),
            ("variables", self.variables_json()?),
        ]))
    }
}

impl LogRow {
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("case", Json::Int(self.case as i64)),
            ("activity", Json::Str(self.activity.clone())),
            ("label", Json::Str(self.label.clone())),
            ("resource", Json::Str(self.resource.clone())),
            ("enqueue", Json::Str(self.enqueue.clone())),
            ("start", Json::Str(self.start.clone())),
            ("end", Json::Str(self.end.clone())),
            ("status", Json::Str(self.status.clone())),
        ])
    }
}

/// The result of a simulation run (`sim.Result`).
#[derive(Debug, Clone)]
pub struct SimResult {
    pub outcome: Outcome,
}

/// One CSV field as Python's `csv` module writes it (`QUOTE_MINIMAL`).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_line(fields: &[String]) -> String {
    let mut line = fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",");
    line.push_str("\r\n");
    line
}

/// The exception of Python's `open(path, "w")` for an I/O error.
pub fn os_error(e: &std::io::Error, path: &str) -> PyError {
    use std::io::ErrorKind;
    let (kind, errno, text) = match e.kind() {
        ErrorKind::NotFound => ("FileNotFoundError", 2, "No such file or directory"),
        ErrorKind::PermissionDenied => ("PermissionError", 13, "Permission denied"),
        ErrorKind::IsADirectory => ("IsADirectoryError", 21, "Is a directory"),
        _ => {
            return PyError::new(
                "OSError",
                format!(
                    "[Errno {}] {e}: {}",
                    e.raw_os_error().unwrap_or(0),
                    repr_str(path)
                ),
            );
        }
    };
    PyError::new(kind, format!("[Errno {errno}] {text}: {}", repr_str(path)))
}

pub(crate) fn write_file(path: &str, text: &str) -> R<()> {
    if path.is_empty() {
        let e = std::io::Error::from(std::io::ErrorKind::NotFound);
        return Err(os_error(&e, path));
    }
    std::fs::write(path, text).map_err(|e| os_error(&e, path))
}

impl SimResult {
    /// `sim.Result._iso`: the ISO time (to the second) of a simulated instant.
    fn iso(&self, s: Option<f64>) -> R<String> {
        match s {
            None => Ok(String::new()),
            Some(s) => Ok(shift(&self.outcome.t0, s)?.isoformat_seconds()),
        }
    }

    /// `Result.log`.
    pub fn log(&self) -> R<Vec<LogRow>> {
        self.outcome
            .rows
            .iter()
            .map(|r: &Row| {
                Ok(LogRow {
                    case: r.case,
                    activity: r.activity.clone(),
                    label: r.label.clone(),
                    resource: r.resource.clone(),
                    enqueue: self.iso(Some(r.enqueue))?,
                    start: self.iso(r.start)?,
                    end: self.iso(r.end)?,
                    status: r.status.to_string(),
                })
            })
            .collect()
    }

    /// `Result.cases`.
    pub fn cases(&self) -> R<Vec<CaseRow>> {
        self.outcome
            .cases
            .iter()
            .map(|c| {
                let ct = c.end.map(|e| e - c.start);
                Ok(CaseRow {
                    case: c.id,
                    start: self.iso(Some(c.start))?,
                    end: self.iso(c.end)?,
                    end_event: c.end_event.clone().unwrap_or_default(),
                    cycle_time_s: ct.map(|x| round(x, 3)),
                    variables: c.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                })
            })
            .collect()
    }

    /// `Result.kpi()`.
    pub fn kpi(&self) -> Kpi {
        let o = &self.outcome;
        let finished: Vec<_> = o.cases.iter().filter(|c| c.end.is_some()).collect();
        let mut by_end: Vec<(String, usize)> = Vec::new();
        for c in &finished {
            let key = c.end_event.clone().unwrap_or_default();
            match by_end.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 += 1,
                None => by_end.push((key, 1)),
            }
        }
        let ct: Vec<f64> = finished
            .iter()
            .map(|c| c.end.unwrap_or(c.start) - c.start)
            .collect();
        let mean = (!ct.is_empty()).then(|| py_sum(&ct) / ct.len() as f64);
        let median = quantile(&ct, 0.5);
        let p90 = quantile(&ct, 0.9);
        // (label, count, total wait, total processing) per activity, in order of first completion
        let mut acts: Vec<(String, (String, u64, f64, f64))> = Vec::new();
        for r in &o.rows {
            if r.status != "completed" {
                continue;
            }
            let pos = match acts.iter().position(|(k, _)| *k == r.activity) {
                Some(p) => p,
                None => {
                    acts.push((r.activity.clone(), (r.label.clone(), 0, 0.0, 0.0)));
                    acts.len() - 1
                }
            };
            let d = &mut acts[pos].1;
            let start = r.start.unwrap_or(r.enqueue);
            d.1 += 1;
            d.2 += start - r.enqueue;
            d.3 += r.end.unwrap_or(start) - start;
        }
        let activities = acts
            .into_iter()
            .map(|(k, (label, n, w, p))| {
                let mean_wait_s = round(w / n as f64, 3);
                let mean_processing_s = round(p / n as f64, 3);
                (
                    k,
                    ActivityKpi {
                        label,
                        count: n,
                        mean_wait_s,
                        mean_processing_s,
                        mean_wait: human(Some(mean_wait_s)),
                        mean_processing: human(Some(mean_processing_s)),
                    },
                )
            })
            .collect();
        let resources = o
            .resources
            .iter()
            .map(|r| {
                let cap = r.count * o.horizon;
                let label = match &r.label {
                    Some(l) if !l.is_empty() => l.clone(),
                    _ => r.id.clone(),
                };
                (
                    r.id.clone(),
                    ResourceKpi {
                        label,
                        count: r.count,
                        utilization: if cap > 0.0 {
                            round(r.busy_time / cap, 4)
                        } else {
                            0.0
                        },
                    },
                )
            })
            .collect();
        let mut warnings: Vec<String> = Vec::new();
        for w in &o.warnings {
            if !warnings.contains(w) {
                warnings.push(w.clone());
            }
        }
        Kpi {
            seed: o.seed.clone(),
            started: o.cases.len(),
            finished: finished.len(),
            by_end_event: by_end,
            mean_s: mean.map(|m| round(m, 3)),
            median_s: median.map(|m| round(m, 3)),
            p90_s: p90.map(|m| round(m, 3)),
            mean: human(mean),
            median: human(median),
            p90: human(p90),
            activities,
            resources,
            horizon_s: round(o.horizon, 3),
            warnings,
        }
    }

    /// `Result.to_json(include_log)`: indicators, cases and optionally the log.
    pub fn to_json(&self, include_log: bool) -> R<String> {
        let kpi = self.kpi().to_json();
        let cases = Json::Arr(
            self.cases()?
                .iter()
                .map(CaseRow::to_json)
                .collect::<R<_>>()?,
        );
        let mut items = vec![("kpi", kpi), ("cases", cases)];
        if include_log {
            items.push((
                "log",
                Json::Arr(self.log()?.iter().map(LogRow::to_json).collect()),
            ));
        }
        Ok(obj(items).pretty())
    }

    /// The event log as CSV text, or the header and the error that stopped it.
    pub fn log_csv(&self) -> (String, R<()>) {
        let header = csv_line(
            &[
                "case", "activity", "label", "resource", "enqueue", "start", "end", "status",
            ]
            .map(String::from),
        );
        let rows = match self.log() {
            Ok(rows) => rows,
            Err(e) => return (header, Err(e)),
        };
        let mut text = header;
        for r in rows {
            text.push_str(&csv_line(&[
                r.case.to_string(),
                r.activity,
                r.label,
                r.resource,
                r.enqueue,
                r.start,
                r.end,
                r.status,
            ]));
        }
        (text, Ok(()))
    }

    /// The table of cases as CSV text, or the header and the error that stopped it.
    pub fn cases_csv(&self) -> (String, R<()>) {
        let header = csv_line(
            &[
                "case",
                "start",
                "end",
                "end_event",
                "cycle_time_s",
                "variables",
            ]
            .map(String::from),
        );
        let mut text = header.clone();
        let rows = match self.cases() {
            Ok(rows) => rows,
            Err(e) => return (header, Err(e)),
        };
        for c in rows {
            let vars = match c.variables_json() {
                Ok(j) => j.compact(),
                Err(e) => return (text, Err(e)),
            };
            text.push_str(&csv_line(&[
                c.case.to_string(),
                c.start,
                c.end,
                c.end_event,
                c.cycle_time_s
                    .map(tok_core::pyutil::float_repr)
                    .unwrap_or_default(),
                vars,
            ]));
        }
        (text, Ok(()))
    }

    /// `Result.write_csv(path_log, path_cases)`. `None` for the log is the `TypeError`
    /// of `open(None)`; an empty path for the cases skips that file.
    pub fn write_csv(&self, path_log: Option<&str>, path_cases: Option<&str>) -> R<()> {
        let Some(path_log) = path_log else {
            return Err(PyError::new(
                "TypeError",
                "expected str, bytes or os.PathLike object, not NoneType",
            ));
        };
        let (text, res) = self.log_csv();
        write_file(path_log, &text)?;
        res?;
        if let Some(p) = path_cases.filter(|p| !p.is_empty()) {
            let (text, res) = self.cases_csv();
            write_file(p, &text)?;
            res?;
        }
        Ok(())
    }
}
