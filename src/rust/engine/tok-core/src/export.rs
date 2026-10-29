//! Semantic BPMN 2.0 export (no diagram interchange). Mirrors `bpmn_export.py`.

use indexmap::IndexMap;

use crate::model::*;
use crate::pyutil::{ascii_fold, strip};
use crate::xml::{BPMN_NS, MODELER_NS, XElem, XSD_NS, XSI_NS, ZEEBE_NS};

fn tag(kind: &str) -> &'static str {
    match kind {
        "start" => "startEvent",
        "end" => "endEvent",
        "catch" => "intermediateCatchEvent",
        "throw" => "intermediateThrowEvent",
        "boundary" => "boundaryEvent",
        "task" => "task",
        "user" => "userTask",
        "service" => "serviceTask",
        "manual" => "manualTask",
        "script" => "scriptTask",
        "rule" => "businessRuleTask",
        "send" => "sendTask",
        "receive" => "receiveTask",
        "call" => "callActivity",
        "sub" | "eventsub" => "subProcess",
        "adhoc" => "adHocSubProcess",
        "transaction" => "transaction",
        "xor" => "exclusiveGateway",
        "and" => "parallelGateway",
        "or" => "inclusiveGateway",
        "complex" => "complexGateway",
        _ => "eventBasedGateway",
    }
}

fn definition(key: &str) -> &'static str {
    match key {
        "message" => "messageEventDefinition",
        "timer" => "timerEventDefinition",
        "signal" => "signalEventDefinition",
        "error" => "errorEventDefinition",
        "escalation" => "escalationEventDefinition",
        "condition" => "conditionalEventDefinition",
        "link" => "linkEventDefinition",
        "terminate" => "terminateEventDefinition",
        "cancel" => "cancelEventDefinition",
        _ => "compensateEventDefinition",
    }
}

/// `'Zamrożenie budżetu' -> 'Zamrozenie_budzetu'` (ASCII identifier fragment of a label).
pub fn slug(text: &str) -> String {
    let t = text.replace('ł', "l").replace('Ł', "L");
    let t = ascii_fold(&t);
    let mut out = String::new();
    let mut in_run = false;
    for c in t.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    let t = out.trim_matches('_').to_string();
    match t.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => t,
        _ => format!("_{t}"),
    }
}

fn timer_kind(v: &str) -> &'static str {
    let v = strip(v);
    if v.starts_with('=') {
        "timeDuration"
    } else if v.starts_with('R') {
        "timeCycle"
    } else if v.starts_with('P') {
        "timeDuration"
    } else {
        "timeDate"
    }
}

const REST_CONNECTOR_TYPE: &str = "io.camunda:http-json:1";

fn binding_type(v: &str) -> &str {
    match v {
        "latest" => "latest",
        "deployment" => "deployment",
        "version-tag" => "versionTag",
        other => other,
    }
}

/// `'a = x; b = y'` (sep `=`) or `'k: v; …'` (sep `:`) into stripped pairs; items without
/// the separator are skipped (Python `split_pairs` of `bpmn_export.py`).
pub fn split_pairs(text: &str, sep: char) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for item in text.split(';') {
        let item = strip(item);
        if item.is_empty() {
            continue;
        }
        if let Some((l, r)) = item.split_once(sep) {
            out.push((strip(l).to_string(), strip(r).to_string()));
        }
    }
    out
}

/// One listener entry `event: type` or `event: type (retries)` (Python regex
/// `^([a-z]+)\s*:\s*(.+?)(?:\s*\((\d+)\))?$` on the stripped item).
pub fn parse_listener(item: &str) -> Option<(String, String, Option<String>)> {
    let item = strip(item);
    let ev_len = item.bytes().take_while(u8::is_ascii_lowercase).count();
    if ev_len == 0 {
        return None;
    }
    let event = &item[..ev_len];
    let rest = item[ev_len..].trim_start_matches(crate::pyutil::is_space);
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start_matches(crate::pyutil::is_space);
    if rest.is_empty() || rest.contains('\n') {
        return None;
    }
    // optional "(digits)" suffix, preceded by optional whitespace; the type keeps at least one char
    if let Some(body) = rest.strip_suffix(')')
        && let Some(open) = body.rfind('(')
    {
        let digits = &body[open + 1..];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let before = body[..open].trim_end_matches(crate::pyutil::is_space);
            if !before.is_empty() {
                return Some((
                    event.to_string(),
                    strip(before).to_string(),
                    Some(digits.to_string()),
                ));
            }
        }
    }
    Some((event.to_string(), strip(rest).to_string(), None))
}

#[derive(Debug, Clone)]
struct Decl {
    id: String,
    code: Option<String>,
    correlation: Option<String>,
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.filter(|x| !x.is_empty())
}

/// Path of a container (process or sub-process) inside the root element.
type Path = Vec<usize>;

/// A data input or output made for `reads` or `writes`: (`input` or `output`, identifier,
/// name).
type MadeIo = (&'static str, String, Option<String>);

struct Exporter<'a> {
    doc: &'a Document,
    camunda: bool,
    decls: [IndexMap<String, Decl>; 4],
    stores: IndexMap<String, String>,
    containers: IndexMap<String, Path>,
    root: XElem,
    collab: Option<Path>,
    /// Data inputs and outputs of activities made for `reads` and `writes` (language 0.5):
    /// activity -> (input or output, identifier, name).
    implicit_io: IndexMap<String, Vec<MadeIo>>,
    /// The `category` of the groups, a root element placed with the declarations at the
    /// end, so that the paths of the processes and the collaboration stay valid.
    category: Option<XElem>,
}

const DECL_KINDS: [&str; 4] = ["message", "signal", "error", "escalation"];

fn bsub(parent: &mut XElem, name: &str, attrs: &[(&str, Option<&str>)]) -> usize {
    let mut e = XElem::new(BPMN_NS, name);
    for (k, v) in attrs {
        if let Some(v) = v {
            e.set(k, *v);
        }
    }
    parent.push(e)
}

fn ext_index(e: &mut XElem) -> usize {
    if let Some(k) = e
        .children
        .iter()
        .position(|c| c.ns == BPMN_NS && c.name == "extensionElements")
    {
        return k;
    }
    let k = docs_len(e);
    e.insert(k, XElem::new(BPMN_NS, "extensionElements"));
    k
}

/// Number of leading `documentation` children (they come first in every BPMN element).
fn docs_len(e: &XElem) -> usize {
    e.children
        .iter()
        .take_while(|c| c.ns == BPMN_NS && c.name == "documentation")
        .count()
}

/// `documentation` of the element from `doc=` (language 0.5), as its first child.
fn documentation(e: &mut XElem, text: Option<&str>) {
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        let mut d = XElem::new(BPMN_NS, "documentation");
        d.text = Some(t.to_string());
        e.insert(0, d);
    }
}

fn zsub(parent: &mut XElem, name: &str, attrs: &[(&str, &str)]) {
    let mut e = XElem::new(ZEEBE_NS, name);
    for (k, v) in attrs {
        e.set(k, *v);
    }
    parent.push(e);
}

impl<'a> Exporter<'a> {
    fn new(doc: &'a Document, target: Option<&str>) -> Self {
        let t = target
            .map(str::to_string)
            .or_else(|| doc.attrs.get("target").cloned())
            .unwrap_or_else(|| "camunda".into());
        Exporter {
            doc,
            camunda: doc.executable_any() && t == "camunda",
            decls: Default::default(),
            stores: IndexMap::new(),
            containers: IndexMap::new(),
            root: XElem::new(BPMN_NS, "definitions"),
            collab: None,
            implicit_io: IndexMap::new(),
            category: None,
        }
    }

    fn feel(&self, expr: &str) -> String {
        if self.camunda {
            format!("={expr}")
        } else {
            expr.to_string()
        }
    }

    fn declare(&mut self, kind: &str, name: &str, code: Option<&str>) -> String {
        let k = DECL_KINDS.iter().position(|x| *x == kind).unwrap_or(0);
        let reg = &mut self.decls[k];
        if let Some(d) = reg.get_mut(name) {
            if let Some(c) = nonempty(code)
                && d.code.as_deref().map(|x| x.is_empty()).unwrap_or(true)
            {
                d.code = Some(c.to_string());
            }
            return d.id.clone();
        }
        let prefix = ["Message", "Signal", "Error", "Escalation"][k];
        let id = format!("{prefix}_{}", slug(name));
        reg.insert(
            name.to_string(),
            Decl {
                id: id.clone(),
                code: code.map(str::to_string),
                correlation: None,
            },
        );
        id
    }

    fn at(&mut self, path: &[usize]) -> &mut XElem {
        let mut e = &mut self.root;
        for &k in path {
            e = &mut e.children[k];
        }
        e
    }

