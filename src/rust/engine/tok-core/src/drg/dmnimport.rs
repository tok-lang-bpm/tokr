//! Import of DMN 1.1–1.5 XML into a document of language 0.7 (the inverse of `export`):
//! decisions with their logic, input data, business knowledge models, knowledge sources,
//! requirements and item definitions. What tok does not keep is reported.

use std::collections::{HashMap, HashSet};

use roxmltree::Node as X;

use crate::import::{ImportError, slug};
use crate::model::*;
use crate::parser::is_id;

fn local<'a>(e: X<'a, '_>) -> &'a str {
    e.tag_name().name()
}

fn kids<'a, 'i>(e: X<'a, 'i>) -> impl Iterator<Item = X<'a, 'i>> {
    e.children().filter(|c| c.is_element())
}

fn kid<'a, 'i>(e: X<'a, 'i>, name: &str) -> Option<X<'a, 'i>> {
    kids(e).find(|c| local(*c) == name)
}

fn text_of(e: X) -> String {
    e.descendants()
        .filter(|n| n.is_text())
        .map(|n| n.text().unwrap_or(""))
        .collect()
}

fn kid_text(e: X, name: &str) -> Option<String> {
    let k = kid(e, name)?;
    // `<text>` inside, or the element's own text
    match kid(k, "text") {
        Some(t) => Some(text_of(t)),
        None => Some(text_of(k)),
    }
}

/// Whether text stands in one cell of a row: no `|` and no `#` outside strings, and its
/// strings closed.
fn cell_text(text: &str) -> bool {
    let mut in_str = false;
    let mut esc = false;
    for c in text.chars() {
        if esc {
            esc = false;
        } else if in_str {
            match c {
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '|' | '#' => return false,
                _ => {}
            }
        }
    }
    !in_str && !text.trim().is_empty()
}

/// The first header or row cell of an imported table or relation that tok cannot write
/// so that it reads back the same: such a table is not kept.
fn unwritable(d: &Decision) -> Option<String> {
    let header = d
        .inputs
        .iter()
        .zip(d.input_values.iter().chain(std::iter::repeat(&None)))
        .chain(
            d.outputs
                .iter()
                .zip(d.output_values.iter().chain(std::iter::repeat(&None))),
        );
    for ((name, typ), values) in header {
        let cell = match values {
            Some(v) => format!("{name}: {typ} = {v}"),
            None => format!("{name}: {typ}"),
        };
        let back = crate::parser::split_header_cell_07(&cell);
        let same = back.as_ref().is_some_and(|(n, t, v)| {
            n == name.trim() && t == typ.trim() && v.as_deref() == values.as_deref().map(str::trim)
        });
        if !same || !cell_text(&cell) {
            return Some(cell);
        }
    }
    let rows = d
        .rules
        .iter()
        .flat_map(|r| r.inputs.iter().chain(r.outputs.iter()));
    let notes = d.rules.iter().filter_map(|r| r.annotation.as_ref());
    rows.chain(notes)
        .chain(d.annotation_title.iter())
        .find(|c| !cell_text(c))
        .cloned()
}

