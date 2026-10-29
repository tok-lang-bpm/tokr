//! The discrete-event simulator (`sim.Simulator`).
//!
//! The simulator runs one process: the process of the first pool that has one, or the
//! top-level process. Time is kept in seconds from the simulation start. Events wait in a
//! binary heap ordered by (time, insertion number), exactly as `heapq` orders the tuples
//! of `sim.py`, and every random draw comes from one [`PyRandom`] in the same order as in
//! Python, so the same model and seed give the same event log.

use std::collections::HashMap;

use indexmap::IndexMap;

use tok_core::Document;
use tok_core::feel_eval::PyValue;
use tok_core::model::{GATEWAY_KINDS, Node, TRIGGERS, is_activity};
use tok_core::pytime::{DateTime, Duration};
use tok_core::pyutil::{PyError, parse_float, repr_str};
use tok_core::simcheck::{Dist, Label, parse_assign, parse_dist};

use crate::fallback::{Vars, eval_decision, eval_expr};
use crate::pyrandom::PyRandom;
use crate::pyval;

type R<T> = Result<T, PyError>;

/// A case that passes more elements than this is stopped (`loop-limit`).
pub const MAX_STEPS: u64 = 10000;
/// Simulated time of the first arrival when `simulate` has no `start=`.
pub const DEFAULT_START: &str = "2027-01-04T08:00";
/// Time between arrivals when `simulate` has no `arrival=`.
pub const DEFAULT_ARRIVAL: &str = "exp(1h)";
/// Number of cases when neither the caller nor `simulate` gives one.
pub const DEFAULT_CASES: i64 = 100;

/// `sim.DistError`.
pub fn dist_error(detail: String) -> PyError {
    PyError::new("tok.sim.DistError", detail)
}

/// `sim.SimError`.
pub fn sim_error(msg: String) -> PyError {
    PyError::new("tok.sim.SimError", msg)
}

/// A number of cases or a seed as Python receives it: an int (the command line), a float
/// or a text (an attribute of `simulate`). The simulator takes `int(float(value))`.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    /// An integer as decimal digits, with an optional sign.
    Int(String),
    Float(f64),
    Text(String),
}

/// `int(float(value))` as a float holding the integer.
pub fn int_of_float(p: &Param) -> R<f64> {
    let f = match p {
        Param::Int(digits) => {
            let f = digits.parse::<f64>().unwrap_or(f64::NAN);
            if f.is_infinite() {
                return Err(PyError::new(
                    "OverflowError",
                    "int too large to convert to float",
                ));
            }
            f
        }
        Param::Float(f) => *f,
        Param::Text(t) => parse_float(t).ok_or_else(|| {
            PyError::new(
                "ValueError",
                format!("could not convert string to float: {}", repr_str(t)),
            )
        })?,
    };
    if f.is_nan() {
        return Err(PyError::new(
            "ValueError",
            "cannot convert float NaN to integer",
        ));
    }
    if f.is_infinite() {
        return Err(PyError::new(
            "OverflowError",
            "cannot convert float infinity to integer",
        ));
    }
    Ok(f.trunc() + 0.0)
}

/// `str(int)` of an integral float.
pub fn int_text(f: f64) -> String {
    format!("{:.0}", f + 0.0)
}

/// A value drawn from a distribution: a number (seconds for durations), a boolean
/// (`bern`) or a `cat` label.
#[derive(Debug, Clone, PartialEq)]
pub enum Sample {
    Num(f64),
    Bool(bool),
    Str(String),
}

impl Sample {
    pub fn to_py(&self) -> PyValue {
        match self {
            Sample::Num(x) => PyValue::Float(*x),
            Sample::Bool(b) => PyValue::Bool(*b),
            Sample::Str(s) => PyValue::Str(s.clone()),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Sample::Num(_) => "float",
            Sample::Bool(_) => "bool",
            Sample::Str(_) => "str",
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Sample::Num(x) => Some(*x),
            Sample::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Sample::Str(_) => None,
        }
    }

    /// `max(0.0, self)` as the delay of an event.
    fn delay(&self) -> R<f64> {
        match self {
            Sample::Num(x) => Ok(if *x > 0.0 { *x } else { 0.0 }),
            Sample::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Sample::Str(_) => Err(PyError::new(
                "TypeError",
                "'>' not supported between instances of 'str' and 'float'",
            )),
        }
    }

    /// `self <= 0`.
    fn le_zero(&self) -> R<bool> {
        match self.number() {
            Some(x) => Ok(x <= 0.0),
            None => Err(PyError::new(
                "TypeError",
                "'<=' not supported between instances of 'str' and 'int'",
            )),
        }
    }

    /// `self < other`.
    fn lt(&self, other: &Sample) -> R<bool> {
        match (self, other) {
            (Sample::Str(a), Sample::Str(b)) => Ok(a < b),
            _ => match (self.number(), other.number()) {
                (Some(a), Some(b)) => Ok(a < b),
                _ => Err(PyError::new(
                    "TypeError",
                    format!(
                        "'<' not supported between instances of '{}' and '{}'",
                        self.type_name(),
                        other.type_name()
                    ),
                )),
            },
        }
    }
}

/// `Dist.sample(rng)`.
pub fn sample(d: &Dist, rng: &mut PyRandom) -> R<Sample> {
    let a = &d.args;
    let x = match d.name {
        "fixed" => a[0],
        "unif" => rng.uniform(a[0], a[1]),
        "tri" => rng.triangular(a[0], a[2], Some(a[1])),
        "norm" => rng.gauss(a[0], a[1]),
        "lnorm" => {
            let (mean, sd) = (a[0], a[1]);
            let s2 = (1.0 + (sd * sd) / (mean * mean)).ln();
            let z = rng.normalvariate(mean.ln() - s2 / 2.0, s2.sqrt());
            let x = z.exp();
            if x.is_infinite() && z.is_finite() {
                return Err(PyError::new("OverflowError", "math range error"));
            }
            x
        }
        "exp" => rng.expovariate(1.0 / a[0]),
        "bern" => return Ok(Sample::Bool(rng.random() < a[0])),
        "cat" => {
            let u = rng.random();
            let mut acc = 0.0;
            for (label, p) in d.labels.iter().zip(a) {
                acc += p;
                if u < acc {
                    return Ok(label_sample(label));
                }
            }
            return Ok(d
                .labels
                .last()
                .map(label_sample)
                .unwrap_or(Sample::Num(0.0)));
        }
        other => return Err(dist_error(format!("unknown distribution {other}"))),
    };
    if d.kind == "duration" && x < 0.0 {
        return Ok(Sample::Num(0.0));
    }
    Ok(Sample::Num(x))
}

fn label_sample(l: &Label) -> Sample {
    match l {
        Label::Str(s) => Sample::Str(s.clone()),
        Label::Bool(b) => Sample::Bool(*b),
        Label::Num(x) => Sample::Num(*x),
    }
}

/// Seconds of an ISO 8601 duration (`P14D`, `PT2H30M`, `P1W`); `None` if the text is
/// not one (`sim.iso_duration`). Years count 365 days and months 30.
pub fn iso_duration(text: &str) -> Option<f64> {
    let t = tok_core::pyutil::strip(text);
    if t == "P" || t == "PT" {
        return None;
    }
    let mut rest = t.strip_prefix('P')?;
    let mut vals = [0.0f64; 7];
    // designators of the date part, then of the time part
    let date = [('Y', 0), ('M', 1), ('W', 2), ('D', 3)];
    let time = [('H', 4), ('M', 5), ('S', 6)];
    let mut next = 0;
    let number = |s: &str| -> Option<(f64, usize)> {
        let digits = |t: &str| {
            t.char_indices()
                .find(|(_, c)| tok_core::pyutil::decimal_value(*c).is_none())
                .map(|(i, _)| i)
                .unwrap_or(t.len())
        };
        let n = digits(s);
        if n == 0 {
            return None;
        }
        let mut end = n;
        if let Some(r) = s[n..].strip_prefix('.') {
            let m = digits(r);
            if m > 0 {
                end = n + 1 + m;
            }
        }
        let ascii: String = s[..end]
            .chars()
            .map(|c| match tok_core::pyutil::decimal_value(c) {
                Some(d) => char::from(b'0' + d as u8),
                None => c,
            })
            .collect();
        Some((ascii.parse::<f64>().ok()?, end))
    };
    while !rest.is_empty() && !rest.starts_with('T') {
        let (v, n) = number(rest)?;
        let unit = rest[n..].chars().next()?;
        let k = date[next..].iter().position(|(u, _)| *u == unit)?;
        vals[date[next + k].1] = v;
        next += k + 1;
        rest = &rest[n + unit.len_utf8()..];
    }
    if let Some(r) = rest.strip_prefix('T') {
        rest = r;
        let mut next = 0;
        while !rest.is_empty() {
            let (v, n) = number(rest)?;
            let unit = rest[n..].chars().next()?;
            let k = time[next..].iter().position(|(u, _)| *u == unit)?;
            vals[time[next + k].1] = v;
            next += k + 1;
            rest = &rest[n + unit.len_utf8()..];
        }
    }
    let [y, mo, w, d, h, mi, s] = vals;
    Some((((y * 365.0 + mo * 30.0 + w * 7.0 + d) * 24.0 + h) * 60.0 + mi) * 60.0 + s)
}

