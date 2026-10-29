//! `Model` of `bpmn.py`: indexes over a parsed BPMN document.

use std::collections::HashMap;

use crate::layout::key;
use crate::xml::{BPMN_NS, Doc, Id, NS, qn};

pub const ACT: &[&str] = &[
    "task",
    "userTask",
    "serviceTask",
    "sendTask",
    "receiveTask",
    "manualTask",
    "scriptTask",
    "businessRuleTask",
    "subProcess",
    "adHocSubProcess",
    "transaction",
    "callActivity",
];
pub const SUBP: &[&str] = &["subProcess", "adHocSubProcess", "transaction"];
pub const EVT: &[&str] = &[
    "startEvent",
    "endEvent",
    "intermediateCatchEvent",
    "intermediateThrowEvent",
    "boundaryEvent",
];
pub const GWS: &[&str] = &[
    "exclusiveGateway",
    "parallelGateway",
    "inclusiveGateway",
    "eventBasedGateway",
    "complexGateway",
];
pub const DATA: &[&str] = &["dataObjectReference", "dataStoreReference"];

pub fn is_act(t: &str) -> bool {
    ACT.contains(&t)
}
pub fn is_subp(t: &str) -> bool {
    SUBP.contains(&t)
}
pub fn is_evt(t: &str) -> bool {
    EVT.contains(&t)
}
pub fn is_gw(t: &str) -> bool {
    GWS.contains(&t)
}
/// Choreography activities (BPMN 2.0.2 §11.5): flow nodes of a choreography.
pub const CHOREO_ACT: &[&str] = &["choreographyTask", "subChoreography", "callChoreography"];
/// Conversation nodes of a collaboration (BPMN 2.0.2 §9.4).
pub const CONV: &[&str] = &["conversation", "subConversation", "callConversation"];

pub fn is_choreo_act(t: &str) -> bool {
    CHOREO_ACT.contains(&t)
}
pub fn is_conv(t: &str) -> bool {
    CONV.contains(&t)
}
pub fn is_flownode(t: &str) -> bool {
    is_act(t) || is_evt(t) || is_gw(t) || is_choreo_act(t)
}
pub fn is_data(t: &str) -> bool {
    DATA.contains(&t)
}
pub fn is_container(t: &str) -> bool {
    t == "process" || is_subp(t) || t == "choreography" || t == "subChoreography"
}
/// Data drawn as a shape: data object and store references and the data inputs and
/// outputs of a process (those of activities are not drawn; they never get a shape).
pub fn is_data_shape(t: &str) -> bool {
    is_data(t) || t == "dataInput" || t == "dataOutput"
}

/// `local(tag)`: local name of BPMN/DI elements; extension elements get `ext:`.
pub fn local(tag: &str) -> String {
    match tag.strip_prefix('{') {
        None => tag.to_string(),
        Some(rest) => {
            let (uri, name) = rest.split_once('}').unwrap_or((rest, ""));
            if NS.iter().any(|(_, u)| *u == uri) {
                name.to_string()
            } else {
                format!("ext:{name}")
            }
        }
    }
}

/// `ns_uri(tag)`.
pub fn ns_uri(tag: &str) -> &str {
    tag.strip_prefix('{')
        .and_then(|r| r.split_once('}'))
        .map_or("", |(u, _)| u)
}

/// `M + name`.
pub fn m(name: &str) -> String {
    qn(BPMN_NS, name)
}

pub fn tag_pl(t: &str) -> Option<&'static str> {
    Some(match t {
        "startEvent" => "zdarzenie początkowe",
        "endEvent" => "zdarzenie końcowe",
        "intermediateCatchEvent" => "pośrednie zdarzenie przechwytujące",
        "intermediateThrowEvent" => "pośrednie zdarzenie rzucające",
        "boundaryEvent" => "zdarzenie graniczne",
        "exclusiveGateway" => "bramka wykluczająca (XOR)",
        "parallelGateway" => "bramka równoległa (AND)",
        "inclusiveGateway" => "bramka inkluzywna (OR)",
        "eventBasedGateway" => "bramka oparta na zdarzeniach",
        "complexGateway" => "bramka złożona",
        "task" => "zadanie",
        "userTask" => "zadanie użytkownika",
        "serviceTask" => "zadanie usługowe",
        "sendTask" => "zadanie wysyłania",
        "receiveTask" => "zadanie odbioru",
        "manualTask" => "zadanie ręczne",
        "scriptTask" => "zadanie skryptowe",
        "businessRuleTask" => "zadanie reguły biznesowej",
        "subProcess" => "podproces",
        "adHocSubProcess" => "podproces ad hoc",
        "transaction" => "transakcja",
        "callActivity" => "czynność wywołania",
        _ => return None,
    })
}