/// FEEL text on one line (for table cells): line breaks outside strings become spaces,
/// inside strings the escape `\n`.
fn one_line(text: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut esc = false;
    for c in text.trim().chars() {
        if in_str {
            if esc {
                esc = false;
                out.push(c);
                continue;
            }
            match c {
                '\\' => {
                    esc = true;
                    out.push(c);
                }
                '"' => {
                    in_str = false;
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\r' => {}
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        } else {
            match c {
                '"' => {
                    in_str = true;
                    out.push(c);
                }
                '\n' | '\r' | '\t' => {
                    if !out.ends_with(' ') {
                        out.push(' ');
                    }
                }
                c => out.push(c),
            }
        }
    }
    out
}

const EXPRESSIONS: &[&str] = &[
    "literalExpression",
    "decisionTable",
    "context",
    "invocation",
    "list",
    "relation",
    "functionDefinition",
    "conditional",
    "filter",
    "for",
    "some",
    "every",
];

fn expression_of<'a, 'i>(e: X<'a, 'i>) -> Option<X<'a, 'i>> {
    kids(e).find(|c| EXPRESSIONS.contains(&local(*c)))
}

struct Importer {
    warnings: Vec<String>,
    ids: HashMap<String, String>,
    used: HashSet<String>,
    /// FEEL names of business knowledge models → tok identifiers.
    bkm_ids: HashMap<String, String>,
    line: usize,
    namespace: Option<String>,
}

fn type_attr(e: X) -> Option<String> {
    e.attribute("typeRef")
        .map(|t| t.strip_prefix("feel:").unwrap_or(t).to_string())
        .filter(|t| !t.is_empty())
}

impl Importer {
    fn warn(&mut self, id: &str, text: &str) {
        let id = if id.is_empty() { "-" } else { id };
        self.warnings.push(format!("{id}: {text}"));
    }

    fn next_line(&mut self) -> usize {
        self.line += 1;
        self.line
    }

    fn tid(&mut self, x: &str) -> String {
        if let Some(v) = self.ids.get(x) {
            return v.clone();
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
            self.warn(x, &msg);
        }
        self.ids.insert(x.to_string(), new.clone());
        self.used.insert(new.clone());
        new
    }

    fn href(&mut self, e: X, owner: &str) -> Option<String> {
        let h = e.attribute("href")?;
        // a reference into this model by its namespace is local
        let h = match (h.split_once('#'), &self.namespace) {
            (Some((ns, id)), Some(own)) if ns == own => format!("#{id}"),
            _ => h.to_string(),
        };
        match h.strip_prefix('#') {
            Some(id) => Some(self.ids.get(id).cloned().unwrap_or_else(|| id.to_string())),
            None => {
                let msg = format!("requirement of an element of another model ({h}) is not kept");
                self.warn(owner, &msg);
                None
            }
        }
    }

    fn base(&mut self, e: X, kind: &str) -> Base {
        let xid = e
            .attribute("id")
            .or_else(|| e.attribute("name"))
            .unwrap_or(kind)
            .to_string();
        let id = self.tid(&xid);
        let mut attrs = Attrs::new();
        if let Some(n) = e.attribute("name").map(super::normalize)
            && n != id
            && !n.is_empty()
        {
            attrs.insert("name".into(), n);
        }
        if let Some(d) = kid(e, "description")
            .map(text_of)
            .filter(|d| !d.trim().is_empty())
        {
            attrs.insert("doc".into(), d.trim().to_string());
        }
        let label = e
            .attribute("label")
            .map(super::normalize)
            .filter(|l| !l.is_empty());
        let line = self.next_line();
        Base::new(kind, Some(id), label, attrs, Vec::new(), line)
    }

    fn requirements(&mut self, e: X, b: &mut Base) {
        let mut reqs: Vec<String> = Vec::new();
        let owner = b.id().to_string();
        for r in kids(e) {
            let child = match local(r) {
                "informationRequirement" | "knowledgeRequirement" | "authorityRequirement" => r,
                _ => continue,
            };
            for c in kids(child) {
                if matches!(
                    local(c),
                    "requiredDecision"
                        | "requiredInput"
                        | "requiredKnowledge"
                        | "requiredAuthority"
                ) && let Some(id) = self.href(c, &owner)
                    && !reqs.contains(&id)
                {
                    reqs.push(id);
                }
            }
        }
        if !reqs.is_empty() {
            b.attrs.insert("requires".into(), reqs.join(" "));
        }
    }

    /// Fill `d` with the logic of the boxed expression `e`.
    fn boxed(&mut self, e: X, d: &mut Decision, owner: &str, default_name: &str) {
        let kind = local(e);
        if kind != "functionDefinition"
            && !d.base.attrs.contains_key("type")
            && d.base.kind != "decision"
            && let Some(t) = type_attr(e)
        {
            d.base.attrs.insert("type".into(), t);
        }
        match kind {
            "literalExpression" => {
                if let Some(t) = kid(e, "text").map(text_of)
                    && !t.trim().is_empty()
                {
                    d.base.attrs.insert("expr".into(), t.trim().to_string());
                }
                if let Some(i) = kid(e, "importedValues") {
                    let _ = i;
                    self.warn(
                        owner,
                        "imported values of a literal expression are not kept",
                    );
                }
            }
            "decisionTable" | "relation" => {
                let saved = d.clone();
                if kind == "decisionTable" {
                    self.table(e, d, owner, default_name);
                } else {
                    self.relation(e, d, owner);
                }
                if let Some(cell) = unwritable(d) {
                    *d = saved;
                    let what = if kind == "relation" {
                        "relation"
                    } else {
                        "decision table"
                    };
                    let msg = format!(
                        "the {what} is not kept: tok cannot write the cell '{cell}' in a row"
                    );
                    self.warn(owner, &msg);
                }
            }
            "context" => {
                for ce in kids(e).filter(|c| local(*c) == "contextEntry") {
                    let var = kid(ce, "variable");
                    let line = self.next_line();
                    let mut attrs = Attrs::new();
                    let (k, name) = match var {
                        Some(v) => {
                            if let Some(t) = type_attr(v) {
                                attrs.insert("type".into(), t);
                            }
                            ("entry", v.attribute("name").map(super::normalize))
                        }
                        None => ("result", None),
                    };
                    let mut line_box = Decision {
                        base: Base::new(k, name.clone(), None, attrs, Vec::new(), line),
                        ..Default::default()
                    };
                    if let Some(x) = expression_of(ce) {
                        let dn = name.clone().unwrap_or_else(|| default_name.to_string());
                        self.boxed(x, &mut line_box, owner, &dn);
                    }
                    d.lines.push(line_box);
                }
            }
            "list" => {
                for x in kids(e).filter(|c| EXPRESSIONS.contains(&local(*c))) {
                    let line = self.next_line();
                    let mut el = Decision {
                        base: Base::new("element", None, None, Attrs::new(), Vec::new(), line),
                        ..Default::default()
                    };
                    self.boxed(x, &mut el, owner, default_name);
                    d.lines.push(el);
                }
            }
            "invocation" => {
                let callee = kid(e, "literalExpression")
                    .and_then(|l| kid(l, "text"))
                    .map(text_of)
                    .map(|t| super::normalize(&t))
                    .unwrap_or_default();
                let target = self.bkm_ids.get(&callee).cloned().unwrap_or_else(|| {
                    if is_id(&callee) {
                        callee.clone()
                    } else {
                        slug(&callee)
                    }
                });
                d.base.attrs.insert("invoke".into(), target);
                let mut pairs = Vec::new();
                for b in kids(e).filter(|c| local(*c) == "binding") {
                    let Some(p) = kid(b, "parameter").and_then(|p| p.attribute("name")) else {
                        continue;
                    };
                    let p = super::normalize(p);
                    match expression_of(b) {
                        Some(x) if local(x) == "literalExpression" => {
                            let t = kid(x, "text").map(text_of).unwrap_or_default();
                            let t = if t.trim().is_empty() {
                                "null".into()
                            } else {
                                one_line(&t)
                            };
                            pairs.push(format!("{p} = {t}"));
                        }
                        Some(_) => {
                            let msg = format!(
                                "binding '{p}' that is not a literal expression is not kept"
                            );
                            self.warn(owner, &msg);
                        }
                        None => pairs.push(format!("{p} = null")),
                    }
                }
                if !pairs.is_empty() {
                    d.base.attrs.insert("with".into(), pairs.join("; "));
                }
            }
            "functionDefinition"
                if matches!(d.base.kind.as_str(), "decision" | "bkm")
                    || d.base.attrs.contains_key("params") =>
            {
                // the logic of a decision or a business knowledge model is a function: a
                // context whose only line is a result with parameters
                let line = self.next_line();
                let mut result = Decision {
                    base: Base::new("result", None, None, Attrs::new(), Vec::new(), line),
                    ..Default::default()
                };
                self.boxed(e, &mut result, owner, default_name);
                d.lines.push(result);
            }
            "functionDefinition" => {
                if let Some(k) = e.attribute("kind").filter(|k| *k != "FEEL") {
                    let msg = format!("function definition of kind {k} is not kept");
                    self.warn(owner, &msg);
                    return;
                }
                let ps = params_of(e);
                d.base.attrs.insert("params".into(), ps);
                if let Some(t) = type_attr(e) {
                    d.base.attrs.insert("type".into(), t);
                }
                if let Some(x) = expression_of(e) {
                    self.boxed(x, d, owner, default_name);
                }
            }
            "conditional" | "filter" | "for" | "some" | "every" => {
                let part = |name: &str| -> Option<String> {
                    let p = kid(e, name)?;
                    let x = expression_of(p)?;
                    if local(x) != "literalExpression" {
                        return None;
                    }
                    Some(
                        kid(x, "text")
                            .map(text_of)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                    )
                };
                let var = e.attribute("iteratorVariable").unwrap_or("item");
                let text =
                    match kind {
                        "conditional" => part("if").zip(part("then")).zip(part("else")).map(
                            |((c, a), b)| {
                                format!(
                                    "if ({c}) then ({a}) else if ({c}) = false then ({b}) else null"
                                )
                            },
                        ),
                        "filter" => part("in")
                            .zip(part("match"))
                            .map(|(i, m)| format!("({i})[{m}]")),
                        "for" => part("in")
                            .zip(part("return"))
                            .map(|(i, r)| format!("for {var} in {i} return ({r})")),
                        _ => part("in")
                            .zip(part("satisfies"))
                            .map(|(i, s)| format!("{kind} {var} in {i} satisfies ({s})")),
                    };
                match text {
                    Some(t) => {
                        d.base.attrs.insert("expr".into(), t);
                    }
                    None => {
                        let msg = format!(
                            "boxed {kind} with parts that are not literal expressions is not kept"
                        );
                        self.warn(owner, &msg);
                    }
                }
            }
            other => {
                let msg = format!("expression {other} is not kept");
                self.warn(owner, &msg);
            }
        }
    }

    fn relation(&mut self, e: X, d: &mut Decision, owner: &str) {
        d.relation = true;
        d.header_done = true;
        for c in kids(e).filter(|c| local(*c) == "column") {
            let name = c
                .attribute("name")
                .map(super::normalize)
                .unwrap_or_default();
            d.inputs
                .push((name, type_attr(c).unwrap_or_else(|| "any".into())));
            d.input_values.push(None);
        }
        for r in kids(e).filter(|c| local(*c) == "row") {
            let mut cells = Vec::new();
            for x in kids(r) {
                if local(x) == "literalExpression" {
                    let t = kid(x, "text").map(text_of).unwrap_or_default();
                    cells.push(if t.trim().is_empty() {
                        "null".to_string()
                    } else {
                        one_line(&t)
                    });
                } else {
                    self.warn(
                        owner,
                        "a relation cell that is not a literal expression is not kept",
                    );
                    cells.push("null".into());
                }
            }
            let line = self.next_line();
            d.rules.push(Rule {
                inputs: cells,
                line,
                ..Default::default()
            });
        }
    }

    fn table(&mut self, e: X, d: &mut Decision, owner: &str, default_name: &str) {
        d.header_done = true;
        if let Some(h) = e.attribute("hitPolicy") {
            let h = h.to_lowercase().replace(' ', "-");
            if h != "unique" {
                d.base.attrs.insert("hit".into(), h);
            }
        }
        if let Some(a) = e.attribute("aggregation") {
            d.base.attrs.insert("aggregation".into(), a.to_lowercase());
        }
        if let Some(o) = e
            .attribute("preferredOrientation")
            .filter(|o| *o != "Rule-as-Row")
        {
            let msg = format!("orientation {o} of the decision table is not kept");
            self.warn(owner, &msg);
        }
        for i in kids(e).filter(|c| local(*c) == "input") {
            let ie = kid(i, "inputExpression");
            let expr = ie
                .and_then(|x| kid(x, "text"))
                .map(text_of)
                .unwrap_or_default();
            let typ = ie.and_then(type_attr).unwrap_or_else(|| "any".into());
            d.inputs.push((one_line(&expr), typ));
            d.input_values.push(
                kid_text(i, "inputValues")
                    .map(|t| one_line(&t))
                    .filter(|t| !t.is_empty()),
            );
        }
        let outputs: Vec<X> = kids(e).filter(|c| local(*c) == "output").collect();
        let single = outputs.len() == 1;
        d.no_inputs = d.inputs.is_empty();
        d.no_outputs = outputs.is_empty();
        let mut defaults: Vec<String> = Vec::new();
        for o in &outputs {
            let name = o
                .attribute("name")
                .map(super::normalize)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| {
                    if single {
                        default_name.to_string()
                    } else {
                        "output".into()
                    }
                });
            d.outputs
                .push((name, type_attr(*o).unwrap_or_else(|| "any".into())));
            d.output_values.push(
                kid_text(*o, "outputValues")
                    .map(|t| one_line(&t))
                    .filter(|t| !t.is_empty()),
            );
            if let Some(t) = kid_text(*o, "defaultOutputEntry").filter(|t| !t.trim().is_empty()) {
                let t = one_line(&t);
                if single {
                    defaults.push(t);
                } else {
                    defaults.push(format!(
                        "{} = {t}",
                        d.outputs.last().map(|x| x.0.clone()).unwrap_or_default()
                    ));
                }
            }
        }
        if !defaults.is_empty() {
            d.base.attrs.insert("default".into(), defaults.join("; "));
        }
        let anns: Vec<String> = kids(e)
            .filter(|c| local(*c) == "annotation")
            .map(|a| a.attribute("name").unwrap_or("").to_string())
            .collect();
        if !anns.is_empty() {
            d.has_annotations = true;
            d.annotation_title = Some(anns.join(" "));
        }
        for r in kids(e).filter(|c| local(*c) == "rule") {
            let cell = |x: X| -> String {
                let t = kid(x, "text").map(text_of).unwrap_or_default();
                if t.trim().is_empty() {
                    "-".to_string()
                } else {
                    one_line(&t)
                }
            };
            let inputs: Vec<String> = kids(r)
                .filter(|c| local(*c) == "inputEntry")
                .map(cell)
                .collect();
            let outputs: Vec<String> = kids(r)
                .filter(|c| local(*c) == "outputEntry")
                .map(cell)
                .collect();
            let mut ann: Vec<String> = Vec::new();
            if let Some(dsc) = kid(r, "description")
                .map(text_of)
                .filter(|t| !t.trim().is_empty())
            {
                ann.push(super::normalize(&dsc));
            }
            for a in kids(r).filter(|c| local(*c) == "annotationEntry") {
                let t = kid(a, "text").map(text_of).unwrap_or_default();
                if !t.trim().is_empty() {
                    ann.push(super::normalize(&t));
                }
            }
            let line = self.next_line();
            d.rules.push(Rule {
                inputs,
                outputs,
                annotation: (!ann.is_empty()).then(|| ann.join(" ")),
                line,
                ..Default::default()
            });
        }
    }

    fn components(&mut self, e: X) -> Vec<Component> {
        let mut out = Vec::new();
        for c in kids(e).filter(|c| local(*c) == "itemComponent") {
            let name = c
                .attribute("name")
                .map(super::normalize)
                .unwrap_or_default();
            let mut attrs = Attrs::new();
            let mut flags = Vec::new();
            if let Some(t) = kid(c, "typeRef").map(text_of) {
                let t = t.trim();
                attrs.insert(
                    "type".into(),
                    t.strip_prefix("feel:").unwrap_or(t).to_string(),
                );
            }
            if let Some(a) = kid_text(c, "allowedValues").or_else(|| kid_text(c, "typeConstraint"))
            {
                attrs.insert("allowed".into(), one_line(&a));
            }
            if c.attribute("isCollection") == Some("true") {
                flags.push("collection".to_string());
            }
            let children = self.components(c);
            if attrs.get("type").is_none() && children.is_empty() {
                attrs.insert("type".into(), "Any".into());
            }
            let line = self.next_line();
            out.push(Component {
                base: Base::new("component", Some(name), None, attrs, flags, line),
                children,
            });
        }
        out
    }
}

fn params_of(e: X) -> String {
    kids(e)
        .filter(|c| local(*c) == "formalParameter")
        .map(|p| {
            let n = p
                .attribute("name")
                .map(super::normalize)
                .unwrap_or_default();
            match type_attr(p) {
                Some(t) => format!("{n}: {t}"),
                None => n,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `xml` is a DMN document (its root is `definitions` in a DMN namespace).
pub fn is_dmn(xml: &str) -> bool {
    roxmltree::Document::parse(xml).is_ok_and(|d| {
        let r = d.root_element();
        local(r) == "definitions"
            && r.tag_name()
                .namespace()
                .is_some_and(|ns| ns.contains("DMN") || ns.contains("dmn"))
    })
}

/// Import DMN XML; returns the document and the warnings about what is not kept.
pub fn import_dmn(xml: &str) -> Result<(Document, Vec<String>), ImportError> {
    let tree =
        roxmltree::Document::parse(xml).map_err(|e| ImportError(format!("not a DMN file: {e}")))?;
    if let Some(e) = crate::xml::too_deep(&tree) {
        return Err(ImportError(e));
    }
    let root = tree.root_element();
    if local(root) != "definitions" {
        return Err(ImportError(
            "not a DMN file: the root element is not definitions".into(),
        ));
    }
    let mut im = Importer {
        warnings: Vec::new(),
        ids: HashMap::new(),
        used: HashSet::new(),
        bkm_ids: HashMap::new(),
        line: 1,
        namespace: root.attribute("namespace").map(str::to_string),
    };
    let mut doc = Document {
        version: Some("0.7".into()),
        title: root.attribute("name").map(super::normalize),
        header_line: 1,
        ..Default::default()
    };
    if let Some(ns) = root
        .attribute("namespace")
        .filter(|n| !n.is_empty() && *n != "http://camunda.org/schema/1.0/dmn")
    {
        doc.attrs.insert("namespace".into(), ns.to_string());
    }
    // identifiers first, so that requirements may point forward
    for e in kids(root) {
        if matches!(
            local(e),
            "decision"
                | "inputData"
                | "businessKnowledgeModel"
                | "knowledgeSource"
                | "itemDefinition"
        ) {
            let x = e
                .attribute("id")
                .or_else(|| e.attribute("name"))
                .unwrap_or("element")
                .to_string();
            let id = im.tid(&x);
            if local(e) == "businessKnowledgeModel" {
                let name = e
                    .attribute("name")
                    .map(super::normalize)
                    .unwrap_or_else(|| id.clone());
                im.bkm_ids.insert(name, id);
            }
        }
    }
    for e in kids(root) {
        let owner = e.attribute("id").unwrap_or("").to_string();
        match local(e) {
            "itemDefinition" => {
                let mut b = im.base(e, "item");
                if let Some(t) = kid(e, "typeRef").map(text_of) {
                    let t = t.trim();
                    b.attrs.insert("type".into(), t.strip_prefix("feel:").unwrap_or(t).to_string());
                }
                if let Some(a) = kid_text(e, "allowedValues").or_else(|| kid_text(e, "typeConstraint")) {
                    b.attrs.insert("allowed".into(), one_line(&a));
                }
                if e.attribute("isCollection") == Some("true") {
                    b.flags.push("collection".into());
                }
                if kid(e, "functionItem").is_some() {
                    im.warn(&owner, "function item of an item definition is not kept");
                }
                let comps = im.components(e);
                if b.attrs.get("type").is_none() && comps.is_empty() {
                    b.attrs.insert("type".into(), "Any".into());
                }
                if !comps.is_empty() {
                    doc.components.insert(b.id().to_string(), comps);
                }
                doc.ids.insert(b.id().to_string(), IdEntry { kind: "item".into(), line: b.line });
                doc.items.insert(b.id().to_string(), b);
            }
            "inputData" => {
                let mut b = im.base(e, "inputdata");
                if let Some(t) = kid(e, "variable").and_then(type_attr) {
                    b.attrs.insert("type".into(), t);
                }
                doc.ids.insert(b.id().to_string(), IdEntry { kind: "inputdata".into(), line: b.line });
                doc.inputdata.insert(b.id().to_string(), b);
            }
            "knowledgeSource" => {
                let mut b = im.base(e, "source");
                im.requirements(e, &mut b);
                if let Some(t) = kid(e, "type").map(text_of).filter(|t| !t.trim().is_empty()) {
                    b.attrs.insert("kind".into(), t.trim().to_string());
                }
                if let Some(u) = e.attribute("locationURI") {
                    b.attrs.insert("uri".into(), u.to_string());
                }
                if kid(e, "owner").is_some() {
                    im.warn(&owner, "owner of a knowledge source is not kept");
                }
                doc.ids.insert(b.id().to_string(), IdEntry { kind: "source".into(), line: b.line });
                doc.sources.insert(b.id().to_string(), b);
            }
            "businessKnowledgeModel" => {
                let mut b = im.base(e, "bkm");
                im.requirements(e, &mut b);
                let mut d = Decision::default();
                if let Some(l) = kid(e, "encapsulatedLogic") {
                    b.attrs.insert("params".into(), params_of(l));
                    if let Some(t) = type_attr(l) {
                        b.attrs.insert("type".into(), t);
                    }
                    if let Some(k) = l.attribute("kind").filter(|k| *k != "FEEL") {
                        let msg = format!("encapsulated logic of kind {k} is not kept");
                        im.warn(&owner, &msg);
                    } else if let Some(x) = expression_of(l) {
                        let name = super::var_name(&b);
                        d.base = b.clone();
                        im.boxed(x, &mut d, &owner, &name);
                    }
                } else {
                    b.attrs.insert("params".into(), String::new());
                }
                let mut attrs = std::mem::take(&mut d.base.attrs);
                attrs.extend(b.attrs.clone());
                b.attrs = attrs;
                d.base = b;
                doc.ids.insert(d.base.id().to_string(), IdEntry { kind: "bkm".into(), line: d.base.line });
                doc.bkms.insert(d.base.id().to_string(), d);
            }
            "decision" => {
                let mut b = im.base(e, "decision");
                im.requirements(e, &mut b);
                if let Some(q) = kid(e, "question").map(text_of).filter(|t| !t.trim().is_empty()) {
                    b.attrs.insert("question".into(), super::normalize(&q));
                }
                if let Some(a) = kid(e, "allowedAnswers").map(text_of).filter(|t| !t.trim().is_empty()) {
                    b.attrs.insert("answers".into(), super::normalize(&a));
                }
                if let Some(t) = kid(e, "variable").and_then(type_attr) {
                    b.attrs.insert("type".into(), t);
                }
                for k in ["supportedObjective", "impactedPerformanceIndicator", "decisionMaker", "decisionOwner", "usingProcess", "usingTask"] {
                    if kid(e, k).is_some() {
                        let msg = format!("{k} of a decision is not kept");
                        im.warn(&owner, &msg);
                    }
                }
                let name = super::var_name(&b);
                let mut d = Decision {
                    base: b,
                    ..Default::default()
                };
                if let Some(x) = expression_of(e) {
                    im.boxed(x, &mut d, &owner, &name);
                }
                // the type of a table with one output is the type of that output
                if d.has_table()
                    && !d.relation
                    && d.outputs.len() == 1
                    && d.base.attr("type") == Some(d.outputs[0].1.as_str())
                    && d.hit() != "collect"
                {
                    d.base.attrs.shift_remove("type");
                }
                doc.ids.insert(d.base.id().to_string(), IdEntry { kind: "decision".into(), line: d.base.line });
                doc.decisions.insert(d.base.id().to_string(), d);
            }
            "DMNDI" => im.warn("-", "the diagram (DMNDI) is not kept; tok lays out the decision requirements diagram again"),
            "extensionElements" | "description" => {}
            other => {
                let msg = format!("{other} is not kept");
                im.warn(&owner, &msg);
            }
        }
    }
    Ok((doc, im.warnings))
}