/// Waiting time of a timer (`sim.timer_seconds`): a duration, a cycle `R…/period` (its
/// first period) or a date-time; `None` if the value is none of them.
pub fn timer_seconds(value: &str, now: &DateTime) -> R<Option<f64>> {
    let v = tok_core::pyutil::strip(value);
    if v.starts_with('R') && v.contains('/') {
        return Ok(iso_duration(v.rsplit('/').next().unwrap_or("")));
    }
    if let Some(d) = iso_duration(v) {
        return Ok(Some(d));
    }
    let Ok(target) = DateTime::fromisoformat(v) else {
        return Ok(None);
    };
    let secs = target.sub_datetime(now)?.total_seconds();
    Ok(Some(if secs > 0.0 { secs } else { 0.0 }))
}

/// `t0 + timedelta(seconds=s)`.
pub fn shift(t0: &DateTime, s: f64) -> R<DateTime> {
    t0.add(&Duration::from_seconds(s)?)
}

// ------------------------------------------------------------------------ state

/// A resource: `count` interchangeable performers and their queue.
#[derive(Debug, Clone)]
pub struct Resource {
    pub id: String,
    pub label: Option<String>,
    /// `int(float(count))`, held as an integral float.
    pub count: f64,
    pub busy: i64,
    pub queue: Vec<usize>,
    pub busy_time: f64,
}

/// Tokens of one process level of a case: the case itself or one embedded sub-process
/// instance (`_Frame`).
#[derive(Debug, Clone)]
struct Frame {
    case: usize,
    scope: String,
    parent: Option<usize>,
    ai: Option<usize>,
    active: i64,
    alive: bool,
    joins: IndexMap<String, i64>,
    or_stack: Vec<i64>,
    ais: Vec<usize>,
    waiters: Vec<usize>,
    children: Vec<usize>,
    last_end: Option<String>,
    /// ad-hoc sub-process: (expression, wait flag)
    until: Option<(String, bool)>,
    /// complex gateways with an activation condition waiting in this frame: (tokens
    /// expected, already fired)
    complex: IndexMap<String, (i64, bool)>,
}

/// A case (`_Case`).
#[derive(Debug, Clone)]
pub struct Case {
    pub id: u64,
    pub start: f64,
    pub end: Option<f64>,
    pub end_event: Option<String>,
    pub vars: Vars,
    steps: u64,
    top: usize,
    /// (activity id, handler id, frame of the activity) of completed activities, in
    /// completion order
    compensable: Vec<(String, String, usize)>,
}

/// An activity instance, task or sub-process (`_AI`).
#[derive(Debug, Clone)]
struct Ai {
    frame: usize,
    node: String,
    enqueue: f64,
    start: Option<f64>,
    end: Option<f64>,
    resource: Option<String>,
    alive: bool,
    released: bool,
    timers: Vec<usize>,
    child: Option<usize>,
    /// multi-instance: variables of one instance
    local: Option<Vars>,
    /// multi-instance: the body this instance belongs to
    body: Option<usize>,
    /// multi-instance body: its instances
    instances: Option<Vec<usize>>,
    /// not written to the event log (compensation throw)
    silent: bool,
    /// standard loop with a condition or a maximum: number of this iteration, from 1
    /// (0 for every other activity)
    iteration: u64,
    /// compensation of a cancelled transaction: the instance of the transaction
    cancel: Option<usize>,
    index: usize,
    items: Vec<PyValue>,
    results: Vec<PyValue>,
    completed: usize,
    next: usize,
}

/// One row of the event log: case, activity, label, resource, enqueue, start, end, status.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub case: u64,
    pub activity: String,
    pub label: String,
    pub resource: String,
    pub enqueue: f64,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub status: &'static str,
}

/// A scheduled call of `sim.py` (`fn` and `args` of a heap entry).
#[derive(Debug, Clone)]
enum Event {
    Arrive(u64),
    Wake {
        frame: usize,
        waiter: usize,
        node: String,
    },
    Done(usize),
    BoundaryFire {
        ai: usize,
        boundary: String,
        waiter: usize,
    },
    RaceReceive {
        frame: usize,
        waiter: usize,
        ai: usize,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    t: f64,
    seq: u64,
    event: Event,
}

/// Python's `<` of the tuples `(t, seq, …)`: `seq` decides only when the times are equal.
fn entry_lt(a: &Entry, b: &Entry) -> bool {
    if a.t == b.t { a.seq < b.seq } else { a.t < b.t }
}

/// `heapq.heappush`.
fn heap_push(heap: &mut Vec<Entry>, item: Entry) {
    heap.push(item);
    let last = heap.len() - 1;
    sift_down(heap, 0, last);
}

/// `heapq.heappop`.
fn heap_pop(heap: &mut Vec<Entry>) -> Option<Entry> {
    let last = heap.pop()?;
    if heap.is_empty() {
        return Some(last);
    }
    let top = std::mem::replace(&mut heap[0], last);
    sift_up(heap, 0);
    Some(top)
}

/// `heapq._siftdown`.
fn sift_down(heap: &mut [Entry], start: usize, mut pos: usize) {
    while pos > start {
        let parent = (pos - 1) >> 1;
        if entry_lt(&heap[pos], &heap[parent]) {
            heap.swap(pos, parent);
            pos = parent;
            continue;
        }
        break;
    }
}

/// `heapq._siftup`.
fn sift_up(heap: &mut [Entry], mut pos: usize) {
    let end = heap.len();
    let start = pos;
    let mut child = 2 * pos + 1;
    while child < end {
        let right = child + 1;
        if right < end && !entry_lt(&heap[child], &heap[right]) {
            child = right;
        }
        heap.swap(pos, child);
        pos = child;
        child = 2 * pos + 1;
    }
    sift_down(heap, start, pos);
}

/// The outcome of a run, the data of `sim.Result`.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub t0: DateTime,
    /// `int(float(seed))` as text
    pub seed: String,
    pub horizon: f64,
    pub rows: Vec<Row>,
    pub cases: Vec<Case>,
    pub resources: Vec<Resource>,
    pub warnings: Vec<String>,
}

/// The simulator of one document (`sim.Simulator`).
pub struct Simulator<'a> {
    doc: &'a Document,
    n_cases: f64,
    seed: String,
    rng: PyRandom,
    t0: DateTime,
    arrival: Dist,
    variables: Vec<(String, Dist)>,
    resources: IndexMap<String, Resource>,
    scope: String,
    out: HashMap<String, Vec<usize>>,
    inc: HashMap<String, usize>,
    boundaries: HashMap<String, Vec<String>>,
    dist_cache: HashMap<String, Dist>,
    heap: Vec<Entry>,
    seq: u64,
    t: f64,
    cases: Vec<Case>,
    rows: Vec<Row>,
    warnings: Vec<String>,
    frames: Vec<Frame>,
    ais: Vec<Ai>,
    waiters: Vec<bool>,
    main_start: String,
    /// the document is in language 0.5 (multiple events and standard loops with a
    /// condition exist only there)
    lang05: bool,
    /// iteration of a standard loop for the next activity instance
    loop_iter: u64,
}