    fn run(mut self) -> XElem {
        let d = self.doc;
        self.root.set(
            "id",
            d.attrs
                .get("id")
                .cloned()
                .unwrap_or_else(|| "definitions".into()),
        );
        if let Some(t) = nonempty(d.title.as_deref()) {
            self.root.set("name", t);
        }
        self.root.set(
            "targetNamespace",
            d.attrs
                .get("namespace")
                .cloned()
                .unwrap_or_else(|| "http://bpmn.io/schema/bpmn".into()),
        );
        self.root.set("exporter", "tok");
        self.root.set(
            "exporterVersion",
            d.version.clone().unwrap_or_else(|| "0.1".into()),
        );
        if self.camunda {
            self.root
                .set_ns(MODELER_NS, "executionPlatform", "Camunda Cloud");
            self.root
                .set_ns(MODELER_NS, "executionPlatformVersion", "8.10.0");
        }
        let decl_slot = self.root.children.len();

        let mut processes: Vec<(String, Option<String>, bool, Option<usize>)> = Vec::new();
        if !d.pools.is_empty() {
            let collab_id = d
                .attrs
                .get("collaboration")
                .cloned()
                .unwrap_or_else(|| "collaboration".into());
            let ci = bsub(&mut self.root, "collaboration", &[("id", Some(&collab_id))]);
            let inner = d.pools.iter().filter(|p| !p.blackbox).count();
            for (pi, p) in d.pools.iter().enumerate() {
                let part = {
                    let collab = &mut self.root.children[ci];
                    bsub(
                        collab,
                        "participant",
                        &[
                            ("id", p.base.id.as_deref()),
                            ("name", p.base.label.as_deref()),
                        ],
                    )
                };
                documentation(
                    &mut self.root.children[ci].children[part],
                    p.base.attr(DOC_KEY),
                );
                if let Some(m) = multiplicity(d, p) {
                    self.root.children[ci].children[part].push(m);
                }
                if !p.blackbox {
                    let pid = p.process_id();
                    self.root.children[ci].children[part].set("processRef", pid.clone());
                    let pname = if let Some(n) = p.base.attr("process-name") {
                        nonempty(Some(n)).map(str::to_string)
                    } else if inner == 1 && nonempty(d.title.as_deref()).is_some() {
                        d.title.clone()
                    } else {
                        p.base.label.clone()
                    };
                    processes.push((
                        pid,
                        pname,
                        p.base.has("executable") || d.executable(),
                        Some(pi),
                    ));
                }
            }
            for m in &d.messages {
                let mf = {
                    let collab = &mut self.root.children[ci];
                    bsub(
                        collab,
                        "messageFlow",
                        &[
                            ("id", m.base.id.as_deref()),
                            ("name", m.base.label.as_deref()),
                            ("sourceRef", Some(&m.source)),
                            ("targetRef", Some(&m.target)),
                        ],
                    )
                };
                let name = nonempty(m.base.attr("message"))
                    .or(m.base.label.as_deref())
                    .map(str::to_string);
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    let id = self.declare("message", &name, None);
                    self.root.children[ci].children[mf].set("messageRef", id);
                }
                documentation(
                    &mut self.root.children[ci].children[mf],
                    m.base.attr(DOC_KEY),
                );
            }
            self.collab = Some(vec![ci]);
        } else {
            let name = match d.attrs.get("process-name") {
                Some(n) => nonempty(Some(n)).map(str::to_string),
                None => d.title.clone(),
            };
            processes.push((d.default_process_id(), name, d.executable(), None));
        }

