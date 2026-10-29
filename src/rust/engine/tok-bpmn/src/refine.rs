//! Refinement of the classic layout: rerouting of flows and placement of labels.
//!
//! The classic layout (columns by longest path, rows by first predecessor, a few candidate
//! routes per flow) gives the starting solution. This module improves it in deterministic
//! passes:
//!
//! - every flow with a defect (crossing another edge, passing through a shape or a label,
//!   running on top of another flow or along a pool or lane border, a message flow crossing
//!   a pool or lane that holds neither of its ends, a boundary flow that does not leave
//!   downwards) gets alternative routes: the classic candidates, vertical channels for message
//!   flows, and orthogonal shortest paths on a grid of free channels that avoid all shapes;
//! - labels of events, gateways, data and flows are moved to the best free position among
//!   candidates around their element (below an element and next to the first long segment of
//!   a flow, as before, come first).
//!
//! A change is accepted only when it lowers the weighted cost and no quality measure of the
//! whole diagram becomes worse than in the classic solution, so the result is never worse
//! than the classic layout on any measure. Everything runs in a fixed order, without hashing
//! or randomness, on integer coordinates.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::layout::{OMap, Pt, key};
use crate::model::{Model, is_act, is_data_shape, is_evt, is_flownode, is_gw, is_subp};
use crate::svg::{flow_label_anchor, msg_label_anchor, wrap};
use crate::xml::Id;

pub(crate) type R = [f64; 4];

/// Quality measures, all counting defects (see `tests/quality/mod.rs`).
const CROSS: usize = 0;
const HITS: usize = 1;
const BENDS: usize = 2;
const MSG: usize = 3;
const OVL: usize = 4;
const BND: usize = 5;
const LBL_EDGE: usize = 6;
const LBL: usize = 7;
const N: usize = 8;
type Vector = [i64; N];
const WEIGHT: [f64; N] = [100.0, 1000.0, 20.0, 150.0, 400.0, 300.0, 50.0, 80.0];

/// Router costs, in hundredths of the weights above.
const C_LEN: i64 = 2;
const C_BEND: i64 = 2000;
const C_CROSS: i64 = 10_000;
const C_OVL: i64 = 40_000;
const C_LABEL: i64 = 5_000;
const C_REGION: i64 = 15_000;
const C_BORDER: i64 = 15_000;

/// Soft costs (same scale as `WEIGHT`).
const S_BORDER: f64 = 150.0;
const S_PARALLEL: f64 = 40.0;
const S_LEN: f64 = 0.02;
const S_OUTSIDE: f64 = 300.0;
const S_NEAR: f64 = 60.0;
/// Distance kept by routes from shapes they do not connect.
const CLEARANCE: f64 = 5.0;
/// Distance within which a message flow label reads as belonging to its neighbours.
const LABEL_NEAR: f64 = 6.0;
/// Preference cost of a label width that breaks a word inside.
const SPLIT_WORD: f64 = 6.0;
/// Preference cost of a message flow label inside a pool or lane holding neither end; it
/// outweighs any distance along the flow (the distance costs 1 per 10 units).
const S_FOREIGN: f64 = 60.0;
/// Distance kept from the borders of pools by sequence flows.
const POOL_INSET: f64 = 8.0;

const FONT: f64 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Seq,
    Msg,
    Other,
}

struct Node {
    r: R,
    tag: String,
    big_sub: bool,
}

struct Edge {
    kind: Kind,
    src: String,
    tgt: String,
    pts: Vec<Pt>,
    /// nodes a flow may cross without counting: its ends and their containers
    exempt: Vec<usize>,
    /// host of the boundary event the flow starts from
    host: Option<usize>,
    s_node: Option<usize>,
    t_node: Option<usize>,
    /// black-box ends (pool index)
    s_pool: Option<usize>,
    t_pool: Option<usize>,
    /// pools and lanes a message flow should not cross
    foreign: Vec<R>,
    /// containers of the ends (never obstacles)
    containers: Vec<usize>,
    area: R,
    label: Option<usize>,
    classic: Vec<Vec<Pt>>,
}

/// A label with its bounds as written into BPMNDI (`di`: x, y, width, height).
pub(crate) struct Label {
    pub owner: String,
    pub text: String,
    pub italic: bool,
    pub di: [i64; 4],
    lines: usize,
    chars: usize,
    /// the node (index) or flow (edge index) the label belongs to
    node: Option<usize>,
    edge: Option<usize>,
    wrap_w: f64,
}

#[allow(clippy::cast_precision_loss)]
fn fl(v: usize) -> f64 {
    v as f64
}

#[allow(clippy::cast_possible_truncation)]
fn to_i(v: f64) -> i64 {
    v.round() as i64
}

#[allow(clippy::cast_precision_loss)]
fn to_f(v: i64) -> f64 {
    v as f64
}

impl Label {
    fn sized(owner: &str, text: &str, wrap_w: f64, italic: bool) -> Label {
        let lines = wrap(text, wrap_w, FONT);
        let chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        Label {
            owner: owner.to_string(),
            text: text.to_string(),
            italic,
            di: [0, 0, 0, 0],
            lines: lines.len().max(1),
            chars,
            node: None,
            edge: None,
            wrap_w,
        }
    }

    /// Width of the bounds: even, and wide enough that wrapping the text at that width
    /// gives the same lines again.
    fn di_w(&self) -> i64 {
        2 * to_i((fl(self.chars) * 5.6 / 2.0).ceil())
    }

    fn di_h(&self) -> i64 {
        i64::try_from(self.lines).unwrap_or(1) * 12 - 1
    }

    /// The area covered by the text, as drawn (centred in the bounds, first baseline
    /// 8 below their top).
    fn text_box(&self, di: [i64; 4]) -> R {
        let c = fl(self.chars);
        let cx = to_f(di[0]) + to_f(di[2]) / 2.0;
        [
            cx - c * 2.75,
            to_f(di[1]) + 0.5,
            c * 5.5,
            (fl(self.lines) - 1.0) * 12.0 + 10.0,
        ]
    }

    /// Bounds placing the text box with its top-left corner at (x0, top).
    fn at(&self, x0: f64, top: f64) -> [i64; 4] {
        let w = self.di_w();
        let c = fl(self.chars);
        [
            to_i(x0 + c * 2.75 - to_f(w) / 2.0),
            to_i(top - 0.5),
            w,
            self.di_h(),
        ]
    }

    fn size(&self) -> (f64, f64) {
        (fl(self.chars) * 5.5, (fl(self.lines) - 1.0) * 12.0 + 10.0)
    }
}

// ------------------------------------------------------------------ geometry

/// Side of a cell of [`Buckets`].
const CELL: f64 = 100.0;
/// A box spanning more cells than this is kept in a list that every query returns.
const MAX_CELLS: i64 = 4096;

/// A uniform grid over the drawing: for a rectangle, the items whose boxes may overlap it.
/// It makes the measures that look for shapes, flows or labels near a route or a label
/// independent of the size of the diagram; every measure counts what it finds, so it gives
/// the same result as a scan of all items.
#[derive(Default)]
struct Buckets {
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// the cells of each item: `(x0, y0, x1, y1)`, or `None` for an item everywhere
    spans: Vec<Option<(i64, i64, i64, i64)>>,
    everywhere: Vec<usize>,
}

