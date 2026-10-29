//! Import BPMN 2.0 XML into a tok [`Document`] (the inverse of the exporter).
//! Mirrors `bpmn_import.py`, including its warning texts.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use roxmltree::Node as X;

use crate::model::*;
use crate::parser::is_id;
use crate::pyutil::{ascii_fold, split_ws, strip};
use crate::xml::{BPMN_NS, XSD_NS, ZEEBE_NS};

/// An error that stops the import (not well-formed XML or not a BPMN document).
#[derive(Debug, Clone)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

fn kind_of_tag(t: &str) -> Option<&'static str> {
    Some(match t {
        "startEvent" => "start",
        "endEvent" => "end",
        "intermediateCatchEvent" => "catch",
        "intermediateThrowEvent" => "throw",
        "boundaryEvent" => "boundary",
        "task" => "task",
        "userTask" => "user",
        "serviceTask" => "service",
        "manualTask" => "manual",
        "scriptTask" => "script",
        "businessRuleTask" => "rule",
        "sendTask" => "send",
        "receiveTask" => "receive",
        "callActivity" => "call",
        "subProcess" => "sub",
        "exclusiveGateway" => "xor",
        "parallelGateway" => "and",
        "inclusiveGateway" => "or",
        "eventBasedGateway" => "eventgw",
        "adHocSubProcess" => "adhoc",
        "transaction" => "transaction",
        "complexGateway" => "complex",
        _ => return None,
    })
}

fn definition_key(t: &str) -> Option<&'static str> {
    Some(match t {
        "messageEventDefinition" => "message",
        "timerEventDefinition" => "timer",
        "signalEventDefinition" => "signal",
        "errorEventDefinition" => "error",
        "escalationEventDefinition" => "escalation",
        "conditionalEventDefinition" => "condition",
        "linkEventDefinition" => "link",
        "terminateEventDefinition" => "terminate",
        "compensateEventDefinition" => "compensate",
        "cancelEventDefinition" => "cancel",
        _ => return None,
    })
}

fn local<'a>(e: X<'a, '_>) -> &'a str {
    e.tag_name().name()
}

fn clark(e: X) -> String {
    match e.tag_name().namespace() {
        Some(ns) => format!("{{{ns}}}{}", e.tag_name().name()),
        None => e.tag_name().name().to_string(),
    }
}

fn is_bpmn(e: X) -> bool {
    e.tag_name().namespace() == Some(BPMN_NS)
}

fn is_b(e: X, name: &str) -> bool {
    e.is_element() && is_bpmn(e) && local(e) == name
}

fn children<'a, 'i>(e: X<'a, 'i>) -> impl Iterator<Item = X<'a, 'i>> {
    e.children().filter(|c| c.is_element())
}

fn find<'a, 'i>(e: X<'a, 'i>, name: &str) -> Option<X<'a, 'i>> {
    children(e).find(|c| is_b(*c, name))
}

fn findall<'a, 'i>(e: X<'a, 'i>, name: &'static str) -> impl Iterator<Item = X<'a, 'i>> {
    children(e).filter(move |c| is_b(*c, name))
}

/// First Camunda (`zeebe:`) child element with the local name `name`.
fn zfind<'a, 'i>(e: X<'a, 'i>, name: &str) -> Option<X<'a, 'i>> {
    children(e).find(|c| c.tag_name().namespace() == Some(ZEEBE_NS) && local(*c) == name)
}

/// `e.find(B + "extensionElements/" + Z + name)`
fn ext_find<'a, 'i>(e: X<'a, 'i>, name: &str) -> Option<X<'a, 'i>> {
    findall(e, "extensionElements").find_map(|x| zfind(x, name))
}

const REST_CONNECTOR_TYPE: &str = "io.camunda:http-json:1";

fn binding_name(v: &str) -> String {
    match v {
        "versionTag" => "version-tag".into(),
        other => other.to_string(),
    }
}

fn connector_input(target: &str) -> Option<&'static str> {
    Some(match target {
        "method" => "method",
        "url" => "url",
        "headers" => "headers-in",
        "queryParameters" => "query",
        "body" => "body",
        _ => return None,
    })
}

/// Text of an element as ElementTree reports it (text before the first child element).
fn text<'a>(e: X<'a, '_>) -> Option<&'a str> {
    e.text()
}

pub(crate) fn slug(text: &str) -> String {
    let t = text.replace('ł', "l").replace('Ł', "L");
    let t = ascii_fold(&t);
    let mut out = String::new();
    let mut in_run = false;
    for c in t.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    let t = out.trim_matches('_').to_string();
    let t = if t.is_empty() { "id".to_string() } else { t };
    match t.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => t,
        _ => format!("_{t}"),
    }
}

fn clean_label(text: Option<&str>) -> Option<String> {
    let t = split_ws(text?).join(" ");
    if t.is_empty() { None } else { Some(t) }
}

/// Documentation text: line ends normalized, trailing spaces of lines and surrounding
/// blank lines removed.
fn doc_text(e: X) -> String {
    let t: String = e
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    let t = t.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = t.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_string()
}

fn strip_feel(text: Option<&str>) -> String {
    let t = strip(text.unwrap_or(""));
    match t.strip_prefix('=') {
        Some(r) => strip(r).to_string(),
        None => t.to_string(),
    }
}

struct Importer<'a, 'i> {
    root: X<'a, 'i>,
    keep_ids: bool,
    doc: Document,
    warnings: Vec<String>,
    idmap: IndexMap<String, String>,
    used: HashSet<String>,
    by_id: IndexMap<String, X<'a, 'i>>,
    warned: HashSet<String>,
    /// compensation boundary event id -> (activity, handler, association id)
    compensation: IndexMap<String, (String, String, Option<String>)>,
    /// element ids skipped by `container` (generated compensation boundaries and associations)
    skip_ids: HashSet<String>,
    /// an association without id was skipped (Python: `None in skip_ids`)
    skip_none: bool,
    /// `documentation` elements read into `doc=` (the others are reported)
    docs_read: HashSet<roxmltree::NodeId>,
    /// pools with a process: (pool index, process element), for `process-name=`
    pool_processes: Vec<(usize, X<'a, 'i>)>,
    /// data inputs and outputs of activities that only serve one data association and
    /// carry nothing tok would not generate (tok writes them for `reads` and `writes`)
    implicit: HashSet<String>,
    /// data object -> number of its references
    object_refs: IndexMap<String, usize>,
    /// data objects that a data association names directly (not through a reference)
    direct_objects: HashSet<String>,
    /// participant of a choreography -> the pool that stands for it (language 0.6)
    choreo_pools: IndexMap<String, String>,
    /// identifier of the file -> the element declared with it first
    declared: HashMap<String, roxmltree::NodeId>,
}

mod lang06;

/// The imported document, the warnings and the identifiers of the BPMN file with the tok
/// identifiers that stand for them.
pub type Imported = (Document, Vec<String>, IndexMap<String, String>);

/// Read BPMN 2.0 XML text. Returns the document and warning strings `"id: explanation"`.
pub fn import_bpmn(xml: &str, keep_ids: bool) -> Result<(Document, Vec<String>), ImportError> {
    import_bpmn_mapped(xml, keep_ids).map(|(doc, warnings, _)| (doc, warnings))
}

/// Like [`import_bpmn`], with the map from identifiers of the BPMN file to tok identifiers
/// (for the layout sidecar of `tok import`).
pub fn import_bpmn_mapped(xml: &str, keep_ids: bool) -> Result<Imported, ImportError> {
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let tree = roxmltree::Document::parse_with_options(xml, opts)
        .map_err(|e| ImportError(e.to_string()))?;
    if let Some(e) = crate::xml::too_deep(&tree) {
        return Err(ImportError(e));
    }
    let root = tree.root_element();
    let mut by_id = IndexMap::new();
    for e in root.descendants().filter(|n| n.is_element()) {
        if let Some(id) = e.attribute("id").filter(|v| !v.is_empty()) {
            by_id.insert(id.to_string(), e);
        }
    }
    let imp = Importer {
        root,
        keep_ids,
        doc: Document::default(),
        warnings: Vec::new(),
        idmap: IndexMap::new(),
        used: HashSet::new(),
        by_id,
        warned: HashSet::new(),
        compensation: IndexMap::new(),
        skip_ids: HashSet::new(),
        skip_none: false,
        docs_read: HashSet::new(),
        pool_processes: Vec::new(),
        implicit: HashSet::new(),
        object_refs: IndexMap::new(),
        direct_objects: HashSet::new(),
        choreo_pools: IndexMap::new(),
        declared: HashMap::new(),
    };
    imp.run()
}

impl<'a, 'i> Importer<'a, 'i> {
    fn warn(&mut self, id: Option<&str>, text: &str, once: Option<String>) {
        if let Some(o) = once {
            if self.warned.contains(&o) {
                return;
            }
            self.warned.insert(o);
        }
        let id = id.filter(|i| !i.is_empty()).unwrap_or("-");
        self.warnings.push(format!("{id}: {text}"));
    }

    fn tid(&mut self, xml_id: Option<&str>) -> Option<String> {
        let x = xml_id.filter(|x| !x.is_empty())?;
        if let Some(v) = self.idmap.get(x) {
            return Some(v.clone());
        }
        let new0 = if is_id(x) { x.to_string() } else { slug(x) };
        let mut new = new0.clone();
        let mut n = 2;
        while self.used.contains(&new) {
            new = format!("{new0}-{n}");
            n += 1;
        }
        if new != x {
            let msg = format!("identifier changed to '{new}'");
            self.warn(Some(x), &msg, None);
        }
        self.idmap.insert(x.to_string(), new.clone());
        self.used.insert(new.clone());
        Some(new)
    }

    fn tid_s(&mut self, xml_id: Option<&str>) -> String {
        self.tid(xml_id).unwrap_or_default()
    }

    /// The tok identifier of an element the import declares. An element without an
    /// `id`, or with the `id` of an element declared before it, gets one of its own,
    /// named after its tag, so that two elements never share an identifier (a container
    /// sharing it with its content would contain itself).
    fn tid_of(&mut self, e: X<'a, 'i>) -> String {
        let xml_id = e.attribute("id").filter(|v| !v.is_empty());
        if let Some(x) = xml_id {
            match self.declared.get(x) {
                None => {
                    self.declared.insert(x.to_string(), e.id());
                    return self.tid_s(Some(x));
                }
                Some(first) if *first == e.id() => return self.tid_s(Some(x)),
                Some(_) => {}
            }
        }
        // a key no identifier of the file can have: XML text holds no NUL character
        let key = format!("\0{}", e.id().get_usize());
        if let Some(t) = self.idmap.get(&key) {
            return t.clone();
        }
        let tag = e.tag_name().name();
        let base = if is_id(tag) {
            tag.to_string()
        } else {
            slug(tag)
        };
        let mut new = base.clone();
        let mut n = 2;
        while self.used.contains(&new) {
            new = format!("{base}-{n}");
            n += 1;
        }
        let msg = match xml_id {
            Some(_) => format!("identifier used by an earlier element; this one is '{new}'"),
            None => format!("{tag} without an identifier is '{new}'"),
        };
        self.warn(xml_id, &msg, None);
        self.idmap.insert(key, new.clone());
        self.used.insert(new.clone());
        new
    }

    /// `doc=` from the first `documentation` of an element (language 0.5). Further
    /// documentation elements and text formats other than plain text are reported.
    fn doc(&mut self, e: X<'a, 'i>, attrs: &mut Attrs) {
        let docs: Vec<X> = findall(e, "documentation").collect();
        let Some(first) = docs.first().copied() else {
            return;
        };
        let id = e.attribute("id").map(str::to_string);
        let t = doc_text(first);
        if !t.is_empty() {
            attrs.insert(DOC_KEY.into(), t);
            self.uses_05();
        }
        self.docs_read.insert(first.id());
        if let Some(f) = first
            .attribute("textFormat")
            .filter(|f| !f.is_empty() && *f != "text/plain")
        {
            let msg = format!("documentation format {f} is not kept (doc= is plain text)");
            self.warn(id.as_deref(), &msg, None);
        }
        if docs.len() > 1 {
            self.warn(
                id.as_deref(),
                "only the first documentation element is kept",
                None,
            );
        }
    }

    fn reserve_ids(&mut self) {
        let keys: Vec<String> = self.by_id.keys().cloned().collect();
        for x in keys {
            if is_id(&x) {
                self.used.insert(x.clone());
                self.idmap.insert(x.clone(), x);
            }
        }
    }

