//! BPMN 2.0 rules (`validate`), the Camunda 8 profile (`validate_camunda`) and
//! `summarize`, ported from `bpmn.py`. Messages are Polish and identical to it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::{
    Model, is_act, is_choreo_act, is_container, is_data, is_evt, is_flownode, is_gw, is_subp,
    local, m, ns_uri, tag_pl_or,
};
use crate::pyfmt::capitalize;
use crate::xml::{BPMN_NS, BPMNDI_NS, Doc, Id, qn};

/// Severity of a diagnostic (`poziom` in `bpmn.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `błąd`
    Error,
    /// `ostrzeżenie`
    Warning,
    /// `uwaga`
    Note,
}

impl Level {
    /// The Polish name used by `bpmn.py`.
    #[must_use]
    pub fn as_pl(self) -> &'static str {
        match self {
            Level::Error => "błąd",
            Level::Warning => "ostrzeżenie",
            Level::Note => "uwaga",
        }
    }
}

/// One diagnostic of the BPMN validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub level: Level,
    pub code: String,
    pub id: Option<String>,
    pub message: String,
}

/// Options of `validate` that correspond to `--kroki` and `--opis` of `bpmn.py`.
#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    /// Require step numbers at the start of element names (`--kroki`).
    pub kroki: bool,
    /// Step numbers of the process description (`--opis`); enables K02/K03.
    pub opis_kroki: Option<Vec<String>>,
}

struct Out(Vec<Issue>);

impl Out {
    fn add(&mut self, level: Level, code: &str, id: Option<&str>, message: String) {
        self.0.push(Issue {
            level,
            code: code.to_string(),
            id: id.map(str::to_string),
            message,
        });
    }
}

fn py(v: Option<&str>) -> &str {
    v.unwrap_or("None")
}

fn def_pl(d: &str) -> String {
    match d {
        "" => "zwykłe (bez wyzwalacza)",
        "messageEventDefinition" => "wiadomość",
        "timerEventDefinition" => "czasowe",
        "signalEventDefinition" => "sygnał",
        "conditionalEventDefinition" => "warunkowe",
        "errorEventDefinition" => "błąd",
        "escalationEventDefinition" => "eskalacja",
        "compensateEventDefinition" => "kompensacja",
        "cancelEventDefinition" => "anulowanie",
        "terminateEventDefinition" => "zakończenie (terminate)",
        "linkEventDefinition" => "łącznik (link)",
        other => other,
    }
    .to_string()
}

fn allowed_defs(kind: &str) -> &'static [&'static str] {
    match kind {
        "start_top" => &[
            "",
            "messageEventDefinition",
            "timerEventDefinition",
            "signalEventDefinition",
            "conditionalEventDefinition",
        ],
        "start_eventsub" => &[
            "",
            "messageEventDefinition",
            "timerEventDefinition",
            "signalEventDefinition",
            "conditionalEventDefinition",
            "errorEventDefinition",
            "escalationEventDefinition",
            "compensateEventDefinition",
        ],
        "start_sub" => &[""],
        "endEvent" => &[
            "",
            "messageEventDefinition",
            "errorEventDefinition",
            "escalationEventDefinition",
            "signalEventDefinition",
            "terminateEventDefinition",
            "compensateEventDefinition",
            "cancelEventDefinition",
        ],
        "intermediateCatchEvent" => &[
            "messageEventDefinition",
            "timerEventDefinition",
            "signalEventDefinition",
            "conditionalEventDefinition",
            "linkEventDefinition",
        ],
        "intermediateThrowEvent" => &[
            "",
            "messageEventDefinition",
            "escalationEventDefinition",
            "signalEventDefinition",
            "compensateEventDefinition",
            "linkEventDefinition",
        ],
        _ => &[
            "messageEventDefinition",
            "timerEventDefinition",
            "signalEventDefinition",
            "conditionalEventDefinition",
            "errorEventDefinition",
            "escalationEventDefinition",
            "compensateEventDefinition",
            "cancelEventDefinition",
        ],
    }
}

const REF_ATTRS: [&str; 13] = [
    "sourceRef",
    "targetRef",
    "attachedToRef",
    "default",
    "processRef",
    "messageRef",
    "signalRef",
    "errorRef",
    "escalationRef",
    "dataObjectRef",
    "dataStoreRef",
    "categoryValueRef",
    "activityRef",
];
const TEXT_REFS: [&str; 5] = [
    "flowNodeRef",
    "incoming",
    "outgoing",
    "sourceRef",
    "targetRef",
];

fn text_of(md: &Model, e: Id) -> String {
    md.doc.text(e).unwrap_or("").to_string()
}

/// Python `any((c.text or "").strip() for c in e)`.
fn any_child_text(md: &Model, e: Id) -> bool {
    md.doc
        .children(e)
        .iter()
        .any(|c| !text_of(md, *c).trim().is_empty())
}

