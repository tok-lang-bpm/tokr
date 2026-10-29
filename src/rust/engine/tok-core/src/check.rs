//! Semantic checks (`tok check`). Mirrors `check.py`: references and structure (R),
//! executable profile (C), decisions and forms (D, M), FEEL (F), simulation (S), levels,
//! and finally the BPMN 2.0 rules, which run only on documents without errors.

use crate::diagnostics::{Diag, LEVELS, Severity};
use crate::dmn;
use crate::feel;
use crate::model::*;
use crate::simcheck;

#[path = "check_06.rs"]
mod lang06;
#[path = "check_07.rs"]
mod lang07;

pub use lang06::choreography_participants;

/// One finding of the BPMN rule validator (`bpmnlib.validate`).
#[derive(Debug, Clone)]
pub struct BpmnIssue {
    /// `błąd`, `ostrzeżenie` or `uwaga`
    pub level: String,
    pub code: String,
    pub id: Option<String>,
    pub message: String,
}

/// The BPMN 2.0 rule validator, provided by the `tok-bpmn` crate.
pub trait BpmnRules {
    /// Validate BPMN XML text as produced by [`crate::export::export_xml`].
    fn validate(&self, xml: &str) -> Vec<BpmnIssue>;
}

const BASIC_KINDS: &[&str] = &[
    "start", "end", "task", "user", "service", "manual", "xor", "and",
];
const ANALYTIC_EXCLUDED_BASE: &[&str] =
    &["form", "decision", "expr", "result", "type", "correlation"];
const EXECUTION_EVENTS: &[&str] = &["start", "end"];
const TASK_EVENTS: &[&str] = &[
    "creating",
    "assigning",
    "updating",
    "completing",
    "canceling",
];

/// Keys rejected at the level analytic: the executable profile and the executable layer.
fn analytic_excluded(kind: &str, k: &str) -> bool {
    ANALYTIC_EXCLUDED_BASE.contains(&k) || is_exec_key(kind, k)
}

/// A key of the executable layer on an element of kind `kind` (`method=` of a transaction
/// is BPMN, not the method of a REST connector).
fn is_exec_key(kind: &str, k: &str) -> bool {
    EXEC_KEYS.contains(&k) && !(kind == "transaction" && k == "method")
}

/// `'a = x; b = y'` (sep `=`) or `'k: v; …'` (sep `:`) into pairs; an item without the
/// separator gives `(item, None)` (Python `split_pairs` of `check.py`).
fn split_pairs(text: &str, sep: char) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for item in text.split(';') {
        let item = crate::pyutil::strip(item);
        if item.is_empty() {
            continue;
        }
        match item.split_once(sep) {
            None => out.push((item.to_string(), None)),
            Some((l, r)) => out.push((
                crate::pyutil::strip(l).to_string(),
                Some(crate::pyutil::strip(r).to_string()),
            )),
        }
    }
    out
}