    fn named(&mut self, r: &str, kind: &str) -> String {
        match self.by_id.get(r).copied() {
            None => {
                self.warn(Some(r), &format!("unknown {kind} reference"), None);
                r.to_string()
            }
            Some(e) => clean_label(e.attribute("name"))
                .unwrap_or_else(|| e.attribute("id").unwrap_or("").to_string()),
        }
    }

    fn uses_03(&mut self) {
        if !matches!(self.doc.version.as_deref(), Some("0.5" | "0.6")) {
            self.doc.version = Some("0.3".into());
        }
    }

    /// The document uses elements of language 0.5.
    fn uses_05(&mut self) {
        if self.doc.version.as_deref() != Some("0.6") {
            self.doc.version = Some("0.5".into());
        }
    }

    /// Compensation boundary events with an association to a handler: tok writes them as
    /// `compensate=` on the activity.
    fn compensation_boundaries(&mut self) {
        let assocs: Vec<X> = self
            .root
            .descendants()
            .filter(|e| is_b(*e, "association"))
            .collect();
        for a in assocs {
            let src = a
                .attribute("sourceRef")
                .and_then(|r| self.by_id.get(r))
                .copied();
            let tgt = a
                .attribute("targetRef")
                .and_then(|r| self.by_id.get(r))
                .copied();
            let (Some(src), Some(tgt)) = (src, tgt) else {
                continue;
            };
            let attached = src.attribute("attachedToRef").filter(|v| !v.is_empty());
            if local(src) == "boundaryEvent"
                && find(src, "compensateEventDefinition").is_some()
                && tgt.attribute("isForCompensation") == Some("true")
                && let Some(act) = attached
            {
                self.compensation.insert(
                    src.attribute("id").unwrap_or("").to_string(),
                    (
                        act.to_string(),
                        tgt.attribute("id").unwrap_or("").to_string(),
                        a.attribute("id").map(str::to_string),
                    ),
                );
            }
        }
        for (bid, (act, _, assoc)) in &self.compensation {
            // a boundary event with a name, documentation or its own identifier is kept
            // (`boundary … on=ACTIVITY compensate`); the exporter associates it with the handler
            let kept = self.by_id.get(bid.as_str()).is_some_and(|b| {
                clean_label(b.attribute("name")).is_some()
                    || find(*b, "documentation").is_some()
                    || (self.keep_ids && *bid != format!("{act}-compensate"))
            });
            if !kept {
                self.skip_ids.insert(bid.clone());
            }
            match assoc {
                Some(i) => {
                    self.skip_ids.insert(i.clone());
                }
                None => self.skip_none = true,
            }
        }
    }

    fn read_binding(&mut self, z: X, a: &mut Attrs) {
        if let Some(b) = z.attribute("bindingType").filter(|v| !v.is_empty()) {
            a.insert("binding".into(), binding_name(b));
            self.uses_03();
        }
        if let Some(v) = z.attribute("versionTag").filter(|v| !v.is_empty()) {
            a.insert("version".into(), v.to_string());
            self.uses_03();
        }
    }

    /// Data inputs, outputs and properties of activities that tok writes itself for `reads`
    /// and `writes`: a property (Camunda Modeler's `__targetRef_placeholder`), or a data
    /// input or output used by one association with one data element, named after it and
    /// with nothing else (and, when identifiers are kept, with the identifier tok gives it).
    fn find_implicit(&mut self) {
        let owners: Vec<X> = self
            .root
            .descendants()
            .filter(|e| {
                is_bpmn(*e)
                    && children(*e).any(|c| {
                        is_b(c, "dataInputAssociation") || is_b(c, "dataOutputAssociation")
                    })
            })
            .collect();
        for e in owners {
            let ends = inner_ends(e);
            let owner = e.attribute("id").unwrap_or("");
            for (id, a) in &ends {
                let Some(x) = self.by_id.get(id.as_str()).copied() else {
                    continue;
                };
                if local(x) == "property" {
                    self.implicit.insert(id.clone());
                    continue;
                }
                let uses = ends.iter().filter(|(i, _)| i == id).count();
                let outer = outer_ends(*a);
                let plain = x.attribute("itemSubjectRef").is_none_or(str::is_empty)
                    && x.attribute("isCollection") != Some("true")
                    && find(x, "documentation").is_none()
                    && find(x, "extensionElements").is_none();
                let named = match outer.as_slice() {
                    [d] => {
                        let label = self.by_id.get(*d).and_then(|de| {
                            clean_label(de.attribute("name")).or_else(|| {
                                de.attribute("dataObjectRef")
                                    .and_then(|r| self.by_id.get(r))
                                    .and_then(|o| clean_label(o.attribute("name")))
                            })
                        });
                        clean_label(x.attribute("name")) == Some(label.unwrap_or(d.to_string()))
                    }
                    _ => false,
                };
                let id_ok = !self.keep_ids || generated_io_id(owner, id);
                if uses == 1 && plain && named && id_ok {
                    self.implicit.insert(id.clone());
                }
            }
        }
    }