/// Run all BPMN rules on a model.
#[allow(clippy::too_many_lines)]
pub fn validate_model(md: &Model, opts: &ValidateOptions) -> Vec<Issue> {
    let mut o = Out(Vec::new());
    let d = &md.doc;
    let r = d.root;
    let all = d.iter(r);
    if d.tag(r) != m("definitions") {
        o.add(
            Level::Error,
            "E002",
            None,
            "Korzeń dokumentu musi być <bpmn:definitions> w przestrzeni nazw \
             http://www.omg.org/spec/BPMN/20100524/MODEL."
                .to_string(),
        );
        return o.0;
    }
    let mut dups: Vec<&String> = md.dups.iter().collect::<HashSet<_>>().into_iter().collect();
    dups.sort();
    for dd in dups {
        o.add(
            Level::Error,
            "E003",
            Some(dd),
            format!("Identyfikator „{dd}” występuje więcej niż raz."),
        );
    }

    // --- references
    for &e in &all {
        let t = md.tag(e);
        if ns_uri(d.tag(e)) != BPMN_NS {
            continue;
        }
        for a in REF_ATTRS {
            if let Some(v) = md.attr(e, a)
                && !v.is_empty()
                && !md.ids.contains_key(v)
            {
                o.add(
                    Level::Error,
                    "E004",
                    md.id_of(e),
                    format!(
                        "{} {}: atrybut {}=\"{}\" wskazuje na nieistniejący element.",
                        t,
                        md.id_of(e).unwrap_or(""),
                        a,
                        v
                    ),
                );
            }
        }
        if TEXT_REFS.contains(&t.as_str()) {
            let txt = text_of(md, e);
            let v = txt.trim();
            if !v.is_empty() && !md.ids.contains_key(v) {
                let owner = md.parent.get(&e).copied();
                let oid = owner.and_then(|x| md.id_of(x));
                let oshow = match owner {
                    Some(_) => py(oid).to_string(),
                    None => "?".to_string(),
                };
                o.add(
                    Level::Error,
                    "E004",
                    oid,
                    format!(
                        "<{t}>{v}</{t}> w elemencie {oshow} wskazuje na nieistniejący element."
                    ),
                );
            }
        }
    }

    // --- types of referenced elements
    let ref_types: [(&str, &str, &str); 7] = [
        ("messageRef", "message", "wiadomość (<bpmn:message>)"),
        ("signalRef", "signal", "sygnał (<bpmn:signal>)"),
        ("errorRef", "error", "błąd (<bpmn:error>)"),
        (
            "escalationRef",
            "escalation",
            "eskalację (<bpmn:escalation>)",
        ),
        ("processRef", "process", "proces (<bpmn:process>)"),
        (
            "dataObjectRef",
            "dataObject",
            "obiekt danych (<bpmn:dataObject>)",
        ),
        (
            "dataStoreRef",
            "dataStore",
            "magazyn danych (<bpmn:dataStore>)",
        ),
    ];
    for &e in &all {
        if ns_uri(d.tag(e)) != BPMN_NS {
            continue;
        }
        for (a, want, opis) in ref_types {
            let v = md.attr(e, a);
            let Some(tgt) = md.get(v) else {
                continue;
            };
            if md.tag(tgt) != want {
                let owner = if md.id_of(e).is_some_and(|i| !i.is_empty())
                    && !md.tag(e).ends_with("EventDefinition")
                {
                    Some(e)
                } else {
                    md.parent.get(&e).copied()
                };
                o.add(
                    Level::Error,
                    "E018",
                    owner.and_then(|x| md.id_of(x)),
                    format!(
                        "{}: atrybut {}=\"{}\" powinien wskazywać {}, a wskazuje {}.",
                        md.label(owner),
                        a,
                        py(v),
                        opis,
                        md.tag(tgt)
                    ),
                );
            }
        }
    }

    // --- sequence flows
    for &f in &md.seq {
        let fid = md.id_of(f);
        let sr = md.attr(f, "sourceRef").filter(|s| !s.is_empty());
        let tr = md.attr(f, "targetRef").filter(|s| !s.is_empty());
        if sr.is_none() || tr.is_none() {
            o.add(
                Level::Error,
                "E005",
                fid,
                format!(
                    "Przepływ sekwencji {} musi mieć atrybuty sourceRef i targetRef.",
                    py(fid)
                ),
            );
            continue;
        }
        let (Some(s), Some(t)) = (md.get(sr), md.get(tr)) else {
            continue;
        };
        if !is_flownode(&md.tag(s)) || !is_flownode(&md.tag(t)) {
            o.add(
                Level::Error,
                "E005",
                fid,
                format!(
                    "Przepływ sekwencji {} łączy {} z {}. Przepływ sekwencji może łączyć tylko zadania, zdarzenia \
                     i bramki (nie obiekty danych, baseny ani adnotacje).",
                    py(fid),
                    md.tag(s),
                    md.tag(t)
                ),
            );
            continue;
        }
        let c = md.parent.get(&f).copied();
        if md.container(s) != c || md.container(t) != c {
            o.add(
                Level::Error,
                "E006",
                fid,
                format!(
                    "Przepływ sekwencji {} ({} → {}) przekracza granicę procesu lub podprocesu. Między basenami \
                     używa się przepływu komunikatów; wnętrze podprocesu ma własne zdarzenie początkowe.",
                    py(fid),
                    md.label(Some(s)),
                    md.label(Some(t))
                ),
            );
        }
    }

    // --- message flows
    for &f in &md.msg {
        let fid = md.id_of(f);
        let (Some(s), Some(t)) = (
            md.get(md.attr(f, "sourceRef")),
            md.get(md.attr(f, "targetRef")),
        ) else {
            continue;
        };
        let ps = md.participant_of(Some(s));
        let pt = md.participant_of(Some(t));
        if ps.is_none() || pt.is_none() {
            o.add(
                Level::Error,
                "E007",
                fid,
                format!(
                    "Przepływ komunikatów {}: jeden z końców nie należy do żadnej basenu (uczestnika). \
                     Komunikaty płyną między basenami.",
                    py(fid)
                ),
            );
        } else if ps == pt {
            o.add(
                Level::Error,
                "E008",
                fid,
                format!(
                    "Przepływ komunikatów {} łączy elementy tego samego basenu {} ({} → {}). Wewnątrz jednego \
                     basenu, także między liniami, używa się przepływu sekwencji; komunikaty płyną tylko między \
                     basenami.",
                    py(fid),
                    md.label(ps),
                    md.label(Some(s)),
                    md.label(Some(t))
                ),
            );
        }
        for (x, role) in [(s, "źródłem"), (t, "celem")] {
            let tx = md.tag(x);
            if is_gw(&tx) || is_data(&tx) || tx == "textAnnotation" {
                o.add(
                    Level::Error,
                    "E009",
                    fid,
                    format!(
                        "Przepływ komunikatów {}: {} nie może być {} ({}).",
                        py(fid),
                        tag_pl_or(&tx),
                        role,
                        md.label(Some(x))
                    ),
                );
            } else if is_evt(&tx) {
                let defs = md.eventdefs(x);
                let has_msg = defs.iter().any(|d| d == "messageEventDefinition");
                if role == "źródłem" {
                    let ok = (tx == "intermediateThrowEvent" || tx == "endEvent") && has_msg;
                    if !ok {
                        o.add(
                            Level::Error,
                            "E009",
                            fid,
                            format!(
                                "Przepływ komunikatów {} wychodzi ze zdarzenia {}, które nie jest zdarzeniem \
                                 rzucającym typu wiadomość.",
                                py(fid),
                                md.label(Some(x))
                            ),
                        );
                    }
                } else {
                    let ok = matches!(
                        tx.as_str(),
                        "startEvent" | "intermediateCatchEvent" | "boundaryEvent"
                    ) && has_msg;
                    if !ok {
                        o.add(
                            Level::Error,
                            "E009",
                            fid,
                            format!(
                                "Przepływ komunikatów {} wchodzi do zdarzenia {}, które nie jest zdarzeniem \
                                 przechwytującym typu wiadomość.",
                                py(fid),
                                md.label(Some(x))
                            ),
                        );
                    }
                }
            }
        }
    }

    // --- events
    for &e in &all {
        let t = md.tag(e);
        if !is_evt(&t) || ns_uri(d.tag(e)) != BPMN_NS {
            continue;
        }
        let eid = md.id_of(e);
        let defs = md.eventdefs(e);
        let cont = md.container(e);
        let mut kind = t.clone();
        if t == "startEvent" {
            kind = match cont {
                Some(c) if md.is_event_sub(c) => "start_eventsub".to_string(),
                Some(c) if is_subp(&md.tag(c)) => "start_sub".to_string(),
                _ => "start_top".to_string(),
            };
        }
        let allowed = allowed_defs(&kind);
        let has_ref = defs.iter().any(|d| d == "ref");
        let mut dset: Vec<String> = defs.iter().filter(|d| *d != "ref").cloned().collect();
        if dset.is_empty() && !has_ref {
            dset.push(String::new());
        }
        for dd in &dset {
            if !allowed.contains(&dd.as_str()) {
                let where_ = match kind.as_str() {
                    "start_top" => "zdarzenie początkowe procesu".to_string(),
                    "start_eventsub" => "zdarzenie początkowe podprocesu zdarzeniowego".to_string(),
                    "start_sub" => {
                        "zdarzenie początkowe zwykłego podprocesu (musi być bez wyzwalacza)"
                            .to_string()
                    }
                    other => tag_pl_or(other),
                };
                o.add(
                    Level::Error,
                    "E010",
                    eid,
                    format!(
                        "{}: wyzwalacz „{}” jest niedozwolony dla: {}.",
                        md.label(Some(e)),
                        def_pl(dd),
                        where_
                    ),
                );
            }
        }
        if defs.iter().any(|d| d == "cancelEventDefinition") {
            let host = if t == "boundaryEvent" {
                md.get(md.attr(e, "attachedToRef"))
            } else {
                cont
            };
            if host.is_none_or(|h| md.tag(h) != "transaction") {
                o.add(
                    Level::Error,
                    "E010",
                    eid,
                    format!(
                        "{}: zdarzenie anulowania dopuszczalne jest tylko w transakcji lub na jej granicy.",
                        md.label(Some(e))
                    ),
                );
            }
        }
        if t == "startEvent" && !md.inc_of(eid).is_empty() {
            o.add(
                Level::Error,
                "E011",
                eid,
                format!(
                    "{}: zdarzenie początkowe nie może mieć przepływu wchodzącego.",
                    md.label(Some(e))
                ),
            );
        }
        if t == "endEvent" && !md.out_of(eid).is_empty() {
            o.add(
                Level::Error,
                "E011",
                eid,
                format!(
                    "{}: zdarzenie końcowe nie może mieć przepływu wychodzącego.",
                    md.label(Some(e))
                ),
            );
        }
        if t == "boundaryEvent" {
            let host = md.get(md.attr(e, "attachedToRef"));
            if let Some(h) = host
                && !is_act(&md.tag(h))
                && !is_choreo_act(&md.tag(h))
            {
                o.add(
                        Level::Error,
                        "E012",
                        eid,
                        format!(
                            "{}: zdarzenie graniczne musi być przyczepione do zadania lub podprocesu, a jest do {}.",
                            md.label(Some(e)),
                            md.label(Some(h))
                        ),
                    );
            }
            if !md.inc_of(eid).is_empty() {
                o.add(
                    Level::Error,
                    "E012",
                    eid,
                    format!(
                        "{}: zdarzenie graniczne nie może mieć przepływu wchodzącego.",
                        md.label(Some(e))
                    ),
                );
            }
            if defs.iter().any(|d| d == "errorEventDefinition")
                && md.attr(e, "cancelActivity") == Some("false")
            {
                o.add(
                    Level::Error,
                    "E012",
                    eid,
                    format!(
                        "{}: graniczne zdarzenie błędu zawsze przerywa czynność (cancelActivity nie może być false).",
                        md.label(Some(e))
                    ),
                );
            }
            if defs.iter().any(|d| d == "compensateEventDefinition") && !md.out_of(eid).is_empty() {
                o.add(
                    Level::Error,
                    "E012",
                    eid,
                    format!(
                        "{}: graniczne zdarzenie kompensacji łączy się z obsługą asocjacją, nie przepływem sekwencji.",
                        md.label(Some(e))
                    ),
                );
            }
        }
        for td in d.findall(e, &m("timerEventDefinition")) {
            if !any_child_text(md, td) {
                o.add(
                    Level::Warning,
                    "W010",
                    eid,
                    format!(
                        "{}: zdarzenie czasowe bez określenia czasu (timeDuration, timeDate lub timeCycle).",
                        md.label(Some(e))
                    ),
                );
            }
        }
        for mdf in d.findall(e, &m("messageEventDefinition")) {
            if md.attr(mdf, "messageRef").is_none_or(str::is_empty) {
                o.add(
                    Level::Note,
                    "L010",
                    eid,
                    format!(
                        "{}: zdarzenie wiadomości bez messageRef (warto zadeklarować <bpmn:message> i wskazać ją).",
                        md.label(Some(e))
                    ),
                );
            }
        }
    }

    // --- event sub-processes and compensation handlers
    for e in d.iter_tag(r, &m("subProcess")) {
        if !md.is_event_sub(e) {
            continue;
        }
        let eid = md.id_of(e);
        if !md.inc_of(eid).is_empty() || !md.out_of(eid).is_empty() {
            o.add(
                Level::Error,
                "E013",
                eid,
                format!(
                    "{}: podproces zdarzeniowy nie ma przepływów wchodzących ani wychodzących — uruchamia go jego \
                     zdarzenie początkowe.",
                    md.label(Some(e))
                ),
            );
        }
        let starts = d.findall(e, &m("startEvent"));
        if starts.len() != 1 {
            o.add(
                Level::Error,
                "E013",
                eid,
                format!(
                    "{}: podproces zdarzeniowy musi mieć dokładnie jedno zdarzenie początkowe (jest {}).",
                    md.label(Some(e)),
                    starts.len()
                ),
            );
        }
        for s in starts {
            if md.eventdefs(s).is_empty() {
                o.add(
                    Level::Error,
                    "E013",
                    md.id_of(s),
                    format!(
                        "{}: zdarzenie początkowe podprocesu zdarzeniowego musi mieć wyzwalacz (np. wiadomość, \
                         sygnał, błąd, eskalację).",
                        md.label(Some(s))
                    ),
                );
            }
        }
    }
    for &e in &all {
        if is_act(&md.tag(e)) && md.attr(e, "isForCompensation") == Some("true") {
            let eid = md.id_of(e);
            if !md.inc_of(eid).is_empty() || !md.out_of(eid).is_empty() {
                o.add(
                    Level::Error,
                    "E014",
                    eid,
                    format!(
                        "{}: czynność kompensacyjna nie ma przepływów sekwencji.",
                        md.label(Some(e))
                    ),
                );
            }
        }
    }

    // --- gateways and activities
    for &e in &all {
        let t = md.tag(e);
        if !is_gw(&t) && !is_act(&t) {
            continue;
        }
        let eid = md.id_of(e);
        let outs = md.out_of(eid);
        let ins = md.inc_of(eid);
        let dflt = md.attr(e, "default").filter(|s| !s.is_empty());
        if let Some(dfid) = dflt {
            let df = md.get(Some(dfid));
            if df.is_none_or(|x| md.attr(x, "sourceRef") != eid) {
                o.add(
                    Level::Error,
                    "E015",
                    eid,
                    format!(
                        "{}: przepływ domyślny {} nie wychodzi z tego elementu.",
                        md.label(Some(e)),
                        dfid
                    ),
                );
            } else if df.is_some_and(|x| d.find(x, &m("conditionExpression")).is_some()) {
                o.add(
                    Level::Error,
                    "E015",
                    eid,
                    format!(
                        "{}: przepływ domyślny {} nie może mieć warunku.",
                        md.label(Some(e)),
                        dfid
                    ),
                );
            }
        }
        if t == "eventBasedGateway" {
            if outs.len() < 2 {
                o.add(
                    Level::Error,
                    "E016",
                    eid,
                    format!(
                        "{}: bramka oparta na zdarzeniach potrzebuje co najmniej dwóch wyjść.",
                        md.label(Some(e))
                    ),
                );
            }
            for &f in outs {
                let Some(tg) = md.get(md.attr(f, "targetRef")) else {
                    continue;
                };
                let tt = md.tag(tg);
                let ok = tt == "receiveTask"
                    || (tt == "intermediateCatchEvent"
                        && md.eventdefs(tg).iter().any(|x| {
                            matches!(
                                x.as_str(),
                                "messageEventDefinition"
                                    | "timerEventDefinition"
                                    | "signalEventDefinition"
                                    | "conditionalEventDefinition"
                            )
                        }));
                if !ok {
                    o.add(
                        Level::Error,
                        "E016",
                        eid,
                        format!(
                            "{}: po bramce opartej na zdarzeniach musi wystąpić pośrednie zdarzenie przechwytujące \
                             (wiadomość, czas, sygnał, warunek) lub zadanie odbioru, a jest {}.",
                            md.label(Some(e)),
                            md.label(Some(tg))
                        ),
                    );
                } else if md.inc_of(md.id_of(tg)).len() > 1 {
                    o.add(
                        Level::Warning,
                        "W016",
                        md.id_of(tg),
                        format!(
                            "{}: zdarzenie po bramce opartej na zdarzeniach nie powinno mieć innych przepływów \
                             wchodzących.",
                            md.label(Some(tg))
                        ),
                    );
                }
                if d.find(f, &m("conditionExpression")).is_some() {
                    o.add(
                        Level::Error,
                        "E016",
                        md.id_of(f),
                        format!(
                            "Przepływy z bramki opartej na zdarzeniach nie mają warunków ({}).",
                            py(md.id_of(f))
                        ),
                    );
                }
            }
        }
        if t == "parallelGateway" {
            for &f in outs {
                if d.find(f, &m("conditionExpression")).is_some() {
                    o.add(
                        Level::Error,
                        "E017",
                        md.id_of(f),
                        format!(
                            "Przepływ {} z bramki równoległej {} nie może mieć warunku.",
                            py(md.id_of(f)),
                            md.label(Some(e))
                        ),
                    );
                }
            }
            if dflt.is_some() {
                o.add(
                    Level::Error,
                    "E017",
                    eid,
                    format!(
                        "{}: bramka równoległa nie ma przepływu domyślnego.",
                        md.label(Some(e))
                    ),
                );
            }
        }
        if is_gw(&t) {
            if ins.len() == 1 && outs.len() == 1 {
                o.add(
                    Level::Warning,
                    "W020",
                    eid,
                    format!(
                        "{}: bramka z jednym wejściem i jednym wyjściem jest zbędna.",
                        md.label(Some(e))
                    ),
                );
            }
            if matches!(
                t.as_str(),
                "exclusiveGateway" | "inclusiveGateway" | "complexGateway"
            ) && outs.len() > 1
            {
                for &f in outs {
                    if md.id_of(f) == md.attr(e, "default")
                        || d.find(f, &m("conditionExpression")).is_some()
                    {
                        continue;
                    }
                    if md.attr(f, "name").unwrap_or("").trim().is_empty() {
                        o.add(
                            Level::Warning,
                            "W021",
                            md.id_of(f),
                            format!(
                                "Gałąź {} bramki {} nie ma ani etykiety, ani warunku, ani statusu przepływu \
                                 domyślnego.",
                                py(md.id_of(f)),
                                md.label(Some(e))
                            ),
                        );
                    }
                }
                let nm = md.name(Some(e));
                if !nm.is_empty() && !nm.ends_with('?') {
                    o.add(
                        Level::Note,
                        "L020",
                        eid,
                        format!(
                            "{}: nazwa rozgałęziającej bramki powinna być pytaniem (kończyć się „?”).",
                            md.label(Some(e))
                        ),
                    );
                }
            }
            if t == "inclusiveGateway" && outs.len() > 1 && dflt.is_none() {
                o.add(
                    Level::Warning,
                    "W050",
                    eid,
                    format!(
                        "{}: rozgałęziająca bramka inkluzywna (OR) powinna mieć przepływ domyślny (atrybut \
                         default), używany gdy żaden warunek nie jest spełniony.",
                        md.label(Some(e))
                    ),
                );
            }
            if ins.len() > 1 && outs.len() > 1 {
                o.add(
                    Level::Note,
                    "L021",
                    eid,
                    format!(
                        "{}: bramka jednocześnie scala i rozgałęzia; czytelniej rozdzielić to na dwie bramki.",
                        md.label(Some(e))
                    ),
                );
            }
        }
        if is_act(&t) && md.name(Some(e)).is_empty() && !md.is_event_sub(e) {
            o.add(
                Level::Note,
                "L030",
                eid,
                format!("{} bez nazwy ({}).", capitalize(&tag_pl_or(&t)), py(eid)),
            );
        }
    }

    // --- lanes
    for &proc in &md.processes {
        let lanes = md.lanes_of(proc);
        if lanes.is_empty() {
            continue;
        }
        let direct: HashSet<Option<String>> = d
            .children(proc)
            .iter()
            .filter(|c| is_flownode(&md.tag(**c)))
            .map(|c| md.id_of(*c).map(str::to_string))
            .collect();
        let mut seen: HashMap<String, Option<String>> = HashMap::new();
        for (lane, _d, _p) in &lanes {
            if !md.is_leaf_lane(*lane) {
                continue;
            }
            let lname = md
                .attr(*lane, "name")
                .filter(|s| !s.is_empty())
                .or_else(|| md.id_of(*lane))
                .map(str::to_string);
            for rf in d.findall(*lane, &m("flowNodeRef")) {
                let v = text_of(md, rf).trim().to_string();
                if v.is_empty() {
                    continue;
                }
                if let Some(prev) = seen.get(&v) {
                    o.add(
                        Level::Error,
                        "E030",
                        Some(&v),
                        format!(
                            "Element {} jest przypisany do dwóch linii ({} i {}).",
                            md.label(md.get(Some(&v))),
                            py(prev.as_deref()),
                            py(lname.as_deref())
                        ),
                    );
                }
                seen.insert(v.clone(), lname.clone());
                if md.ids.contains_key(&v) && !direct.contains(&Some(v.clone())) {
                    o.add(
                        Level::Warning,
                        "W030",
                        Some(&v),
                        format!(
                            "Linia {} wskazuje element {}, który nie jest bezpośrednim elementem procesu {} \
                             (elementy wnętrza podprocesu nie należą do linii).",
                            py(lname.as_deref()),
                            v,
                            py(md.id_of(proc))
                        ),
                    );
                }
            }
        }
        for &c in d.children(proc) {
            let ct = md.tag(c);
            let in_seen = md.id_of(c).is_some_and(|i| seen.contains_key(i));
            if is_flownode(&ct) && ct != "boundaryEvent" && !in_seen {
                o.add(
                    Level::Warning,
                    "W031",
                    md.id_of(c),
                    format!(
                        "{} nie jest przypisany do żadnej linii (dodaj <bpmn:flowNodeRef> w odpowiednim \
                         <bpmn:lane>).",
                        md.label(Some(c))
                    ),
                );
            }
        }
    }

    // --- reachability and dead ends
    for &cont in &all {
        if !is_container(&md.tag(cont)) || ns_uri(d.tag(cont)) != BPMN_NS {
            continue;
        }
        let nodes: Vec<Id> = d
            .children(cont)
            .iter()
            .copied()
            .filter(|c| is_flownode(&md.tag(*c)))
            .collect();
        if nodes.is_empty() {
            continue;
        }
        // an instantiating receive task or event-based gateway starts the process like a
        // start event (BPMN 2.0.2, 10.3.3 Receive Task and 10.6.6)
        let starts: Vec<Option<String>> = nodes
            .iter()
            .filter(|n| {
                md.tag(**n) == "startEvent"
                    || (matches!(md.tag(**n).as_str(), "receiveTask" | "eventBasedGateway")
                        && md.attr(**n, "instantiate") == Some("true"))
            })
            .map(|n| md.id_of(*n).map(str::to_string))
            .collect();
        let is_top = md.tag(cont) == "process";
        if starts.is_empty() {
            if is_top {
                o.add(
                    Level::Warning,
                    "W040",
                    md.id_of(cont),
                    format!(
                        "Proces {} nie ma zdarzenia początkowego.",
                        md.label(Some(cont))
                    ),
                );
            }
            continue;
        }
        if is_top && !nodes.iter().any(|n| md.tag(*n) == "endEvent") {
            o.add(
                Level::Warning,
                "W040",
                md.id_of(cont),
                format!(
                    "Proces {} nie ma zdarzenia końcowego.",
                    md.label(Some(cont))
                ),
            );
        }
        let mut links: Vec<(Option<String>, Vec<Option<String>>)> = Vec::new();
        for &n in &nodes {
            if md.tag(n) == "intermediateCatchEvent" {
                for ld in d.findall(n, &m("linkEventDefinition")) {
                    let key = md.attr(ld, "name").map(str::to_string);
                    let nid = md.id_of(n).map(str::to_string);
                    match links.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, v)) => v.push(nid),
                        None => links.push((key, vec![nid])),
                    }
                }
            }
        }
        let mut seen: HashSet<Option<String>> = HashSet::new();
        let mut stack: Vec<Option<String>> = starts.clone();
        while let Some(u) = stack.pop() {
            if seen.contains(&u) {
                continue;
            }
            seen.insert(u.clone());
            for &f in md.out_of(u.as_deref()) {
                stack.push(md.attr(f, "targetRef").map(str::to_string));
            }
            for &b in md.boundaries_of(u.as_deref()) {
                stack.push(md.id_of(b).map(str::to_string));
            }
            if let Some(ue) = md.get(u.as_deref())
                && md.tag(ue) == "intermediateThrowEvent"
            {
                for ld in d.findall(ue, &m("linkEventDefinition")) {
                    let key = md.attr(ld, "name").map(str::to_string);
                    if let Some((_, v)) = links.iter().find(|(k, _)| *k == key) {
                        stack.extend(v.iter().cloned());
                    }
                }
            }
        }
        for &n in &nodes {
            let nid = md.id_of(n);
            let t = md.tag(n);
            if md.is_event_sub(n) || md.attr(n, "isForCompensation") == Some("true") {
                continue;
            }
            let defs = md.eventdefs(n);
            if t == "boundaryEvent" && defs.iter().any(|x| x == "compensateEventDefinition") {
                continue;
            }
            if !seen.contains(&nid.map(str::to_string)) {
                o.add(
                    Level::Warning,
                    "W041",
                    nid,
                    format!(
                        "{} jest nieosiągalny ze zdarzenia początkowego.",
                        md.label(Some(n))
                    ),
                );
            }
            if t != "endEvent" && md.out_of(nid).is_empty() {
                if t == "intermediateThrowEvent" && defs.iter().any(|x| x == "linkEventDefinition")
                {
                    continue;
                }
                o.add(
                    Level::Warning,
                    "W042",
                    nid,
                    format!(
                        "{} nie ma przepływu wychodzącego — ścieżka powinna kończyć się zdarzeniem końcowym.",
                        md.label(Some(n))
                    ),
                );
            }
        }
    }

    // --- pairing of splits and joins
    let trace_back = |f: Id| -> Option<Id> {
        let mut src = md.get(md.attr(f, "sourceRef"));
        let mut guard = 0;
        while let Some(s) = src {
            if is_gw(&md.tag(s)) || guard >= 500 {
                break;
            }
            let ins_ = md.inc_of(md.id_of(s));
            if ins_.len() != 1 || md.out_of(md.id_of(s)).len() != 1 {
                return None;
            }
            src = md.get(md.attr(ins_[0], "sourceRef"));
            guard += 1;
        }
        src
    };
    for &j in &all {
        let jt = md.tag(j);
        if !is_gw(&jt) || md.inc_of(md.id_of(j)).len() < 2 {
            continue;
        }
        let origins: Vec<Option<Id>> = md
            .inc_of(md.id_of(j))
            .iter()
            .map(|f| trace_back(*f))
            .collect();
        if origins.iter().any(Option::is_none) {
            continue;
        }
        let oids: HashSet<Option<&str>> = origins
            .iter()
            .map(|x| x.and_then(|y| md.id_of(y)))
            .collect();
        if oids.len() != 1 {
            continue;
        }
        let Some(g) = origins[0] else {
            continue;
        };
        let gt = md.tag(g);
        if md.out_of(md.id_of(g)).len() < 2 || gt == jt {
            continue;
        }
        if gt == "eventBasedGateway" && jt == "exclusiveGateway" {
            continue;
        }
        // a complex gateway joins any split by its activation condition, and the branches
        // of a complex split are joined like those of an inclusive one
        if jt == "complexGateway" || (gt == "complexGateway" && jt == "inclusiveGateway") {
            continue;
        }
        let skutek = match gt.as_str() {
            "parallelGateway" => {
                "wszystkie gałęzie dotrą do złączenia, więc dalsza część procesu wykona się kilka razy"
            }
            "exclusiveGateway" => {
                "tylko jedna gałąź dotrze do złączenia, więc złączenie może czekać w nieskończoność (zakleszczenie)"
            }
            "inclusiveGateway" => {
                "liczba aktywnych gałęzi jest zmienna; złącz je bramką inkluzywną (OR)"
            }
            "eventBasedGateway" => {
                "aktywna jest tylko jedna gałąź; złącz je bramką wykluczającą (XOR)"
            }
            "complexGateway" => "złączenie powinno mieć ten sam typ co rozwidlenie",
            _ => "",
        };
        o.add(
            Level::Warning,
            "W051",
            md.id_of(j),
            format!(
                "{} ({}) łączy gałęzie rozwidlone przez {} ({}): {}.",
                md.label(Some(j)),
                tag_pl_or(&jt),
                md.label(Some(g)),
                tag_pl_or(&gt),
                skutek
            ),
        );
    }

    // --- escalations and errors caught on sub-processes
    for b in d.iter_tag(r, &m("boundaryEvent")) {
        let Some(host) = md.get(md.attr(b, "attachedToRef")) else {
            continue;
        };
        if !is_subp(&md.tag(host)) {
            continue;
        }
        for (dname, ra, rodzaj, zaimek) in [
            (
                "escalationEventDefinition",
                "escalationRef",
                "eskalację",
                "jej",
            ),
            ("errorEventDefinition", "errorRef", "błąd", "go"),
        ] {
            for dd in d.findall(b, &m(dname)) {
                let rf = md.attr(dd, ra);
                let throwers = d.iter(host).into_iter().any(|x| {
                    x != host
                        && matches!(md.tag(x).as_str(), "intermediateThrowEvent" | "endEvent")
                        && d.findall(x, &m(dname))
                            .iter()
                            .any(|y| rf.is_none() || md.attr(*y, ra) == rf)
                });
                if !throwers {
                    let shown = match rf {
                        Some(v) if !v.is_empty() => format!(" „{v}”"),
                        _ => String::new(),
                    };
                    o.add(
                        Level::Warning,
                        "W052",
                        md.id_of(b),
                        format!(
                            "{} łapie {}{} na podprocesie {}, ale nic wewnątrz podprocesu {} nie rzuca — zdarzenie \
                             nigdy nie wystąpi.",
                            md.label(Some(b)),
                            rodzaj,
                            shown,
                            md.label(Some(host)),
                            zaimek
                        ),
                    );
                }
            }
        }
    }
    for &x in &all {
        if !matches!(md.tag(x).as_str(), "intermediateThrowEvent" | "endEvent") {
            continue;
        }
        for dd in d.findall(x, &m("escalationEventDefinition")) {
            let rf = md.attr(dd, "escalationRef");
            let matches_ref = |y: Id| {
                let v = md.attr(y, "escalationRef");
                v.is_none() || v == rf
            };
            let mut caught = false;
            let mut anc = md.container(x);
            while let Some(a) = anc {
                if caught {
                    break;
                }
                for &b in md.boundaries_of(md.id_of(a)) {
                    for ee in d.findall(b, &m("escalationEventDefinition")) {
                        if matches_ref(ee) {
                            caught = true;
                        }
                    }
                }
                for es in d.iter_tag(a, &m("subProcess")) {
                    if md.is_event_sub(es) {
                        for st in d.findall(es, &m("startEvent")) {
                            for ee in d.findall(st, &m("escalationEventDefinition")) {
                                if matches_ref(ee) {
                                    caught = true;
                                }
                            }
                        }
                    }
                }
                anc = md.container(a);
            }
            if !caught {
                o.add(
                    Level::Warning,
                    "W053",
                    md.id_of(x),
                    format!(
                        "{} rzuca eskalację, której nic nie łapie (brak granicznego zdarzenia eskalacji z tym samym \
                         escalationRef na otaczającym podprocesie).",
                        md.label(Some(x))
                    ),
                );
            }
        }
    }

    // --- message elements without message flows
    if md.participants.len() >= 2 {
        let mut ends: HashSet<Option<String>> = HashSet::new();
        for &f in &md.msg {
            ends.insert(md.attr(f, "sourceRef").map(str::to_string));
            ends.insert(md.attr(f, "targetRef").map(str::to_string));
        }
        for &e in &all {
            let t = md.tag(e);
            let is_msg = matches!(t.as_str(), "sendTask" | "receiveTask")
                || (matches!(
                    t.as_str(),
                    "intermediateThrowEvent" | "intermediateCatchEvent" | "boundaryEvent"
                ) && md
                    .eventdefs(e)
                    .iter()
                    .any(|x| x == "messageEventDefinition"));
            if is_msg && !ends.contains(&md.id_of(e).map(str::to_string)) {
                o.add(
                    Level::Warning,
                    "W054",
                    md.id_of(e),
                    format!(
                        "{} wysyła lub odbiera wiadomość, ale nie łączy go żaden przepływ komunikatów. Jeśli partner \
                         jest osobnym basenem, dodaj messageFlow; jeśli jest wewnątrz tego samego basenu, rozważ inny \
                         typ elementu.",
                        md.label(Some(e))
                    ),
                );
            }
        }
    }

    // --- step numbers (course convention)
    if opts.kroki {
        for &e in &all {
            let t = md.tag(e);
            if !is_flownode(&t) || ns_uri(d.tag(e)) != BPMN_NS || t == "boundaryEvent" {
                continue;
            }
            let cont = md.container(e);
            let defs = md.eventdefs(e);
            if t == "endEvent" && defs.is_empty() {
                continue;
            }
            if t == "startEvent" && defs.is_empty() && cont.is_some_and(|c| is_subp(&md.tag(c))) {
                continue;
            }
            if is_gw(&t) && md.out_of(md.id_of(e)).len() <= 1 && md.name(Some(e)).is_empty() {
                continue;
            }
            if crate::steps::step_match(&md.name(Some(e))).is_none() {
                o.add(
                    Level::Warning,
                    "K01",
                    md.id_of(e),
                    format!(
                        "{}: nazwa powinna zaczynać się numerem kroku z opisu procesu, np. „12. Zapytanie ofertowe”.",
                        md.label(Some(e))
                    ),
                );
            }
        }
    }
    if opts.kroki
        && let Some(opis) = opts.opis_kroki.as_ref().filter(|v| !v.is_empty())
    {
        let opis: HashSet<String> = opis.iter().cloned().collect();
        let mut w_modelu: HashSet<String> = HashSet::new();
        for &e in &all {
            if is_flownode(&md.tag(e)) {
                w_modelu.extend(crate::steps::step_numbers(&md.name(Some(e))));
            }
        }
        let mut missing: Vec<&String> = opis.difference(&w_modelu).collect();
        missing.sort_by_key(|k| crate::steps::step_key(k));
        for k in missing {
            o.add(
                    Level::Warning,
                    "K02",
                    None,
                    format!(
                        "Krok {k} z opisu procesu nie ma w modelu żadnego elementu (nazwa zaczynająca się od „{k}.”)."
                    ),
                );
        }
        let mut extra: Vec<&String> = w_modelu.difference(&opis).collect();
        extra.sort_by_key(|k| crate::steps::step_key(k));
        for k in extra {
            o.add(
                Level::Warning,
                "K03",
                None,
                format!(
                    "Model zawiera element z numerem kroku {k}, którego nie ma w opisie procesu."
                ),
            );
        }
    }

    // --- DI
    if !d.findall(r, &qn(BPMNDI_NS, "BPMNDiagram")).is_empty() {
        let mut have: HashSet<Option<String>> = HashSet::new();
        for &s in &all {
            let t = md.tag(s);
            if t == "BPMNShape" || t == "BPMNEdge" {
                let be = md.attr(s, "bpmnElement");
                have.insert(be.map(str::to_string));
                if be.is_none_or(|b| !md.ids.contains_key(b)) {
                    o.add(
                        Level::Warning,
                        "D01",
                        md.id_of(s),
                        format!(
                            "Współrzędne {} dotyczą elementu {}, którego już nie ma w modelu. Po zmianach w modelu \
                             uruchom: bpmn.py all (odświeży układ).",
                            py(md.id_of(s)),
                            py(be)
                        ),
                    );
                }
            }
        }
        let mut missing: Vec<String> = Vec::new();
        for &e in &all {
            let t = md.tag(e);
            let kind = is_flownode(&t)
                || is_data(&t)
                || matches!(
                    t.as_str(),
                    "sequenceFlow" | "messageFlow" | "participant" | "lane" | "textAnnotation"
                );
            // the message flows of a choreography are drawn as the envelopes of its
            // participant bands, not as edges (BPMN 2.0.2 §12.2.3)
            let in_choreography = t == "messageFlow"
                && md
                    .parent
                    .get(&e)
                    .is_some_and(|p| md.tag(*p) == "choreography");
            if kind
                && !in_choreography
                && ns_uri(d.tag(e)) == BPMN_NS
                && !have.contains(&md.id_of(e).map(str::to_string))
            {
                missing.push(py(md.id_of(e)).to_string());
            }
        }
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
            o.add(
                Level::Warning,
                "D02",
                None,
                format!(
                    "{} elementów nie ma współrzędnych (BPMNDI), np. {}. Uruchom: bpmn.py layout.",
                    missing.len(),
                    shown.join(", ")
                ),
            );
        }
    }
    o.0
}