        let n_processes = processes.len();
        for (pid, name, executable, pool) in processes {
            let pi = bsub(
                &mut self.root,
                "process",
                &[
                    ("id", Some(&pid)),
                    ("name", name.as_deref()),
                    (
                        "isExecutable",
                        Some(if executable { "true" } else { "false" }),
                    ),
                ],
            );
            self.containers.insert(pid.clone(), vec![pi]);
            if pool.is_none() {
                // the header documents the process of a document without pools
                documentation(
                    &mut self.root.children[pi],
                    d.attrs.get(DOC_KEY).map(String::as_str),
                );
            }
            let tag = pool
                .and_then(|px| d.pools[px].base.attrs.get("version-tag").cloned())
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    if pool.is_none() || n_processes == 1 {
                        d.attrs.get("version-tag").cloned()
                    } else {
                        None
                    }
                })
                .filter(|t| !t.is_empty());
            if self.camunda
                && executable
                && let Some(tag) = tag
            {
                let x = ext_index(&mut self.root.children[pi]);
                zsub(
                    &mut self.root.children[pi].children[x],
                    "versionTag",
                    &[("value", &tag)],
                );
            }
            match pool {
                Some(px) => {
                    let pool = &d.pools[px];
                    let lanes: Vec<&Lane> = pool
                        .children
                        .iter()
                        .filter_map(|c| d.lanes.get(c))
                        .collect();
                    if !lanes.is_empty() {
                        let mut ls = XElem::new(BPMN_NS, "laneSet");
                        ls.set("id", format!("{}-lanes", pool.base.id()));
                        for lane in lanes {
                            self.lane(&mut ls, lane);
                        }
                        self.root.children[pi].push(ls);
                    }
                    let ids = self.ordered_nodes(&pool.children);
                    self.block(vec![pi], &ids);
                }
                None => {
                    let ids = d.top.clone();
                    self.block(vec![pi], &ids);
                }
            }
        }

        self.choreographies();
        self.flows();
        self.data();
        self.artifacts();
        self.compensation_links();
        self.order_adhoc();
        self.process_data();
        self.performers();
        self.supported_interfaces();
        let roots_05 = self.declarations_05();
        self.conversations();

        let mut decl: Vec<XElem> = roots_05.0;
        for (k, kind) in DECL_KINDS.iter().enumerate() {
            for (name, info) in &self.decls[k] {
                let mut e = XElem::new(BPMN_NS, kind);
                e.set("id", info.id.clone());
                e.set("name", name.clone());
                if let Some(code) = nonempty(info.code.as_deref()) {
                    if *kind == "error" {
                        e.set("errorCode", code);
                    }
                    if *kind == "escalation" {
                        e.set("escalationCode", code);
                    }
                }
                if *kind == "message"
                    && self.camunda
                    && let Some(corr) = nonempty(info.correlation.as_deref())
                {
                    let mut x = XElem::new(BPMN_NS, "extensionElements");
                    zsub(
                        &mut x,
                        "subscription",
                        &[("correlationKey", &self.feel(corr))],
                    );
                    e.push(x);
                }
                decl.push(e);
            }
        }
        for (name, sid) in &self.stores {
            let mut e = XElem::new(BPMN_NS, "dataStore");
            e.set("id", sid.clone());
            e.set("name", name.clone());
            decl.push(e);
        }
        decl.extend(roots_05.1);
        if let Some(cat) = self.category.take() {
            self.root.insert(decl_slot, cat);
        }
        for (k, e) in decl.into_iter().enumerate() {
            self.root.insert(decl_slot + k, e);
        }
        if !d.pools.is_empty() {
            // the header documents the collaboration of a document with pools (the definitions
            // of BPMN 2.0.2 are no base element and have no documentation)
            if let Some(c) = self
                .root
                .children
                .iter_mut()
                .find(|c| c.ns == BPMN_NS && c.name == "collaboration")
            {
                documentation(c, d.attrs.get(DOC_KEY).map(String::as_str));
            }
        }
        self.root
    }

    fn ordered_nodes(&self, children: &[String]) -> Vec<String> {
        fn walk(d: &Document, ids: &[String], out: &mut Vec<String>) {
            for i in ids {
                if let Some(l) = d.lanes.get(i) {
                    walk(d, &l.children, out);
                } else if d.nodes.contains_key(i) {
                    out.push(i.clone());
                }
            }
        }
        let mut out = Vec::new();
        walk(self.doc, children, &mut out);
        out
    }

    fn lane(&self, parent: &mut XElem, lane: &Lane) {
        let d = self.doc;
        let mut e = XElem::new(BPMN_NS, "lane");
        if let Some(id) = &lane.base.id {
            e.set("id", id.clone());
        }
        if let Some(l) = &lane.base.label {
            e.set("name", l.clone());
        }
        documentation(&mut e, lane.base.attr(DOC_KEY));
        for c in &lane.children {
            if let Some(node) = d.nodes.get(c) {
                let mut r = XElem::new(BPMN_NS, "flowNodeRef");
                r.text = Some(c.clone());
                e.push(r);
                if node.base.attrs.contains_key("compensate")
                    && compensation_boundary(d, c).is_none()
                {
                    let mut r = XElem::new(BPMN_NS, "flowNodeRef");
                    r.text = Some(compensation_id(c));
                    e.push(r);
                }
            }
        }
        let sublanes: Vec<&Lane> = lane
            .children
            .iter()
            .filter_map(|c| d.lanes.get(c))
            .collect();
        if !sublanes.is_empty() {
            let mut cls = XElem::new(BPMN_NS, "childLaneSet");
            cls.set("id", format!("{}-lanes", lane.base.id()));
            for s in sublanes {
                self.lane(&mut cls, s);
            }
            e.push(cls);
        }
        parent.push(e);
    }

    fn block(&mut self, container: Path, ids: &[String]) {
        for i in ids {
            if let Some(n) = self.doc.nodes.get(i) {
                self.node(container.clone(), n);
            }
        }
    }

    fn node(&mut self, container: Path, n: &Node) {
        let a = &n.base.attrs;
        let mut e = XElem::new(BPMN_NS, tag(&n.base.kind));
        if let Some(id) = &n.base.id {
            e.set("id", id.clone());
        }
        if let Some(l) = &n.base.label {
            e.set("name", l.clone());
        }
        documentation(&mut e, n.base.attr(DOC_KEY));
        let kind = n.base.kind.as_str();
        if kind == "boundary" {
            e.set("attachedToRef", a.get("on").cloned().unwrap_or_default());
            if n.base.has("noninterrupting") {
                e.set("cancelActivity", "false");
            }
        }
        if kind == "start" && n.base.has("noninterrupting") {
            e.set("isInterrupting", "false");
        }
        if kind == "eventsub" {
            e.set("triggeredByEvent", "true");
        }
        if kind == "call"
            && let Some(p) = a.get("process")
            && !p.starts_with('=')
        {
            e.set("calledElement", p.clone());
        }
        if kind == "call"
            && let Some(t) = a.get("task")
        {
            e.set("calledElement", t.clone());
        }
        if kind == "transaction"
            && let Some(m) = a.get("method")
            && let Some((_, v)) = TRANSACTION_METHODS.iter().find(|(k, _)| k == m)
        {
            e.set("method", *v);
        }
        if (kind == "receive" || kind == "eventgw") && n.base.has("instantiate") {
            e.set("instantiate", "true");
        }
        if kind == "eventgw" && n.base.has("parallel") {
            e.set("eventGatewayType", "Parallel");
        }
        if matches!(kind, "start" | "catch" | "boundary") && n.base.has("parallel") {
            e.set("parallelMultiple", "true");
        }
        if matches!(kind, "service" | "send" | "receive")
            && let Some(o) = a.get("operation")
        {
            e.set("operationRef", o.clone());
        }
        if kind == "adhoc" {
            if n.base.has("sequential") {
                e.set("ordering", "Sequential");
            }
            if self.camunda && n.base.has("wait") {
                e.set("cancelRemainingInstances", "false");
            }
        }
        if n.base.has("compensation") {
            e.set("isForCompensation", "true");
        }
        if (kind == "send" || kind == "receive")
            && let Some(m) = a.get("message")
        {
            let id = self.declare("message", m, None);
            e.set("messageRef", id);
        }
        if kind == "script"
            && !self.camunda
            && let Some(x) = a.get("expr")
        {
            e.set("scriptFormat", "feel");
            let mut s = XElem::new(BPMN_NS, "script");
            s.text = Some(x.clone());
            e.push(s);
        }
        if self.camunda {
            self.zeebe(&mut e, n);
        }
        if is_event(kind) {
            self.event_definition(&mut e, n);
        }
        if kind == "complex"
            && let Some(c) = a.get("activation")
        {
            e.push(self.formal("activationCondition", c));
        }
        if n.base.has("loop") {
            let mut lc = XElem::new(BPMN_NS, "standardLoopCharacteristics");
            if n.base.has("test-before") {
                lc.set("testBefore", "true");
            }
            if let Some(m) = a.get("loop-max") {
                lc.set("loopMaximum", m.clone());
            }
            if let Some(w) = a.get("while") {
                lc.push(self.formal("loopCondition", w));
            }
            e.push(lc);
        } else if is_activity(kind)
            && kind != "adhoc"
            && (n.base.has("parallel") || n.base.has("sequential"))
        {
            let mut mi = XElem::new(BPMN_NS, "multiInstanceLoopCharacteristics");
            mi.set(
                "isSequential",
                if n.base.has("sequential") {
                    "true"
                } else {
                    "false"
                },
            );
            if self.camunda
                && let Some(coll) = a.get("collection")
            {
                let mut lc: Vec<(&str, String)> = vec![("inputCollection", self.feel(coll))];
                if let Some(v) = a.get("item") {
                    lc.push(("inputElement", v.clone()));
                }
                if let Some(v) = a.get("output") {
                    lc.push(("outputCollection", v.clone()));
                }
                if let Some(v) = a.get("output-item") {
                    lc.push(("outputElement", self.feel(v)));
                }
                let x = ext_index(&mut mi);
                let pairs: Vec<(&str, &str)> = lc.iter().map(|(k, v)| (*k, v.as_str())).collect();
                zsub(&mut mi.children[x], "loopCharacteristics", &pairs);
            }
            if self.camunda
                && let Some(u) = a.get("until")
            {
                mi.push(self.formal("completionCondition", u));
            }
            e.push(mi);
        }
        let idx = self.at(&container).push(e);
        if is_sub(kind) {
            let mut path = container.clone();
            path.push(idx);
            self.containers
                .insert(n.base.id().to_string(), path.clone());
            let children = n.children.clone();
            self.block(path.clone(), &children);
            if kind == "adhoc"
                && self.camunda
                && let Some(u) = a.get("until")
            {
                let cc = self.formal("completionCondition", u);
                self.at(&path).push(cc);
            }
        }
        if a.contains_key("compensate") && compensation_boundary(self.doc, n.base.id()).is_none() {
            let bid = compensation_id(n.base.id());
            let mut b = XElem::new(BPMN_NS, "boundaryEvent");
            b.set("id", bid.clone());
            b.set("attachedToRef", n.base.id().to_string());
            let mut df = XElem::new(BPMN_NS, "compensateEventDefinition");
            df.set("id", format!("{bid}-def"));
            b.push(df);
            self.at(&container).push(b);
        }
    }

    /// `<bpmn:NAME xsi:type="bpmn:tFormalExpression">=expr</bpmn:NAME>`
    fn formal(&self, name: &str, expr: &str) -> XElem {
        let mut c = XElem::new(BPMN_NS, name);
        c.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
        c.text = Some(self.feel(expr));
        c
    }

    fn node_executable(&self, n: &Node) -> bool {
        let d = self.doc;
        if d.executable() {
            return true;
        }
        d.pools
            .iter()
            .any(|p| p.base.id.as_deref() == n.pool.as_deref() && p.base.has("executable"))
    }

    /// Associations from generated compensation boundary events to their handlers.
    fn compensation_links(&mut self) {
        let d = self.doc;
        for n in d.nodes.values() {
            let Some(h) = n.base.attrs.get("compensate") else {
                continue;
            };
            if !d.nodes.contains_key(h) {
                continue;
            }
            // looked up again: the category of groups is inserted before the processes
            let Some(path) = n.scope.as_ref().and_then(|s| self.root.find_path(s)) else {
                continue;
            };
            let bid = compensation_boundary(d, n.base.id())
                .map_or_else(|| compensation_id(n.base.id()), str::to_string);
            let mut a = XElem::new(BPMN_NS, "association");
            a.set("id", format!("{bid}-assoc"));
            a.set("associationDirection", "One");
            a.set("sourceRef", bid);
            a.set("targetRef", h.clone());
            self.at(&path).push(a);
        }
    }

    /// `completionCondition` is the last child of an ad-hoc sub-process (BPMN schema order).
    fn order_adhoc(&mut self) {
        fn walk(e: &mut XElem) {
            if e.ns == BPMN_NS && e.name == "adHocSubProcess" {
                let mut moved = Vec::new();
                let mut k = 0;
                while k < e.children.len() {
                    if e.children[k].ns == BPMN_NS && e.children[k].name == "completionCondition" {
                        moved.push(e.children.remove(k));
                    } else {
                        k += 1;
                    }
                }
                e.children.extend(moved);
            }
            for c in &mut e.children {
                walk(c);
            }
        }
        walk(&mut self.root);
    }

    fn binding_attrs(attrs: &mut Vec<(&'static str, String)>, a: &Attrs) {
        if let Some(b) = a.get("binding") {
            attrs.push(("bindingType", binding_type(b).to_string()));
        }
        if let Some(v) = a.get("version") {
            attrs.push(("versionTag", v.clone()));
        }
    }

    fn zsub_owned(parent: &mut XElem, name: &str, attrs: &[(&'static str, String)]) {
        let pairs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        zsub(parent, name, &pairs);
    }

    fn zeebe(&mut self, e: &mut XElem, n: &Node) {
        if !self.node_executable(n) {
            // only the 0.1 executable profile applies to processes that are not executable
            self.zeebe_01(e, n);
            return;
        }
        let a = &n.base.attrs;
        let kind = n.base.kind.as_str();
        if kind == "user" {
            let x = ext_index(e);
            zsub(&mut e.children[x], "userTask", &[]);
            if let Some(f) = a.get("form") {
                let mut at = vec![("formId", f.clone())];
                Self::binding_attrs(&mut at, a);
                Self::zsub_owned(&mut e.children[x], "formDefinition", &at);
            }
            let mut assign = Vec::new();
            for (key, attr) in [
                ("assignee", "assignee"),
                ("candidate-users", "candidateUsers"),
                ("candidate-groups", "candidateGroups"),
            ] {
                if let Some(v) = a.get(key) {
                    assign.push((attr, v.clone()));
                }
            }
            if !assign.is_empty() {
                Self::zsub_owned(&mut e.children[x], "assignmentDefinition", &assign);
            }
            let mut sched = Vec::new();
            for (key, attr) in [("due", "dueDate"), ("follow-up", "followUpDate")] {
                if let Some(v) = a.get(key) {
                    sched.push((attr, v.clone()));
                }
            }
            if !sched.is_empty() {
                Self::zsub_owned(&mut e.children[x], "taskSchedule", &sched);
            }
            if let Some(p) = a.get("priority") {
                zsub(&mut e.children[x], "priorityDefinition", &[("priority", p)]);
            }
        } else if kind == "start" && a.contains_key("form") {
            let mut at = vec![("formId", a["form"].clone())];
            Self::binding_attrs(&mut at, a);
            let x = ext_index(e);
            Self::zsub_owned(&mut e.children[x], "formDefinition", &at);
        } else if kind == "rule" && a.contains_key("decision") {
            let res = match nonempty(a.get("result").map(String::as_str)) {
                Some(r) => r.to_string(),
                None => self.default_result(&a["decision"]),
            };
            let mut at = vec![
                ("decisionId", a["decision"].clone()),
                ("resultVariable", res),
            ];
            Self::binding_attrs(&mut at, a);
            let x = ext_index(e);
            Self::zsub_owned(&mut e.children[x], "calledDecision", &at);
        } else if kind == "script" && a.contains_key("expr") {
            let expr = self.feel(&a["expr"]);
            let res = a.get("result").cloned().unwrap_or_else(|| "result".into());
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "script",
                &[("expression", &expr), ("resultVariable", &res)],
            );
        } else if kind == "call" && a.contains_key("process") {
            let mut ce = vec![("processId", a["process"].clone())];
            Self::binding_attrs(&mut ce, a);
            if a.get("child-vars").map(String::as_str) == Some("none") {
                ce.push(("propagateAllChildVariables", "false".into()));
            }
            if a.get("parent-vars").map(String::as_str) == Some("none") {
                ce.push(("propagateAllParentVariables", "false".into()));
            }
            if let Some(b) = a.get("business-id") {
                ce.push(("businessId", b.clone()));
            }
            let x = ext_index(e);
            Self::zsub_owned(&mut e.children[x], "calledElement", &ce);
        } else if kind == "adhoc" && a.contains_key("active") {
            let v = self.feel(&a["active"]);
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "adHoc",
                &[("activeElementsCollection", &v)],
            );
        }
        let job_kind = kind == "service"
            || kind == "send"
            || ((kind == "throw" || kind == "end")
                && (a.contains_key("message") || n.base.has("message")));
        let inputs = split_pairs(a.get("inputs").map(String::as_str).unwrap_or(""), '=');
        let mut headers = split_pairs(a.get("headers").map(String::as_str).unwrap_or(""), ':');
        if kind == "service" && a.get("connector").map(String::as_str) == Some("rest") {
            let mut td = vec![("type", REST_CONNECTOR_TYPE.to_string())];
            if let Some(r) = a.get("retries") {
                td.push(("retries", r.clone()));
            }
            let x = ext_index(e);
            Self::zsub_owned(&mut e.children[x], "taskDefinition", &td);
            let mut conn: Vec<(String, String)> = vec![
                ("authentication.type".into(), "noAuth".into()),
                (
                    "method".into(),
                    a.get("method").cloned().unwrap_or_else(|| "GET".into()),
                ),
                ("url".into(), a.get("url").cloned().unwrap_or_default()),
            ];
            for (key, target) in [
                ("headers-in", "headers"),
                ("query", "queryParameters"),
                ("body", "body"),
            ] {
                if let Some(v) = a.get(key) {
                    conn.push((target.into(), v.clone()));
                }
            }
            Self::io_mapping(e, &conn, &[]);
            if let Some(r) = a.get("result") {
                headers.insert(0, ("resultVariable".into(), r.clone()));
            }
        } else if job_kind && a.contains_key("type") {
            let mut td = vec![("type", a["type"].clone())];
            if let Some(r) = a.get("retries") {
                td.push(("retries", r.clone()));
            }
            let x = ext_index(e);
            Self::zsub_owned(&mut e.children[x], "taskDefinition", &td);
        }
        if !inputs.is_empty() || a.contains_key("outputs") {
            let ins: Vec<(String, String)> = inputs
                .iter()
                .map(|(t, s)| (t.clone(), self.feel(s)))
                .collect();
            let outs: Vec<(String, String)> =
                split_pairs(a.get("outputs").map(String::as_str).unwrap_or(""), '=')
                    .iter()
                    .map(|(t, s)| (t.clone(), self.feel(s)))
                    .collect();
            Self::io_mapping(e, &ins, &outs);
        }
        if !headers.is_empty() {
            let x = ext_index(e);
            let mut th = XElem::new(ZEEBE_NS, "taskHeaders");
            for (k, v) in &headers {
                let mut h = XElem::new(ZEEBE_NS, "header");
                h.set("key", k.clone());
                h.set("value", v.clone());
                th.push(h);
            }
            e.children[x].push(th);
        }
        if let Some(ev) = a.get("events")
            && matches!(kind, "start" | "catch" | "boundary")
        {
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "conditionalFilter",
                &[("variableEvents", ev)],
            );
        }
        if let Some(text) = a.get("listeners") {
            let mut execl = Vec::new();
            let mut taskl = Vec::new();
            for item in text.split(';') {
                if let Some((ev, typ, retries)) = parse_listener(item) {
                    let mut li: Vec<(&'static str, String)> =
                        vec![("eventType", ev.clone()), ("type", typ)];
                    if let Some(r) = retries {
                        li.push(("retries", r));
                    }
                    if ev == "start" || ev == "end" {
                        execl.push(li);
                    } else {
                        taskl.push(li);
                    }
                }
            }
            for (box_name, item_name, items) in [
                ("executionListeners", "executionListener", execl),
                ("taskListeners", "taskListener", taskl),
            ] {
                if items.is_empty() {
                    continue;
                }
                let x = ext_index(e);
                let mut bx = XElem::new(ZEEBE_NS, box_name);
                for li in &items {
                    let mut it = XElem::new(ZEEBE_NS, item_name);
                    for (k, v) in li {
                        it.set(k, v.clone());
                    }
                    bx.push(it);
                }
                e.children[x].push(bx);
            }
        }
    }

    fn io_mapping(e: &mut XElem, inputs: &[(String, String)], outputs: &[(String, String)]) {
        let x = ext_index(e);
        let ext = &mut e.children[x];
        let k = match ext
            .children
            .iter()
            .position(|c| c.ns == ZEEBE_NS && c.name == "ioMapping")
        {
            Some(k) => k,
            None => ext.push(XElem::new(ZEEBE_NS, "ioMapping")),
        };
        let bx = &mut ext.children[k];
        for (target, source) in inputs {
            let mut i = XElem::new(ZEEBE_NS, "input");
            i.set("source", source.clone());
            i.set("target", target.clone());
            bx.push(i);
        }
        for (target, source) in outputs {
            let mut o = XElem::new(ZEEBE_NS, "output");
            o.set("source", source.clone());
            o.set("target", target.clone());
            bx.push(o);
        }
    }

    /// The executable profile of language 0.1 (used where the process itself is not executable).
    fn zeebe_01(&mut self, e: &mut XElem, n: &Node) {
        let a = &n.base.attrs;
        let kind = n.base.kind.as_str();
        if kind == "user" {
            let x = ext_index(e);
            zsub(&mut e.children[x], "userTask", &[]);
            if let Some(f) = a.get("form") {
                zsub(&mut e.children[x], "formDefinition", &[("formId", f)]);
            }
        } else if kind == "start" && a.contains_key("form") {
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "formDefinition",
                &[("formId", &a["form"])],
            );
        } else if kind == "rule" && a.contains_key("decision") {
            let res = match nonempty(a.get("result").map(String::as_str)) {
                Some(r) => r.to_string(),
                None => self.default_result(&a["decision"]),
            };
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "calledDecision",
                &[("decisionId", &a["decision"]), ("resultVariable", &res)],
            );
        } else if kind == "script" && a.contains_key("expr") {
            let expr = self.feel(&a["expr"]);
            let res = a.get("result").cloned().unwrap_or_else(|| "result".into());
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "script",
                &[("expression", &expr), ("resultVariable", &res)],
            );
        } else if (kind == "service" || kind == "send") && a.contains_key("type") {
            let x = ext_index(e);
            zsub(
                &mut e.children[x],
                "taskDefinition",
                &[("type", &a["type"])],
            );
        }
    }

    fn default_result(&self, decision: &str) -> String {
        match self.doc.decisions.get(decision) {
            Some(d) if d.outputs.len() == 1 => d.outputs[0].0.clone(),
            _ => "result".into(),
        }
    }

    fn event_definition(&mut self, e: &mut XElem, n: &Node) {
        let a = &n.base.attrs;
        // a 0.1 or 0.3 document exports only the first trigger, as before
        let multiple = self.doc.lang_05();
        let mut count = 0;
        let next_id = |count: &mut usize| {
            *count += 1;
            if *count == 1 {
                format!("{}-def", n.base.id())
            } else {
                format!("{}-def-{count}", n.base.id())
            }
        };
        let triggers: Vec<(&str, &str)> = TRIGGERS
            .iter()
            .flat_map(|k| n.base.values(k).into_iter().map(move |v| (*k, v)))
            .collect();
        for (key, v) in triggers {
            if count > 0 && !multiple {
                return;
            }
            let v = &v.to_string();
            let mut df = XElem::new(BPMN_NS, definition(key));
            df.set("id", next_id(&mut count));
            match key {
                "message" => {
                    let id = self.declare("message", v, None);
                    if let Some(c) = a.get("correlation") {
                        let k = 0;
                        if let Some(info) = self.decls[k].get_mut(v.as_str()) {
                            info.correlation = Some(c.clone());
                        }
                    }
                    df.set("messageRef", id);
                }
                "signal" => {
                    let id = self.declare("signal", v, None);
                    df.set("signalRef", id);
                }
                "error" => {
                    let id = self.declare("error", v, a.get("code").map(String::as_str));
                    df.set("errorRef", id);
                }
                "escalation" => {
                    let id = self.declare("escalation", v, a.get("code").map(String::as_str));
                    df.set("escalationRef", id);
                }
                "timer" => {
                    let mut t = XElem::new(BPMN_NS, timer_kind(v));
                    t.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
                    t.text = Some(v.clone());
                    df.push(t);
                }
                "condition" => {
                    let mut c = XElem::new(BPMN_NS, "condition");
                    c.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
                    c.text = Some(self.feel(v));
                    df.push(c);
                }
                _ => df.set("name", v.clone()),
            }
            e.push(df);
        }
        // triggers without a value (language 0.5): an event definition without a reference
        for flag in NAMELESS_TRIGGERS {
            if count > 0 && !multiple {
                return;
            }
            if n.base.has(flag) {
                let mut df = XElem::new(BPMN_NS, definition(flag));
                df.set("id", next_id(&mut count));
                if *flag == "condition" {
                    // a conditional event definition always has a condition
                    let mut c = XElem::new(BPMN_NS, "condition");
                    c.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
                    df.push(c);
                }
                e.push(df);
            }
        }
        for flag in TRIGGER_FLAGS.iter().chain(CATCH_ALL_FLAGS) {
            if count > 0 && !multiple {
                return;
            }
            if n.base.has(flag) {
                let mut df = XElem::new(BPMN_NS, definition(flag));
                df.set("id", next_id(&mut count));
                if *flag == "compensate"
                    && let Some(act) = a.get("activity")
                {
                    df.set("activityRef", act.clone());
                }
                e.push(df);
            }
        }
    }

    fn flows(&mut self) {
        let d = self.doc;
        for f in &d.flows {
            let (Some(src), Some(tgt)) = (d.nodes.get(&f.source), d.nodes.get(&f.target)) else {
                continue;
            };
            if src.scope != tgt.scope {
                continue;
            }
            let Some(path) = src
                .scope
                .as_ref()
                .and_then(|s| self.containers.get(s))
                .cloned()
            else {
                continue;
            };
            let mut e = XElem::new(BPMN_NS, "sequenceFlow");
            if let Some(id) = &f.base.id {
                e.set("id", id.clone());
            }
            if let Some(l) = &f.base.label {
                e.set("name", l.clone());
            }
            e.set("sourceRef", f.source.clone());
            e.set("targetRef", f.target.clone());
            documentation(&mut e, f.base.attr(DOC_KEY));
            if let Some(w) = f.base.attrs.get("when") {
                let mut c = XElem::new(BPMN_NS, "conditionExpression");
                c.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
                c.text = Some(self.feel(w));
                e.push(c);
            }
            self.at(&path).push(e);
            if f.base.has("default")
                && let Some(p) = self.root.find_path(src.base.id())
                && let Some(el) = self.root.at_mut(&p)
            {
                el.set("default", f.base.id().to_string());
            }
        }
    }

    fn data_scope(&self, x: &Data) -> String {
        let d = self.doc;
        if let Some(s) = nonempty(x.scope.as_deref()) {
            return s.to_string();
        }
        for a in &d.assocs {
            if a.data == x.base.id()
                && let Some(n) = d.nodes.get(&a.activity)
            {
                return n.scope.clone().unwrap_or_default();
            }
        }
        for p in &d.pools {
            if !p.blackbox {
                return p.process_id();
            }
        }
        d.default_process_id()
    }

    fn data(&mut self) {
        let d = self.doc;
        // data objects named by `object=` (language 0.5), shared by the references with the
        // same object name in one process or sub-process: (scope, name) -> identifier
        let mut shared: IndexMap<(String, String), String> = IndexMap::new();
        for x in d
            .data
            .values()
            .filter(|x| !IO_KINDS.contains(&x.base.kind.as_str()))
        {
            let scope = self.data_scope(x);
            let Some(path) = self.containers.get(&scope).cloned() else {
                continue;
            };
            let id = x.base.id().to_string();
            if x.base.kind == "data" {
                let object = x.base.attr("object");
                let known = object.and_then(|o| shared.get(&(scope.clone(), o.to_string())));
                let oid = known.cloned().unwrap_or_else(|| format!("{id}-object"));
                let obj = if known.is_some() {
                    None
                } else {
                    let mut obj = XElem::new(BPMN_NS, "dataObject");
                    obj.set("id", oid.clone());
                    match object {
                        Some(o) => {
                            if !o.is_empty() {
                                obj.set("name", o);
                            }
                            shared.insert((scope.clone(), o.to_string()), oid.clone());
                        }
                        None => {
                            if let Some(l) = &x.base.label {
                                obj.set("name", l.clone());
                            }
                        }
                    }
                    if let Some(t) = x.base.attrs.get("type") {
                        obj.set("itemSubjectRef", t.clone());
                    }
                    if x.base.has("collection") {
                        obj.set("isCollection", "true");
                    }
                    Some(obj)
                };
                let mut r = XElem::new(BPMN_NS, "dataObjectReference");
                r.set("id", id.clone());
                if let Some(l) = &x.base.label {
                    r.set("name", l.clone());
                }
                r.set("dataObjectRef", oid);
                documentation(&mut r, x.base.attr(DOC_KEY));
                if let Some(st) = x.base.attrs.get("state") {
                    let mut s = XElem::new(BPMN_NS, "dataState");
                    s.set("id", format!("{id}-state"));
                    s.set("name", st.clone());
                    r.push(s);
                }
                let c = self.at(&path);
                if let Some(obj) = obj {
                    c.push(obj);
                }
                c.push(r);
            } else {
                let name = nonempty(x.base.label.as_deref()).unwrap_or(&id).to_string();
                let sid = self
                    .stores
                    .entry(name)
                    .or_insert_with(|| format!("{id}-store"))
                    .clone();
                let mut r = XElem::new(BPMN_NS, "dataStoreReference");
                r.set("id", id.clone());
                if let Some(l) = &x.base.label {
                    r.set("name", l.clone());
                }
                r.set("dataStoreRef", sid);
                if let Some(t) = x.base.attrs.get("type") {
                    r.set("itemSubjectRef", t.clone());
                }
                documentation(&mut r, x.base.attr(DOC_KEY));
                self.at(&path).push(r);
            }
        }
        let mut n = 0;
        for a in &d.assocs {
            let Some(path) = self.root.find_path(&a.activity) else {
                continue;
            };
            if !d.data.contains_key(&a.data) {
                continue;
            }
            n += 1;
            let Some(act) = self.root.at_mut(&path) else {
                continue;
            };
            let pos = docs_len(act)
                + usize::from(
                    act.children
                        .iter()
                        .any(|c| c.ns == BPMN_NS && c.name == "extensionElements"),
                );
            if d.lang_05() {
                // the association connects the data element with a data input or output of
                // the activity's ioSpecification (BPMN 2.0.2, 10.4.1)
                let name = d.data[&a.data]
                    .base
                    .label
                    .clone()
                    .or_else(|| Some(a.data.clone()));
                let reads = a.base.kind == "reads";
                // `input=` / `output=` name a declared data input or output of the activity
                let declared = a.base.attr(if reads { "input" } else { "output" });
                let io_id = match declared {
                    Some(io) => io.to_string(),
                    None => format!("{}-{}-{n}", a.activity, if reads { "in" } else { "out" }),
                };
                let mut assoc = XElem::new(
                    BPMN_NS,
                    if reads {
                        "dataInputAssociation"
                    } else {
                        "dataOutputAssociation"
                    },
                );
                assoc.set(
                    "id",
                    format!("{}-{}-{n}", a.activity, if reads { "dia" } else { "doa" }),
                );
                let (source, target) = if reads {
                    (a.data.clone(), io_id.clone())
                } else {
                    (io_id.clone(), a.data.clone())
                };
                assoc.push(text_elem("sourceRef", &source));
                assoc.push(text_elem("targetRef", &target));
                insert_ranked(act, assoc);
                if declared.is_none() {
                    self.implicit_io
                        .entry(a.activity.clone())
                        .or_default()
                        .push((if reads { "input" } else { "output" }, io_id, name));
                }
                continue;
            }
            if a.base.kind == "reads" {
                let prop_id = format!("{}-in-{n}", a.activity);
                let mut prop = XElem::new(BPMN_NS, "property");
                prop.set("id", prop_id.clone());
                prop.set("name", "__targetRef_placeholder");
                let mut assoc = XElem::new(BPMN_NS, "dataInputAssociation");
                assoc.set("id", format!("{}-dia-{n}", a.activity));
                let mut s = XElem::new(BPMN_NS, "sourceRef");
                s.text = Some(a.data.clone());
                assoc.push(s);
                let mut t = XElem::new(BPMN_NS, "targetRef");
                t.text = Some(prop_id);
                assoc.push(t);
                act.insert(pos, prop);
                let ap = after_props(act);
                act.insert(ap, assoc);
            } else {
                let mut assoc = XElem::new(BPMN_NS, "dataOutputAssociation");
                assoc.set("id", format!("{}-doa-{n}", a.activity));
                let mut t = XElem::new(BPMN_NS, "targetRef");
                t.text = Some(a.data.clone());
                assoc.push(t);
                let ap = after_props(act);
                act.insert(ap, assoc);
            }
        }
    }

    fn first_container(&self) -> Option<Path> {
        self.containers.values().next().cloned()
    }

    fn artifacts(&mut self) {
        let d = self.doc;
        for note in &d.notes {
            let target = note.attrs.get("on").cloned();
            let mut cont = None;
            if let Some(t) = &target
                && let Some(node) = d.nodes.get(t)
            {
                cont = node
                    .scope
                    .as_ref()
                    .and_then(|s| self.containers.get(s))
                    .cloned();
            }
            if cont.is_none() {
                cont = self.collab.clone().or_else(|| self.first_container());
            }
            let Some(path) = cont else { continue };
            let mut t = XElem::new(BPMN_NS, "textAnnotation");
            t.set("id", note.id().to_string());
            documentation(&mut t, note.attr(DOC_KEY));
            let mut tx = XElem::new(BPMN_NS, "text");
            tx.text = Some(note.label.clone().unwrap_or_default());
            t.push(tx);
            let c = self.at(&path);
            // the artifacts of a choreography precede its flow elements (`tCollaboration`)
            let mut at = if c.name == "choreography" {
                c.children
                    .iter()
                    .position(|x| {
                        !matches!(
                            x.name.as_str(),
                            "documentation"
                                | "participant"
                                | "messageFlow"
                                | "textAnnotation"
                                | "association"
                        )
                    })
                    .unwrap_or(c.children.len())
            } else {
                c.children.len()
            };
            c.insert(at, t);
            at += 1;
            if let Some(tg) = target.filter(|t| !t.is_empty()) {
                let mut a = XElem::new(BPMN_NS, "association");
                a.set("id", format!("{}-assoc", note.id()));
                a.set("sourceRef", note.id().to_string());
                a.set("targetRef", tg);
                c.insert(at, a);
            }
        }
        if !d.groups.is_empty() {
            let mut cat = XElem::new(BPMN_NS, "category");
            cat.set("id", "categories");
            for g in &d.groups {
                let mut cv = XElem::new(BPMN_NS, "categoryValue");
                let cvid = format!("{}-value", g.id());
                cv.set("id", cvid.clone());
                cv.set(
                    "value",
                    nonempty(g.label.as_deref()).unwrap_or(g.id()).to_string(),
                );
                cat.push(cv);
                let path = self
                    .collab
                    .clone()
                    .or_else(|| self.first_container())
                    .unwrap_or_default();
                let mut ge = XElem::new(BPMN_NS, "group");
                ge.set("id", g.id().to_string());
                ge.set("categoryValueRef", cvid);
                documentation(&mut ge, g.attr(DOC_KEY));
                self.at(&path).push(ge);
            }
            self.category = Some(cat);
        }
    }
}

