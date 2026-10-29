//! Checks of language 0.7: decision requirements graphs (R076, R079, R080), boxed
//! expressions (R077, R078), types (R081), decision tables and relations (D003, D005,
//! D006, D007, D010), FEEL at conformance level 3 (F001, F002) and, in executable
//! documents for Camunda, C130–C132.

use std::collections::{HashMap, HashSet};

use super::{Checker, s};
use crate::diagnostics::Diag;
use crate::drg::{self, Kind, bindings, kind_of, params, requires, var_name};
use crate::feel::{self, FeelError};
use crate::feel3::parse::{Expr, Names, free_names, parse_expression, parse_tests};
use crate::model::*;

/// A check of the attributes of one statement: base, keys, flags, kind.
type AttrCheck<'a> = dyn FnMut(&Base, &[&str], &[&str], &str) + 'a;

fn pl_kind(k: &str) -> &'static str {
    match k {
        "decision" => "decyzja",
        "inputdata" => "dane wejściowe",
        "bkm" => "model wiedzy biznesowej",
        "source" => "źródło wiedzy",
        "entry" => "wpis kontekstu",
        _ => "element",
    }
}

/// A FEEL text and where it stands.
struct Text {
    text: String,
    line: usize,
    tests: bool,
}

fn feel_diag(e: &FeelError, line: usize, id: Option<&str>, text: &str) -> Diag {
    let detail = e.detail().to_string();
    let pl = feel::detail_pl(&detail);
    Diag::new(
        e.code(),
        line,
        id,
        &[
            ("detail", format!("{detail} ({})", text.trim())),
            ("detail_pl", format!("{pl} ({})", text.trim())),
        ],
    )
}

/// Every FEEL text of a box and its lines.
fn texts(b: &Decision, out: &mut Vec<Text>) {
    let line = b.base.line;
    if let Some(e) = b.base.attr("expr") {
        out.push(Text {
            text: e.to_string(),
            line,
            tests: false,
        });
    }
    if let Some(w) = b.base.attr("with")
        && let Ok(bs) = bindings(w)
    {
        for (_, e) in bs {
            out.push(Text {
                text: e,
                line,
                tests: false,
            });
        }
    }
    if let Some(d) = b.base.attr("default") {
        if b.outputs.len() > 1 {
            for (_, e) in bindings(d).unwrap_or_default() {
                out.push(Text {
                    text: e,
                    line,
                    tests: false,
                });
            }
        } else {
            out.push(Text {
                text: d.to_string(),
                line,
                tests: false,
            });
        }
    }
    let hline = b.header_line.unwrap_or(line);
    if b.has_table() {
        if !b.relation {
            for (e, _) in &b.inputs {
                out.push(Text {
                    text: e.clone(),
                    line: hline,
                    tests: false,
                });
            }
        }
        for v in b.input_values.iter().chain(&b.output_values).flatten() {
            out.push(Text {
                text: v.clone(),
                line: hline,
                tests: true,
            });
        }
        for r in &b.rules {
            for c in &r.inputs {
                out.push(Text {
                    text: c.clone(),
                    line: r.line,
                    tests: !b.relation,
                });
            }
            for c in &r.outputs {
                if c.trim() != "-" {
                    out.push(Text {
                        text: c.clone(),
                        line: r.line,
                        tests: false,
                    });
                }
            }
        }
    }
    for l in &b.lines {
        texts(l, out);
    }
}