/// `TAG_PL.get(t, t)`.
pub fn tag_pl_or(t: &str) -> String {
    tag_pl(t).map_or_else(|| t.to_string(), str::to_string)
}

/// Layout hints for diagrams that are not drawn like processes (choreographies): sizes of
/// nodes, top and bottom padding of containers, the gap between rows, the corner of
/// boundary events. Empty for processes.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub size: HashMap<String, (i64, i64)>,
    pub pad: HashMap<String, (i64, i64)>,
    pub row_gap: Option<i64>,
    /// Hosts whose boundary events stand at the bottom-left corner.
    pub boundary_left: std::collections::HashSet<String>,
}

pub struct Model {
    pub doc: Doc,
    pub hints: Hints,
    pub parent: HashMap<Id, Id>,
    pub ids: HashMap<String, Id>,
    pub dups: Vec<String>,
    pub seq: Vec<Id>,
    pub msg: Vec<Id>,
    pub assoc: Vec<Id>,
    pub processes: Vec<Id>,
    pub collabs: Vec<Id>,
    pub participants: Vec<Id>,
    pub out: HashMap<String, Vec<Id>>,
    pub inc: HashMap<String, Vec<Id>>,
    /// `attachedToRef -> boundary events`, in document order of first host appearance.
    pub boundaries: Vec<(String, Vec<Id>)>,
}

impl Model {
    pub fn new(doc: Doc) -> Model {
        let mut m = Model {
            doc,
            hints: Hints::default(),
            parent: HashMap::new(),
            ids: HashMap::new(),
            dups: Vec::new(),
            seq: Vec::new(),
            msg: Vec::new(),
            assoc: Vec::new(),
            processes: Vec::new(),
            collabs: Vec::new(),
            participants: Vec::new(),
            out: HashMap::new(),
            inc: HashMap::new(),
            boundaries: Vec::new(),
        };
        m.reindex();
        m
    }

    pub fn reindex(&mut self) {
        let d = &self.doc;
        let r = d.root;
        self.parent.clear();
        let all = d.iter(r);
        for p in &all {
            for c in d.children(*p) {
                self.parent.insert(*c, *p);
            }
        }
        self.ids.clear();
        self.dups.clear();
        for e in &all {
            if let Some(i) = d.get(*e, "id") {
                if self.ids.contains_key(i) {
                    self.dups.push(i.to_string());
                } else {
                    self.ids.insert(i.to_string(), *e);
                }
            }
        }
        self.seq = d.iter_tag(r, &m("sequenceFlow"));
        self.msg = d.iter_tag(r, &m("messageFlow"));
        self.assoc = d.iter_tag(r, &m("association"));
        self.processes = d.findall(r, &m("process"));
        self.collabs = d.findall(r, &m("collaboration"));
        self.participants = self
            .collabs
            .iter()
            .flat_map(|c| d.findall(*c, &m("participant")))
            .collect();
        self.out.clear();
        self.inc.clear();
        for f in &self.seq {
            self.out
                .entry(key(d.get(*f, "sourceRef")))
                .or_default()
                .push(*f);
            self.inc
                .entry(key(d.get(*f, "targetRef")))
                .or_default()
                .push(*f);
        }
        self.boundaries.clear();
        for b in d.iter_tag(r, &m("boundaryEvent")) {
            let host = key(d.get(b, "attachedToRef"));
            match self.boundaries.iter_mut().find(|(h, _)| *h == host) {
                Some((_, v)) => v.push(b),
                None => self.boundaries.push((host, vec![b])),
            }
        }
    }

    pub fn tag(&self, e: Id) -> String {
        local(self.doc.tag(e))
    }

    pub fn attr(&self, e: Id, k: &str) -> Option<&str> {
        self.doc.get(e, k)
    }

    pub fn id_of(&self, e: Id) -> Option<&str> {
        self.doc.get(e, "id")
    }

    pub fn get(&self, i: Option<&str>) -> Option<Id> {
        match i {
            Some(i) if !i.is_empty() => self.ids.get(i).copied(),
            _ => None,
        }
    }

    /// `m.out.get(i, [])` (a missing id behaves like Python's `None` key).
    pub fn out_of(&self, i: Option<&str>) -> &[Id] {
        self.out.get(&key(i)).map_or(&[], Vec::as_slice)
    }