/// Position of a child in the BPMN schema order of activities, events and processes
/// (`tBaseElement`, `tCallableElement`, `tProcess`, `tActivity`, `tEvent`); elements of the
/// subtypes (event definitions, inner flow elements, scripts) come last.
fn schema_rank(name: &str) -> u8 {
    match name {
        "documentation" => 0,
        "extensionElements" => 1,
        "supportedInterfaceRef" => 2,
        "ioSpecification" => 3,
        "ioBinding" => 4,
        "property" => 5,
        // data inputs and outputs of events (activities have them in the ioSpecification)
        "dataInput" | "dataOutput" => 6,
        "dataInputAssociation" => 7,
        "dataOutputAssociation" => 8,
        "inputSet" | "outputSet" => 9,
        "laneSet" => 10,
        "performer" | "humanPerformer" | "potentialOwner" | "resourceRole" => 11,
        "standardLoopCharacteristics" | "multiInstanceLoopCharacteristics" => 12,
        _ => 13,
    }
}

/// Insert `child` before the first child that comes later in the schema order.
fn insert_ranked(parent: &mut XElem, child: XElem) {
    let r = schema_rank(&child.name);
    let pos = parent
        .children
        .iter()
        .position(|c| c.ns == BPMN_NS && schema_rank(&c.name) > r)
        .unwrap_or(parent.children.len());
    parent.insert(pos, child);
}

