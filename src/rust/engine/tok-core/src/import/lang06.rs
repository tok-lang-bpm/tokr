//! Import of the BPMN elements of language 0.6: participant multiplicity, conversation
//! nodes of collaborations, and choreographies with their activities.

use indexmap::IndexMap;

use super::{Importer, X, children, clean_label, find, findall, is_bpmn, kind_of_tag, local, text};
use crate::model::*;

/// Children of a choreography or sub-choreography that the import reads or reports itself.
pub(super) const CHOREOGRAPHY_PARTS: &[&str] = &[
    "participant",
    "messageFlow",
    "sequenceFlow",
    "textAnnotation",
    "association",
    "group",
    "documentation",
    "extensionElements",
    "incoming",
    "outgoing",
    "participantRef",
    "choreographyTask",
    "subChoreography",
    "callChoreography",
];

/// A message flow of a choreography: source and target pool, message name.
struct ChoreoFlow {
    source: String,
    message: Option<String>,
}

fn ref_texts<'a>(e: X<'a, '_>, name: &'static str) -> Vec<&'a str> {
    findall(e, name)
        .filter_map(|r| text(r))
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect()
}

impl<'a, 'i> Importer<'a, 'i> {
    /// The document uses elements of language 0.6.
    pub(super) fn uses_06(&mut self) {
        self.doc.version = Some("0.6".into());
    }

    /// `min=` and `max=` of a pool from `participantMultiplicity` (the minimum is left out
    /// when it is 2 and a maximum is given, the default of tok).
    pub(super) fn multiplicity(&mut self, part: X<'a, 'i>, attrs: &mut Attrs) {
        let Some(m) = find(part, "participantMultiplicity") else {
            return;
        };
        let min = m.attribute("minimum").unwrap_or("0").trim().to_string();
        match m.attribute("maximum").map(str::trim) {
            Some(max) => {
                if min != "2" {
                    attrs.insert("min".into(), min);
                }
                attrs.insert("max".into(), max.to_string());
            }
            None => {
                attrs.insert("min".into(), min);
            }
        }
        self.uses_06();
    }

