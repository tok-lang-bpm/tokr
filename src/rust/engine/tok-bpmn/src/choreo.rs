//! Diagrams of choreographies (BPMN 2.0.2 §11, §12.2 participant bands): every
//! choreography gets its own `BPMNDiagram`. Its flow nodes are laid out as the nodes of a
//! process (choreography tasks as tasks, sub-choreographies as sub-processes), below the
//! collaboration diagram, and each choreography activity gets one participant band per
//! participant: the initiating participant on top, the others at the bottom.

use crate::layout;
use crate::model::{Model, m};
use crate::pyfmt::round_str;
use crate::xml::{BPMN_NS, BPMNDI_NS, DC_NS, DI_NS, Doc, Id, qn};

/// A shape of the laid-out choreography: element, bounds, label bounds, `isExpanded`.
type ShapeDi = (String, [f64; 4], Option<[f64; 4]>, Option<String>);
/// An edge: element, waypoints, label bounds.
type EdgeDi = (String, Vec<(f64, f64)>, Option<[f64; 4]>);
/// A participant band: activity, participant, activity shape, bounds, kind, message visible.
type BandDi = (String, String, String, [f64; 4], &'static str, bool);

/// Vertical space between the diagram above and a choreography.
const GAP: f64 = 80.0;
/// Height of a participant band.
pub const BAND_H: f64 = 20.0;

/// Elements of a choreography that are not drawn as flow nodes.
const SKIP: &[&str] = &[
    "participant",
    "messageFlow",
    "participantRef",
    "messageFlowRef",
    "participantAssociation",
    "correlationKey",
    "documentation",
    "extensionElements",
    "textAnnotation",
    "association",
    "group",
];

fn copy_into(md: &Model, src: Id, dst: &mut Doc, parent: Id) {
    for &c in md.doc.children(src) {
        let t = md.tag(c);
        if SKIP.contains(&t.as_str()) || md.doc.tag(c).starts_with('{') && !is_bpmn(&md.doc, c) {
            continue;
        }
        let tag = match t.as_str() {
            "choreographyTask" => "task",
            "subChoreography" => "subProcess",
            "callChoreography" => "callActivity",
            other => other,
        };
        let attrs: Vec<(String, String)> = md.doc.elems[c]
            .attrs
            .iter()
            .filter(|(k, _)| {
                !matches!(
                    k.as_str(),
                    "initiatingParticipantRef" | "loopType" | "calledChoreographyRef"
                )
            })
            .cloned()
            .collect();
        let refs: Vec<(&str, &str)> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let e = dst.sub_element(parent, &qn(BPMN_NS, tag), &refs);
        dst.elems[e].text = md.doc.elems[c].text.clone();
        copy_into(md, c, dst, e);
    }
}

fn is_bpmn(d: &Doc, e: Id) -> bool {
    d.tag(e).starts_with(&format!("{{{BPMN_NS}}}"))
}

fn num(v: Option<&str>) -> f64 {
    v.and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0)
}

fn bounds(d: &Doc, e: Id) -> Option<[f64; 4]> {
    let b = d.find(e, &qn(DC_NS, "Bounds"))?;
    Some([
        num(d.get(b, "x")),
        num(d.get(b, "y")),
        num(d.get(b, "width")),
        num(d.get(b, "height")),
    ])
}

fn label(d: &Doc, e: Id) -> Option<[f64; 4]> {
    let l = d.find(e, &qn(BPMNDI_NS, "BPMNLabel"))?;
    bounds(d, l)
}

fn write_bounds(d: &mut Doc, parent: Id, b: [f64; 4]) {
    let (x, y, w, h) = (
        round_str(b[0]),
        round_str(b[1]),
        round_str(b[2]),
        round_str(b[3]),
    );
    d.sub_element(
        parent,
        &qn(DC_NS, "Bounds"),
        &[("x", &x), ("y", &y), ("width", &w), ("height", &h)],
    );
}

fn write_label(d: &mut Doc, parent: Id, b: Option<[f64; 4]>) {
    if let Some(b) = b {
        let l = d.sub_element(parent, &qn(BPMNDI_NS, "BPMNLabel"), &[]);
        write_bounds(d, l, b);
    }
}