/// A `dataInput`, `dataOutput` or `property` (item-aware elements of language 0.5).
fn item_aware(name: &str, x: &Data) -> XElem {
    let mut e = XElem::new(BPMN_NS, name);
    e.set("id", x.base.id().to_string());
    if let Some(l) = &x.base.label {
        e.set("name", l.clone());
    }
    if let Some(t) = x.base.attrs.get("type") {
        e.set("itemSubjectRef", t.clone());
    }
    if name != "property" && x.base.has("collection") {
        e.set("isCollection", "true");
    }
    documentation(&mut e, x.base.attr(DOC_KEY));
    e
}

fn text_elem(name: &str, text: &str) -> XElem {
    let mut e = XElem::new(BPMN_NS, name);
    e.text = Some(text.to_string());
    e
}

/// Identifier of the `correlationProperty` exported for a `prop` name.
pub fn correlation_property_id(name: &str) -> String {
    format!("CorrelationProperty_{}", slug(name))
}

impl Exporter<'_> {
    /// The element (process, sub-process or activity) that owns an input, output or property.
    fn io_owner(&self, x: &Data) -> String {
        let d = self.doc;
        if let Some(on) = x.base.attrs.get("on") {
            return on.clone();
        }
        if let Some(s) = nonempty(x.scope.as_deref()) {
            return s.to_string();
        }
        d.pools
            .iter()
            .find(|p| !p.blackbox)
            .map(Pool::process_id)
            .unwrap_or_else(|| d.default_process_id())
    }

    /// `ioSpecification` with the data inputs and outputs, and `property` elements.
    fn process_data(&mut self) {
        let d = self.doc;
        let mut owners: IndexMap<String, Vec<&Data>> = IndexMap::new();
        for x in d
            .data
            .values()
            .filter(|x| IO_KINDS.contains(&x.base.kind.as_str()))
        {
            owners.entry(self.io_owner(x)).or_default().push(x);
        }
        for act in self.implicit_io.keys() {
            owners.entry(act.clone()).or_default();
        }
        for (owner, xs) in owners {
            let implicit = self.implicit_io.get(&owner).cloned().unwrap_or_default();
            let made = |k: &str| -> Vec<XElem> {
                implicit
                    .iter()
                    .filter(|(kind, _, _)| *kind == k)
                    .map(|(_, id, name)| {
                        let mut e = XElem::new(
                            BPMN_NS,
                            if k == "input" {
                                "dataInput"
                            } else {
                                "dataOutput"
                            },
                        );
                        e.set("id", id.clone());
                        if let Some(nm) = name {
                            e.set("name", nm.clone());
                        }
                        e
                    })
                    .collect()
            };
            let (made_in, made_out) = (made("input"), made("output"));
            let Some(path) = self.root.find_path(&owner) else {
                continue;
            };
            let of = |k: &str| -> Vec<&Data> {
                xs.iter().copied().filter(|x| x.base.kind == k).collect()
            };
            let (ins, outs, props) = (of("input"), of("output"), of("property"));
            let Some(el) = self.root.at_mut(&path) else {
                continue;
            };
            if el.name.ends_with("Event") {
                // an event has its data inputs (throwing) or outputs (catching) and their
                // set directly, without an ioSpecification
                let (item, set, refs, xs, made) =
                    if el.name.contains("Throw") || el.name == "endEvent" {
                        ("dataInput", "inputSet", "dataInputRefs", &ins, made_in)
                    } else {
                        ("dataOutput", "outputSet", "dataOutputRefs", &outs, made_out)
                    };
                let mut ids = Vec::new();
                for x in xs {
                    ids.push(x.base.id().to_string());
                    insert_ranked(el, item_aware(item, x));
                }
                for e in made {
                    ids.push(e.get("id").unwrap_or("").to_string());
                    insert_ranked(el, e);
                }
                if !ids.is_empty() {
                    let mut st = XElem::new(BPMN_NS, set);
                    st.set(
                        "id",
                        format!(
                            "{owner}-{}",
                            if item == "dataInput" {
                                "inputs"
                            } else {
                                "outputs"
                            }
                        ),
                    );
                    for i in &ids {
                        st.push(text_elem(refs, i));
                    }
                    insert_ranked(el, st);
                }
                for x in props {
                    insert_ranked(el, item_aware("property", x));
                }
                continue;
            }
            if !ins.is_empty() || !outs.is_empty() || !implicit.is_empty() {
                let mut io = XElem::new(BPMN_NS, "ioSpecification");
                io.set("id", format!("{owner}-io"));
                let mut in_ids = Vec::new();
                let mut out_ids = Vec::new();
                for x in &ins {
                    io.push(item_aware("dataInput", x));
                    in_ids.push(x.base.id().to_string());
                }
                for e in made_in {
                    in_ids.push(e.get("id").unwrap_or("").to_string());
                    io.push(e);
                }
                for x in &outs {
                    io.push(item_aware("dataOutput", x));
                    out_ids.push(x.base.id().to_string());
                }
                for e in made_out {
                    out_ids.push(e.get("id").unwrap_or("").to_string());
                    io.push(e);
                }
                let mut is = XElem::new(BPMN_NS, "inputSet");
                is.set("id", format!("{owner}-inputs"));
                for i in &in_ids {
                    is.push(text_elem("dataInputRefs", i));
                }
                io.push(is);
                let mut os = XElem::new(BPMN_NS, "outputSet");
                os.set("id", format!("{owner}-outputs"));
                for i in &out_ids {
                    os.push(text_elem("dataOutputRefs", i));
                }
                io.push(os);
                insert_ranked(el, io);
            }
            for x in props {
                insert_ranked(el, item_aware("property", x));
            }
        }
    }

    /// `performer` (`humanPerformer` on user and manual tasks) of activities with `by=`
    /// (documents of language 0.5).
    fn performers(&mut self) {
        let d = self.doc;
        if !d.lang_05() {
            return;
        }
        for n in d.nodes.values() {
            let Some(r) = n.base.attrs.get("by") else {
                continue;
            };
            if !d.resources.contains_key(r) || !is_activity(&n.base.kind) {
                continue;
            }
            let name = if n.base.kind == "user" || n.base.kind == "manual" {
                "humanPerformer"
            } else {
                "performer"
            };
            let mut p = XElem::new(BPMN_NS, name);
            p.set("id", format!("{}-performer", n.base.id()));
            p.push(text_elem("resourceRef", r));
            if let Some(path) = self.root.find_path(n.base.id())
                && let Some(el) = self.root.at_mut(&path)
            {
                insert_ranked(el, p);
            }
        }
    }

    /// The process (not sub-process) that contains a node.
    fn process_of(&self, n: &Node) -> Option<String> {
        let mut scope = n.scope.clone()?;
        while let Some(m) = self.doc.nodes.get(&scope) {
            scope = m.scope.clone()?;
        }
        Some(scope)
    }

    /// `supportedInterfaceRef` of processes whose tasks use operations.
    fn supported_interfaces(&mut self) {
        let d = self.doc;
        let mut refs: IndexMap<String, Vec<String>> = IndexMap::new();
        for n in d.nodes.values() {
            let Some(op) = n.base.attrs.get("operation") else {
                continue;
            };
            let Some(ifc) = d
                .interfaces
                .values()
                .find(|x| x.operations.iter().any(|o| o.id() == op))
            else {
                continue;
            };
            let Some(pid) = self.process_of(n) else {
                continue;
            };
            let list = refs.entry(pid).or_default();
            if !list.iter().any(|x| x == ifc.base.id()) {
                list.push(ifc.base.id().to_string());
            }
        }
        for (pid, list) in refs {
            if let Some(path) = self.root.find_path(&pid)
                && let Some(el) = self.root.at_mut(&path)
            {
                for i in list {
                    insert_ranked(el, text_elem("supportedInterfaceRef", &i));
                }
            }
        }
    }

    /// Root elements of language 0.5: item definitions (placed before the other
    /// declarations), and resources, correlation properties, interfaces and global tasks
    /// (placed after them). Correlation keys go into the collaboration.
    fn declarations_05(&mut self) -> (Vec<XElem>, Vec<XElem>) {
        let d = self.doc;
        let mut first = Vec::new();
        let mut last = Vec::new();
        for it in d.items.values() {
            let mut e = XElem::new(BPMN_NS, "itemDefinition");
            e.set("id", it.id().to_string());
            if let Some(l) = nonempty(it.label.as_deref()) {
                e.push(text_elem("documentation", l));
            }
            if let Some(s) = it.attr("structure") {
                e.set("structureRef", s);
            }
            if let Some(k) = it.attr("kind") {
                e.set(
                    "itemKind",
                    if k == "physical" {
                        "Physical"
                    } else {
                        "Information"
                    },
                );
            }
            if it.has("collection") {
                e.set("isCollection", "true");
            }
            if it.attr("structure").is_some_and(|s| s.starts_with("xsd:"))
                && !self.root.declare.contains(&XSD_NS)
            {
                self.root.declare.push(XSD_NS);
            }
            if let (Some((p, _)), Some(ns)) = (
                it.attr("structure").and_then(|s| s.split_once(':')),
                it.attr("namespace"),
            ) && !self.root.prefixed.iter().any(|(q, _)| q == p)
            {
                self.root.prefixed.push((p.to_string(), ns.to_string()));
            }
            first.push(e);
        }
        if d.lang_05() {
            for r in d.resources.values() {
                let mut e = XElem::new(BPMN_NS, "resource");
                e.set("id", r.id().to_string());
                e.set("name", nonempty(r.label.as_deref()).unwrap_or(r.id()));
                last.push(e);
            }
        }
        let mut props: IndexMap<String, XElem> = IndexMap::new();
        let mut keys = Vec::new();
        for k in d.keys.values() {
            let mut ke = XElem::new(BPMN_NS, "correlationKey");
            ke.set("id", k.base.id().to_string());
            if let Some(l) = &k.base.label {
                ke.set("name", l.clone());
            }
            let mut seen: Vec<String> = Vec::new();
            for p in &k.props {
                let name = p.id().to_string();
                let pid = correlation_property_id(&name);
                if !props.contains_key(&name) {
                    let mut e = XElem::new(BPMN_NS, "correlationProperty");
                    e.set("id", pid.clone());
                    e.set("name", name.clone());
                    props.insert(name.clone(), e);
                }
                if let Some(m) = p.attr("message") {
                    let mid = self.declare("message", m, None);
                    let mut rx = XElem::new(BPMN_NS, "correlationPropertyRetrievalExpression");
                    rx.set("messageRef", mid);
                    let mut mp = text_elem("messagePath", p.attr("path").unwrap_or(""));
                    mp.set_ns(XSI_NS, "type", "bpmn:tFormalExpression");
                    rx.push(mp);
                    if let Some(prop) = props.get_mut(&name) {
                        prop.push(rx);
                    }
                }
                if !seen.contains(&pid) {
                    ke.push(text_elem("correlationPropertyRef", &pid));
                    seen.push(pid);
                }
            }
            keys.push(ke);
        }
        last.extend(props.into_values());
        if !keys.is_empty() {
            let cid = d
                .attrs
                .get("collaboration")
                .cloned()
                .unwrap_or_else(|| "collaboration".into());
            if !d.pools.is_empty()
                && let Some(path) = self.root.find_path(&cid)
                && let Some(c) = self.root.at_mut(&path)
            {
                for k in keys {
                    c.push(k);
                }
            }
        }
        for x in d.interfaces.values() {
            let mut e = XElem::new(BPMN_NS, "interface");
            e.set("id", x.base.id().to_string());
            e.set(
                "name",
                nonempty(x.base.label.as_deref()).unwrap_or(x.base.id()),
            );
            documentation(&mut e, x.base.attr(DOC_KEY));
            for op in &x.operations {
                let mut o = XElem::new(BPMN_NS, "operation");
                o.set("id", op.id().to_string());
                o.set("name", nonempty(op.label.as_deref()).unwrap_or(op.id()));
                documentation(&mut o, op.attr(DOC_KEY));
                if let Some(m) = op.attr("in") {
                    let id = self.declare("message", m, None);
                    o.push(text_elem("inMessageRef", &id));
                }
                if let Some(m) = op.attr("out") {
                    let id = self.declare("message", m, None);
                    o.push(text_elem("outMessageRef", &id));
                }
                for err in op.values("error") {
                    let id = self.declare("error", err, None);
                    o.push(text_elem("errorRef", &id));
                }
                e.push(o);
            }
            last.push(e);
        }
        for g in d.globals.values() {
            let tag = g
                .attr("kind")
                .and_then(|k| GLOBAL_KINDS.iter().find(|(x, _)| *x == k))
                .map_or("globalTask", |(_, t)| *t);
            let mut e = XElem::new(BPMN_NS, tag);
            e.set("id", g.id().to_string());
            if let Some(l) = &g.label {
                e.set("name", l.clone());
            }
            documentation(&mut e, g.attr(DOC_KEY));
            last.push(e);
        }
        (first, last)
    }
}