/// The start event of a scope (the process or a sub-process): the first plain start
/// event, else the first start event, else the instantiating receive task or event-based
/// gateway of a scope without a start event (language 0.5).
pub fn start_of(doc: &Document, scope: &str) -> Option<String> {
    let starts: Vec<&Node> = doc
        .nodes
        .values()
        .filter(|n| n.scope.as_deref() == Some(scope) && n.base.kind == "start")
        .collect();
    let plain: Vec<&&Node> = starts
        .iter()
        .filter(|n| !TRIGGERS.iter().any(|k| n.base.attrs.contains_key(*k)))
        .collect();
    match plain.first() {
        Some(n) => Some(node_id(n).to_string()),
        None => starts
            .first()
            .map(|n| node_id(n).to_string())
            .or_else(|| instantiating(doc, scope)),
    }
}

/// The instantiating receive task or event-based gateway of a scope without a start
/// event (language 0.5): it starts each case as a start event would.
fn instantiating(doc: &Document, scope: &str) -> Option<String> {
    doc.nodes
        .values()
        .find(|n| {
            n.scope.as_deref() == Some(scope)
                && (n.base.kind == "receive" || n.base.kind == "eventgw")
                && n.base.has("instantiate")
        })
        .map(|n| node_id(n).to_string())
}

/// The process the simulator runs: that of the first pool that has one, or the
/// top-level process.
pub fn main_scope(doc: &Document) -> String {
    doc.pools
        .iter()
        .find(|p| !p.blackbox)
        .map(|p| p.process_id())
        .unwrap_or_else(|| doc.default_process_id())
}

fn node_id(n: &Node) -> &str {
    n.base.id()
}

impl<'a> Simulator<'a> {
    /// `Simulator(doc, cases, seed)`: `None` takes the value of `simulate` or the default.
    pub fn new(doc: &'a Document, cases: Option<Param>, seed: Option<Param>) -> R<Self> {
        let attrs = doc.simulate.as_ref().map(|s| &s.attrs);
        let attr = |k: &str| attrs.and_then(|a| a.get(k)).cloned();
        let cases = cases.unwrap_or_else(|| match attr("cases") {
            Some(t) => Param::Text(t),
            None => Param::Int(DEFAULT_CASES.to_string()),
        });
        let n_cases = int_of_float(&cases)?;
        let seed = seed.unwrap_or_else(|| match attr("seed") {
            Some(t) => Param::Text(t),
            None => Param::Int("1".into()),
        });
        let seed = int_of_float(&seed)?;
        let rng = PyRandom::from_float_seed(seed)
            .map_err(|e| PyError::new("OverflowError", e.to_string()))?;
        let t0 = DateTime::fromisoformat(&attr("start").unwrap_or_else(|| DEFAULT_START.into()))?;
        let arrival = parse_dist(&attr("arrival").unwrap_or_else(|| DEFAULT_ARRIVAL.into()))
            .map_err(dist_error)?;
        let mut variables = Vec::new();
        for v in &doc.variables {
            let d = parse_dist(v.label.as_deref().unwrap_or("")).map_err(dist_error)?;
            variables.push((v.id().to_string(), d));
        }
        let mut resources = IndexMap::new();
        for r in doc.resources.values() {
            let count = match r.attrs.get("count") {
                Some(t) => int_of_float(&Param::Text(t.clone()))?,
                None => 1.0,
            };
            resources.insert(
                r.id().to_string(),
                Resource {
                    id: r.id().to_string(),
                    label: r.label.clone(),
                    count,
                    busy: 0,
                    queue: Vec::new(),
                    busy_time: 0.0,
                },
            );
        }
        let scope = main_scope(doc);
        let mut out: HashMap<String, Vec<usize>> = HashMap::new();
        let mut inc: HashMap<String, usize> = HashMap::new();
        for (i, f) in doc.flows.iter().enumerate() {
            if !doc.nodes.contains_key(&f.source) || !doc.nodes.contains_key(&f.target) {
                continue;
            }
            out.entry(f.source.clone()).or_default().push(i);
            *inc.entry(f.target.clone()).or_default() += 1;
        }
        let mut boundaries: HashMap<String, Vec<String>> = HashMap::new();
        for n in doc.nodes.values() {
            if n.base.kind == "boundary"
                && let Some(on) = n.base.attrs.get("on")
            {
                boundaries
                    .entry(on.clone())
                    .or_default()
                    .push(node_id(n).to_string());
            }
        }
        Ok(Simulator {
            doc,
            n_cases,
            seed: int_text(seed),
            rng,
            t0,
            arrival,
            variables,
            resources,
            scope,
            out,
            inc,
            boundaries,
            dist_cache: HashMap::new(),
            heap: Vec::new(),
            seq: 0,
            t: 0.0,
            cases: Vec::new(),
            rows: Vec::new(),
            warnings: Vec::new(),
            frames: Vec::new(),
            ais: Vec::new(),
            waiters: Vec::new(),
            main_start: String::new(),
            lang05: doc.lang_05(),
            loop_iter: 0,
        })
    }

