//! Checks of language 0.6: conversations (R070, R071), choreographies (R072, R073),
//! participant multiplicity (R074), the new statements at the levels basic and analytic
//! (R040) and, in executable documents for Camunda, C120–C123.

use super::{Checker, s};
use crate::diagnostics::Diag;
use crate::model::*;

/// Triggers allowed on each kind of event in a choreography (BPMN 2.0.2 §11.6, Tables
/// 11.6–11.8); `None` stands for an event without a trigger.
fn choreography_triggers(kind: &str) -> (&'static [&'static str], bool) {
    match kind {
        "start" => (&["timer", "condition", "signal"], true),
        "catch" => (&["timer", "condition", "signal", "link"], false),
        "throw" => (&["link"], true),
        "boundary" => (&["timer", "condition", "signal"], false),
        "end" => (&["terminate"], true),
        _ => (&[], true),
    }
}

/// Every trigger of an event: attributes (also repeated ones) and trigger flags.
fn triggers(b: &Base) -> Vec<String> {
    let mut out: Vec<String> = TRIGGERS
        .iter()
        .filter(|k| !b.values(k).is_empty())
        .map(|k| (*k).to_string())
        .collect();
    for f in TRIGGER_FLAGS
        .iter()
        .chain(CATCH_ALL_FLAGS)
        .chain(NAMELESS_TRIGGERS)
    {
        if b.has(f) && !out.iter().any(|x| x == f) {
            out.push((*f).to_string());
        }
    }
    out
}