/// Identifier of the participant that stands for pool `pool` in choreography
/// `choreography` (language 0.6).
pub fn choreography_participant_id(choreography: &str, pool: &str) -> String {
    format!("{choreography}-{pool}")
}

/// Identifier of the conversation link between a conversation node and a pool.
pub fn conversation_link_id(node: &str, pool: &str) -> String {
    format!("{node}-link-{pool}")
}

/// `loopType` of a choreography activity.
fn choreography_loop(b: &Base) -> Option<&'static str> {
    if b.has("loop") {
        Some("Standard")
    } else if b.has("parallel") {
        Some("MultiInstanceParallel")
    } else if b.has("sequential") {
        Some("MultiInstanceSequential")
    } else {
        None
    }
}

/// `participantMultiplicity` of a pool with `min=` or `max=` (language 0.6); the
/// minimum is 2 when only the maximum is given.
fn multiplicity(d: &Document, p: &Pool) -> Option<XElem> {
    if !d.lang_06() {
        return None;
    }
    let (min, max) = (p.base.attr("min"), p.base.attr("max"));
    if min.is_none() && max.is_none() {
        return None;
    }
    let mut m = XElem::new(BPMN_NS, "participantMultiplicity");
    m.set("minimum", min.unwrap_or("2").to_string());
    if let Some(x) = max {
        m.set("maximum", x.to_string());
    }
    Some(m)
}

