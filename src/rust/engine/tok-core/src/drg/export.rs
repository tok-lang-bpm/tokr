//! DMN 1.5 export of the decision requirements graphs of a 0.7 document: one DMN file per
//! connected graph with its DMN types, its elements, their boxed expressions and a DMNDI
//! diagram laid out automatically, and an SVG picture of the diagram.

use std::collections::HashMap;

use super::{Kind, base_of, bindings, dmn_items, graph_name, kind_of, params, requires, var_name};
use crate::model::{Base, Component, Decision, Document};
use crate::xml::{DMN_DC_NS, DMN_DI_NS, DMN15_NS, DMNDI15_NS, MODELER_NS, XElem};

thread_local! {
    /// While a document for Camunda is exported: the names of its FEEL (see [`fx`]).
    static CAMUNDA: std::cell::RefCell<Option<crate::feel3::Names>> = const { std::cell::RefCell::new(None) };
}

/// A FEEL text as written into the DMN file: unchanged, or for Camunda with the names that
/// are not plain names in backticks (`feel3::parse::camunda_text`).
fn fx(text: &str, tests: bool) -> String {
    CAMUNDA.with(|c| match &*c.borrow() {
        Some(names) => crate::feel3::parse::camunda_text(text, names, tests),
        None => text.to_string(),
    })
}

/// Whether the decisions of a document are exported for Camunda 8: a 0.7 document whose
/// header or a pool is `executable`, with `target=camunda` (the default).
pub fn for_camunda(doc: &Document) -> bool {
    doc.executable_any() && doc.attrs.get("target").is_none_or(|t| t == "camunda")
}

fn el(name: &str) -> XElem {
    XElem::new(DMN15_NS, name)
}

fn with_text(name: &str, text: &str) -> XElem {
    let mut e = el(name);
    e.push(text_el(text));
    e
}

fn text_el(text: &str) -> XElem {
    let mut t = el("text");
    t.text = Some(text.to_string());
    t
}

fn plain(name: &str, text: &str) -> XElem {
    let mut e = el(name);
    e.text = Some(text.to_string());
    e
}

/// `typeRef` of a `type=` value: a DMN type by its name, `any` as `Any`.
fn type_ref(doc: &Document, t: &str) -> String {
    let t = t.trim();
    if let Some(item) = doc.items.get(t)
        && super::is_dmn_item(doc, item)
    {
        return var_name(item);
    }
    if t == "any" {
        return "Any".into();
    }
    t.to_string()
}

fn set_type(doc: &Document, e: &mut XElem, t: Option<&str>) {
    if let Some(t) = t {
        e.set("typeRef", type_ref(doc, t));
    }
}

fn names(e: &mut XElem, b: &Base) {
    let name = var_name(b);
    e.set("id", b.id());
    e.set("name", &name);
    if let Some(l) = b.label.as_deref().filter(|l| !l.is_empty() && **l != name) {
        e.set("label", l);
    }
}

fn description(e: &mut XElem, b: &Base) {
    if let Some(d) = b.attr("doc") {
        e.push(plain("description", d));
    }
}

fn variable(doc: &Document, b: &Base, t: Option<&str>) -> XElem {
    let mut v = el("variable");
    v.set("id", format!("{}-variable", b.id()));
    v.set("name", var_name(b));
    set_type(doc, &mut v, t);
    v
}

fn hit_policy(h: &str) -> String {
    match h {
        "rule-order" => "RULE ORDER".into(),
        "output-order" => "OUTPUT ORDER".into(),
        other => other.to_uppercase(),
    }
}

