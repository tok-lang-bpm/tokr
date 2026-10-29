//! Decision requirements graphs of language 0.7: the elements of a graph (decisions,
//! input data, business knowledge models, knowledge sources), their names, requirements
//! and types, evaluation (`eval`), DMN 1.5 export with a diagram (`export`) and DMN
//! import (`dmnimport`).

pub mod dmnimport;
pub mod eval;
pub mod export;
pub mod values;

use crate::feel3::parse::{Names, Type, parse_type};
use crate::feel3::{Tests, TypeDef, Types, parse_tests};
use crate::model::{Base, Component, Decision, Document, FEEL_TYPES};
use crate::pyutil::strip;

/// The kind of an element of a decision requirements graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Decision,
    InputData,
    Bkm,
    Source,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::InputData => "inputdata",
            Kind::Bkm => "bkm",
            Kind::Source => "source",
        }
    }
}

/// The kind of the graph element `id`.
pub fn kind_of(doc: &Document, id: &str) -> Option<Kind> {
    if doc.decisions.contains_key(id) {
        Some(Kind::Decision)
    } else if doc.inputdata.contains_key(id) {
        Some(Kind::InputData)
    } else if doc.bkms.contains_key(id) {
        Some(Kind::Bkm)
    } else if doc.sources.contains_key(id) {
        Some(Kind::Source)
    } else {
        None
    }
}

/// The base of the graph element `id`.
pub fn base_of<'d>(doc: &'d Document, id: &str) -> Option<&'d Base> {
    doc.decisions
        .get(id)
        .map(|d| &d.base)
        .or_else(|| doc.inputdata.get(id))
        .or_else(|| doc.bkms.get(id).map(|d| &d.base))
        .or_else(|| doc.sources.get(id))
}

/// The FEEL name of an element: `name=`, else its identifier.
pub fn var_name(b: &Base) -> String {
    b.attr("name")
        .map(normalize)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| b.id().to_string())
}

