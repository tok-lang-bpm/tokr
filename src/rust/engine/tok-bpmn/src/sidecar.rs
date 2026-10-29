//! Layout sidecar (`FILE.tok.layout`): the diagram of a BPMN file kept next to the tok
//! document it was imported into, so that drawing the document again keeps the geometry the
//! modeller gave it.
//!
//! ```text
//! tok-layout 1
//! ID x y w h              a shape
//! ID x1 y1 x2 y2 …        an edge (its waypoints)
//! ID/label x y w h        the label of a shape or an edge
//! ```
//!
//! Numbers are whole units of the BPMN file. A line with four numbers is a shape or an edge
//! with two waypoints, as the element it names is a node or a flow.
//!
//! Drawing with a sidecar ([`layout_with`]): the automatic layout runs first; then every
//! shape whose identifier the sidecar names takes its geometry from it, new shapes are
//! placed next to a neighbour that kept its geometry (as far from it as in the automatic
//! layout, moved aside from other shapes), containers grow around new content, and edges
//! keep their waypoints when both ends kept their geometry and the edge still touches them;
//! the other edges are routed again.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::choreo;
use crate::layout::{self, Pt};
use crate::model::{Model, is_choreo_act, is_subp, m};
use crate::pyfmt::round_str;
use crate::xml::{BPMNDI_NS, DC_NS, DI_NS, Doc, Id, qn};

type R = [f64; 4];

/// First line of a sidecar file.
pub const HEADER: &str = "tok-layout 1";

/// The geometry of a diagram by tok identifier, in file order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sidecar {
    /// Shapes (four numbers) and edges (waypoints, two numbers each).
    pub entries: Vec<(String, Vec<i64>)>,
    /// Label bounds of shapes and edges.
    pub labels: Vec<(String, [i64; 4])>,
}

fn whole(v: f64) -> i64 {
    // coordinates of diagrams are far inside the range of i64
    #[allow(clippy::cast_possible_truncation)]
    let r = v.round() as i64;
    r
}

#[allow(clippy::cast_precision_loss)]
fn f(v: i64) -> f64 {
    v as f64
}