fn table(doc: &Document, d: &Decision, id: &str) -> XElem {
    let mut t = el("decisionTable");
    t.set("id", format!("{id}-table"));
    t.set("hitPolicy", hit_policy(d.hit()));
    if let Some(a) = d.base.attr("aggregation") {
        t.set("aggregation", a.to_uppercase());
    }
    for (k, (expr, typ)) in d.inputs.iter().enumerate() {
        let k = k + 1;
        let mut inp = el("input");
        inp.set("id", format!("{id}-input-{k}"));
        inp.set("label", expr);
        let mut ie = el("inputExpression");
        ie.set("id", format!("{id}-input-{k}-expr"));
        ie.set("typeRef", type_ref(doc, typ));
        ie.push(text_el(&fx(expr, false)));
        inp.push(ie);
        if let Some(Some(v)) = d.input_values.get(k - 1) {
            inp.push(with_text("inputValues", &fx(v, true)));
        }
        t.push(inp);
    }
    for (k, (name, typ)) in d.outputs.iter().enumerate() {
        let k = k + 1;
        let mut o = el("output");
        o.set("id", format!("{id}-output-{k}"));
        o.set("label", name);
        o.set("name", name);
        o.set("typeRef", type_ref(doc, typ));
        if let Some(Some(v)) = d.output_values.get(k - 1) {
            o.push(with_text("outputValues", &fx(v, true)));
        }
        if let Some(def) = default_entry(d, name) {
            let mut de = el("defaultOutputEntry");
            de.set("id", format!("{id}-output-{k}-default"));
            de.push(text_el(&fx(&def, false)));
            o.push(de);
        }
        t.push(o);
    }
    for (r, rule) in d.rules.iter().enumerate() {
        let r = r + 1;
        let mut rel = el("rule");
        rel.set("id", format!("{id}-rule-{r}"));
        if let Some(a) = rule.annotation.as_deref().filter(|a| !a.is_empty()) {
            rel.push(plain("description", a));
        }
        for (k, cell) in rule.inputs.iter().enumerate() {
            let mut e = el("inputEntry");
            e.set("id", format!("{id}-rule-{r}-in-{}", k + 1));
            e.push(text_el(&fx(cell, true)));
            rel.push(e);
        }
        for (k, cell) in rule.outputs.iter().enumerate() {
            let mut e = el("outputEntry");
            e.set("id", format!("{id}-rule-{r}-out-{}", k + 1));
            e.push(text_el(&if cell == "-" {
                String::new()
            } else {
                fx(cell, false)
            }));
            rel.push(e);
        }
        t.push(rel);
    }
    t
}

/// The default output entry of the output `name` (`default=`).
fn default_entry(d: &Decision, name: &str) -> Option<String> {
    let text = d.base.attr("default")?;
    if d.outputs.len() == 1 {
        return Some(text.to_string());
    }
    bindings(text)
        .ok()?
        .into_iter()
        .find(|(n, _)| *n == super::normalize(name))
        .map(|(_, e)| e)
}

fn relation(doc: &Document, d: &Decision, id: &str) -> XElem {
    let mut r = el("relation");
    r.set("id", format!("{id}-relation"));
    for (k, (name, typ)) in d.inputs.iter().enumerate() {
        let mut c = el("column");
        c.set("id", format!("{id}-column-{}", k + 1));
        c.set("name", name);
        if typ != "any" {
            c.set("typeRef", type_ref(doc, typ));
        }
        r.push(c);
    }
    for (k, rule) in d.rules.iter().enumerate() {
        let mut row = el("row");
        row.set("id", format!("{id}-row-{}", k + 1));
        for (j, cell) in rule.inputs.iter().enumerate() {
            let mut le = el("literalExpression");
            le.set("id", format!("{id}-row-{}-{}", k + 1, j + 1));
            le.push(text_el(&fx(cell, false)));
            row.push(le);
        }
        r.push(row);
    }
    r
}

fn bkm_name(doc: &Document, id: &str) -> String {
    doc.bkms
        .get(id)
        .map(|b| var_name(&b.base))
        .unwrap_or_else(|| id.to_string())
}

