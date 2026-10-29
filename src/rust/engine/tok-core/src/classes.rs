//! BPMN 2.0.2 process modelling conformance sub-classes (clause 2, Tables 2.1–2.4):
//! which class every element of a document belongs to, `tok check --class` (R075) and the
//! smallest class a document fits (`"bpmn_class"` of `tok check --json`).
//!
//! The Descriptive class is contained in the Analytic class; the Common Executable class
//! is a separate set (it lacks pools, lanes and message flows, and has data inputs,
//! outputs, item definitions and operations). `full` is the whole modelling language:
//! full Process Modeling Conformance with conversations, and choreographies (Choreography
//! Modeling Conformance).

use crate::diagnostics::Diag;
use crate::model::*;

/// The classes in the order in which the smallest one is chosen.
pub const CLASSES: &[&str] = &["descriptive", "analytic", "common-executable", "full"];

const D: u8 = 1;
const A: u8 = 2;
const X: u8 = 4;
const DA: u8 = D | A;
const AX: u8 = A | X;
const DAX: u8 = D | A | X;
/// Only in the full class.
const F: u8 = 0;

fn bit(class: &str) -> Option<u8> {
    match class {
        "descriptive" => Some(D),
        "analytic" => Some(A),
        "common-executable" => Some(X),
        _ => None,
    }
}

/// One element of the document with the sub-classes that contain it.
#[derive(Debug, Clone)]
pub struct Item {
    pub line: usize,
    pub id: String,
    /// BPMN element (or attribute) the tok statement exports.
    pub what: String,
    pub mask: u8,
}

impl Item {
    /// The first class (in [`CLASSES`] order) that contains the element.
    pub fn needed(&self) -> &'static str {
        CLASSES
            .iter()
            .copied()
            .find(|c| bit(c).is_none_or(|b| self.mask & b != 0))
            .unwrap_or("full")
    }
    fn fits(&self, class: &str) -> bool {
        bit(class).is_none_or(|b| self.mask & b != 0)
    }
}

struct Inventory<'a> {
    doc: &'a Document,
    items: Vec<Item>,
}