impl Checker<'_> {
    /// All checks of the statements of language 0.7.
    pub(super) fn layer_07(&mut self, camunda: bool) {
        let names = drg::names(self.doc);
        self.attributes_07();
        self.requirements_07();
        self.cycles_07();
        self.duplicates_07();
        let boxes: Vec<(String, Decision)> = self
            .doc
            .bkms
            .values()
            .chain(self.doc.decisions.values())
            .map(|d| (d.base.id().to_string(), d.clone()))
            .collect();
        for (root, b) in &boxes {
            self.box_07(root, b, true);
            self.feel_07(root, b, &names);
            self.uses_07(root, b, &names);
        }
        self.types_07(&names);
        if camunda && self.doc.executable_any() {
            self.camunda_07(&boxes);
        }
    }

    fn attributes_07(&mut self) {
        let mut found = Vec::new();
        let d = &*self.doc;
        let mut check = |b: &Base, keys: &[&str], flags: &[&str], kind: &str| {
            for k in b.attrs.keys() {
                if !keys.contains(&k.as_str()) {
                    found.push(Diag::new(
                        "R010",
                        b.line,
                        b.id.as_deref(),
                        &[("key", k.clone()), ("kind", kind.to_string())],
                    ));
                }
            }
            for f in &b.flags {
                if !flags.contains(&f.as_str()) {
                    found.push(Diag::new(
                        "R011",
                        b.line,
                        b.id.as_deref(),
                        &[("flag", f.clone()), ("kind", kind.to_string())],
                    ));
                }
            }
        };
        for b in d.inputdata.values() {
            check(b, INPUTDATA_KEYS, &[], "inputdata");
        }
        for b in d.sources.values() {
            check(b, SOURCE_KEYS, &[], "source");
        }
        for b in d.bkms.values() {
            check(&b.base, BKM_KEYS, &[], "bkm");
        }
        fn lines(b: &Decision, check: &mut AttrCheck) {
            for l in &b.lines {
                check(&l.base, BOX_LINE_KEYS, &[], &l.base.kind);
                lines(l, check);
            }
        }
        for b in d.bkms.values().chain(d.decisions.values()) {
            lines(b, &mut check);
        }
        fn comps(list: &[Component], check: &mut AttrCheck) {
            for c in list {
                check(&c.base, COMPONENT_KEYS, ITEM_FLAGS, "item");
                comps(&c.children, check);
            }
        }
        for list in d.components.values() {
            comps(list, &mut check);
        }
        self.diags.extend(found);
    }

    /// `requires=` names elements of the graph of an allowed kind (R002, R003).
    fn requirements_07(&mut self) {
        let mut todo: Vec<(String, usize, String, Kind)> = Vec::new();
        for (b, k) in drg::elements(self.doc) {
            for r in requires(b) {
                todo.push((r, b.line, b.id().to_string(), k));
            }
        }
        for (r, line, _owner, k) in todo {
            let allowed: &[&str] = match k {
                Kind::Decision => &["decision", "inputdata", "bkm", "source"],
                Kind::Bkm => &["bkm", "source"],
                Kind::Source => &["decision", "inputdata", "source"],
                Kind::InputData => &[],
            };
            self.reference(&r, line, "requirement", &|x| allowed.contains(&x));
        }
    }

    /// R076: requirements without cycles.
    fn cycles_07(&mut self) {
        let d = &*self.doc;
        let mut found = Vec::new();
        let edges_of = |id: &str| -> Vec<String> {
            let mut out = drg::base_of(d, id).map(requires).unwrap_or_default();
            if let Some(b) = d.decisions.get(id).or_else(|| d.bkms.get(id)) {
                drg::invoked(b, &mut out);
            }
            out.retain(|x| kind_of(d, x).is_some());
            out
        };
        let elements = drg::elements(d);
        // the requirements of every element reached, once
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        let mut pending: Vec<String> = elements.iter().map(|(b, _)| b.id().to_string()).collect();
        while let Some(id) = pending.pop() {
            if !edges.contains_key(&id) {
                let out = edges_of(&id);
                pending.extend(out.iter().filter(|n| !edges.contains_key(*n)).cloned());
                edges.insert(id, out);
            }
        }
        let on_cycle = on_cycles(&edges);
        for (b, _) in &elements {
            let start = b.id().to_string();
            if !on_cycle.contains(&start) {
                continue;
            }
            // depth-first search for a path back to `start`; the path of a node is that of
            // the node it was reached from, plus itself
            let mut stack: Vec<String> = vec![start.clone()];
            let mut from: HashMap<String, String> = HashMap::new();
            let mut seen: HashSet<String> = HashSet::new();
            let path_to = |n: &str, from: &HashMap<String, String>| -> Vec<String> {
                let mut p = vec![n.to_string()];
                let mut cur = n;
                while let Some(prev) = from.get(cur) {
                    p.push(prev.clone());
                    cur = prev;
                }
                p.reverse();
                p
            };
            let mut cycle = None;
            while let Some(cur) = stack.pop() {
                for n in &edges[&cur] {
                    if *n == start {
                        let mut p = path_to(&cur, &from);
                        p.push(n.clone());
                        cycle = Some(p);
                        break;
                    }
                    if seen.insert(n.clone()) {
                        from.insert(n.clone(), cur.clone());
                        stack.push(n.clone());
                    }
                }
                if cycle.is_some() {
                    break;
                }
            }
            if let Some(p) = cycle {
                let path = p.join(" -> ");
                found.push(Diag::new(
                    "R076",
                    b.line,
                    Some(&start),
                    &[("id", start.clone()), ("path", path)],
                ));
            }
        }
        self.diags.extend(found);
    }

    /// R080: names of the graph's elements are unique.
    fn duplicates_07(&mut self) {
        let mut found = Vec::new();
        let mut seen: Vec<(String, String)> = Vec::new();
        for (b, k) in drg::elements(self.doc) {
            if k == Kind::Source {
                continue;
            }
            let n = var_name(b);
            if let Some((_, other)) = seen.iter().find(|(x, _)| *x == n) {
                found.push(Diag::new(
                    "R080",
                    b.line,
                    b.id.as_deref(),
                    &[("name", n.clone()), ("a", other.clone()), ("b", s(b.id()))],
                ));
            } else {
                seen.push((n, b.id().to_string()));
            }
        }
        self.diags.extend(found);
    }

    fn r077(&mut self, b: &Base, root: &str, detail: String, detail_pl: String) {
        let line = b.line;
        self.add(
            "R077",
            line,
            Some(root),
            &[("detail", detail), ("detail_pl", detail_pl)],
        );
    }

    /// R077, R078, R080 and the decision table checks of one box.
    fn box_07(&mut self, root: &str, b: &Decision, top: bool) {
        let what = if top {
            root.to_string()
        } else {
            format!("{} of '{root}'", b.base.kind)
        };
        let what_pl = if top {
            root.to_string()
        } else {
            format!("{} w „{root}”", b.base.kind)
        };
        let forms = [
            b.base.attrs.contains_key("expr"),
            b.base.attrs.contains_key("invoke"),
            b.has_table(),
            !b.lines.is_empty(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if forms > 1 {
            self.r077(
                &b.base,
                root,
                format!("'{what}' has more than one form of logic (expr=, invoke=, a table, entries or elements)"),
                format!("„{what_pl}” ma więcej niż jedną postać logiki (expr=, invoke=, tabela, wpisy albo elementy)"),
            );
        }
        if b.base.attrs.contains_key("with") && !b.base.attrs.contains_key("invoke") {
            self.r077(
                &b.base,
                root,
                format!("with= on '{what}' requires invoke="),
                format!("with= w „{what_pl}” wymaga invoke="),
            );
        }
        if ["hit", "aggregation", "default"]
            .iter()
            .any(|k| b.base.attrs.contains_key(*k))
            && !(b.has_table() && !b.relation)
        {
            self.r077(
                &b.base,
                root,
                format!("hit=, aggregation= and default= on '{what}' need a decision table"),
                format!("hit=, aggregation= i default= w „{what_pl}” wymagają tabeli decyzyjnej"),
            );
        }
        let elements = b.lines.iter().filter(|l| l.base.kind == "element").count();
        if elements > 0 && elements < b.lines.len() {
            self.r077(
                &b.base,
                root,
                format!("the block of '{what}' mixes element lines with entry and result lines"),
                format!("blok „{what_pl}” łączy wiersze element z wierszami entry i result"),
            );
        }
        let results: Vec<usize> = b
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.base.kind == "result")
            .map(|(k, _)| k)
            .collect();
        if results.len() > 1 || results.first().is_some_and(|k| *k + 1 != b.lines.len()) {
            self.r077(
                &b.base,
                root,
                format!("'result' must be the last line of the context of '{what}', and only one"),
                format!(
                    "„result” musi być ostatnim i jedynym takim wierszem kontekstu „{what_pl}”"
                ),
            );
        }
        // parameters of functions
        if let Some(p) = b.base.attr("params") {
            match params(p) {
                Err(item) => self.r078_list(&b.base, root, "params", &item),
                Ok(ps) => {
                    let mut seen: Vec<String> = Vec::new();
                    for (n, _) in ps {
                        if seen.contains(&n) {
                            self.add(
                                "R080",
                                b.base.line,
                                Some(root),
                                &[("name", n.clone()), ("a", s(root)), ("b", s(root))],
                            );
                        }
                        seen.push(n);
                    }
                }
            }
        }
        // invocation
        if let Some(i) = b.base.attr("invoke") {
            let i = i.to_string();
            if self
                .reference(&i, b.base.line, "bkm", &|k| k == "bkm")
                .is_some()
                && let Some(w) = b.base.attr("with")
            {
                match bindings(w) {
                    Err(item) => self.r078_list(&b.base, root, "with", &item),
                    Ok(bs) => {
                        let declared: Vec<String> = self
                            .doc
                            .bkms
                            .get(&i)
                            .and_then(|k| k.base.attr("params"))
                            .and_then(|p| params(p).ok())
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(n, _)| n)
                            .collect();
                        for (p, _) in bs {
                            if !declared.contains(&p) {
                                self.add(
                                    "R078",
                                    b.base.line,
                                    Some(root),
                                    &[
                                        (
                                            "detail",
                                            format!("with= names '{p}', which is not a parameter of '{i}'"),
                                        ),
                                        (
                                            "detail_pl",
                                            format!("with= wskazuje „{p}”, który nie jest parametrem „{i}”"),
                                        ),
                                    ],
                                );
                            }
                        }
                    }
                }
            }
        }
        // entries of one context have different names
        let mut seen: Vec<String> = Vec::new();
        for l in &b.lines {
            if l.base.kind == "entry" {
                let n = drg::normalize(l.base.id());
                if seen.contains(&n) {
                    self.add(
                        "R080",
                        l.base.line,
                        Some(root),
                        &[("name", n.clone()), ("a", s(root)), ("b", s(root))],
                    );
                }
                seen.push(n);
            }
        }
        if b.has_table() {
            self.table_07(root, b);
        }
        for l in &b.lines {
            self.box_07(root, l, false);
        }
    }

    fn r078_list(&mut self, b: &Base, root: &str, key: &str, item: &str) {
        self.add(
            "R078",
            b.line,
            Some(root),
            &[
                (
                    "detail",
                    format!("malformed {key}=\"…\" item '{item}' on '{root}'"),
                ),
                (
                    "detail_pl",
                    format!("niepoprawny element listy {key}=\"…\": „{item}” w „{root}”"),
                ),
            ],
        );
    }

    fn table_07(&mut self, root: &str, b: &Decision) {
        let line = b.base.line;
        if b.relation {
            return;
        }
        let hit = b.hit();
        if !HIT_POLICIES_07.contains(&hit) {
            self.add("D003", line, Some(root), &[("hit", s(hit))]);
        }
        if b.rules.is_empty() {
            self.add("D005", line, Some(root), &[("id", s(root))]);
        }
        if ["priority", "output-order"].contains(&hit) {
            // the output values may also come from the allowed values of an output's type
            let has_values = b.outputs.iter().enumerate().any(|(k, (_, typ))| {
                b.output_values.get(k).is_some_and(Option::is_some)
                    || self.doc.items.values().any(|it| {
                        (it.id() == typ || var_name(it) == *typ) && it.attrs.contains_key("allowed")
                    })
            });
            if !has_values {
                self.add(
                    "D006",
                    b.header_line.unwrap_or(line),
                    Some(root),
                    &[("hit", s(hit))],
                );
            }
        }
        if let Some(a) = b.base.attr("aggregation") {
            let (en, pl) = if hit != "collect" {
                (
                    format!("aggregation={a} requires hit=collect"),
                    format!("aggregation={a} wymaga hit=collect"),
                )
            } else if !AGGREGATIONS.contains(&a) {
                (
                    format!("unknown aggregation '{a}' (sum, count, min, max)"),
                    format!("nieznana agregacja „{a}” (sum, count, min, max)"),
                )
            } else if b.outputs.len() != 1 {
                (
                    format!("aggregation={a} needs a table with one output"),
                    format!("aggregation={a} wymaga tabeli z jednym wynikiem"),
                )
            } else {
                (String::new(), String::new())
            };
            if !en.is_empty() {
                self.add(
                    "D007",
                    line,
                    Some(root),
                    &[("detail", en), ("detail_pl", pl)],
                );
            }
        }
        if hit == "unique" && !b.inputs.is_empty() {
            for (code, l, args) in crate::dmn::check_decision(b) {
                if code == "D010" {
                    self.add(code, l, Some(root), &args);
                }
            }
        }
    }

    fn feel_07(&mut self, root: &str, b: &Decision, names: &Names) {
        let mut ts = Vec::new();
        texts(b, &mut ts);
        let mut found = Vec::new();
        for t in ts {
            let r = if t.tests {
                parse_tests(&t.text, names).err()
            } else {
                parse_expression(&t.text, names).err()
            };
            if let Some(e) = r {
                found.push(feel_diag(&e, t.line, Some(root), &t.text));
            }
        }
        self.diags.extend(found);
    }

    /// R079: a decision or business knowledge model uses the name of an element it does
    /// not require.
    fn uses_07(&mut self, root: &str, b: &Decision, names: &Names) {
        let d = &*self.doc;
        let Some(kind) = kind_of(d, root) else { return };
        let mut ts = Vec::new();
        texts(b, &mut ts);
        let mut used: Vec<String> = Vec::new();
        for t in &ts {
            let e: Option<Expr> = if t.tests {
                parse_tests(&t.text, names).ok().map(|x| match x {
                    crate::feel3::Tests::Any => Expr::Null,
                    crate::feel3::Tests::Some(v) | crate::feel3::Tests::Not(v) => Expr::List(v),
                })
            } else {
                parse_expression(&t.text, names).ok()
            };
            if let Some(e) = e {
                free_names(&e, &mut used);
            }
        }
        // names bound inside the box: parameters and context entries
        fn bound(b: &Decision, out: &mut Vec<String>) {
            if let Some(p) = b.base.attr("params")
                && let Ok(ps) = params(p)
            {
                out.extend(ps.into_iter().map(|(n, _)| n));
            }
            for l in &b.lines {
                if l.base.kind == "entry" {
                    out.push(drg::normalize(l.base.id()));
                }
                bound(l, out);
            }
        }
        let mut local = Vec::new();
        bound(b, &mut local);
        used.retain(|n| !local.contains(n));
        let reqs = requires(&b.base);
        let mut invoked = Vec::new();
        drg::invoked(b, &mut invoked);
        let required_names: Vec<String> = reqs
            .iter()
            .filter_map(|r| drg::base_of(d, r))
            .map(var_name)
            .collect();
        let mut found = Vec::new();
        for (other, k) in drg::elements(d) {
            if other.id() == root || k == Kind::Source {
                continue;
            }
            let n = var_name(other);
            let in_text = (used.contains(&n) && !required_names.contains(&n))
                || (k == Kind::Bkm && invoked.iter().any(|i| i == other.id()));
            if in_text && !reqs.iter().any(|r| r == other.id()) {
                let _ = kind;
                found.push(Diag::new(
                    "R079",
                    b.base.line,
                    Some(root),
                    &[
                        ("id", s(root)),
                        ("name", n),
                        ("kind", s(k.word())),
                        ("kind_pl", s(pl_kind(k.word()))),
                        ("other", s(other.id())),
                    ],
                ));
            }
        }
        self.diags.extend(found);
    }

    /// R081: types of `type=`, parameters and table columns.
    fn types_07(&mut self, _names: &Names) {
        let d = &*self.doc;
        let mut found = Vec::new();
        let mut type_ref = |t: &str, line: usize, id: Option<&str>| {
            if !drg::known_type(d, t) {
                found.push(Diag::new(
                    "R081",
                    line,
                    id,
                    &[
                        (
                            "detail",
                            format!("unknown type '{t}' (a FEEL type or an item of the document)"),
                        ),
                        (
                            "detail_pl",
                            format!("nieznany typ „{t}” (typ FEEL albo item dokumentu)"),
                        ),
                    ],
                ));
            }
        };
        for (b, _) in drg::elements(d) {
            if let Some(t) = b.attr("type") {
                type_ref(t, b.line, b.id.as_deref());
            }
            if let Some(p) = b.attr("params")
                && let Ok(ps) = params(p)
            {
                for (_, t) in ps {
                    if let Some(t) = t {
                        type_ref(&t, b.line, b.id.as_deref());
                    }
                }
            }
        }
        fn lines(b: &Decision, root: &str, f: &mut dyn FnMut(&str, usize, Option<&str>)) {
            for (_, t) in b.inputs.iter().chain(&b.outputs) {
                f(t, b.header_line.unwrap_or(b.base.line), Some(root));
            }
            for l in &b.lines {
                if let Some(t) = l.base.attr("type") {
                    f(t, l.base.line, Some(root));
                }
                if let Some(p) = l.base.attr("params")
                    && let Ok(ps) = params(p)
                {
                    for t in ps.into_iter().filter_map(|(_, t)| t) {
                        f(&t, l.base.line, Some(root));
                    }
                }
                lines(l, root, f);
            }
        }
        for b in d.bkms.values().chain(d.decisions.values()) {
            lines(b, b.base.id(), &mut type_ref);
        }
        fn comps(list: &[Component], root: &str, f: &mut dyn FnMut(&str, usize, Option<&str>)) {
            for c in list {
                if let Some(t) = c.base.attr("type") {
                    f(t, c.base.line, Some(root));
                }
                comps(&c.children, root, f);
            }
        }
        for (id, list) in &d.components {
            comps(list, id, &mut type_ref);
        }
        let mut conflicts = Vec::new();
        for it in d.items.values() {
            if let Some(t) = it.attr("type") {
                type_ref(t, it.line, it.id.as_deref());
            }
            if drg::is_dmn_item(d, it)
                && (it.attrs.contains_key("structure") || it.attrs.contains_key("kind"))
            {
                conflicts.push(Diag::new(
                    "R081",
                    it.line,
                    it.id.as_deref(),
                    &[
                        (
                            "detail",
                            format!("item '{}' is both a BPMN item (structure=, kind=) and a DMN type (type=, allowed=, components)", it.id()),
                        ),
                        (
                            "detail_pl",
                            format!("item „{}” jest jednocześnie typem BPMN (structure=, kind=) i typem DMN (type=, allowed=, składowe)", it.id()),
                        ),
                    ],
                ));
            }
        }
        found.extend(conflicts);
        // allowed values are unary tests
        let names = drg::names(d);
        let mut allowed: Vec<(String, usize, Option<String>)> = Vec::new();
        for it in d.items.values() {
            if let Some(a) = it.attr("allowed") {
                allowed.push((a.to_string(), it.line, it.id.clone()));
            }
        }
        fn comp_allowed(
            list: &[Component],
            root: &str,
            out: &mut Vec<(String, usize, Option<String>)>,
        ) {
            for c in list {
                if let Some(a) = c.base.attr("allowed") {
                    out.push((a.to_string(), c.base.line, Some(root.to_string())));
                }
                comp_allowed(&c.children, root, out);
            }
        }
        for (id, list) in &d.components {
            comp_allowed(list, id, &mut allowed);
        }
        for (a, line, id) in allowed {
            if let Err(e) = parse_tests(&a, &names) {
                found.push(feel_diag(&e, line, id.as_deref(), &a));
            }
        }
        self.diags.extend(found);
    }

    fn camunda_07(&mut self, boxes: &[(String, Decision)]) {
        let mut found = Vec::new();
        for b in self.doc.bkms.values() {
            if b.base.attrs.contains_key("invoke") {
                found.push(Diag::new(
                    "C130",
                    b.base.line,
                    b.base.id.as_deref(),
                    &[("id", s(b.base.id()))],
                ));
            }
        }
        fn functions(root: &str, b: &Decision, found: &mut Vec<Diag>) {
            for l in &b.lines {
                if l.base.attrs.contains_key("params") && !l.base.attrs.contains_key("expr") {
                    found.push(Diag::new(
                        "C131",
                        l.base.line,
                        Some(root),
                        &[("id", s(root))],
                    ));
                }
                functions(root, l, found);
            }
        }
        for (root, b) in boxes {
            functions(root, b, &mut found);
        }
        for d in self.doc.decisions.values() {
            let has_logic = d.base.attrs.contains_key("expr")
                || d.base.attrs.contains_key("invoke")
                || d.has_table()
                || !d.lines.is_empty();
            if !has_logic {
                found.push(Diag::new(
                    "C132",
                    d.base.line,
                    d.base.id.as_deref(),
                    &[("id", s(d.base.id()))],
                ));
            }
        }
        self.diags.extend(found);
    }
}