/// The boxed expression of a box, without a function definition around it.
fn logic(doc: &Document, d: &Decision, id: &str) -> Option<XElem> {
    if let Some(e) = d.base.attr("expr") {
        let mut le = el("literalExpression");
        le.set("id", format!("{id}-expr"));
        le.push(text_el(&fx(e, false)));
        return Some(le);
    }
    if let Some(b) = d.base.attr("invoke") {
        let mut inv = el("invocation");
        inv.set("id", format!("{id}-invocation"));
        let mut callee = el("literalExpression");
        callee.set("id", format!("{id}-invocation-callee"));
        callee.push(text_el(&bkm_name(doc, b)));
        inv.push(callee);
        let declared = doc
            .bkms
            .get(b)
            .and_then(|k| k.base.attr("params"))
            .and_then(|p| params(p).ok())
            .unwrap_or_default();
        for (k, (p, expr)) in bindings(d.base.attr("with").unwrap_or(""))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            let mut bind = el("binding");
            let mut par = el("parameter");
            par.set("id", format!("{id}-binding-{}", k + 1));
            par.set("name", &p);
            if let Some((_, Some(t))) = declared.iter().find(|(n, _)| *n == p) {
                par.set("typeRef", type_ref(doc, t));
            }
            bind.push(par);
            let mut le = el("literalExpression");
            le.set("id", format!("{id}-binding-{}-expr", k + 1));
            le.push(text_el(&fx(&expr, false)));
            bind.push(le);
            inv.push(bind);
        }
        return Some(inv);
    }
    if d.has_table() {
        return Some(if d.relation {
            relation(doc, d, id)
        } else {
            table(doc, d, id)
        });
    }
    if d.lines.is_empty() {
        return None;
    }
    if d.lines.iter().all(|l| l.base.kind == "element") {
        let mut list = el("list");
        list.set("id", format!("{id}-list"));
        for (k, l) in d.lines.iter().enumerate() {
            if let Some(x) = expression(doc, l, &format!("{id}-{}", k + 1)) {
                list.push(x);
            }
        }
        return Some(list);
    }
    let mut ctx = el("context");
    ctx.set("id", format!("{id}-context"));
    for (k, l) in d.lines.iter().enumerate() {
        let lid = format!("{id}-{}", k + 1);
        let mut ce = el("contextEntry");
        ce.set("id", format!("{lid}-entry"));
        if l.base.kind == "entry" {
            let mut v = el("variable");
            v.set("id", format!("{lid}-variable"));
            v.set("name", super::normalize(l.base.id()));
            if !l.base.attrs.contains_key("params") {
                set_type(doc, &mut v, l.base.attr("type"));
            }
            ce.push(v);
        }
        if let Some(x) = expression(doc, l, &lid) {
            ce.push(x);
        }
        ctx.push(ce);
    }
    Some(ctx)
}

/// The boxed expression of an entry, a result or an element: a function definition when
/// it has `params=`.
fn expression(doc: &Document, d: &Decision, id: &str) -> Option<XElem> {
    let inner = logic(doc, d, id);
    let Some(p) = d.base.attr("params") else {
        return inner;
    };
    let mut f = el("functionDefinition");
    f.set("id", format!("{id}-function"));
    f.set("kind", "FEEL");
    set_type(doc, &mut f, d.base.attr("type"));
    for (k, (name, t)) in params(p).unwrap_or_default().into_iter().enumerate() {
        let mut fp = el("formalParameter");
        fp.set("id", format!("{id}-param-{}", k + 1));
        fp.set("name", name);
        set_type(doc, &mut fp, t.as_deref());
        f.push(fp);
    }
    if let Some(x) = inner {
        f.push(x);
    }
    Some(f)
}

/// Identifier of the requirement of `target` on `source`.
pub fn requirement_id(target: &str, source: &str) -> String {
    format!("{target}-requires-{source}")
}

fn requirement(tag: &str, target: &str, req: &str, child: &str) -> XElem {
    let mut r = el(tag);
    r.set("id", requirement_id(target, req));
    let mut c = el(child);
    c.set("href", format!("#{req}"));
    r.push(c);
    r
}