fn ref_elem(name: &str, id: &str) -> XElem {
    text_elem(name, id)
}

impl Exporter<'_> {
    /// Conversation nodes and conversation links of the collaboration (language 0.6):
    /// the nodes before the correlation keys, the links last (`tCollaboration`).
    fn conversations(&mut self) {
        let d = self.doc;
        if d.conversations.is_empty() {
            return;
        }
        let Some(path) = self.collab.clone() else {
            return;
        };
        let mut nodes = Vec::new();
        let mut links = Vec::new();
        for (id, c) in d.conversations.iter().filter(|(_, c)| c.parent.is_none()) {
            nodes.push(self.conversation_node(id));
            for p in c.pools() {
                let mut l = XElem::new(BPMN_NS, "conversationLink");
                l.set("id", conversation_link_id(id, &p));
                l.set("sourceRef", p.clone());
                l.set("targetRef", id.clone());
                links.push(l);
            }
        }
        let collab = self.at(&path);
        let at = collab
            .children
            .iter()
            .position(|x| x.name == "correlationKey")
            .unwrap_or(collab.children.len());
        for (k, n) in nodes.into_iter().enumerate() {
            collab.insert(at + k, n);
        }
        for l in links {
            collab.push(l);
        }
    }

    fn conversation_node(&self, id: &str) -> XElem {
        let d = self.doc;
        let c = &d.conversations[id];
        let kind = c.base.kind.as_str();
        let tag = match kind {
            "subconversation" => "subConversation",
            "callconversation" => "callConversation",
            _ => "conversation",
        };
        let mut e = XElem::new(BPMN_NS, tag);
        e.set("id", id.to_string());
        if let Some(l) = &c.base.label {
            e.set("name", l.clone());
        }
        let called = c.base.attr("collaboration");
        if let Some(cc) = called {
            e.set("calledCollaborationRef", cc.to_string());
        }
        documentation(&mut e, c.base.attr(DOC_KEY));
        for p in c.pools() {
            e.push(ref_elem("participantRef", &p));
        }
        for m in words(c.base.attr("messages").unwrap_or("")) {
            e.push(ref_elem("messageFlowRef", &m));
        }
        for k in &c.children {
            e.push(self.conversation_node(k));
        }
        if kind == "callconversation"
            && let Some(text) = c.base.attr("map")
        {
            let inner_of = |i: &str| match called {
                Some(cc) if d.choreographies.contains_key(cc) => choreography_participant_id(cc, i),
                _ => i.to_string(),
            };
            for (i, o) in participant_map(text).unwrap_or_default() {
                let mut pa = XElem::new(BPMN_NS, "participantAssociation");
                pa.push(ref_elem("innerParticipantRef", &inner_of(&i)));
                pa.push(ref_elem("outerParticipantRef", &o));
                e.push(pa);
            }
        }
        e
    }

    /// Each choreography as a root element (language 0.6): its participants (the pools of
    /// its activities), the message flows of its tasks, then its flow elements.
    fn choreographies(&mut self) {
        let d = self.doc;
        for (cid, c) in &d.choreographies {
            let mut e = XElem::new(BPMN_NS, "choreography");
            e.set("id", cid.clone());
            if let Some(l) = &c.base.label {
                e.set("name", l.clone());
            }
            documentation(&mut e, c.base.attr(DOC_KEY));
            for p in crate::check::choreography_participants(d, cid) {
                let mut pe = XElem::new(BPMN_NS, "participant");
                pe.set("id", choreography_participant_id(cid, &p));
                if let Some(pool) = d.pool(&p) {
                    if let Some(l) = &pool.base.label {
                        pe.set("name", l.clone());
                    }
                    if let Some(m) = multiplicity(d, pool) {
                        pe.push(m);
                    }
                }
                e.push(pe);
            }
            for n in d
                .nodes
                .values()
                .filter(|n| n.choreography.as_deref() == Some(cid) && n.base.kind == "choreo")
            {
                let (from, to) = (
                    n.base.attr("from").unwrap_or(""),
                    n.base.attr("to").unwrap_or(""),
                );
                for (suffix, src, tgt, key) in [
                    ("message", from, to, "message"),
                    ("reply", to, from, "reply"),
                ] {
                    let name = n.base.attr(key);
                    if key == "reply" && name.is_none() {
                        continue;
                    }
                    let mut mf = XElem::new(BPMN_NS, "messageFlow");
                    mf.set("id", format!("{}-{suffix}", n.base.id()));
                    mf.set("sourceRef", choreography_participant_id(cid, src));
                    mf.set("targetRef", choreography_participant_id(cid, tgt));
                    if let Some(m) = name.filter(|m| !m.is_empty()) {
                        let mid = self.declare("message", m, None);
                        mf.set("messageRef", mid);
                    }
                    e.push(mf);
                }
            }
            let ci = self.root.push(e);
            self.containers.insert(cid.clone(), vec![ci]);
            self.choreography_block(vec![ci], cid, &c.children);
        }
    }

    fn choreography_block(&mut self, container: Path, cid: &str, ids: &[String]) {
        for i in ids {
            let Some(n) = self.doc.nodes.get(i) else {
                continue;
            };
            if !is_choreo(&n.base.kind) {
                self.node(container.clone(), n);
                continue;
            }
            let e = self.choreography_activity(cid, n);
            let k = self.at(&container).push(e);
            if n.base.kind == "subchoreo" {
                let mut path = container.clone();
                path.push(k);
                self.containers.insert(i.clone(), path.clone());
                let kids = n.children.clone();
                self.choreography_block(path, cid, &kids);
            }
        }
    }

    fn choreography_activity(&mut self, cid: &str, n: &Node) -> XElem {
        let b = &n.base;
        let kind = b.kind.as_str();
        let tag = match kind {
            "choreo" => "choreographyTask",
            "subchoreo" => "subChoreography",
            _ => "callChoreography",
        };
        let part = |p: &str| choreography_participant_id(cid, p);
        let mut e = XElem::new(BPMN_NS, tag);
        e.set("id", b.id().to_string());
        if let Some(l) = &b.label {
            e.set("name", l.clone());
        }
        if let Some(c) = b.attr("choreography").filter(|_| kind == "callchoreo") {
            e.set("calledChoreographyRef", c.to_string());
        }
        if let Some(f) = b.attr("from") {
            e.set("initiatingParticipantRef", part(f));
        }
        if let Some(lt) = choreography_loop(b) {
            e.set("loopType", lt);
        }
        documentation(&mut e, b.attr(DOC_KEY));
        let pools: Vec<String> = if kind == "choreo" {
            ["from", "to"]
                .iter()
                .filter_map(|k| b.attr(k).map(str::to_string))
                .collect()
        } else {
            words(b.attr("pools").unwrap_or(""))
        };
        for p in &pools {
            e.push(ref_elem("participantRef", &part(p)));
        }
        if kind == "choreo" {
            e.push(ref_elem("messageFlowRef", &format!("{}-message", b.id())));
            if b.attrs.contains_key("reply") {
                e.push(ref_elem("messageFlowRef", &format!("{}-reply", b.id())));
            }
        }
        if kind == "callchoreo"
            && let Some(text) = b.attr("map")
        {
            let called = b.attr("choreography").unwrap_or("");
            for (i, o) in participant_map(text).unwrap_or_default() {
                let mut pa = XElem::new(BPMN_NS, "participantAssociation");
                pa.push(ref_elem(
                    "innerParticipantRef",
                    &choreography_participant_id(called, &i),
                ));
                pa.push(ref_elem("outerParticipantRef", &part(&o)));
                e.push(pa);
            }
        }
        e
    }
}