/// Lowest point of the diagrams already in the document.
fn bottom(d: &Doc) -> Option<f64> {
    let mut y: Option<f64> = None;
    let mut see = |v: f64| y = Some(y.map_or(v, |c: f64| c.max(v)));
    for b in d.iter_tag(d.root, &qn(DC_NS, "Bounds")) {
        see(num(d.get(b, "y")) + num(d.get(b, "height")));
    }
    for p in d.iter_tag(d.root, &qn(DI_NS, "waypoint")) {
        see(num(d.get(p, "y")));
    }
    y
}

/// Participants of a choreography activity: the initiating one first.
pub fn band_order(md: &Model, act: Id) -> Vec<String> {
    let mut parts: Vec<String> = md
        .doc
        .findall(act, &m("participantRef"))
        .iter()
        .map(|r| md.doc.text(*r).unwrap_or("").trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if let Some(init) = md.attr(act, "initiatingParticipantRef")
        && let Some(k) = parts.iter().position(|p| p == init)
    {
        let first = parts.remove(k);
        parts.insert(0, first);
    }
    parts
}

/// Whether the participant sends a message of the choreography task (its band shows the
/// envelope).
fn sends(md: &Model, act: Id, participant: &str) -> bool {
    md.doc.findall(act, &m("messageFlowRef")).iter().any(|r| {
        let fid = md.doc.text(*r).unwrap_or("").trim().to_string();
        md.get(Some(&fid))
            .is_some_and(|f| md.attr(f, "sourceRef") == Some(participant))
    })
}

/// Bounds and kinds of the bands of an activity drawn at `b`.
pub fn bands(b: [f64; 4], n: usize) -> Vec<([f64; 4], &'static str)> {
    let [x, y, w, h] = b;
    if n == 0 {
        return Vec::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let bh = BAND_H.min((h - 20.0).max(10.0) / n as f64);
    let mut out = vec![([x, y, w, bh], "top_initiating")];
    let rest = n - 1;
    for k in 0..rest {
        #[allow(clippy::cast_precision_loss)]
        let from_bottom = (rest - k) as f64;
        let kind = if k + 1 == rest {
            "bottom_non_initiating"
        } else {
            "middle_non_initiating"
        };
        out.push(([x, y + h - bh * from_bottom, w, bh], kind));
    }
    out
}

/// Sizes of the choreography activities (room for their bands) and padding of the
/// expanded sub-choreographies; rows keep room for the message envelopes.
fn hints(md: &Model, chor: Id) -> crate::model::Hints {
    let mut h = crate::model::Hints {
        row_gap: Some(90),
        ..Default::default()
    };
    for e in md.doc.iter(chor) {
        if !crate::model::is_choreo_act(&md.tag(e)) {
            continue;
        }
        let Some(id) = md.id_of(e).map(str::to_string) else {
            continue;
        };
        h.boundary_left.insert(id.clone());
        let n = i64::try_from(band_order(md, e).len().max(2)).unwrap_or(2);
        if md.tag(e) == "subChoreography" && md.has_flow_children(e) {
            h.pad.insert(id, (90, 20 * (n - 1) + 45));
        } else {
            h.size.insert(id, (110, 60 + 20 * n));
        }
    }
    h
}

/// Lay out every choreography of the document in a diagram of its own.
pub fn layout_choreographies(md: &mut Model) {
    let root = md.doc.root;
    let chors: Vec<Id> = md.doc.findall(root, &m("choreography"));
    if chors.is_empty() {
        return;
    }
    let mut base = bottom(&md.doc).map_or(0.0, |b| b + GAP);
    for c in chors {
        let Some(cid) = md.id_of(c).map(str::to_string) else {
            continue;
        };
        let head = format!(
            "<bpmn:definitions xmlns:bpmn=\"{BPMN_NS}\" id=\"choreography-layout\">\
             <bpmn:process id=\"{}\" isExecutable=\"false\"/></bpmn:definitions>",
            crate::svg::esc(&cid)
        );
        let Ok(mut tmp) = Doc::parse(&head) else {
            continue;
        };
        let Some(proc) = tmp.find(tmp.root, &m("process")) else {
            continue;
        };
        copy_into(md, c, &mut tmp, proc);
        let mut tm = Model::new(tmp);
        tm.hints = hints(md, c);
        layout::layout_model(&mut tm);
        let td = &tm.doc;
        let shapes: Vec<ShapeDi> = td
            .iter_tag(td.root, &qn(BPMNDI_NS, "BPMNShape"))
            .into_iter()
            .filter_map(|s| {
                let el = td.get(s, "bpmnElement")?.to_string();
                let b = bounds(td, s)?;
                Some((
                    el,
                    b,
                    label(td, s),
                    td.get(s, "isExpanded").map(str::to_string),
                ))
            })
            .collect();
        let edges: Vec<EdgeDi> = td
            .iter_tag(td.root, &qn(BPMNDI_NS, "BPMNEdge"))
            .into_iter()
            .filter_map(|e| {
                let el = td.get(e, "bpmnElement")?.to_string();
                let pts = td
                    .findall(e, &qn(DI_NS, "waypoint"))
                    .iter()
                    .map(|p| (num(td.get(*p, "x")), num(td.get(*p, "y"))))
                    .collect();
                Some((el, pts, label(td, e)))
            })
            .collect();
        let top = shapes
            .iter()
            .map(|s| s.1[1])
            .chain(edges.iter().flat_map(|e| e.1.iter().map(|p| p.1)))
            .fold(f64::INFINITY, f64::min);
        if !top.is_finite() {
            continue;
        }
        // room for the envelopes above the first row
        let dy = base - top + 30.0;
        let sh = |b: [f64; 4]| [b[0], b[1] + dy, b[2], b[3]];
        let d = &mut md.doc;
        let diag = d.sub_element(
            root,
            &qn(BPMNDI_NS, "BPMNDiagram"),
            &[("id", &format!("BPMNDiagram_{cid}"))],
        );
        let plane = d.sub_element(
            diag,
            &qn(BPMNDI_NS, "BPMNPlane"),
            &[("id", &format!("BPMNPlane_{cid}")), ("bpmnElement", &cid)],
        );
        let mut low = base;
        let mut acts: Vec<(String, [f64; 4])> = Vec::new();
        for (el, b, lb, expanded) in &shapes {
            let b = sh(*b);
            low = low.max(b[1] + b[3]);
            let di = format!("{el}_di");
            let mut attrs: Vec<(&str, &str)> = vec![("id", &di), ("bpmnElement", el)];
            if let Some(x) = expanded {
                attrs.push(("isExpanded", x));
            }
            let s = d.sub_element(plane, &qn(BPMNDI_NS, "BPMNShape"), &attrs);
            write_bounds(d, s, b);
            write_label(d, s, lb.map(sh));
            if let Some(l) = lb {
                low = low.max(l[1] + l[3] + dy);
            }
            acts.push((el.clone(), b));
        }
        for (el, pts, lb) in &edges {
            let di = format!("{el}_di");
            let e = d.sub_element(
                plane,
                &qn(BPMNDI_NS, "BPMNEdge"),
                &[("id", &di), ("bpmnElement", el)],
            );
            for (x, y) in pts {
                let (xs, ys) = (round_str(*x), round_str(*y + dy));
                d.sub_element(e, &qn(DI_NS, "waypoint"), &[("x", &xs), ("y", &ys)]);
                low = low.max(*y + dy);
            }
            write_label(d, e, lb.map(sh));
        }
        md.reindex();
        // participant bands of the choreography activities
        let mut band_shapes: Vec<BandDi> = Vec::new();
        for (el, b) in &acts {
            let Some(a) = md.get(Some(el)) else {
                continue;
            };
            if !crate::model::is_choreo_act(&md.tag(a)) {
                continue;
            }
            let parts = band_order(md, a);
            for (p, (bb, kind)) in parts.iter().zip(bands(*b, parts.len())) {
                let visible = md.tag(a) == "choreographyTask" && sends(md, a, p);
                band_shapes.push((el.clone(), p.clone(), format!("{el}_di"), bb, kind, visible));
            }
        }
        let d = &mut md.doc;
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (el, p, act_di, bb, kind, visible) in band_shapes {
            let mut di = format!("{el}-{p}_di");
            let mut k = 2;
            while !used.insert(di.clone()) {
                di = format!("{el}-{p}-{k}_di");
                k += 1;
            }
            let s = d.sub_element(
                plane,
                &qn(BPMNDI_NS, "BPMNShape"),
                &[
                    ("id", &di),
                    ("bpmnElement", &p),
                    ("isMessageVisible", if visible { "true" } else { "false" }),
                    ("participantBandKind", kind),
                    ("choreographyActivityShape", &act_di),
                ],
            );
            write_bounds(d, s, bb);
        }
        md.reindex();
        base = low + 30.0 + GAP;
    }
}
