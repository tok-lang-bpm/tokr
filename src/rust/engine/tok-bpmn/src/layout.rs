//! Automatic layout (`layout()` of `bpmn.py`): computes positions of all shapes
//! and waypoints of all edges, and writes them as BPMNDI into the document.

use std::collections::{HashMap, HashSet};

use crate::model::{
    Model, is_act, is_conv, is_data, is_data_shape, is_flownode, is_gw, is_subp, m,
};
use crate::pyfmt::round_str;
use crate::refine;
use crate::svg::wrap;
use crate::xml::{BPMNDI_NS, DC_NS, DI_NS, Id, qn};

const COL_GAP: i64 = 50;
const ROW_GAP: i64 = 50;
const PAD_L: i64 = 30;
const PAD_R: i64 = 30;
const PAD_T: i64 = 30;
const PAD_B: i64 = 30;
const SUB_PAD_T: i64 = 40;
const MIN_LANE_H: i64 = 130;
const POOL_GAP: i64 = 40;
const BB_H: i64 = 70;
/// Gap below a pool where conversation nodes stand, and the size of a conversation node.
const CONV_GAP: i64 = 110;
const CONV_W: i64 = 40;
const CONV_H: i64 = 35;
const CONV_HALF: f64 = 20.0;

/// Python `None` for a missing id or reference, as a dictionary key.
const NONE: &str = "\u{0}";

/// Key of an optional id: the id, or the `None` sentinel.
pub(crate) fn key(v: Option<&str>) -> String {
    v.unwrap_or(NONE).to_string()
}

type Rect = [i64; 4];
pub(crate) type Pt = (f64, f64);

/// Insertion-ordered map (Python `dict` semantics for the parts that need it).
#[derive(Debug, Clone, Default)]
pub(crate) struct OMap<V> {
    pub keys: Vec<String>,
    pub map: HashMap<String, V>,
}