impl Sidecar {
    /// Read a sidecar file.
    ///
    /// # Errors
    /// [`Error::Sidecar`] when the header is missing or a line is malformed.
    pub fn parse(text: &str) -> Result<Sidecar, Error> {
        let mut lines = text.lines().map(str::trim).enumerate();
        match lines.find(|(_, l)| !l.is_empty()) {
            Some((_, h)) if h == HEADER => {}
            _ => return Err(Error::Sidecar(format!("the first line must be '{HEADER}'"))),
        }
        let mut sc = Sidecar::default();
        for (k, line) in lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(id) = parts.next() else {
                continue;
            };
            let nums: Result<Vec<i64>, _> = parts.map(str::parse::<i64>).collect();
            let bad = || Error::Sidecar(format!("line {}: malformed '{line}'", k + 1));
            let nums = nums.map_err(|_| bad())?;
            if let Some(owner) = id.strip_suffix("/label") {
                let [x, y, w, h] = nums[..] else {
                    return Err(bad());
                };
                sc.labels.push((owner.to_string(), [x, y, w, h]));
            } else {
                if nums.len() < 4 || nums.len() % 2 != 0 {
                    return Err(bad());
                }
                sc.entries.push((id.to_string(), nums));
            }
        }
        Ok(sc)
    }

    /// The text of the sidecar file.
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER}\n");
        let labels: HashMap<&str, &[i64; 4]> =
            self.labels.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let mut written: HashSet<&str> = HashSet::new();
        for (id, nums) in &self.entries {
            let n: Vec<String> = nums.iter().map(i64::to_string).collect();
            out.push_str(&format!("{id} {}\n", n.join(" ")));
            if let Some(l) = labels.get(id.as_str()) {
                out.push_str(&format!("{id}/label {} {} {} {}\n", l[0], l[1], l[2], l[3]));
                written.insert(id);
            }
        }
        for (id, l) in &self.labels {
            if !written.contains(id.as_str()) {
                out.push_str(&format!("{id}/label {} {} {} {}\n", l[0], l[1], l[2], l[3]));
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn num(d: &Doc, e: Id, k: &str) -> f64 {
    d.get(e, k)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn bounds_of(d: &Doc, e: Id) -> Option<R> {
    let b = d.find(e, &qn(DC_NS, "Bounds"))?;
    Some([
        num(d, b, "x"),
        num(d, b, "y"),
        num(d, b, "width"),
        num(d, b, "height"),
    ])
}

fn label_of(d: &Doc, e: Id) -> Option<R> {
    let l = d.find(e, &qn(BPMNDI_NS, "BPMNLabel"))?;
    bounds_of(d, l)
}

fn points_of(d: &Doc, e: Id) -> Vec<Pt> {
    d.findall(e, &qn(DI_NS, "waypoint"))
        .iter()
        .map(|p| (num(d, *p, "x"), num(d, *p, "y")))
        .collect()
}

fn text_ref(md: &Model, e: Id, name: &str) -> Option<String> {
    md.doc
        .find(e, &m(name))
        .and_then(|r| md.doc.text(r))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// What an edge connects, as a key that survives a change of identifiers: its kind and its
/// two ends (data associations: data element and activity).
fn ends(md: &Model, e: Id) -> Option<(String, String, String)> {
    let t = md.tag(e);
    match t.as_str() {
        "sequenceFlow" | "messageFlow" | "association" | "conversationLink" => Some((
            t,
            md.attr(e, "sourceRef")?.to_string(),
            md.attr(e, "targetRef")?.to_string(),
        )),
        "dataInputAssociation" => {
            let owner = md.id_of(*md.parent.get(&e)?)?.to_string();
            Some((t, text_ref(md, e, "sourceRef")?, owner))
        }
        "dataOutputAssociation" => {
            let owner = md.id_of(*md.parent.get(&e)?)?.to_string();
            Some((t, owner, text_ref(md, e, "targetRef")?))
        }
        _ => None,
    }
}

/// A boundary event as its host and the kinds of its event definitions.
fn boundary_key(md: &Model, e: Id) -> Option<(String, Vec<String>)> {
    if md.tag(e) != "boundaryEvent" {
        return None;
    }
    Some((md.attr(e, "attachedToRef")?.to_string(), md.eventdefs(e)))
}

/// The geometry of the diagram of `original` (a BPMN file), by the identifiers of
/// `exported` (the BPMN export of the tok document imported from it). `rename` maps
/// identifiers of the file to tok identifiers; elements that tok identifies otherwise
/// (data associations, associations of notes, flows with derived identifiers, generated
/// compensation boundary events) are matched by what they connect. Bands of choreography
/// activities are not stored: they follow from the bounds of their activity.
///
/// # Errors
/// [`Error::Xml`] when either text is not well-formed XML.
pub fn extract(
    original: &str,
    exported: &str,
    rename: &HashMap<String, String>,
) -> Result<Sidecar, Error> {
    let om = Model::new(Doc::parse(original)?);
    let em = Model::new(Doc::parse(exported)?);
    let tok = |x: &str| -> String { rename.get(x).cloned().unwrap_or_else(|| x.to_string()) };
    // exported edges and boundary events by what they connect
    let mut by_ends: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    let mut by_host: HashMap<(String, Vec<String>), Vec<String>> = HashMap::new();
    for e in em.doc.iter(em.doc.root) {
        let Some(id) = em.id_of(e).map(str::to_string) else {
            continue;
        };
        if let Some(k) = ends(&em, e) {
            by_ends.entry(k).or_default().push(id.clone());
        }
        if let Some(k) = boundary_key(&em, e) {
            by_host.entry(k).or_default().push(id);
        }
    }
    for v in by_ends.values_mut().chain(by_host.values_mut()) {
        v.reverse();
    }
    let mut sc = Sidecar::default();
    let mut seen: HashSet<String> = HashSet::new();
    let d = &om.doc;
    let offsets = plane_offsets(d);
    for s in d.iter(d.root) {
        let t = d.tag(s);
        let shape = t == qn(BPMNDI_NS, "BPMNShape");
        if (!shape && t != qn(BPMNDI_NS, "BPMNEdge"))
            || d.get(s, "choreographyActivityShape").is_some()
        {
            continue;
        }
        let Some(el) = d.get(s, "bpmnElement") else {
            continue;
        };
        let direct = tok(el);
        let id = if em.ids.contains_key(&direct) {
            Some(direct)
        } else {
            om.get(Some(el)).and_then(|oe| {
                if let Some((k, a, b)) = ends(&om, oe) {
                    by_ends.get_mut(&(k, tok(&a), tok(&b)))?.pop()
                } else {
                    let (h, defs) = boundary_key(&om, oe)?;
                    by_host.get_mut(&(tok(&h), defs))?.pop()
                }
            })
        };
        let Some(id) = id.filter(|i| !seen.contains(i)) else {
            continue;
        };
        seen.insert(id.clone());
        let (dx, dy) = offsets
            .get(&plane_of(&om, s))
            .copied()
            .unwrap_or((0.0, 0.0));
        if shape {
            let Some(b) = bounds_of(d, s) else {
                continue;
            };
            sc.entries.push((
                id.clone(),
                vec![whole(b[0] + dx), whole(b[1] + dy), whole(b[2]), whole(b[3])],
            ));
        } else {
            let pts = points_of(d, s);
            if pts.len() < 2 {
                continue;
            }
            sc.entries.push((
                id.clone(),
                pts.iter()
                    .flat_map(|(x, y)| [whole(x + dx), whole(y + dy)])
                    .collect(),
            ));
        }
        if let Some(l) = label_of(d, s) {
            sc.labels.push((
                id,
                [whole(l[0] + dx), whole(l[1] + dy), whole(l[2]), whole(l[3])],
            ));
        }
    }
    Ok(sc)
}

/// The plane (`BPMNPlane`) that holds a shape or an edge.
fn plane_of(md: &Model, e: Id) -> Option<Id> {
    let mut p = md.parent.get(&e).copied();
    while let Some(x) = p {
        if md.doc.tag(x) == qn(BPMNDI_NS, "BPMNPlane") {
            return Some(x);
        }
        p = md.parent.get(&x).copied();
    }
    None
}

/// tok draws one diagram: the first plane of the file keeps its coordinates and every
/// further plane (another collaboration or process, the content of a collapsed
/// sub-process, a choreography) moves below the one before it, aligned on the left.
fn plane_offsets(d: &Doc) -> HashMap<Option<Id>, (f64, f64)> {
    let mut out = HashMap::new();
    let mut first_left: Option<f64> = None;
    let mut bottom: Option<f64> = None;
    for plane in d.iter_tag(d.root, &qn(BPMNDI_NS, "BPMNPlane")) {
        let mut xs: Vec<f64> = Vec::new();
        let mut ys: Vec<f64> = Vec::new();
        for e in d.iter(plane) {
            if d.tag(e) == qn(DC_NS, "Bounds") {
                xs.push(num(d, e, "x"));
                ys.push(num(d, e, "y"));
                ys.push(num(d, e, "y") + num(d, e, "height"));
            } else if d.tag(e) == qn(DI_NS, "waypoint") {
                xs.push(num(d, e, "x"));
                ys.push(num(d, e, "y"));
            }
        }
        if xs.is_empty() {
            continue;
        }
        let left = xs.iter().copied().fold(f64::INFINITY, f64::min);
        let top = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let low = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let (dx, dy) = match (first_left, bottom) {
            // whole units, so that the coordinates of the plane stay whole after the move
            (Some(l), Some(b)) => ((l - left).round(), (b + 80.0 - top).round()),
            _ => {
                first_left = Some(left);
                (0.0, 0.0)
            }
        };
        bottom = Some(low + dy);
        out.insert(Some(plane), (dx, dy));
    }
    out
}

// ------------------------------------------------------------------ drawing

/// Lay the model out and keep the geometry of the sidecar (see the module documentation).
///
/// # Errors
/// [`Error::Xml`] when the input is not well-formed XML.
pub fn layout_with(xml: &str, sc: &Sidecar) -> Result<String, Error> {
    let mut md = Model::new(Doc::parse(xml)?);
    layout::layout_model(&mut md);
    apply(&mut md, sc);
    Ok(md.doc.save())
}

/// [`layout_with`], rendered as SVG.
///
/// # Errors
/// [`Error::Xml`] for malformed XML, [`Error::NoDiagram`] when nothing can be drawn.
pub fn render_svg_with(xml: &str, title: Option<&str>, sc: &Sidecar) -> Result<String, Error> {
    let mut md = Model::new(Doc::parse(xml)?);
    layout::layout_model(&mut md);
    apply(&mut md, sc);
    crate::svg::render_model(&md, title)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Kept,
    Placed,
}

struct Shape {
    di: Id,
    auto: R,
    label: Option<R>,
}

struct Placer<'m> {
    md: &'m Model,
    shapes: HashMap<String, Shape>,
    order: Vec<String>,
    fin: HashMap<String, (R, State)>,
    /// flow node -> its leaf lane
    lane_of: HashMap<String, String>,
    /// element -> elements connected with it by an edge
    adj: HashMap<String, Vec<String>>,
}

fn overlaps(a: R, b: R) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn grow(a: R, d: f64) -> R {
    [a[0] - d, a[1] - d, a[2] + 2.0 * d, a[3] + 2.0 * d]
}

fn inside(p: Pt, r: R) -> bool {
    p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3]
}

fn union(a: R, b: R) -> R {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// The point where the line from the centre of `b` towards (px, py) leaves `b`.
fn clip(b: R, px: f64, py: f64) -> Pt {
    let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    let (dx, dy) = (px - cx, py - cy);
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return (cx, cy);
    }
    let sx = if dx.abs() > 1e-9 {
        (b[2] / 2.0) / dx.abs()
    } else {
        f64::INFINITY
    };
    let sy = if dy.abs() > 1e-9 {
        (b[3] / 2.0) / dy.abs()
    } else {
        f64::INFINITY
    };
    let s = sx.min(sy);
    (cx + dx * s, cy + dy * s)
}

impl Placer<'_> {
    fn tag(&self, id: &str) -> String {
        self.md
            .get(Some(id))
            .map(|e| self.md.tag(e))
            .unwrap_or_default()
    }

    fn is_container(&self, id: &str) -> bool {
        let t = self.tag(id);
        t == "participant"
            || t == "lane"
            || ((is_subp(&t) || t == "subChoreography")
                && self
                    .md
                    .get(Some(id))
                    .is_some_and(|e| self.md.has_flow_children(e)))
    }

    /// The drawn container of an element: its sub-process, else its leaf lane, else its
    /// pool; the parent lane or pool of a lane.
    fn container(&self, id: &str) -> Option<String> {
        let md = self.md;
        let e = md.get(Some(id))?;
        let t = md.tag(e);
        if t == "participant" {
            return None;
        }
        if t == "lane" {
            let mut p = md.parent.get(&e).copied();
            while let Some(x) = p {
                let xt = md.tag(x);
                if xt == "lane" {
                    return md.id_of(x).map(str::to_string);
                }
                if xt == "process" {
                    return md
                        .participant_of(Some(x))
                        .and_then(|q| md.id_of(q))
                        .map(str::to_string);
                }
                p = md.parent.get(&x).copied();
            }
            return None;
        }
        if let Some(&p) = md.parent.get(&e) {
            let pt = md.tag(p);
            if is_subp(&pt) || pt == "subChoreography" {
                return md.id_of(p).map(str::to_string);
            }
        }
        if let Some(l) = self.lane_of.get(id) {
            return Some(l.clone());
        }
        md.participant_of(Some(e))
            .and_then(|q| md.id_of(q))
            .filter(|q| *q != id)
            .map(str::to_string)
    }

    /// Elements connected with `id` by a flow, an association or attachment: the host of a
    /// boundary event first, then the sources of incoming and the targets of outgoing
    /// sequence flows, then everything else.
    fn neighbours(&self, id: &str) -> Vec<String> {
        let md = self.md;
        let mut out = Vec::new();
        if let Some(e) = md.get(Some(id))
            && let Some(h) = md.attr(e, "attachedToRef")
        {
            out.push(h.to_string());
        }
        for f in md.inc_of(Some(id)) {
            out.extend(md.attr(*f, "sourceRef").map(str::to_string));
        }
        for f in md.out_of(Some(id)) {
            out.extend(md.attr(*f, "targetRef").map(str::to_string));
        }
        for &b in md.boundaries_of(Some(id)) {
            out.extend(md.id_of(b).map(str::to_string));
        }
        out.extend(self.adj.get(id).cloned().unwrap_or_default());
        out
    }

    fn obstacles(&self, except: &str) -> Vec<R> {
        self.fin
            .iter()
            .filter(|(k, _)| k.as_str() != except && !self.is_container(k))
            .map(|(_, (r, _))| *r)
            .collect()
    }

    /// Move `r` right until it overlaps no placed shape.
    fn free(&self, id: &str, mut r: R) -> R {
        let obs = self.obstacles(id);
        for _ in 0..200 {
            if !obs.iter().any(|o| overlaps(grow(r, 10.0), *o)) {
                break;
            }
            r[0] += 20.0;
        }
        r
    }

    fn place_leaf(&self, id: &str) -> Option<R> {
        let auto = self.shapes.get(id)?.auto;
        let md = self.md;
        let boundary = md
            .get(Some(id))
            .is_some_and(|e| md.tag(e) == "boundaryEvent");
        for n in self.neighbours(id) {
            let (Some((nf, _)), Some(ns)) = (self.fin.get(&n), self.shapes.get(&n)) else {
                continue;
            };
            let na = ns.auto;
            if boundary {
                // as far from the bottom-right corner of the host as in the automatic layout
                let x = nf[0] + nf[2] - (na[0] + na[2] - auto[0]);
                let y = nf[1] + nf[3] - (na[1] + na[3] - auto[1]);
                return Some([x, y, auto[2], auto[3]]);
            }
            let r = [
                nf[0] + auto[0] - na[0],
                nf[1] + auto[1] - na[1],
                auto[2],
                auto[3],
            ];
            return Some(self.free(id, r));
        }
        let c = self.container(id)?;
        let (cf, _) = self.fin.get(&c)?;
        let ca = self.shapes.get(&c)?.auto;
        let r = [
            cf[0] + auto[0] - ca[0],
            cf[1] + auto[1] - ca[1],
            auto[2],
            auto[3],
        ];
        Some(self.free(id, r))
    }

    fn children(&self, id: &str) -> Vec<String> {
        self.order
            .iter()
            .filter(|k| self.container(k).as_deref() == Some(id))
            .cloned()
            .collect()
    }

    fn place_container(&self, id: &str) -> Option<R> {
        let auto = self.shapes.get(id)?.auto;
        let kids: Vec<R> = self
            .children(id)
            .iter()
            .filter_map(|k| self.fin.get(k).map(|(r, _)| *r))
            .collect();
        if let Some(first) = kids.first() {
            let b = kids.iter().fold(*first, |a, r| union(a, *r));
            // the band with the name of a pool or a lane on the left
            let left = if matches!(self.tag(id).as_str(), "participant" | "lane") {
                50.0
            } else {
                20.0
            };
            return Some([b[0] - left, b[1] - 30.0, b[2] + left + 20.0, b[3] + 50.0]);
        }
        let c = self.container(id)?;
        let (cf, _) = self.fin.get(&c)?;
        let ca = self.shapes.get(&c)?.auto;
        Some([
            cf[0] + auto[0] - ca[0],
            cf[1] + auto[1] - ca[1],
            auto[2],
            auto[3],
        ])
    }

    fn run(&mut self, sc: &Sidecar) {
        for (id, nums) in &sc.entries {
            if nums.len() == 4 && self.shapes.contains_key(id) {
                let r = [f(nums[0]), f(nums[1]), f(nums[2]), f(nums[3])];
                self.fin.insert(id.clone(), (r, State::Kept));
            }
        }
        if self.fin.is_empty() {
            return;
        }
        let mut pending: Vec<String> = self
            .order
            .iter()
            .filter(|k| !self.fin.contains_key(*k))
            .cloned()
            .collect();
        loop {
            let mut progress = false;
            for id in pending.clone() {
                let r = if self.is_container(&id) {
                    self.place_container(&id)
                } else {
                    self.place_leaf(&id)
                };
                if let Some(r) = r {
                    self.fin.insert(id.clone(), (r, State::Placed));
                    pending.retain(|k| *k != id);
                    progress = true;
                }
            }
            if !progress || pending.is_empty() {
                break;
            }
        }
        if !pending.is_empty() {
            // what has no placed neighbour goes below the diagram, as laid out automatically
            let bottom = self
                .fin
                .values()
                .map(|(r, _)| r[1] + r[3])
                .fold(f64::NEG_INFINITY, f64::max);
            let left = self
                .fin
                .values()
                .map(|(r, _)| r[0])
                .fold(f64::INFINITY, f64::min);
            let autos: Vec<R> = pending.iter().map(|k| self.shapes[k].auto).collect();
            let ax = autos.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
            let ay = autos.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
            for (k, a) in pending.iter().zip(autos) {
                let r = [a[0] - ax + left, a[1] - ay + bottom + 60.0, a[2], a[3]];
                self.fin.insert(k.clone(), (r, State::Placed));
            }
        }
        self.grow_containers();
    }

    /// Kept containers grow to hold their new content: pools and lanes to the right,
    /// sub-processes in both directions.
    fn grow_containers(&mut self) {
        let ids: Vec<String> = self.order.clone();
        for id in ids.iter().rev() {
            if !self.is_container(id) {
                continue;
            }
            let Some((cr, _)) = self.fin.get(id).copied() else {
                continue;
            };
            let kids: Vec<R> = self
                .children(id)
                .iter()
                .filter(|k| self.fin.get(*k).is_some_and(|(_, s)| *s == State::Placed))
                .filter_map(|k| self.fin.get(k).map(|(r, _)| *r))
                .collect();
            let mut r = cr;
            for k in kids {
                let need = union(r, grow(k, 20.0));
                if matches!(self.tag(id).as_str(), "participant" | "lane") {
                    r[2] = r[2].max(need[0] + need[2] - r[0]);
                } else {
                    r = need;
                }
            }
            if r != cr {
                self.fin.insert(id.clone(), (r, State::Placed));
                if self.tag(id) == "participant" {
                    // the lanes of the pool end where the pool ends
                    let right = r[0] + r[2];
                    let lanes: Vec<(String, R)> = ids
                        .iter()
                        .filter(|l| {
                            self.tag(l) == "lane"
                                && self.pool_of_lane(l).as_deref() == Some(id.as_str())
                        })
                        .filter_map(|l| self.fin.get(l).map(|(lr, _)| (l.clone(), *lr)))
                        .collect();
                    for (l, lr) in lanes {
                        let lr = [lr[0], lr[1], right - lr[0], lr[3]];
                        self.fin.insert(l, (lr, State::Placed));
                    }
                }
            }
        }
    }

    fn pool_of_lane(&self, lane: &str) -> Option<String> {
        let mut c = self.container(lane);
        while let Some(x) = c {
            if self.tag(&x) == "participant" {
                return Some(x);
            }
            c = self.container(&x);
        }
        None
    }
}

/// An edge between `s` and `t` routed again: orthogonal for sequence flows, vertical for
/// message flows and conversation links, straight for associations.
fn route(kind: &str, s: R, t: R) -> Vec<Pt> {
    let (scx, scy) = (s[0] + s[2] / 2.0, s[1] + s[3] / 2.0);
    let (tcx, tcy) = (t[0] + t[2] / 2.0, t[1] + t[3] / 2.0);
    match kind {
        "sequenceFlow" => {
            if t[0] >= s[0] + s[2] {
                let (x1, x2) = (s[0] + s[2], t[0]);
                if (scy - tcy).abs() < 1.0 {
                    vec![(x1, scy), (x2, tcy)]
                } else {
                    let mx = ((x1 + x2) / 2.0).round();
                    vec![(x1, scy), (mx, scy), (mx, tcy), (x2, tcy)]
                }
            } else if t[1] >= s[1] + s[3] || s[1] >= t[1] + t[3] {
                let down = t[1] >= s[1] + s[3];
                let (y1, y2) = if down {
                    (s[1] + s[3], t[1])
                } else {
                    (s[1], t[1] + t[3])
                };
                if (scx - tcx).abs() < 1.0 {
                    vec![(scx, y1), (tcx, y2)]
                } else {
                    let my = ((y1 + y2) / 2.0).round();
                    vec![(scx, y1), (scx, my), (tcx, my), (tcx, y2)]
                }
            } else {
                // back to the left: around below both shapes
                let low = (s[1] + s[3]).max(t[1] + t[3]) + 30.0;
                vec![
                    (scx, s[1] + s[3]),
                    (scx, low),
                    (tcx, low),
                    (tcx, t[1] + t[3]),
                ]
            }
        }
        "messageFlow" | "conversationLink" => {
            let x = scx.clamp(t[0] + 10.0, (t[0] + t[2] - 10.0).max(t[0] + 10.0));
            let x = if (x - scx).abs() < 1.0 { scx } else { x };
            let (y1, y2) = if tcy >= scy {
                (s[1] + s[3], t[1])
            } else {
                (s[1], t[1] + t[3])
            };
            if (x - scx).abs() < 1.0 {
                vec![(scx, y1), (scx, y2)]
            } else {
                let my = ((y1 + y2) / 2.0).round();
                vec![(scx, y1), (scx, my), (x, my), (x, y2)]
            }
        }
        _ => vec![clip(s, tcx, tcy), clip(t, scx, scy)],
    }
}

/// The label of a routed edge: next to the middle of its longest segment.
fn edge_label(pts: &[Pt], size: (f64, f64)) -> R {
    let mut best = (0.0, (0.0, 0.0), true);
    for w in pts.windows(2) {
        let ((x1, y1), (x2, y2)) = (w[0], w[1]);
        let len = (x2 - x1).abs() + (y2 - y1).abs();
        if len > best.0 {
            best = (
                len,
                ((x1 + x2) / 2.0, (y1 + y2) / 2.0),
                (y2 - y1).abs() < 1.0,
            );
        }
    }
    let (_, (mx, my), horizontal) = best;
    if horizontal {
        [mx - size.0 / 2.0, my - size.1 - 4.0, size.0, size.1]
    } else {
        [mx + 6.0, my - size.1 / 2.0, size.0, size.1]
    }
}

fn set_bounds(d: &mut Doc, owner: Id, r: R) {
    let b = match d.find(owner, &qn(DC_NS, "Bounds")) {
        Some(b) => b,
        None => d.sub_element(owner, &qn(DC_NS, "Bounds"), &[]),
    };
    d.set(b, "x", &round_str(r[0]));
    d.set(b, "y", &round_str(r[1]));
    d.set(b, "width", &round_str(r[2]));
    d.set(b, "height", &round_str(r[3]));
}

fn set_label(d: &mut Doc, owner: Id, r: R) {
    let l = match d.find(owner, &qn(BPMNDI_NS, "BPMNLabel")) {
        Some(l) => l,
        None => d.sub_element(owner, &qn(BPMNDI_NS, "BPMNLabel"), &[]),
    };
    set_bounds(d, l, r);
}

fn set_points(d: &mut Doc, owner: Id, pts: &[Pt]) {
    for w in d.findall(owner, &qn(DI_NS, "waypoint")) {
        d.remove_child(owner, w);
    }
    // waypoints come before the label
    let label = d.find(owner, &qn(BPMNDI_NS, "BPMNLabel"));
    if let Some(l) = label {
        d.remove_child(owner, l);
    }
    for (x, y) in pts {
        d.sub_element(
            owner,
            &qn(DI_NS, "waypoint"),
            &[("x", &round_str(*x)), ("y", &round_str(*y))],
        );
    }
    if let Some(l) = label {
        d.elems[owner].children.push(l);
    }
}

/// Groups have no shape in the automatic layout; the sidecar draws those it names in the
/// first diagram.
fn add_groups(md: &mut Model, sc: &Sidecar) {
    let root = md.doc.root;
    let Some(plane) = md
        .doc
        .find(root, &qn(BPMNDI_NS, "BPMNDiagram"))
        .and_then(|dg| md.doc.find(dg, &qn(BPMNDI_NS, "BPMNPlane")))
    else {
        return;
    };
    let drawn: HashSet<String> = md
        .doc
        .iter_tag(root, &qn(BPMNDI_NS, "BPMNShape"))
        .iter()
        .filter_map(|s| md.doc.get(*s, "bpmnElement").map(str::to_string))
        .collect();
    let mut added = false;
    for (id, nums) in &sc.entries {
        let is_group = md.get(Some(id)).is_some_and(|e| md.tag(e) == "group");
        if nums.len() != 4 || !is_group || drawn.contains(id) {
            continue;
        }
        let di = format!("{id}_di");
        let s = md.doc.sub_element(
            plane,
            &qn(BPMNDI_NS, "BPMNShape"),
            &[("id", &di), ("bpmnElement", id)],
        );
        set_bounds(
            &mut md.doc,
            s,
            [f(nums[0]), f(nums[1]), f(nums[2]), f(nums[3])],
        );
        added = true;
    }
    if added {
        md.reindex();
    }
}

fn apply(md: &mut Model, sc: &Sidecar) {
    if sc.is_empty() {
        return;
    }
    add_groups(md, sc);
    let d = &md.doc;
    let mut shapes: HashMap<String, Shape> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut edges: Vec<(String, Id, Option<R>)> = Vec::new();
    let mut bands: Vec<(Id, String)> = Vec::new();
    for s in d.iter(d.root) {
        let t = d.tag(s);
        if t == qn(BPMNDI_NS, "BPMNShape") {
            if let Some(act) = d.get(s, "choreographyActivityShape") {
                bands.push((s, act.to_string()));
                continue;
            }
            let (Some(el), Some(b)) = (d.get(s, "bpmnElement"), bounds_of(d, s)) else {
                continue;
            };
            if !shapes.contains_key(el) {
                order.push(el.to_string());
                shapes.insert(
                    el.to_string(),
                    Shape {
                        di: s,
                        auto: b,
                        label: label_of(d, s),
                    },
                );
            }
        } else if t == qn(BPMNDI_NS, "BPMNEdge")
            && let Some(el) = d.get(s, "bpmnElement")
        {
            edges.push((el.to_string(), s, label_of(d, s)));
        }
    }
    let mut lane_of: HashMap<String, String> = HashMap::new();
    for proc in &md.processes {
        for (lane, _, _) in md.lanes_of(*proc) {
            if !md.is_leaf_lane(lane) {
                continue;
            }
            let Some(lid) = md.id_of(lane) else {
                continue;
            };
            for r in md.doc.findall(lane, &m("flowNodeRef")) {
                if let Some(t) = md.doc.text(r).map(str::trim).filter(|t| !t.is_empty()) {
                    lane_of
                        .entry(t.to_string())
                        .or_insert_with(|| lid.to_string());
                }
            }
        }
    }
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for e in md.doc.iter(md.doc.root) {
        if let Some((_, a, b)) = ends(md, e) {
            adj.entry(a.clone()).or_default().push(b.clone());
            adj.entry(b).or_default().push(a);
        }
    }
    let mut p = Placer {
        md,
        shapes,
        order,
        fin: HashMap::new(),
        lane_of,
        adj,
    };
    p.run(sc);
    if p.fin.is_empty() {
        return;
    }
    let entries: HashMap<&str, &Vec<i64>> =
        sc.entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let labels: HashMap<&str, [i64; 4]> = sc.labels.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let sc_label =
        |id: &str| -> Option<R> { labels.get(id).map(|l| [f(l[0]), f(l[1]), f(l[2]), f(l[3])]) };
    // shapes
    let mut shape_writes: Vec<(Id, R, Option<R>)> = Vec::new();
    for id in &p.order {
        let s = &p.shapes[id];
        let Some((r, state)) = p.fin.get(id).copied() else {
            continue;
        };
        let moved = |l: R| [l[0] + r[0] - s.auto[0], l[1] + r[1] - s.auto[1], l[2], l[3]];
        let lb = match (state, sc_label(id)) {
            (State::Kept, Some(l)) if entries.contains_key(id.as_str()) => Some(l),
            _ => s.label.map(moved),
        };
        shape_writes.push((s.di, r, lb));
    }
    // edges
    let mut edge_writes: Vec<(Id, Vec<Pt>, Option<R>)> = Vec::new();
    for (id, di, auto_label) in &edges {
        let Some(e) = md.get(Some(id)) else {
            continue;
        };
        let Some((_, a, b)) = ends(md, e) else {
            continue;
        };
        let (Some((ra, sa)), Some((rb, sb))) = (p.fin.get(&a).copied(), p.fin.get(&b).copied())
        else {
            continue;
        };
        let stored: Option<Vec<Pt>> = entries.get(id.as_str()).and_then(|n| {
            (n.len() >= 4 && !(n.len() == 4 && p.shapes.contains_key(id)))
                .then(|| n.chunks(2).map(|c| (f(c[0]), f(c[1]))).collect::<Vec<Pt>>())
        });
        let keep = stored.as_ref().filter(|pts| {
            sa == State::Kept
                && sb == State::Kept
                && pts.first().is_some_and(|q| inside(*q, grow(ra, 25.0)))
                && pts.last().is_some_and(|q| inside(*q, grow(rb, 25.0)))
        });
        let (pts, lb) = match keep {
            Some(pts) => (pts.clone(), sc_label(id).or(*auto_label)),
            None => {
                let pts = route(&md.tag(e), ra, rb);
                let lb = auto_label.map(|l| edge_label(&pts, (l[2], l[3])));
                (pts, lb)
            }
        };
        if pts.len() >= 2 {
            edge_writes.push((*di, pts, lb));
        }
    }
    // bands follow their activity
    let mut band_writes: Vec<(Id, R)> = Vec::new();
    let di_of: HashMap<String, String> = p
        .shapes
        .iter()
        .filter_map(|(k, s)| md.doc.get(s.di, "id").map(|i| (i.to_string(), k.clone())))
        .collect();
    let mut by_act: HashMap<String, Vec<Id>> = HashMap::new();
    for (s, act) in &bands {
        by_act.entry(act.clone()).or_default().push(*s);
    }
    for (act, list) in &by_act {
        let Some(el) = di_of.get(act) else {
            continue;
        };
        let Some((r, _)) = p.fin.get(el).copied() else {
            continue;
        };
        let is_act = md.get(Some(el)).is_some_and(|e| is_choreo_act(&md.tag(e)));
        if !is_act {
            continue;
        }
        for (s, (b, _)) in list.iter().zip(choreo::bands(r, list.len())) {
            band_writes.push((*s, b));
        }
    }
    let d = &mut md.doc;
    for (di, r, lb) in shape_writes {
        set_bounds(d, di, r);
        if let Some(l) = lb {
            set_label(d, di, l);
        }
    }
    for (di, pts, lb) in edge_writes {
        set_points(d, di, &pts);
        if let Some(l) = lb {
            set_label(d, di, l);
        }
    }
    for (s, r) in band_writes {
        set_bounds(d, s, r);
    }
    md.reindex();
}