fn compensation_id(activity_id: &str) -> String {
    format!("{activity_id}-compensate")
}

/// A boundary event with the flag `compensate` attached to the activity: it is associated
/// with the activity's handler (`compensate=`) instead of a generated one.
fn compensation_boundary<'d>(d: &'d Document, activity_id: &str) -> Option<&'d str> {
    d.nodes
        .values()
        .find(|b| {
            b.base.kind == "boundary"
                && b.base.attr("on") == Some(activity_id)
                && b.base.has("compensate")
        })
        .map(|b| b.base.id())
}

fn after_props(act: &XElem) -> usize {
    let mut pos = docs_len(act);
    for (k, c) in act.children.iter().enumerate() {
        if c.ns == BPMN_NS
            && matches!(
                c.name.as_str(),
                "extensionElements" | "property" | "dataInputAssociation" | "dataOutputAssociation"
            )
        {
            pos = k + 1;
        }
    }
    pos
}

/// The semantic BPMN tree (attributes in insertion order, no layout).
pub fn export_tree(doc: &Document, target: Option<&str>) -> XElem {
    Exporter::new(doc, target).run()
}

/// The exported tree serialized without indentation, as `ET.tostring(root)` would print it.
pub fn export_xml(doc: &Document, target: Option<&str>) -> String {
    let root = export_tree(doc, target);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}",
        root.to_string_root()
    )
}

/// Canonical semantic XML (attributes sorted, indented), used by the conformance corpus.
pub fn semantic_xml(doc: &Document, target: Option<&str>) -> String {
    let mut root = export_tree(doc, target);
    root.sort_attrs();
    root.indent(0);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}\n",
        root.to_string_root()
    )
}