/// The nodes of a graph that lie on a cycle: in a strongly connected component of two or
/// more nodes, or with an edge to themselves (Tarjan's algorithm, without recursion).
fn on_cycles(edges: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut low: HashMap<&str, usize> = HashMap::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut out = HashSet::new();
    let mut nodes: Vec<&str> = edges.keys().map(String::as_str).collect();
    nodes.sort_unstable();
    for root in nodes {
        if index.contains_key(root) {
            continue;
        }
        // (node, position of the next edge to follow)
        let mut work: Vec<(&str, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut next)) = work.last_mut() {
            if *next == 0 && !index.contains_key(v) {
                let i = index.len();
                index.insert(v, i);
                low.insert(v, i);
                stack.push(v);
                on_stack.insert(v);
            }
            let succ = edges.get(v).map_or(&[][..], Vec::as_slice);
            if let Some(w) = succ.get(*next) {
                *next += 1;
                let w = w.as_str();
                if !index.contains_key(w) {
                    work.push((w, 0));
                } else if on_stack.contains(w) {
                    let m = low[v].min(index[w]);
                    low.insert(v, m);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                let m = low[parent].min(low[v]);
                low.insert(parent, m);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack.remove(w);
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                let self_edge = succ.iter().any(|w| w == v);
                if component.len() > 1 || self_edge {
                    out.extend(component.into_iter().map(str::to_string));
                }
            }
        }
    }
    out
}