fn requirements(doc: &Document, b: &Base, e: &mut XElem) {
    let reqs = requires(b);
    let kinds: Vec<(String, Option<Kind>)> =
        reqs.iter().map(|r| (r.clone(), kind_of(doc, r))).collect();
    let me = kind_of(doc, b.id());
    for (r, k) in &kinds {
        match (me, k) {
            (Some(Kind::Decision), Some(Kind::Decision)) => {
                e.push(requirement(
                    "informationRequirement",
                    b.id(),
                    r,
                    "requiredDecision",
                ));
            }
            (Some(Kind::Decision), Some(Kind::InputData)) => {
                e.push(requirement(
                    "informationRequirement",
                    b.id(),
                    r,
                    "requiredInput",
                ));
            }
            _ => {}
        }
    }
    for (r, k) in &kinds {
        if *k == Some(Kind::Bkm) && matches!(me, Some(Kind::Decision | Kind::Bkm)) {
            e.push(requirement(
                "knowledgeRequirement",
                b.id(),
                r,
                "requiredKnowledge",
            ));
        }
    }
    for (r, k) in &kinds {
        let child = match (me, k) {
            (Some(Kind::Source), Some(Kind::Decision)) => "requiredDecision",
            (Some(Kind::Source), Some(Kind::InputData)) => "requiredInput",
            (_, Some(Kind::Source)) => "requiredAuthority",
            _ => continue,
        };
        e.push(requirement("authorityRequirement", b.id(), r, child));
    }
}

fn components(doc: &Document, list: &[Component], prefix: &str, parent: &mut XElem) {
    for (k, c) in list.iter().enumerate() {
        let id = format!("{prefix}-{}", k + 1);
        let mut ic = el("itemComponent");
        ic.set("id", &id);
        ic.set("name", super::normalize(c.base.id()));
        if c.base.has("collection") {
            ic.set("isCollection", "true");
        }
        if let Some(t) = c.base.attr("type") {
            ic.push(plain("typeRef", &type_ref(doc, t)));
        }
        if let Some(a) = c.base.attr("allowed") {
            ic.push(with_text("allowedValues", &fx(a, true)));
        }
        components(doc, &c.children, &id, &mut ic);
        parent.push(ic);
    }
}

fn item_definition(doc: &Document, b: &Base) -> XElem {
    let mut it = el("itemDefinition");
    names(&mut it, b);
    if b.has("collection") {
        it.set("isCollection", "true");
    }
    if let Some(t) = b.attr("type") {
        it.push(plain("typeRef", &type_ref(doc, t)));
    }
    if let Some(a) = b.attr("allowed") {
        it.push(with_text("allowedValues", &fx(a, true)));
    }
    if let Some(list) = doc.components.get(b.id()) {
        components(doc, list, b.id(), &mut it);
    }
    it
}

fn drg_element(doc: &Document, id: &str) -> Option<XElem> {
    match kind_of(doc, id)? {
        Kind::InputData => {
            let b = doc.inputdata.get(id)?;
            let mut e = el("inputData");
            names(&mut e, b);
            description(&mut e, b);
            e.push(variable(doc, b, b.attr("type")));
            Some(e)
        }
        Kind::Source => {
            let b = doc.sources.get(id)?;
            let mut e = el("knowledgeSource");
            names(&mut e, b);
            if let Some(u) = b.attr("uri") {
                e.set("locationURI", u);
            }
            description(&mut e, b);
            requirements(doc, b, &mut e);
            if let Some(k) = b.attr("kind") {
                e.push(plain("type", k));
            }
            Some(e)
        }
        Kind::Bkm => {
            let d = doc.bkms.get(id)?;
            let b = &d.base;
            let mut e = el("businessKnowledgeModel");
            names(&mut e, b);
            description(&mut e, b);
            let mut v = el("variable");
            v.set("id", format!("{id}-variable"));
            v.set("name", var_name(b));
            e.push(v);
            let mut f = el("encapsulatedLogic");
            f.set("id", format!("{id}-logic"));
            f.set("kind", "FEEL");
            set_type(doc, &mut f, b.attr("type"));
            for (k, (name, t)) in params(b.attr("params").unwrap_or(""))
                .unwrap_or_default()
                .into_iter()
                .enumerate()
            {
                let mut fp = el("formalParameter");
                fp.set("id", format!("{id}-param-{}", k + 1));
                fp.set("name", name);
                set_type(doc, &mut fp, t.as_deref());
                f.push(fp);
            }
            if let Some(x) = logic(doc, d, id) {
                f.push(x);
            }
            e.push(f);
            requirements(doc, b, &mut e);
            Some(e)
        }
        Kind::Decision => {
            let d = doc.decisions.get(id)?;
            let b = &d.base;
            let mut e = el("decision");
            names(&mut e, b);
            description(&mut e, b);
            if let Some(q) = b.attr("question") {
                e.push(plain("question", q));
            }
            if let Some(a) = b.attr("answers") {
                e.push(plain("allowedAnswers", a));
            }
            let t = b.attr("type").map(str::to_string).or_else(|| {
                (d.has_table() && !d.relation && d.outputs.len() == 1 && d.hit() != "collect")
                    .then(|| d.outputs[0].1.clone())
            });
            e.push(variable(doc, b, t.as_deref()));
            requirements(doc, b, &mut e);
            if let Some(x) = logic(doc, d, id) {
                e.push(x);
            }
            Some(e)
        }
    }
}