/// `^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$`
fn is_target(s: &str) -> bool {
    let s = s.strip_suffix('\n').unwrap_or(s);
    !s.is_empty()
        && s.split('.').all(|part| {
            let mut ch = part.chars();
            matches!(ch.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// An XML name without a namespace prefix (an NCName): a letter or `_`, then letters,
/// digits, `_`, `-` and `.`.
fn is_xml_name(v: &str) -> bool {
    let mut ch = v.chars();
    matches!(ch.next(), Some(c) if c.is_alphabetic() || c == '_')
        && ch.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// `n` ASCII digits at `b[at..]`.
fn digits_at(b: &[u8], at: usize, n: usize) -> bool {
    b.len() >= at + n && b[at..at + n].iter().all(u8::is_ascii_digit)
}

/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})(\[[^\]]+\])?$`
fn is_iso_zoned(v: &str) -> bool {
    let v = v.strip_suffix('\n').unwrap_or(v);
    let b = v.as_bytes();
    if !(digits_at(b, 0, 4)
        && b.get(4) == Some(&b'-')
        && digits_at(b, 5, 2)
        && b.get(7) == Some(&b'-')
        && digits_at(b, 8, 2)
        && b.get(10) == Some(&b'T')
        && digits_at(b, 11, 2)
        && b.get(13) == Some(&b':')
        && digits_at(b, 14, 2))
    {
        return false;
    }
    let mut k = 16;
    if b.get(k) == Some(&b':') && digits_at(b, k + 1, 2) {
        k += 3;
        if b.get(k) == Some(&b'.') {
            let n = b[k + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if n > 0 {
                k += 1 + n;
            }
        }
    }
    match b.get(k) {
        Some(b'Z') => k += 1,
        Some(b'+' | b'-')
            if digits_at(b, k + 1, 2) && b.get(k + 3) == Some(&b':') && digits_at(b, k + 4, 2) =>
        {
            k += 6
        }
        _ => return false,
    }
    let rest = &b[k..];
    if rest.is_empty() {
        return true;
    }
    rest.len() >= 3
        && rest[0] == b'['
        && rest[rest.len() - 1] == b']'
        && !rest[1..rest.len() - 1].contains(&b']')
}

/// `'start: type'` or `'start: type (5)'` as checked by `tok check` (None when malformed).
fn parse_listener(item: &str) -> Option<(String, String, Option<String>)> {
    crate::export::parse_listener(item).filter(|(_, t, _)| !t.is_empty())
}

fn kind_name(what: &str) -> (&'static str, &'static str) {
    match what {
        "node" => ("flow node", "element przepływu"),
        "activity" => ("activity", "czynność"),
        "reader" => (
            "activity, end or intermediate throw event",
            "czynność, zdarzenie końcowe lub pośrednie rzucające",
        ),
        "writer" => (
            "activity, start, intermediate catch or boundary event",
            "czynność, zdarzenie początkowe, pośrednie przechwytujące lub graniczne",
        ),
        "io owner" => ("activity or event", "czynność lub zdarzenie"),
        "data" => ("data object or store", "obiekt lub magazyn danych"),
        "form" => ("form", "formularz"),
        "decision" => ("decision", "decyzja"),
        "resource" => ("resource", "zasób"),
        "choreography" => ("choreography", "choreografia"),
        "message flow" => ("message flow", "przepływ komunikatów"),
        "requirement" => (
            "decision, input data, business knowledge model or knowledge source of an allowed kind",
            "decyzja, dane wejściowe, model wiedzy biznesowej albo źródło wiedzy dozwolonego rodzaju",
        ),
        "bkm" => ("business knowledge model", "model wiedzy biznesowej"),
        _ => ("pool", "basen"),
    }
}

struct Checker<'a> {
    doc: &'a mut Document,
    level: &'static str,
    diags: Vec<Diag>,
}

/// Run all checks. Parser diagnostics are not included.
pub fn check(doc: &mut Document, level: &str, bpmn: Option<&dyn BpmnRules>) -> Vec<Diag> {
    let level = LEVELS
        .iter()
        .find(|l| **l == level)
        .copied()
        .unwrap_or("full");
    let mut c = Checker {
        doc,
        level,
        diags: Vec::new(),
    };
    c.attributes();
    c.structure();
    c.references();
    c.flows();
    if c.doc.executable_any() {
        c.executable();
    }
    c.layer_03();
    c.layer_05();
    let camunda = c.doc.attrs.get("target").is_none_or(|t| t == "camunda");
    c.layer_06(camunda);
    if c.doc.lang_07() {
        c.layer_07(camunda);
    }
    if c.doc.lang_10() {
        c.layer_10();
    }
    c.decisions_and_forms();
    c.feel();
    c.simulation();
    c.levels();
    if let Some(rules) = bpmn
        && !c.diags.iter().any(Diag::is_error)
    {
        c.bpmn_rules(rules);
    }
    c.diags
}

fn s(v: &str) -> String {
    v.to_string()
}

impl Checker<'_> {
    /// Deprecations of language 1.0 (`R090`): a key or flag given more than once on one
    /// line, whose earlier values the language drops. The keys that may repeat (the
    /// triggers of events and `error` of operations, language 0.5) are not repeated here.
    fn layer_10(&mut self) {
        let repeated = std::mem::take(&mut self.doc.repeated);
        for (line, key) in &repeated {
            self.add("R090", *line, None, &[("key", key.clone())]);
        }
        self.doc.repeated = repeated;
    }

    fn add(&mut self, code: &str, line: usize, id: Option<&str>, args: &[(&str, String)]) {
        self.diags.push(Diag::new(code, line, id, args));
    }

    /// Kind of the element named `id`, or a diagnostic (R002, R003) when it is missing or wrong.
    fn reference(
        &mut self,
        id: &str,
        line: usize,
        what: &str,
        ok: &dyn Fn(&str) -> bool,
    ) -> Option<String> {
        let (en, pl) = kind_name(what);
        let Some(entry) = self.doc.ids.get(id) else {
            self.add(
                "R002",
                line,
                Some(id),
                &[("what", s(en)), ("what_pl", s(pl)), ("id", s(id))],
            );
            return None;
        };
        let kind = entry.kind.clone();
        if !ok(&kind) {
            self.add(
                "R003",
                line,
                Some(id),
                &[
                    ("id", s(id)),
                    ("actual", kind),
                    ("expected", s(en)),
                    ("expected_pl", s(pl)),
                ],
            );
            return None;
        }
        Some(kind)
    }

    fn attributes(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        for n in d.nodes.values() {
            let b = &n.base;
            let keys = node_keys(&b.kind);
            let flags = node_flags(&b.kind);
            for k in b.attrs.keys() {
                if !keys.contains(&k.as_str()) {
                    found.push(Diag::new(
                        "R010",
                        b.line,
                        b.id.as_deref(),
                        &[("key", k.clone()), ("kind", b.kind.clone())],
                    ));
                }
            }
            for f in &b.flags {
                if !flags.contains(&f.as_str()) {
                    found.push(Diag::new(
                        "R011",
                        b.line,
                        b.id.as_deref(),
                        &[("flag", f.clone()), ("kind", b.kind.clone())],
                    ));
                }
            }
            if is_event(&b.kind) && !d.lang_05() {
                let mut trig: Vec<&str> = TRIGGERS
                    .iter()
                    .copied()
                    .filter(|k| b.attrs.contains_key(*k))
                    .collect();
                trig.extend(
                    TRIGGER_FLAGS
                        .iter()
                        .chain(CATCH_ALL_FLAGS)
                        .chain(NAMELESS_TRIGGERS)
                        .copied()
                        .filter(|f| b.has(f)),
                );
                if trig.len() > 1 {
                    found.push(Diag::new(
                        "R012",
                        b.line,
                        b.id.as_deref(),
                        &[("triggers", trig.join(", "))],
                    ));
                }
            }
        }
        let mut groups: Vec<(Vec<&Base>, &[&str], &[&str])> = vec![
            (
                d.flows.iter().map(|f| &f.base).collect(),
                FLOW_KEYS,
                FLOW_FLAGS,
            ),
            (
                d.messages.iter().map(|m| &m.base).collect(),
                MESSAGE_KEYS,
                &[],
            ),
            (
                d.data
                    .values()
                    .map(|x| &x.base)
                    .filter(|b| !IO_KINDS.contains(&b.kind.as_str()))
                    .collect(),
                DATA_KEYS,
                DATA_FLAGS,
            ),
            (
                d.data
                    .values()
                    .map(|x| &x.base)
                    .filter(|b| b.kind == "input" || b.kind == "output")
                    .collect(),
                IO_KEYS,
                IO_FLAGS,
            ),
            (
                d.data
                    .values()
                    .map(|x| &x.base)
                    .filter(|b| b.kind == "property")
                    .collect(),
                IO_KEYS,
                &[],
            ),
            (
                d.items.values().collect(),
                if d.lang_07() { ITEM_KEYS_07 } else { ITEM_KEYS },
                ITEM_FLAGS,
            ),
            (d.globals.values().collect(), GLOBAL_KEYS, &[]),
            (
                d.interfaces.values().map(|x| &x.base).collect(),
                INTERFACE_KEYS,
                &[],
            ),
            (
                d.interfaces
                    .values()
                    .flat_map(|x| x.operations.iter())
                    .collect(),
                OPERATION_KEYS,
                &[],
            ),
            (d.keys.values().map(|x| &x.base).collect(), &[], &[]),
            (
                d.keys.values().flat_map(|x| x.props.iter()).collect(),
                PROP_KEYS,
                &[],
            ),
            (d.notes.iter().collect(), NOTE_KEYS, &[]),
            (d.groups.iter().collect(), GROUP_KEYS, &[]),
            (
                d.pools.iter().map(|p| &p.base).collect(),
                d.pool_keys(),
                POOL_FLAGS,
            ),
            (d.lanes.values().map(|l| &l.base).collect(), LANE_KEYS, &[]),
            (
                d.assocs
                    .iter()
                    .map(|a| &a.base)
                    .filter(|b| b.kind == "reads")
                    .collect(),
                READS_KEYS,
                &[],
            ),
            (
                d.assocs
                    .iter()
                    .map(|a| &a.base)
                    .filter(|b| b.kind == "writes")
                    .collect(),
                WRITES_KEYS,
                &[],
            ),
            (
                d.decisions.values().map(|x| &x.base).collect(),
                if d.lang_07() {
                    DECISION_KEYS_07
                } else {
                    DECISION_KEYS
                },
                &[],
            ),
            (d.resources.values().collect(), RESOURCE_KEYS, &[]),
        ];
        if let Some(sim) = &d.simulate {
            groups.push((vec![sim], SIMULATE_KEYS, &[]));
        }
        for f in d.forms.values() {
            groups.push((
                f.items
                    .iter()
                    .map(|i| &i.base)
                    .filter(|b| b.kind == "field")
                    .collect(),
                FIELD_KEYS,
                FIELD_FLAGS,
            ));
        }
        for (items, keys, flags) in groups {
            for b in items {
                for k in b.attrs.keys() {
                    if !keys.contains(&k.as_str()) && k != "id" {
                        found.push(Diag::new(
                            "R010",
                            b.line,
                            b.id.as_deref(),
                            &[("key", k.clone()), ("kind", b.kind.clone())],
                        ));
                    }
                }
                for f in &b.flags {
                    if !flags.contains(&f.as_str()) {
                        found.push(Diag::new(
                            "R011",
                            b.line,
                            b.id.as_deref(),
                            &[("flag", f.clone()), ("kind", b.kind.clone())],
                        ));
                    }
                }
            }
        }
        for k in d.attrs.keys() {
            if !HEADER_KEYS.contains(&k.as_str()) {
                found.push(Diag::new(
                    "R010",
                    d.header_line,
                    None,
                    &[("key", k.clone()), ("kind", s("tok"))],
                ));
            }
        }
        for f in &d.flags {
            if !HEADER_FLAGS.contains(&f.as_str()) {
                found.push(Diag::new(
                    "R011",
                    d.header_line,
                    None,
                    &[("flag", f.clone()), ("kind", s("tok"))],
                ));
            }
        }
        self.diags.extend(found);
    }

    fn structure(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        if !d.pools.is_empty() {
            for i in &d.top {
                if let Some(n) = d.nodes.get(i) {
                    found.push(Diag::new(
                        "R031",
                        n.base.line,
                        Some(i),
                        &[("id", i.clone())],
                    ));
                }
            }
        }
        for p in &d.pools {
            if p.children.iter().any(|c| d.lanes.contains_key(c)) {
                for c in &p.children {
                    if let Some(n) = d.nodes.get(c) {
                        found.push(Diag::new(
                            "R030",
                            n.base.line,
                            Some(c),
                            &[("id", c.clone())],
                        ));
                    }
                }
            }
        }
        self.diags.extend(found);
    }

    fn references(&mut self) {
        let is_act = |k: &str| is_activity(k);
        // a boundary event of a choreography is attached to a choreography activity
        let host = |k: &str| is_activity(k) || is_choreo(k);
        let node_ids: Vec<String> = self.doc.nodes.keys().cloned().collect();
        for nid in node_ids {
            let n = self.doc.nodes[&nid].clone();
            let a = &n.base.attrs;
            let line = n.base.line;
            if n.base.kind == "boundary" {
                match a.get("on") {
                    None => self.add("R032", line, Some(&nid), &[("id", nid.clone())]),
                    Some(on) => {
                        if self.reference(on, line, "activity", &host).is_some() {
                            let scope = self.doc.nodes.get(on).and_then(|x| x.scope.clone());
                            if let Some(node) = self.doc.nodes.get_mut(&nid) {
                                node.scope = scope;
                            }
                        }
                    }
                }
            } else if a.contains_key("on") {
                self.add(
                    "R013",
                    line,
                    Some(&nid),
                    &[("key", s("on")), ("requirement", s("a boundary event"))],
                );
            }
            if let Some(f) = a.get("form") {
                self.reference(f, line, "form", &|k| k == "form");
                if n.base.kind != "user" && n.base.kind != "start" {
                    self.add(
                        "R013",
                        line,
                        Some(&nid),
                        &[
                            ("key", s("form")),
                            ("requirement", s("a user task or a start event")),
                        ],
                    );
                }
            }
            if let Some(dec) = a.get("decision") {
                self.reference(dec, line, "decision", &|k| k == "decision");
                if n.base.kind != "rule" {
                    self.add(
                        "R013",
                        line,
                        Some(&nid),
                        &[("key", s("decision")), ("requirement", s("a rule task"))],
                    );
                }
            }
            if let Some(by) = a.get("by") {
                self.reference(by, line, "resource", &|k| k == "resource");
            }
            if a.contains_key("expr") && n.base.kind != "script" {
                self.add(
                    "R013",
                    line,
                    Some(&nid),
                    &[("key", s("expr")), ("requirement", s("a script task"))],
                );
            }
            if a.contains_key("process") && n.base.kind != "call" {
                self.add(
                    "R013",
                    line,
                    Some(&nid),
                    &[("key", s("process")), ("requirement", s("a call activity"))],
                );
            }
            if let Some(h) = a.get("compensate")
                && self.reference(h, line, "activity", &is_act).is_some()
            {
                let has_flag = self
                    .doc
                    .nodes
                    .get(h)
                    .is_some_and(|x| x.base.has("compensation"));
                if !has_flag {
                    self.add(
                        "R051",
                        line,
                        Some(&nid),
                        &[
                            (
                                "detail",
                                format!("compensation handler '{h}' needs the flag compensation"),
                            ),
                            (
                                "detail_pl",
                                format!("procedura kompensacji „{h}” wymaga flagi compensation"),
                            ),
                        ],
                    );
                }
            }
            if let Some(act) = a.get("activity") {
                let kind = n.base.kind.as_str();
                if !((kind == "throw" || kind == "end") && n.base.has("compensate")) {
                    self.add(
                        "R013",
                        line,
                        Some(&nid),
                        &[
                            ("key", s("activity")),
                            ("requirement", s("a compensation throw or end event")),
                        ],
                    );
                } else {
                    self.reference(act, line, "activity", &is_act);
                }
            }
        }
        let assocs: Vec<(String, String, usize, bool)> = self
            .doc
            .assocs
            .iter()
            .map(|x| {
                (
                    x.activity.clone(),
                    x.data.clone(),
                    x.base.line,
                    x.base.kind == "reads",
                )
            })
            .collect();
        let lang_05 = self.doc.lang_05();
        for (act, data, line, reads) in assocs {
            // a document of language 0.5: a throwing event reads data, a catching event
            // writes it (in 0.1 and 0.3 only activities do, as before)
            let owner = |k: &str| {
                is_activity(k)
                    || lang_05
                        && if reads {
                            THROWING_EVENTS.contains(&k)
                        } else {
                            CATCHING_EVENTS.contains(&k)
                        }
            };
            let what = match (lang_05, reads) {
                (false, _) => "activity",
                (true, true) => "reader",
                (true, false) => "writer",
            };
            self.reference(&act, line, what, &owner);
            self.reference(&data, line, "data", &|k| {
                k == "data" || k == "store" || IO_KINDS.contains(&k)
            });
        }
        // language 0.5: `reads … input=ID`, `writes … output=ID` name a data input or output
        // declared for the activity (`input ID on=ACTIVITY`)
        let ends: Vec<(String, &'static str, String, usize)> = self
            .doc
            .assocs
            .iter()
            .filter_map(|a| {
                let key = if a.base.kind == "reads" {
                    "input"
                } else {
                    "output"
                };
                a.base
                    .attr(key)
                    .map(|v| (a.activity.clone(), key, v.to_string(), a.base.line))
            })
            .collect();
        for (act, key, io, line) in ends {
            let what = if key == "input" {
                ("data input", "wejście danych")
            } else {
                ("data output", "wyjście danych")
            };
            if self.declared(&io, line, Some(&act), &[key], key, what) {
                // an input of the activity (`on=`), or of the sub-process in whose block it stands
                let owner = self.doc.data.get(&io).and_then(|x| {
                    x.base
                        .attr("on")
                        .map(str::to_string)
                        .or_else(|| x.scope.clone())
                });
                if owner.as_deref() != Some(act.as_str()) {
                    self.add(
                        "R013",
                        line,
                        Some(&act),
                        &[
                            ("key", s(key)),
                            ("requirement", format!("{} declared with on={act}", what.0)),
                        ],
                    );
                }
            }
        }
        let objects: Vec<(String, usize)> = self
            .doc
            .data
            .values()
            .filter(|x| x.base.kind != "data" && x.base.attrs.contains_key("object"))
            .map(|x| (x.base.id().to_string(), x.base.line))
            .collect();
        for (id, line) in objects {
            self.add(
                "R013",
                line,
                Some(&id),
                &[
                    ("key", s("object")),
                    ("requirement", s("a data object (data)")),
                ],
            );
        }
        if self.doc.attrs.contains_key("process-name") && !self.doc.pools.is_empty() {
            let line = self.doc.header_line;
            self.add(
                "R013",
                line,
                None,
                &[
                    ("key", s("process-name")),
                    (
                        "requirement",
                        s("a document without pools (a pool names its own process)"),
                    ),
                ],
            );
        }
        let blackbox: Vec<(String, usize)> = self
            .doc
            .pools
            .iter()
            .filter(|p| p.blackbox && p.base.attrs.contains_key("process-name"))
            .map(|p| (p.base.id().to_string(), p.base.line))
            .collect();
        for (id, line) in blackbox {
            self.add(
                "R013",
                line,
                Some(&id),
                &[
                    ("key", s("process-name")),
                    ("requirement", s("a pool with a process (a block)")),
                ],
            );
        }
        let notes: Vec<(Option<String>, usize)> = self
            .doc
            .notes
            .iter()
            .map(|n| (n.attrs.get("on").cloned(), n.line))
            .collect();
        for (on, line) in notes {
            if let Some(on) = on {
                self.reference(&on, line, "node", &|_| true);
            }
        }
        let groups: Vec<(String, usize)> = self
            .doc
            .groups
            .iter()
            .map(|g| (g.attrs.get("members").cloned().unwrap_or_default(), g.line))
            .collect();
        for (members, line) in groups {
            for m in crate::pyutil::split_ws(&members) {
                self.reference(m, line, "node", &|k| is_node(k));
            }
        }
    }

    fn flows(&mut self) {
        let flows: Vec<(String, String, usize, String)> = self
            .doc
            .flows
            .iter()
            .map(|f| {
                (
                    f.source.clone(),
                    f.target.clone(),
                    f.base.line,
                    f.base.id().to_string(),
                )
            })
            .collect();
        for (src, tgt, line, id) in flows {
            let a = self.reference(&src, line, "node", &|k| is_node(k) || is_choreo(k));
            let b = self.reference(&tgt, line, "node", &|k| is_node(k) || is_choreo(k));
            if a.is_some() && b.is_some() {
                let ss = self.doc.nodes.get(&src).and_then(|n| n.scope.clone());
                let ts = self.doc.nodes.get(&tgt).and_then(|n| n.scope.clone());
                if ss != ts {
                    self.add(
                        "R021",
                        line,
                        Some(&id),
                        &[("source", src.clone()), ("target", tgt.clone())],
                    );
                }
            }
        }
        let msgs: Vec<(String, String, usize, String)> = self
            .doc
            .messages
            .iter()
            .map(|m| {
                (
                    m.source.clone(),
                    m.target.clone(),
                    m.base.line,
                    m.base.id().to_string(),
                )
            })
            .collect();
        let ok = |k: &str| is_node(k) || k == "pool";
        for (src, tgt, line, id) in msgs {
            let a = self.reference(&src, line, "node", &ok);
            let b = self.reference(&tgt, line, "node", &ok);
            if let (Some(ka), Some(kb)) = (a, b) {
                let pool_of = |id: &str, kind: &str| -> Option<String> {
                    if kind == "pool" {
                        Some(id.to_string())
                    } else {
                        self.doc.nodes.get(id).and_then(|n| n.pool.clone())
                    }
                };
                let ps = pool_of(&src, &ka);
                let pt = pool_of(&tgt, &kb);
                if ps.is_none() || pt.is_none() || ps == pt {
                    self.add(
                        "R022",
                        line,
                        Some(&id),
                        &[("source", src.clone()), ("target", tgt.clone())],
                    );
                }
            }
        }
    }

    fn executable(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        let mut outs: indexmap::IndexMap<&str, Vec<&Flow>> = Default::default();
        for f in &d.flows {
            outs.entry(f.source.as_str()).or_default().push(f);
        }
        for n in d.nodes.values().filter(|n| n.choreography.is_none()) {
            let a = &n.base.attrs;
            let id = n.base.id();
            let kind = n.base.kind.as_str();
            let line = n.base.line;
            if kind == "user" && !a.contains_key("form") {
                found.push(Diag::text(
                    "C011",
                    line,
                    Some(id),
                    Severity::Note,
                    format!("Zadanie użytkownika „{id}” nie ma formularza (form=)."),
                    Some(format!("user task '{id}' has no form (form=)")),
                ));
            } else if (kind == "service" || kind == "send")
                && !a.contains_key("type")
                && a.get("connector").map(String::as_str) != Some("rest")
            {
                let pl_kind = if kind == "service" {
                    "Zadanie usługowe"
                } else {
                    "Zadanie wysyłania"
                };
                found.push(Diag::text(
                    "C020",
                    line,
                    Some(id),
                    Severity::Error,
                    format!("{pl_kind} „{id}” wymaga type=\"…\" (typ zadania dla workera)."),
                    Some(format!(
                        "{kind} task '{id}' needs type=\"…\" (job type for a worker)"
                    )),
                ));
            } else if kind == "rule" && !a.contains_key("decision") {
                found.push(Diag::text(
                    "C030",
                    line,
                    Some(id),
                    Severity::Error,
                    format!("Zadanie reguły biznesowej „{id}” wymaga decision=ID."),
                    Some(format!("rule task '{id}' needs decision=ID")),
                ));
            } else if kind == "script" && !a.contains_key("expr") {
                found.push(Diag::text(
                    "C040",
                    line,
                    Some(id),
                    Severity::Error,
                    format!("Zadanie skryptowe „{id}” wymaga expr=\"…\"."),
                    Some(format!("script task '{id}' needs expr=\"…\"")),
                ));
            }
            if (((kind == "catch" || kind == "boundary") && a.contains_key("message"))
                || kind == "receive")
                && !a.contains_key("correlation")
            {
                found.push(Diag::text(
                    "C052",
                    line,
                    Some(id),
                    Severity::Error,
                    format!("„{id}” czeka na wiadomość i wymaga correlation=\"…\"."),
                    Some(format!(
                        "'{id}' waits for a message and needs correlation=\"…\""
                    )),
                ));
            }
            if kind == "xor" || kind == "or" {
                let fl = outs.get(id).cloned().unwrap_or_default();
                if fl.len() > 1 {
                    for f in fl {
                        if !f.base.attrs.contains_key("when") && !f.base.has("default") {
                            found.push(Diag::text(
                                "C070",
                                f.base.line,
                                f.base.id.as_deref(),
                                Severity::Error,
                                format!(
                                    "Przepływ {} -> {} z bramki „{id}” wymaga when=\"…\" albo flagi default.",
                                    f.source, f.target
                                ),
                                Some(format!(
                                    "flow {} -> {} leaving gateway '{id}' needs when=\"…\" or the default flag",
                                    f.source, f.target
                                )),
                            ));
                        }
                    }
                }
            }
        }
        self.diags.extend(found);
    }

    fn node_executable(&self, n: &Node) -> bool {
        let d = &*self.doc;
        if n.choreography.is_some() {
            // a choreography is not a process (C122 reports it)
            return false;
        }
        d.executable()
            || d.pools
                .iter()
                .any(|p| p.base.id.as_deref() == n.pool.as_deref() && p.base.has("executable"))
    }

    fn process_ids(&self) -> Vec<String> {
        let d = &*self.doc;
        let mut ids = vec![d.default_process_id()];
        ids.extend(d.pools.iter().map(Pool::process_id));
        ids
    }

    /// Ad-hoc blocks, compensation handlers and the executable layer (spec §5.3, §10.3).
    fn layer_03(&mut self) {
        let mut incoming: std::collections::HashSet<String> = Default::default();
        let mut outgoing: std::collections::HashSet<String> = Default::default();
        for f in &self.doc.flows {
            outgoing.insert(f.source.clone());
            incoming.insert(f.target.clone());
        }
        let process_ids = self.process_ids();
        let node_ids: Vec<String> = self.doc.nodes.keys().cloned().collect();
        for nid in node_ids {
            let n = self.doc.nodes[&nid].clone();
            let a = &n.base.attrs;
            let kind = n.base.kind.as_str();
            let line = n.base.line;
            let id = Some(nid.as_str());
            if kind == "adhoc" {
                let inner: Vec<&Node> = n
                    .children
                    .iter()
                    .filter_map(|c| self.doc.nodes.get(c))
                    .collect();
                if !inner.iter().any(|x| is_activity(&x.base.kind))
                    || inner
                        .iter()
                        .any(|x| x.base.kind == "start" || x.base.kind == "end")
                {
                    self.add("R050", line, id, &[("id", nid.clone())]);
                }
            }
            if n.base.has("compensation") && (incoming.contains(&nid) || outgoing.contains(&nid)) {
                self.add(
                    "R051",
                    line,
                    id,
                    &[
                        (
                            "detail",
                            format!("compensation handler '{nid}' must not have sequence flows"),
                        ),
                        (
                            "detail_pl",
                            format!(
                                "procedura kompensacji „{nid}” nie może mieć przepływów sekwencji"
                            ),
                        ),
                    ],
                );
            }
            if a.contains_key("events")
                && kind == "start"
                && n.scope.as_ref().is_some_and(|sc| process_ids.contains(sc))
            {
                self.add("R052", line, id, &[("id", nid.clone())]);
            }
            if a.contains_key("collection") && !(n.base.has("parallel") || n.base.has("sequential"))
            {
                self.add(
                    "R054",
                    line,
                    id,
                    &[
                        (
                            "detail",
                            format!("collection= on '{nid}' needs the flag parallel or sequential"),
                        ),
                        (
                            "detail_pl",
                            format!("collection= w „{nid}” wymaga flagi parallel albo sequential"),
                        ),
                    ],
                );
            }
            if a.contains_key("output") != a.contains_key("output-item") {
                self.add(
                    "R054",
                    line,
                    id,
                    &[
                        (
                            "detail",
                            format!("'{nid}' needs both output= and output-item="),
                        ),
                        (
                            "detail_pl",
                            format!("„{nid}” wymaga jednocześnie output= i output-item="),
                        ),
                    ],
                );
            }
            for key in ["inputs", "outputs"] {
                if let Some(text) = a.get(key) {
                    for (left, right) in split_pairs(text, '=') {
                        let bad = match &right {
                            None => true,
                            Some(r) => !is_target(&left) || r.is_empty(),
                        };
                        if bad {
                            let item = match &right {
                                None => left.clone(),
                                Some(r) => format!("{left} = {r}"),
                            };
                            self.add(
                                "R053",
                                line,
                                id,
                                &[
                                    ("key", s(key)),
                                    ("item", item),
                                    ("expected", s("target = expression")),
                                    ("expected_pl", s("cel = wyrażenie")),
                                ],
                            );
                        }
                    }
                }
            }
            if let Some(text) = a.get("headers") {
                for (left, right) in split_pairs(text, ':') {
                    if right.is_none() || left.is_empty() {
                        self.add(
                            "R053",
                            line,
                            id,
                            &[
                                ("key", s("headers")),
                                ("item", left),
                                ("expected", s("key: value")),
                                ("expected_pl", s("klucz: wartość")),
                            ],
                        );
                    }
                }
            }
            if let Some(text) = a.get("listeners") {
                let mut allowed: Vec<&str> = EXECUTION_EVENTS.to_vec();
                if kind == "user" {
                    allowed.extend(TASK_EVENTS);
                }
                for item in text.split(';') {
                    let item_s = crate::pyutil::strip(item);
                    if item_s.is_empty() {
                        continue;
                    }
                    let ok = parse_listener(item)
                        .is_some_and(|(ev, _, _)| allowed.contains(&ev.as_str()));
                    if !ok {
                        let joined = allowed.join(", ");
                        self.add(
                            "R053",
                            line,
                            id,
                            &[
                                ("key", s("listeners")),
                                ("item", item_s.to_string()),
                                (
                                    "expected",
                                    format!("event: type with event one of {joined}"),
                                ),
                                (
                                    "expected_pl",
                                    format!("zdarzenie: typ, gdzie zdarzenie to {joined}"),
                                ),
                            ],
                        );
                    }
                }
            }
            for key in USER_TASK_KEYS {
                if a.contains_key(*key) && kind != "user" {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[("key", s(key)), ("requirement", s("a user task"))],
                    );
                }
            }
            if let Some(conn) = a.get("connector") {
                if kind != "service" {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[
                            ("key", s("connector")),
                            ("requirement", s("a service task")),
                        ],
                    );
                } else if conn != "rest" {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[
                            ("key", s("connector")),
                            ("requirement", s("the value rest")),
                        ],
                    );
                } else if !a.contains_key("url") || !a.contains_key("method") {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[
                            ("key", s("connector=rest")),
                            ("requirement", s("method= and url=")),
                        ],
                    );
                }
            }
            for key in CONNECTOR_KEYS {
                if a.contains_key(*key)
                    && !a.contains_key("connector")
                    && !(kind == "transaction" && *key == "method")
                {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[("key", s(key)), ("requirement", s("connector=rest"))],
                    );
                }
            }
            for key in ["active", "wait"] {
                if (a.contains_key(key) || n.base.has(key)) && kind != "adhoc" {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[("key", s(key)), ("requirement", s("an ad-hoc sub-process"))],
                    );
                }
            }
            if !self.node_executable(&n) {
                for key in EXEC_KEYS {
                    if a.contains_key(*key) && is_exec_key(kind, key) {
                        self.add("R055", line, id, &[("key", s(key))]);
                    }
                }
                for f in EXEC_FLAGS {
                    if n.base.has(f) {
                        self.add("R055", line, id, &[("key", s(f))]);
                    }
                }
                if is_event(kind) && a.contains_key("type") {
                    self.add("R055", line, id, &[("key", s("type"))]);
                }
            } else {
                self.executable_03(&n);
            }
        }
        let d = &*self.doc;
        let mut found = Vec::new();
        if d.attrs.contains_key("version-tag") && !d.executable() {
            found.push(Diag::new(
                "R055",
                d.header_line,
                None,
                &[("key", s("version-tag"))],
            ));
        }
        for p in &d.pools {
            if p.base.attrs.contains_key("version-tag")
                && !(p.base.has("executable") || d.executable())
            {
                found.push(Diag::new(
                    "R055",
                    p.base.line,
                    p.base.id.as_deref(),
                    &[("key", s("version-tag"))],
                ));
            }
        }
        self.diags.extend(found);
    }

    fn executable_03(&mut self, n: &Node) {
        let a = &n.base.attrs;
        let kind = n.base.kind.as_str();
        let line = n.base.line;
        let nid = n.base.id().to_string();
        if (kind == "throw" || kind == "end")
            && (a.contains_key("message") || n.base.has("message"))
            && !a.contains_key("type")
        {
            let kind_pl = if kind == "throw" {
                "rzucające"
            } else {
                "końcowe"
            };
            self.add(
                "C022",
                line,
                Some(&nid),
                &[
                    ("kind", s(kind)),
                    ("id", nid.clone()),
                    ("kind_pl", s(kind_pl)),
                ],
            );
        }
        for key in ["due", "follow-up"] {
            if let Some(v) = a.get(key)
                && !v.starts_with('=')
                && !is_iso_zoned(v)
            {
                self.add(
                    "C090",
                    line,
                    Some(&nid),
                    &[("key", s(key)), ("value", v.clone())],
                );
            }
        }
        if let Some(v) = a.get("priority")
            && !v.starts_with('=')
        {
            let ok = !v.is_empty()
                && v.bytes().all(|b| b.is_ascii_digit())
                && v.parse::<u64>().map(|x| x <= 100).unwrap_or(false);
            if !ok {
                self.add("C091", line, Some(&nid), &[("value", v.clone())]);
            }
        }
    }

    /// `R066` (undeclared) or `R003` (wrong kind) for a reference to a declaration of
    /// language 0.5; true when the reference is good.
    fn declared(
        &mut self,
        target: &str,
        line: usize,
        owner: Option<&str>,
        kinds: &[&str],
        keyword: &str,
        what: (&str, &str),
    ) -> bool {
        match self.doc.ids.get(target).map(|e| e.kind.clone()) {
            None => {
                self.add(
                    "R066",
                    line,
                    owner,
                    &[
                        ("what", s(what.0)),
                        ("what_pl", s(what.1)),
                        ("id", s(target)),
                        ("keyword", s(keyword)),
                    ],
                );
                false
            }
            Some(k) if !kinds.contains(&k.as_str()) => {
                self.add(
                    "R003",
                    line,
                    Some(target),
                    &[
                        ("id", s(target)),
                        ("actual", k),
                        ("expected", s(what.0)),
                        ("expected_pl", s(what.1)),
                    ],
                );
                false
            }
            Some(_) => true,
        }
    }

    fn detail(&mut self, code: &str, line: usize, id: Option<&str>, en: String, pl: String) {
        self.add(code, line, id, &[("detail", en), ("detail_pl", pl)]);
    }

    /// Elements of language 0.5 (BPMN 2.0.2 beyond Camunda 8): R060–R066 and, in executable
    /// documents for Camunda, C100–C110.
    fn layer_05(&mut self) {
        let incoming: std::collections::HashSet<String> =
            self.doc.flows.iter().map(|f| f.target.clone()).collect();
        let process_ids = self.process_ids();
        let lang_05 = self.doc.lang_05();
        let camunda = self.doc.attrs.get("target").is_none_or(|t| t == "camunda");
        let node_ids: Vec<String> = self.doc.nodes.keys().cloned().collect();
        for nid in node_ids {
            let n = self.doc.nodes[&nid].clone();
            let b = &n.base;
            let kind = b.kind.as_str();
            let line = b.line;
            let id = Some(nid.as_str());
            let kind_of = |x: Option<&str>| -> Option<String> {
                x.and_then(|i| self.doc.nodes.get(i))
                    .map(|m| m.base.kind.clone())
            };
            let scope_kind = kind_of(n.scope.as_deref());
            let on_kind = kind_of(b.attr("on"));
            if b.has("cancel") {
                match kind {
                    "end" => {
                        if scope_kind.as_deref() != Some("transaction") {
                            self.detail(
                                "R060",
                                line,
                                id,
                                format!("cancel end event '{nid}' must be inside a transaction"),
                                format!(
                                    "zdarzenie końcowe anulowania „{nid}” musi leżeć wewnątrz transakcji"
                                ),
                            );
                        }
                    }
                    "boundary" => {
                        if b.attr("on").is_some() && on_kind.as_deref() != Some("transaction") {
                            self.detail(
                                "R060",
                                line,
                                id,
                                format!(
                                    "cancel boundary event '{nid}' must be attached to a transaction"
                                ),
                                format!(
                                    "zdarzenie graniczne anulowania „{nid}” musi być przypięte do transakcji"
                                ),
                            );
                        }
                        if b.has("noninterrupting") {
                            self.detail(
                                "R060",
                                line,
                                id,
                                format!("cancel boundary event '{nid}' is always interrupting"),
                                format!(
                                    "zdarzenie graniczne anulowania „{nid}” zawsze przerywa czynność"
                                ),
                            );
                        }
                    }
                    _ => self.detail(
                        "R060",
                        line,
                        id,
                        format!(
                            "'{nid}': cancel is allowed only on end events inside a transaction and on boundary events of a transaction"
                        ),
                        format!(
                            "„{nid}”: cancel może wystąpić tylko w zdarzeniu końcowym wewnątrz transakcji i w zdarzeniu granicznym transakcji"
                        ),
                    ),
                }
            }
            if kind == "transaction"
                && let Some(m) = b.attr("method")
                && !TRANSACTION_METHODS.iter().any(|(k, _)| *k == m)
            {
                self.detail(
                    "R060",
                    line,
                    id,
                    format!("method=\"{m}\" of transaction '{nid}' must be compensate, store or image"),
                    format!(
                        "method=\"{m}\" transakcji „{nid}” musi mieć wartość compensate, store albo image"
                    ),
                );
            }
            let loop_parts: Vec<&str> = ["while", "loop-max"]
                .into_iter()
                .filter(|k| b.attrs.contains_key(*k))
                .chain(b.has("test-before").then_some("test-before"))
                .collect();
            if !loop_parts.is_empty() && !b.has("loop") {
                let parts = loop_parts.join(", ");
                self.detail(
                    "R061",
                    line,
                    id,
                    format!("{parts} on '{nid}' requires the flag loop"),
                    format!("{parts} w „{nid}” wymaga flagi loop"),
                );
            }
            if let Some(v) = b.attr("loop-max")
                && (v.is_empty() || !v.bytes().all(|c| c.is_ascii_digit()))
            {
                self.detail(
                    "R061",
                    line,
                    id,
                    format!("loop-max of '{nid}' must be a whole number, not '{v}'"),
                    format!("loop-max w „{nid}” musi być liczbą całkowitą, a nie „{v}”"),
                );
            }
            if is_event(kind) {
                let mut trig: Vec<(&str, bool)> = Vec::new();
                for k in TRIGGERS {
                    for _ in b.values(k) {
                        trig.push((k, false));
                    }
                }
                for f in TRIGGER_FLAGS
                    .iter()
                    .chain(CATCH_ALL_FLAGS)
                    .chain(NAMELESS_TRIGGERS)
                {
                    if b.has(f) {
                        trig.push((f, true));
                    }
                }
                let alone: Vec<&str> = trig
                    .iter()
                    .filter(|(t, flag)| {
                        SINGLE_TRIGGERS.contains(t) || (*flag && CATCH_ALL_FLAGS.contains(t))
                    })
                    .map(|(t, _)| *t)
                    .collect();
                if lang_05 && trig.len() > 1 && !alone.is_empty() {
                    let names = alone.join(", ");
                    self.detail(
                        "R062",
                        line,
                        id,
                        format!("'{nid}': {names} cannot be combined with other triggers"),
                        format!("„{nid}”: {names} nie może łączyć się z innymi wyzwalaczami"),
                    );
                }
                if b.has("parallel") {
                    if !matches!(kind, "start" | "catch" | "boundary") {
                        self.detail(
                            "R062",
                            line,
                            id,
                            format!("parallel on '{nid}' requires a catching event (start, catch, boundary)"),
                            format!(
                                "parallel w „{nid}” wymaga zdarzenia przechwytującego (start, catch, boundary)"
                            ),
                        );
                    } else if trig.len() < 2 {
                        self.detail(
                            "R062",
                            line,
                            id,
                            format!("parallel on '{nid}' requires two or more triggers"),
                            format!("parallel w „{nid}” wymaga co najmniej dwóch wyzwalaczy"),
                        );
                    }
                }
            } else if let Some((k, _)) = b.extra.first() {
                self.detail(
                    "R062",
                    line,
                    id,
                    format!("'{k}' is given more than once on '{nid}'; only events have several triggers"),
                    format!(
                        "„{k}” podano kilka razy w „{nid}”; kilka wyzwalaczy może mieć tylko zdarzenie"
                    ),
                );
            }
            if b.has("instantiate") {
                if kind == "receive" || kind == "eventgw" {
                    if incoming.contains(&nid) {
                        self.detail(
                            "R063",
                            line,
                            id,
                            format!("instantiating '{nid}' cannot have incoming sequence flows"),
                            format!(
                                "„{nid}” rozpoczynające proces nie może mieć przychodzących przepływów sekwencji"
                            ),
                        );
                    }
                    if !n.scope.as_ref().is_some_and(|sc| process_ids.contains(sc)) {
                        self.detail(
                            "R063",
                            line,
                            id,
                            format!("instantiating '{nid}' must be in a process, not in a sub-process"),
                            format!(
                                "„{nid}” rozpoczynające proces musi leżeć w procesie, a nie w podprocesie"
                            ),
                        );
                    }
                } else {
                    self.detail(
                        "R063",
                        line,
                        id,
                        format!(
                            "instantiate on '{nid}' is allowed only on receive tasks and event-based gateways"
                        ),
                        format!(
                            "instantiate w „{nid}” może wystąpić tylko w zadaniu odbioru i w bramce opartej na zdarzeniach"
                        ),
                    );
                }
            }
            if kind == "eventgw" && b.has("parallel") && !b.has("instantiate") {
                self.detail(
                    "R063",
                    line,
                    id,
                    format!("parallel event-based gateway '{nid}' requires the flag instantiate"),
                    format!(
                        "równoległa bramka oparta na zdarzeniach „{nid}” wymaga flagi instantiate"
                    ),
                );
            }
            if let Some(t) = b.attr("task") {
                if kind != "call" {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[("key", s("task")), ("requirement", s("a call activity"))],
                    );
                } else if b.attrs.contains_key("process") {
                    self.detail(
                        "R065",
                        line,
                        id,
                        format!("call activity '{nid}' has both process= and task=; it calls one of them"),
                        format!(
                            "czynność wywołania „{nid}” ma jednocześnie process= i task=; wywołuje jedno z nich"
                        ),
                    );
                }
                self.declared(
                    t,
                    line,
                    id,
                    &["global"],
                    "global",
                    ("global task", "zadanie globalne"),
                );
            }
            if let Some(o) = b.attr("operation") {
                if !matches!(kind, "service" | "send" | "receive") {
                    self.add(
                        "R013",
                        line,
                        id,
                        &[
                            ("key", s("operation")),
                            ("requirement", s("a service, send or receive task")),
                        ],
                    );
                }
                self.declared(
                    o,
                    line,
                    id,
                    &["operation"],
                    "operation",
                    ("operation", "operacja"),
                );
            }
            if camunda && self.node_executable(&n) {
                self.camunda_05(&n);
            }
        }
        self.data_05();
        self.declarations_05(camunda);
    }

    /// C100–C109: elements of a node that Camunda 8.10 does not execute.
    fn camunda_05(&mut self, n: &Node) {
        let b = &n.base;
        let kind = b.kind.as_str();
        let line = b.line;
        let nid = b.id().to_string();
        let id = Some(nid.as_str());
        let arg = || vec![("id", nid.clone())];
        if kind == "transaction" {
            self.add("C100", line, id, &arg());
        }
        if b.has("cancel") {
            self.add("C101", line, id, &arg());
        }
        if kind == "complex" {
            self.add("C102", line, id, &arg());
        }
        if b.has("loop")
            && (b.attrs.contains_key("while")
                || b.attrs.contains_key("loop-max")
                || b.has("test-before"))
        {
            self.add("C103", line, id, &arg());
        }
        if is_event(kind) {
            let count = TRIGGERS.iter().map(|k| b.values(k).len()).sum::<usize>()
                + TRIGGER_FLAGS
                    .iter()
                    .chain(CATCH_ALL_FLAGS)
                    .chain(NAMELESS_TRIGGERS)
                    .filter(|f| b.has(f))
                    .count();
            if count > 1 {
                self.add(
                    if b.has("parallel") { "C105" } else { "C104" },
                    line,
                    id,
                    &arg(),
                );
            }
        }
        if b.has("instantiate") && kind == "receive" {
            self.add("C106", line, id, &arg());
        }
        if b.has("instantiate") && kind == "eventgw" {
            self.add("C107", line, id, &arg());
        }
        if let Some(t) = b.attr("task") {
            self.add("C108", line, id, &[("id", nid.clone()), ("task", s(t))]);
        }
        if let Some(o) = b.attr("operation") {
            self.add(
                "C109",
                line,
                id,
                &[("id", nid.clone()), ("operation", s(o))],
            );
        }
    }

    /// Process data: `type=`, `on=` and the state transitions of data objects (R064).
    fn data_05(&mut self) {
        // (identifier, kind, type=, on=, line)
        type DataRef = (String, String, Option<String>, Option<String>, usize);
        let items: Vec<DataRef> = self
            .doc
            .data
            .values()
            .map(|x| {
                (
                    x.base.id().to_string(),
                    x.base.kind.clone(),
                    x.base.attrs.get("type").cloned(),
                    x.base.attrs.get("on").cloned(),
                    x.base.line,
                )
            })
            .collect();
        for (did, kind, typ, on, line) in items {
            if let Some(t) = typ {
                self.declared(
                    &t,
                    line,
                    Some(&did),
                    &["item"],
                    "item",
                    ("item definition", "definicja elementu danych"),
                );
            }
            if let Some(on) = on {
                if !IO_KINDS.contains(&kind.as_str()) {
                    continue;
                }
                // inputs of activities and throwing events, outputs of activities and
                // catching events, properties of activities and events
                let ok = |k: &str| match kind.as_str() {
                    "input" => is_activity(k) || THROWING_EVENTS.contains(&k),
                    "output" => is_activity(k) || CATCHING_EVENTS.contains(&k),
                    _ => is_activity(k) || is_event(k),
                };
                self.reference(&on, line, "io owner", &ok);
            }
        }
        // a state transition reads a data object in one state and writes it in another
        let d = &*self.doc;
        let object = |id: &str| -> Option<(String, String)> {
            let x = d.data.get(id)?;
            if x.base.kind != "data" {
                return None;
            }
            let state = x.base.attrs.get("state")?.clone();
            Some((
                x.base.label.clone().unwrap_or_else(|| id.to_string()),
                state,
            ))
        };
        let mut found = Vec::new();
        for r in d.assocs.iter().filter(|a| a.base.kind == "reads") {
            let Some((name, state)) = object(&r.data) else {
                continue;
            };
            let same = d.assocs.iter().any(|w| {
                w.base.kind == "writes"
                    && w.activity == r.activity
                    && object(&w.data).is_some_and(|(n2, s2)| n2 == name && s2 == state)
            });
            if same {
                found.push(Diag::new(
                    "R064",
                    r.base.line,
                    Some(&r.activity),
                    &[
                        ("activity", r.activity.clone()),
                        ("data", name),
                        ("state", state),
                    ],
                ));
            }
        }
        self.diags.extend(found);
    }

    /// Items, global tasks, interfaces and correlation keys.
    fn declarations_05(&mut self, camunda: bool) {
        let mut found = Vec::new();
        let d = &*self.doc;
        for it in d.items.values() {
            if let Some(k) = it.attr("kind")
                && !ITEM_KINDS.contains(&k)
            {
                found.push(Diag::new(
                    "R013",
                    it.line,
                    it.id.as_deref(),
                    &[
                        ("key", s("kind")),
                        ("requirement", s("information or physical")),
                    ],
                ));
            }
            // structureRef is a QName: a name without a prefix, an XML Schema type, or a
            // prefixed name whose namespace `namespace=` gives
            let ns = it.attr("namespace");
            let qname_ok = |v: &str| match v.split_once(':') {
                None => is_xml_name(v),
                Some((p, l)) => {
                    is_xml_name(p)
                        && is_xml_name(l)
                        && ((p == "xsd" && ns.is_none())
                            || (ns.is_some() && !RESERVED_PREFIXES.contains(&p)))
                }
            };
            if let Some(n) = ns
                && (n.is_empty()
                    || !it
                        .attr("structure")
                        .is_some_and(|v| v.contains(':') && !v.starts_with("xsd:")))
            {
                found.push(Diag::new(
                    "R013",
                    it.line,
                    it.id.as_deref(),
                    &[
                        ("key", s("namespace")),
                        (
                            "requirement",
                            s("a structure name with a namespace prefix (structure=\"feel:string\")"),
                        ),
                    ],
                ));
            }
            if let Some(v) = it.attr("structure")
                && !qname_ok(v)
            {
                found.push(Diag::new(
                    "R013",
                    it.line,
                    it.id.as_deref(),
                    &[
                        ("key", s("structure")),
                        (
                            "requirement",
                            s("a type name (Wniosek), an XML Schema type (xsd:string) or a prefixed name with namespace="),
                        ),
                    ],
                ));
            }
        }
        // one namespace per prefix in the document
        let mut prefixes: Vec<(&str, &str)> = Vec::new();
        for it in d.items.values() {
            let (Some(v), Some(n)) = (it.attr("structure"), it.attr("namespace")) else {
                continue;
            };
            let Some((p, _)) = v.split_once(':') else {
                continue;
            };
            match prefixes.iter().find(|(q, _)| *q == p) {
                Some((_, m)) if *m != n => found.push(Diag::new(
                    "R013",
                    it.line,
                    it.id.as_deref(),
                    &[
                        ("key", s("namespace")),
                        (
                            "requirement",
                            format!("the namespace of prefix '{p}' given elsewhere ({m})"),
                        ),
                    ],
                )),
                Some(_) => {}
                None => prefixes.push((p, n)),
            }
        }
        for g in d.globals.values() {
            if let Some(k) = g.attr("kind")
                && !GLOBAL_KINDS.iter().any(|(x, _)| *x == k)
            {
                found.push(Diag::new(
                    "R065",
                    g.line,
                    g.id.as_deref(),
                    &[
                        (
                            "detail",
                            format!("kind=\"{k}\" of global task '{}' must be task, user, manual, script or rule", g.id()),
                        ),
                        (
                            "detail_pl",
                            format!("kind=\"{k}\" zadania globalnego „{}” musi mieć wartość task, user, manual, script albo rule", g.id()),
                        ),
                    ],
                ));
            }
        }
        for x in d.interfaces.values() {
            for op in &x.operations {
                if !op.attrs.contains_key("in") {
                    found.push(Diag::new(
                        "R013",
                        op.line,
                        op.id.as_deref(),
                        &[
                            ("key", s("operation")),
                            ("requirement", s("in=\"Message\"")),
                        ],
                    ));
                }
            }
        }
        let executable = d.executable_any();
        for k in d.keys.values() {
            if d.pools.is_empty() {
                found.push(Diag::new(
                    "R013",
                    k.base.line,
                    k.base.id.as_deref(),
                    &[
                        ("key", s("key")),
                        (
                            "requirement",
                            s("pools (BPMN keeps correlation keys in a collaboration)"),
                        ),
                    ],
                ));
            }
            for p in &k.props {
                if !p.attrs.contains_key("message") || !p.attrs.contains_key("path") {
                    found.push(Diag::new(
                        "R013",
                        p.line,
                        None,
                        &[("key", s("prop")), ("requirement", s("message= and path="))],
                    ));
                }
            }
            if executable && camunda {
                found.push(Diag::new(
                    "C110",
                    k.base.line,
                    k.base.id.as_deref(),
                    &[("id", s(k.base.id()))],
                ));
            }
        }
        self.diags.extend(found);
    }

    fn decisions_and_forms(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        // language 0.7: decision tables are checked with the other boxes (`layer_07`)
        let old_tables = if d.lang_07() { 0 } else { d.decisions.len() };
        for dec in d.decisions.values().take(old_tables) {
            let id = dec.base.id();
            let line = dec.base.line;
            if !HIT_POLICIES.contains(&dec.hit()) {
                found.push(Diag::new(
                    "D003",
                    line,
                    Some(id),
                    &[("hit", dec.hit().to_string())],
                ));
            }
            if dec.inputs.is_empty() && dec.outputs.is_empty() {
                found.push(Diag::new("D001", line, Some(id), &[("id", s(id))]));
            } else if dec.rules.is_empty() {
                found.push(Diag::new("D005", line, Some(id), &[("id", s(id))]));
            }
            for (code, l, args) in dmn::check_decision(dec) {
                if !["D001", "D003", "D005"].contains(&code) {
                    found.push(Diag::new(code, l, Some(id), &args));
                }
            }
        }
        for f in d.forms.values() {
            for it in &f.items {
                if it.base.kind != "field" {
                    continue;
                }
                let t = it
                    .base
                    .attrs
                    .get("type")
                    .cloned()
                    .unwrap_or_else(|| "textfield".into());
                if !FIELD_TYPES.contains(&t.as_str()) {
                    found.push(Diag::new(
                        "M001",
                        it.base.line,
                        it.base.id.as_deref(),
                        &[("type", t)],
                    ));
                } else if (t == "select" || t == "radio") && !it.base.attrs.contains_key("options")
                {
                    found.push(Diag::new(
                        "M002",
                        it.base.line,
                        it.base.id.as_deref(),
                        &[("id", it.base.id().to_string()), ("type", t)],
                    ));
                }
            }
        }
        self.diags.extend(found);
    }

    fn feel(&mut self) {
        let mut exprs: Vec<(String, usize, Option<String>)> = Vec::new();
        for n in self.doc.nodes.values() {
            let a = &n.base.attrs;
            for k in [
                "expr",
                "condition",
                "correlation",
                "collection",
                "output-item",
                "until",
                "active",
                "while",
                "activation",
            ] {
                if let Some(t) = a.get(k) {
                    exprs.push((t.clone(), n.base.line, n.base.id.clone()));
                }
            }
            for k in ["inputs", "outputs"] {
                for (_, right) in split_pairs(a.get(k).map(String::as_str).unwrap_or(""), '=') {
                    if let Some(r) = right.filter(|r| !r.is_empty()) {
                        exprs.push((r, n.base.line, n.base.id.clone()));
                    }
                }
            }
            for k in [
                "assignee",
                "candidate-users",
                "candidate-groups",
                "due",
                "follow-up",
                "priority",
                "process",
                "type",
                "timer",
                "business-id",
                "url",
                "body",
                "query",
                "headers-in",
            ] {
                if let Some(v) = a.get(k)
                    && let Some(rest) = v.strip_prefix('=')
                {
                    let rest = crate::pyutil::strip(rest);
                    if !rest.is_empty() {
                        exprs.push((rest.to_string(), n.base.line, n.base.id.clone()));
                    }
                }
            }
        }
        // in a document of language 0.5, `when=""` is a condition without an expression
        let lang_05 = self.doc.lang_05();
        for f in &self.doc.flows {
            if let Some(t) = f.base.attrs.get("when")
                && !(lang_05 && t.trim().is_empty())
            {
                exprs.push((t.clone(), f.base.line, f.base.id.clone()));
            }
        }
        let names07 = self.doc.lang_07().then(|| crate::drg::names(self.doc));
        for (text, line, id) in exprs {
            let found = match &names07 {
                Some(names) => crate::feel3::parse_expression(&text, names)
                    .err()
                    .map(|e| (e.code(), e.detail().to_string())),
                None => feel::check_expr(&text),
            };
            if let Some((code, detail)) = found {
                let pl = feel::detail_pl(&detail);
                self.add(
                    code,
                    line,
                    id.as_deref(),
                    &[("detail", detail), ("detail_pl", pl)],
                );
            }
        }
    }

    fn simulation(&mut self) {
        for (code, line, id, mut args) in simcheck::check_sim(self.doc) {
            if code == "R002" {
                if self
                    .diags
                    .iter()
                    .any(|x| x.code == "R002" && x.line == line)
                {
                    continue;
                }
                let target = args
                    .iter()
                    .find(|a| a.0 == "id")
                    .map(|a| a.1.clone())
                    .unwrap_or_default();
                args = vec![
                    ("what", s("resource")),
                    ("id", target),
                    ("what_pl", s("zasób")),
                ];
            }
            self.add(code, line, id.as_deref(), &args);
        }
    }

    fn levels(&mut self) {
        if self.level == "full" {
            return;
        }
        let level = self.level;
        let mut found = Vec::new();
        self.levels_06(level, &mut found);
        let d = &*self.doc;
        for n in d.nodes.values().filter(|n| n.choreography.is_none()) {
            let b = &n.base;
            let exec_layer = EXEC_KEYS
                .iter()
                .any(|k| b.attrs.contains_key(*k) && is_exec_key(&b.kind, k))
                || EXEC_FLAGS.iter().any(|f| b.has(f))
                || (is_event(&b.kind) && b.attrs.contains_key("type"));
            let lang_05 = ["transaction", "complex"].contains(&b.kind.as_str())
                || ["cancel", "test-before", "instantiate"]
                    .iter()
                    .any(|f| b.has(f))
                || ((is_event(&b.kind) || b.kind == "eventgw") && b.has("parallel"))
                || ["while", "loop-max", "activation", "task", "operation"]
                    .iter()
                    .any(|k| b.attrs.contains_key(*k))
                || !b.extra.is_empty();
            let bad = lang_05
                || if level == "basic" {
                    !BASIC_KINDS.contains(&b.kind.as_str())
                        || TRIGGERS.iter().any(|k| b.attrs.contains_key(*k))
                        || TRIGGER_FLAGS
                            .iter()
                            .chain(CATCH_ALL_FLAGS)
                            .chain(NAMELESS_TRIGGERS)
                            .chain(TASK_FLAGS.iter())
                            .any(|f| b.has(f))
                        || b.attrs.contains_key("compensate")
                        || exec_layer
                } else {
                    b.attrs.keys().any(|k| analytic_excluded(&b.kind, k)) || exec_layer
                };
            if bad {
                found.push(Diag::new(
                    "R040",
                    b.line,
                    b.id.as_deref(),
                    &[("keyword", b.kind.clone()), ("level", s(level))],
                ));
            }
        }
        // process data, declarations and correlation of language 0.5 (level full only)
        let decl_05 = d
            .data
            .values()
            .map(|x| &x.base)
            .filter(|b| IO_KINDS.contains(&b.kind.as_str()) || b.attrs.contains_key("type"))
            .chain(d.items.values())
            .chain(d.globals.values())
            .chain(d.interfaces.values().map(|x| &x.base))
            .chain(d.keys.values().map(|x| &x.base));
        for b in decl_05 {
            found.push(Diag::new(
                "R040",
                b.line,
                b.id.as_deref(),
                &[("keyword", b.kind.clone()), ("level", s(level))],
            ));
        }
        let drg_first = || {
            let mut v: Vec<&Base> = d
                .decisions
                .values()
                .map(|x| &x.base)
                .chain(d.bkms.values().map(|x| &x.base))
                .chain(d.inputdata.values())
                .chain(d.sources.values())
                .collect();
            v.sort_by_key(|b| b.line);
            v
        };
        let has_drg = !d.decisions.is_empty()
            || !d.bkms.is_empty()
            || !d.inputdata.is_empty()
            || !d.sources.is_empty();
        if level == "basic" && (has_drg || !d.forms.is_empty() || d.simulate.is_some()) {
            let el = drg_first()
                .into_iter()
                .chain(d.forms.values().map(|f| &f.base))
                .chain(d.simulate.iter())
                .next();
            let (line, kw) = el.map(|e| (e.line, e.kind.clone())).unwrap_or((0, s("?")));
            found.push(Diag::new(
                "R040",
                line,
                None,
                &[("keyword", kw), ("level", s(level))],
            ));
        }
        for p in &d.pools {
            if p.base.attrs.contains_key("version-tag") {
                found.push(Diag::new(
                    "R040",
                    p.base.line,
                    p.base.id.as_deref(),
                    &[("keyword", s("version-tag")), ("level", s(level))],
                ));
            }
        }
        if d.attrs.contains_key("version-tag") {
            found.push(Diag::new(
                "R040",
                d.header_line,
                None,
                &[("keyword", s("version-tag")), ("level", s(level))],
            ));
        }
        if level == "analytic" && (has_drg || !d.forms.is_empty() || d.executable_any()) {
            let el = drg_first()
                .into_iter()
                .chain(d.forms.values().map(|f| &f.base))
                .next();
            let (line, kw) = el
                .map(|e| (e.line, e.kind.clone()))
                .unwrap_or((d.header_line, s("executable")));
            found.push(Diag::new(
                "R040",
                line,
                None,
                &[("keyword", kw), ("level", s(level))],
            ));
        }
        self.diags.extend(found);
    }

    fn bpmn_rules(&mut self, rules: &dyn BpmnRules) {
        let xml = crate::export::export_xml(self.doc, None);
        for issue in rules.validate(&xml) {
            if issue.code.starts_with("D0") || issue.code.starts_with('K') {
                continue;
            }
            let sev = match issue.level.as_str() {
                "ostrzeżenie" => Severity::Warning,
                "uwaga" => Severity::Note,
                _ => Severity::Error,
            };
            let line = issue
                .id
                .as_ref()
                .and_then(|i| self.doc.ids.get(i))
                .map(|e| e.line)
                .unwrap_or(0);
            self.diags.push(Diag::text(
                &issue.code,
                line,
                issue.id.as_deref(),
                sev,
                issue.message.clone(),
                None,
            ));
        }
    }
}