    /// `m.inc.get(i, [])`.
    pub fn inc_of(&self, i: Option<&str>) -> &[Id] {
        self.inc.get(&key(i)).map_or(&[], Vec::as_slice)
    }

    /// `m.boundaries.get(host, [])`.
    pub fn boundaries_of(&self, host: Option<&str>) -> &[Id] {
        let k = key(host);
        self.boundaries
            .iter()
            .find(|(h, _)| *h == k)
            .map_or(&[], |(_, v)| v.as_slice())
    }

    pub fn container(&self, e: Id) -> Option<Id> {
        let mut p = self.parent.get(&e).copied();
        while let Some(x) = p {
            if is_container(&self.tag(x)) {
                return Some(x);
            }
            p = self.parent.get(&x).copied();
        }
        None
    }

    pub fn process_of(&self, e: Id) -> Option<Id> {
        if self.tag(e) == "process" {
            return Some(e);
        }
        let mut p = self.parent.get(&e).copied();
        while let Some(x) = p {
            if self.tag(x) == "process" {
                return Some(x);
            }
            p = self.parent.get(&x).copied();
        }
        None
    }

    pub fn participant_of(&self, e: Option<Id>) -> Option<Id> {
        let e = e?;
        if self.tag(e) == "participant" {
            return Some(e);
        }
        let proc = self.process_of(e)?;
        let pid = self.id_of(proc);
        self.participants
            .iter()
            .copied()
            .find(|p| self.attr(*p, "processRef") == pid)
    }

    /// `eventdefs(e)`: local names of event definitions, then "ref" per eventDefinitionRef.
    pub fn eventdefs(&self, e: Id) -> Vec<String> {
        let mut d: Vec<String> = self
            .doc
            .children(e)
            .iter()
            .map(|c| self.tag(*c))
            .filter(|t| t.ends_with("EventDefinition"))
            .collect();
        for _ in self.doc.findall(e, &m("eventDefinitionRef")) {
            d.push("ref".to_string());
        }
        d
    }

    pub fn is_event_sub(&self, e: Id) -> bool {
        self.tag(e) == "subProcess" && self.attr(e, "triggeredByEvent") == Some("true")
    }

    /// `Model.name(e)`.
    pub fn name(&self, e: Option<Id>) -> String {
        match e {
            None => "?".to_string(),
            Some(e) => self.attr(e, "name").unwrap_or("").trim().to_string(),
        }
    }

    /// `Model.label(e)`.
    pub fn label(&self, e: Option<Id>) -> String {
        let Some(e) = e else {
            return "?".to_string();
        };
        let n = self.name(Some(e));
        if !n.is_empty() {
            return format!("„{}” ({})", n, self.id_of(e).unwrap_or("None"));
        }
        match self.id_of(e) {
            Some(i) if !i.is_empty() => i.to_string(),
            _ => self.tag(e),
        }
    }

    /// `lanes_of(proc)`: (lane, depth, parent lane).
    pub fn lanes_of(&self, proc: Id) -> Vec<(Id, usize, Option<Id>)> {
        let mut res = Vec::new();
        for ls in self.doc.findall(proc, &m("laneSet")) {
            self.walk_lanes(ls, 0, None, &mut res);
        }
        res
    }

    fn walk_lanes(
        &self,
        ls: Id,
        depth: usize,
        parent: Option<Id>,
        res: &mut Vec<(Id, usize, Option<Id>)>,
    ) {
        for lane in self.doc.findall(ls, &m("lane")) {
            res.push((lane, depth, parent));
            if let Some(cls) = self.doc.find(lane, &m("childLaneSet")) {
                self.walk_lanes(cls, depth + 1, Some(lane), res);
            }
        }
    }

    pub fn is_leaf_lane(&self, lane: Id) -> bool {
        match self.doc.find(lane, &m("childLaneSet")) {
            None => true,
            Some(cls) => self.doc.findall(cls, &m("lane")).is_empty(),
        }
    }

    /// Data inputs and outputs of the `ioSpecification` of a process, in document order.
    pub fn process_io(&self, proc: Id) -> Vec<Id> {
        let mut res = Vec::new();
        if self.tag(proc) != "process" {
            return res;
        }
        for io in self.doc.findall(proc, &m("ioSpecification")) {
            for c in self.doc.children(io) {
                if matches!(self.tag(*c).as_str(), "dataInput" | "dataOutput") {
                    res.push(*c);
                }
            }
        }
        res
    }

    pub fn has_flow_children(&self, e: Id) -> bool {
        self.doc
            .children(e)
            .iter()
            .any(|c| is_flownode(&self.tag(*c)))
    }
}