impl<V: Clone> OMap<V> {
    pub fn insert(&mut self, k: &str, v: V) {
        if !self.map.contains_key(k) {
            self.keys.push(k.to_string());
        }
        self.map.insert(k.to_string(), v);
    }
    pub fn get(&self, k: &str) -> Option<&V> {
        self.map.get(k)
    }
    pub fn contains(&self, k: &str) -> bool {
        self.map.contains_key(k)
    }
    pub fn items(&self) -> Vec<(String, V)> {
        self.keys
            .iter()
            .map(|k| (k.clone(), self.map[k].clone()))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
struct LBox {
    pos: OMap<Rect>,
    children: HashMap<String, LBox>,
    w: i64,
    h: i64,
    lanes: Vec<(Option<String>, i64, i64)>,
    back: HashSet<String>,
    /// the barycentre sweeps changed the order of some nodes
    reordered: bool,
}

fn even10(v: i64) -> i64 {
    (v + 9).div_euclid(10) * 10
}

fn node_size(md: &Model, e: Id) -> (i64, i64) {
    if let Some(s) = md.id_of(e).and_then(|i| md.hints.size.get(i)) {
        return *s;
    }
    let t = md.tag(e);
    if is_act(&t) {
        return (100, 80);
    }
    if crate::model::is_evt(&t) {
        return (36, 36);
    }
    if is_gw(&t) {
        return (50, 50);
    }
    if t == "dataObjectReference" || t == "dataInput" || t == "dataOutput" {
        return (36, 50);
    }
    if t == "dataStoreReference" {
        return (50, 50);
    }
    if t == "textAnnotation" {
        let txt = md.doc.findtext(e, &m("text")).unwrap_or_default();
        let lines = wrap(txt.trim(), 120.0, 11.0).len().max(1);
        let lines = i64::try_from(lines).unwrap_or(i64::MAX / 32);
        return (130, (even10(lines * 14 + 10)).max(30));
    }
    (100, 80)
}

fn strip_text(md: &Model, e: Id) -> String {
    md.doc.text(e).unwrap_or("").trim().to_string()
}

#[allow(clippy::too_many_lines)]
fn layout_container(
    md: &Model,
    cont: Id,
    lane_list: Option<&[Id]>,
    top_pad: i64,
    reorder: bool,
) -> LBox {
    let mut bx = LBox::default();
    let d = &md.doc;
    let kids: Vec<Id> = d.children(cont).to_vec();
    let nodes: Vec<Id> = kids
        .iter()
        .copied()
        .filter(|c| {
            let t = md.tag(*c);
            is_flownode(&t) && t != "boundaryEvent" && !md.is_event_sub(*c)
        })
        .collect();
    let evsubs: Vec<Id> = kids
        .iter()
        .copied()
        .filter(|c| md.is_event_sub(*c))
        .collect();
    // the data inputs and outputs of a process are placed after its data objects, in the
    // columns next to the activities that read and write them
    let mut extras: Vec<Id> = kids
        .iter()
        .copied()
        .filter(|c| {
            let t = md.tag(*c);
            is_data(&t) || t == "textAnnotation"
        })
        .collect();
    extras.extend(md.process_io(cont));
    let kid = |e: Id| key(md.id_of(e));
    let mut sizes: HashMap<String, (i64, i64)> = HashMap::new();
    for &n in nodes.iter().chain(evsubs.iter()) {
        let nid = kid(n);
        if is_subp(&md.tag(n)) && md.has_flow_children(n) {
            let top = md.hints.pad.get(&nid).map_or(SUB_PAD_T, |p| p.0);
            let cb = layout_container(md, n, None, top, reorder);
            bx.back.extend(cb.back.iter().cloned());
            bx.reordered |= cb.reordered;
            sizes.insert(nid.clone(), (cb.w, cb.h));
            bx.children.insert(nid, cb);
        } else {
            sizes.insert(nid, node_size(md, n));
        }
    }
    for &x in &extras {
        sizes.insert(kid(x), node_size(md, x));
    }

    let ids: Vec<String> = nodes.iter().map(|n| kid(*n)).collect();
    let idset: HashSet<String> = ids.iter().cloned().collect();
    let mut host_of: HashMap<String, String> = HashMap::new();
    for &n in &nodes {
        for &b in md.boundaries_of(md.id_of(n)) {
            host_of.insert(kid(b), kid(n));
        }
    }
    let mut succ: HashMap<String, Vec<(String, bool)>> =
        ids.iter().map(|i| (i.clone(), Vec::new())).collect();
    let mut pred: HashMap<String, Vec<(String, bool)>> =
        ids.iter().map(|i| (i.clone(), Vec::new())).collect();
    let mut flows_of: HashMap<(String, String), Vec<String>> = HashMap::new();
    for &f in &md.seq {
        if md.parent.get(&f).copied() != Some(cont) {
            continue;
        }
        let s = key(md.attr(f, "sourceRef"));
        let t = key(md.attr(f, "targetRef"));
        let via_b = host_of.contains_key(&s);
        let s2 = host_of.get(&s).cloned().unwrap_or(s);
        if idset.contains(&s2) && idset.contains(&t) {
            if let Some(v) = succ.get_mut(&s2) {
                v.push((t.clone(), via_b));
            }
            if let Some(v) = pred.get_mut(&t) {
                v.push((s2.clone(), via_b));
            }
            flows_of.entry((s2, t)).or_default().push(kid(f));
        }
    }

    // reversed edges (loops): DFS
    let starts: Vec<String> = ids
        .iter()
        .filter(|i| {
            md.get(Some(i.as_str()))
                .is_some_and(|e| md.tag(e) == "startEvent")
        })
        .cloned()
        .collect();
    let mut roots: Vec<String> = starts.clone();
    roots.extend(
        ids.iter()
            .filter(|i| pred[*i].is_empty() && !starts.contains(i))
            .cloned(),
    );
    roots.extend(ids.iter().cloned());
    let mut color: HashMap<String, u8> = ids.iter().map(|i| (i.clone(), 0)).collect();
    let mut disc: HashMap<String, usize> = HashMap::new();
    let mut back: HashSet<(String, String)> = HashSet::new();
    let mut cnt = 0usize;
    for rt in &roots {
        if color[rt] != 0 {
            continue;
        }
        color.insert(rt.clone(), 1);
        disc.insert(rt.clone(), cnt);
        cnt += 1;
        let mut stack: Vec<(String, usize)> = vec![(rt.clone(), 0)];
        while let Some((u, pos)) = stack.last().cloned() {
            let nxt = succ[&u].get(pos).cloned();
            let Some((v, _)) = nxt else {
                color.insert(u, 2);
                stack.pop();
                continue;
            };
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
            match color[&v] {
                1 => {
                    back.insert((u.clone(), v.clone()));
                }
                0 => {
                    color.insert(v.clone(), 1);
                    disc.insert(v.clone(), cnt);
                    cnt += 1;
                    stack.push((v, 0));
                }
                _ => {}
            }
        }
    }
    for (u, v) in &back {
        if let Some(fl) = flows_of.get(&(u.clone(), v.clone())) {
            bx.back.extend(fl.iter().cloned());
        }
    }

    // layers
    let mut layer: HashMap<String, i64> = ids.iter().map(|i| (i.clone(), 0)).collect();
    let mut indeg: HashMap<String, i64> = ids.iter().map(|i| (i.clone(), 0)).collect();
    let mut fwd: HashMap<String, Vec<String>> =
        ids.iter().map(|i| (i.clone(), Vec::new())).collect();
    for u in &ids {
        for (v, _) in succ[u].clone() {
            if back.contains(&(u.clone(), v.clone())) {
                continue;
            }
            if let Some(x) = fwd.get_mut(u) {
                x.push(v.clone());
            }
            if let Some(x) = indeg.get_mut(&v) {
                *x += 1;
            }
        }
    }
    let mut queue: Vec<String> = ids.iter().filter(|i| indeg[*i] == 0).cloned().collect();
    queue.sort_by_key(|i| disc.get(i).copied().unwrap_or(0));
    let mut topo: Vec<String> = Vec::new();
    while !queue.is_empty() {
        let u = queue.remove(0);
        topo.push(u.clone());
        for v in fwd[&u].clone() {
            let lu = layer[&u];
            if let Some(x) = layer.get_mut(&v) {
                *x = (*x).max(lu + 1);
            }
            let mut dv = 1;
            if let Some(x) = indeg.get_mut(&v) {
                *x -= 1;
                dv = *x;
            }
            if dv == 0 {
                queue.push(v);
            }
        }
    }
    for i in &ids {
        if !topo.contains(i) {
            topo.push(i.clone());
        }
    }

    // lanes
    let nl = lane_list.map_or(1, <[Id]>::len);
    let mut lane_idx: HashMap<String, usize> = HashMap::new();
    if let Some(ll) = lane_list {
        for (k, ln) in ll.iter().enumerate() {
            for rf in d.findall(*ln, &m("flowNodeRef")) {
                lane_idx.entry(strip_text(md, rf)).or_insert(k);
            }
        }
    }
    for i in &topo {
        if !lane_idx.contains_key(i) {
            let ps: Vec<&String> = pred[i]
                .iter()
                .map(|(p, _)| p)
                .filter(|p| lane_idx.contains_key(*p))
                .collect();
            let v = ps.first().map_or(0, |p| lane_idx[*p]);
            lane_idx.insert(i.clone(), v);
        }
    }

    // rows
    let mut occ: HashSet<(usize, i64, i64)> = HashSet::new();
    let mut row: HashMap<String, i64> = HashMap::new();
    let place = |i: &str,
                 lay: i64,
                 pref: i64,
                 occ: &mut HashSet<(usize, i64, i64)>,
                 row: &mut HashMap<String, i64>,
                 lane_idx: &HashMap<String, usize>| {
        let li = lane_idx[i];
        let mut rr = pref.max(0);
        while occ.contains(&(li, lay, rr)) {
            rr += 1;
        }
        occ.insert((li, lay, rr));
        row.insert(i.to_string(), rr);
    };
    let mut order_ids = ids.clone();
    order_ids.sort_by_key(|x| (layer[x], disc.get(x).copied().unwrap_or(1_000_000)));
    for i in &order_ids {
        let mut pref = 0;
        for (p, vb) in &pred[i] {
            if back.contains(&(p.clone(), i.clone())) {
                continue;
            }
            if lane_idx.get(p) == lane_idx.get(i) && row.contains_key(p) {
                pref = row[p] + i64::from(*vb);
                break;
            }
        }
        place(i, layer[i], pref, &mut occ, &mut row, &lane_idx);
    }
    if reorder {
        bx.reordered |=
            barycentre_sweeps(&ids, &layer, &lane_idx, &pred, &succ, &back, nl, &mut row);
    }

    // data objects and annotations
    let mut assoc_target: OMap<Vec<String>> = OMap::default();
    let link = |xid: &str, nid: &str, at: &mut OMap<Vec<String>>| {
        let mut lst = at.get(xid).cloned().unwrap_or_default();
        if !lst.iter().any(|x| x == nid) {
            lst.push(nid.to_string());
        }
        at.insert(xid, lst);
    };
    let evsub_ids: HashSet<String> = evsubs.iter().map(|e| kid(*e)).collect();
    let lift = |e0: Id| -> String {
        let mut e = e0;
        loop {
            let eid = kid(e);
            if idset.contains(&eid) || evsub_ids.contains(&eid) {
                return eid;
            }
            if let Some(h) = host_of.get(&eid) {
                return h.clone();
            }
            match md.parent.get(&e).copied() {
                None => return eid,
                Some(p) if p == cont => return eid,
                Some(p) => e = p,
            }
        }
    };
    for n in d.iter(cont) {
        if n == cont || !is_flownode(&md.tag(n)) {
            continue;
        }
        let nid = lift(n);
        for da in d.findall(n, &m("dataInputAssociation")) {
            for s in d.findall(da, &m("sourceRef")) {
                link(&strip_text(md, s), &nid, &mut assoc_target);
            }
        }
        for da in d.findall(n, &m("dataOutputAssociation")) {
            if let Some(t) = d.find(da, &m("targetRef")) {
                link(&strip_text(md, t), &nid, &mut assoc_target);
            }
        }
    }
    for &a in &md.assoc {
        let s = key(md.attr(a, "sourceRef"));
        let t = key(md.attr(a, "targetRef"));
        if idset.contains(&t) {
            link(&s, &t, &mut assoc_target);
        }
        if idset.contains(&s) {
            link(&t, &s, &mut assoc_target);
        }
    }
    let last_layer = ids.iter().map(|i| layer[i]).max().unwrap_or(0);
    for &x in &extras {
        let xid = kid(x);
        let targets = assoc_target.get(&xid).cloned().unwrap_or_default();
        let hosts: Vec<String> = targets
            .iter()
            .filter(|h| idset.contains(*h))
            .cloned()
            .collect();
        if hosts.is_empty() {
            let evh: Vec<String> = targets
                .iter()
                .filter(|h| evsub_ids.contains(*h))
                .cloned()
                .collect();
            if let (Some(first), Some(ll)) = (evh.first(), lane_list) {
                for (kk, ln) in ll.iter().enumerate() {
                    if d.findall(*ln, &m("flowNodeRef"))
                        .iter()
                        .any(|r_| strip_text(md, *r_) == *first)
                    {
                        lane_idx.insert(xid.clone(), kk);
                    }
                }
                layer.insert(xid.clone(), 0);
                lane_idx.entry(xid.clone()).or_insert(nl - 1);
                place(&xid, 0, 0, &mut occ, &mut row, &lane_idx);
                continue;
            }
        }
        let mut host: Option<String> = None;
        for h in &hosts {
            if !occ.contains(&(lane_idx[h], layer[h], row[h] + 1)) {
                host = Some(h.clone());
                break;
            }
        }
        if host.is_none() {
            host = hosts.first().cloned();
        }

        // a process input stands in the column before its reader, an output in the column
        // after its writer (when there is one), so that they do not pile up under it
        let xt = md.tag(x);
        let pref = match host.filter(|h| idset.contains(h)) {
            Some(h) => {
                let hl = layer[&h];
                let l = match xt.as_str() {
                    "dataInput" => (hl - 1).max(0),
                    "dataOutput" => (hl + 1).min(last_layer),
                    _ => hl,
                };
                layer.insert(xid.clone(), l);
                lane_idx.insert(xid.clone(), lane_idx[&h]);
                row[&h] + 1
            }
            None => {
                let l = if xt == "dataOutput" { last_layer } else { 0 };
                layer.insert(xid.clone(), l);
                lane_idx.insert(xid.clone(), 0);
                0
            }
        };
        place(&xid, layer[&xid], pref, &mut occ, &mut row, &lane_idx);
    }

    // event sub-processes: own row at the bottom of their lane
    let mut ev_rows: OMap<i64> = OMap::default();
    for &ev in &evsubs {
        let eid = kid(ev);
        let mut k = nl - 1;
        if let Some(ll) = lane_list {
            for (kk, ln) in ll.iter().enumerate() {
                if d.findall(*ln, &m("flowNodeRef"))
                    .iter()
                    .any(|r_| strip_text(md, *r_) == eid)
                {
                    k = kk;
                }
            }
        }
        lane_idx.insert(eid.clone(), k);
        let mut used: Vec<i64> = occ
            .iter()
            .filter(|(ll, _, _)| *ll == k)
            .map(|(_, _, rr)| *rr)
            .collect();
        for (e2, rr) in ev_rows.items() {
            if lane_idx[&e2] == k {
                used.push(rr);
            }
        }
        let v = used.iter().max().map_or(0, |x| x + 1);
        ev_rows.insert(&eid, v);
    }

    // columns
    let mut all_ids: Vec<String> = ids.clone();
    all_ids.extend(extras.iter().map(|x| kid(*x)));
    let nlay = all_ids.iter().map(|i| layer[i]).max().map_or(1, |x| x + 1);
    let nlay_u = usize::try_from(nlay).unwrap_or(1);
    let mut colw: Vec<i64> = vec![36; nlay_u];
    for i in &all_ids {
        let l = usize::try_from(layer[i]).unwrap_or(0);
        colw[l] = colw[l].max(sizes[i].0);
    }
    let mut colx: Vec<i64> = Vec::new();
    let mut x = PAD_L;
    for w in &colw {
        colx.push(x);
        x += w + COL_GAP;
    }
    let mut content_w = x - COL_GAP + PAD_R;
    for &ev in &evsubs {
        content_w = content_w.max(PAD_L + sizes[&kid(ev)].0 + PAD_R);
    }
    content_w = even10(content_w);

    // row and lane heights
    let mut rowh: OMap<i64> = OMap::default();
    for i in &all_ids {
        let k = format!("{}:{}", lane_idx[i], row[i]);
        let v = rowh.get(&k).copied().unwrap_or(0).max(sizes[i].1);
        rowh.insert(&k, v);
    }
    for (eid, rr) in ev_rows.items() {
        let k = format!("{}:{}", lane_idx[&eid], rr);
        let v = rowh.get(&k).copied().unwrap_or(0).max(sizes[&eid].1);
        rowh.insert(&k, v);
    }
    let rowh_get = |k: usize, rr: i64| rowh.get(&format!("{k}:{rr}")).copied().unwrap_or(0);
    let mut lane_geo: Vec<(i64, i64, HashMap<i64, i64>)> = Vec::new();
    let mut y = 0;
    for k in 0..nl {
        let mut rows: Vec<i64> = rowh
            .keys
            .iter()
            .filter_map(|s| {
                let (a, b) = s.split_once(':')?;
                (a.parse::<usize>().ok()? == k)
                    .then(|| b.parse::<i64>().ok())
                    .flatten()
            })
            .collect();
        rows.sort_unstable();
        let top = y + top_pad;
        let mut rtop: HashMap<i64, i64> = HashMap::new();
        let mut yy = top;
        let row_gap = md.hints.row_gap.unwrap_or(ROW_GAP);
        for rr in &rows {
            rtop.insert(*rr, yy);
            yy += rowh_get(k, *rr) + row_gap;
        }
        let bottom = if rows.is_empty() { top } else { yy - row_gap };
        let min_h = if lane_list.is_some() { MIN_LANE_H } else { 0 };
        let pad_b = md.hints.pad.get(&kid(cont)).map_or(PAD_B, |p| p.1);
        let lane_h = even10((bottom - y + pad_b).max(min_h));
        lane_geo.push((y, lane_h, rtop));
        y += lane_h;
    }
    bx.h = y;
    bx.w = content_w;
    for i in &all_ids {
        let k = lane_idx[i];
        let rr = row[i];
        let rtop = &lane_geo[k].2;
        let (w, h) = sizes[i];
        let l = usize::try_from(layer[i]).unwrap_or(0);
        let cx = colx[l] + colw[l].div_euclid(2);
        let cy = rtop[&rr] + rowh_get(k, rr).div_euclid(2);
        bx.pos
            .insert(i, [cx - w.div_euclid(2), cy - h.div_euclid(2), w, h]);
    }
    for (eid, rr) in ev_rows.items() {
        let k = lane_idx[&eid];
        let rtop = &lane_geo[k].2;
        let (w, h) = sizes[&eid];
        bx.pos.insert(&eid, [PAD_L, rtop[&rr], w, h]);
    }
    bx.lanes = match lane_list {
        Some(ll) => (0..nl)
            .map(|k| {
                (
                    md.id_of(ll[k]).map(str::to_string),
                    lane_geo[k].0,
                    lane_geo[k].1,
                )
            })
            .collect(),
        None => vec![(None, 0, bx.h)],
    };
    bx
}

/// Order the nodes of every column of a lane by the mean position of their neighbours
/// (predecessors in sweeps to the right, successors in sweeps to the left). The rows a column
/// occupies stay the same, so do the heights of rows and lanes; only their assignment to the
/// nodes changes. Ties keep the classic order. Returns whether any node moved.
#[allow(clippy::too_many_arguments)]
fn barycentre_sweeps(
    ids: &[String],
    layer: &HashMap<String, i64>,
    lane_idx: &HashMap<String, usize>,
    pred: &HashMap<String, Vec<(String, bool)>>,
    succ: &HashMap<String, Vec<(String, bool)>>,
    back: &HashSet<(String, String)>,
    nl: usize,
    row: &mut HashMap<String, i64>,
) -> bool {
    #[allow(clippy::cast_precision_loss)]
    let pos = |i: &str, row: &HashMap<String, i64>| lane_idx[i] as f64 * 1000.0 + row[i] as f64;
    let nlay = ids.iter().map(|i| layer[i]).max().map_or(0, |x| x + 1);
    // the nodes of each layer and lane, in the order of `ids` (layers and lanes do not
    // change during the sweeps)
    let mut groups: HashMap<(i64, usize), Vec<&String>> = HashMap::new();
    for i in ids {
        groups.entry((layer[i], lane_idx[i])).or_default().push(i);
    }
    let mut moved = false;
    for sweep in 0..4 {
        let forward = sweep % 2 == 0;
        let layers: Vec<i64> = if forward {
            (0..nlay).collect()
        } else {
            (0..nlay).rev().collect()
        };
        for l in layers {
            for k in 0..nl {
                let Some(group) = groups.get(&(l, k)) else {
                    continue;
                };
                if group.len() < 2 {
                    continue;
                }
                let mut keyed: Vec<(f64, i64, String)> = group
                    .iter()
                    .map(|v| {
                        let nb: Vec<f64> = if forward {
                            pred[*v]
                                .iter()
                                .filter(|(p, _)| !back.contains(&(p.clone(), (*v).clone())))
                                .map(|(p, vb)| pos(p, row) + f64::from(u8::from(*vb)))
                                .collect()
                        } else {
                            succ[*v]
                                .iter()
                                .filter(|(t, _)| !back.contains(&((*v).clone(), t.clone())))
                                .map(|(t, _)| pos(t, row))
                                .collect()
                        };
                        #[allow(clippy::cast_precision_loss)]
                        let b = if nb.is_empty() {
                            pos(v, row)
                        } else {
                            nb.iter().sum::<f64>() / nb.len() as f64
                        };
                        (b, row[*v], (*v).clone())
                    })
                    .collect();
                let mut rows_used: Vec<i64> = keyed.iter().map(|x| x.1).collect();
                rows_used.sort_unstable();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                for (r, (_, old, v)) in rows_used.iter().zip(keyed.iter()) {
                    if r != old {
                        moved = true;
                    }
                    row.insert(v.clone(), *r);
                }
            }
        }
    }
    moved
}

fn place_box(bx: &LBox, x0: i64, y0: i64, out: &mut OMap<Rect>) {
    for (i, r) in bx.pos.items() {
        out.insert(&i, [x0 + r[0], y0 + r[1], r[2], r[3]]);
        if let Some(cb) = bx.children.get(&i) {
            place_box(cb, x0 + r[0], y0 + r[1], out);
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn rf(r: Rect) -> [f64; 4] {
    [r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64]
}

fn clip(b: Rect, px: f64, py: f64) -> Pt {
    let [x, y, w, h] = rf(b);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (dx, dy) = (px - cx, py - cy);
    if dx == 0.0 && dy == 0.0 {
        return (cx, cy);
    }
    let sx = if dx == 0.0 { 1e9 } else { (w / 2.0) / dx.abs() };
    let sy = if dy == 0.0 { 1e9 } else { (h / 2.0) / dy.abs() };
    let s = sx.min(sy);
    (cx + dx * s, cy + dy * s)
}

fn seg_hits(p1: Pt, p2: Pt, b: [f64; 4]) -> bool {
    let [x, y, w, h] = b;
    let (x0, y0, x1, y1) = (x + 3.0, y + 3.0, x + w - 3.0, y + h - 3.0);
    let ((ax, ay), (bx, by)) = (p1, p2);
    if (ax - bx).abs() < 0.5 {
        return x0 < ax && ax < x1 && ay.min(by).max(y0) < ay.max(by).min(y1);
    }
    if (ay - by).abs() < 0.5 {
        return y0 < ay && ay < y1 && ax.min(bx).max(x0) < ax.max(bx).min(x1);
    }
    false
}

fn score(pts: &[Pt], obstacles: &[[f64; 4]]) -> usize {
    let mut hits = 0;
    for k in 0..pts.len().saturating_sub(1) {
        for ob in obstacles {
            if seg_hits(pts[k], pts[k + 1], *ob) {
                hits += 1;
            }
        }
    }
    hits * 100 + pts.len()
}

/// Side of a cell of the grid of [`Obstacles`].
const OBSTACLE_CELL: f64 = 200.0;

/// The shapes that routes avoid (flow nodes, data and annotations, in the order of the
/// placement), with the element a boundary event is attached to, and a grid of their boxes
/// by columns and by cells. A route counts the shapes it runs through, so the grid gives
/// the same counts as a scan of all shapes, without one per candidate route.
struct Obstacles {
    items: Vec<(String, Option<String>, [f64; 4])>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    columns: HashMap<i64, Vec<usize>>,
    /// shapes too large (or too far out) for the grid: every query returns them
    everywhere: Vec<usize>,
}

/// A shape covering more cells than this is kept in `Obstacles::everywhere`.
const OBSTACLE_MAX_CELLS: i64 = 4096;

/// The cells from `a` to `b` (widened by a margin for rounding at cell borders), or
/// `None` for values out of range.
fn cell_range(a: f64, b: f64) -> Option<(i64, i64)> {
    let c = |v: f64| (v / OBSTACLE_CELL).floor();
    let (lo, hi) = (c(a.min(b) - 2.0), c(a.max(b) + 2.0));
    #[allow(clippy::cast_possible_truncation)]
    (lo.is_finite() && hi.is_finite() && lo.abs() < 1e12 && hi.abs() < 1e12 && hi - lo < 1e6)
        .then_some((lo as i64, hi as i64))
}

impl Obstacles {
    fn new(md: &Model, out: &OMap<Rect>) -> Obstacles {
        let mut o = Obstacles {
            items: Vec::new(),
            cells: HashMap::new(),
            columns: HashMap::new(),
            everywhere: Vec::new(),
        };
        for i in &out.keys {
            let Some(e) = md.get(Some(i)) else {
                continue;
            };
            let t = md.tag(e);
            if !(is_flownode(&t) || is_data_shape(&t) || t == "textAnnotation") {
                continue;
            }
            let attached = (t == "boundaryEvent").then(|| key(md.attr(e, "attachedToRef")));
            let b = rf(out.map[i]);
            let k = o.items.len();
            match (cell_range(b[0], b[0] + b[2]), cell_range(b[1], b[1] + b[3])) {
                (Some((x0, x1)), Some((y0, y1)))
                    if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) <= OBSTACLE_MAX_CELLS =>
                {
                    for x in x0..=x1 {
                        o.columns.entry(x).or_default().push(k);
                        for y in y0..=y1 {
                            o.cells.entry((x, y)).or_default().push(k);
                        }
                    }
                }
                _ => o.everywhere.push(k),
            }
            o.items.push((i.clone(), attached, b));
        }
        o
    }

    /// The shapes in the cells of `x0..x1` × `y0..y1` (all rows when `ys` is `None`), in
    /// the order of the placement; every shape whose box meets the area is among them.
    fn near(&self, xs: (f64, f64), ys: Option<(f64, f64)>) -> Vec<usize> {
        let (Some((x0, x1)), y) = (cell_range(xs.0, xs.1), ys.map(|(a, b)| cell_range(a, b)))
        else {
            return (0..self.items.len()).collect();
        };
        let cells = match y {
            Some(Some((y0, y1))) => (x1 - x0 + 1).saturating_mul(y1 - y0 + 1),
            _ => x1 - x0 + 1,
        };
        if cells > OBSTACLE_MAX_CELLS {
            return (0..self.items.len()).collect();
        }
        let mut out = self.everywhere.clone();
        match y {
            Some(None) => return (0..self.items.len()).collect(),
            Some(Some((y0, y1))) => {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        if let Some(v) = self.cells.get(&(x, y)) {
                            out.extend_from_slice(v);
                        }
                    }
                }
            }
            None => {
                for x in x0..=x1 {
                    if let Some(v) = self.columns.get(&x) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The obstacles of one route: the shapes except its ends (`skip`), the boundary events
/// attached to them and the containers of the route.
struct RouteObstacles<'a> {
    all: &'a Obstacles,
    skip: HashSet<String>,
    anc: HashSet<String>,
}

impl RouteObstacles<'_> {
    fn new<'a>(
        md: &Model,
        all: &'a Obstacles,
        skip: HashSet<String>,
        container: Option<Id>,
    ) -> RouteObstacles<'a> {
        let mut anc: HashSet<String> = HashSet::new();
        let mut c = container;
        while let Some(x) = c {
            anc.insert(key(md.id_of(x)));
            c = md.container(x);
        }
        RouteObstacles { all, skip, anc }
    }

    fn counts(&self, k: usize) -> bool {
        let (id, attached, _) = &self.all.items[k];
        !self.skip.contains(id)
            && !self.anc.contains(id)
            && !attached.as_ref().is_some_and(|a| self.skip.contains(a))
    }

    /// The boxes of the obstacles meeting the area (all rows when `ys` is `None`).
    fn boxes(&self, xs: (f64, f64), ys: Option<(f64, f64)>) -> Vec<[f64; 4]> {
        self.all
            .near(xs, ys)
            .into_iter()
            .filter(|&k| self.counts(k))
            .map(|k| self.all.items[k].2)
            .collect()
    }

    /// `score` of a route against these obstacles.
    fn score(&self, pts: &[Pt]) -> usize {
        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in pts {
            (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
        }
        score(pts, &self.boxes((x0, x1), Some((y0, y1))))
    }

    /// `min(cands, key=score)`: first candidate with the lowest score, and all candidates.
    fn best(&self, cands: Vec<Vec<Pt>>) -> (Vec<Pt>, Vec<Vec<Pt>>) {
        let mut best: Option<(usize, &Vec<Pt>)> = None;
        for c in &cands {
            let s = self.score(c);
            if best.as_ref().is_none_or(|(bs, _)| s < *bs) {
                best = Some((s, c));
            }
        }
        let b = best.map(|(_, c)| c.clone()).unwrap_or_default();
        (b, cands)
    }
}

#[allow(clippy::too_many_lines)]
fn route_seq(
    md: &Model,
    f: Id,
    out: &OMap<Rect>,
    shapes: &Obstacles,
    back_ids: &HashSet<String>,
    counter: &mut i64,
) -> (Vec<Pt>, Vec<Vec<Pt>>) {
    let s_id = key(md.attr(f, "sourceRef"));
    let t_id = key(md.attr(f, "targetRef"));
    let [sx, sy, sw, sh] = rf(out.map[&s_id]);
    let [tx, ty, tw, th] = rf(out.map[&t_id]);
    let se = md.get(Some(&s_id));
    let te = md.get(Some(&t_id));
    let stag = se.map(|e| md.tag(e)).unwrap_or_default();
    let ttag = te.map(|e| md.tag(e)).unwrap_or_default();
    let (scx, scy) = (sx + sw / 2.0, sy + sh / 2.0);
    let (tcx, tcy) = (tx + tw / 2.0, ty + th / 2.0);
    let (sb, tb) = (sy + sh, ty + th);
    let s_split = is_gw(&stag) && md.out_of(Some(&s_id)).len() > 1;
    let t_join = is_gw(&ttag) && md.inc_of(Some(&t_id)).len() > 1;
    let mut skip: HashSet<String> = [s_id.clone(), t_id.clone()].into_iter().collect();
    if stag == "boundaryEvent" {
        skip.insert(key(se.and_then(|e| md.attr(e, "attachedToRef"))));
    }
    let obs = RouteObstacles::new(md, shapes, skip, md.parent.get(&f).copied());
    let mut cands: Vec<Vec<Pt>> = Vec::new();
    #[allow(clippy::cast_precision_loss)]
    let cnt = |c: i64| c as f64;

    if stag == "boundaryEvent" {
        if tx >= sx + sw {
            if tcy >= sb + 5.0 {
                cands.push(vec![(scx, sb), (scx, tcy), (tx, tcy)]);
            }
            let yb = sb + 20.0;
            cands.push(vec![
                (scx, sb),
                (scx, yb),
                (tx - 20.0, yb),
                (tx - 20.0, tcy),
                (tx, tcy),
            ]);
        }
        if ty > sb + 30.0 {
            cands.push(vec![
                (scx, sb),
                (scx, ty - 20.0),
                (tcx, ty - 20.0),
                (tcx, ty),
            ]);
        }
        *counter += 1;
        let yb = sb.max(tb) + 25.0 + 8.0 * cnt(counter.rem_euclid(4));
        cands.push(vec![(scx, sb), (scx, yb), (tcx, yb), (tcx, tb)]);
        return obs.best(cands);
    }

    let backward = back_ids.contains(&key(md.id_of(f))) || tx < sx + sw - 1.0;
    let half_gap = (COL_GAP / 2) as f64;
    if !backward {
        if (scy - tcy).abs() < 1.5 {
            cands.push(vec![(sx + sw, scy), (tx, tcy)]);
        }
        if s_split {
            let ey = if tcy > scy { sb } else { sy };
            cands.push(vec![(scx, ey), (scx, tcy), (tx, tcy)]);
        }
        if t_join {
            let ey = if scy < tcy { ty } else { tb };
            cands.push(vec![(sx + sw, scy), (tcx, scy), (tcx, ey)]);
        }
        let midx = tx - half_gap;
        cands.push(vec![(sx + sw, scy), (midx, scy), (midx, tcy), (tx, tcy)]);
        let midx2 = sx + sw + half_gap;
        if midx2 < tx - 5.0 {
            cands.push(vec![(sx + sw, scy), (midx2, scy), (midx2, tcy), (tx, tcy)]);
        }
        if s_split {
            let ey = if tcy > scy { sb } else { sy };
            let jy = ey + if tcy > scy { 15.0 } else { -15.0 };
            cands.push(vec![
                (scx, ey),
                (scx, jy),
                (midx, jy),
                (midx, tcy),
                (tx, tcy),
            ]);
        }
        return obs.best(cands);
    }
    // backward flow (loop)
    if ty - 20.0 > sb + 5.0 {
        cands.push(vec![
            (scx, sb),
            (scx, ty - 20.0),
            (tcx, ty - 20.0),
            (tcx, ty),
        ]);
    }
    if tb + 20.0 < sy - 5.0 {
        cands.push(vec![
            (scx, sy),
            (scx, tb + 20.0),
            (tcx, tb + 20.0),
            (tcx, tb),
        ]);
    }
    *counter += 1;
    let k = 8.0 * cnt(counter.rem_euclid(4));
    let lo: Vec<[f64; 4]> = obs
        .boxes((scx.min(tcx), scx.max(tcx)), None)
        .into_iter()
        .filter(|b| b[0] < scx.max(tcx) && b[0] + b[2] > scx.min(tcx))
        .collect();
    let yb = sb.max(tb) + 25.0 + k;
    cands.push(vec![(scx, sb), (scx, yb), (tcx, yb), (tcx, tb)]);
    let ya = sy.min(ty) - 25.0 - k;
    cands.push(vec![(scx, sy), (scx, ya), (tcx, ya), (tcx, ty)]);
    if !lo.is_empty() {
        let mut m_ = sb.max(tb);
        for b in &lo {
            if b[1] >= sy.min(ty) - 5.0 {
                m_ = m_.max(b[1] + b[3]);
            }
        }
        let yb2 = m_ + 25.0 + k;
        cands.push(vec![(scx, sb), (scx, yb2), (tcx, yb2), (tcx, tb)]);
    }
    obs.best(cands)
}

fn event_side_point(b: Rect, right: bool) -> Pt {
    let [x, y, w, h] = rf(b);
    let dy = 8.0;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    if (w - h).abs() < f64::EPSILON && w <= 40.0 {
        let r = w / 2.0;
        let dx = (r * r - dy * dy).powf(0.5);
        return if right {
            (cx + dx, cy + dy)
        } else {
            (cx - dx, cy + dy)
        };
    }
    if right {
        (x + w, cy + dy)
    } else {
        (x, cy + dy)
    }
}

fn route_msg_node_pool(
    md: &Model,
    node_id: &str,
    pool: Rect,
    out: &OMap<Rect>,
    shapes: &Obstacles,
    from_node: bool,
    k: i64,
) -> (Vec<Pt>, Vec<Vec<Pt>>) {
    let n = out.map[node_id];
    let [nx, ny, nw, nh] = rf(n);
    let ncx = nx + nw / 2.0;
    let [_px, py, _pw, ph] = rf(pool);
    let below = py >= ny + nh;
    let edge_y = if below { py } else { py + ph };
    let skip: HashSet<String> = std::iter::once(node_id.to_string()).collect();
    let obs = RouteObstacles::new(md, shapes, skip, None);
    let mut cands: Vec<Vec<Pt>> = Vec::new();
    if below {
        cands.push(vec![(ncx, ny + nh), (ncx, edge_y)]);
    } else {
        cands.push(vec![(ncx, ny), (ncx, edge_y)]);
    }
    #[allow(clippy::cast_precision_loss)]
    let kf = k as f64;
    for right in [true, false] {
        let p0 = event_side_point(n, right);
        let chx = if right {
            nx + nw + 14.0 + 6.0 * kf
        } else {
            nx - 14.0 - 6.0 * kf
        };
        cands.push(vec![p0, (chx, p0.1), (chx, edge_y)]);
    }
    let (mut b, mut all) = obs.best(cands);
    if !from_node {
        b.reverse();
        for c in &mut all {
            c.reverse();
        }
    }
    (b, all)
}

fn spread_ports(md: &Model, wp: &mut OMap<Vec<Pt>>, out: &OMap<Rect>) {
    let mut sides: OMap<Vec<(String, usize)>> = OMap::default();
    for (fid, pts) in wp.items() {
        let Some(e) = md.get(Some(&fid)) else {
            continue;
        };
        let t = md.tag(e);
        if (t != "sequenceFlow" && t != "messageFlow") || pts.len() < 2 {
            continue;
        }
        for (idx, nid) in [
            (0usize, key(md.attr(e, "sourceRef"))),
            (pts.len() - 1, key(md.attr(e, "targetRef"))),
        ] {
            let ne = md.get(Some(&nid));
            if !out.contains(&nid) || ne.is_none_or(|x| !is_act(&md.tag(x))) {
                continue;
            }
            let [x, y, w, h] = rf(out.map[&nid]);
            let (px, py) = pts[idx];
            let side = if (px - (x + w / 2.0)).abs() < 0.6 && (py - y).abs() < 0.6 {
                Some("top")
            } else if (px - (x + w / 2.0)).abs() < 0.6 && (py - (y + h)).abs() < 0.6 {
                Some("bottom")
            } else if (px - x).abs() < 0.6 && (py - (y + h / 2.0)).abs() < 0.6 {
                Some("left")
            } else if (px - (x + w)).abs() < 0.6 && (py - (y + h / 2.0)).abs() < 0.6 {
                Some("right")
            } else {
                None
            };
            if let Some(side) = side {
                let k = format!("{nid}\u{1}{side}");
                let mut lst = sides.get(&k).cloned().unwrap_or_default();
                lst.push((fid.clone(), idx));
                sides.insert(&k, lst);
            }
        }
    }
    for (k, mut lst) in sides.items() {
        if lst.len() < 2 {
            continue;
        }
        let (nid, side) = k.split_once('\u{1}').unwrap_or((&k, ""));
        let [_x, _y, w, h] = rf(out.map[nid]);
        let horiz = side == "top" || side == "bottom";
        let far = |item: &(String, usize)| -> f64 {
            let pts = &wp.map[&item.0];
            let q = if item.1 == 0 {
                pts[pts.len() - 1]
            } else {
                pts[0]
            };
            if horiz { q.0 } else { q.1 }
        };
        let mut keyed: Vec<(f64, (String, usize))> =
            lst.drain(..).map(|it| (far(&it), it)).collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        #[allow(clippy::cast_precision_loss)]
        let n = keyed.len() as f64;
        let span = if horiz { w } else { h };
        let step = 16.0_f64.min((span - 20.0) / (n - 1.0).max(1.0));
        for (k, (_, (fid, idx))) in keyed.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let d = (k as f64 - (n - 1.0) / 2.0) * step;
            let Some(pts) = wp.map.get_mut(&fid) else {
                continue;
            };
            let nb = if idx == 0 { 1 } else { pts.len() - 2 };
            if horiz {
                let old = pts[idx].0;
                pts[idx] = (old + d, pts[idx].1);
                if pts.len() == 2 || (pts[nb].0 - old).abs() < 0.6 {
                    pts[nb] = (pts[nb].0 + d, pts[nb].1);
                }
            } else {
                let old = pts[idx].1;
                pts[idx] = (pts[idx].0, old + d);
                if pts.len() == 2 || (pts[nb].1 - old).abs() < 0.6 {
                    pts[nb] = (pts[nb].0, pts[nb].1 + d);
                }
            }
        }
    }
}

/// A complete layout: shapes, lanes, routes, label bounds and its quality.
struct Plan {
    out: OMap<Rect>,
    lanes_out: OMap<Rect>,
    wp: OMap<Vec<Pt>>,
    labels: HashMap<String, [i64; 4]>,
    plane_ref: Option<String>,
    quality: refine::Quality,
}

/// Area of the drawing (shapes and waypoints), in square units.
fn drawing_area(out: &OMap<Rect>, lanes_out: &OMap<Rect>, wp: &OMap<Vec<Pt>>) -> f64 {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for r in out.map.values().chain(lanes_out.map.values()) {
        let [x, y, w, h] = rf(*r);
        b = [b[0].min(x), b[1].min(y), b[2].max(x + w), b[3].max(y + h)];
    }
    for pts in wp.map.values() {
        for p in pts {
            let (x, y) = (p.0.round_ties_even(), p.1.round_ties_even());
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
    }
    if b[0] > b[2] {
        return 0.0;
    }
    (b[2] - b[0]) * (b[3] - b[1])
}

/// Generate the complete BPMNDI of the model (replacing an existing diagram).
/// Returns the number of shapes, like `bpmn.py layout()`.
///
/// The classic layout is refined (see `refine.rs`); a second layout with the nodes of every
/// column ordered by barycentre sweeps is used instead when it is better and no quality
/// measure (nor the area) is worse than in the classic layout.
pub fn layout_model(md: &mut Model) -> usize {
    let Some(classic) = plan(md, false, None) else {
        return 0;
    };
    let chosen = match plan(md, true, Some(&classic.quality)) {
        Some(p) if p.quality.better_than(&classic.quality) => p,
        _ => classic,
    };
    let n = write_di(md, chosen);
    crate::choreo::layout_choreographies(md);
    n
}

/// Lay the model out; with `reorder`, the nodes are ordered by barycentre sweeps and the
/// refinement keeps every measure within the starting values of `limit` (the classic
/// layout). `None` when the sweeps change nothing.
#[allow(clippy::too_many_lines)]
fn plan(md: &mut Model, reorder: bool, limit: Option<&refine::Quality>) -> Option<Plan> {
    let mut reordered = false;
    let mut out: OMap<Rect> = OMap::default();
    let mut back: HashSet<String> = HashSet::new();
    let mut lanes_out: OMap<Rect> = OMap::default();
    let mut pools: HashMap<String, Rect> = HashMap::new();
    let mut content_x: HashMap<String, i64> = HashMap::new();
    let plane_ref: Option<String>;
    // conversation nodes: (collaboration, centre line of the gap, width, nodes with the
    // participants they link); placed after the refinement, away from the message flows
    let mut conv_rows: Vec<ConvRow> = Vec::new();
    if let Some(&collab) = md.collabs.first() {
        plane_ref = md.id_of(collab).map(str::to_string);
        struct Entry {
            p: Id,
            lb: Option<LBox>,
            lanes_all: Vec<(Id, usize, Option<Id>)>,
            maxd: i64,
        }
        let mut entries: Vec<Entry> = Vec::new();
        for p in md.doc.findall(collab, &m("participant")) {
            let proc = md.get(md.attr(p, "processRef"));
            match proc {
                Some(pr) if md.tag(pr) == "process" && md.has_flow_children(pr) => {
                    let lanes_all = md.lanes_of(pr);
                    let leaves: Vec<Id> = lanes_all
                        .iter()
                        .filter(|(l, _, _)| md.is_leaf_lane(*l))
                        .map(|(l, _, _)| *l)
                        .collect();
                    let maxd = lanes_all
                        .iter()
                        .map(|(_, dd, _)| i64::try_from(*dd).unwrap_or(0))
                        .max()
                        .map_or(0, |x| x + 1);
                    let lb = layout_container(
                        md,
                        pr,
                        if leaves.is_empty() {
                            None
                        } else {
                            Some(&leaves)
                        },
                        PAD_T,
                        reorder,
                    );
                    reordered |= lb.reordered;
                    back.extend(lb.back.iter().cloned());
                    entries.push(Entry {
                        p,
                        lb: Some(lb),
                        lanes_all,
                        maxd,
                    });
                }
                _ => entries.push(Entry {
                    p,
                    lb: None,
                    lanes_all: Vec::new(),
                    maxd: 0,
                }),
            }
        }
        let mut w_all = 600;
        for en in &entries {
            if let Some(lb) = &en.lb {
                w_all = w_all.max(30 + en.maxd * 30 + lb.w);
            }
        }
        // a conversation node stands in the gap below the first pool it links
        let index: HashMap<String, usize> = entries
            .iter()
            .enumerate()
            .map(|(k, en)| (key(md.id_of(en.p)), k))
            .collect();
        let conv = conversation_nodes(md, collab);
        let mut gaps: Vec<(usize, Vec<LinkedNode>)> = Vec::new();
        for (node, linked) in conv {
            let Some(first) = linked.iter().filter_map(|p| index.get(p)).min().copied() else {
                continue;
            };
            match gaps.iter_mut().find(|(g, _)| *g == first) {
                Some((_, v)) => v.push((node, linked)),
                None => gaps.push((first, vec![(node, linked)])),
            }
        }
        let gap_after = |k: usize| {
            if gaps.iter().any(|(g, _)| *g == k) {
                CONV_GAP
            } else {
                POOL_GAP
            }
        };
        let mut y = 0;
        for (k, en) in entries.iter().enumerate() {
            let pid = key(md.id_of(en.p));
            let Some(lb) = &en.lb else {
                out.insert(&pid, [0, y, w_all, BB_H]);
                pools.insert(pid.clone(), [0, y, w_all, BB_H]);
                y += BB_H + gap_after(k);
                continue;
            };
            out.insert(&pid, [0, y, w_all, lb.h]);
            pools.insert(pid.clone(), [0, y, w_all, lb.h]);
            let x0 = 30 + en.maxd * 30;
            content_x.insert(pid.clone(), x0);
            place_box(lb, x0, y, &mut out);
            let band: HashMap<String, (i64, i64)> = lb
                .lanes
                .iter()
                .filter_map(|(lid, ly, lh)| {
                    lid.clone()
                        .filter(|l| !l.is_empty())
                        .map(|l| (l, (*ly, *lh)))
                })
                .collect();
            if !en.lanes_all.is_empty() {
                fn lane_range(
                    md: &Model,
                    lane: Id,
                    band: &HashMap<String, (i64, i64)>,
                    h: i64,
                ) -> (i64, i64) {
                    if md.is_leaf_lane(lane) {
                        return band.get(&key(md.id_of(lane))).copied().unwrap_or((0, h));
                    }
                    let cls = md.doc.find(lane, &m("childLaneSet"));
                    let kids = cls
                        .map(|c| md.doc.findall(c, &m("lane")))
                        .unwrap_or_default();
                    let rs: Vec<(i64, i64)> =
                        kids.iter().map(|c| lane_range(md, *c, band, h)).collect();
                    let y0 = rs.iter().map(|(a, _)| *a).min().unwrap_or(0);
                    let y1 = rs.iter().map(|(a, b)| a + b).max().unwrap_or(0);
                    (y0, y1 - y0)
                }
                for (lane, depth, _par) in &en.lanes_all {
                    let (ly, lh) = lane_range(md, *lane, &band, lb.h);
                    let dep = i64::try_from(*depth).unwrap_or(0);
                    lanes_out.insert(
                        &key(md.id_of(*lane)),
                        [30 + dep * 30, y + ly, w_all - 30 - dep * 30, lh],
                    );
                }
            }
            y += lb.h + gap_after(k);
        }
        for (g, nodes) in gaps {
            let Some(above) = pools.get(&key(md.id_of(entries[g].p))).copied() else {
                continue;
            };
            let cy = above[1] + above[3] + CONV_GAP / 2;
            let linked = nodes
                .into_iter()
                .map(|(node, linked)| {
                    let rects = linked
                        .iter()
                        .filter_map(|p| pools.get(p).map(|r| (p.clone(), *r)))
                        .collect();
                    (key(md.id_of(node)), rects)
                })
                .collect();
            conv_rows.push(ConvRow {
                collab,
                cy,
                width: w_all,
                nodes: linked,
            });
        }
    } else {
        plane_ref = md
            .processes
            .first()
            .and_then(|p| md.id_of(*p))
            .map(str::to_string);
        let mut y = 0;
        for &proc in &md.processes.clone() {
            let lb = layout_container(md, proc, None, PAD_T, reorder);
            reordered |= lb.reordered;
            back.extend(lb.back.iter().cloned());
            place_box(&lb, 0, y, &mut out);
            y += lb.h + POOL_GAP;
        }
    }

    if reorder && !reordered {
        return None;
    }

    // boundary events
    for (host, bl) in md.boundaries.clone() {
        let Some(&[hx, hy, hw, hh]) = out.get(&host) else {
            continue;
        };
        for (k, b) in bl.iter().enumerate() {
            let k = i64::try_from(k).unwrap_or(0);
            // on a choreography activity the bottom-right is the place of the envelope of the
            // returned message: its boundary events stand at the bottom-left
            let cx = if md.hints.boundary_left.contains(&host) {
                hx + 24 + 40 * k
            } else {
                (hx + hw - 30 - 44 * k).max(hx + 18)
            };
            out.insert(&key(md.id_of(*b)), [cx - 18, hy + hh - 18, 36, 36]);
        }
    }

    // edges
    let mut wp: OMap<Vec<Pt>> = OMap::default();
    let mut classic: HashMap<String, Vec<Vec<Pt>>> = HashMap::new();
    let mut counter = 0i64;
    let shapes = Obstacles::new(md, &out);
    for &f in &md.seq {
        if out.contains(&key(md.attr(f, "sourceRef")))
            && out.contains(&key(md.attr(f, "targetRef")))
        {
            let (pts, cands) = route_seq(md, f, &out, &shapes, &back, &mut counter);
            wp.insert(&key(md.id_of(f)), pts);
            classic.insert(key(md.id_of(f)), cands);
        }
    }
    let mut allbox = out.clone();
    for (k, v) in lanes_out.items() {
        allbox.insert(&k, v);
    }
    let mut mcount: HashMap<(String, String), i64> = HashMap::new();
    // message flows between two black-box pools, per unordered pair: drawn side by side
    let is_pool = |id: &str| md.get(Some(id)).is_some_and(|e| md.tag(e) == "participant");
    let pool_pair = |f: Id| {
        let a = key(md.attr(f, "sourceRef"));
        let b = key(md.attr(f, "targetRef"));
        (is_pool(&a) && is_pool(&b)).then(|| if a <= b { (a, b) } else { (b, a) })
    };
    let mut pair_total: HashMap<(String, String), i64> = HashMap::new();
    for &f in &md.msg {
        if let Some(pp) = pool_pair(f) {
            *pair_total.entry(pp).or_insert(0) += 1;
        }
    }
    let mut pair_seen: HashMap<(String, String), i64> = HashMap::new();
    for &f in &md.msg {
        let s_id = key(md.attr(f, "sourceRef"));
        let t_id = key(md.attr(f, "targetRef"));
        if !allbox.contains(&s_id) || !allbox.contains(&t_id) {
            continue;
        }
        let se = md.get(Some(&s_id));
        let te = md.get(Some(&t_id));
        let s = allbox.map[&s_id];
        let t = allbox.map[&t_id];
        let ps = md.participant_of(se);
        let pt = md.participant_of(te);
        let s_is_pool = se.is_some_and(|e| md.tag(e) == "participant");
        let t_is_pool = te.is_some_and(|e| md.tag(e) == "participant");
        let kk = (
            ps.map_or_else(|| s_id.clone(), |p| key(md.id_of(p))),
            pt.map_or_else(|| t_id.clone(), |p| key(md.id_of(p))),
        );
        let k = mcount.get(&kk).copied().unwrap_or(0);
        mcount.insert(kk, k + 1);
        let fid = key(md.id_of(f));
        if t_is_pool && !s_is_pool && out.contains(&s_id) {
            let (pts, cands) = route_msg_node_pool(md, &s_id, t, &out, &shapes, true, k);
            wp.insert(&fid, pts);
            classic.insert(fid, cands);
            continue;
        }
        if s_is_pool && !t_is_pool && out.contains(&t_id) {
            let (pts, cands) = route_msg_node_pool(md, &t_id, s, &out, &shapes, false, k);
            wp.insert(&fid, pts);
            classic.insert(fid, cands);
            continue;
        }
        let s_pool: Option<Rect> = match ps {
            Some(p) => pools.get(&key(md.id_of(p))).copied(),
            None => Some(s),
        };
        let [sx, sy, sw, sh] = rf(s);
        let [tx, ty, tw, th] = rf(t);
        let mut sxc = sx + sw / 2.0;
        let mut txc = tx + tw / 2.0;
        if s_is_pool {
            sxc = txc.max(sx + 40.0).min(sx + sw - 10.0);
        }
        if t_is_pool {
            txc = sxc.max(tx + 40.0).min(tx + tw - 10.0);
        }
        if let Some(pp) = pool_pair(f) {
            let n = pair_total[&pp];
            let j = pair_seen.get(&pp).copied().unwrap_or(0);
            pair_seen.insert(pp, j + 1);
            #[allow(clippy::cast_precision_loss)]
            let x = sxc + (j as f64 - (n - 1) as f64 / 2.0) * 30.0;
            let lo = sx.max(tx) + 40.0;
            let hi = (sx + sw).min(tx + tw) - 10.0;
            sxc = x.max(lo).min(hi);
            txc = sxc;
        }
        #[allow(clippy::cast_precision_loss)]
        let (kf, gap) = (k as f64, POOL_GAP as f64);
        let (y1, y2, gy);
        if sy + sh / 2.0 <= ty + th / 2.0 {
            y1 = sy + sh;
            y2 = ty;
            gy = match s_pool {
                Some(sp) => {
                    let sp = rf(sp);
                    sp[1] + sp[3] + gap / 2.0 - 6.0 + 6.0 * kf
                }
                None => (y1 + y2) / 2.0,
            };
        } else {
            y1 = sy;
            y2 = ty + th;
            gy = match s_pool {
                Some(sp) => rf(sp)[1] - gap / 2.0 + 6.0 - 6.0 * kf,
                None => (y1 + y2) / 2.0,
            };
        }
        if (sxc - txc).abs() < 1.0 {
            wp.insert(&fid, vec![(sxc, y1), (txc, y2)]);
        } else {
            wp.insert(&fid, vec![(sxc, y1), (sxc, gy), (txc, gy), (txc, y2)]);
        }
    }
    spread_ports(md, &mut wp, &out);
    for &a in &md.assoc {
        let s_id = key(md.attr(a, "sourceRef"));
        let t_id = key(md.attr(a, "targetRef"));
        if out.contains(&s_id) && out.contains(&t_id) {
            let s = out.map[&s_id];
            let t = out.map[&t_id];
            let [sx, sy, sw, sh] = rf(s);
            let [tx, ty, tw, th] = rf(t);
            let p1 = clip(s, tx + tw / 2.0, ty + th / 2.0);
            let p2 = clip(t, sx + sw / 2.0, sy + sh / 2.0);
            wp.insert(&key(md.id_of(a)), vec![p1, p2]);
        }
    }
    let mut n_da = 0;
    for e in md.doc.iter(md.doc.root) {
        let t = md.tag(e);
        if t != "dataInputAssociation" && t != "dataOutputAssociation" {
            continue;
        }
        let Some(owner) = md.parent.get(&e).copied() else {
            continue;
        };
        let oid = key(md.id_of(owner));
        if !out.contains(&oid) {
            continue;
        }
        let pair: Vec<(String, String)> = if t == "dataInputAssociation" {
            md.doc
                .findall(e, &m("sourceRef"))
                .iter()
                .map(|s| strip_text(md, *s))
                .filter(|sid| out.contains(sid))
                .map(|sid| (sid, oid.clone()))
                .collect()
        } else {
            let tid = md
                .doc
                .find(e, &m("targetRef"))
                .map(|tg| strip_text(md, tg))
                .unwrap_or_default();
            if out.contains(&tid) {
                vec![(oid.clone(), tid)]
            } else {
                Vec::new()
            }
        };
        if pair.is_empty() {
            continue;
        }
        if md.id_of(e).is_none_or(str::is_empty) {
            n_da += 1;
            let mut nid = format!("DataAssociation_{n_da}");
            while md.ids.contains_key(&nid) {
                n_da += 1;
                nid = format!("DataAssociation_{n_da}");
            }
            md.doc.set(e, "id", &nid);
            md.ids.insert(nid, e);
        }
        let (a, b) = pair[0].clone();
        let s = out.map[&a];
        let tb = out.map[&b];
        let [sx, sy, sw, sh] = rf(s);
        let [tx, ty, tw, th] = rf(tb);
        let p1 = clip(s, tx + tw / 2.0, ty + th / 2.0);
        let p2 = clip(tb, sx + sw / 2.0, sy + sh / 2.0);
        wp.insert(&key(md.id_of(e)), vec![p1, p2]);
    }

    // refinement: routes and labels
    let scene_input = refine::Input {
        out: &out,
        lanes: &lanes_out,
        content_x: &content_x,
        wp: &wp,
        classic: &classic,
    };
    let area0 = drawing_area(&out, &lanes_out, &wp);
    let mut scene = refine::Scene::build(md, &scene_input);
    if let Some(q) = limit {
        scene.limit(q);
    }
    scene.refine();
    for (k, fid) in scene.edge_ids.iter().enumerate() {
        wp.insert(fid, scene.route(k).to_vec());
    }
    let labels: HashMap<String, [i64; 4]> = scene
        .labels
        .iter()
        .map(|l| (l.owner.clone(), l.di))
        .collect();
    let quality = scene.quality(area0, drawing_area(&out, &lanes_out, &wp));
    for row in conv_rows {
        place_conversations(md, &row, &labels, &mut out, &mut wp);
    }
    Some(Plan {
        out,
        lanes_out,
        wp,
        labels,
        plane_ref,
        quality,
    })
}

/// Write the BPMNDI of a layout into the document; returns the number of shapes.
fn write_di(md: &mut Model, plan: Plan) -> usize {
    let Plan {
        out,
        lanes_out,
        wp,
        labels,
        plane_ref,
        ..
    } = plan;
    let root = md.doc.root;
    for dg in md.doc.findall(root, &qn(BPMNDI_NS, "BPMNDiagram")) {
        md.doc.remove_child(root, dg);
    }
    let diag = md.doc.sub_element(
        root,
        &qn(BPMNDI_NS, "BPMNDiagram"),
        &[("id", "BPMNDiagram_1")],
    );
    let plane_ref_s = plane_ref.unwrap_or_default();
    let plane = md.doc.sub_element(
        diag,
        &qn(BPMNDI_NS, "BPMNPlane"),
        &[("id", "BPMNPlane_1"), ("bpmnElement", &plane_ref_s)],
    );
    let mut shapes = out.clone();
    for (k, v) in lanes_out.items() {
        shapes.insert(&k, v);
    }
    let mut order: Vec<String> = Vec::new();
    for &p in &md.participants {
        let pid = key(md.id_of(p));
        if shapes.contains(&pid) {
            order.push(pid);
        }
    }
    for &proc in &md.processes {
        for (ln, _, _) in md.lanes_of(proc) {
            let lid = key(md.id_of(ln));
            if shapes.contains(&lid) {
                order.push(lid);
            }
        }
    }
    for e in md.doc.iter(root) {
        let i = key(md.id_of(e));
        if shapes.contains(&i) && !order.contains(&i) && md.tag(e) != "boundaryEvent" {
            order.push(i);
        }
    }
    for e in md.doc.iter_tag(root, &m("boundaryEvent")) {
        let i = key(md.id_of(e));
        if shapes.contains(&i) {
            order.push(i);
        }
    }
    for i in &order {
        let Some(e) = md.get(Some(i)) else {
            continue;
        };
        let t = md.tag(e);
        let di_id = format!("{i}_di");
        let mut attrs: Vec<(&str, &str)> = vec![("id", &di_id), ("bpmnElement", i)];
        if t == "participant" || t == "lane" {
            attrs.push(("isHorizontal", "true"));
        }
        if is_subp(&t) {
            attrs.push((
                "isExpanded",
                if md.has_flow_children(e) {
                    "true"
                } else {
                    "false"
                },
            ));
        }
        if t == "exclusiveGateway" {
            attrs.push(("isMarkerVisible", "true"));
        }
        let sh = md
            .doc
            .sub_element(plane, &qn(BPMNDI_NS, "BPMNShape"), &attrs);
        let [x, y, w, h] = rf(shapes.map[i]);
        let (xs, ys, ws, hs) = (round_str(x), round_str(y), round_str(w), round_str(h));
        md.doc.sub_element(
            sh,
            &qn(DC_NS, "Bounds"),
            &[("x", &xs), ("y", &ys), ("width", &ws), ("height", &hs)],
        );
        if let Some(lb) = labels.get(i) {
            write_label(md, sh, *lb);
        }
    }
    for e in md.doc.iter(root) {
        if md.id_of(e).is_none() {
            continue;
        }
        let i = key(md.id_of(e));
        if let Some(pts) = wp.get(&i).cloned() {
            let di_id = format!("{i}_di");
            let ed = md.doc.sub_element(
                plane,
                &qn(BPMNDI_NS, "BPMNEdge"),
                &[("id", &di_id), ("bpmnElement", &i)],
            );
            for (px, py) in pts {
                let (xs, ys) = (round_str(px), round_str(py));
                md.doc
                    .sub_element(ed, &qn(DI_NS, "waypoint"), &[("x", &xs), ("y", &ys)]);
            }
            if let Some(lb) = labels.get(&i) {
                write_label(md, ed, *lb);
            }
        }
    }
    md.reindex();
    order.len()
}

/// A conversation node with the participants it links.
type LinkedNode = (Id, Vec<String>);

/// Conversation nodes in the gap below one pool.
struct ConvRow {
    collab: Id,
    cy: i64,
    width: i64,
    /// node -> (participant, its bounds)
    nodes: Vec<(String, Vec<(String, Rect)>)>,
}

/// Place the conversation nodes of a gap where no vertical line (message flows, other
/// links) crosses it, and draw their links to the pools straight up and down.
fn place_conversations(
    md: &Model,
    row: &ConvRow,
    labels: &HashMap<String, [i64; 4]>,
    out: &mut OMap<Rect>,
    wp: &mut OMap<Vec<Pt>>,
) {
    #[allow(clippy::cast_precision_loss)]
    let cy = row.cy as f64;
    // occupied stretches of the gap: vertical lines crossing it, placed nodes and labels
    let mut taken: Vec<(f64, f64)> = Vec::new();
    for pts in wp.map.values() {
        for s in pts.windows(2) {
            let ((x1, y1), (x2, y2)) = (s[0], s[1]);
            if (x1 - x2).abs() < 1.0 && y1.min(y2) <= cy && y1.max(y2) >= cy {
                taken.push((x1 - 1.0, x1 + 1.0));
            }
        }
    }
    for l in labels.values() {
        if l[1] <= row.cy + CONV_H / 2 && l[1] + l[3] >= row.cy - CONV_H / 2 {
            #[allow(clippy::cast_precision_loss)]
            taken.push((l[0] as f64, (l[0] + l[2]) as f64));
        }
    }
    let half = CONV_W / 2;
    for (nid, linked) in &row.nodes {
        // the name stands to the right of the hexagon
        let name = md
            .get(Some(nid))
            .map(|e| md.name(Some(e)))
            .unwrap_or_default();
        let label_w = crate::svg::wrap(&name, 110.0, 11.0)
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let label_w = label_w as f64 * 6.2;
        // the candidate nearest to the middle whose stretch is free, else the one with the
        // most room around it (the leftmost on a tie)
        let mid = row.width / 2;
        let mut best: Option<(bool, i64, f64, i64)> = None;
        let mut x = 30 + half + 20;
        while x <= row.width - half - 20 {
            #[allow(clippy::cast_precision_loss)]
            let xf = x as f64;
            let (a, b) = (xf - CONV_HALF - 20.0, xf + CONV_HALF + label_w + 16.0);
            let clear = taken
                .iter()
                .map(|(l, r)| {
                    if *r < a {
                        a - r
                    } else if *l > b {
                        l - b
                    } else {
                        0.0
                    }
                })
                .fold(f64::INFINITY, f64::min);
            let roomy = clear > 0.0;
            let near = -(x - mid).abs();
            let better = match best {
                None => true,
                Some((r, n, c, _)) => {
                    (roomy && !r) || (roomy && r && near > n) || (!roomy && !r && clear > c + 0.5)
                }
            };
            if better {
                best = Some((roomy, near, clear, x));
            }
            x += 10;
        }
        let cx = best.map_or(mid, |b| b.3);
        let r = [cx - half, row.cy - CONV_H / 2, CONV_W, CONV_H];
        out.insert(nid, r);
        #[allow(clippy::cast_precision_loss)]
        let xf = cx as f64;
        taken.push((xf - CONV_HALF, xf + CONV_HALF + label_w + 6.0));
        for (p, pr) in linked {
            #[allow(clippy::cast_precision_loss)]
            let pts = if pr[1] > r[1] {
                vec![(xf, (r[1] + r[3]) as f64), (xf, pr[1] as f64)]
            } else {
                vec![(xf, r[1] as f64), (xf, (pr[1] + pr[3]) as f64)]
            };
            if let Some(l) = link_id(md, row.collab, nid, p) {
                wp.insert(&l, pts);
            }
        }
    }
}

/// Top-level conversation nodes of a collaboration with the participants they link
/// (by conversation links, else by their participant references).
fn conversation_nodes(md: &Model, collab: Id) -> Vec<(Id, Vec<String>)> {
    let d = &md.doc;
    let links: Vec<Id> = d.findall(collab, &m("conversationLink"));
    let mut out = Vec::new();
    for &c in d.children(collab) {
        if !is_conv(&md.tag(c)) {
            continue;
        }
        let cid = md.id_of(c);
        let mut linked: Vec<String> = links
            .iter()
            .filter(|l| md.attr(**l, "targetRef") == cid || md.attr(**l, "sourceRef") == cid)
            .map(|l| {
                if md.attr(*l, "targetRef") == cid {
                    key(md.attr(*l, "sourceRef"))
                } else {
                    key(md.attr(*l, "targetRef"))
                }
            })
            .collect();
        if linked.is_empty() {
            linked = d
                .findall(c, &m("participantRef"))
                .iter()
                .map(|r| strip_text(md, *r))
                .collect();
        }
        out.push((c, linked));
    }
    out
}

/// The conversation link between a conversation node and a participant.
fn link_id(md: &Model, collab: Id, node: &str, participant: &str) -> Option<String> {
    md.doc
        .findall(collab, &m("conversationLink"))
        .into_iter()
        .find(|l| {
            let (s, t) = (md.attr(*l, "sourceRef"), md.attr(*l, "targetRef"));
            (s == Some(participant) && t == Some(node))
                || (t == Some(participant) && s == Some(node))
        })
        .and_then(|l| md.id_of(l).map(str::to_string))
}

/// `<bpmndi:BPMNLabel><dc:Bounds .../></bpmndi:BPMNLabel>` inside a shape or an edge.
fn write_label(md: &mut Model, parent: Id, b: [i64; 4]) {
    let lb = md.doc.sub_element(parent, &qn(BPMNDI_NS, "BPMNLabel"), &[]);
    let (xs, ys, ws, hs) = (
        b[0].to_string(),
        b[1].to_string(),
        b[2].to_string(),
        b[3].to_string(),
    );
    md.doc.sub_element(
        lb,
        &qn(DC_NS, "Bounds"),
        &[("x", &xs), ("y", &ys), ("width", &ws), ("height", &hs)],
    );
}