// ----------------------------------------------------------------- Camunda 8

const ZEEBE: &str = "http://camunda.org/schema/zeebe/1.0";
const MODELER: &str = "http://camunda.org/schema/modeler/1.0";

fn zeebe(md: &Model, e: Id, name: &str) -> Option<Id> {
    let ext = md.doc.find(e, &m("extensionElements"))?;
    md.doc.find(ext, &qn(ZEEBE, name))
}

/// Identifiers of forms (`.form`) and decisions (`.dmn`) in a folder.
fn resource_ids(folder: &Path) -> (HashSet<String>, HashSet<String>) {
    let mut forms = HashSet::new();
    let mut decisions = HashSet::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (forms, decisions);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if name.ends_with(".form") {
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            {
                match v.get("id") {
                    Some(serde_json::Value::String(s)) if !s.is_empty() => {
                        forms.insert(s.clone());
                    }
                    _ => {}
                }
            }
        } else if name.ends_with(".dmn")
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(doc) = Doc::parse(&text)
        {
            for e in doc.iter(doc.root) {
                if doc.tag(e).ends_with("}decision")
                    && let Some(i) = doc.get(e, "id").filter(|s| !s.is_empty())
                {
                    decisions.insert(i.to_string());
                }
            }
        }
    }
    (forms, decisions)
}