    /// Pools that stand for the participants of the choreographies: the pool whose
    /// identifier follows the prefix `CHOREOGRAPHY-` (as tok exports them), else the only
    /// pool with the name of the participant, else a new black-box pool.
    pub(super) fn choreography_participants(&mut self) {
        let chors: Vec<X> = findall(self.root, "choreography").collect();
        for c in chors {
            let cx = c.attribute("id").unwrap_or("").to_string();
            for p in findall(c, "participant") {
                let Some(xid) = p.attribute("id") else {
                    continue;
                };
                let name = clean_label(p.attribute("name"));
                let by_prefix = xid
                    .strip_prefix(&format!("{cx}-"))
                    .filter(|rest| self.doc.pool(rest).is_some())
                    .map(str::to_string);
                let by_name = || -> Option<String> {
                    let n = name.as_ref()?;
                    let found: Vec<&Pool> = self
                        .doc
                        .pools
                        .iter()
                        .filter(|q| q.base.label.as_ref() == Some(n))
                        .collect();
                    (found.len() == 1).then(|| found[0].base.id().to_string())
                };
                let pool = match by_prefix.or_else(by_name) {
                    Some(pool) => pool,
                    None => {
                        let pid = self.tid_s(Some(xid));
                        let mut attrs = Attrs::new();
                        self.multiplicity(p, &mut attrs);
                        self.doc.pools.push(Pool {
                            base: Base::new("pool", Some(pid.clone()), name, attrs, Vec::new(), 0),
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
                        pid
                    }
                };
                self.choreo_pools.insert(xid.to_string(), pool);
            }
        }
    }

    /// Every choreography as a `choreography` block.
    pub(super) fn choreographies(&mut self) {
        let chors: Vec<X> = findall(self.root, "choreography").collect();
        for c in chors {
            let cid = self.tid_of(c);
            let mut flows: IndexMap<String, ChoreoFlow> = IndexMap::new();
            for mf in findall(c, "messageFlow") {
                let pool = |r: Option<&str>| {
                    r.and_then(|r| self.choreo_pools.get(r))
                        .cloned()
                        .unwrap_or_default()
                };
                let source = pool(mf.attribute("sourceRef"));
                let message = mf
                    .attribute("messageRef")
                    .filter(|r| !r.is_empty())
                    .map(|r| r.to_string());
                let message = message.map(|r| self.named(&r, "message"));
                flows.insert(
                    mf.attribute("id").unwrap_or("").to_string(),
                    ChoreoFlow { source, message },
                );
            }
            let mut attrs = Attrs::new();
            self.doc(c, &mut attrs);
            for k in findall(c, "correlationKey") {
                self.warn(
                    k.attribute("id"),
                    "correlation key of a choreography is not kept",
                    None,
                );
            }
            let base = Base::new(
                "choreography",
                Some(cid.clone()),
                clean_label(c.attribute("name")),
                attrs,
                Vec::new(),
                0,
            );
            self.doc.ids.insert(
                cid.clone(),
                IdEntry {
                    kind: "choreography".into(),
                    line: 0,
                },
            );
            let kids = self.choreography_block(c, &cid, &cid, &flows);
            self.doc.choreographies.insert(
                cid,
                Choreography {
                    base,
                    children: kids,
                },
            );
            self.uses_06();
        }
    }

    fn pool_of_participant(&self, xid: &str) -> String {
        self.choreo_pools
            .get(xid)
            .cloned()
            .unwrap_or_else(|| xid.to_string())
    }

    fn choreography_block(
        &mut self,
        e: X<'a, 'i>,
        cid: &str,
        scope: &str,
        flows: &IndexMap<String, ChoreoFlow>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        let kids: Vec<X> = children(e).filter(|c| is_bpmn(*c)).collect();
        for c in kids {
            let t = local(c);
            let nid = if let Some(kind) = kind_of_tag(t) {
                if is_activity(kind) {
                    self.warn(
                        c.attribute("id"),
                        &format!("{t} cannot stand in a choreography and was skipped"),
                        None,
                    );
                    continue;
                }
                self.node(c, t, scope, None)
            } else if matches!(
                t,
                "choreographyTask" | "subChoreography" | "callChoreography"
            ) {
                self.choreography_activity(c, t, cid, scope, flows)
            } else {
                continue;
            };
            if let Some(n) = self.doc.nodes.get_mut(&nid) {
                n.choreography = Some(cid.to_string());
                n.scope = Some(scope.to_string());
            }
            out.push(nid);
        }
        out
    }

    fn choreography_activity(
        &mut self,
        e: X<'a, 'i>,
        t: &str,
        cid: &str,
        scope: &str,
        flows: &IndexMap<String, ChoreoFlow>,
    ) -> String {
        let kind = match t {
            "choreographyTask" => "choreo",
            "subChoreography" => "subchoreo",
            _ => "callchoreo",
        };
        let nid = self.tid_of(e);
        let mut attrs = Attrs::new();
        let mut flags = Vec::new();
        let parts: Vec<String> = ref_texts(e, "participantRef")
            .iter()
            .map(|r| self.pool_of_participant(r))
            .collect();
        let from = e
            .attribute("initiatingParticipantRef")
            .map(|r| self.pool_of_participant(r))
            .or_else(|| parts.first().cloned());
        if kind == "choreo" {
            let from = from.unwrap_or_default();
            let to = parts
                .iter()
                .find(|p| **p != from)
                .cloned()
                .unwrap_or_default();
            attrs.insert("from".into(), from.clone());
            attrs.insert("to".into(), to);
            let mut generated = true;
            for r in ref_texts(e, "messageFlowRef") {
                let Some(f) = flows.get(r) else {
                    continue;
                };
                let initiating = f.source == from;
                let own = format!("{}-{}", nid, if initiating { "message" } else { "reply" });
                generated &= self.tid(Some(r)).as_deref() == Some(own.as_str());
                if initiating {
                    if let Some(m) = &f.message {
                        attrs.insert("message".into(), m.clone());
                    }
                } else {
                    attrs.insert("reply".into(), f.message.clone().unwrap_or_default());
                }
            }
            if !generated && self.keep_ids {
                self.warn(
                    e.attribute("id"),
                    "identifiers of the message flows of a choreography task are generated (TASK-message, TASK-reply)",
                    None,
                );
            }
        } else {
            attrs.insert("pools".into(), parts.join(" "));
            if let Some(f) = from {
                attrs.insert("from".into(), f);
            }
        }
        if kind == "callchoreo" {
            if let Some(c) = e
                .attribute("calledChoreographyRef")
                .filter(|c| !c.is_empty())
            {
                let c = self.tid_s(Some(c));
                attrs.insert("choreography".into(), c);
            }
            let map = self.associations_06(e);
            if !map.is_empty() {
                attrs.insert("map".into(), map);
            }
        }
        match e.attribute("loopType") {
            Some("Standard") => flags.push("loop".to_string()),
            Some("MultiInstanceParallel") => flags.push("parallel".to_string()),
            Some("MultiInstanceSequential") => flags.push("sequential".to_string()),
            _ => {}
        }
        self.doc(e, &mut attrs);
        let mut base = Base::new(
            kind,
            Some(nid.clone()),
            clean_label(e.attribute("name")),
            attrs,
            Vec::new(),
            0,
        );
        base.flags = flags;
        self.doc.ids.insert(
            nid.clone(),
            IdEntry {
                kind: kind.to_string(),
                line: 0,
            },
        );
        self.doc.nodes.insert(
            nid.clone(),
            Node {
                base,
                scope: Some(scope.to_string()),
                lane: None,
                pool: None,
                children: Vec::new(),
                choreography: Some(cid.to_string()),
            },
        );
        if kind == "subchoreo" {
            let inner = self.choreography_block(e, cid, &nid, flows);
            if let Some(n) = self.doc.nodes.get_mut(&nid) {
                n.children = inner;
            }
        }
        nid
    }

    /// `map="inner:outer; …"` from the participant associations of a call.
    fn associations_06(&mut self, e: X<'a, 'i>) -> String {
        let mut items = Vec::new();
        for pa in findall(e, "participantAssociation") {
            let inner = find(pa, "innerParticipantRef")
                .and_then(|r| text(r))
                .map(str::trim);
            let outer = find(pa, "outerParticipantRef")
                .and_then(|r| text(r))
                .map(str::trim);
            if let (Some(i), Some(o)) = (inner, outer) {
                let i = self.pool_of_participant(i);
                let o = self.pool_of_participant(o);
                items.push(format!("{i}:{o}"));
            }
        }
        items.join("; ")
    }

    /// Conversation nodes of the collaborations; their conversation links become pools of
    /// the nodes (tok draws one link per pool).
    pub(super) fn conversations(&mut self, collabs: &[X<'a, 'i>]) {
        for c in collabs {
            let links: Vec<X> = findall(*c, "conversationLink").collect();
            for n in children(*c).filter(|x| is_bpmn(*x)) {
                if matches!(
                    local(n),
                    "conversation" | "subConversation" | "callConversation"
                ) {
                    self.conversation_node(n, None, &links);
                }
            }
            for l in &links {
                let ends = [l.attribute("sourceRef"), l.attribute("targetRef")];
                let to_node = ends.iter().flatten().any(|r| {
                    self.by_id.get(*r).is_some_and(|x| {
                        matches!(
                            local(*x),
                            "conversation" | "subConversation" | "callConversation"
                        )
                    })
                });
                let to_pool = ends.iter().flatten().any(|r| {
                    self.by_id
                        .get(*r)
                        .is_some_and(|x| local(*x) == "participant")
                });
                if !(to_node && to_pool) {
                    self.warn(
                        l.attribute("id"),
                        "conversation link that does not connect a pool with a conversation node was skipped",
                        None,
                    );
                }
            }
        }
    }

    fn conversation_node(&mut self, e: X<'a, 'i>, parent: Option<String>, links: &[X<'a, 'i>]) {
        let t = local(e);
        let kind = match t {
            "subConversation" => "subconversation",
            "callConversation" => "callconversation",
            _ => "conversation",
        };
        let xid = e.attribute("id").unwrap_or("");
        let nid = self.tid_of(e);
        let mut pools: Vec<String> = ref_texts(e, "participantRef")
            .iter()
            .map(|r| self.tid_s(Some(r)))
            .collect();
        if parent.is_none() {
            for l in links {
                let (s, tg) = (l.attribute("sourceRef"), l.attribute("targetRef"));
                let other = if tg == Some(xid) {
                    s
                } else if s == Some(xid) {
                    tg
                } else {
                    continue;
                };
                let Some(o) = other.filter(|o| {
                    self.by_id
                        .get(*o)
                        .is_some_and(|x| local(*x) == "participant")
                }) else {
                    continue;
                };
                let pid = self.tid_s(Some(o));
                if !pools.contains(&pid) {
                    pools.push(pid.clone());
                }
                if self.keep_ids
                    && l.attribute("id")
                        != Some(crate::export::conversation_link_id(&nid, &pid).as_str())
                {
                    self.warn(
                        l.attribute("id"),
                        "identifier of a conversation link is generated (NODE-link-POOL)",
                        None,
                    );
                }
            }
        }
        let mut attrs = Attrs::new();
        attrs.insert("pools".into(), pools.join(" "));
        let messages: Vec<String> = ref_texts(e, "messageFlowRef")
            .iter()
            .map(|r| self.tid_s(Some(r)))
            .collect();
        if !messages.is_empty() {
            attrs.insert("messages".into(), messages.join(" "));
        }
        if kind == "callconversation" {
            if let Some(c) = e
                .attribute("calledCollaborationRef")
                .filter(|c| !c.is_empty())
            {
                let called = if self
                    .by_id
                    .get(c)
                    .is_some_and(|x| local(*x) == "choreography")
                {
                    self.tid_s(Some(c))
                } else {
                    c.to_string()
                };
                attrs.insert("collaboration".into(), called);
            }
            let map = self.associations_06(e);
            if !map.is_empty() {
                attrs.insert("map".into(), map);
            }
        }
        for k in findall(e, "correlationKey") {
            self.warn(
                k.attribute("id"),
                "correlation key of a conversation node is not kept",
                None,
            );
        }
        self.doc(e, &mut attrs);
        let base = Base::new(
            kind,
            Some(nid.clone()),
            clean_label(e.attribute("name")),
            attrs,
            Vec::new(),
            0,
        );
        self.doc.ids.insert(
            nid.clone(),
            IdEntry {
                kind: kind.to_string(),
                line: 0,
            },
        );
        if let Some(p) = &parent
            && let Some(x) = self.doc.conversations.get_mut(p)
        {
            x.children.push(nid.clone());
        }
        self.doc.conversations.insert(
            nid.clone(),
            Conversation {
                base,
                parent,
                children: Vec::new(),
            },
        );
        self.uses_06();
        if kind == "subconversation" {
            let inner: Vec<X> = children(e)
                .filter(|x| {
                    is_bpmn(*x)
                        && matches!(
                            local(*x),
                            "conversation" | "subConversation" | "callConversation"
                        )
                })
                .collect();
            for n in inner {
                self.conversation_node(n, Some(nid.clone()), links);
            }
        }
    }
}