impl Inventory<'_> {
    fn add(&mut self, b: &Base, what: &str, mask: u8) {
        self.push(b.line, b.id(), what, mask);
    }
    fn push(&mut self, line: usize, id: &str, what: &str, mask: u8) {
        if mask == DAX {
            return;
        }
        self.items.push(Item {
            line,
            id: id.to_string(),
            what: what.to_string(),
            mask,
        });
    }

    fn triggers(b: &Base) -> Vec<&'static str> {
        let mut t: Vec<&'static str> = TRIGGERS
            .iter()
            .copied()
            .filter(|k| !b.values(k).is_empty())
            .collect();
        for f in TRIGGER_FLAGS
            .iter()
            .chain(CATCH_ALL_FLAGS)
            .chain(NAMELESS_TRIGGERS)
        {
            if b.has(f) && !t.contains(f) {
                t.push(f);
            }
        }
        t
    }

    fn event(&mut self, n: &Node) {
        let b = &n.base;
        let kind = b.kind.as_str();
        let trig = Self::triggers(b);
        let count: usize = TRIGGERS.iter().map(|k| b.values(k).len()).sum::<usize>()
            + trig.iter().filter(|t| !TRIGGERS.contains(t)).count();
        let in_eventsub = n
            .scope
            .as_ref()
            .and_then(|s| self.doc.nodes.get(s))
            .is_some_and(|s| s.base.kind == "eventsub");
        if count > 1 || b.has("parallel") {
            return self.add(b, "multiple event", F);
        }
        let t = trig.first().copied();
        let nonint = b.has("noninterrupting");
        let (what, mask): (String, u8) = match (kind, t) {
            ("start", None) => ("startEvent".into(), DAX),
            ("start", Some("message")) if !in_eventsub => ("messageStartEvent".into(), DAX),
            ("start", Some("timer")) if !in_eventsub => ("timerStartEvent".into(), DA),
            ("start", Some("signal")) if !in_eventsub => ("signalStartEvent".into(), A),
            ("start", Some("condition")) if !in_eventsub => ("conditionalStartEvent".into(), A),
            ("start", Some(x)) => (format!("{x} start event"), F),
            ("end", None) => ("endEvent".into(), DAX),
            ("end", Some("message")) => ("messageEndEvent".into(), DAX),
            ("end", Some("terminate")) => ("terminateEndEvent".into(), DAX),
            ("end", Some("signal")) => ("signalEndEvent".into(), A),
            ("end", Some("error")) => ("errorEndEvent".into(), A),
            ("end", Some("escalation")) => ("escalationEndEvent".into(), A),
            ("end", Some(x)) => (format!("{x} end event"), F),
            ("catch", Some(x @ ("message" | "timer"))) => {
                (format!("catching {x} intermediate event"), AX)
            }
            ("catch", Some(x @ ("signal" | "condition" | "link"))) => {
                (format!("catching {x} intermediate event"), A)
            }
            ("throw", Some("message")) => ("throwing message intermediate event".into(), AX),
            ("throw", Some(x @ ("signal" | "escalation" | "link"))) => {
                (format!("throwing {x} intermediate event"), A)
            }
            ("throw", None) => ("intermediate event without a trigger".into(), F),
            ("boundary", Some("error")) => ("boundary error event".into(), AX),
            ("boundary", Some("escalation")) if nonint => {
                ("non-interrupting boundary escalation event".into(), A)
            }
            ("boundary", Some(x @ ("message" | "timer" | "signal" | "condition"))) => {
                let kind = if nonint {
                    "non-interrupting boundary"
                } else {
                    "boundary"
                };
                (format!("{kind} {x} event"), A)
            }
            (k, Some(x)) => (format!("{x} {k} event"), F),
            (k, None) => (format!("{k} event"), F),
        };
        self.add(b, &what, mask);
    }

    fn node(&mut self, n: &Node) {
        let b = &n.base;
        let kind = b.kind.as_str();
        if n.choreography.is_some() {
            return;
        }
        if is_event(kind) {
            self.event(n);
        } else {
            let (what, mask) = match kind {
                "task" => ("task", DA),
                "user" => ("userTask", DAX),
                "service" => ("serviceTask", DAX),
                "send" => ("sendTask", A),
                "receive" if b.has("instantiate") => ("instantiating receiveTask", F),
                "receive" => ("receiveTask", A),
                "manual" => ("manualTask", F),
                "script" => ("scriptTask", F),
                "rule" => ("businessRuleTask", F),
                "call" if b.attrs.contains_key("task") => ("callActivity of a global task", F),
                "call" => ("callActivity", DAX),
                "sub" => ("subProcess", DAX),
                "eventsub" => ("event subProcess", F),
                "adhoc" => ("adHocSubProcess", F),
                "transaction" => ("transaction", F),
                "xor" => ("exclusiveGateway", DAX),
                "and" => ("parallelGateway", DAX),
                "or" => ("inclusiveGateway", A),
                "eventgw" if b.has("instantiate") => ("instantiating eventBasedGateway", F),
                "eventgw" => ("eventBasedGateway", AX),
                "complex" => ("complexGateway", F),
                _ => ("flow node", F),
            };
            self.add(b, what, mask);
        }
        if b.has("loop") {
            self.add(b, "standardLoopCharacteristics", AX);
            if b.attrs.contains_key("while") {
                self.add(b, "loopCondition", X);
            }
            if b.has("test-before") || b.attrs.contains_key("loop-max") {
                self.add(b, "testBefore, loopMaximum", F);
            }
        } else if is_activity(kind) && (b.has("parallel") || b.has("sequential")) {
            self.add(b, "multiInstanceLoopCharacteristics", AX);
        }
        if b.has("compensation") || b.attrs.contains_key("compensate") {
            self.add(b, "compensation", F);
        }
        if b.attrs.contains_key("operation") {
            let mask = if kind == "service" { X } else { F };
            self.add(b, "operationRef", mask);
        }
        if self.doc.lang_05() && b.attrs.contains_key("by") {
            let mask = if kind == "user" { X } else { F };
            self.add(b, "performer", mask);
        }
        if b.attrs.contains_key(DOC_KEY) {
            self.add(b, "documentation", DA);
        }
    }

    fn run(&mut self) {
        let d = self.doc;
        if d.attrs.contains_key(DOC_KEY) {
            self.push(d.header_line, "tok", "documentation", DA);
        }
        for n in d.nodes.values() {
            self.node(n);
        }
        for f in &d.flows {
            let b = &f.base;
            let src = d.nodes.get(&f.source);
            if src.is_some_and(|s| s.choreography.is_some()) {
                continue;
            }
            let src_kind = src.map_or("", |s| s.base.kind.as_str());
            if b.attrs.contains_key("when") {
                let mask = match src_kind {
                    "xor" | "or" | "eventgw" => AX,
                    _ => F,
                };
                self.add(b, "conditional sequenceFlow", mask);
            }
            if b.has("default") {
                let mask = match src_kind {
                    "xor" => AX,
                    "or" => A,
                    _ => F,
                };
                self.add(b, "default sequenceFlow", mask);
            }
            if b.attrs.contains_key(DOC_KEY) {
                self.add(b, "documentation", DA);
            }
        }
        for m in &d.messages {
            let named = m.base.label.as_deref().is_some_and(|l| !l.is_empty())
                || m.base.attr("message").is_some_and(|l| !l.is_empty());
            let mask = if named { A } else { DA };
            let what = if named {
                "messageFlow with a message"
            } else {
                "messageFlow"
            };
            self.add(&m.base, what, mask);
            if m.base.attrs.contains_key(DOC_KEY) {
                self.add(&m.base, "documentation", DA);
            }
        }
        for x in d.data.values() {
            let b = &x.base;
            match b.kind.as_str() {
                "data" => {
                    if b.attrs.contains_key("state") {
                        self.add(b, "dataState", F);
                    }
                    if b.has("collection") || b.attrs.contains_key("type") {
                        self.add(b, "dataObject isCollection, itemSubjectRef", X);
                    }
                }
                "store" => {
                    self.add(b, "dataStoreReference", DA);
                    if b.attrs.contains_key("type") {
                        self.add(b, "itemSubjectRef of a data store", F);
                    }
                }
                "property" => self.add(b, "property", F),
                _ => self.add(
                    b,
                    if b.kind == "input" {
                        "dataInput"
                    } else {
                        "dataOutput"
                    },
                    X,
                ),
            }
            if b.attrs.contains_key(DOC_KEY) {
                self.add(b, "documentation", DA);
            }
        }
        for a in &d.assocs {
            let on_event = d
                .nodes
                .get(&a.activity)
                .is_some_and(|n| is_event(&n.base.kind));
            if on_event || a.base.attrs.contains_key("input") || a.base.attrs.contains_key("output")
            {
                self.add(
                    &a.base,
                    "data association with a named data input or output",
                    X,
                );
            }
        }
        for n in &d.notes {
            if n.attrs.contains_key("on") {
                self.add(n, "association", DA);
            }
            if n.attrs.contains_key(DOC_KEY) {
                self.add(n, "documentation", DA);
            }
        }
        for g in &d.groups {
            self.add(g, "group", DA);
        }
        for p in &d.pools {
            self.add(&p.base, "participant", DA);
            if d.lang_06() && (p.base.attrs.contains_key("min") || p.base.attrs.contains_key("max"))
            {
                self.add(&p.base, "participantMultiplicity", F);
            }
            if p.base.attrs.contains_key(DOC_KEY) {
                self.add(&p.base, "documentation", DA);
            }
        }
        for l in d.lanes.values() {
            self.add(&l.base, "lane", DA);
        }
        for it in d.items.values() {
            self.add(it, "itemDefinition", X);
        }
        for g in d.globals.values() {
            self.add(g, "global task", F);
        }
        for x in d.interfaces.values() {
            self.add(&x.base, "interface", X);
        }
        for k in d.keys.values() {
            self.add(&k.base, "correlationKey", F);
        }
        if d.lang_05() {
            for r in d.resources.values() {
                self.add(r, "resource", X);
            }
        }
        for c in d.conversations.values() {
            let what = match c.base.kind.as_str() {
                "subconversation" => "subConversation",
                "callconversation" => "callConversation",
                _ => "conversation",
            };
            self.add(&c.base, what, F);
        }
        for c in d.choreographies.values() {
            self.add(&c.base, "choreography", F);
        }
    }
}

/// Every element of the document that is not in all three sub-classes, with the
/// sub-classes that contain it.
pub fn inventory(doc: &Document) -> Vec<Item> {
    let mut inv = Inventory {
        doc,
        items: Vec::new(),
    };
    inv.run();
    inv.items
}

/// The smallest class (in [`CLASSES`] order) that contains every element of the document.
pub fn bpmn_class(doc: &Document) -> &'static str {
    let items = inventory(doc);
    CLASSES
        .iter()
        .copied()
        .find(|c| items.iter().all(|i| i.fits(c)))
        .unwrap_or("full")
}

/// R075 for every element outside `class` (`tok check --class`).
pub fn check_class(doc: &Document, class: &str) -> Vec<Diag> {
    inventory(doc)
        .into_iter()
        .filter(|i| !i.fits(class))
        .map(|i| {
            Diag::new(
                "R075",
                i.line,
                Some(&i.id),
                &[
                    ("id", i.id.clone()),
                    ("what", i.what.clone()),
                    ("class", class.to_string()),
                    ("needed", i.needed().to_string()),
                ],
            )
        })
        .collect()
}