/// White space of a name collapsed to single spaces.
pub fn normalize(n: &str) -> String {
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The identifiers of `requires=`, in written order, without repetitions.
pub fn requires(b: &Base) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in b.attr("requires").unwrap_or("").split_whitespace() {
        if !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// Every element of the graph in document order: input data, knowledge sources, business
/// knowledge models, decisions.
pub fn elements(doc: &Document) -> Vec<(&Base, Kind)> {
    let mut out: Vec<(&Base, Kind)> = Vec::new();
    out.extend(doc.inputdata.values().map(|b| (b, Kind::InputData)));
    out.extend(doc.sources.values().map(|b| (b, Kind::Source)));
    out.extend(doc.bkms.values().map(|b| (&b.base, Kind::Bkm)));
    out.extend(doc.decisions.values().map(|b| (&b.base, Kind::Decision)));
    out.sort_by_key(|(b, _)| b.line);
    out
}

/// Split at `sep` outside strings and brackets.
pub fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    let mut esc = false;
    for c in text.chars() {
        if in_str {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            c if c == sep && depth <= 0 => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The parameters of `params="a: number, b: string"`: name and type text.
pub fn params(text: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    if strip(text).is_empty() {
        return Ok(out);
    }
    for part in split_top(text, ',') {
        let part = strip(&part);
        let (name, typ) = match part.split_once(':') {
            Some((n, t)) => (normalize(n), Some(strip(t).to_string())),
            None => (normalize(part), None),
        };
        if name.is_empty() || typ.as_deref() == Some("") {
            return Err(part.to_string());
        }
        out.push((name, typ));
    }
    Ok(out)
}

/// The bindings of `with="p = FEEL; q = FEEL"`.
pub fn bindings(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for part in split_top(text, ';') {
        let part = strip(&part);
        if part.is_empty() {
            continue;
        }
        let Some(pos) = first_assign(part) else {
            return Err(part.to_string());
        };
        let name = normalize(&part[..pos]);
        let expr = strip(&part[pos + 1..]).to_string();
        if name.is_empty() || expr.is_empty() {
            return Err(part.to_string());
        }
        out.push((name, expr));
    }
    Ok(out)
}

/// The position of the first `=` that is not part of `!=`, `<=`, `>=`.
fn first_assign(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (0..b.len()).find(|&i| b[i] == b'=' && (i == 0 || !matches!(b[i - 1], b'!' | b'<' | b'>')))
}

/// Whether an `item` of a 0.7 document is a DMN type (`type=`, `allowed=` or components).
pub fn is_dmn_item(doc: &Document, b: &Base) -> bool {
    doc.lang_07()
        && (b.attrs.contains_key("type")
            || b.attrs.contains_key("allowed")
            || doc.components.get(b.id()).is_some_and(|c| !c.is_empty()))
}

/// The DMN types of the document by identifier, with their FEEL names.
pub fn dmn_items(doc: &Document) -> Vec<&Base> {
    doc.items.values().filter(|b| is_dmn_item(doc, b)).collect()
}

/// A type of `type=`: a FEEL type, a type expression, or a DMN type of the document
/// (by identifier or name).
pub fn type_of(doc: &Document, text: &str) -> Type {
    let t = strip(text);
    if let Some(item) = doc.items.get(t)
        && is_dmn_item(doc, item)
    {
        return Type::Named(var_name(item));
    }
    if t == "any" {
        return Type::Any;
    }
    Type::from_ref(t)
}

/// Whether `type=` names a known type.
pub fn known_type(doc: &Document, text: &str) -> bool {
    let t = strip(text);
    if FEEL_TYPES.contains(&t) || Type::builtin(t).is_some() {
        return true;
    }
    let items: Vec<String> = dmn_items(doc)
        .iter()
        .flat_map(|b| [b.id().to_string(), var_name(b)])
        .collect();
    if items.iter().any(|i| i == t) {
        return true;
    }
    fn names_ok(ty: &Type, items: &[String]) -> bool {
        match ty {
            Type::Named(n) => items.contains(n),
            Type::List(x) | Type::Range(x) => names_ok(x, items),
            Type::Context(f) => f.iter().all(|(_, x)| names_ok(x, items)),
            Type::Function(p, r) => p.iter().all(|x| names_ok(x, items)) && names_ok(r, items),
            _ => true,
        }
    }
    let mut names = Names::default();
    for i in &items {
        names.add(i);
    }
    parse_type(t, &names).is_ok_and(|ty| names_ok(&ty, &items))
}

fn type_def(doc: &Document, b: &Base, components: &[Component]) -> TypeDef {
    TypeDef {
        base: b.attr("type").map(|t| type_of(doc, t)),
        collection: b.has("collection"),
        allowed: b
            .attr("allowed")
            .and_then(|a| parse_tests(a, &names(doc)).ok()),
        components: components
            .iter()
            .map(|c| (normalize(c.base.id()), type_def(doc, &c.base, &c.children)))
            .collect(),
    }
}

/// The DMN types of the document for the evaluator.
pub fn types(doc: &Document) -> Types {
    let mut t = Types::default();
    for b in dmn_items(doc) {
        let comps = doc.components.get(b.id()).map(Vec::as_slice).unwrap_or(&[]);
        t.defs.insert(var_name(b), type_def(doc, b, comps));
    }
    t
}

/// Allowed values of a type, parsed.
pub fn allowed(doc: &Document, text: &str) -> Option<Tests> {
    parse_tests(text, &names(doc)).ok()
}

fn box_names(d: &Decision, n: &mut Names) {
    for (name, _) in &d.outputs {
        n.add(name);
    }
    if d.relation {
        for (name, _) in &d.inputs {
            n.add(name);
        }
    }
    if let Some(p) = d.base.attr("params")
        && let Ok(ps) = params(p)
    {
        for (name, _) in ps {
            n.add(&name);
        }
    }
    if let Some(w) = d.base.attr("with")
        && let Ok(bs) = bindings(w)
    {
        for (name, _) in bs {
            n.add(&name);
        }
    }
    for l in &d.lines {
        if l.base.kind == "entry" {
            n.add(l.base.id());
        }
        box_names(l, n);
    }
}

fn component_names(list: &[Component], n: &mut Names) {
    for c in list {
        n.add(c.base.id());
        component_names(&c.children, n);
    }
}

/// The names FEEL resolves in this document: the built-in functions, every element of
/// the graph, types and their components, entries, parameters and table columns.
pub fn names(doc: &Document) -> Names {
    let mut n = Names::builtins();
    for (b, _) in elements(doc) {
        n.add(&var_name(b));
        n.add(b.id());
    }
    for b in doc.items.values() {
        n.add(&var_name(b));
    }
    for list in doc.components.values() {
        component_names(list, &mut n);
    }
    for d in doc.decisions.values().chain(doc.bkms.values()) {
        box_names(d, &mut n);
    }
    n
}

/// The connected decision requirements graphs of the document: lists of element
/// identifiers in document order, the graphs ordered by their first decision.
pub fn graphs(doc: &Document) -> Vec<Vec<String>> {
    let els = elements(doc);
    let ids: Vec<String> = els.iter().map(|(b, _)| b.id().to_string()).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let next = p[c];
            p[c] = r;
            c = next;
        }
        r
    }
    let index = |id: &str| ids.iter().position(|x| x == id);
    for (k, (b, _)) in els.iter().enumerate() {
        let mut links = requires(b);
        if let Some(d) = doc.decisions.get(b.id()).or_else(|| doc.bkms.get(b.id())) {
            invoked(d, &mut links);
        }
        for r in links {
            if let Some(j) = index(&r) {
                let (a, c) = (find(&mut parent, k), find(&mut parent, j));
                parent[a] = c;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<String>)> = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        let root = find(&mut parent, k);
        match groups.iter_mut().find(|g| g.0 == root) {
            Some(g) => g.1.push(id.clone()),
            None => groups.push((root, vec![id.clone()])),
        }
    }
    let first_decision = |g: &Vec<String>| {
        g.iter()
            .filter_map(|id| doc.decisions.get_index_of(id))
            .min()
            .unwrap_or(usize::MAX)
    };
    let mut out: Vec<Vec<String>> = groups.into_iter().map(|g| g.1).collect();
    out.sort_by_key(|g| {
        (
            first_decision(g),
            g.first().and_then(|id| ids.iter().position(|x| x == id)),
        )
    });
    out
}

/// The business knowledge models a box invokes (`invoke=`), recursively.
pub fn invoked(d: &Decision, out: &mut Vec<String>) {
    if let Some(i) = d.base.attr("invoke")
        && !out.iter().any(|x| x == i)
    {
        out.push(i.to_string());
    }
    for l in &d.lines {
        invoked(l, out);
    }
}

/// The identifier a graph's DMN file is named after: its first decision, else its first
/// element.
pub fn graph_name<'g>(doc: &Document, g: &'g [String]) -> &'g str {
    g.iter()
        .filter(|id| doc.decisions.contains_key(id.as_str()))
        .min_by_key(|id| doc.decisions.get_index_of(id.as_str()))
        .or_else(|| g.first())
        .map(String::as_str)
        .unwrap_or("decisions")
}

/// The name of a decision table output for a single-output table without a name.
pub fn output_name(dec: &Decision, k: usize) -> String {
    dec.outputs.get(k).map(|o| o.0.clone()).unwrap_or_default()
}