// ---------------------------------------------------------------- layout

/// A placed shape: identifier, kind and bounds.
#[derive(Debug, Clone)]
pub struct Shape {
    pub id: String,
    pub kind: Kind,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

/// An edge of a requirement: its identifier, kind of requirement, ends and waypoints.
#[derive(Debug, Clone)]
pub struct Edge {
    pub id: String,
    /// `information`, `knowledge` or `authority`.
    pub kind: &'static str,
    pub source: String,
    pub target: String,
    pub points: Vec<(i64, i64)>,
}

fn size(k: Kind) -> (i64, i64) {
    match k {
        Kind::Decision => (180, 80),
        Kind::InputData => (125, 45),
        Kind::Bkm => (135, 46),
        Kind::Source => (100, 63),
    }
}

fn edge_kind(target: Option<Kind>, source: Option<Kind>) -> &'static str {
    match (target, source) {
        (_, Some(Kind::Source)) | (Some(Kind::Source), _) => "authority",
        (_, Some(Kind::Bkm)) => "knowledge",
        _ => "information",
    }
}

/// Layered layout: an element stands one row below the lowest element that requires
/// it; rows are ordered by the positions of the elements that require them.
pub fn layout(doc: &Document, graph: &[String]) -> (Vec<Shape>, Vec<Edge>) {
    let ids: Vec<&String> = graph.iter().collect();
    let in_graph = |x: &str| graph.iter().any(|g| g == x);
    // edges requirer -> required
    let mut reqs: Vec<(String, String)> = Vec::new();
    for id in &ids {
        if let Some(b) = base_of(doc, id) {
            for r in requires(b) {
                if in_graph(&r) && r != **id {
                    reqs.push(((*id).clone(), r));
                }
            }
        }
    }
    let mut level: HashMap<String, usize> = ids.iter().map(|i| ((*i).clone(), 0)).collect();
    for _ in 0..ids.len() + 1 {
        let mut changed = false;
        for (a, b) in &reqs {
            let la = level[a];
            if level[b] < la + 1 && la < ids.len() {
                level.insert(b.clone(), la + 1);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let rows = level.values().copied().max().unwrap_or(0) + 1;
    let mut row_of: Vec<Vec<String>> = vec![Vec::new(); rows];
    for id in &ids {
        row_of[level[*id]].push((*id).clone());
    }
    // order each row by the mean position of its requirers in the row above
    for r in 1..rows {
        let above: HashMap<String, usize> = (0..r)
            .flat_map(|q| {
                row_of[q]
                    .iter()
                    .enumerate()
                    .map(|(k, x)| (x.clone(), k))
                    .collect::<Vec<_>>()
            })
            .collect();
        let key = |x: &String| -> f64 {
            let ps: Vec<f64> = reqs
                .iter()
                .filter(|(_, b)| b == x)
                .filter_map(|(a, _)| above.get(a).map(|k| *k as f64))
                .collect();
            if ps.is_empty() {
                f64::MAX
            } else {
                ps.iter().sum::<f64>() / ps.len() as f64
            }
        };
        let mut keyed: Vec<(f64, usize, String)> = row_of[r]
            .iter()
            .enumerate()
            .map(|(k, x)| (key(x), k, x.clone()))
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        row_of[r] = keyed.into_iter().map(|t| t.2).collect();
    }
    // centres: rows start side by side, then each element moves towards the mean of the
    // elements it is connected with, keeping its order and a gap of 40 units
    let step = 220.0;
    let gap = 40.0;
    let width = |id: &str| size(kind_of(doc, id).unwrap_or(Kind::Decision)).0 as f64;
    let mut cx: HashMap<String, f64> = HashMap::new();
    for row in &row_of {
        for (k, id) in row.iter().enumerate() {
            cx.insert(id.clone(), k as f64 * step);
        }
    }
    let neighbours = |id: &str, up: bool| -> Vec<String> {
        reqs.iter()
            .filter_map(|(a, b)| {
                if up && b == id {
                    Some(a.clone())
                } else if !up && a == id {
                    Some(b.clone())
                } else {
                    None
                }
            })
            .collect()
    };
    for pass in 0..8 {
        let up = pass % 2 == 0;
        let order: Vec<usize> = if up {
            (0..rows).collect()
        } else {
            (0..rows).rev().collect()
        };
        for r in order {
            let row = row_of[r].clone();
            let mut want: Vec<f64> = row
                .iter()
                .map(|id| {
                    let ns = neighbours(id, up);
                    let xs: Vec<f64> = ns.iter().filter_map(|n| cx.get(n).copied()).collect();
                    if xs.is_empty() {
                        cx[id]
                    } else {
                        xs.iter().sum::<f64>() / xs.len() as f64
                    }
                })
                .collect();
            // keep the order and the gaps: push right, then pull left where there is room
            for k in 1..row.len() {
                let min = want[k - 1] + (width(&row[k - 1]) + width(&row[k])) / 2.0 + gap;
                if want[k] < min {
                    want[k] = min;
                }
            }
            for (k, id) in row.iter().enumerate() {
                cx.insert(id.clone(), want[k]);
            }
        }
    }
    let left = row_of
        .iter()
        .flatten()
        .map(|id| cx[id] - width(id) / 2.0)
        .fold(f64::INFINITY, f64::min);
    let shift = 150.0 - if left.is_finite() { left } else { 0.0 };
    let mut shapes: Vec<Shape> = Vec::new();
    for (r, row) in row_of.iter().enumerate() {
        for id in row {
            let kind = kind_of(doc, id).unwrap_or(Kind::Decision);
            let (w, h) = size(kind);
            let c = (cx[id] + shift).round() as i64;
            let cy = 80 + r as i64 * 150 + 40;
            shapes.push(Shape {
                id: id.clone(),
                kind,
                x: c - w / 2,
                y: cy - h / 2,
                w,
                h,
            });
        }
    }
    let shape = |id: &str| shapes.iter().find(|s| s.id == id).cloned();
    let mut edges = Vec::new();
    for (a, b) in &reqs {
        let (Some(t), Some(s)) = (shape(a), shape(b)) else {
            continue;
        };
        let ins: Vec<&(String, String)> = reqs.iter().filter(|(x, _)| x == a).collect();
        let outs: Vec<&(String, String)> = reqs.iter().filter(|(_, y)| y == b).collect();
        let i = ins.iter().position(|e| e.1 == *b).unwrap_or(0) as i64;
        let o = outs.iter().position(|e| e.0 == *a).unwrap_or(0) as i64;
        let tx = t.x + t.w * (i + 1) / (ins.len() as i64 + 1);
        let sx = s.x + s.w * (o + 1) / (outs.len() as i64 + 1);
        let (sy, ty) = if s.y > t.y {
            (s.y, t.y + t.h)
        } else {
            (s.y + s.h, t.y)
        };
        edges.push(Edge {
            id: requirement_id(a, b),
            kind: edge_kind(kind_of(doc, a), kind_of(doc, b)),
            source: b.clone(),
            target: a.clone(),
            points: vec![(sx, sy), (tx, ty)],
        });
    }
    (shapes, edges)
}

fn bounds(x: i64, y: i64, w: i64, h: i64) -> XElem {
    let mut b = XElem::new(DMN_DC_NS, "Bounds");
    b.set("height", h.to_string());
    b.set("width", w.to_string());
    b.set("x", x.to_string());
    b.set("y", y.to_string());
    b
}

fn diagram(doc: &Document, graph: &[String], name: &str) -> XElem {
    let (shapes, edges) = layout(doc, graph);
    let mut di = XElem::new(DMNDI15_NS, "DMNDI");
    let mut dg = XElem::new(DMNDI15_NS, "DMNDiagram");
    dg.set("id", format!("DMNDiagram_{name}"));
    for s in &shapes {
        let mut sh = XElem::new(DMNDI15_NS, "DMNShape");
        sh.set("id", format!("DMNShape_{}", s.id));
        sh.set("dmnElementRef", &s.id);
        sh.push(bounds(s.x, s.y, s.w, s.h));
        dg.push(sh);
    }
    for e in &edges {
        let mut ed = XElem::new(DMNDI15_NS, "DMNEdge");
        ed.set("id", format!("DMNEdge_{}", e.id));
        ed.set("dmnElementRef", &e.id);
        for (x, y) in &e.points {
            let mut w = XElem::new(DMN_DI_NS, "waypoint");
            w.set("x", x.to_string());
            w.set("y", y.to_string());
            ed.push(w);
        }
        dg.push(ed);
    }
    di.push(dg);
    di
}

/// The DMN 1.5 files of a 0.7 document: (file stem, XML text), one per graph.
pub fn export_dmn15(doc: &Document) -> Vec<(String, String)> {
    let ns = doc
        .attrs
        .get("namespace")
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| "http://camunda.org/schema/1.0/dmn".into());
    let mut out = Vec::new();
    let names = for_camunda(doc).then(|| super::names(doc));
    let previous = CAMUNDA.with(|c| c.replace(names));
    for g in super::graphs(doc) {
        let name = graph_name(doc, &g).to_string();
        let title = base_of(doc, &name)
            .map(|b| {
                b.label
                    .clone()
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| var_name(b))
            })
            .unwrap_or_else(|| name.clone());
        let mut root = el("definitions");
        root.set("id", format!("{name}-definitions"));
        root.set("name", &title);
        root.set("namespace", &ns);
        root.set("exporter", "tok");
        root.set("exporterVersion", doc.version.as_deref().unwrap_or("0.7"));
        root.set_ns(MODELER_NS, "executionPlatform", "Camunda Cloud");
        root.set_ns(MODELER_NS, "executionPlatformVersion", "8.10.0");
        for b in dmn_items(doc) {
            root.push(item_definition(doc, b));
        }
        for id in &g {
            if let Some(e) = drg_element(doc, id) {
                root.push(e);
            }
        }
        root.push(diagram(doc, &g, &name));
        root.indent(0);
        out.push((
            name,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}\n",
                root.to_string_root()
            ),
        ));
    }
    CAMUNDA.with(|c| c.replace(previous));
    out
}