/// The different words of a list attribute.
fn distinct(text: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words(text.unwrap_or("")) {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// `inner:outer` pairs of `map=`; malformed items are returned separately.
fn map_items(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for item in text.split(';').map(str::trim).filter(|x| !x.is_empty()) {
        match participant_map(item) {
            Some(mut pairs) if pairs.len() == 1 => ok.push(pairs.remove(0)),
            _ => bad.push(item.to_string()),
        }
    }
    (ok, bad)
}

impl Checker<'_> {
    /// R070–R074, the attributes of the new statements, and C120–C123.
    pub(super) fn layer_06(&mut self, camunda: bool) {
        self.attributes_06();
        self.multiplicity();
        self.conversations();
        self.choreographies();
        if camunda && self.doc.executable_any() {
            self.camunda_06();
        }
    }

    /// R010 and R011 for conversation nodes and choreographies (their activities are
    /// flow nodes and are checked with the others).
    fn attributes_06(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        let items = d
            .conversations
            .values()
            .map(|c| (&c.base, conversation_keys(&c.base.kind)))
            .chain(
                d.choreographies
                    .values()
                    .map(|c| (&c.base, CHOREOGRAPHY_KEYS)),
            );
        for (b, keys) in items {
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
                found.push(Diag::new(
                    "R011",
                    b.line,
                    b.id.as_deref(),
                    &[("flag", f.clone()), ("kind", b.kind.clone())],
                ));
            }
        }
        self.diags.extend(found);
    }

    /// R074: `min=` and `max=` of a pool are whole numbers with 0 <= min <= max and
    /// max >= 1 (min is 2 when only max is given, max is 1 when only min is given).
    fn multiplicity(&mut self) {
        if !self.doc.lang_06() {
            return;
        }
        let mut found = Vec::new();
        for p in &self.doc.pools {
            let (min, max) = (p.base.attr("min"), p.base.attr("max"));
            if min.is_none() && max.is_none() {
                continue;
            }
            let num = |v: Option<&str>| v.map(|x| x.parse::<i64>().ok());
            let ok = match (num(min), num(max)) {
                (Some(None), _) | (_, Some(None)) => false,
                (Some(Some(a)), Some(Some(b))) => a >= 0 && a <= b && b >= 1,
                // without max= the maximum is that of BPMN, 1
                (Some(Some(a)), None) => (0..=1).contains(&a),
                (None, Some(Some(b))) => b >= 2,
                (None, None) => true,
            };
            if !ok {
                found.push(Diag::new(
                    "R074",
                    p.base.line,
                    p.base.id.as_deref(),
                    &[("id", s(p.base.id()))],
                ));
            }
        }
        self.diags.extend(found);
    }

    /// Pools of `pools=` that exist; R002/R003 for the others.
    fn pool_list(&mut self, text: &str, line: usize) -> Vec<String> {
        let mut out = Vec::new();
        for p in words(text) {
            if self.reference(&p, line, "pool", &|k| k == "pool").is_some() && !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// R070, R071 and the participant associations of call conversations.
    fn conversations(&mut self) {
        let items: Vec<Conversation> = self.doc.conversations.values().cloned().collect();
        for c in items {
            let b = &c.base;
            let nid = b.id().to_string();
            let id = Some(nid.as_str());
            let line = b.line;
            let pools = self.pool_list(b.attr("pools").unwrap_or(""), line);
            if distinct(b.attr("pools")).len() < 2 {
                self.detail(
                    "R070",
                    line,
                    id,
                    format!("'{nid}' must link at least two different pools (pools=)"),
                    format!("„{nid}” musi łączyć co najmniej dwa różne baseny (pools=)"),
                );
            }
            if let Some(parent) = c
                .parent
                .as_ref()
                .and_then(|p| self.doc.conversations.get(p))
            {
                let outer = parent.pools();
                let pname = parent.base.id().to_string();
                for p in pools.iter().filter(|p| !outer.contains(p)) {
                    self.detail(
                        "R070",
                        line,
                        id,
                        format!(
                            "pool '{p}' of '{nid}' is not linked by the sub-conversation '{pname}'"
                        ),
                        format!("basen „{p}” w „{nid}” nie należy do podkonwersacji „{pname}”"),
                    );
                }
            }
            for m in words(b.attr("messages").unwrap_or("")) {
                if self
                    .reference(&m, line, "message flow", &|k| k == "message")
                    .is_none()
                {
                    continue;
                }
                let Some(mf) = self.doc.messages.iter().find(|x| x.base.id() == m) else {
                    continue;
                };
                let ends = [mf.source.clone(), mf.target.clone()].map(|e| self.pool_of(&e));
                let between = match (&ends[0], &ends[1]) {
                    (Some(a), Some(z)) => a != z && pools.contains(a) && pools.contains(z),
                    _ => false,
                };
                if !between {
                    self.add(
                        "R071",
                        line,
                        id,
                        &[("flow", m.clone()), ("id", nid.clone())],
                    );
                }
            }
            if b.kind == "callconversation" {
                let called = b.attr("collaboration").map(str::to_string);
                if let Some(cid) = &called
                    && self.doc.ids.contains_key(cid)
                {
                    self.reference(cid, line, "choreography", &|k| k == "choreography");
                }
                let inner = called
                    .as_deref()
                    .filter(|c| self.doc.choreographies.contains_key(*c))
                    .map(|c| self.choreography_pools(c));
                self.participant_map(b, &pools, inner.as_deref(), "R070");
            }
        }
    }

    /// The pool of a pool or flow node.
    fn pool_of(&self, id: &str) -> Option<String> {
        if self.doc.pool(id).is_some() {
            return Some(id.to_string());
        }
        self.doc.nodes.get(id).and_then(|n| n.pool.clone())
    }

    /// Pools that take part in the activities of a choreography, in document order.
    pub(super) fn choreography_pools(&self, choreography: &str) -> Vec<String> {
        choreography_participants(self.doc, choreography)
    }

    /// `map="inner:outer; …"`: R053 for malformed items, `code` when an outer participant
    /// is not one of `pools` or an inner one not a participant of the called element.
    fn participant_map(
        &mut self,
        b: &Base,
        pools: &[String],
        inner: Option<&[String]>,
        code: &str,
    ) {
        let Some(text) = b.attr("map") else {
            return;
        };
        let nid = b.id().to_string();
        let (pairs, bad) = map_items(text);
        for item in bad {
            self.add(
                "R053",
                b.line,
                Some(&nid),
                &[
                    ("key", s("map")),
                    ("item", item),
                    ("expected", s("inner:outer")),
                    ("expected_pl", s("wewnętrzny:zewnętrzny")),
                ],
            );
        }
        for (i, o) in pairs {
            if !pools.contains(&o) {
                self.detail(
                    code,
                    b.line,
                    Some(&nid),
                    format!("map= of '{nid}': '{o}' is not one of its pools"),
                    format!("map= w „{nid}”: „{o}” nie należy do jego basenów (pools=)"),
                );
            }
            if let Some(known) = inner
                && !known.contains(&i)
            {
                self.detail(
                    code,
                    b.line,
                    Some(&nid),
                    format!("map= of '{nid}': '{i}' is not a participant of the called element"),
                    format!("map= w „{nid}”: „{i}” nie jest uczestnikiem wywoływanego elementu"),
                );
            }
        }
    }

    /// R072 (participants of choreography activities) and R073 (what a choreography may
    /// contain).
    fn choreographies(&mut self) {
        let nodes: Vec<Node> = self
            .doc
            .nodes
            .values()
            .filter(|n| n.choreography.is_some())
            .cloned()
            .collect();
        for n in nodes {
            let b = &n.base;
            let kind = b.kind.as_str();
            let nid = b.id().to_string();
            let id = Some(nid.as_str());
            let line = b.line;
            if !is_choreo(kind) && !CHOREOGRAPHY_NODE_KINDS.contains(&kind) {
                self.detail(
                    "R073",
                    line,
                    id,
                    format!(
                        "'{nid}' ({kind}) cannot stand in a choreography; a choreography contains events, gateways and choreography activities"
                    ),
                    format!(
                        "„{nid}” ({kind}) nie może wystąpić w choreografii; choreografia zawiera zdarzenia, bramki i czynności choreografii"
                    ),
                );
                continue;
            }
            if is_event(kind) {
                let (allowed, none_ok) = choreography_triggers(kind);
                let trig = triggers(b);
                let bad: Vec<String> = trig
                    .iter()
                    .filter(|t| !allowed.contains(&t.as_str()))
                    .cloned()
                    .collect();
                if !bad.is_empty() {
                    let list = bad.join(", ");
                    self.detail(
                        "R073",
                        line,
                        id,
                        format!("'{nid}': the trigger {list} is not allowed on {kind} events in a choreography"),
                        format!("„{nid}”: wyzwalacz {list} nie może wystąpić w zdarzeniach {kind} w choreografii"),
                    );
                } else if trig.is_empty() && !none_ok {
                    self.detail(
                        "R073",
                        line,
                        id,
                        format!("'{nid}': {kind} events in a choreography need a timer, conditional or signal trigger"),
                        format!("„{nid}”: zdarzenia {kind} w choreografii wymagają wyzwalacza czasowego, warunkowego albo sygnału"),
                    );
                }
                continue;
            }
            if !is_choreo(kind) {
                continue;
            }
            self.choreography_activity(&n);
        }
        for c in self
            .doc
            .choreographies
            .values()
            .cloned()
            .collect::<Vec<_>>()
        {
            if c.children.is_empty() {
                let cid = c.base.id().to_string();
                self.detail(
                    "R073",
                    c.base.line,
                    Some(&cid),
                    format!("choreography '{cid}' has no flow nodes"),
                    format!("choreografia „{cid}” nie zawiera elementów przepływu"),
                );
            }
        }
    }

    /// R072 for one `choreo`, `subchoreo` or `callchoreo`.
    fn choreography_activity(&mut self, n: &Node) {
        let b = &n.base;
        let kind = b.kind.as_str();
        let nid = b.id().to_string();
        let id = Some(nid.as_str());
        let line = b.line;
        let participants: Vec<String> = if kind == "choreo" {
            let from = b.attr("from").map(str::to_string);
            let to = b.attr("to").map(str::to_string);
            let (Some(f), Some(t)) = (from, to) else {
                self.detail(
                    "R072",
                    line,
                    id,
                    format!("choreography task '{nid}' needs from= and to=, two different pools"),
                    format!(
                        "zadanie choreografii „{nid}” wymaga from= i to=, dwóch różnych basenów"
                    ),
                );
                return;
            };
            let a = self.reference(&f, line, "pool", &|k| k == "pool");
            let z = self.reference(&t, line, "pool", &|k| k == "pool");
            if a.is_some() && z.is_some() && f == t {
                self.detail(
                    "R072",
                    line,
                    id,
                    format!("choreography task '{nid}' needs from= and to=, two different pools"),
                    format!(
                        "zadanie choreografii „{nid}” wymaga from= i to=, dwóch różnych basenów"
                    ),
                );
            }
            vec![f, t]
        } else {
            let pools = self.pool_list(b.attr("pools").unwrap_or(""), line);
            let named = distinct(b.attr("pools"));
            let from = b.attr("from");
            if named.len() < 2 || !from.is_some_and(|f| named.iter().any(|p| p == f)) {
                self.detail(
                    "R072",
                    line,
                    id,
                    format!("'{nid}' needs pools= with at least two pools and from= one of them"),
                    format!("„{nid}” wymaga pools= z co najmniej dwoma basenami i from= wskazującego jeden z nich"),
                );
            }
            if kind == "callchoreo" {
                match b.attr("choreography") {
                    None => self.detail(
                        "R072",
                        line,
                        id,
                        format!("call choreography '{nid}' needs choreography=ID"),
                        format!("wywołanie choreografii „{nid}” wymaga choreography=ID"),
                    ),
                    Some(c) => {
                        let c = c.to_string();
                        let inner = self
                            .reference(&c, line, "choreography", &|k| k == "choreography")
                            .map(|_| self.choreography_pools(&c));
                        self.participant_map(b, &pools, inner.as_deref(), "R072");
                    }
                }
            }
            pools
        };
        // the participants of an activity inside a sub-choreography are its participants
        if let Some(sub) = n.scope.as_ref().and_then(|sc| self.doc.nodes.get(sc))
            && sub.base.kind == "subchoreo"
        {
            let outer = words(sub.base.attr("pools").unwrap_or(""));
            let sid = sub.base.id().to_string();
            for p in participants.iter().filter(|p| !outer.contains(p)) {
                if self.doc.pool(p).is_none() {
                    continue;
                }
                self.detail(
                    "R072",
                    line,
                    id,
                    format!(
                        "pool '{p}' of '{nid}' is not a participant of the sub-choreography '{sid}'"
                    ),
                    format!("basen „{p}” w „{nid}” nie jest uczestnikiem podchoreografii „{sid}”"),
                );
            }
        }
    }

    /// C120–C123: conversations, choreographies and multiplicity in executable documents.
    fn camunda_06(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        for c in d.conversations.values() {
            let code = if c.base.kind == "callconversation" {
                "C121"
            } else {
                "C120"
            };
            found.push(Diag::new(
                code,
                c.base.line,
                c.base.id.as_deref(),
                &[("id", s(c.base.id()))],
            ));
        }
        for c in d.choreographies.values() {
            found.push(Diag::new(
                "C122",
                c.base.line,
                c.base.id.as_deref(),
                &[("id", s(c.base.id()))],
            ));
        }
        if d.lang_06() {
            for p in &d.pools {
                if p.base.attrs.contains_key("min") || p.base.attrs.contains_key("max") {
                    found.push(Diag::new(
                        "C123",
                        p.base.line,
                        p.base.id.as_deref(),
                        &[("id", s(p.base.id()))],
                    ));
                }
            }
        }
        self.diags.extend(found);
    }

    /// R040 for the statements of language 0.6 at the levels basic and analytic.
    pub(super) fn levels_06(&self, level: &str, found: &mut Vec<Diag>) {
        let d = &*self.doc;
        let mut push = |b: &Base, keyword: &str| {
            found.push(Diag::new(
                "R040",
                b.line,
                b.id.as_deref(),
                &[("keyword", s(keyword)), ("level", s(level))],
            ));
        };
        for c in d.conversations.values() {
            push(&c.base, &c.base.kind);
        }
        for c in d.choreographies.values() {
            push(&c.base, "choreography");
        }
        if d.lang_06() {
            for p in &d.pools {
                for k in ["min", "max"] {
                    if p.base.attrs.contains_key(k) {
                        push(&p.base, k);
                        break;
                    }
                }
            }
        }
    }
}

/// Pools that take part in the activities of a choreography (`from=`, `to=`, `pools=`),
/// in document order.
pub fn choreography_participants(doc: &Document, choreography: &str) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    for n in doc
        .nodes
        .values()
        .filter(|n| n.choreography.as_deref() == Some(choreography) && is_choreo(&n.base.kind))
    {
        for k in ["from", "to"] {
            if let Some(v) = n.base.attr(k) {
                used.push(v.to_string());
            }
        }
        used.extend(words(n.base.attr("pools").unwrap_or("")));
    }
    doc.pools
        .iter()
        .map(|p| p.base.id().to_string())
        .filter(|p| used.contains(p))
        .collect()
}