fn message_of(md: &Model, e: Id) -> Option<Id> {
    let mut rf = md.attr(e, "messageRef");
    if rf.is_none() {
        rf = md
            .doc
            .find(e, &m("messageEventDefinition"))
            .and_then(|x| md.attr(x, "messageRef"));
    }
    md.get(rf)
}

/// Camunda 8 executable profile (codes C…). `folder` holds `.form` and `.dmn` files.
#[allow(clippy::too_many_lines)]
pub fn validate_camunda_model(md: &Model, folder: &Path) -> Vec<Issue> {
    let mut o = Out(Vec::new());
    let d = &md.doc;
    let r = d.root;
    if md.attr(r, &qn(MODELER, "executionPlatform")) != Some("Camunda Cloud") {
        o.add(
            Level::Warning,
            "C001",
            md.id_of(r),
            "Brak atrybutu modeler:executionPlatform=\"Camunda Cloud\" w <bpmn:definitions>. Camunda Modeler \
             otworzy plik jako model Camunda 7."
                .to_string(),
        );
    }
    let executable: Vec<Id> = md
        .processes
        .iter()
        .copied()
        .filter(|p| md.attr(*p, "isExecutable") == Some("true"))
        .collect();
    if executable.is_empty() {
        o.add(
            Level::Error,
            "C002",
            None,
            "Żaden proces nie ma isExecutable=\"true\". Silnik nie wdroży modelu.".to_string(),
        );
    }
    let (forms, decisions) = resource_ids(folder);
    for proc in executable {
        for e in d.iter(proc) {
            let t = md.tag(e);
            let lab = md.label(Some(e));
            let eid = md.id_of(e);
            if t == "userTask" {
                if zeebe(md, e, "userTask").is_none() {
                    o.add(
                        Level::Warning,
                        "C010",
                        eid,
                        format!(
                            "Zadanie użytkownika {lab} nie ma <zeebe:userTask/>. Bez niego nie trafi do Tasklist jako \
                             zadanie Camunda."
                        ),
                    );
                }
                match zeebe(md, e, "formDefinition") {
                    None => o.add(
                        Level::Note,
                        "C011",
                        eid,
                        format!(
                            "Zadanie użytkownika {lab} nie ma formularza (zeebe:formDefinition). W Tasklist można je \
                             tylko zakończyć bez danych."
                        ),
                    ),
                    Some(fd) => {
                        if let Some(fid) = md.attr(fd, "formId").filter(|s| !s.is_empty())
                            && !forms.contains(fid) {
                                o.add(
                                    Level::Warning,
                                    "C012",
                                    eid,
                                    format!(
                                        "Formularza „{fid}” (zadanie {lab}) nie ma w żadnym pliku .form w folderze \
                                         modelu."
                                    ),
                                );
                            }
                    }
                }
            } else if t == "startEvent" {
                if let Some(fd) = zeebe(md, e, "formDefinition")
                    && let Some(fid) = md.attr(fd, "formId").filter(|s| !s.is_empty())
                    && !forms.contains(fid)
                {
                    o.add(
                                Level::Warning,
                                "C012",
                                eid,
                                format!("Formularza startowego „{fid}” nie ma w żadnym pliku .form w folderze modelu."),
                            );
                }
            } else if t == "serviceTask" || t == "sendTask" {
                let td = zeebe(md, e, "taskDefinition");
                let typ = td
                    .and_then(|x| md.attr(x, "type"))
                    .filter(|s| !s.is_empty());
                match typ {
                    None => o.add(
                        Level::Error,
                        "C020",
                        eid,
                        format!(
                            "{} {} wymaga <zeebe:taskDefinition type=\"…\"/>. Bez niego wdrożenie się nie powiedzie.",
                            capitalize(&tag_pl_or(&t)),
                            lab
                        ),
                    ),
                    Some(ty) => o.add(
                        Level::Note,
                        "C021",
                        eid,
                        format!(
                            "{} {} czeka na program (worker) obsługujący typ „{}”. Bez niego instancja zatrzyma się \
                             w tym miejscu.",
                            capitalize(&tag_pl_or(&t)),
                            lab,
                            ty
                        ),
                    ),
                }
            } else if t == "businessRuleTask" {
                let cd = zeebe(md, e, "calledDecision");
                match cd {
                    None if zeebe(md, e, "taskDefinition").is_none() => o.add(
                        Level::Error,
                        "C030",
                        eid,
                        format!(
                            "Zadanie reguły biznesowej {lab} wymaga <zeebe:calledDecision decisionId=\"…\" \
                             resultVariable=\"…\"/>."
                        ),
                    ),
                    None => {}
                    Some(cd) => {
                        let did = md.attr(cd, "decisionId").filter(|s| !s.is_empty());
                        let rv = md.attr(cd, "resultVariable").filter(|s| !s.is_empty());
                        match (did, rv) {
                            (Some(did), Some(_)) => {
                                if !decisions.contains(did) {
                                    o.add(
                                        Level::Warning,
                                        "C032",
                                        eid,
                                        format!(
                                            "Decyzji „{did}” (zadanie {lab}) nie ma w żadnym pliku .dmn w folderze \
                                             modelu."
                                        ),
                                    );
                                }
                            }
                            _ => o.add(
                                Level::Error,
                                "C031",
                                eid,
                                format!("zeebe:calledDecision w {lab} wymaga atrybutów decisionId i resultVariable."),
                            ),
                        }
                    }
                }
            } else if t == "scriptTask" {
                match zeebe(md, e, "script") {
                    None if zeebe(md, e, "taskDefinition").is_none() => o.add(
                        Level::Error,
                        "C040",
                        eid,
                        format!(
                            "Zadanie skryptowe {lab} wymaga <zeebe:script expression=\"=…\" resultVariable=\"…\"/>."
                        ),
                    ),
                    None => {}
                    Some(sc) => {
                        let expr_ok = md.attr(sc, "expression").unwrap_or("").trim().starts_with('=');
                        let rv_ok = md.attr(sc, "resultVariable").is_some_and(|s| !s.is_empty());
                        if !expr_ok || !rv_ok {
                            o.add(
                                Level::Error,
                                "C041",
                                eid,
                                format!(
                                    "zeebe:script w {lab}: wyrażenie FEEL musi zaczynać się od „=”, a resultVariable \
                                     jest wymagany."
                                ),
                            );
                        }
                    }
                }
            } else if t == "receiveTask"
                || ((t == "intermediateCatchEvent" || t == "boundaryEvent")
                    && md
                        .eventdefs(e)
                        .iter()
                        .any(|x| x == "messageEventDefinition"))
            {
                match message_of(md, e) {
                    None => o.add(
                        Level::Error,
                        "C050",
                        eid,
                        format!("{lab} nie wskazuje wiadomości (messageRef)."),
                    ),
                    Some(msg) => {
                        let sub = zeebe(md, msg, "subscription");
                        if md.attr(msg, "name").is_none_or(str::is_empty) {
                            o.add(
                                Level::Error,
                                "C051",
                                md.id_of(msg),
                                format!(
                                    "Wiadomość {} musi mieć nazwę (name): po niej silnik dopasowuje wiadomość.",
                                    md.label(Some(msg))
                                ),
                            );
                        }
                        let key_ok = sub
                            .and_then(|s| md.attr(s, "correlationKey"))
                            .is_some_and(|k| k.trim().starts_with('='));
                        if !key_ok {
                            o.add(
                                Level::Error,
                                "C052",
                                md.id_of(msg),
                                format!(
                                    "Wiadomość {}, na którą czeka {}, wymaga <zeebe:subscription correlationKey=\"=…\"/>.",
                                    md.label(Some(msg)),
                                    lab
                                ),
                            );
                        }
                    }
                }
            } else if t == "complexGateway" || t == "transaction" {
                o.add(
                    Level::Warning,
                    "C060",
                    eid,
                    format!(
                        "{} {} nie jest wykonywany przez Camunda 8.",
                        capitalize(&tag_pl_or(&t)),
                        lab
                    ),
                );
            }

            if t == "exclusiveGateway" || t == "inclusiveGateway" {
                let outs = md.out_of(eid);
                if outs.len() > 1 {
                    for &f in outs {
                        let fid = md.id_of(f);
                        if fid == md.attr(e, "default") {
                            continue;
                        }
                        let body = d
                            .find(f, &m("conditionExpression"))
                            .map(|ce| text_of(md, ce).trim().to_string())
                            .unwrap_or_default();
                        if body.is_empty() {
                            o.add(
                                Level::Error,
                                "C070",
                                fid,
                                format!(
                                    "Przepływ {} z bramki {} nie ma warunku. Silnik wymaga warunku na każdym wyjściu \
                                     oprócz domyślnego.",
                                    md.label(Some(f)),
                                    lab
                                ),
                            );
                        } else if !body.starts_with('=') {
                            o.add(
                                Level::Error,
                                "C071",
                                fid,
                                format!(
                                    "Warunek przepływu {} musi być wyrażeniem FEEL zaczynającym się od „=”, np. \
                                     =zgoda = true.",
                                    md.label(Some(f))
                                ),
                            );
                        }
                    }
                }
            }
            if t == "timerEventDefinition" && !any_child_text(md, e) {
                let ev = md.parent.get(&e).copied();
                o.add(
                    Level::Error,
                    "C080",
                    ev.and_then(|x| md.id_of(x)),
                    format!(
                        "Zdarzenie czasowe {} nie ma wartości (timeDuration, timeDate albo timeCycle).",
                        md.label(ev)
                    ),
                );
            }
        }
    }
    o.0
}

/// `summarize(m)`.
#[must_use]
pub fn summarize_model(md: &Model) -> String {
    let d = &md.doc;
    let all = d.iter(d.root);
    let n_nodes = all
        .iter()
        .filter(|e| is_flownode(&local(d.tag(**e))))
        .count();
    let n_lanes = d.iter_tag(d.root, &m("lane")).len();
    format!(
        "węzły: {}, przepływy sekwencji: {}, przepływy komunikatów: {}, baseny: {}, linie: {}",
        n_nodes,
        md.seq.len(),
        md.msg.len(),
        md.participants.len(),
        n_lanes
    )
}