// ---------------------------------------------------------------- picture

fn esc(s: &str) -> String {
    crate::xml::escape_attrib(s)
}

/// Words of a label broken into lines of about `width` characters.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// An SVG picture of the diagram of one graph.
pub fn svg(doc: &Document, graph: &[String]) -> String {
    let (shapes, edges) = layout(doc, graph);
    let maxx = shapes.iter().map(|s| s.x + s.w).max().unwrap_or(200) + 150;
    let maxy = shapes.iter().map(|s| s.y + s.h).max().unwrap_or(100) + 80;
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{maxx}\" height=\"{maxy}\" viewBox=\"0 0 {maxx} {maxy}\" font-family=\"Arial, Helvetica, sans-serif\" font-size=\"12\">\n"
    ));
    out.push_str("  <defs>\n    <marker id=\"information\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"9\" markerHeight=\"9\" orient=\"auto\"><path d=\"M 0 0 L 10 5 L 0 10 Z\" fill=\"#222\"/></marker>\n    <marker id=\"knowledge\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"9\" markerHeight=\"9\" orient=\"auto\"><path d=\"M 0 0 L 10 5 L 0 10\" fill=\"none\" stroke=\"#222\"/></marker>\n    <marker id=\"authority\" viewBox=\"0 0 10 10\" refX=\"5\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\"><circle cx=\"5\" cy=\"5\" r=\"4\" fill=\"#222\"/></marker>\n  </defs>\n");
    out.push_str(&format!(
        "  <rect width=\"{maxx}\" height=\"{maxy}\" fill=\"white\"/>\n"
    ));
    for s in &shapes {
        let (x, y, w, h) = (s.x, s.y, s.w, s.h);
        let shape = match s.kind {
            Kind::Decision => format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"#fff\" stroke=\"#222\" stroke-width=\"2\"/>"
            ),
            Kind::InputData => format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{}\" ry=\"{}\" fill=\"#fff\" stroke=\"#222\" stroke-width=\"2\"/>",
                h / 2,
                h / 2
            ),
            Kind::Bkm => {
                let c = 12;
                format!(
                    "<polygon points=\"{},{y} {},{y} {},{} {},{} {x},{} {x},{}\" fill=\"#fff\" stroke=\"#222\" stroke-width=\"2\"/>",
                    x + c,
                    x + w,
                    x + w,
                    y + h - c,
                    x + w - c,
                    y + h,
                    y + h,
                    y + c
                )
            }
            Kind::Source => {
                let b = y + h - 10;
                format!(
                    "<path d=\"M {x} {y} L {} {y} L {} {b} C {} {} {} {} {x} {b} Z\" fill=\"#fff\" stroke=\"#222\" stroke-width=\"2\"/>",
                    x + w,
                    x + w,
                    x + w * 65 / 100,
                    b - 16,
                    x + w * 35 / 100,
                    b + 16,
                )
            }
        };
        out.push_str(&format!("  <g id=\"{}\">{shape}", esc(&s.id)));
        let label = base_of(doc, &s.id)
            .map(|b| {
                b.label
                    .clone()
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| var_name(b))
            })
            .unwrap_or_default();
        let width = ((w - 16) / 7).max(6) as usize;
        let lines = wrap(&label, width);
        let n = lines.len() as i64;
        let cy = if s.kind == Kind::Source {
            y + (h - 10) / 2
        } else {
            y + h / 2
        };
        for (k, l) in lines.iter().enumerate() {
            let ly = cy - (n - 1) * 7 + k as i64 * 14 + 4;
            out.push_str(&format!(
                "<text x=\"{}\" y=\"{ly}\" text-anchor=\"middle\">{}</text>",
                x + w / 2,
                crate::xml::escape_cdata(l)
            ));
        }
        out.push_str("</g>\n");
    }
    for e in &edges {
        let pts: Vec<String> = e.points.iter().map(|(x, y)| format!("{x},{y}")).collect();
        let dash = if e.kind == "information" {
            ""
        } else {
            " stroke-dasharray=\"6 4\""
        };
        out.push_str(&format!(
            "  <polyline points=\"{}\" fill=\"none\" stroke=\"#222\" stroke-width=\"1.5\"{dash} marker-end=\"url(#{})\"/>\n",
            pts.join(" "),
            e.kind
        ));
    }
    out.push_str("</svg>\n");
    out
}

/// The SVG pictures of the graphs: (file stem, SVG text).
pub fn export_svg(doc: &Document) -> Vec<(String, String)> {
    super::graphs(doc)
        .into_iter()
        .map(|g| (graph_name(doc, &g).to_string(), svg(doc, &g)))
        .collect()
}

/// The decision files `tok export` writes: in a 0.7 document one DMN 1.5 file and one
/// picture per decision requirements graph (`NAME.dmn`, `NAME.drd.svg`), in earlier
/// documents one DMN 1.3 file per decision table (`ID.dmn`).
pub fn decision_files(doc: &Document) -> Vec<(String, String)> {
    if !doc.lang_07() {
        return doc
            .decisions
            .values()
            .map(|d| {
                (
                    format!("{}.dmn", d.base.id()),
                    crate::dmn::export_dmn(d, Some(doc)),
                )
            })
            .collect();
    }
    let mut out = Vec::new();
    for ((stem, xml), (_, svg)) in export_dmn15(doc).into_iter().zip(export_svg(doc)) {
        out.push((format!("{stem}.dmn"), xml));
        out.push((format!("{stem}.drd.svg"), svg));
    }
    out
}