    /// References of data objects, and data objects named directly by data associations.
    fn find_objects(&mut self) {
        for e in self.root.descendants().filter(|e| is_bpmn(*e)) {
            match local(e) {
                "dataObject" => {
                    if let Some(i) = e.attribute("id") {
                        self.object_refs.entry(i.to_string()).or_insert(0);
                    }
                }
                "dataObjectReference" => {
                    if let Some(r) = e.attribute("dataObjectRef") {
                        *self.object_refs.entry(r.to_string()).or_insert(0) += 1;
                    }
                }
                "sourceRef" | "targetRef"
                    if e.parent_element().is_some_and(|a| {
                        is_b(a, "dataInputAssociation") || is_b(a, "dataOutputAssociation")
                    }) =>
                {
                    if let Some(t) = text(e).map(strip)
                        && self.by_id.get(t).is_some_and(|x| local(*x) == "dataObject")
                    {
                        self.direct_objects.insert(t.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn run(mut self) -> Result<Imported, ImportError> {
        let root = self.root;
        self.compensation_boundaries();
        self.find_implicit();
        self.find_objects();
        if !is_b(root, "definitions") {
            return Err(ImportError(format!(
                "not a BPMN 2.0 document: root element is {}",
                clark(root)
            )));
        }
        self.reserve_ids();
        self.doc.version = Some("0.1".into());
        self.doc.title = clean_label(root.attribute("name"));
        if let Some(id) = root
            .attribute("id")
            .filter(|i| !i.is_empty() && *i != "definitions")
        {
            let t = self.tid_s(Some(id));
            self.doc.attrs.insert("id".into(), t);
        }
        if let Some(ns) = root
            .attribute("targetNamespace")
            .filter(|n| !n.is_empty() && *n != "http://bpmn.io/schema/bpmn")
        {
            self.doc.attrs.insert("namespace".into(), ns.to_string());
        }
        let processes: Vec<X> = findall(root, "process").collect();
        let collabs: Vec<X> = findall(root, "collaboration").collect();
        for e in children(root) {
            let t = local(e);
            if ["globalChoreographyTask", "globalConversation"].contains(&t) {
                self.warn(
                    e.attribute("id"),
                    &format!("{t} is not supported and was skipped"),
                    None,
                );
            }
        }
        for c in collabs.iter().skip(1) {
            self.warn(
                c.attribute("id"),
                "collaboration was merged into the first one (tok has one collaboration)",
                None,
            );
        }
        let collab = collabs.first().copied();
        if let Some(c) = collab
            && c.attribute("name").is_some_and(|n| !n.trim().is_empty())
        {
            self.warn(
                c.attribute("id"),
                "the name of the collaboration is not kept",
                None,
            );
        }
        for (a, default) in [
            ("expressionLanguage", "http://www.w3.org/1999/XPath"),
            ("typeLanguage", XSD_NS),
        ] {
            if let Some(v) = root.attribute(a).filter(|v| !v.is_empty() && *v != default) {
                let msg = format!(
                    "{a} {v} is not kept (tok writes FEEL expressions and XML Schema types)"
                );
                self.warn(root.attribute("id"), &msg, None);
            }
        }
        for imp in findall(root, "import") {
            let msg = format!(
                "import of {} ({}) is not kept",
                imp.attribute("location").unwrap_or(""),
                imp.attribute("namespace").unwrap_or("")
            );
            self.warn(None, &msg, None);
        }
        if let Some(c) = collab
            && c.attribute("id") != Some("collaboration")
        {
            let t = self.tid_of(c);
            self.doc.attrs.insert("collaboration".into(), t);
        }
        let n_exec = processes
            .iter()
            .filter(|p| p.attribute("isExecutable") == Some("true"))
            .count();
        let all_exec = !processes.is_empty() && n_exec == processes.len();
        if all_exec {
            self.doc.flags.push("executable".into());
        }
        let mut proc_by_id: IndexMap<Option<&str>, X> = IndexMap::new();
        for p in &processes {
            proc_by_id.insert(p.attribute("id"), *p);
        }
        let single = (collab.is_none() && processes.len() == 1).then(|| processes[0]);
        if collab.is_none() && processes.len() == 1 {
            let p = processes[0];
            if p.attribute("id") != Some("process") {
                let t = self.tid_of(p);
                self.doc.attrs.insert("process".into(), t);
            }
            if self.doc.title.is_none() {
                self.doc.title = clean_label(p.attribute("name"));
            }
            self.process(p, None);
        } else {
            let mut referenced: HashSet<&str> = HashSet::new();
            let participants: Vec<X> = collabs
                .iter()
                .flat_map(|c| findall(*c, "participant"))
                .collect();
            for part in participants {
                let pid = self.tid_of(part);
                let mut attrs = Attrs::new();
                self.doc(part, &mut attrs);
                self.multiplicity(part, &mut attrs);
                let base = Base::new(
                    "pool",
                    Some(pid.clone()),
                    clean_label(part.attribute("name")),
                    attrs,
                    Vec::new(),
                    0,
                );
                self.doc.pools.push(Pool {
                    base,
                    blackbox: true,
                    children: Vec::new(),
                });
                self.doc.ids.insert(
                    pid.clone(),
                    IdEntry {
                        kind: "pool".into(),
                        line: 0,
                    },
                );
                let r = part.attribute("processRef");
                if let Some(r) = r.filter(|r| !r.is_empty()) {
                    if let Some(p) = proc_by_id.get(&Some(r)).copied() {
                        referenced.insert(r);
                        let idx = self.doc.pools.len() - 1;
                        self.attach_process(idx, p, all_exec);
                    } else {
                        let msg = format!("process '{r}' not found; imported as a black-box pool");
                        self.warn(part.attribute("id"), &msg, None);
                    }
                }
            }
            for p in &processes {
                if p.attribute("id")
                    .map(|i| referenced.contains(i))
                    .unwrap_or(false)
                {
                    continue;
                }
                let pid = self.tid_of(*p);
                let mut pool_id = format!("{pid}-pool");
                while self.used.contains(&pool_id) {
                    pool_id.push('_');
                }
                self.used.insert(pool_id.clone());
                let label = clean_label(p.attribute("name")).or_else(|| Some(pid.clone()));
                let base = Base::new(
                    "pool",
                    Some(pool_id.clone()),
                    label,
                    Attrs::new(),
                    Vec::new(),
                    0,
                );
                self.doc.pools.push(Pool {
                    base,
                    blackbox: true,
                    children: Vec::new(),
                });
                self.doc.ids.insert(
                    pool_id.clone(),
                    IdEntry {
                        kind: "pool".into(),
                        line: 0,
                    },
                );
                let idx = self.doc.pools.len() - 1;
                self.attach_process(idx, *p, all_exec);
                let msg = format!("process without a participant was placed in pool '{pool_id}'");
                self.warn(p.attribute("id"), &msg, None);
            }
        }
        let inner: Vec<usize> = self
            .doc
            .pools
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.blackbox)
            .map(|(i, _)| i)
            .collect();
        if self.doc.title.is_none() && inner.len() == 1 {
            let target = self.doc.pools[inner[0]].process_id();
            let mut proc = proc_by_id.get(&Some(target.as_str())).copied();
            if proc.is_none() {
                for p in &processes {
                    if self.tid_of(*p) == target {
                        proc = Some(*p);
                        break;
                    }
                }
            }
            if let Some(p) = proc
                && let Some(l) = clean_label(p.attribute("name"))
            {
                self.doc.title = Some(l);
            }
        }
        for (k, c) in collabs.iter().enumerate() {
            let mfs: Vec<X> = findall(*c, "messageFlow").collect();
            for mf in mfs {
                self.message_flow(mf);
            }
            for e in children(*c) {
                let t = local(e);
                if [
                    "participantAssociation",
                    "conversationAssociation",
                    "messageFlowAssociation",
                    "choreography",
                ]
                .contains(&t)
                    || (t == "correlationKey" && k > 0)
                {
                    self.warn(
                        e.attribute("id"),
                        &format!("{t} is not supported and was skipped"),
                        None,
                    );
                }
            }
        }
        self.choreography_participants();
        self.conversations(&collabs);
        self.choreographies();
        self.process_names(single);
        if !self.doc.pools.is_empty() {
            // the header documents the collaboration of a document with pools; files written
            // by tok 1.0.1 and older put it into the definitions
            let holder = collab
                .filter(|c| find(*c, "documentation").is_some())
                .unwrap_or(root);
            let mut attrs = std::mem::take(&mut self.doc.attrs);
            self.doc(holder, &mut attrs);
            self.doc.attrs = attrs;
        }
        self.flows_and_artifacts();
        self.declarations_05(collab);
        self.unreferenced();
        self.sweep();
        self.derive_flow_ids();
        Ok((self.doc, self.warnings, self.idmap))
    }

    fn attach_process(&mut self, pool_idx: usize, p: X<'a, 'i>, all_exec: bool) {
        self.doc.pools[pool_idx].blackbox = false;
        let pool_id = self.doc.pools[pool_idx].base.id().to_string();
        if p.attribute("id") != Some(format!("{pool_id}-process").as_str()) {
            let t = self.tid_of(p);
            self.doc.pools[pool_idx]
                .base
                .attrs
                .insert("process".into(), t);
        }
        if p.attribute("isExecutable") == Some("true") && !all_exec {
            self.doc.pools[pool_idx]
                .base
                .flags
                .push("executable".into());
        }
        self.pool_processes.push((pool_idx, p));
        self.process(p, Some(pool_idx));
    }

    /// `process-name=` where the process name differs from the one tok derives: the title
    /// in a document without pools or with one pool with a process, otherwise the label of
    /// the pool (language 0.5; an empty value means a process without a name).
    fn process_names(&mut self, single: Option<X<'a, 'i>>) {
        let title = self.doc.title.clone().filter(|t| !t.is_empty());
        if let Some(p) = single {
            let name = clean_label(p.attribute("name"));
            if name != title {
                self.doc
                    .attrs
                    .insert("process-name".into(), name.unwrap_or_default());
                self.uses_05();
            }
            return;
        }
        let inner = self.pool_processes.len();
        for (i, p) in self.pool_processes.clone() {
            let name = clean_label(p.attribute("name"));
            let pool = &self.doc.pools[i];
            let derived = if inner == 1 && title.is_some() {
                title.clone()
            } else {
                pool.base.label.clone()
            };
            if name != derived {
                self.doc.pools[i]
                    .base
                    .attrs
                    .insert("process-name".into(), name.unwrap_or_default());
                self.uses_05();
            }
        }
    }

    /// Attributes of a process that tok does not keep.
    fn process_attributes(&mut self, p: X<'a, 'i>) {
        let id = p.attribute("id");
        if let Some(t) = p
            .attribute("processType")
            .filter(|t| !t.is_empty() && *t != "None")
        {
            self.warn(id, &format!("processType=\"{t}\" is not kept"), None);
        }
        if p.attribute("isClosed") == Some("true") {
            self.warn(id, "isClosed=\"true\" is not kept", None);
        }
        if children(p).any(|c| {
            is_bpmn(c)
                && matches!(
                    local(c),
                    "performer" | "humanPerformer" | "potentialOwner" | "resourceRole"
                )
        }) {
            self.warn(id, "performers of a process are not kept", None);
        }
    }

    fn process(&mut self, p: X<'a, 'i>, pool: Option<usize>) {
        if let Some(vt) = ext_find(p, "versionTag")
            && let Some(v) = vt.attribute("value").filter(|v| !v.is_empty())
        {
            match pool {
                Some(i) => {
                    self.doc.pools[i]
                        .base
                        .attrs
                        .insert("version-tag".into(), v.to_string());
                }
                None => {
                    self.doc.attrs.insert("version-tag".into(), v.to_string());
                }
            }
            self.uses_03();
        }
        self.process_attributes(p);
        if pool.is_none() {
            // the header documents the process of a document without pools
            let mut attrs = std::mem::take(&mut self.doc.attrs);
            self.doc(p, &mut attrs);
            self.doc.attrs = attrs;
        }
        let scope = match pool {
            Some(i) => self.doc.pools[i].process_id(),
            None => self.doc.default_process_id(),
        };
        let pool_id = pool.map(|i| self.doc.pools[i].base.id().to_string());
        let mut lane_of: IndexMap<String, String> = IndexMap::new();
        let mut lanes = Vec::new();
        let sets: Vec<X> = findall(p, "laneSet").collect();
        for ls in sets {
            let ls_lanes: Vec<X> = findall(ls, "lane").collect();
            for lane in ls_lanes {
                lanes.push(self.lane(lane, None, pool_id.as_deref(), &mut lane_of));
            }
        }
        let order = self.container(p, &scope, pool_id.as_deref(), None);
        let Some(pi) = pool else {
            self.doc.top.extend(order.children);
            return;
        };
        if lanes.is_empty() {
            let pool = &mut self.doc.pools[pi];
            pool.children.extend(order.children);
            pool.children.extend(order.data);
            return;
        }
        let pos: IndexMap<String, usize> = order
            .nodes
            .iter()
            .enumerate()
            .map(|(k, n)| (n.clone(), k))
            .collect();
        let mut outside = Vec::new();
        for nid in &order.nodes {
            match lane_of.get(nid).cloned() {
                None => outside.push(nid.clone()),
                Some(lid) => {
                    if let Some(n) = self.doc.nodes.get_mut(nid) {
                        n.lane = Some(lid.clone());
                    }
                    if let Some(l) = self.doc.lanes.get_mut(&lid) {
                        l.children.push(nid.clone());
                    }
                }
            }
        }
        self.sort_lanes(&lanes, &pos);
        let merged = self.merge(&lanes, &[], &pos);
        self.doc.pools[pi].children.extend(merged);
        for nid in outside {
            self.doc.pools[pi].children.push(nid.clone());
            self.warn(
                Some(&nid),
                "node is in a pool with lanes but in no lane",
                None,
            );
        }
        self.doc.pools[pi].children.extend(order.data);
    }

    fn lane(
        &mut self,
        e: X<'a, 'i>,
        parent: Option<&str>,
        pool: Option<&str>,
        lane_of: &mut IndexMap<String, String>,
    ) -> String {
        let lid = self.tid_of(e);
        let mut attrs = Attrs::new();
        self.doc(e, &mut attrs);
        let base = Base::new(
            "lane",
            Some(lid.clone()),
            clean_label(e.attribute("name")),
            attrs,
            Vec::new(),
            0,
        );
        self.doc.lanes.insert(
            lid.clone(),
            Lane {
                base,
                pool: pool.map(str::to_string),
                parent: parent.map(str::to_string),
                children: Vec::new(),
                sub: Vec::new(),
            },
        );
        self.doc.ids.insert(
            lid.clone(),
            IdEntry {
                kind: "lane".into(),
                line: 0,
            },
        );
        let refs: Vec<X> = findall(e, "flowNodeRef").collect();
        for r in refs {
            let xid = strip(text(r).unwrap_or("")).to_string();
            if !xid.is_empty() {
                let t = self.tid_s(Some(&xid));
                lane_of.insert(t, lid.clone());
            }
        }
        let sets: Vec<X> = findall(e, "childLaneSet").collect();
        for cls in sets {
            let subs: Vec<X> = findall(cls, "lane").collect();
            for c in subs {
                let sid = self.lane(c, Some(&lid), pool, lane_of);
                if let Some(l) = self.doc.lanes.get_mut(&lid) {
                    l.sub.push(sid);
                }
            }
        }
        lid
    }

    fn sort_lanes(&mut self, lanes: &[String], pos: &IndexMap<String, usize>) {
        for lid in lanes {
            let sub = self
                .doc
                .lanes
                .get(lid)
                .map(|l| l.sub.clone())
                .unwrap_or_default();
            self.sort_lanes(&sub, pos);
            let nodes: Vec<String> = self
                .doc
                .lanes
                .get(lid)
                .map(|l| {
                    l.children
                        .iter()
                        .filter(|c| self.doc.nodes.contains_key(*c))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let merged = self.merge(&sub, &nodes, pos);
            if let Some(l) = self.doc.lanes.get_mut(lid) {
                l.children = merged;
            }
        }
    }

    fn first_pos(&self, lid: &str, pos: &IndexMap<String, usize>) -> Option<usize> {
        let mut best: Option<usize> = None;
        let lane = self.doc.lanes.get(lid)?;
        for c in &lane.children {
            let k = if self.doc.nodes.contains_key(c) {
                pos.get(c).copied()
            } else {
                self.first_pos(c, pos)
            };
            if let Some(k) = k
                && best.map(|b| k < b).unwrap_or(true)
            {
                best = Some(k);
            }
        }
        best
    }

    fn merge(
        &self,
        lanes: &[String],
        nodes: &[String],
        pos: &IndexMap<String, usize>,
    ) -> Vec<String> {
        let mut items: Vec<(f64, u8, usize, String)> = Vec::new();
        let mut last = -1.0f64;
        for (k, lid) in lanes.iter().enumerate() {
            let p = self.first_pos(lid, pos);
            let key = match p {
                Some(p) => p as f64,
                None => last + 0.001,
            };
            last = key;
            items.push((key, 1, k, lid.clone()));
        }
        for nid in nodes {
            items.push((
                pos.get(nid).map(|p| *p as f64).unwrap_or(1e9),
                0,
                0,
                nid.clone(),
            ));
        }
        items.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        items.into_iter().map(|x| x.3).collect()
    }

    fn container(
        &mut self,
        e: X<'a, 'i>,
        scope: &str,
        pool: Option<&str>,
        sub: Option<&str>,
    ) -> Order {
        let mut order = Order::default();
        let kids: Vec<X> = children(e).collect();
        for c in kids {
            if !is_bpmn(c) {
                continue;
            }
            let skip = match c.attribute("id") {
                Some(i) => self.skip_ids.contains(i),
                None => self.skip_none,
            };
            if skip {
                continue;
            }
            let t = local(c);
            if kind_of_tag(t).is_some() {
                let nid = self.node(c, t, scope, pool);
                order.children.push(nid.clone());
                order.nodes.push(nid.clone());
                let block = sub.or(pool).map(str::to_string);
                for did in self.own_data(c, &nid, scope, block) {
                    order.data.push(did);
                }
                let is_sub_kind = self
                    .doc
                    .nodes
                    .get(&nid)
                    .map(|n| is_sub(&n.base.kind))
                    .unwrap_or(false);
                if is_sub_kind {
                    let inner = self.container(c, &nid, pool, Some(&nid));
                    if let Some(n) = self.doc.nodes.get_mut(&nid) {
                        n.children = inner.children.into_iter().chain(inner.data).collect();
                    }
                }
            } else if ["choreographyTask", "subChoreography", "callChoreography"].contains(&t) {
                self.warn(
                    c.attribute("id"),
                    &format!("{t} is not supported in tok and was skipped"),
                    None,
                );
            } else if t == "dataObjectReference" || t == "dataStoreReference" {
                let block = sub.or(pool).map(str::to_string);
                let did = self.data(c, t, scope, block);
                order.data.push(did);
            } else if t == "dataObject" {
                let id = c.attribute("id").unwrap_or("");
                let refs = self.object_refs.get(id).copied().unwrap_or(0);
                if refs == 0 || self.direct_objects.contains(id) {
                    // tok declares data objects through their references (`data`)
                    let block = sub.or(pool).map(str::to_string);
                    let did = self.data(c, t, scope, block);
                    order.data.push(did);
                    self.warn(
                        Some(id),
                        "data object without a reference (or named directly by a data association) was imported as a data object reference",
                        None,
                    );
                }
            } else if t == "laneSet" && sub.is_some() {
                self.warn(
                    c.attribute("id"),
                    "lanes inside a sub-process are not supported and were ignored",
                    None,
                );
            } else if t == "ioSpecification" || t == "property" {
                // data inputs, outputs and properties of the process or sub-process itself
                let block = sub.or(pool).map(str::to_string);
                if t == "property" {
                    if let Some(did) = self.io_data(c, "property", scope, block, None) {
                        order.data.push(did);
                    }
                } else {
                    self.io_sets(c);
                    for (x, kind) in io_items(c) {
                        if x.attribute("id").is_some_and(|i| self.implicit.contains(i)) {
                            continue;
                        }
                        if let Some(did) = self.io_data(x, kind, scope, block.clone(), None) {
                            order.data.push(did);
                        }
                    }
                }
            }
        }
        if sub.is_none() && pool.is_none() {
            let data = order.data.clone();
            order.children.extend(data);
        }
        order
    }

    fn node(&mut self, e: X<'a, 'i>, t: &str, scope: &str, pool: Option<&str>) -> String {
        let mut kind = kind_of_tag(t).unwrap_or("task");
        if t == "subProcess" && e.attribute("triggeredByEvent") == Some("true") {
            kind = "eventsub";
        }
        let nid = self.tid_of(e);
        let mut n = Node {
            base: Base::new(
                kind,
                Some(nid.clone()),
                clean_label(e.attribute("name")),
                Attrs::new(),
                Vec::new(),
                0,
            ),
            scope: Some(scope.to_string()),
            lane: None,
            pool: pool.map(str::to_string),
            children: Vec::new(),
            choreography: None,
        };
        if kind == "boundary" {
            let on = self
                .tid(e.attribute("attachedToRef"))
                .unwrap_or_else(|| "None".into());
            n.base.attrs.insert("on".into(), on);
            if e.attribute("cancelActivity") == Some("false") {
                n.base.flags.push("noninterrupting".into());
            }
        }
        if kind == "start" && e.attribute("isInterrupting") == Some("false") {
            n.base.flags.push("noninterrupting".into());
        }
        if kind == "call"
            && let Some(ce) = e.attribute("calledElement").filter(|c| !c.is_empty())
        {
            let global = self
                .by_id
                .get(ce)
                .is_some_and(|g| GLOBAL_KINDS.iter().any(|(_, t)| *t == local(*g)));
            if global {
                let t = self.tid_s(Some(ce));
                n.base.attrs.insert("task".into(), t);
                self.uses_05();
            } else {
                n.base.attrs.insert("process".into(), ce.to_string());
            }
        }
        self.node_05(e, &mut n);
        if kind == "adhoc" {
            self.uses_03();
            if e.attribute("ordering") == Some("Sequential") {
                n.base.flags.push("sequential".into());
            }
            if e.attribute("cancelRemainingInstances") == Some("false") {
                n.base.flags.push("wait".into());
            }
            if let Some(cc) = find(e, "completionCondition")
                && !strip(text(cc).unwrap_or("")).is_empty()
            {
                n.base.attrs.insert("until".into(), strip_feel(text(cc)));
            }
        }
        let own_id = e.attribute("id").unwrap_or("");
        let handlers: Vec<String> = self
            .compensation
            .values()
            .filter(|(act, _, _)| act == own_id)
            .map(|(_, h, _)| h.clone())
            .collect();
        for h in handlers {
            let t = self.tid_s(Some(&h));
            n.base.attrs.insert("compensate".into(), t);
            self.uses_03();
        }
        if e.attribute("isForCompensation") == Some("true") {
            n.base.flags.push("compensation".into());
        }
        if (kind == "send" || kind == "receive")
            && let Some(r) = e.attribute("messageRef").filter(|r| !r.is_empty())
        {
            let name = self.named(r, "message");
            n.base.attrs.insert("message".into(), name.clone());
            if kind == "receive" && self.correlation(r).is_some() {
                let msg = format!(
                    "correlation key of a receive task cannot be expressed in tok; use a message catch event (message '{name}')"
                );
                self.warn(Some(&nid), &msg, None);
            }
        }
        if kind == "script"
            && let Some(s) = find(e, "script")
            && !strip(text(s).unwrap_or("")).is_empty()
        {
            n.base.attrs.insert("expr".into(), strip_feel(text(s)));
        }
        if is_event(kind) {
            self.event_definitions(e, &mut n);
        }
        if let Some(lc) = find(e, "standardLoopCharacteristics") {
            n.base.flags.push("loop".into());
            if lc.attribute("testBefore") == Some("true") {
                n.base.flags.push("test-before".into());
                self.uses_05();
            }
            if let Some(m) = lc.attribute("loopMaximum").filter(|v| !v.is_empty()) {
                n.base.attrs.insert("loop-max".into(), m.to_string());
                self.uses_05();
            }
            if let Some(c) = find(lc, "loopCondition")
                && !strip(text(c).unwrap_or("")).is_empty()
            {
                n.base.attrs.insert("while".into(), strip_feel(text(c)));
                self.uses_05();
            }
        }
        if let Some(mi) = find(e, "multiInstanceLoopCharacteristics") {
            n.base.flags.push(
                if mi.attribute("isSequential") == Some("true") {
                    "sequential"
                } else {
                    "parallel"
                }
                .into(),
            );
            if let Some(lc) = ext_find(mi, "loopCharacteristics") {
                self.uses_03();
                let a = &mut n.base.attrs;
                let get = |name: &str| lc.attribute(name).filter(|v| !v.is_empty());
                if let Some(v) = get("inputCollection") {
                    a.insert("collection".into(), strip_feel(Some(v)));
                }
                if let Some(v) = get("inputElement") {
                    a.insert("item".into(), v.to_string());
                }
                if let Some(v) = get("outputCollection") {
                    a.insert("output".into(), v.to_string());
                }
                if let Some(v) = get("outputElement") {
                    a.insert("output-item".into(), strip_feel(Some(v)));
                }
            }
            if let Some(cc) = find(mi, "completionCondition")
                && !strip(text(cc).unwrap_or("")).is_empty()
            {
                n.base.attrs.insert("until".into(), strip_feel(text(cc)));
                self.uses_03();
            }
        }
        self.extensions(e, &mut n);
        let mut attrs = std::mem::take(&mut n.base.attrs);
        self.doc(e, &mut attrs);
        n.base.attrs = attrs;
        self.unkept_attributes(e, kind);
        self.doc.ids.insert(
            nid.clone(),
            IdEntry {
                kind: kind.to_string(),
                line: 0,
            },
        );
        self.doc.nodes.insert(nid.clone(), n);
        nid
    }

    /// The process element that contains `e` executes (`isExecutable="true"`).
    fn in_executable_process(e: X) -> bool {
        e.ancestors()
            .find(|a| is_b(*a, "process"))
            .is_some_and(|p| p.attribute("isExecutable") == Some("true"))
    }

    /// Attributes and parts of a flow node that tok does not keep.
    fn unkept_attributes(&mut self, e: X<'a, 'i>, kind: &str) {
        let id = e.attribute("id");
        let t = local(e);
        let mut lost: Vec<String> = Vec::new();
        for a in ["startQuantity", "completionQuantity"] {
            if let Some(v) = e.attribute(a).filter(|v| !v.is_empty() && *v != "1") {
                lost.push(format!("{a}=\"{v}\""));
            }
        }
        let implementation_default = match t {
            "serviceTask" | "sendTask" | "receiveTask" => Some("##WebService"),
            "userTask" | "businessRuleTask" => Some("##unspecified"),
            _ => None,
        };
        if let Some(v) = e
            .attribute("implementation")
            .filter(|v| !v.is_empty() && Some(*v) != implementation_default)
        {
            lost.push(format!("implementation=\"{v}\""));
        }
        if t == "scriptTask"
            && let Some(f) = e.attribute("scriptFormat").filter(|f| !f.is_empty())
        {
            lost.push(format!("scriptFormat=\"{f}\""));
        }
        if let Some(mi) = find(e, "multiInstanceLoopCharacteristics") {
            for part in [
                "loopCardinality",
                "loopDataInputRef",
                "loopDataOutputRef",
                "inputDataItem",
                "outputDataItem",
                "complexBehaviorDefinition",
            ] {
                if find(mi, part).is_some() {
                    lost.push(format!("multi-instance {part}"));
                }
            }
            if let Some(b) = mi.attribute("behavior").filter(|b| *b != "All") {
                lost.push(format!("multi-instance behavior=\"{b}\""));
            }
            if find(mi, "completionCondition").is_some() && !Self::in_executable_process(e) {
                lost.push("the completion condition of a multi-instance activity (exported only in executable processes)".into());
            }
        }
        let roles: Vec<X> = children(e)
            .filter(|c| {
                is_bpmn(*c)
                    && matches!(
                        local(*c),
                        "performer" | "humanPerformer" | "potentialOwner" | "resourceRole"
                    )
            })
            .collect();
        for (k, r) in roles.iter().enumerate() {
            let role = local(*r);
            let exported = if kind == "user" || kind == "manual" {
                "humanPerformer"
            } else {
                "performer"
            };
            if k > 0 {
                lost.push(format!("{role} (only the first performer is kept)"));
            } else if find(*r, "resourceRef").is_none() {
                lost.push(format!("{role} without a resource reference"));
            } else if role != exported {
                lost.push(format!("{role} (kept as by=, exported as {exported})"));
            }
            for part in ["resourceAssignmentExpression", "resourceParameterBinding"] {
                if find(*r, part).is_some() {
                    lost.push(format!("{part} of {role}"));
                }
            }
        }
        for d in children(e).filter(|c| is_bpmn(*c) && definition_key(local(*c)).is_some()) {
            if local(d) == "messageEventDefinition" && d.attribute("operationRef").is_some() {
                lost.push("operationRef of the message event definition".into());
            }
            if local(d) == "compensateEventDefinition"
                && d.attribute("waitForCompletion") == Some("false")
            {
                lost.push("waitForCompletion=\"false\"".into());
            }
            if local(d) == "linkEventDefinition"
                && children(d).any(|c| is_b(c, "source") || is_b(c, "target"))
            {
                lost.push(
                    "source and target of the link event definition (tok pairs links by name)"
                        .into(),
                );
            }
            if let Some(x) =
                children(d).find(|c| c.attribute("language").is_some_and(|l| !l.is_empty()))
            {
                let l = x.attribute("language").unwrap_or("");
                lost.push(format!("expression language {l}"));
            }
        }
        for x in lost {
            self.warn(id, &format!("{x} is not kept"), None);
        }
    }

    fn correlation(&self, msg_ref: &str) -> Option<String> {
        let msg = self.by_id.get(msg_ref)?;
        let ext = find(*msg, "extensionElements")?;
        let sub = children(ext)
            .find(|c| c.tag_name().namespace() == Some(ZEEBE_NS) && local(*c) == "subscription")?;
        let key = sub.attribute("correlationKey").filter(|k| !k.is_empty())?;
        Some(strip_feel(Some(key)))
    }

    fn event_definitions(&mut self, e: X<'a, 'i>, n: &mut Node) {
        let nid = n.base.id().to_string();
        let defs: Vec<X> = children(e)
            .filter(|c| is_bpmn(*c) && definition_key(local(*c)).is_some())
            .collect();
        for c in children(e) {
            if is_bpmn(c) && local(c) == "eventDefinitionRef" {
                self.warn(
                    Some(&nid),
                    "reusable event definitions (eventDefinitionRef) are not supported",
                    None,
                );
            }
        }
        // several definitions make a multiple event (language 0.5)
        if defs.len() > 1 {
            self.uses_05();
        }
        if e.attribute("parallelMultiple") == Some("true") {
            n.base.flags.push("parallel".into());
            self.uses_05();
        }
        for c in defs {
            self.event_definition(c, n);
        }
    }

    /// A trigger value: the first one of a key goes to the attributes, further ones (a
    /// multiple event) to the repeated values.
    fn put_trigger(n: &mut Node, key: &str, value: String) {
        if n.base.attrs.contains_key(key) {
            n.base.extra.push((key.to_string(), value));
        } else {
            n.base.attrs.insert(key.to_string(), value);
        }
    }

    /// A trigger without a value: the flag `message`, `timer`, `signal`, `condition`,
    /// `error` or `escalation` (language 0.5). A second one of the same kind is reported.
    fn nameless(&mut self, n: &mut Node, key: &str) {
        if n.base.has(key) {
            let msg = format!("a second {key} definition without a reference is not kept");
            let nid = n.base.id().to_string();
            self.warn(Some(&nid), &msg, None);
            return;
        }
        n.base.flags.push(key.to_string());
        self.uses_05();
    }

    fn event_definition(&mut self, c: X<'a, 'i>, n: &mut Node) {
        let nid = n.base.id().to_string();
        let key = definition_key(local(c)).unwrap_or("message");
        let label_or_id = n
            .base
            .label
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| nid.clone());
        match key {
            "terminate" | "compensate" | "cancel" => {
                if !n.base.has(key) {
                    n.base.flags.push(key.into());
                }
                if key == "cancel" {
                    self.uses_05();
                }
                if key == "compensate"
                    && let Some(r) = c.attribute("activityRef").filter(|a| !a.is_empty())
                {
                    let t = self.tid_s(Some(r));
                    n.base.attrs.insert("activity".into(), t);
                    self.uses_03();
                }
            }
            "message" => {
                let r = c.attribute("messageRef").filter(|r| !r.is_empty());
                let Some(r) = r else {
                    // a message event without a message (language 0.5)
                    self.nameless(n, "message");
                    return;
                };
                let name = self.named(r, "message");
                Self::put_trigger(n, "message", name);
                if let Some(corr) = self.correlation(r).filter(|c| !c.is_empty()) {
                    n.base.attrs.insert("correlation".into(), corr);
                }
            }
            "signal" | "error" | "escalation" => {
                let attr = format!("{key}Ref");
                match c.attribute(attr.as_str()).filter(|r| !r.is_empty()) {
                    Some(r) => {
                        let name = self.named(r, key);
                        Self::put_trigger(n, key, name);
                        let code_attr = format!("{key}Code");
                        if let Some(code) = self
                            .by_id
                            .get(r)
                            .and_then(|d| d.attribute(code_attr.as_str()))
                            .filter(|c| !c.is_empty())
                        {
                            n.base.attrs.insert("code".into(), code.to_string());
                        }
                    }
                    None if CATCH_ALL_FLAGS.contains(&key)
                        && (n.base.kind == "boundary" || n.base.kind == "start") =>
                    {
                        n.base.flags.push(key.into());
                        self.uses_03();
                    }
                    // an event without a signal, error or escalation (language 0.5)
                    None => self.nameless(n, key),
                }
            }
            "timer" => {
                let mut value = None;
                for part in ["timeDuration", "timeCycle", "timeDate"] {
                    if let Some(x) = find(c, part)
                        && !strip(text(x).unwrap_or("")).is_empty()
                    {
                        value = Some(strip(text(x).unwrap_or("")).to_string());
                        break;
                    }
                }
                match value {
                    Some(v) => Self::put_trigger(n, "timer", v),
                    // a timer without a time (language 0.5)
                    None => self.nameless(n, "timer"),
                }
            }
            "condition" => {
                let x = find(c, "condition");
                let v = strip_feel(x.and_then(text));
                if v.is_empty() {
                    // a conditional event without a condition (language 0.5)
                    self.nameless(n, "condition");
                } else {
                    Self::put_trigger(n, "condition", v);
                }
            }
            _ => {
                let v = c
                    .attribute("name")
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .unwrap_or(label_or_id);
                Self::put_trigger(n, "link", v);
            }
        }
    }

    /// Attributes of language 0.5 on a flow node: transaction method, activation condition,
    /// instantiation, operation and performer.
    fn node_05(&mut self, e: X<'a, 'i>, n: &mut Node) {
        let kind = n.base.kind.clone();
        if kind == "transaction" {
            self.uses_05();
            if let Some((m, _)) = e
                .attribute("method")
                .and_then(|v| TRANSACTION_METHODS.iter().find(|(_, x)| *x == v))
            {
                n.base.attrs.insert("method".into(), (*m).to_string());
            }
        }
        if kind == "complex" {
            self.uses_05();
            if let Some(c) = find(e, "activationCondition")
                && !strip(text(c).unwrap_or("")).is_empty()
            {
                n.base
                    .attrs
                    .insert("activation".into(), strip_feel(text(c)));
            }
        }
        if (kind == "receive" || kind == "eventgw") && e.attribute("instantiate") == Some("true") {
            n.base.flags.push("instantiate".into());
            self.uses_05();
        }
        if kind == "eventgw" && e.attribute("eventGatewayType") == Some("Parallel") {
            n.base.flags.push("parallel".into());
            self.uses_05();
        }
        if matches!(kind.as_str(), "service" | "send" | "receive")
            && let Some(o) = e.attribute("operationRef").filter(|o| !o.is_empty())
        {
            let t = self.tid_s(Some(o));
            n.base.attrs.insert("operation".into(), t);
            self.uses_05();
        }
        let role = children(e).find(|c| {
            is_bpmn(*c) && matches!(local(*c), "performer" | "humanPerformer" | "potentialOwner")
        });
        if let Some(r) = role
            .and_then(|r| find(r, "resourceRef"))
            .and_then(text)
            .map(strip)
            .filter(|r| !r.is_empty())
        {
            let t = self.tid_s(Some(r));
            n.base.attrs.insert("by".into(), t);
            self.uses_05();
        }
    }

    /// Data inputs, outputs and properties that an activity or event declares itself
    /// (`input … on=ID`); those made for data associations (`reads`, `writes`) are left out.
    fn own_data(
        &mut self,
        e: X<'a, 'i>,
        nid: &str,
        scope: &str,
        block: Option<String>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        if matches!(local(e), "subProcess" | "adHocSubProcess" | "transaction") {
            // read with the content of the sub-process (`container`)
            return out;
        }
        // an event has its data inputs or outputs and their set directly
        let io = if is_event(kind_of_tag(local(e)).unwrap_or("")) {
            Some(e)
        } else {
            find(e, "ioSpecification")
        };
        if let Some(io) = io {
            self.io_sets(io);
            for (x, kind) in io_items(io) {
                if x.attribute("id").is_some_and(|i| self.implicit.contains(i)) {
                    continue;
                }
                if let Some(did) = self.io_data(x, kind, scope, block.clone(), Some(nid)) {
                    out.push(did);
                }
            }
        }
        let props: Vec<X> = findall(e, "property").collect();
        for p in props {
            if p.attribute("name") == Some("__targetRef_placeholder")
                || p.attribute("id").is_some_and(|i| self.implicit.contains(i))
            {
                continue;
            }
            if let Some(did) = self.io_data(p, "property", scope, block.clone(), Some(nid)) {
                out.push(did);
            }
        }
        out
    }

    /// tok writes one input set with every data input and one output set with every data
    /// output; other sets are reported.
    fn io_sets(&mut self, io: X<'a, 'i>) {
        for (item, set, refs) in [
            ("dataInput", "inputSet", "dataInputRefs"),
            ("dataOutput", "outputSet", "dataOutputRefs"),
        ] {
            let all: HashSet<&str> = children(io)
                .filter(|x| is_b(*x, item))
                .filter_map(|x| x.attribute("id"))
                .collect();
            let sets: Vec<X> = children(io).filter(|x| is_b(*x, set)).collect();
            let plain = sets.len() <= 1
                && sets.iter().all(|s| {
                    let members: HashSet<&str> = children(*s)
                        .filter(|r| is_b(*r, refs))
                        .filter_map(text)
                        .map(strip)
                        .collect();
                    children(*s).all(|r| is_b(r, refs)) && (members == all || all.is_empty())
                });
            if !plain {
                let owner = io.parent_element().and_then(|p| p.attribute("id"));
                let msg = format!(
                    "the {set}s of the ioSpecification are not kept (tok writes one set of all {item}s)"
                );
                self.warn(owner, &msg, None);
            }
        }
    }

    /// One `input`, `output` or `property` (language 0.5).
    fn io_data(
        &mut self,
        x: X<'a, 'i>,
        kind: &str,
        scope: &str,
        block: Option<String>,
        on: Option<&str>,
    ) -> Option<String> {
        if kind == "property" && x.attribute("name") == Some("__targetRef_placeholder") {
            return None;
        }
        let did = self.tid_of(x);
        let mut base = Base::new(
            kind,
            Some(did.clone()),
            clean_label(x.attribute("name")),
            Attrs::new(),
            Vec::new(),
            0,
        );
        if let Some(t) = x.attribute("itemSubjectRef").filter(|t| !t.is_empty()) {
            let t = self.tid_s(Some(t));
            base.attrs.insert("type".into(), t);
        }
        if kind != "property" && x.attribute("isCollection") == Some("true") {
            base.flags.push("collection".into());
        }
        if let Some(o) = on {
            base.attrs.insert("on".into(), o.to_string());
        }
        self.doc(x, &mut base.attrs);
        let mut d = Data {
            base,
            scope: None,
            block: None,
        };
        if let Some(b) = block {
            d.scope = Some(scope.to_string());
            d.block = Some(b);
        }
        self.doc.ids.insert(
            did.clone(),
            IdEntry {
                kind: kind.to_string(),
                line: 0,
            },
        );
        self.doc.data.insert(did.clone(), d);
        self.uses_05();
        Some(did)
    }

    /// Root elements of language 0.5: item definitions, resources, global tasks, interfaces,
    /// and the correlation keys of the collaboration.
    fn declarations_05(&mut self, collab: Option<X<'a, 'i>>) {
        let root = self.root;
        let mut props: IndexMap<String, X> = IndexMap::new();
        let elems: Vec<X> = children(root).filter(|c| is_bpmn(*c)).collect();
        for e in elems {
            let t = local(e);
            let name = || clean_label(e.attribute("name"));
            match t {
                "itemDefinition" => {
                    let id = self.tid_of(e);
                    // the label of an item is its documentation (an item definition has no name)
                    let docs = find(e, "documentation");
                    if let Some(d) = docs {
                        self.docs_read.insert(d.id());
                    }
                    let label = docs.and_then(|x| clean_label(text(x)));
                    let mut b =
                        Base::new("item", Some(id.clone()), label, Attrs::new(), Vec::new(), 0);
                    if let Some(s) = e.attribute("structureRef").filter(|s| !s.is_empty()) {
                        // tok writes XML Schema types as xsd:NAME and other names without a
                        // namespace prefix
                        let v = match s.split_once(':') {
                            None => s.to_string(),
                            Some((p, name)) if e.lookup_namespace_uri(Some(p)) == Some(XSD_NS) => {
                                format!("xsd:{name}")
                            }
                            Some((p, name)) => match e.lookup_namespace_uri(Some(p)) {
                                // a prefixed name keeps its namespace (language 0.5)
                                Some(ns) if !RESERVED_PREFIXES.contains(&p) => {
                                    b.attrs.insert("namespace".into(), ns.to_string());
                                    format!("{p}:{name}")
                                }
                                _ => {
                                    self.warn(
                                        e.attribute("id"),
                                        &format!(
                                            "namespace prefix of structureRef '{s}' was dropped"
                                        ),
                                        None,
                                    );
                                    name.to_string()
                                }
                            },
                        };
                        b.attrs.insert("structure".into(), v);
                    }
                    match e.attribute("itemKind") {
                        Some("Physical") => {
                            b.attrs.insert("kind".into(), "physical".into());
                        }
                        Some("Information") => {
                            b.attrs.insert("kind".into(), "information".into());
                        }
                        _ => {}
                    }
                    if e.attribute("isCollection") == Some("true") {
                        b.flags.push("collection".into());
                    }
                    self.register_05("item", &id);
                    self.doc.items.insert(id, b);
                }
                "resource" => {
                    let id = self.tid_of(e);
                    let label = name().filter(|l| *l != id);
                    let b = Base::new(
                        "resource",
                        Some(id.clone()),
                        label,
                        Attrs::new(),
                        Vec::new(),
                        0,
                    );
                    self.register_05("resource", &id);
                    self.doc.resources.insert(id, b);
                }
                "interface" => {
                    let id = self.tid_of(e);
                    let label = name().filter(|l| *l != id);
                    let mut attrs = Attrs::new();
                    self.doc(e, &mut attrs);
                    let base =
                        Base::new("interface", Some(id.clone()), label, attrs, Vec::new(), 0);
                    let mut operations = Vec::new();
                    let ops: Vec<X> = findall(e, "operation").collect();
                    for op in ops {
                        let oid = self.tid_of(op);
                        let label = clean_label(op.attribute("name")).filter(|l| *l != oid);
                        let mut attrs = Attrs::new();
                        self.doc(op, &mut attrs);
                        let mut b =
                            Base::new("operation", Some(oid.clone()), label, attrs, Vec::new(), 0);
                        for (child, key, kind) in [
                            ("inMessageRef", "in", "message"),
                            ("outMessageRef", "out", "message"),
                            ("errorRef", "error", "error"),
                        ] {
                            let refs: Vec<String> = children(op)
                                .filter(|c| is_b(*c, child))
                                .filter_map(|c| text(c).map(|t| strip(t).to_string()))
                                .filter(|t| !t.is_empty())
                                .collect();
                            for r in refs {
                                let v = self.named(&r, kind);
                                if b.attrs.contains_key(key) {
                                    b.extra.push((key.to_string(), v));
                                } else {
                                    b.attrs.insert(key.to_string(), v);
                                }
                            }
                        }
                        self.register_05("operation", &oid);
                        operations.push(b);
                    }
                    self.register_05("interface", &id);
                    self.doc
                        .interfaces
                        .insert(id, Interface { base, operations });
                }
                "correlationProperty" => {
                    if let Some(i) = e.attribute("id") {
                        props.insert(i.to_string(), e);
                    }
                }
                _ => {
                    if let Some((kind, _)) = GLOBAL_KINDS.iter().find(|(_, g)| *g == t) {
                        let id = self.tid_of(e);
                        let mut attrs = Attrs::new();
                        self.doc(e, &mut attrs);
                        let mut b =
                            Base::new("global", Some(id.clone()), name(), attrs, Vec::new(), 0);
                        if *kind != "task" {
                            b.attrs.insert("kind".into(), (*kind).to_string());
                        }
                        let default = if *kind == "user" || *kind == "rule" {
                            "##unspecified"
                        } else {
                            ""
                        };
                        if let Some(v) = e
                            .attribute("implementation")
                            .filter(|v| !v.is_empty() && *v != default)
                        {
                            let msg = format!("implementation=\"{v}\" is not kept");
                            self.warn(e.attribute("id"), &msg, None);
                        }
                        self.register_05("global", &id);
                        self.doc.globals.insert(id, b);
                    }
                }
            }
        }
        let Some(c) = collab else {
            return;
        };
        let keys: Vec<X> = findall(c, "correlationKey").collect();
        for k in keys {
            let id = self.tid_of(k);
            let base = Base::new(
                "key",
                Some(id.clone()),
                clean_label(k.attribute("name")),
                Attrs::new(),
                Vec::new(),
                0,
            );
            let mut key = CorrelationKey {
                base,
                props: Vec::new(),
            };
            let refs: Vec<String> = children(k)
                .filter(|r| is_b(*r, "correlationPropertyRef"))
                .filter_map(|r| text(r).map(|t| strip(t).to_string()))
                .collect();
            for r in refs {
                let Some(p) = props.get(&r).copied() else {
                    self.warn(Some(&r), "unknown correlation property reference", None);
                    continue;
                };
                let pname = clean_label(p.attribute("name")).unwrap_or_else(|| r.clone());
                let exprs: Vec<X> = findall(p, "correlationPropertyRetrievalExpression").collect();
                for x in exprs {
                    let mut b = Base::new(
                        "prop",
                        Some(pname.clone()),
                        None,
                        Attrs::new(),
                        Vec::new(),
                        0,
                    );
                    if let Some(m) = x.attribute("messageRef").filter(|m| !m.is_empty()) {
                        let v = self.named(m, "message");
                        b.attrs.insert("message".into(), v);
                    }
                    let path = find(x, "messagePath")
                        .and_then(text)
                        .map(strip)
                        .unwrap_or("");
                    b.attrs.insert("path".into(), path.to_string());
                    key.props.push(b);
                }
            }
            self.register_05("key", &id);
            self.doc.keys.insert(id, key);
        }
    }

    /// Declarations that no element uses (tok declares messages, signals, errors,
    /// escalations and data stores through their uses), and their attributes tok does not
    /// keep.
    fn unreferenced(&mut self) {
        let root = self.root;
        let mut used: HashSet<&str> = HashSet::new();
        for e in root.descendants().filter(|e| is_bpmn(*e)) {
            for a in [
                "messageRef",
                "signalRef",
                "errorRef",
                "escalationRef",
                "dataStoreRef",
                "categoryValueRef",
            ] {
                if let Some(v) = e.attribute(a) {
                    used.insert(v.trim());
                }
            }
            if matches!(local(e), "inMessageRef" | "outMessageRef" | "errorRef")
                && let Some(t) = text(e)
            {
                used.insert(t.trim());
            }
        }
        let decls: Vec<X> = children(root).filter(|e| is_bpmn(*e)).collect();
        for e in decls {
            let t = local(e);
            let id = e.attribute("id");
            if matches!(
                t,
                "message" | "signal" | "error" | "escalation" | "dataStore"
            ) && !id.is_some_and(|i| used.contains(i))
            {
                let msg = format!("{t} that no element uses is not kept");
                self.warn(id, &msg, None);
                continue;
            }
            let mut lost: Vec<String> = Vec::new();
            match t {
                "message" => {
                    if let Some(r) = e.attribute("itemRef").filter(|r| !r.is_empty()) {
                        lost.push(format!("itemRef=\"{r}\""));
                    }
                }
                "signal" | "error" | "escalation" => {
                    if let Some(r) = e.attribute("structureRef").filter(|r| !r.is_empty()) {
                        lost.push(format!("structureRef=\"{r}\""));
                    }
                }
                "dataStore" => {
                    if let Some(c) = e.attribute("capacity").filter(|c| !c.is_empty()) {
                        lost.push(format!("capacity=\"{c}\""));
                    }
                    if e.attribute("isUnlimited") == Some("false") {
                        lost.push("isUnlimited=\"false\"".into());
                    }
                    if let Some(r) = e.attribute("itemSubjectRef").filter(|r| !r.is_empty()) {
                        lost.push(format!("itemSubjectRef=\"{r}\" of the data store"));
                    }
                }
                "category" => {
                    for v in children(e).filter(|v| is_b(*v, "categoryValue")) {
                        if !v.attribute("id").is_some_and(|i| used.contains(i)) {
                            let msg = format!(
                                "category value '{}' that no group uses is not kept",
                                v.attribute("value").unwrap_or("")
                            );
                            self.warn(v.attribute("id"), &msg, None);
                        }
                    }
                    if e.attribute("name").is_some_and(|n| !n.trim().is_empty()) {
                        lost.push("the name of the category".into());
                    }
                }
                _ => {}
            }
            for x in lost {
                self.warn(id, &format!("{x} is not kept"), None);
            }
        }
    }

    /// Everything else tok does not keep: documentation of elements that have no `doc=`,
    /// extension elements and attributes of other tools, BPMN elements tok does not know,
    /// expression languages, `isImmediate`.
    fn sweep(&mut self) {
        let root = self.root;
        let mut immediate = 0;
        for e in root.descendants().filter(|e| is_bpmn(*e)) {
            let t = local(e);
            let parent = e.parent_element();
            let ptag = parent.map(local).unwrap_or("");
            let pid = parent.and_then(|p| p.attribute("id"));
            if t == "documentation" && !self.docs_read.contains(&e.id()) {
                let msg = format!("documentation of {ptag} is not kept");
                self.warn(pid, &msg, None);
            }
            if t == "extensionElements" && kind_of_tag(ptag).is_none() {
                for z in children(e) {
                    let ns = z.tag_name().namespace();
                    let read = ns == Some(ZEEBE_NS)
                        && matches!(
                            (ptag, local(z)),
                            ("message", "subscription")
                                | ("process", "versionTag")
                                | ("multiInstanceLoopCharacteristics", "loopCharacteristics")
                        );
                    if !read {
                        let tag = clark(z);
                        self.warn(
                            pid,
                            &format!("extension element {tag} was skipped"),
                            Some(format!("ext-{tag}")),
                        );
                    }
                }
            }
            for a in e.attributes() {
                let Some(ns) = a.namespace() else {
                    continue;
                };
                if ns == BPMN_NS
                    || ns == crate::xml::MODELER_NS
                    || (ns == crate::xml::XSI_NS && a.name() == "type")
                {
                    continue;
                }
                let qn = format!("{{{ns}}}{}", a.name());
                self.warn(
                    e.attribute("id").or(pid),
                    &format!("attribute {qn} was skipped"),
                    Some(format!("attr-{qn}")),
                );
            }
            if t == "sequenceFlow" && e.attribute("isImmediate").is_some() {
                immediate += 1;
            }
            if let Some(l) = e.attribute("language").filter(|l| !l.is_empty())
                && matches!(t, "conditionExpression" | "transformation")
            {
                let msg = format!("expression language {l} is not kept");
                self.warn(pid, &msg, Some(format!("language-{l}")));
            }
            let known = match ptag {
                "process" | "subProcess" | "transaction" | "adHocSubProcess" => {
                    kind_of_tag(t).is_some() || CONTAINER_PARTS.contains(&t)
                }
                "definitions" => {
                    ROOT_PARTS.contains(&t) || GLOBAL_KINDS.iter().any(|(_, g)| *g == t)
                }
                "collaboration" => COLLABORATION_PARTS.contains(&t),
                "choreography" | "subChoreography" => {
                    kind_of_tag(t).is_some() || lang06::CHOREOGRAPHY_PARTS.contains(&t)
                }
                _ => true,
            };
            if !known {
                let msg = format!("{t} is not supported and was skipped");
                self.warn(e.attribute("id"), &msg, None);
            }
        }
        if immediate > 0 {
            let msg = format!(
                "isImmediate of {immediate} sequence flow(s) is not kept (tok does not record it)"
            );
            self.warn(None, &msg, None);
        }
    }

    fn register_05(&mut self, kind: &str, id: &str) {
        self.doc.ids.insert(
            id.to_string(),
            IdEntry {
                kind: kind.to_string(),
                line: 0,
            },
        );
        self.uses_05();
    }

    fn extensions(&mut self, e: X<'a, 'i>, n: &mut Node) {
        let Some(x) = find(e, "extensionElements") else {
            return;
        };
        let nid = n.base.id().to_string();
        let kind = n.base.kind.clone();
        for z in children(x) {
            let t = local(z);
            if z.tag_name().namespace() != Some(ZEEBE_NS) {
                let tag = clark(z);
                self.warn(
                    Some(&nid),
                    &format!("extension element {tag} was skipped"),
                    Some(format!("ext-{tag}")),
                );
                continue;
            }
            if !Self::in_executable_process(e) {
                // the text keeps them; `tok check` reports them (R055)
                self.warn(
                    Some(&nid),
                    "Camunda extensions of a process that is not executable are not exported",
                    Some("zeebe-not-executable".into()),
                );
            }
            let attr = |name: &str| z.attribute(name).filter(|v| !v.is_empty());
            if t == "userTask" && kind == "user" {
                continue;
            }
            if t == "formDefinition" && (kind == "user" || kind == "start") {
                match attr("formId") {
                    Some(f) => {
                        n.base.attrs.insert("form".into(), f.to_string());
                        let mut a = std::mem::take(&mut n.base.attrs);
                        self.read_binding(z, &mut a);
                        n.base.attrs = a;
                    }
                    None => self.warn(
                        Some(&nid),
                        "form reference without formId (formKey or external form) was skipped",
                        None,
                    ),
                }
            } else if t == "calledDecision" && kind == "rule" {
                let mut a = std::mem::take(&mut n.base.attrs);
                if let Some(d) = attr("decisionId") {
                    a.insert("decision".into(), d.to_string());
                }
                if let Some(r) = attr("resultVariable") {
                    a.insert("result".into(), r.to_string());
                }
                self.read_binding(z, &mut a);
                n.base.attrs = a;
            } else if t == "calledElement" && kind == "call" {
                let mut a = std::mem::take(&mut n.base.attrs);
                if let Some(p) = attr("processId") {
                    a.insert("process".into(), p.to_string());
                }
                self.read_binding(z, &mut a);
                if z.attribute("propagateAllChildVariables") == Some("false") {
                    a.insert("child-vars".into(), "none".into());
                }
                if z.attribute("propagateAllParentVariables") == Some("false") {
                    a.insert("parent-vars".into(), "none".into());
                }
                if let Some(b) = attr("businessId") {
                    a.insert("business-id".into(), b.to_string());
                }
                n.base.attrs = a;
                if z.attributes().count() > 1 {
                    self.uses_03();
                }
            } else if t == "adHoc" && kind == "adhoc" {
                if let Some(v) = attr("activeElementsCollection") {
                    n.base.attrs.insert("active".into(), strip_feel(Some(v)));
                }
            } else if t == "assignmentDefinition" && kind == "user" {
                for (at, key) in [
                    ("assignee", "assignee"),
                    ("candidateUsers", "candidate-users"),
                    ("candidateGroups", "candidate-groups"),
                ] {
                    if let Some(v) = attr(at) {
                        n.base.attrs.insert(key.into(), v.to_string());
                    }
                }
                self.uses_03();
            } else if t == "taskSchedule" && kind == "user" {
                for (at, key) in [("dueDate", "due"), ("followUpDate", "follow-up")] {
                    if let Some(v) = attr(at) {
                        n.base.attrs.insert(key.into(), v.to_string());
                    }
                }
                self.uses_03();
            } else if t == "priorityDefinition" && kind == "user" {
                if let Some(v) = attr("priority") {
                    n.base.attrs.insert("priority".into(), v.to_string());
                }
                self.uses_03();
            } else if t == "ioMapping" {
                let pair = |c: X, name: &str| -> Option<(String, String)> {
                    if c.tag_name().namespace() == Some(ZEEBE_NS) && local(c) == name {
                        Some((
                            c.attribute("target").unwrap_or("None").to_string(),
                            c.attribute("source").unwrap_or("").to_string(),
                        ))
                    } else {
                        None
                    }
                };
                let mut ins: Vec<(String, String)> =
                    children(z).filter_map(|c| pair(c, "input")).collect();
                let outs: Vec<(String, String)> =
                    children(z).filter_map(|c| pair(c, "output")).collect();
                let a = &mut n.base.attrs;
                if a.get("connector").map(String::as_str) == Some("rest")
                    || a.get("type").map(String::as_str) == Some(REST_CONNECTOR_TYPE)
                {
                    let mut rest = Vec::new();
                    for (target, source) in ins {
                        if let Some(key) = connector_input(&target) {
                            a.insert(key.into(), source);
                        } else if target == "authentication.type" && source == "noAuth" {
                            continue;
                        } else {
                            rest.push((target, source));
                        }
                    }
                    ins = rest;
                    a.insert("connector".into(), "rest".into());
                    a.shift_remove("type");
                }
                if !ins.is_empty() {
                    let v: Vec<String> = ins
                        .iter()
                        .map(|(tg, src)| format!("{tg} = {}", strip_feel(Some(src))))
                        .collect();
                    a.insert("inputs".into(), v.join("; "));
                }
                if !outs.is_empty() {
                    let v: Vec<String> = outs
                        .iter()
                        .map(|(tg, src)| format!("{tg} = {}", strip_feel(Some(src))))
                        .collect();
                    a.insert("outputs".into(), v.join("; "));
                }
                self.uses_03();
            } else if t == "taskHeaders" {
                let mut hs: Vec<(String, String)> = children(z)
                    .filter(|c| c.tag_name().namespace() == Some(ZEEBE_NS) && local(*c) == "header")
                    .map(|h| {
                        (
                            h.attribute("key").unwrap_or("None").to_string(),
                            h.attribute("value").unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                let is_rest = zfind(x, "taskDefinition")
                    .is_some_and(|td| td.attribute("type") == Some(REST_CONNECTOR_TYPE));
                let a = &mut n.base.attrs;
                if a.get("connector").map(String::as_str) == Some("rest") || is_rest {
                    let mut keep = Vec::new();
                    for (k, v) in hs {
                        if k == "resultVariable" {
                            a.insert("result".into(), v);
                        } else {
                            keep.push((k, v));
                        }
                    }
                    hs = keep;
                }
                if !hs.is_empty() {
                    let v: Vec<String> = hs.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    a.insert("headers".into(), v.join("; "));
                }
                self.uses_03();
            } else if t == "conditionalFilter" {
                if let Some(v) = attr("variableEvents") {
                    n.base.attrs.insert("events".into(), v.to_string());
                    self.uses_03();
                }
            } else if t == "executionListeners" || t == "taskListeners" {
                let mut items = Vec::new();
                for li in children(z) {
                    let ev = li.attribute("eventType").filter(|v| !v.is_empty());
                    let typ = li.attribute("type").filter(|v| !v.is_empty());
                    if let (Some(ev), Some(typ)) = (ev, typ) {
                        let retries = li
                            .attribute("retries")
                            .filter(|v| !v.is_empty())
                            .map(|r| format!(" ({r})"))
                            .unwrap_or_default();
                        items.push(format!("{ev}: {typ}{retries}"));
                    }
                }
                if !items.is_empty() {
                    let a = &mut n.base.attrs;
                    let mut all: Vec<String> = a
                        .get("listeners")
                        .filter(|p| !p.is_empty())
                        .cloned()
                        .into_iter()
                        .collect();
                    all.extend(items);
                    a.insert("listeners".into(), all.join("; "));
                    self.uses_03();
                }
            } else if t == "script" && kind == "script" {
                let a = &mut n.base.attrs;
                if let Some(x) = attr("expression") {
                    a.insert("expr".into(), strip_feel(Some(x)));
                }
                if let Some(r) = attr("resultVariable") {
                    a.insert("result".into(), r.to_string());
                }
            } else if t == "taskDefinition"
                && (kind == "service"
                    || kind == "send"
                    || ((kind == "throw" || kind == "end")
                        && (n.base.attrs.contains_key("message") || n.base.has("message"))))
            {
                if let Some(ty) = attr("type") {
                    n.base.attrs.insert("type".into(), ty.to_string());
                    if ty == REST_CONNECTOR_TYPE && kind == "service" {
                        n.base.attrs.insert("connector".into(), "rest".into());
                        n.base.attrs.shift_remove("type");
                        self.uses_03();
                    }
                }
                if let Some(r) = attr("retries") {
                    n.base.attrs.insert("retries".into(), r.to_string());
                    self.uses_03();
                }
                if kind == "throw" || kind == "end" {
                    self.uses_03();
                }
            } else {
                self.warn(
                    Some(&nid),
                    &format!("Camunda extension zeebe:{t} was skipped"),
                    Some(format!("zeebe-{t}-{kind}")),
                );
            }
        }
    }

    fn data(&mut self, e: X<'a, 'i>, t: &str, scope: &str, block: Option<String>) -> String {
        let did = self.tid_of(e);
        let mut attrs = Attrs::new();
        self.doc(e, &mut attrs);
        let mut x = if t == "dataObjectReference" || t == "dataObject" {
            // a data object without a reference stands for itself
            let obj = if t == "dataObject" {
                Some(e)
            } else {
                e.attribute("dataObjectRef")
                    .and_then(|r| self.by_id.get(r).copied())
            };
            let object_name = obj.and_then(|o| clean_label(o.attribute("name")));
            let label = if t == "dataObject" {
                object_name.clone()
            } else {
                clean_label(e.attribute("name")).or_else(|| object_name.clone())
            };
            let mut x = Data {
                base: Base::new(
                    "data",
                    Some(did.clone()),
                    label.clone(),
                    attrs,
                    Vec::new(),
                    0,
                ),
                scope: None,
                block: None,
            };
            if let Some(o) = obj {
                // `object=` names a data object shared by several references, or one whose
                // name differs from the reference's (language 0.5)
                let oid = o.attribute("id").unwrap_or("");
                let refs = self.object_refs.get(oid).copied().unwrap_or(0);
                let shared = if t == "dataObject" {
                    refs > 0
                } else {
                    refs > 1 || self.direct_objects.contains(oid)
                };
                if shared || object_name != label {
                    x.base
                        .attrs
                        .insert("object".into(), object_name.clone().unwrap_or_default());
                    self.uses_05();
                }
            }
            if let Some(st) = find(e, "dataState")
                && let Some(name) = st.attribute("name").filter(|v| !v.is_empty())
            {
                x.base.attrs.insert("state".into(), name.to_string());
            }
            if obj.and_then(|o| o.attribute("isCollection")) == Some("true") {
                x.base.flags.push("collection".into());
            }
            if let Some(t) = obj
                .and_then(|o| o.attribute("itemSubjectRef"))
                .or(e.attribute("itemSubjectRef"))
                .filter(|t| !t.is_empty())
            {
                let t = self.tid_s(Some(t));
                x.base.attrs.insert("type".into(), t);
                self.uses_05();
            }
            x
        } else {
            let store = e
                .attribute("dataStoreRef")
                .and_then(|r| self.by_id.get(r).copied());
            let store_name = store.and_then(|o| clean_label(o.attribute("name")));
            let label = clean_label(e.attribute("name")).or_else(|| store_name.clone());
            if store.is_some() && store_name != label {
                self.warn(
                    e.attribute("id"),
                    "the data store is named after its reference (tok shares data stores by label)",
                    None,
                );
            }
            let mut x = Data {
                base: Base::new("store", Some(did.clone()), label, attrs, Vec::new(), 0),
                scope: None,
                block: None,
            };
            if let Some(t) = e.attribute("itemSubjectRef").filter(|t| !t.is_empty()) {
                let t = self.tid_s(Some(t));
                x.base.attrs.insert("type".into(), t);
                self.uses_05();
            }
            x
        };
        if let Some(b) = block {
            x.scope = Some(scope.to_string());
            x.block = Some(b);
        }
        self.doc.ids.insert(
            did.clone(),
            IdEntry {
                kind: x.base.kind.clone(),
                line: 0,
            },
        );
        self.doc.data.insert(did.clone(), x);
        did
    }

    fn flows_and_artifacts(&mut self) {
        let mut flows = Vec::new();
        let mut assocs = Vec::new();
        let mut notes = Vec::new();
        let mut groups = Vec::new();
        for e in self
            .root
            .descendants()
            .filter(|n| n.is_element() && is_bpmn(*n))
        {
            match local(e) {
                "sequenceFlow" => flows.push(e),
                "dataInputAssociation" | "dataOutputAssociation" => assocs.push(e),
                "textAnnotation" => notes.push(e),
                "group" => groups.push(e),
                _ => {}
            }
        }
        for e in flows {
            self.sequence_flow(e);
        }
        self.associations(&assocs);
        let mut annotated: IndexMap<String, Option<String>> = IndexMap::new();
        let mut unkept: Vec<(Option<&str>, String)> = Vec::new();
        for a in self.root.descendants().filter(|n| is_b(*n, "association")) {
            let (src, tgt) = (a.attribute("sourceRef"), a.attribute("targetRef"));
            let is_note = |r: Option<&str>| {
                r.and_then(|r| self.by_id.get(r))
                    .map(|e| local(*e) == "textAnnotation")
                    .unwrap_or(false)
            };
            let note = if is_note(src) {
                Some((src, tgt))
            } else if is_note(tgt) {
                Some((tgt, src))
            } else {
                None
            };
            match note {
                Some((n, other)) => {
                    let n = n.unwrap_or("").to_string();
                    if annotated.contains_key(&n) {
                        unkept.push((
                            a.attribute("id"),
                            format!(
                                "a second association of text annotation '{n}' is not kept (a note has one on=)"
                            ),
                        ));
                    } else {
                        annotated.insert(n, other.map(str::to_string));
                    }
                }
                None => {
                    let skipped = match a.attribute("id") {
                        Some(i) => self.skip_ids.contains(i),
                        None => self.skip_none,
                    };
                    if !skipped {
                        unkept.push((
                            a.attribute("id"),
                            format!(
                                "association between '{}' and '{}' is not kept (tok associates notes and compensation handlers)",
                                src.unwrap_or(""),
                                tgt.unwrap_or("")
                            ),
                        ));
                    }
                }
            }
        }
        for (id, msg) in unkept {
            self.warn(id, &msg, None);
        }
        for e in notes {
            let t = find(e, "text").and_then(text);
            let id = self.tid_of(e);
            let label = clean_label(Some(t.unwrap_or(""))).unwrap_or_default();
            let mut attrs = Attrs::new();
            self.doc(e, &mut attrs);
            if let Some(f) = e
                .attribute("textFormat")
                .filter(|f| !f.is_empty() && *f != "text/plain")
            {
                let msg = format!("text format {f} of the annotation is not kept");
                self.warn(e.attribute("id"), &msg, None);
            }
            let mut note = Base::new("note", Some(id.clone()), Some(label), attrs, Vec::new(), 0);
            let target = e
                .attribute("id")
                .and_then(|i| annotated.get(i))
                .cloned()
                .flatten();
            if let Some(tg) = target.filter(|t| !t.is_empty()) {
                let t = self.tid_s(Some(&tg));
                note.attrs.insert("on".into(), t);
            }
            self.doc.ids.insert(
                id,
                IdEntry {
                    kind: "note".into(),
                    line: 0,
                },
            );
            self.doc.notes.push(note);
        }
        for e in groups {
            let cv = e
                .attribute("categoryValueRef")
                .and_then(|r| self.by_id.get(r).copied());
            let label = cv.and_then(|c| clean_label(c.attribute("value")));
            let id = self.tid_of(e);
            let mut attrs = Attrs::new();
            self.doc(e, &mut attrs);
            self.doc.groups.push(Base::new(
                "group",
                Some(id.clone()),
                label,
                attrs,
                Vec::new(),
                0,
            ));
            self.doc.ids.insert(
                id.clone(),
                IdEntry {
                    kind: "group".into(),
                    line: 0,
                },
            );
            self.warn(
                Some(&id),
                "group membership is not stored in BPMN; add members=\"…\" by hand",
                Some("groups".into()),
            );
        }
    }

    fn sequence_flow(&mut self, e: X<'a, 'i>) {
        let src = self.tid_s(e.attribute("sourceRef"));
        let tgt = self.tid_s(e.attribute("targetRef"));
        if !self.doc.nodes.contains_key(&src) || !self.doc.nodes.contains_key(&tgt) {
            self.warn(
                e.attribute("id"),
                "sequence flow to a skipped element was dropped",
                None,
            );
            return;
        }
        let fid = self.tid_of(e);
        let mut base = Base::new(
            "flow",
            Some(fid),
            clean_label(e.attribute("name")),
            Attrs::new(),
            Vec::new(),
            0,
        );
        if let Some(c) = find(e, "conditionExpression") {
            let w = strip_feel(text(c));
            if w.is_empty() {
                // a condition without an expression (`when=""`, language 0.5)
                self.uses_05();
            }
            base.attrs.insert("when".into(), w);
        }
        self.doc(e, &mut base.attrs);
        if let Some(srcel) = e.attribute("sourceRef").and_then(|r| self.by_id.get(r))
            && srcel.attribute("default").is_some()
            && srcel.attribute("default") == e.attribute("id")
        {
            base.flags.push("default".into());
        }
        self.doc.flows.push(Flow {
            base,
            source: src,
            target: tgt,
            explicit_id: false,
        });
    }

    fn associations(&mut self, assocs: &[X<'a, 'i>]) {
        let mut items: Vec<((u8, u64), usize, Association)> = Vec::new();
        for (k, e) in assocs.iter().enumerate() {
            let Some(owner) = e.parent_element() else {
                continue;
            };
            let Some(oid) = owner.attribute("id").filter(|i| !i.is_empty()) else {
                continue;
            };
            let act = self.tid_s(Some(oid));
            if kind_of_tag(local(owner)).is_some_and(is_event) {
                // data of events (language 0.5)
                self.uses_05();
            }
            let (refname, kind) = if local(*e) == "dataInputAssociation" {
                ("sourceRef", "reads")
            } else {
                ("targetRef", "writes")
            };
            // the end inside the activity: a data input or output declared with `input`
            // or `output … on=ACTIVITY` gives `input=` / `output=` (language 0.5)
            let inner: Vec<String> = children(*e)
                .filter(|r| {
                    is_b(
                        *r,
                        if kind == "reads" {
                            "targetRef"
                        } else {
                            "sourceRef"
                        },
                    )
                })
                .filter_map(text)
                .map(|t| strip(t).to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let (io_key, io_kind) = if kind == "reads" {
                ("input", "input")
            } else {
                ("output", "output")
            };
            let mut declared = None;
            for (k, x) in inner.iter().enumerate() {
                let t = self.idmap.get(x).cloned().unwrap_or_else(|| x.clone());
                if self
                    .doc
                    .data
                    .get(&t)
                    .is_some_and(|d| d.base.kind == io_kind)
                {
                    if k == 0 {
                        declared = Some(t);
                    } else {
                        self.warn(
                            e.attribute("id"),
                            "only the first source of a data output association is kept",
                            None,
                        );
                    }
                }
            }
            for part in ["transformation", "assignment"] {
                if find(*e, part).is_some() {
                    let msg = format!("{part} of the data association is not kept");
                    self.warn(e.attribute("id"), &msg, None);
                }
            }
            let refs: Vec<String> = children(*e)
                .filter(|r| is_b(*r, refname))
                .filter_map(|r| {
                    text(r)
                        .filter(|t| !t.is_empty())
                        .map(|t| strip(t).to_string())
                })
                .collect();
            let mut data_ids = Vec::new();
            for r in &refs {
                let t = self.tid_s(Some(r));
                if self.doc.data.contains_key(&t) {
                    data_ids.push(t);
                }
            }
            if data_ids.is_empty() {
                self.warn(
                    e.attribute("id"),
                    "data association without a data object or store was skipped",
                    None,
                );
                continue;
            }
            let id = e.attribute("id").unwrap_or("");
            let key = match tail_number(id) {
                Some(n) => (0u8, n),
                None => (1u8, k as u64),
            };
            for x in data_ids {
                let mut attrs = Attrs::new();
                if let Some(io) = &declared {
                    attrs.insert(io_key.into(), io.clone());
                    self.uses_05();
                }
                let base = Base::new(
                    kind,
                    Some(format!("{kind}-{act}-{x}")),
                    None,
                    attrs,
                    Vec::new(),
                    0,
                );
                items.push((
                    key,
                    k,
                    Association {
                        base,
                        activity: act.clone(),
                        data: x,
                    },
                ));
            }
        }
        items.sort_by_key(|a| (a.0, a.1));
        self.doc.assocs.extend(items.into_iter().map(|t| t.2));
    }

    fn message_flow(&mut self, e: X<'a, 'i>) {
        let src = self.tid_s(e.attribute("sourceRef"));
        let tgt = self.tid_s(e.attribute("targetRef"));
        if src.is_empty() || tgt.is_empty() {
            self.warn(
                e.attribute("id"),
                "message flow without sourceRef or targetRef was dropped",
                None,
            );
            return;
        }
        let mid = self.tid_of(e);
        let label = clean_label(e.attribute("name"));
        let mut base = Base::new(
            "message",
            Some(mid),
            label.clone(),
            Attrs::new(),
            Vec::new(),
            0,
        );
        if let Some(r) = e.attribute("messageRef").filter(|r| !r.is_empty()) {
            let name = self.named(r, "message");
            if Some(&name) != label.as_ref() {
                base.attrs.insert("message".into(), name);
            }
        } else if let Some(l) = &label {
            let msg = format!("message flow without a message; tok links it to the message '{l}'");
            self.warn(e.attribute("id"), &msg, None);
        }
        self.doc(e, &mut base.attrs);
        self.doc.messages.push(MessageFlow {
            base,
            source: src,
            target: tgt,
            explicit_id: false,
        });
    }

    fn derive_flow_ids(&mut self) {
        let d = &mut self.doc;
        let mut known: HashSet<String> = d.nodes.keys().cloned().collect();
        known.extend(d.lanes.keys().cloned());
        known.extend(d.data.keys().cloned());
        known.extend(d.pools.iter().map(|p| p.base.id().to_string()));
        let mut counter: IndexMap<String, usize> = IndexMap::new();
        let peek =
            |counter: &IndexMap<String, usize>, known: &HashSet<String>, src: &str, tgt: &str| {
                let base = format!("f-{src}-{tgt}");
                let mut n = counter.get(&base).copied().unwrap_or(0) + 1;
                let mut fid = if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                };
                while known.contains(&fid) {
                    n += 1;
                    fid = format!("{base}-{n}");
                }
                (base, fid, n)
            };
        for f in &mut d.flows {
            let (base, expected, n) = peek(&counter, &known, &f.source, &f.target);
            if self.keep_ids && f.base.id() != expected {
                f.explicit_id = true;
            } else {
                f.base.id = Some(expected);
                counter.insert(base, n);
            }
            known.insert(f.base.id().to_string());
            d.ids.insert(
                f.base.id().to_string(),
                IdEntry {
                    kind: "flow".into(),
                    line: 0,
                },
            );
        }
        // message flows that a conversation names keep their identifiers (language 0.6)
        let referenced: HashSet<String> = d
            .conversations
            .values()
            .flat_map(|c| words(c.base.attr("messages").unwrap_or("")))
            .collect();
        for m in &mut d.messages {
            let (base, expected, n) = peek(&counter, &known, &format!("m-{}", m.source), &m.target);
            let expected = expected[2..].to_string();
            if referenced.contains(m.base.id()) {
                m.explicit_id = m.base.id() != expected;
            } else if self.keep_ids && m.base.id() != expected {
                m.explicit_id = true;
            } else {
                m.base.id = Some(expected);
                counter.insert(base, n);
            }
            known.insert(m.base.id().to_string());
            d.ids.insert(
                m.base.id().to_string(),
                IdEntry {
                    kind: "message".into(),
                    line: 0,
                },
            );
        }
        let mut renamed = Vec::new();
        for x in d.notes.iter_mut().chain(d.groups.iter_mut()) {
            if known.contains(x.id()) {
                let old = x.id().to_string();
                let new = if x.kind == "note" {
                    format!("{old}-note")
                } else {
                    format!("{old}-group")
                };
                x.id = Some(new.clone());
                renamed.push((old, new));
            }
        }
        for (old, new) in renamed {
            for v in self.idmap.values_mut() {
                if *v == old {
                    v.clone_from(&new);
                }
            }
            self.warn(
                Some(&old),
                &format!("renamed to '{new}' to avoid a clash with a derived flow identifier"),
                None,
            );
        }
    }
}

/// The data inputs and outputs of an `ioSpecification` with their tok statement.
fn io_items<'a, 'i>(io: X<'a, 'i>) -> Vec<(X<'a, 'i>, &'static str)> {
    children(io)
        .filter_map(|x| match local(x) {
            "dataInput" if is_bpmn(x) => Some((x, "input")),
            "dataOutput" if is_bpmn(x) => Some((x, "output")),
            _ => None,
        })
        .collect()
}

/// Children of a process or sub-process that the import reads or reports itself.
const CONTAINER_PARTS: &[&str] = &[
    "sequenceFlow",
    "dataObject",
    "dataObjectReference",
    "dataStoreReference",
    "laneSet",
    "ioSpecification",
    "property",
    "textAnnotation",
    "association",
    "group",
    "documentation",
    "extensionElements",
    "incoming",
    "outgoing",
    "dataInputAssociation",
    "dataOutputAssociation",
    "standardLoopCharacteristics",
    "multiInstanceLoopCharacteristics",
    "completionCondition",
    "supportedInterfaceRef",
    "performer",
    "humanPerformer",
    "potentialOwner",
    "resourceRole",
    "choreographyTask",
    "subChoreography",
    "callChoreography",
];

/// Root elements that the import reads or reports itself.
const ROOT_PARTS: &[&str] = &[
    "import",
    "itemDefinition",
    "message",
    "signal",
    "error",
    "escalation",
    "dataStore",
    "resource",
    "interface",
    "correlationProperty",
    "category",
    "collaboration",
    "process",
    "choreography",
    "globalChoreographyTask",
    "globalConversation",
    "documentation",
    "extensionElements",
];

/// Children of a collaboration that the import reads or reports itself.
const COLLABORATION_PARTS: &[&str] = &[
    "participant",
    "messageFlow",
    "correlationKey",
    "textAnnotation",
    "association",
    "group",
    "documentation",
    "extensionElements",
    "conversation",
    "subConversation",
    "callConversation",
    "conversationLink",
    "participantAssociation",
    "conversationAssociation",
    "messageFlowAssociation",
    "choreography",
];

/// The ends inside the element of its data associations: targets of the input
/// associations and sources of the output associations, each with the association.
fn inner_ends<'a, 'i>(e: X<'a, 'i>) -> Vec<(String, X<'a, 'i>)> {
    let mut out = Vec::new();
    for (assoc, end) in [
        ("dataInputAssociation", "targetRef"),
        ("dataOutputAssociation", "sourceRef"),
    ] {
        for a in children(e).filter(|c| is_b(*c, assoc)) {
            for r in children(a).filter(|c| is_b(*c, end)) {
                if let Some(t) = text(r) {
                    out.push((strip(t).to_string(), a));
                }
            }
        }
    }
    out
}

/// The data element at the outer end of a data association: the sources of an input
/// association, the target of an output association.
fn outer_ends<'a>(a: X<'a, '_>) -> Vec<&'a str> {
    let end = if local(a) == "dataInputAssociation" {
        "sourceRef"
    } else {
        "targetRef"
    };
    children(a)
        .filter(|c| is_b(*c, end))
        .filter_map(text)
        .map(strip)
        .filter(|t| !t.is_empty())
        .collect()
}

/// `ACTIVITY-in-N` or `ACTIVITY-out-N`, the identifiers tok gives the data inputs and
/// outputs of `reads` and `writes`.
fn generated_io_id(activity: &str, id: &str) -> bool {
    let Some(rest) = id.strip_prefix(activity) else {
        return false;
    };
    let Some(n) = rest
        .strip_prefix("-in-")
        .or_else(|| rest.strip_prefix("-out-"))
    else {
        return false;
    };
    !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

/// `-(?:dia|doa)-(\d+)$`
fn tail_number(id: &str) -> Option<u64> {
    let digits_start = id
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    let digits = &id[digits_start..];
    if digits.is_empty() {
        return None;
    }
    let head = &id[..digits_start];
    if head.ends_with("-dia-") || head.ends_with("-doa-") {
        digits.parse().ok()
    } else {
        None
    }
}

#[derive(Default)]
struct Order {
    children: Vec<String>,
    nodes: Vec<String>,
    data: Vec<String>,
}