/// The cells a box covers, widened by a margin that absorbs rounding at cell borders.
fn cell_span(r: R) -> Option<(i64, i64, i64, i64)> {
    let c = |v: f64| (v / CELL).floor();
    let (x0, y0) = (c(r[0] - 2.0), c(r[1] - 2.0));
    let (x1, y1) = (c(r[0] + r[2].max(0.0) + 2.0), c(r[1] + r[3].max(0.0) + 2.0));
    if ![x0, y0, x1, y1]
        .iter()
        .all(|v| v.is_finite() && v.abs() < 1e12)
    {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let span = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
    ((span.2 - span.0 + 1).saturating_mul(span.3 - span.1 + 1) <= MAX_CELLS).then_some(span)
}

impl Buckets {
    fn new(boxes: impl IntoIterator<Item = R>) -> Buckets {
        let mut g = Buckets::default();
        for r in boxes {
            g.spans.push(None);
            g.put(g.spans.len() - 1, r);
        }
        g
    }

    fn put(&mut self, i: usize, r: R) {
        let span = cell_span(r);
        self.spans[i] = span;
        match span {
            None => self.everywhere.push(i),
            Some((x0, y0, x1, y1)) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        self.cells.entry((x, y)).or_default().push(i);
                    }
                }
            }
        }
    }

    /// Item `i` now has box `r`.
    fn update(&mut self, i: usize, r: R) {
        match self.spans[i] {
            None => self.everywhere.retain(|&k| k != i),
            Some((x0, y0, x1, y1)) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        if let Some(v) = self.cells.get_mut(&(x, y)) {
                            v.retain(|&k| k != i);
                        }
                    }
                }
            }
        }
        self.put(i, r);
    }

    /// The items whose boxes may overlap `r` (a superset of those that do), in
    /// increasing order.
    fn near(&self, r: R) -> Vec<usize> {
        let mut out = self.everywhere.clone();
        match cell_span(r) {
            None => out.extend(0..self.spans.len()),
            Some((x0, y0, x1, y1)) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        if let Some(v) = self.cells.get(&(x, y)) {
                            out.extend_from_slice(v);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn shrink(r: R, d: f64) -> R {
    [r[0] + d, r[1] + d, r[2] - 2.0 * d, r[3] - 2.0 * d]
}

/// The segment passes through the open interior of the rectangle.
fn seg_in_rect(a: Pt, b: Pt, r: R) -> bool {
    if r[2] <= 0.0 || r[3] <= 0.0 {
        return false;
    }
    let (x0, y0, x1, y1) = (r[0], r[1], r[0] + r[2], r[1] + r[3]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    for (p, q) in [
        (-dx, a.0 - x0),
        (dx, x1 - a.0),
        (-dy, a.1 - y0),
        (dy, y1 - a.1),
    ] {
        if p.abs() < 1e-12 {
            if q <= 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    t1 - t0 > 1e-9
}

fn poly_in_rect(pts: &[Pt], r: R) -> bool {
    pts.windows(2).any(|w| seg_in_rect(w[0], w[1], r))
}

fn grow(r: R, d: f64) -> R {
    shrink(r, -d)
}

/// Boxes more than one unit apart: routes in them neither cross nor overlap.
fn apart(a: R, b: R) -> bool {
    a[0] > b[0] + b[2] + 1.0
        || b[0] > a[0] + a[2] + 1.0
        || a[1] > b[1] + b[3] + 1.0
        || b[1] > a[1] + a[3] + 1.0
}

fn rects_overlap(a: R, b: R) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn contains(outer: R, inner: R) -> bool {
    inner[0] >= outer[0] - 0.5
        && inner[1] >= outer[1] - 0.5
        && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
        && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
}

fn orient(a: Pt, b: Pt, c: Pt) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn seg_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let eps = 1e-6
        * (1.0 + (b.0 - a.0).abs() + (b.1 - a.1).abs())
        * (1.0 + (d.0 - c.0).abs() + (d.1 - c.1).abs());
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
}

fn seg_overlap(a: Pt, b: Pt, c: Pt, d: Pt, min: f64) -> bool {
    let horiz = |p: Pt, q: Pt| (p.1 - q.1).abs() < 0.5 && (p.0 - q.0).abs() > 0.5;
    let vert = |p: Pt, q: Pt| (p.0 - q.0).abs() < 0.5 && (p.1 - q.1).abs() > 0.5;
    if horiz(a, b) && horiz(c, d) && (a.1 - c.1).abs() < 1.0 {
        let lo = a.0.min(b.0).max(c.0.min(d.0));
        let hi = a.0.max(b.0).min(c.0.max(d.0));
        return hi - lo > min;
    }
    if vert(a, b) && vert(c, d) && (a.0 - c.0).abs() < 1.0 {
        let lo = a.1.min(b.1).max(c.1.min(d.1));
        let hi = a.1.max(b.1).min(c.1.max(d.1));
        return hi - lo > min;
    }
    false
}

fn bends(pts: &[Pt]) -> i64 {
    let mut dirs: Vec<Pt> = Vec::new();
    for w in pts.windows(2) {
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let l = (dx * dx + dy * dy).sqrt();
        if l < 0.5 {
            continue;
        }
        dirs.push((dx / l, dy / l));
    }
    let n = dirs
        .windows(2)
        .filter(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs() > 1e-3)
        .count();
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn length(pts: &[Pt]) -> f64 {
    pts.windows(2)
        .map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs())
        .sum()
}

fn bbox_of(pts: &[Pt]) -> R {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in pts {
        b[0] = b[0].min(p.0);
        b[1] = b[1].min(p.1);
        b[2] = b[2].max(p.0);
        b[3] = b[3].max(p.1);
    }
    [b[0], b[1], b[2] - b[0], b[3] - b[1]]
}

fn inside(pts: &[Pt], area: R) -> bool {
    pts.iter().all(|p| {
        p.0 >= area[0] - 0.5
            && p.0 <= area[0] + area[2] + 0.5
            && p.1 >= area[1] - 0.5
            && p.1 <= area[1] + area[3] + 0.5
    })
}

/// Remove repeated and collinear middle points.
fn simplify(pts: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::new();
    for &p in pts {
        if out
            .last()
            .is_some_and(|q| (q.0 - p.0).abs() < 0.5 && (q.1 - p.1).abs() < 0.5)
        {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let same_x = (a.0 - b.0).abs() < 0.5 && (b.0 - p.0).abs() < 0.5;
            let same_y = (a.1 - b.1).abs() < 0.5 && (b.1 - p.1).abs() < 0.5;
            let forward = (b.0 - a.0) * (p.0 - b.0) + (b.1 - a.1) * (p.1 - b.1) > 0.0;
            if (same_x || same_y) && forward {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

fn round_pts(pts: &[Pt]) -> Vec<Pt> {
    pts.iter()
        .map(|p| (p.0.round_ties_even(), p.1.round_ties_even()))
        .collect()
}

// ------------------------------------------------------------------ scene

/// Geometry of the laid-out diagram handed over by `layout_model`.
pub(crate) struct Input<'a> {
    pub out: &'a OMap<[i64; 4]>,
    pub lanes: &'a OMap<[i64; 4]>,
    /// left edge of the content of each pool (right of the pool and lane headers)
    pub content_x: &'a HashMap<String, i64>,
    pub wp: &'a OMap<Vec<Pt>>,
    /// classic candidate routes of each flow
    pub classic: &'a HashMap<String, Vec<Vec<Pt>>>,
}

/// Quality of a refined layout: measures, their starting values (the budget), weighted
/// score and area of the drawing before and after the refinement.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Quality {
    total: Vector,
    budget: Vector,
    score: f64,
    area0: f64,
    area: f64,
}

impl Quality {
    /// Better than `classic` while no measure exceeds its starting value in `classic` and the
    /// drawing is not larger.
    pub(crate) fn better_than(&self, classic: &Quality) -> bool {
        (0..N).all(|k| self.total[k] <= classic.budget[k])
            && self.area <= classic.area0
            && self.score < classic.score - 0.5
    }
}

pub(crate) struct Scene {
    nodes: Vec<Node>,
    node_ix: HashMap<String, usize>,
    pools: Vec<(String, R)>,
    pub edge_ids: Vec<String>,
    edges: Vec<Edge>,
    pub labels: Vec<Label>,
    borders_h: Vec<(f64, f64, f64)>,
    borders_v: Vec<(f64, f64, f64)>,
    /// title bands of pools and lanes, kept free of labels
    headers: Vec<R>,
    drawing: R,
    total: Vector,
    budget: Vector,
    /// the boxes of the nodes, of the routes of the edges and of the text of the labels
    node_grid: Buckets,
    edge_grid: Buckets,
    label_grid: Buckets,
}

#[allow(clippy::cast_precision_loss)]
fn rect(r: [i64; 4]) -> R {
    [r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]
}

fn ancestors(md: &Model, e: Id) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = md.parent.get(&e).copied();
    while let Some(x) = p {
        let t = md.tag(x);
        if t == "process" {
            break;
        }
        if is_subp(&t) {
            out.push(key(md.id_of(x)));
        }
        p = md.parent.get(&x).copied();
    }
    out
}

fn is_node_tag(t: &str) -> bool {
    is_flownode(t) || is_data_shape(t) || t == "textAnnotation"
}

/// Text of the external label of a node (events, gateways, data), as drawn.
fn node_label_text(md: &Model, e: Id, t: &str) -> String {
    let nm = md.name(Some(e));
    if t == "dataObjectReference" {
        let dobj = md.get(md.attr(e, "dataObjectRef"));
        let mut lab = if nm.is_empty() {
            dobj.map(|o| md.name(Some(o))).unwrap_or_default()
        } else {
            nm
        };
        let st = md.doc.find(e, &crate::model::m("dataState"));
        if let Some(stn) = st
            .and_then(|x| md.attr(x, "name"))
            .filter(|v| !v.is_empty())
        {
            lab.push_str(&format!(" [{stn}]"));
        }
        return lab;
    }
    if t == "dataStoreReference" {
        if nm.is_empty() {
            return md
                .get(md.attr(e, "dataStoreRef"))
                .map(|o| md.name(Some(o)))
                .unwrap_or_default();
        }
        return nm;
    }
    nm
}

impl Scene {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn build(md: &Model, inp: &Input) -> Scene {
        let mut nodes = Vec::new();
        let mut node_ix = HashMap::new();
        let mut pools = Vec::new();
        for i in &inp.out.keys {
            let Some(e) = md.get(Some(i)) else {
                continue;
            };
            let t = md.tag(e);
            let r = rect(inp.out.map[i]);
            if t == "participant" {
                pools.push((i.clone(), r));
            } else if is_node_tag(&t) {
                node_ix.insert(i.clone(), nodes.len());
                nodes.push(Node {
                    r,
                    big_sub: is_subp(&t) && r[2] > 100.0,
                    tag: t,
                });
            }
        }
        let mut lanes = Vec::new();
        let mut borders_h = Vec::new();
        let mut borders_v = Vec::new();
        let mut headers = Vec::new();
        for i in &inp.lanes.keys {
            let r = rect(inp.lanes.map[i]);
            let mut set = HashSet::new();
            if let Some(l) = md.get(Some(i)) {
                for d in md.doc.iter_tag(l, &crate::model::m("flowNodeRef")) {
                    set.insert(md.doc.text(d).unwrap_or("").trim().to_string());
                }
            }
            borders_h.push((r[1], r[0], r[0] + r[2]));
            borders_h.push((r[1] + r[3], r[0], r[0] + r[2]));
            borders_v.push((r[0] + 30.0, r[1], r[1] + r[3]));
            headers.push([r[0], r[1], 30.0, r[3]]);
            lanes.push((r, set));
        }
        for (_, r) in &pools {
            borders_h.push((r[1], r[0], r[0] + r[2]));
            borders_h.push((r[1] + r[3], r[0], r[0] + r[2]));
            headers.push([r[0], r[1], 30.0, r[3]]);
            borders_v.push((r[0], r[1], r[1] + r[3]));
            borders_v.push((r[0] + r[2], r[1], r[1] + r[3]));
        }
        let pool_ix = |id: &str| pools.iter().position(|(p, _)| p == id);

        // the whole drawing: nothing may be routed outside it
        let mut all = Vec::new();
        for i in &inp.out.keys {
            let r = rect(inp.out.map[i]);
            all.push((r[0], r[1]));
            all.push((r[0] + r[2], r[1] + r[3]));
        }
        for i in &inp.lanes.keys {
            let r = rect(inp.lanes.map[i]);
            all.push((r[0], r[1]));
            all.push((r[0] + r[2], r[1] + r[3]));
        }
        for i in &inp.wp.keys {
            all.extend(round_pts(&inp.wp.map[i]));
        }
        let drawing = bbox_of(&all);
        let msg_x0 = inp
            .content_x
            .values()
            .copied()
            .max()
            .map_or(drawing[0], to_f);

        let mut edge_ids = Vec::new();
        let mut edges = Vec::new();
        for fid in &inp.wp.keys {
            let pts = round_pts(&inp.wp.map[fid]);
            if pts.len() < 2 {
                continue;
            }
            let Some(e) = md.get(Some(fid)) else {
                continue;
            };
            let t = md.tag(e);
            let kind = match t.as_str() {
                "sequenceFlow" => Kind::Seq,
                "messageFlow" => Kind::Msg,
                _ => Kind::Other,
            };
            let (src, tgt) = match t.as_str() {
                "dataInputAssociation" => (
                    md.doc
                        .find(e, &crate::model::m("sourceRef"))
                        .map(|s| md.doc.text(s).unwrap_or("").trim().to_string())
                        .unwrap_or_default(),
                    md.parent
                        .get(&e)
                        .and_then(|p| md.id_of(*p))
                        .unwrap_or("")
                        .to_string(),
                ),
                "dataOutputAssociation" => (
                    md.parent
                        .get(&e)
                        .and_then(|p| md.id_of(*p))
                        .unwrap_or("")
                        .to_string(),
                    md.doc
                        .find(e, &crate::model::m("targetRef"))
                        .map(|s| md.doc.text(s).unwrap_or("").trim().to_string())
                        .unwrap_or_default(),
                ),
                _ => (
                    md.attr(e, "sourceRef").unwrap_or("").to_string(),
                    md.attr(e, "targetRef").unwrap_or("").to_string(),
                ),
            };
            let se = md.get(Some(&src));
            let te = md.get(Some(&tgt));
            let mut containers = Vec::new();
            for x in [se, te].into_iter().flatten() {
                for a in ancestors(md, x) {
                    if let Some(&k) = node_ix.get(&a) {
                        containers.push(k);
                    }
                }
            }
            let s_node = node_ix.get(&src).copied();
            let t_node = node_ix.get(&tgt).copied();
            let host = se
                .filter(|x| md.tag(*x) == "boundaryEvent")
                .and_then(|x| md.attr(x, "attachedToRef"))
                .and_then(|h| node_ix.get(h).copied());
            let mut exempt: Vec<usize> = containers.clone();
            exempt.extend(s_node);
            exempt.extend(t_node);
            exempt.extend(host);
            let mut foreign = Vec::new();
            let (mut s_pool, mut t_pool) = (None, None);
            if kind == Kind::Msg {
                let pool_of = |x: Option<Id>| md.participant_of(x).map(|p| key(md.id_of(p)));
                let mine = [pool_of(se), pool_of(te)];
                if se.is_some_and(|x| md.tag(x) == "participant") {
                    s_pool = pool_ix(&src);
                }
                if te.is_some_and(|x| md.tag(x) == "participant") {
                    t_pool = pool_ix(&tgt);
                }
                for (pid, r) in &pools {
                    if !mine.contains(&Some(pid.clone())) {
                        foreign.push(shrink(*r, 1.0));
                    }
                }
                let own: Vec<R> = mine
                    .iter()
                    .flatten()
                    .filter_map(|p| pools.iter().find(|(q, _)| q == p).map(|(_, r)| *r))
                    .collect();
                for (r, set) in &lanes {
                    let holds = set.contains(&src) || set.contains(&tgt);
                    if !holds && own.iter().any(|o| contains(*o, *r)) {
                        foreign.push(shrink(*r, 1.0));
                    }
                }
            }
            // routing area: the sub-process or the pool content, inside the drawing
            let mut area = drawing;
            if kind == Kind::Seq {
                let parent = md.parent.get(&e).copied();
                if let Some(p) = parent {
                    let pt = md.tag(p);
                    if is_subp(&pt) {
                        if let Some(&k) = node_ix.get(&key(md.id_of(p))) {
                            area = shrink(nodes[k].r, 2.0);
                        }
                    } else if let Some(part) = md.participant_of(Some(p)) {
                        let pid = key(md.id_of(part));
                        if let Some((_, r)) = pools.iter().find(|(q, _)| *q == pid) {
                            let x0 = inp.content_x.get(&pid).copied().map_or(r[0], to_f);
                            area = [
                                x0 + POOL_INSET,
                                r[1] + POOL_INSET,
                                r[0] + r[2] - x0 - 2.0 * POOL_INSET,
                                r[3] - 2.0 * POOL_INSET,
                            ];
                        }
                    }
                }
            } else if kind == Kind::Msg {
                area = [
                    msg_x0 + 2.0,
                    drawing[1],
                    drawing[0] + drawing[2] - msg_x0 - 2.0,
                    drawing[3],
                ];
            }
            let classic: Vec<Vec<Pt>> = inp
                .classic
                .get(fid)
                .map(|v| v.iter().map(|c| round_pts(c)).collect())
                .unwrap_or_default();
            edge_ids.push(fid.clone());
            edges.push(Edge {
                kind,
                src,
                tgt,
                pts,
                exempt,
                host,
                s_node,
                t_node,
                s_pool,
                t_pool,
                foreign,
                containers,
                area,
                label: None,
                classic,
            });
        }

        let mut sc = Scene {
            nodes,
            node_ix,
            pools,
            edge_ids,
            edges,
            labels: Vec::new(),
            borders_h,
            borders_v,
            headers,
            drawing,
            total: [0; N],
            budget: [0; N],
            node_grid: Buckets::default(),
            edge_grid: Buckets::default(),
            label_grid: Buckets::default(),
        };
        sc.make_labels(md, inp);
        sc.node_grid = Buckets::new(sc.nodes.iter().map(|n| n.r));
        sc.edge_grid = Buckets::new(sc.edges.iter().map(|e| bbox_of(&e.pts)));
        sc.label_grid = Buckets::new((0..sc.labels.len()).map(|l| sc.label_box(l)));
        sc.total = sc.totals();
        sc.budget = sc.total;
        sc
    }

    /// Labels at the positions of the classic drawing.
    fn make_labels(&mut self, md: &Model, inp: &Input) {
        for i in &inp.out.keys {
            let Some(&k) = self.node_ix.get(i) else {
                continue;
            };
            let Some(e) = md.get(Some(i)) else {
                continue;
            };
            let t = self.nodes[k].tag.clone();
            if !(is_evt(&t) || is_gw(&t) || is_data_shape(&t)) {
                continue;
            }
            let text = node_label_text(md, e, &t);
            if text.is_empty() {
                continue;
            }
            let ww = if is_gw(&t) { 120.0 } else { 110.0 };
            let mut l = Label::sized(i, &text, ww, false);
            let r = self.nodes[k].r;
            let (w, _) = l.size();
            l.di = l.at(r[0] + r[2] / 2.0 - w / 2.0, r[1] + r[3] + 13.0 - 7.5);
            l.node = Some(k);
            self.labels.push(l);
        }
        for (ix, fid) in self.edge_ids.iter().enumerate() {
            let ed = &self.edges[ix];
            if ed.kind == Kind::Other {
                continue;
            }
            let Some(e) = md.get(Some(fid)) else {
                continue;
            };
            let nm = md.name(Some(e));
            if nm.is_empty() {
                continue;
            }
            let msg = ed.kind == Kind::Msg;
            let mut l = Label::sized(fid, &nm, if msg { 110.0 } else { 90.0 }, msg);
            l.edge = Some(ix);
            l.di = classic_flow_label(&l, &ed.pts, msg);
            self.edges[ix].label = Some(self.labels.len());
            self.labels.push(l);
        }
    }

    // -------------------------------------------------------------- measures

    fn edge_terms(&self, e: usize, pts: &[Pt], v: &mut Vector) {
        let ed = &self.edges[e];
        for k in self.node_grid.near(bbox_of(pts)) {
            if !ed.exempt.contains(&k) && poly_in_rect(pts, shrink(self.nodes[k].r, 3.0)) {
                v[HITS] += 1;
            }
        }
        if ed.kind != Kind::Other {
            v[BENDS] += bends(pts);
        }
        if ed.kind == Kind::Msg {
            for r in &ed.foreign {
                if poly_in_rect(pts, *r) {
                    v[MSG] += 1;
                }
            }
        }
        if let Some(h) = ed.host.filter(|_| ed.kind == Kind::Seq) {
            let (a, b) = (pts[0], pts[1]);
            if !(b.1 > a.1 + 0.5 && (b.0 - a.0).abs() < 0.5) {
                v[BND] += 1;
            }
            if poly_in_rect(pts, shrink(self.nodes[h].r, 3.0)) {
                v[BND] += 1;
            }
            let left: f64 = pts.windows(2).map(|w| (w[0].0 - w[1].0).max(0.0)).sum();
            let need = (pts[0].0 - pts[pts.len() - 1].0).max(0.0);
            if left > need + 1.0 {
                v[BND] += 1;
            }
        }
    }

    fn pair_terms(&self, e: usize, pts: &[Pt], f: usize, qts: &[Pt], v: &mut Vector) {
        let (a, b) = (&self.edges[e], &self.edges[f]);
        let mut ovl = false;
        for s in pts.windows(2) {
            for t in qts.windows(2) {
                if seg_cross(s[0], s[1], t[0], t[1]) {
                    v[CROSS] += 1;
                }
                if !ovl && seg_overlap(s[0], s[1], t[0], t[1], 4.0) {
                    ovl = true;
                }
            }
        }
        if ovl && a.src != b.src && a.tgt != b.tgt {
            v[OVL] += 1;
        }
    }

    /// Overlaps of a label with shapes and pool borders.
    fn label_fixed_terms(&self, tb: R) -> i64 {
        let ra = shrink(tb, 1.0);
        let mut n = 0;
        for k in self.node_grid.near(ra) {
            let nd = &self.nodes[k];
            if !nd.big_sub && rects_overlap(ra, nd.r) {
                n += 1;
            }
        }
        for (_, r) in &self.pools {
            if rects_overlap(ra, *r) && !contains(*r, tb) {
                n += 1;
            }
        }
        n
    }

    fn label_box(&self, l: usize) -> R {
        let lb = &self.labels[l];
        lb.text_box(lb.di)
    }

    fn totals(&self) -> Vector {
        let mut v = [0; N];
        for e in 0..self.edges.len() {
            let pts = &self.edges[e].pts;
            self.edge_terms(e, pts, &mut v);
            // two routes whose boxes are apart neither cross nor overlap
            let eb = bbox_of(pts);
            for f in self.edge_grid.near(grow(eb, 1.0)) {
                if f > e && !apart(eb, bbox_of(&self.edges[f].pts)) {
                    self.pair_terms(e, pts, f, &self.edges[f].pts, &mut v);
                }
            }
        }
        let boxes: Vec<R> = (0..self.labels.len()).map(|l| self.label_box(l)).collect();
        for (a, ba) in boxes.iter().enumerate() {
            let ra = shrink(*ba, 1.0);
            for b in self.label_grid.near(ra) {
                if b > a && rects_overlap(ra, shrink(boxes[b], 1.0)) {
                    v[LBL] += 1;
                }
            }
            v[LBL] += self.label_fixed_terms(*ba);
            for f in self.edge_grid.near(ra) {
                if poly_in_rect(&self.edges[f].pts, ra) {
                    v[LBL_EDGE] += 1;
                }
            }
        }
        v
    }

    /// All terms involving edge `e` (with route `pts`) and its label (with bounds `ldi`).
    fn local(&self, e: usize, pts: &[Pt], ldi: Option<[i64; 4]>) -> Vector {
        let mut v = [0; N];
        self.edge_terms(e, pts, &mut v);
        let eb = bbox_of(pts);
        for f in self.edge_grid.near(grow(eb, 1.0)) {
            if f == e {
                continue;
            }
            let q = &self.edges[f].pts;
            if apart(eb, bbox_of(q)) {
                continue;
            }
            self.pair_terms(e, pts, f, q, &mut v);
        }
        let own = self.edges[e].label;
        for l in self.label_grid.near(eb) {
            if Some(l) == own {
                continue;
            }
            if poly_in_rect(pts, shrink(self.label_box(l), 1.0)) {
                v[LBL_EDGE] += 1;
            }
        }
        if let (Some(l), Some(di)) = (own, ldi) {
            let tb = self.labels[l].text_box(di);
            self.label_terms(l, tb, Some((e, pts)), &mut v);
        }
        v
    }

    /// Terms of label `l` placed at text box `tb`; `edge` replaces the route of one edge.
    fn label_terms(&self, l: usize, tb: R, edge: Option<(usize, &[Pt])>, v: &mut Vector) {
        let ra = shrink(tb, 1.0);
        for m in self.label_grid.near(ra) {
            if m != l && rects_overlap(ra, shrink(self.label_box(m), 1.0)) {
                v[LBL] += 1;
            }
        }
        v[LBL] += self.label_fixed_terms(tb);
        for f in self.edge_grid.near(ra) {
            if edge.is_some_and(|(e, _)| e == f) {
                continue;
            }
            if poly_in_rect(&self.edges[f].pts, ra) {
                v[LBL_EDGE] += 1;
            }
        }
        // the edge with another route: wherever that route runs
        if let Some((_, p)) = edge
            && poly_in_rect(p, ra)
        {
            v[LBL_EDGE] += 1;
        }
    }

    /// Soft cost of a route: runs along borders, close parallel message flows, length.
    fn soft(&self, e: usize, pts: &[Pt]) -> f64 {
        let mut c = S_LEN * length(pts);
        let ed = &self.edges[e];
        for k in self.node_grid.near(grow(bbox_of(pts), CLEARANCE)) {
            if ed.exempt.contains(&k) {
                continue;
            }
            let n = &self.nodes[k];
            if poly_in_rect(pts, shrink(n.r, -CLEARANCE)) && !poly_in_rect(pts, shrink(n.r, 3.0)) {
                c += S_NEAR;
            }
        }
        if !inside(pts, self.edges[e].area) {
            c += S_OUTSIDE;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a.1 - b.1).abs() < 0.5 {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                if self.borders_h.iter().any(|(y, bx0, bx1)| {
                    (a.1 - y).abs() <= 2.0 && x1.min(*bx1) - x0.max(*bx0) > 4.0
                }) {
                    c += S_BORDER;
                }
            } else if (a.0 - b.0).abs() < 0.5 {
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                if self.borders_v.iter().any(|(x, by0, by1)| {
                    (a.0 - x).abs() <= 2.0 && y1.min(*by1) - y0.max(*by0) > 4.0
                }) {
                    c += S_BORDER;
                }
            }
        }
        if self.edges[e].kind == Kind::Msg {
            for (f, ed) in self.edges.iter().enumerate() {
                if f == e || ed.kind != Kind::Msg {
                    continue;
                }
                if close_parallel(pts, &ed.pts) {
                    c += S_PARALLEL;
                }
            }
        }
        c
    }

    /// Keep every measure within the starting values of another layout of the same model.
    pub(crate) fn limit(&mut self, other: &Quality) {
        self.budget = other.budget;
    }

    /// Quality of the current state; `area0` and `area` are the areas of the drawing before
    /// and after the refinement.
    pub(crate) fn quality(&self, area0: f64, area: f64) -> Quality {
        #[allow(clippy::cast_precision_loss)]
        let score = (0..N).map(|k| WEIGHT[k] * self.total[k] as f64).sum();
        Quality {
            total: self.total,
            budget: self.budget,
            score,
            area0,
            area,
        }
    }

    /// Current route of edge `k` (in the order of `edge_ids`).
    pub(crate) fn route(&self, k: usize) -> &[Pt] {
        &self.edges[k].pts
    }

    // -------------------------------------------------------------- routing

    /// Labels, routes, labels again.
    pub(crate) fn refine(&mut self) {
        self.place_labels();
        self.reroute();
        self.place_labels();
    }

    /// Improve the routes of sequence and message flows.
    fn reroute(&mut self) {
        // an edge is tried again only when something changed near it
        let mut dirty = vec![true; self.edges.len()];
        for _ in 0..3 {
            let mut changed = false;
            for e in 0..self.edges.len() {
                if self.edges[e].kind == Kind::Other || !dirty[e] {
                    continue;
                }
                dirty[e] = false;
                let before = self.edges[e].pts.clone();
                if !self.improve_edge(e) {
                    continue;
                }
                changed = true;
                let mut pts = before;
                pts.extend(self.edges[e].pts.iter().copied());
                let b = bbox_of(&pts);
                let zone = [b[0] - 60.0, b[1] - 60.0, b[2] + 120.0, b[3] + 120.0];
                for f in self.edge_grid.near(zone) {
                    if f != e && rects_overlap(zone, bbox_of(&self.edges[f].pts)) {
                        dirty[f] = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn flow_label_at(&self, e: usize, pts: &[Pt]) -> Option<[i64; 4]> {
        let l = self.edges[e].label?;
        let lb = &self.labels[l];
        Some(classic_flow_label(lb, pts, lb.italic))
    }

    fn improve_edge(&mut self, e: usize) -> bool {
        let cur_pts = self.edges[e].pts.clone();
        let cur_ldi = self.edges[e].label.map(|l| self.labels[l].di);
        let cur = self.local(e, &cur_pts, cur_ldi);
        let cur_soft = self.soft(e, &cur_pts);
        let defect =
            cur[CROSS] + cur[HITS] + cur[MSG] + cur[OVL] + cur[BND] + cur[LBL_EDGE] + cur[LBL] > 0
                || cur[BENDS] >= 2
                || cur_soft > S_LEN * length(&cur_pts) + 0.5;
        if !defect {
            return false;
        }
        let alts = self.alternatives(e);
        // score, route, bounds of the label, measures
        type Choice = (f64, Vec<Pt>, Option<[i64; 4]>, Vector);
        let mut best: Option<Choice> = None;
        for alt in alts {
            if alt.len() < 2 || alt == cur_pts || !inside(&alt, self.edges[e].area) {
                continue;
            }
            let ldi = self.flow_label_at(e, &alt);
            let v = self.local(e, &alt, ldi);
            let mut ok = true;
            let mut score = self.soft(e, &alt) - cur_soft;
            for k in 0..N {
                let d = v[k] - cur[k];
                if d > 0 && self.total[k] + d > self.budget[k] {
                    ok = false;
                    break;
                }
                #[allow(clippy::cast_precision_loss)]
                {
                    score += WEIGHT[k] * d as f64;
                }
            }
            if !ok || score > -0.5 {
                continue;
            }
            if best.as_ref().is_none_or(|b| score < b.0) {
                best = Some((score, alt, ldi, v));
            }
        }
        let Some((_, pts, ldi, v)) = best else {
            return false;
        };
        for k in 0..N {
            self.total[k] += v[k] - cur[k];
        }
        self.edge_grid.update(e, bbox_of(&pts));
        self.edges[e].pts = pts;
        if let (Some(l), Some(di)) = (self.edges[e].label, ldi) {
            self.labels[l].di = di;
            self.label_grid.update(l, self.label_box(l));
        }
        true
    }

    fn alternatives(&self, e: usize) -> Vec<Vec<Pt>> {
        let ed = &self.edges[e];
        let mut alts: Vec<Vec<Pt>> = ed.classic.clone();
        if ed.kind == Kind::Msg {
            alts.extend(self.msg_channels(e));
        }
        alts.extend(self.grid_routes(e));
        let mut seen: Vec<Vec<Pt>> = Vec::new();
        for a in alts {
            let a = simplify(&a);
            if !seen.contains(&a) {
                seen.push(a);
            }
        }
        seen
    }

    /// Straight vertical message flows at several positions along the sides of the nodes.
    fn msg_channels(&self, e: usize) -> Vec<Vec<Pt>> {
        let ed = &self.edges[e];
        let mut out: Vec<Vec<Pt>> = Vec::new();
        let xs_of = |k: usize| -> Vec<f64> {
            let n = &self.nodes[k];
            let cx = n.r[0] + n.r[2] / 2.0;
            if is_act(&n.tag) {
                vec![cx, cx - 20.0, cx + 20.0, cx - 35.0, cx + 35.0]
            } else {
                vec![cx]
            }
        };
        let vert = |n: R, p: R, x: f64| -> Vec<Pt> {
            if p[1] >= n[1] + n[3] {
                vec![(x, n[1] + n[3]), (x, p[1])]
            } else {
                vec![(x, n[1]), (x, p[1] + p[3])]
            }
        };
        match (ed.s_node, ed.t_node, ed.s_pool, ed.t_pool) {
            (Some(s), None, None, Some(p)) => {
                let pr = self.pools[p].1;
                for x in xs_of(s) {
                    if x >= pr[0] + 40.0 && x <= pr[0] + pr[2] - 10.0 {
                        out.push(vert(self.nodes[s].r, pr, x));
                    }
                }
            }
            (None, Some(t), Some(p), None) => {
                let pr = self.pools[p].1;
                for x in xs_of(t) {
                    if x >= pr[0] + 40.0 && x <= pr[0] + pr[2] - 10.0 {
                        let mut v = vert(self.nodes[t].r, pr, x);
                        v.reverse();
                        out.push(v);
                    }
                }
            }
            (Some(s), Some(t), _, _) => {
                let (sr, tr) = (self.nodes[s].r, self.nodes[t].r);
                for x in xs_of(s) {
                    if xs_of(t).iter().any(|x2| (x2 - x).abs() < 0.5)
                        || (x > tr[0] + 5.0
                            && x < tr[0] + tr[2] - 5.0
                            && is_act(&self.nodes[t].tag))
                    {
                        if tr[1] >= sr[1] + sr[3] {
                            out.push(vec![(x, sr[1] + sr[3]), (x, tr[1])]);
                        } else if sr[1] >= tr[1] + tr[3] {
                            out.push(vec![(x, sr[1]), (x, tr[1] + tr[3])]);
                        }
                    }
                }
            }
            _ => {}
        }
        let ends: Vec<Pt> = self
            .edges
            .iter()
            .enumerate()
            .filter(|(f, _)| *f != e)
            .flat_map(|(_, fe)| [fe.pts[0], fe.pts[fe.pts.len() - 1]])
            .collect();
        out.retain(|c| {
            [c[0], c[c.len() - 1]].iter().all(|p| {
                !ends
                    .iter()
                    .any(|u| (u.0 - p.0).abs() < 8.0 && (u.1 - p.1).abs() < 8.0)
            })
        });
        out
    }

    /// Ports of a node for the grid router: point, direction of travel, cost.
    fn ports(&self, e: usize, start: bool) -> Vec<Port> {
        let ed = &self.edges[e];
        let k = if start { ed.s_node } else { ed.t_node };
        let Some(k) = k else {
            return Vec::new();
        };
        let n = &self.nodes[k];
        let [x, y, w, h] = n.r;
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let cur = if start {
            ed.pts[0]
        } else {
            ed.pts[ed.pts.len() - 1]
        };
        // keep the point of the current route on its side (spread ports)
        let pick = |side_pt: Pt, on_side: bool| if on_side { cur } else { side_pt };
        let on_left = (cur.0 - x).abs() < 0.6 && cur.1 > y && cur.1 < y + h;
        let on_right = (cur.0 - (x + w)).abs() < 0.6 && cur.1 > y && cur.1 < y + h;
        let on_top = (cur.1 - y).abs() < 0.6 && cur.0 > x && cur.0 < x + w;
        let on_bottom = (cur.1 - (y + h)).abs() < 0.6 && cur.0 > x && cur.0 < x + w;
        let gw = is_gw(&n.tag);
        let mut v = Vec::new();
        if ed.kind == Kind::Msg {
            let xs: Vec<f64> = if is_act(&n.tag) {
                vec![cx, cx - 20.0, cx + 20.0, cx - 35.0, cx + 35.0]
            } else {
                vec![cx]
            };
            for (k2, px) in xs.iter().enumerate() {
                let c = i64::try_from(k2).unwrap_or(0) * 200;
                if start {
                    v.push(Port::new((*px, y + h), DOWN, c));
                    v.push(Port::new((*px, y), UP, c));
                } else {
                    v.push(Port::new((*px, y), DOWN, c));
                    v.push(Port::new((*px, y + h), UP, c));
                }
            }
            if is_evt(&n.tag) {
                if start {
                    v.push(Port::new((x + w, cy), RIGHT, 3000));
                    v.push(Port::new((x, cy), LEFT, 3000));
                } else {
                    v.push(Port::new((x + w, cy), LEFT, 3000));
                    v.push(Port::new((x, cy), RIGHT, 3000));
                }
            }
            // a message flow does not share a port with another flow of the node
            let used: Vec<Pt> = self
                .edges
                .iter()
                .enumerate()
                .filter(|(f, _)| *f != e)
                .flat_map(|(_, fe)| [fe.pts[0], fe.pts[fe.pts.len() - 1]])
                .filter(|p| {
                    p.0 >= x - 1.0 && p.0 <= x + w + 1.0 && p.1 >= y - 1.0 && p.1 <= y + h + 1.0
                })
                .collect();
            v.retain(|pt| {
                !used
                    .iter()
                    .any(|u| (u.0 - pt.p.0).abs() < 8.0 && (u.1 - pt.p.1).abs() < 8.0)
            });
            return v;
        }
        if start && ed.host.is_some() {
            v.push(Port::new((cx, y + h), DOWN, 0));
            return v;
        }
        let side_cost = if gw { 0 } else { 800 };
        if start {
            v.push(Port::new(pick((x + w, cy), on_right), RIGHT, 0));
            v.push(Port::new(pick((cx, y + h), on_bottom), DOWN, side_cost));
            v.push(Port::new(pick((cx, y), on_top), UP, side_cost));
        } else {
            v.push(Port::new(pick((x, cy), on_left), RIGHT, 0));
            v.push(Port::new(pick((cx, y), on_top), DOWN, side_cost));
            v.push(Port::new(pick((cx, y + h), on_bottom), UP, side_cost));
        }
        v
    }

    fn grid_routes(&self, e: usize) -> Vec<Vec<Pt>> {
        let ed = &self.edges[e];
        let mut starts = self.ports(e, true);
        let mut ends = self.ports(e, false);
        let mut reverse = false;
        let mut line_end: Option<(f64, f64, f64, u8)> = None;
        if ed.kind == Kind::Msg {
            // a black-box pool: any point of its near border
            let node_pool = match (ed.s_node, ed.t_pool, ed.t_node, ed.s_pool) {
                (Some(s), Some(p), _, _) => Some((s, p, false)),
                (_, _, Some(t), Some(p)) => Some((t, p, true)),
                _ => None,
            };
            if let Some((k, p, rev)) = node_pool {
                let n = self.nodes[k].r;
                let pr = self.pools[p].1;
                if rev {
                    // route from the node to the pool, then reverse
                    let saved = ends;
                    ends = Vec::new();
                    starts = saved
                        .into_iter()
                        .map(|mut pt| {
                            pt.dir = opposite(pt.dir);
                            pt
                        })
                        .collect();
                    reverse = true;
                }
                let below = pr[1] >= n[1] + n[3];
                let edge_y = if below { pr[1] } else { pr[1] + pr[3] };
                let x0 = (pr[0] + 40.0).max(n[0] - 150.0);
                let x1 = (pr[0] + pr[2] - 10.0).min(n[0] + n[2] + 150.0);
                if x0 > x1 {
                    return Vec::new();
                }
                line_end = Some((edge_y, x0, x1, if below { DOWN } else { UP }));
                starts.retain(|pt| {
                    pt.dir == if below { DOWN } else { UP } || pt.dir == LEFT || pt.dir == RIGHT
                });
            } else if ed.s_node.is_none() || ed.t_node.is_none() {
                return Vec::new();
            }
        }
        if starts.is_empty() || (ends.is_empty() && line_end.is_none()) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let span = {
            let mut pts: Vec<Pt> = starts.iter().map(|p| p.p).collect();
            pts.extend(ends.iter().map(|p| p.p));
            if let Some((y, x0, x1, _)) = line_end {
                pts.push((x0, y));
                pts.push((x1, y));
            }
            bbox_of(&pts)
        };
        for margin in [160.0, f64::INFINITY] {
            let win = if margin.is_finite() {
                let w = [
                    span[0] - margin,
                    span[1] - margin,
                    span[2] + 2.0 * margin,
                    span[3] + 2.0 * margin,
                ];
                intersect(w, ed.area)
            } else {
                ed.area
            };
            let grid = Grid::new(self, e, &starts, &ends, line_end, win);
            let std_s: Vec<Port> = starts.iter().filter(|p| p.cost == 0).copied().collect();
            let std_e: Vec<Port> = ends.iter().filter(|p| p.cost == 0).copied().collect();
            let vert = |v: &[Port]| -> Vec<Port> {
                v.iter()
                    .filter(|p| p.dir == UP || p.dir == DOWN)
                    .copied()
                    .collect()
            };
            let (ver_s, ver_e) = (vert(&starts), vert(&ends));
            let variants: [(&[Port], &[Port], i64); 3] = [
                (&starts, &ends, C_BEND),
                (&std_s, &std_e, 3 * C_BEND),
                (&ver_s, &ver_e, C_BEND),
            ];
            for (s, t, bend) in variants {
                if s.is_empty() || (t.is_empty() && line_end.is_none()) {
                    continue;
                }
                if let Some(mut p) = grid.shortest(s, t, line_end, bend) {
                    if reverse {
                        p.reverse();
                    }
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    // -------------------------------------------------------------- labels

    /// Move labels to free positions.
    fn place_labels(&mut self) {
        for _ in 0..2 {
            let mut changed = false;
            for l in 0..self.labels.len() {
                changed |= self.improve_label(l);
            }
            if !changed {
                break;
            }
        }
    }

    fn improve_label(&mut self, l: usize) -> bool {
        let cur_di = self.labels[l].di;
        let mut cur = [0; N];
        self.label_terms(l, self.label_box(l), None, &mut cur);
        let cur_box = self.label_box(l);
        let cur_pref = self.label_pref(l, cur_di)
            + self.label_border(cur_box)
            + self.label_foreign(l, cur_box);
        if cur[LBL] + cur[LBL_EDGE] == 0 && cur_pref <= 0.0 {
            return false;
        }
        let mut best: Option<(f64, [i64; 4], Vector, usize, usize, f64)> = None;
        let saved = (
            self.labels[l].lines,
            self.labels[l].chars,
            self.labels[l].wrap_w,
        );
        for (wrap_w, cands) in self.label_candidates(l) {
            let lines = wrap(&self.labels[l].text, wrap_w, FONT);
            let chars = lines.iter().map(|x| x.chars().count()).max().unwrap_or(0);
            self.labels[l].lines = lines.len().max(1);
            self.labels[l].chars = chars;
            self.labels[l].wrap_w = wrap_w;
            for (x0, top, pref) in cands {
                let di = self.labels[l].at(x0, top);
                if di == cur_di {
                    continue;
                }
                let tb = self.labels[l].text_box(di);
                let free = self.headers.iter().all(|r| !rects_overlap(tb, *r))
                    && tb[0] >= self.drawing[0]
                    && tb[0] + tb[2] <= self.drawing[0] + self.drawing[2];
                if !free {
                    continue;
                }
                let pref = pref + self.label_border(tb) + self.label_foreign(l, tb);
                let mut v = [0; N];
                self.label_terms(l, tb, None, &mut v);
                let mut ok = true;
                let mut score = pref - cur_pref;
                for k in [LBL, LBL_EDGE] {
                    let d = v[k] - cur[k];
                    if d > 0 && self.total[k] + d > self.budget[k] {
                        ok = false;
                    }
                    #[allow(clippy::cast_precision_loss)]
                    {
                        score += WEIGHT[k] * d as f64;
                    }
                }
                if !ok || score > -0.5 {
                    continue;
                }
                if best.as_ref().is_none_or(|b| score < b.0) {
                    best = Some((score, di, v, self.labels[l].lines, chars, wrap_w));
                }
            }
        }
        (
            self.labels[l].lines,
            self.labels[l].chars,
            self.labels[l].wrap_w,
        ) = saved;
        let Some((_, di, v, lines, chars, wrap_w)) = best else {
            return false;
        };
        for k in [LBL, LBL_EDGE] {
            self.total[k] += v[k] - cur[k];
        }
        let lb = &mut self.labels[l];
        lb.di = di;
        lb.lines = lines;
        lb.chars = chars;
        lb.wrap_w = wrap_w;
        self.label_grid.update(l, self.label_box(l));
        true
    }

    /// A label of a message flow among elements it does not belong to, where it would read
    /// as theirs: inside a pool or lane that holds neither end of the flow, or close to a
    /// shape other than the ends, to another flow, or to another label.
    fn label_foreign(&self, l: usize, tb: R) -> f64 {
        let Some(e) = self.labels[l].edge else {
            return 0.0;
        };
        let ed = &self.edges[e];
        if ed.kind != Kind::Msg {
            return 0.0;
        }
        let (cx, cy) = (tb[0] + tb[2] / 2.0, tb[1] + tb[3] / 2.0);
        let mut c = 0.0;
        if ed
            .foreign
            .iter()
            .any(|r| cx > r[0] && cx < r[0] + r[2] && cy > r[1] && cy < r[1] + r[3])
        {
            c += S_FOREIGN;
        }
        let near = [
            tb[0] - LABEL_NEAR,
            tb[1] - LABEL_NEAR,
            tb[2] + 2.0 * LABEL_NEAR,
            tb[3] + 2.0 * LABEL_NEAR,
        ];
        let ends = [ed.s_node, ed.t_node];
        for k in self.node_grid.near(near) {
            let n = &self.nodes[k];
            if !n.big_sub && !ends.contains(&Some(k)) && rects_overlap(near, n.r) {
                c += 2.0;
            }
        }
        for f in self.edge_grid.near(near) {
            if f != e && poly_in_rect(&self.edges[f].pts, near) {
                c += 2.0;
            }
        }
        for m in self.label_grid.near(near) {
            let own = self.labels[m].node.is_some_and(|k| ends.contains(&Some(k)));
            if m != l && !own && rects_overlap(near, self.label_box(m)) {
                c += 2.0;
            }
        }
        c
    }

    /// A label lying across the border of a lane.
    fn label_border(&self, tb: R) -> f64 {
        let crossed = self.borders_h.iter().any(|(y, x0, x1)| {
            *y > tb[1] + 1.0 && *y < tb[1] + tb[3] - 1.0 && tb[0] < *x1 && tb[0] + tb[2] > *x0
        });
        // the border of an expanded sub-process
        let sub = self.node_grid.near(tb).into_iter().any(|k| {
            let n = &self.nodes[k];
            n.big_sub
                && rects_overlap(shrink(tb, 1.0), n.r)
                && !contains(n.r, tb)
                && !contains(tb, n.r)
        });
        (if crossed { 6.0 } else { 0.0 }) + if sub { 8.0 } else { 0.0 }
    }

    /// Preference of a position (0 for the classic one, more for positions further away).
    fn label_pref(&self, l: usize, di: [i64; 4]) -> f64 {
        let lb = &self.labels[l];
        let classic = match (lb.node, lb.edge) {
            (Some(k), _) => {
                let r = self.nodes[k].r;
                let (w, _) = lb.size();
                lb.at(r[0] + r[2] / 2.0 - w / 2.0, r[1] + r[3] + 5.5)
            }
            (_, Some(e)) => classic_flow_label(lb, &self.edges[e].pts, lb.italic),
            _ => di,
        };
        let d = (to_f(di[0] + di[2] / 2) - to_f(classic[0] + classic[2] / 2)).abs()
            + (to_f(di[1]) - to_f(classic[1])).abs();
        // A message flow label between the pools, outside all of them, reads as the
        // flow's wherever it is along the flow.
        if lb.edge.is_some_and(|e| self.edges[e].kind == Kind::Msg) {
            let (cx, cy) = (
                to_f(di[0]) + to_f(di[2]) / 2.0,
                to_f(di[1]) + to_f(di[3]) / 2.0,
            );
            let in_pool = self
                .pools
                .iter()
                .any(|(_, r)| cx > r[0] && cx < r[0] + r[2] && cy > r[1] && cy < r[1] + r[3]);
            if !in_pool {
                return (d / 10.0).min(1.0);
            }
        }
        d / 10.0
    }

    /// Candidate positions (top-left of the text box) per wrapping width.
    fn label_candidates(&self, l: usize) -> Vec<(f64, Vec<Spot>)> {
        let lb = &self.labels[l];
        let mut widths = vec![lb.wrap_w];
        let others: &[f64] = if lb.node.is_some() {
            &[70.0, 160.0]
        } else {
            &[60.0]
        };
        for &w in others {
            if wrap(&lb.text, w, FONT) != wrap(&lb.text, lb.wrap_w, FONT) {
                widths.push(w);
            }
        }
        let splits = |w: f64| {
            let words: usize = wrap(&lb.text, w, FONT)
                .iter()
                .map(|x| x.split_whitespace().count())
                .sum();
            words > lb.text.split_whitespace().count()
        };
        let cur_splits = splits(lb.wrap_w);
        let mut out = Vec::new();
        for ww in widths {
            let lines = wrap(&lb.text, ww, FONT);
            let chars = lines.iter().map(|x| x.chars().count()).max().unwrap_or(0);
            let w = fl(chars) * 5.5;
            let h = (fl(lines.len().max(1)) - 1.0) * 12.0 + 10.0;
            let extra = if (ww - lb.wrap_w).abs() < 0.5 {
                0.0
            } else if splits(ww) && !cur_splits {
                // Breaking a word that fits now is a last resort against overlaps.
                SPLIT_WORD
            } else {
                2.0
            };
            let mut c: Vec<Spot> = Vec::new();
            if let Some(k) = lb.node {
                let [x, y, nw, nh] = self.nodes[k].r;
                let cx = x + nw / 2.0;
                let cy = y + nh / 2.0;
                c.push((cx - w / 2.0, y + nh + 5.5, extra));
                c.push((cx + 6.0, y + nh + 3.0, extra + 1.0));
                c.push((cx - 6.0 - w, y + nh + 3.0, extra + 1.0));
                c.push((x + nw + 4.0, cy - h / 2.0, extra + 2.0));
                c.push((x - 4.0 - w, cy - h / 2.0, extra + 2.0));
                c.push((cx - w / 2.0, y - 4.0 - h, extra + 2.5));
                c.push((cx + 6.0, y - 3.0 - h, extra + 3.0));
                c.push((cx - 6.0 - w, y - 3.0 - h, extra + 3.0));
                c.push((x + nw + 4.0, y + nh + 2.0, extra + 3.0));
                c.push((x - 4.0 - w, y + nh + 2.0, extra + 3.0));
            } else if let Some(e) = lb.edge {
                let pts = &self.edges[e].pts;
                let mut walked = 0.0;
                for (k, s) in pts.windows(2).enumerate() {
                    let (a, b) = (s[0], s[1]);
                    let len = (b.0 - a.0).abs() + (b.1 - a.1).abs();
                    let base = extra + walked / 60.0 + fl(k) * 0.5;
                    if (a.1 - b.1).abs() < 0.5 && len >= 20.0 {
                        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                        for (t, p) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)] {
                            let lx = (x0 + 8.0) + t * (x1 - x0 - 16.0 - w).max(0.0);
                            c.push((lx, a.1 - 3.0 - h, base + p));
                            c.push((lx, a.1 + 4.0, base + p + 0.5));
                        }
                    } else if (a.0 - b.0).abs() < 0.5 && len >= 20.0 {
                        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                        for (t, p) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)] {
                            let ty = (y0 + 6.0) + t * (y1 - y0 - 12.0 - h).max(0.0);
                            c.push((a.0 + 6.0, ty, base + p));
                            c.push((a.0 - 6.0 - w, ty, base + p + 0.5));
                        }
                    }
                    walked += len;
                }
            }
            out.push((ww, c));
        }
        out
    }
}

/// Candidate position of a label: left and top of the text, preference (lower is better).
type Spot = (f64, f64, f64);

/// Bounds of the label of a flow at the position of the classic drawing.
fn classic_flow_label(l: &Label, pts: &[Pt], msg: bool) -> [i64; 4] {
    let (ax, ay) = if msg {
        msg_label_anchor(pts)
    } else {
        flow_label_anchor(pts)
    };
    l.at(ax, ay - 7.5)
}

fn intersect(a: R, b: R) -> R {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// Two message flows running side by side closer than 10 units.
fn close_parallel(a: &[Pt], b: &[Pt]) -> bool {
    for s in a.windows(2) {
        for t in b.windows(2) {
            let sv = (s[0].0 - s[1].0).abs() < 0.5;
            let tv = (t[0].0 - t[1].0).abs() < 0.5;
            if sv && tv {
                let d = (s[0].0 - t[0].0).abs();
                let lo = s[0].1.min(s[1].1).max(t[0].1.min(t[1].1));
                let hi = s[0].1.max(s[1].1).min(t[0].1.max(t[1].1));
                if d < 10.0 && hi - lo > 20.0 {
                    return true;
                }
            }
            let sh = (s[0].1 - s[1].1).abs() < 0.5;
            let th = (t[0].1 - t[1].1).abs() < 0.5;
            if sh && th {
                let d = (s[0].1 - t[0].1).abs();
                let lo = s[0].0.min(s[1].0).max(t[0].0.min(t[1].0));
                let hi = s[0].0.max(s[1].0).min(t[0].0.max(t[1].0));
                if d < 10.0 && hi - lo > 20.0 {
                    return true;
                }
            }
        }
    }
    false
}

// ------------------------------------------------------------------ grid router

const RIGHT: u8 = 0;
const LEFT: u8 = 1;
const DOWN: u8 = 2;
const UP: u8 = 3;

fn opposite(d: u8) -> u8 {
    match d {
        RIGHT => LEFT,
        LEFT => RIGHT,
        DOWN => UP,
        _ => DOWN,
    }
}

#[derive(Clone, Copy)]
struct Port {
    p: Pt,
    dir: u8,
    cost: i64,
}

impl Port {
    fn new(p: Pt, dir: u8, cost: i64) -> Port {
        Port {
            p: (p.0.round(), p.1.round()),
            dir,
            cost,
        }
    }
}

/// Orthogonal grid over the free channels between shapes, with the cost of every move.
struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// cost of the move from (i, j) to (i + 1, j); `None` when blocked
    h: Vec<Option<i64>>,
    /// cost of the move from (i, j) to (i, j + 1)
    v: Vec<Option<i64>>,
    blocked: Vec<bool>,
}

fn uniq(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    v
}

impl Grid {
    #[allow(clippy::too_many_lines)]
    fn new(
        sc: &Scene,
        e: usize,
        starts: &[Port],
        ends: &[Port],
        line_end: Option<(f64, f64, f64, u8)>,
        win: R,
    ) -> Grid {
        let ed = &sc.edges[e];
        let (wx0, wy0, wx1, wy1) = (win[0], win[1], win[0] + win[2], win[1] + win[3]);
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for p in starts.iter().chain(ends.iter()) {
            xs.push(p.p.0);
            ys.push(p.p.1);
        }
        // the shapes near the window: the ends of the flow are left from their border, other
        // shapes keep a clearance; the containers of the ends are no obstacles
        let zone = [wx0 - 20.0, wy0 - 20.0, win[2] + 40.0, win[3] + 40.0];
        let near: Vec<R> = sc
            .node_grid
            .near(grow(zone, CLEARANCE))
            .into_iter()
            .filter(|k| !ed.containers.contains(k))
            .map(|k| {
                let r = sc.nodes[k].r;
                if Some(k) == ed.s_node || Some(k) == ed.t_node {
                    shrink(r, 3.0)
                } else {
                    shrink(r, -CLEARANCE)
                }
            })
            .filter(|o| rects_overlap(*o, zone))
            .collect();
        let mut ox = vec![wx0, wx1];
        let mut oy = vec![wy0, wy1];
        for o in &near {
            ox.push(o[0] - 3.0);
            ox.push(o[0] + o[2] + 3.0);
            oy.push(o[1] - 3.0);
            oy.push(o[1] + o[3] + 3.0);
            ys.push((o[1] - 15.0).round());
            ys.push((o[1] + o[3] + 15.0).round());
        }
        // middles of the free channels between shapes
        for (src, dst) in [(&mut ox, &mut xs), (&mut oy, &mut ys)] {
            let s = uniq(std::mem::take(src));
            for w in s.windows(2) {
                if w[1] - w[0] > 16.0 {
                    dst.push(((w[0] + w[1]) / 2.0).round());
                }
            }
        }
        if let Some((y, x0, x1, _)) = line_end {
            ys.push(y);
            // a few positions along the border for the message flow
            let mut x = x0;
            while x <= x1 {
                xs.push(x.round());
                x += 40.0;
            }
        }
        // channels in the gaps between pools
        for (_, r) in &sc.pools {
            for d in [12.0, 20.0, 28.0] {
                ys.push(r[1] + r[3] + d);
                ys.push(r[1] - d);
            }
        }
        let xs: Vec<f64> = uniq(xs)
            .into_iter()
            .filter(|x| *x >= wx0 - 0.5 && *x <= wx1 + 0.5)
            .collect();
        let ys: Vec<f64> = uniq(ys)
            .into_iter()
            .filter(|y| *y >= wy0 - 0.5 && *y <= wy1 + 0.5)
            .collect();
        let (nx, ny) = (xs.len(), ys.len());
        let mut blocked = vec![false; nx * ny];
        for (j, y) in ys.iter().enumerate() {
            for o in near.iter().filter(|o| *y > o[1] && *y < o[1] + o[3]) {
                let i0 = xs.partition_point(|x| *x <= o[0]);
                for i in i0..nx {
                    if xs[i] >= o[0] + o[2] {
                        break;
                    }
                    blocked[j * nx + i] = true;
                }
            }
        }
        // other edges, split by orientation: only those near the window, since a move on
        // the grid meets nothing farther than 6 units from its line
        let reach = grow(win, 10.0);
        let mut oh: Vec<(f64, f64, f64, bool)> = Vec::new();
        let mut ov: Vec<(f64, f64, f64, bool)> = Vec::new();
        let mut odiag: Vec<(Pt, Pt)> = Vec::new();
        for f in sc.edge_grid.near(reach) {
            let fe = &sc.edges[f];
            if f == e {
                continue;
            }
            let related = fe.src == ed.src || fe.tgt == ed.tgt;
            for s in fe.pts.windows(2) {
                let (a, b) = (s[0], s[1]);
                if (a.1 - b.1).abs() < 0.5 {
                    oh.push((a.1, a.0.min(b.0), a.0.max(b.0), related));
                } else if (a.0 - b.0).abs() < 0.5 {
                    ov.push((a.0, a.1.min(b.1), a.1.max(b.1), related));
                } else {
                    odiag.push((a, b));
                }
            }
        }
        let labels: Vec<R> = sc
            .label_grid
            .near(reach)
            .into_iter()
            .filter(|l| Some(*l) != ed.label)
            .map(|l| shrink(sc.label_box(l), 1.0))
            .collect();
        let msg = ed.kind == Kind::Msg;
        let mut h = vec![None; nx * ny];
        let mut v = vec![None; nx * ny];
        // moves along one grid line: `fixed` is the coordinate of the line, the moves run
        // between consecutive values of `run`; items are filtered once per line
        for horiz in [true, false] {
            let (lines, run) = if horiz { (&ys, &xs) } else { (&xs, &ys) };
            for (a, &fixed) in lines.iter().enumerate() {
                // (lo, hi) of a rectangle along the line, if the line passes through it
                let across = |r: &R| -> Option<(f64, f64)> {
                    let (c0, c1, l0, l1) = if horiz {
                        (r[1], r[1] + r[3], r[0], r[0] + r[2])
                    } else {
                        (r[0], r[0] + r[2], r[1], r[1] + r[3])
                    };
                    (fixed > c0 && fixed < c1).then_some((l0, l1))
                };
                let obs: Vec<(f64, f64)> = near.iter().filter_map(across).collect();
                let labs: Vec<(f64, f64)> = labels.iter().filter_map(across).collect();
                let regs: Vec<(f64, f64)> = ed.foreign.iter().filter_map(across).collect();
                let (perp_all, para_all) = if horiz { (&ov, &oh) } else { (&oh, &ov) };
                let perp: Vec<f64> = perp_all
                    .iter()
                    .filter(|(_, p0, p1, _)| fixed > p0 + 0.5 && fixed < p1 - 0.5)
                    .map(|(pc, _, _, _)| *pc)
                    .collect();
                let para: Vec<&(f64, f64, f64, bool)> = para_all
                    .iter()
                    .filter(|(pc, _, _, _)| (pc - fixed).abs() < 6.0)
                    .collect();
                let borders = if horiz { &sc.borders_h } else { &sc.borders_v };
                let bord: Vec<(f64, f64)> = borders
                    .iter()
                    .filter(|(bc, _, _)| (bc - fixed).abs() <= 2.0)
                    .map(|(_, b0, b1)| (*b0, *b1))
                    .collect();
                for b in 0..run.len().saturating_sub(1) {
                    let (lo, hi) = (run[b], run[b + 1]);
                    let (ia, ib) = if horiz {
                        (a * nx + b, a * nx + b + 1)
                    } else {
                        (b * nx + a, (b + 1) * nx + a)
                    };
                    if blocked[ia] || blocked[ib] {
                        continue;
                    }
                    if obs.iter().any(|(o0, o1)| hi > *o0 && lo < *o1) {
                        continue;
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    let mut c = ((hi - lo) * to_f(C_LEN)) as i64;
                    for pc in &perp {
                        if *pc > lo + 0.5 && *pc < hi - 0.5 {
                            c += C_CROSS;
                        }
                    }
                    for (pc, p0, p1, related) in &para {
                        let shared = hi.min(*p1) - lo.max(*p0);
                        if shared <= 0.5 {
                            continue;
                        }
                        if (pc - fixed).abs() < 1.0 {
                            if !related {
                                c += C_OVL;
                            }
                        } else if msg {
                            c += C_LABEL / 2;
                        }
                    }
                    if !odiag.is_empty() {
                        let (pa, pb) = if horiz {
                            ((lo, fixed), (hi, fixed))
                        } else {
                            ((fixed, lo), (fixed, hi))
                        };
                        for (p, q) in &odiag {
                            if seg_cross(pa, pb, *p, *q) {
                                c += C_CROSS;
                            }
                        }
                    }
                    for (l0, l1) in &labs {
                        if hi > *l0 && lo < *l1 {
                            c += C_LABEL;
                        }
                    }
                    for (r0, r1) in &regs {
                        // crossing the border of a region (entering or leaving it)
                        if hi > *r0 && lo < *r1 && !(lo > *r0 && hi < *r1) {
                            c += C_REGION;
                        }
                    }
                    for (b0, b1) in &bord {
                        if hi.min(*b1) - lo.max(*b0) > 0.5 {
                            c += C_BORDER;
                        }
                    }
                    if horiz {
                        h[ia] = Some(c);
                    } else {
                        v[ia] = Some(c);
                    }
                }
            }
        }
        Grid {
            xs,
            ys,
            h,
            v,
            blocked,
        }
    }

    fn index(&self, p: Pt) -> Option<usize> {
        let i = self.xs.iter().position(|x| (x - p.0).abs() < 0.5)?;
        let j = self.ys.iter().position(|y| (y - p.1).abs() < 0.5)?;
        Some(j * self.xs.len() + i)
    }

    /// Cheapest path from one of the start ports to one of the end ports (or to the border
    /// line `line_end`: y, x range, direction of arrival); A* with the Manhattan distance to
    /// the nearest goal as the estimate.
    #[allow(clippy::too_many_lines)]
    fn shortest(
        &self,
        starts: &[Port],
        ends: &[Port],
        line_end: Option<(f64, f64, f64, u8)>,
        bend: i64,
    ) -> Option<Vec<Pt>> {
        let nx = self.xs.len();
        let n = nx * self.ys.len();
        let mut goal = vec![i64::MAX; n * 4];
        let mut goal_pts: Vec<Pt> = Vec::new();
        let mut any = false;
        for p in ends {
            if let Some(ix) = self.index(p.p) {
                let s = ix * 4 + usize::from(p.dir);
                goal[s] = goal[s].min(p.cost);
                goal_pts.push(p.p);
                any = true;
            }
        }
        let mut line: Option<(f64, f64, f64)> = None;
        if let Some((y, x0, x1, d)) = line_end
            && let Some(j) = self.ys.iter().position(|yy| (yy - y).abs() < 0.5)
        {
            for (i, x) in self.xs.iter().enumerate() {
                if *x >= x0 - 0.5 && *x <= x1 + 0.5 {
                    goal[(j * nx + i) * 4 + usize::from(d)] = 0;
                    any = true;
                }
            }
            line = Some((y, x0, x1));
        }
        if !any {
            return None;
        }
        let estimate = |ix: usize| -> i64 {
            let (x, y) = (self.xs[ix % nx], self.ys[ix / nx]);
            let mut best = f64::INFINITY;
            for g in &goal_pts {
                best = best.min((g.0 - x).abs() + (g.1 - y).abs());
            }
            if let Some((ly, x0, x1)) = line {
                let dx = if x < x0 {
                    x0 - x
                } else if x > x1 {
                    x - x1
                } else {
                    0.0
                };
                best = best.min(dx + (ly - y).abs());
            }
            #[allow(clippy::cast_possible_truncation)]
            let h = (best * to_f(C_LEN)) as i64;
            h
        };
        let mut dist = vec![i64::MAX; n * 4];
        let mut prev = vec![usize::MAX; n * 4];
        let mut first = vec![false; n * 4];
        let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
        for p in starts {
            let Some(ix) = self.index(p.p) else {
                continue;
            };
            let s = ix * 4 + usize::from(p.dir);
            first[s] = true;
            if p.cost < dist[s] {
                dist[s] = p.cost;
                heap.push(Reverse((p.cost + estimate(ix), s)));
            }
        }
        let mut found: Option<(i64, usize)> = None;
        while let Some(Reverse((key, s))) = heap.pop() {
            if let Some((fd, _)) = found
                && key >= fd
            {
                break;
            }
            let d = dist[s];
            let ix = s / 4;
            if key > d + estimate(ix) {
                continue;
            }
            if goal[s] != i64::MAX {
                let total = d + goal[s];
                if found.is_none_or(|(fd, _)| total < fd) {
                    found = Some((total, s));
                }
            }
            let dir = u8::try_from(s % 4).unwrap_or(0);
            let (i, j) = (ix % nx, ix / nx);
            for nd in [RIGHT, LEFT, DOWN, UP] {
                // no U-turns; the first move leaves the port straight
                if nd == opposite(dir) || (first[s] && prev[s] == usize::MAX && nd != dir) {
                    continue;
                }
                let step = match nd {
                    RIGHT if i + 1 < nx => self.h[ix].map(|c| (ix + 1, c)),
                    LEFT if i > 0 => self.h[ix - 1].map(|c| (ix - 1, c)),
                    DOWN if j + 1 < self.ys.len() => self.v[ix].map(|c| (ix + nx, c)),
                    UP if j > 0 => self.v[ix - nx].map(|c| (ix - nx, c)),
                    _ => None,
                };
                let Some((nix, c)) = step else {
                    continue;
                };
                if self.blocked[nix] {
                    continue;
                }
                let turn = if nd == dir { 0 } else { bend };
                let ns = nix * 4 + usize::from(nd);
                let ndist = d + c + turn;
                if ndist < dist[ns] {
                    dist[ns] = ndist;
                    prev[ns] = s;
                    heap.push(Reverse((ndist + estimate(nix), ns)));
                }
            }
        }
        let (_, mut s) = found?;
        let mut pts = Vec::new();
        loop {
            let ix = s / 4;
            pts.push((self.xs[ix % nx], self.ys[ix / nx]));
            if prev[s] == usize::MAX {
                break;
            }
            s = prev[s];
        }
        pts.reverse();
        Some(simplify(&pts))
    }
}