    fn node(&self, id: &str) -> &'a Node {
        // identifiers come from the document's own flows and nodes
        &self.doc.nodes[id]
    }

    fn dist(&mut self, text: &str) -> R<Dist> {
        if let Some(d) = self.dist_cache.get(text) {
            return Ok(d.clone());
        }
        let d = parse_dist(text).map_err(dist_error)?;
        self.dist_cache.insert(text.to_string(), d.clone());
        Ok(d)
    }

    fn draw(&mut self, text: &str) -> R<Sample> {
        let d = self.dist(text)?;
        sample(&d, &mut self.rng)
    }

    fn start_of(&self, scope: &str) -> Option<String> {
        start_of(self.doc, scope)
    }

    fn schedule(&mut self, dt: &Sample, event: Event) -> R<()> {
        let t = self.t + dt.delay()?;
        self.seq += 1;
        heap_push(
            &mut self.heap,
            Entry {
                t,
                seq: self.seq,
                event,
            },
        );
        Ok(())
    }

    fn now_dt(&self) -> R<DateTime> {
        shift(&self.t0, self.t)
    }

    fn new_waiter(&mut self) -> usize {
        self.waiters.push(true);
        self.waiters.len() - 1
    }

    fn new_ai(&mut self, frame: usize, node: &str) -> usize {
        self.ais.push(Ai {
            frame,
            node: node.to_string(),
            enqueue: self.t,
            start: None,
            end: None,
            resource: None,
            alive: true,
            released: false,
            timers: Vec::new(),
            child: None,
            local: None,
            body: None,
            instances: None,
            silent: false,
            iteration: std::mem::take(&mut self.loop_iter),
            cancel: None,
            index: 0,
            items: Vec::new(),
            results: Vec::new(),
            completed: 0,
            next: 0,
        });
        self.ais.len() - 1
    }

    fn new_frame(
        &mut self,
        case: usize,
        scope: &str,
        parent: Option<usize>,
        ai: Option<usize>,
    ) -> usize {
        self.frames.push(Frame {
            case,
            scope: scope.to_string(),
            parent,
            ai,
            active: 0,
            alive: true,
            joins: IndexMap::new(),
            or_stack: Vec::new(),
            ais: Vec::new(),
            waiters: Vec::new(),
            children: Vec::new(),
            last_end: None,
            until: None,
            complex: IndexMap::new(),
        });
        self.frames.len() - 1
    }

    // ------------------------------------------------------------------ driver

    /// `run()`: simulate every case to the end.
    pub fn run(mut self) -> R<Outcome> {
        let start = self
            .start_of(&self.scope.clone())
            .ok_or_else(|| sim_error(format!("the process '{}' has no start event", self.scope)))?;
        self.main_start = start;
        if self.n_cases > 0.0 {
            self.schedule(&Sample::Num(0.0), Event::Arrive(0))?;
        }
        while let Some(e) = heap_pop(&mut self.heap) {
            self.t = e.t;
            match e.event {
                Event::Arrive(k) => self.arrive(k)?,
                Event::Wake {
                    frame,
                    waiter,
                    node,
                } => self.wake(frame, waiter, &node)?,
                Event::Done(ai) => self.done(ai)?,
                Event::BoundaryFire {
                    ai,
                    boundary,
                    waiter,
                } => self.boundary_fire(ai, &boundary, waiter)?,
                Event::RaceReceive { frame, waiter, ai } => self.race_receive(frame, waiter, ai)?,
            }
        }
        Ok(Outcome {
            t0: self.t0,
            seed: self.seed,
            horizon: self.t,
            rows: self.rows,
            cases: self.cases,
            resources: self.resources.into_values().collect(),
            warnings: self.warnings,
        })
    }

    fn arrive(&mut self, k: u64) -> R<()> {
        let mut vars = Vars::new();
        for i in 0..self.variables.len() {
            let s = sample(&self.variables[i].1, &mut self.rng)?;
            vars.insert(self.variables[i].0.clone(), s.to_py());
        }
        let case = self.cases.len();
        self.cases.push(Case {
            id: k + 1,
            start: self.t,
            end: None,
            end_event: None,
            vars,
            steps: 0,
            top: 0,
            compensable: Vec::new(),
        });
        let scope = self.scope.clone();
        let frame = self.new_frame(case, &scope, None, None);
        self.cases[case].top = frame;
        self.frames[frame].active = 1;
        let start = self.main_start.clone();
        self.enter(frame, &start)?;
        if ((k + 1) as f64) < self.n_cases {
            let d = self.arrival.clone();
            let s = sample(&d, &mut self.rng)?;
            self.schedule(&s, Event::Arrive(k + 1))?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ tokens

    fn consume(&mut self, frame: usize) -> R<()> {
        self.frames[frame].active -= 1;
        let f = &self.frames[frame];
        if let Some((expr, wait)) = f.until.clone()
            && f.alive
            && f.active > 0
            && !wait
        {
            let vars = self.cases[f.case].vars.clone();
            let what = format!("until of '{}'", f.scope);
            if self.truth(&expr, &vars, &what) {
                self.kill(frame)?;
                self.frames[frame].alive = true;
                self.frames[frame].active = 0;
            }
        }
        if self.frames[frame].active <= 0 && self.frames[frame].alive {
            self.frame_done(frame)?;
        }
        Ok(())
    }

    fn frame_done(&mut self, frame: usize) -> R<()> {
        self.frames[frame].alive = false;
        let f = &self.frames[frame];
        if f.parent.is_none() {
            let case = f.case;
            if self.cases[case].end.is_none() {
                let pending = f.joins.values().any(|v| *v != 0);
                let end_event = match &f.last_end {
                    Some(e) if !e.is_empty() => Some(e.clone()),
                    _ if pending => Some("deadlock".to_string()),
                    _ => None,
                };
                let c = &mut self.cases[case];
                c.end = Some(self.t);
                c.end_event = end_event;
            }
            Ok(())
        } else {
            match f.ai {
                Some(ai) => self.finish_ai(ai),
                None => Ok(()),
            }
        }
    }

    fn kill(&mut self, frame: usize) -> R<()> {
        self.frames[frame].alive = false;
        for w in self.frames[frame].waiters.clone() {
            self.waiters[w] = false;
        }
        for ai in self.frames[frame].ais.clone() {
            if self.ais[ai].alive {
                self.ais[ai].alive = false;
                self.release(ai)?;
                for w in self.ais[ai].timers.clone() {
                    self.waiters[w] = false;
                }
            }
        }
        for ch in self.frames[frame].children.clone() {
            if self.frames[ch].alive {
                self.kill(ch)?;
            }
        }
        Ok(())
    }

    fn enter(&mut self, frame: usize, node_id: &str) -> R<()> {
        let iteration = std::mem::take(&mut self.loop_iter);
        if !self.frames[frame].alive {
            return Ok(());
        }
        let case = self.frames[frame].case;
        self.cases[case].steps += 1;
        if self.cases[case].steps > MAX_STEPS {
            let top = self.cases[case].top;
            self.kill(top)?;
            let c = &mut self.cases[case];
            if c.end.is_none() {
                c.end = Some(self.t);
                c.end_event = Some("loop-limit".into());
            }
            return Ok(());
        }
        let n = self.node(node_id);
        let k = n.base.kind.as_str();
        if (k == "throw" || k == "end")
            && n.base.has("compensate")
            && self.start_compensation(frame, n)?
        {
            return Ok(());
        }
        if k == "end"
            && n.base.has("cancel")
            && let Some(tx) = self.transaction_frame(frame)
        {
            return self.cancel_transaction(tx);
        }
        if self.standard_loop(n) {
            // the first entry tests the condition only with test-before; later
            // iterations come from `complete`, which has tested it
            let iteration = if iteration > 0 {
                iteration
            } else if n.base.has("test-before") && !self.loop_again(frame, n, 0) {
                return self.leave(frame, node_id);
            } else {
                1
            };
            // taken by the activity instance that the dispatch below creates first
            self.loop_iter = iteration;
        }
        match k {
            "end" => {
                self.frames[frame].last_end = Some(node_id.to_string());
                if n.base.has("terminate") {
                    self.kill(frame)?;
                    self.frames[frame].alive = true;
                    self.frames[frame].active = 0;
                    self.frame_done(frame)
                } else {
                    self.consume(frame)
                }
            }
            "start" | "throw" | "boundary" => self.leave(frame, node_id),
            "catch" => {
                let wait = if self.multiple(n) {
                    self.multiple_wait(n, true)?
                } else {
                    self.event_wait(n)?
                };
                let pass = match &wait {
                    None => true,
                    Some(w) => w.le_zero()?,
                };
                if pass {
                    self.leave(frame, node_id)
                } else {
                    let w = self.new_waiter();
                    self.frames[frame].waiters.push(w);
                    let wait = wait.unwrap_or(Sample::Num(0.0));
                    self.schedule(
                        &wait,
                        Event::Wake {
                            frame,
                            waiter: w,
                            node: node_id.to_string(),
                        },
                    )
                }
            }
            _ if GATEWAY_KINDS.contains(&k) => self.gateway(frame, n),
            "sub" | "transaction" => self.start_sub(frame, n),
            "adhoc" => self.start_adhoc(frame, n),
            "eventsub" => self.consume(frame),
            _ if (n.base.has("parallel") || n.base.has("sequential"))
                && n.base.attrs.contains_key("collection") =>
            {
                self.start_multi(frame, n)
            }
            _ => self.start_activity(frame, n),
        }
    }

    fn wake(&mut self, frame: usize, waiter: usize, node_id: &str) -> R<()> {
        if self.waiters[waiter] && self.frames[frame].alive {
            self.waiters[waiter] = false;
            self.leave(frame, node_id)?;
        }
        Ok(())
    }

    fn event_wait(&mut self, n: &Node) -> R<Option<Sample>> {
        let a = &n.base.attrs;
        if let Some(timer) = a.get("timer") {
            let now = self.now_dt()?;
            let w = timer_seconds(timer, &now)?;
            if w.is_none() {
                self.warnings.push(format!(
                    "timer of '{}' is not understood: {timer}",
                    node_id(n)
                ));
            }
            return Ok(w.map(Sample::Num));
        }
        if let Some(d) = a.get("delay") {
            return Ok(Some(self.draw(d)?));
        }
        if let Some(d) = a.get("duration") {
            return Ok(Some(self.draw(d)?));
        }
        Ok(Some(Sample::Num(0.0)))
    }

    /// Move the token that is at `node_id` along its outgoing flows.
    fn leave(&mut self, frame: usize, node_id: &str) -> R<()> {
        let flows = self.out.get(node_id).cloned().unwrap_or_default();
        if flows.is_empty() {
            return self.consume(frame);
        }
        if flows.len() == 1 {
            let target = self.doc.flows[flows[0]].target.clone();
            return self.enter(frame, &target);
        }
        let ctx = self.cases[self.frames[frame].case].vars.clone();
        let doc = self.doc;
        let mut taken = Vec::new();
        for &i in &flows {
            let f = &doc.flows[i];
            if !f.base.has("default") && (!f.base.attrs.contains_key("when") || self.cond(i, &ctx))
            {
                taken.push(i);
            }
        }
        if taken.is_empty() {
            taken = flows
                .iter()
                .copied()
                .filter(|i| self.doc.flows[*i].base.has("default"))
                .collect();
            if taken.is_empty() {
                taken = vec![flows[0]];
            }
        }
        let targets: Vec<String> = taken
            .iter()
            .map(|i| self.doc.flows[*i].target.clone())
            .collect();
        self.route(frame, &targets)
    }

    fn route(&mut self, frame: usize, targets: &[String]) -> R<()> {
        if targets.is_empty() {
            return self.consume(frame);
        }
        self.frames[frame].active += targets.len() as i64 - 1;
        for tg in targets {
            self.enter(frame, tg)?;
        }
        Ok(())
    }

    fn cond(&mut self, flow: usize, ctx: &Vars) -> bool {
        let doc = self.doc;
        let f = &doc.flows[flow];
        let when = &f.base.attrs["when"];
        match eval_expr(when, ctx) {
            Ok(v) => pyval::truthy(&v),
            Err(ex) => {
                self.warnings.push(format!(
                    "condition of flow '{}' could not be evaluated: {}",
                    f.base.id.as_deref().unwrap_or("None"),
                    ex.message
                ));
                false
            }
        }
    }

    // ---------------------------------------------------------------- gateways

    fn gateway(&mut self, frame: usize, n: &Node) -> R<()> {
        let id = node_id(n);
        let kind = n.base.kind.as_str();
        let nin = self.inc.get(id).copied().unwrap_or(0) as i64;
        let ctx = self.cases[self.frames[frame].case].vars.clone();
        let activation = n.base.attr("activation").filter(|_| kind == "complex");
        if nin > 1
            && let Some(expr) = activation
        {
            if !self.complex_join(frame, id, expr, nin)? {
                return Ok(());
            }
        } else if nin > 1 && (kind == "and" || kind == "or" || kind == "complex") {
            // a complex gateway without an activation condition joins like `or`
            let f = &mut self.frames[frame];
            let expected = if kind == "and" {
                nin
            } else {
                f.or_stack.last().copied().unwrap_or(nin)
            };
            let got = f.joins.get(id).copied().unwrap_or(0) + 1;
            if got < expected {
                f.joins.insert(id.to_string(), got);
                return self.consume(frame);
            }
            f.joins.insert(id.to_string(), 0);
            if kind != "and" {
                f.or_stack.pop();
            }
        }
        let flows = self.out.get(id).cloned().unwrap_or_default();
        if flows.len() <= 1 {
            return self.leave(frame, id);
        }
        let doc = self.doc;
        match kind {
            "and" => {
                let targets: Vec<String> =
                    flows.iter().map(|i| doc.flows[*i].target.clone()).collect();
                self.route(frame, &targets)
            }
            "xor" => {
                let t = self.xor_choice(&flows, &ctx);
                self.route(frame, &[t])
            }
            "or" | "complex" => {
                let targets = self.or_choice(&flows, &ctx);
                self.frames[frame].or_stack.push(targets.len() as i64);
                self.route(frame, &targets)
            }
            _ => self.race(frame, n, &flows),
        }
    }

    fn prob(&self, flow: usize) -> f64 {
        // the checker reports invalid probabilities as errors
        parse_float(&self.doc.flows[flow].base.attrs["p"]).unwrap_or(f64::NAN)
    }

    fn xor_choice(&mut self, flows: &[usize], ctx: &Vars) -> String {
        let doc = self.doc;
        for &i in flows {
            let f = &doc.flows[i];
            if f.base.attrs.contains_key("when") && !f.base.has("default") && self.cond(i, ctx) {
                return f.target.clone();
            }
        }
        let ps: Vec<usize> = flows
            .iter()
            .copied()
            .filter(|i| self.doc.flows[*i].base.attrs.contains_key("p"))
            .collect();
        if !ps.is_empty() {
            let u = self.rng.random();
            let mut acc = 0.0;
            for &i in &ps {
                acc += self.prob(i);
                if u < acc {
                    return self.doc.flows[i].target.clone();
                }
            }
        }
        for &i in flows {
            if self.doc.flows[i].base.has("default") {
                return self.doc.flows[i].target.clone();
            }
        }
        if let Some(&last) = ps.last() {
            return self.doc.flows[last].target.clone();
        }
        self.doc.flows[flows[0]].target.clone()
    }

    fn or_choice(&mut self, flows: &[usize], ctx: &Vars) -> Vec<String> {
        let doc = self.doc;
        let mut chosen = Vec::new();
        for &i in flows {
            let f = &doc.flows[i];
            if f.base.has("default") {
                continue;
            }
            if f.base.attrs.contains_key("when") {
                if self.cond(i, ctx) {
                    chosen.push(self.doc.flows[i].target.clone());
                }
            } else if f.base.attrs.contains_key("p") && self.rng.random() < self.prob(i) {
                chosen.push(self.doc.flows[i].target.clone());
            }
        }
        if chosen.is_empty() {
            chosen = flows
                .iter()
                .filter(|i| self.doc.flows[**i].base.has("default"))
                .map(|i| self.doc.flows[*i].target.clone())
                .collect();
            if chosen.is_empty() {
                chosen = vec![self.doc.flows[flows[0]].target.clone()];
            }
        }
        chosen
    }

    fn race(&mut self, frame: usize, n: &Node, flows: &[usize]) -> R<()> {
        let mut best: Option<&'a Node> = None;
        let mut best_t: Option<Sample> = None;
        for &i in flows {
            let target = self.node(&self.doc.flows[i].target);
            let a = &target.base.attrs;
            let w = if target.base.kind == "receive" {
                if let Some(d) = a.get("delay") {
                    Some(self.draw(d)?)
                } else if let Some(d) = a.get("duration") {
                    Some(self.draw(d)?)
                } else {
                    None
                }
            } else if self.multiple(target) {
                self.multiple_wait(target, false)?
            } else if a.contains_key("timer") || a.contains_key("delay") {
                self.event_wait(target)?
            } else {
                None
            };
            if let Some(w) = w {
                let better = match &best_t {
                    None => true,
                    Some(b) => w.lt(b)?,
                };
                if better {
                    best = Some(target);
                    best_t = Some(w);
                }
            }
        }
        if best.is_none() && n.base.has("instantiate") {
            // an instantiating gateway is reached because one of its events arrived:
            // without known waiting times that is the event of the first flow
            best = flows.first().map(|i| self.node(&self.doc.flows[*i].target));
            best_t = Some(Sample::Num(0.0));
        }
        let (Some(best), Some(best_t)) = (best, best_t) else {
            self.warnings.push(format!(
                "event-based gateway '{}': no event can happen (add delay= to message events)",
                node_id(n)
            ));
            return Ok(());
        };
        let w = self.new_waiter();
        self.frames[frame].waiters.push(w);
        if best.base.kind == "receive" {
            let ai = self.new_ai(frame, node_id(best));
            self.frames[frame].ais.push(ai);
            self.schedule(
                &best_t,
                Event::RaceReceive {
                    frame,
                    waiter: w,
                    ai,
                },
            )
        } else {
            self.schedule(
                &best_t,
                Event::Wake {
                    frame,
                    waiter: w,
                    node: node_id(best).to_string(),
                },
            )
        }
    }

    fn race_receive(&mut self, frame: usize, waiter: usize, ai: usize) -> R<()> {
        if self.waiters[waiter] && self.frames[frame].alive {
            self.ais[ai].start = Some(self.t);
            self.complete(ai)?;
        }
        Ok(())
    }

    // -------------------------------------------------------------- activities

    fn start_activity(&mut self, frame: usize, n: &Node) -> R<()> {
        let ai = self.new_ai(frame, node_id(n));
        self.frames[frame].ais.push(ai);
        self.arm_boundaries(ai)?;
        self.claim(ai, n)
    }

    /// Wait for a performer of `by=` or begin at once.
    fn claim(&mut self, ai: usize, n: &Node) -> R<()> {
        if let Some(rid) = n.base.attrs.get("by").filter(|r| !r.is_empty())
            && let Some(res) = self.resources.get_mut(rid)
        {
            self.ais[ai].resource = Some(rid.clone());
            if (res.busy as f64) < res.count {
                return self.begin(ai);
            }
            res.queue.push(ai);
            return Ok(());
        }
        self.begin(ai)
    }

    fn begin(&mut self, ai: usize) -> R<()> {
        self.ais[ai].start = Some(self.t);
        if let Some(rid) = &self.ais[ai].resource
            && let Some(res) = self.resources.get_mut(rid)
        {
            res.busy += 1;
        }
        let n = self.node(&self.ais[ai].node);
        let dur = match n.base.attrs.get("duration") {
            Some(d) => self.draw(d)?,
            None => Sample::Num(0.0),
        };
        self.schedule(&dur, Event::Done(ai))
    }

    fn done(&mut self, ai: usize) -> R<()> {
        if self.ais[ai].alive && self.frames[self.ais[ai].frame].alive {
            self.complete(ai)?;
        }
        Ok(())
    }

    fn complete(&mut self, ai: usize) -> R<()> {
        let n = self.node(&self.ais[ai].node);
        let frame = self.ais[ai].frame;
        let case = self.frames[frame].case;
        self.ais[ai].alive = false;
        self.ais[ai].end = Some(self.t);
        self.release(ai)?;
        for w in self.ais[ai].timers.clone() {
            self.waiters[w] = false;
        }
        let a = &n.base.attrs;
        // an instance of a multi-instance activity writes into its own variables and
        // evaluates with a copy of the case variables and its own, taken before `assign`
        let local = self.ais[ai].local.is_some();
        let snapshot: Option<Vars> = self.ais[ai].local.as_ref().map(|l| {
            let mut ctx = self.cases[case].vars.clone();
            for (k, v) in l {
                ctx.insert(k.clone(), v.clone());
            }
            ctx
        });
        let ctx = |sim: &Self| -> Vars {
            snapshot
                .clone()
                .unwrap_or_else(|| sim.cases[case].vars.clone())
        };
        if let Some(assign) = a.get("assign") {
            for (name, d) in parse_assign(assign).map_err(dist_error)? {
                let v = sample(&d, &mut self.rng)?.to_py();
                self.store(ai, case, local, name, v);
            }
        }
        if n.base.kind == "rule"
            && let Some(decision) = a.get("decision")
        {
            let dec = self.doc.decisions.get(decision);
            let res = match a.get("result").filter(|r| !r.is_empty()) {
                Some(r) => r.clone(),
                None => match dec {
                    Some(d) if d.outputs.len() == 1 => d.outputs[0].0.clone(),
                    // language 0.7: other decision logic gives the value of the decision
                    Some(d) if self.doc.lang_07() => tok_core::drg::var_name(&d.base),
                    _ => "result".to_string(),
                },
            };
            let c = ctx(self);
            match eval_decision(self.doc, decision, &c) {
                Ok(v) => self.store(ai, case, local, res, v),
                Err(ex) => self.warnings.push(format!(
                    "decision '{decision}' could not be evaluated: {}",
                    ex.message
                )),
            }
        }
        if n.base.kind == "script"
            && let Some(expr) = a.get("expr")
        {
            let c = ctx(self);
            match eval_expr(expr, &c) {
                Ok(v) => {
                    let res = a.get("result").cloned().unwrap_or_else(|| "result".into());
                    self.store(ai, case, local, res, v)
                }
                Err(ex) => self.warnings.push(format!(
                    "expression of '{}' could not be evaluated: {}",
                    node_id(n),
                    ex.message
                )),
            }
        }
        if !self.ais[ai].silent {
            self.log(ai, "completed");
        }
        if self.ais[ai].body.is_some() {
            return self.instance_done(ai);
        }
        if let Some(h) = a.get("compensate")
            && self.ais[ai].instances.is_none()
        {
            self.cases[case]
                .compensable
                .push((node_id(n).to_string(), h.clone(), frame));
        }
        let iteration = self.ais[ai].iteration;
        if iteration > 0 && self.loop_again(frame, n, iteration) {
            self.loop_iter = iteration + 1;
            return self.enter(frame, node_id(n));
        }
        self.leave(frame, node_id(n))
    }

    fn store(&mut self, ai: usize, case: usize, local: bool, name: String, v: PyValue) {
        if local && let Some(l) = self.ais[ai].local.as_mut() {
            l.insert(name, v);
            return;
        }
        self.cases[case].vars.insert(name, v);
    }

    fn release(&mut self, ai: usize) -> R<()> {
        let Some(rid) = self.ais[ai].resource.clone() else {
            return Ok(());
        };
        if self.ais[ai].start.is_none() || self.ais[ai].released {
            if self.ais[ai].start.is_none()
                && let Some(res) = self.resources.get_mut(&rid)
                && let Some(pos) = res.queue.iter().position(|x| *x == ai)
            {
                res.queue.remove(pos);
            }
            return Ok(());
        }
        self.ais[ai].released = true;
        let start = self.ais[ai].start.unwrap_or(self.t);
        let t = self.t;
        let Some(res) = self.resources.get_mut(&rid) else {
            return Ok(());
        };
        res.busy -= 1;
        res.busy_time += t - start;
        while let Some(res) = self.resources.get_mut(&rid)
            && !res.queue.is_empty()
            && (res.busy as f64) < res.count
        {
            let nxt = res.queue.remove(0);
            if self.ais[nxt].alive && self.frames[self.ais[nxt].frame].alive {
                self.begin(nxt)?;
            }
        }
        Ok(())
    }

    fn log(&mut self, ai: usize, status: &'static str) {
        let a = &self.ais[ai];
        let n = self.node(&a.node);
        self.rows.push(Row {
            case: self.cases[self.frames[a.frame].case].id,
            activity: a.node.clone(),
            label: n.base.label.clone().unwrap_or_default(),
            resource: a.resource.clone().unwrap_or_default(),
            enqueue: a.enqueue,
            start: a.start,
            end: a.end,
            status,
        });
    }

    fn arm_boundaries(&mut self, ai: usize) -> R<()> {
        let node = self.ais[ai].node.clone();
        let Some(bs) = self.boundaries.get(&node).cloned() else {
            return Ok(());
        };
        for b in bs {
            let bn = self.node(&b);
            if bn.base.has("cancel") {
                // followed only when the transaction is cancelled
                continue;
            }
            let a = &bn.base.attrs;
            let w = if self.multiple(bn) {
                self.multiple_wait(bn, false)?
            } else if let Some(timer) = a.get("timer") {
                let now = self.now_dt()?;
                timer_seconds(timer, &now)?.map(Sample::Num)
            } else if let Some(d) = a.get("delay") {
                Some(self.draw(d)?)
            } else {
                None
            };
            let Some(w) = w else {
                continue;
            };
            let wt = self.new_waiter();
            self.ais[ai].timers.push(wt);
            self.schedule(
                &w,
                Event::BoundaryFire {
                    ai,
                    boundary: b,
                    waiter: wt,
                },
            )?;
        }
        Ok(())
    }

    fn boundary_fire(&mut self, ai: usize, b: &str, waiter: usize) -> R<()> {
        let frame = self.ais[ai].frame;
        if !self.waiters[waiter] || !self.ais[ai].alive || !self.frames[frame].alive {
            return Ok(());
        }
        self.waiters[waiter] = false;
        if self.node(b).base.has("noninterrupting") {
            self.frames[frame].active += 1;
            return self.leave(frame, b);
        }
        self.ais[ai].alive = false;
        self.ais[ai].end = Some(self.t);
        self.release(ai)?;
        for w in self.ais[ai].timers.clone() {
            self.waiters[w] = false;
        }
        if let Some(child) = self.ais[ai].child {
            self.kill(child)?;
        }
        self.log(ai, "interrupted");
        self.leave(frame, b)
    }

    fn start_sub(&mut self, frame: usize, n: &Node) -> R<()> {
        let Some(start) = self.start_of(node_id(n)) else {
            return self.start_activity(frame, n);
        };
        let ai = self.new_ai(frame, node_id(n));
        self.ais[ai].start = Some(self.t);
        self.frames[frame].ais.push(ai);
        self.arm_boundaries(ai)?;
        let case = self.frames[frame].case;
        let child = self.new_frame(case, node_id(n), Some(frame), Some(ai));
        self.ais[ai].child = Some(child);
        self.frames[frame].children.push(child);
        self.frames[child].active = 1;
        self.enter(child, &start)
    }

    fn finish_ai(&mut self, ai: usize) -> R<()> {
        if self.ais[ai].alive && self.frames[self.ais[ai].frame].alive {
            if let Some(tx) = self.ais[ai].cancel {
                self.ais[ai].alive = false;
                self.ais[ai].end = Some(self.t);
                return self.cancelled(tx);
            }
            self.complete(ai)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ language 0.3

    fn truth(&mut self, expr: &str, ctx: &Vars, what: &str) -> bool {
        match eval_expr(expr, ctx) {
            Ok(v) => pyval::truthy(&v),
            Err(ex) => {
                self.warnings
                    .push(format!("{what} could not be evaluated: {}", ex.message));
                false
            }
        }
    }

    /// Multi-instance activity with an input collection: one instance per element.
    fn start_multi(&mut self, frame: usize, n: &Node) -> R<()> {
        let a = &n.base.attrs;
        let vars = self.cases[self.frames[frame].case].vars.clone();
        let items = match eval_expr(&a["collection"], &vars) {
            Ok(v) => Some(v),
            Err(ex) => {
                self.warnings.push(format!(
                    "collection of '{}' could not be evaluated: {}",
                    node_id(n),
                    ex.message
                ));
                None
            }
        };
        let Some(PyValue::List(items)) = items else {
            return self.start_activity(frame, n);
        };
        let body = self.new_ai(frame, node_id(n));
        {
            let b = &mut self.ais[body];
            b.start = Some(self.t);
            b.instances = Some(Vec::new());
            b.results = vec![PyValue::None; items.len()];
            b.items = items;
        }
        self.frames[frame].ais.push(body);
        self.arm_boundaries(body)?;
        if self.ais[body].items.is_empty() {
            return self.finish_body(body);
        }
        let k = if n.base.has("parallel") {
            self.ais[body].items.len()
        } else {
            1
        };
        for _ in 0..k {
            self.next_instance(body)?;
        }
        Ok(())
    }

    fn next_instance(&mut self, body: usize) -> R<()> {
        let n = self.node(&self.ais[body].node);
        let i = self.ais[body].next;
        self.ais[body].next += 1;
        let frame = self.ais[body].frame;
        let inst = self.new_ai(frame, node_id(n));
        let mut local = Vars::new();
        if let Some(item) = n.base.attrs.get("item") {
            local.insert(item.clone(), self.ais[body].items[i].clone());
        }
        local.insert("loopCounter".into(), PyValue::Int((i + 1).to_string()));
        {
            let x = &mut self.ais[inst];
            x.body = Some(body);
            x.index = i;
            x.local = Some(local);
        }
        if let Some(list) = self.ais[body].instances.as_mut() {
            list.push(inst);
        }
        self.frames[frame].ais.push(inst);
        self.claim(inst, n)
    }

    fn instance_done(&mut self, inst: usize) -> R<()> {
        let Some(body) = self.ais[inst].body else {
            return Ok(());
        };
        let frame = self.ais[body].frame;
        if !self.ais[body].alive || !self.frames[frame].alive {
            return Ok(());
        }
        let n = self.node(&self.ais[body].node);
        let a = &n.base.attrs;
        self.ais[body].completed += 1;
        let mut ctx = self.cases[self.frames[frame].case].vars.clone();
        if let Some(l) = &self.ais[inst].local {
            for (k, v) in l {
                ctx.insert(k.clone(), v.clone());
            }
        }
        if let Some(oi) = a.get("output-item") {
            match eval_expr(oi, &ctx) {
                Ok(v) => {
                    let idx = self.ais[inst].index;
                    self.ais[body].results[idx] = v;
                }
                Err(ex) => self.warnings.push(format!(
                    "output-item of '{}' could not be evaluated: {}",
                    node_id(n),
                    ex.message
                )),
            }
        }
        let total = self.ais[body].items.len();
        let instances = self.ais[body].instances.clone().unwrap_or_default();
        let active = instances.iter().filter(|x| self.ais[**x].alive).count();
        if let Some(until) = a.get("until") {
            let mut uctx = ctx.clone();
            let completed = self.ais[body].completed;
            for (k, v) in [
                ("numberOfInstances", total),
                ("numberOfCompletedInstances", completed),
                ("numberOfActiveInstances", active),
                ("numberOfTerminatedInstances", 0),
            ] {
                uctx.insert(k.into(), PyValue::Int(v.to_string()));
            }
            let what = format!("until of '{}'", node_id(n));
            if self.truth(until, &uctx, &what) {
                for x in instances {
                    if self.ais[x].alive {
                        self.ais[x].alive = false;
                        self.release(x)?;
                        for w in self.ais[x].timers.clone() {
                            self.waiters[w] = false;
                        }
                    }
                }
                return self.finish_body(body);
            }
        }
        if self.ais[body].completed >= total {
            self.finish_body(body)
        } else if n.base.has("sequential") && self.ais[body].next < total {
            self.next_instance(body)
        } else {
            Ok(())
        }
    }

    fn finish_body(&mut self, body: usize) -> R<()> {
        let n = self.node(&self.ais[body].node);
        let a = &n.base.attrs;
        self.ais[body].alive = false;
        self.ais[body].end = Some(self.t);
        for w in self.ais[body].timers.clone() {
            self.waiters[w] = false;
        }
        let frame = self.ais[body].frame;
        let case = self.frames[frame].case;
        if let Some(out) = a.get("output") {
            let results = PyValue::List(self.ais[body].results.clone());
            self.cases[case].vars.insert(out.clone(), results);
        }
        if let Some(h) = a.get("compensate") {
            self.cases[case]
                .compensable
                .push((node_id(n).to_string(), h.clone(), frame));
        }
        self.leave(frame, node_id(n))
    }

    /// Ad-hoc sub-process: the listed activities start at once (all inner activities
    /// without incoming flows when no list is given); it completes when their paths have
    /// finished.
    fn start_adhoc(&mut self, frame: usize, n: &Node) -> R<()> {
        let inner: Vec<&Node> = n
            .children
            .iter()
            .filter_map(|c| self.doc.nodes.get(c))
            .collect();
        let mut chosen: Option<Vec<String>> = None;
        if let Some(active) = n.base.attrs.get("active") {
            let vars = self.cases[self.frames[frame].case].vars.clone();
            match eval_expr(active, &vars) {
                Ok(PyValue::List(items)) => {
                    chosen = Some(
                        items
                            .into_iter()
                            .filter_map(|x| match x {
                                PyValue::Str(s) => Some(s),
                                _ => None,
                            })
                            .collect(),
                    )
                }
                Ok(_) => {}
                Err(ex) => self.warnings.push(format!(
                    "active of '{}' could not be evaluated: {}",
                    node_id(n),
                    ex.message
                )),
            }
        }
        let chosen = chosen.unwrap_or_else(|| {
            inner
                .iter()
                .filter(|x| {
                    is_activity(&x.base.kind) && self.inc.get(node_id(x)).copied().unwrap_or(0) == 0
                })
                .map(|x| node_id(x).to_string())
                .collect()
        });
        let chosen: Vec<String> = chosen
            .into_iter()
            .filter(|c| inner.iter().any(|x| node_id(x) == c))
            .collect();
        let ai = self.new_ai(frame, node_id(n));
        self.ais[ai].start = Some(self.t);
        self.frames[frame].ais.push(ai);
        self.arm_boundaries(ai)?;
        if chosen.is_empty() {
            return self.complete(ai);
        }
        let case = self.frames[frame].case;
        let child = self.new_frame(case, node_id(n), Some(frame), Some(ai));
        if let Some(until) = n.base.attrs.get("until") {
            self.frames[child].until = Some((until.clone(), n.base.has("wait")));
        }
        self.ais[ai].child = Some(child);
        self.frames[frame].children.push(child);
        self.frames[child].active = chosen.len() as i64;
        for c in &chosen {
            self.enter(child, c)?;
        }
        Ok(())
    }

    /// Run the compensation handlers of completed activities; continue at `n` when they
    /// finish. Returns false when there is nothing to compensate.
    fn start_compensation(&mut self, frame: usize, n: &Node) -> R<bool> {
        let case = self.frames[frame].case;
        let target = n.base.attrs.get("activity");
        let todo: Vec<String> = self.cases[case]
            .compensable
            .iter()
            .rev()
            .filter(|(act, _, _)| target.is_none_or(|t| act == t))
            .map(|(_, h, _)| h.clone())
            .filter(|h| self.doc.nodes.contains_key(h))
            .collect();
        if todo.is_empty() {
            return Ok(false);
        }
        self.cases[case]
            .compensable
            .retain(|(act, _, _)| !target.is_none_or(|t| act == t));
        if n.base.kind == "end" {
            self.frames[frame].last_end = Some(node_id(n).to_string());
        }
        let ai = self.new_ai(frame, node_id(n));
        self.ais[ai].start = Some(self.t);
        self.ais[ai].silent = true;
        self.frames[frame].ais.push(ai);
        let scope = self.frames[frame].scope.clone();
        let child = self.new_frame(case, &scope, Some(frame), Some(ai));
        self.ais[ai].child = Some(child);
        self.frames[frame].children.push(child);
        self.frames[child].active = todo.len() as i64;
        for h in todo {
            let hn = self.node(&h);
            self.start_activity(child, hn)?;
        }
        Ok(true)
    }

    // ------------------------------------------------------------ language 0.5

    /// A standard loop with a condition (`while=`) or a maximum (`loop-max=`). A `loop`
    /// marker without either runs once, as in language 0.3.
    fn standard_loop(&self, n: &Node) -> bool {
        let k = n.base.kind.as_str();
        self.lang05
            && n.base.has("loop")
            && is_activity(k)
            && k != "eventsub"
            && (n.base.attrs.contains_key("while") || n.base.attrs.contains_key("loop-max"))
    }

    /// Whether a standard loop runs again after `done` iterations: fewer than `loop-max`
    /// and `while` holds (absent: true). `loopCounter` is the number of iterations done.
    fn loop_again(&mut self, frame: usize, n: &Node, done: u64) -> bool {
        if let Some(max) = n.base.attr("loop-max").and_then(parse_float)
            && done as f64 >= max
        {
            return false;
        }
        let Some(expr) = n.base.attr("while") else {
            return true;
        };
        let mut ctx = self.cases[self.frames[frame].case].vars.clone();
        ctx.insert("loopCounter".into(), PyValue::Int(done.to_string()));
        let what = format!("while of '{}'", node_id(n));
        self.truth(expr, &ctx, &what)
    }

    /// A catching event with two or more triggers.
    fn multiple(&self, n: &Node) -> bool {
        self.lang05
            && TRIGGERS
                .iter()
                .map(|k| n.base.values(k).len())
                .sum::<usize>()
                >= 2
    }

    /// Waiting time of a multiple event: the earliest of its triggers, or with `parallel`
    /// the latest. A timer waits for its duration; every other trigger waits for the
    /// event's `delay=` (one draw for all of them). Without `delay=` such a trigger
    /// happens at once on a `catch` event (as a single one does) and never on a boundary
    /// event or after an event-based gateway.
    fn multiple_wait(&mut self, n: &Node, catch: bool) -> R<Option<Sample>> {
        let mut waits: Vec<Option<f64>> = Vec::new();
        let timers = n.base.values("timer");
        if !timers.is_empty() {
            let now = self.now_dt()?;
            for timer in timers {
                let w = timer_seconds(timer, &now)?;
                if w.is_none() {
                    self.warnings.push(format!(
                        "timer of '{}' is not understood: {timer}",
                        node_id(n)
                    ));
                }
                waits.push(w);
            }
        }
        let others: usize = TRIGGERS
            .iter()
            .filter(|k| **k != "timer")
            .map(|k| n.base.values(k).len())
            .sum();
        if others > 0 {
            let w = match n.base.attr("delay") {
                Some(d) => Some(self.draw(d)?.delay()?),
                None if catch => Some(0.0),
                None => None,
            };
            waits.extend(std::iter::repeat_n(w, others));
        }
        let w = if n.base.has("parallel") {
            waits
                .into_iter()
                .collect::<Option<Vec<f64>>>()
                .and_then(|ws| ws.into_iter().reduce(f64::max))
        } else {
            waits.into_iter().flatten().reduce(f64::min)
        };
        Ok(w.map(Sample::Num))
    }

    /// Join of a complex gateway with an activation condition. Each arriving token
    /// counts in `_activationCount`; the first token for which `activation` holds goes
    /// on, the later ones are consumed until all expected tokens have arrived (one per
    /// incoming flow, or as many as the matching `or` split activated), and then the
    /// gateway waits for the next activation. Returns whether this token goes on.
    fn complex_join(&mut self, frame: usize, id: &str, expr: &str, nin: i64) -> R<bool> {
        let case = self.frames[frame].case;
        let f = &mut self.frames[frame];
        let (expected, fired) = match f.complex.get(id) {
            Some(state) => *state,
            None => (f.or_stack.pop().unwrap_or(nin), false),
        };
        let got = f.joins.get(id).copied().unwrap_or(0) + 1;
        let fire = !fired && {
            let mut ctx = self.cases[case].vars.clone();
            ctx.insert("_activationCount".into(), PyValue::Int(got.to_string()));
            let what = format!("activation of '{id}'");
            self.truth(expr, &ctx, &what)
        };
        let f = &mut self.frames[frame];
        if got >= expected && (fired || fire) {
            f.joins.insert(id.to_string(), 0);
            f.complex.shift_remove(id);
        } else {
            // a count left above zero when every token has arrived without activation
            // reports the case as a deadlock, as an unfinished join does
            f.joins.insert(id.to_string(), got);
            f.complex.insert(id.to_string(), (expected, fired || fire));
        }
        if !fire {
            self.consume(frame)?;
        }
        Ok(fire)
    }

    /// The frame of the transaction that a cancel end event in `frame` cancels.
    fn transaction_frame(&self, frame: usize) -> Option<usize> {
        let mut cur = Some(frame);
        while let Some(i) = cur {
            if let Some(ai) = self.frames[i].ai
                && self.ais[ai].cancel.is_none()
                && self.node(&self.ais[ai].node).base.kind == "transaction"
            {
                return Some(i);
            }
            cur = self.frames[i].parent;
        }
        None
    }

    /// Cancel end event: stop every token of the transaction, run the compensation
    /// handlers of the activities completed inside it (the latest first, all at once as
    /// a compensation throw does) and then leave through the cancel boundary event.
    fn cancel_transaction(&mut self, tx: usize) -> R<()> {
        let (Some(tx_ai), Some(parent)) = (self.frames[tx].ai, self.frames[tx].parent) else {
            return Ok(());
        };
        let case = self.frames[tx].case;
        self.kill(tx)?;
        self.ais[tx_ai].alive = false;
        self.release(tx_ai)?;
        for w in self.ais[tx_ai].timers.clone() {
            self.waiters[w] = false;
        }
        let frames = &self.frames;
        let todo: Vec<String> = self.cases[case]
            .compensable
            .iter()
            .rev()
            .filter(|(_, _, f)| within(frames, *f, tx))
            .map(|(_, h, _)| h.clone())
            .filter(|h| self.doc.nodes.contains_key(h))
            .collect();
        self.cases[case]
            .compensable
            .retain(|(_, _, f)| !within(frames, *f, tx));
        if todo.is_empty() {
            return self.cancelled(tx_ai);
        }
        let tx_id = self.ais[tx_ai].node.clone();
        let ai = self.new_ai(parent, &tx_id);
        self.ais[ai].start = Some(self.t);
        self.ais[ai].cancel = Some(tx_ai);
        self.frames[parent].ais.push(ai);
        let child = self.new_frame(case, &tx_id, Some(parent), Some(ai));
        self.ais[ai].child = Some(child);
        self.frames[parent].children.push(child);
        self.frames[child].active = todo.len() as i64;
        for h in todo {
            let hn = self.node(&h);
            self.start_activity(child, hn)?;
        }
        Ok(())
    }

    /// The end of a cancelled transaction, after its compensation: the instance is
    /// logged as interrupted and the token follows the cancel boundary event. A
    /// cancelled transaction does not complete, so without a cancel boundary event its
    /// outgoing flows are not taken and the token ends there (in BPMN only the cancel
    /// event attached to the transaction continues the process).
    fn cancelled(&mut self, tx_ai: usize) -> R<()> {
        self.ais[tx_ai].end = Some(self.t);
        self.log(tx_ai, "interrupted");
        let frame = self.ais[tx_ai].frame;
        let boundary = self
            .boundaries
            .get(&self.ais[tx_ai].node)
            .and_then(|bs| bs.iter().find(|b| self.node(b).base.has("cancel")).cloned());
        match boundary {
            Some(b) => self.leave(frame, &b),
            None => self.consume(frame),
        }
    }
}

/// Whether `frame` is `ancestor` or runs inside it.
fn within(frames: &[Frame], frame: usize, ancestor: usize) -> bool {
    let mut cur = Some(frame);
    while let Some(i) = cur {
        if i == ancestor {
            return true;
        }
        cur = frames[i].parent;
    }
    false
}
