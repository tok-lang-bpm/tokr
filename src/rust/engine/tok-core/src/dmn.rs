//! Decision tables: checks (D-codes, FEEL in cells, overlap warnings), DMN 1.3 export and
//! evaluation. Mirrors `dmn.py`.

use crate::dec::Dec;
use crate::feel::{self, Ast, Positive, Unary};
use crate::feel_eval::{self as fe, Context, PyValue, feel_error};
use crate::model::{Decision, Document, HIT_POLICIES};
use crate::pyutil::{PyError, strip};
use crate::xml::{DMN_NS, MODELER_NS, XElem};

/// (code, line, arguments)
pub type Check = (&'static str, usize, Vec<(&'static str, String)>);

/// Diagnostics of one decision (`dmn.check_decision`).
pub fn check_decision(d: &Decision) -> Vec<Check> {
    let mut out: Vec<Check> = Vec::new();
    let id = d.base.id().to_string();
    if d.inputs.is_empty() && d.outputs.is_empty() {
        out.push(("D001", d.base.line, vec![("id", id.clone())]));
    }
    if !HIT_POLICIES.contains(&d.hit()) {
        out.push(("D003", d.base.line, vec![("hit", d.hit().to_string())]));
    }
    if d.rules.is_empty() {
        out.push(("D005", d.base.line, vec![("id", id)]));
    }
    let hline = d.header_line.unwrap_or(d.base.line);
    let feel_err =
        |res: Option<(&'static str, String)>, line: usize, cell: &str, out: &mut Vec<Check>| {
            if let Some((code, detail)) = res {
                out.push((
                    code,
                    line,
                    vec![
                        ("detail", format!("{detail} ({cell})")),
                        (
                            "detail_pl",
                            format!("{} ({cell})", feel::detail_pl(&detail)),
                        ),
                    ],
                ));
            }
        };
    for (expr, _) in &d.inputs {
        feel_err(feel::check_expr(expr), hline, expr, &mut out);
    }
    for rule in &d.rules {
        for cell in &rule.inputs {
            feel_err(feel::check_unary(cell), rule.line, cell, &mut out);
        }
        for cell in &rule.outputs {
            if crate::pyutil::strip(cell) == "-" {
                continue;
            }
            feel_err(feel::check_expr(cell), rule.line, cell, &mut out);
        }
    }
    if d.hit() == "unique" && !d.inputs.is_empty() {
        out.extend(overlaps(d));
    }
    out
}

#[derive(Debug, Clone)]
enum Atom {
    Any,
    Str(String),
    Bool(bool),
    Iv(Option<Dec>, bool, Option<Dec>, bool),
}

#[derive(Debug, Clone)]
enum Lit {
    Num(Dec),
    Str(String),
    Bool(bool),
}

fn literal(node: &Ast) -> Option<Lit> {
    match node {
        Ast::Num(t) => Dec::parse(t).map(Lit::Num),
        Ast::Str(s) => Some(Lit::Str(s.clone())),
        Ast::Bool(b) => Some(Lit::Bool(*b)),
        Ast::Neg(inner) => match inner.as_ref() {
            Ast::Num(t) => Dec::parse(t).map(|d| Lit::Num(d.negate())),
            _ => None,
        },
        _ => None,
    }
}

fn atoms(cell: &str) -> Option<Vec<Atom>> {
    let u = feel::parse_unary_tests(cell).ok()?;
    let tests = match u {
        Unary::Any => return Some(vec![Atom::Any]),
        Unary::NotTests(_) => return None,
        Unary::Tests(t) => t,
    };
    let mut out = Vec::new();
    for p in tests {
        match p {
            Positive::Eq(node) => match literal(&node)? {
                Lit::Num(d) => out.push(Atom::Iv(Some(d), true, Some(d), true)),
                Lit::Bool(b) => out.push(Atom::Bool(b)),
                Lit::Str(s) => out.push(Atom::Str(s)),
            },
            Positive::Cmp(op, node) => {
                let Some(Lit::Num(d)) = literal(&node) else {
                    return None;
                };
                out.push(match op.as_str() {
                    "<" => Atom::Iv(None, false, Some(d), false),
                    "<=" => Atom::Iv(None, false, Some(d), true),
                    ">" => Atom::Iv(Some(d), false, None, false),
                    ">=" => Atom::Iv(Some(d), true, None, false),
                    "=" => Atom::Iv(Some(d), true, Some(d), true),
                    _ => return None,
                });
            }
            Positive::Range(lc, lo, hi, hc) => {
                let (Some(Lit::Num(lo)), Some(Lit::Num(hi))) = (literal(&lo), literal(&hi)) else {
                    return None;
                };
                out.push(Atom::Iv(Some(lo), lc, Some(hi), hc));
            }
        }
    }
    Some(out)
}

#[derive(Debug, Clone)]
enum Val {
    Any,
    Null,
    Str(String),
    Bool(bool),
    Num(Dec),
}

fn example(a: &Atom) -> Val {
    match a {
        Atom::Any => Val::Any,
        Atom::Str(s) => Val::Str(s.clone()),
        Atom::Bool(b) => Val::Bool(*b),
        Atom::Iv(lo, loc, hi, hic) => match (lo, hi) {
            (Some(lo), Some(hi)) => {
                if *loc {
                    Val::Num(*lo)
                } else if *hic && hi != lo {
                    Val::Num(*hi)
                } else {
                    lo.plus(*hi)
                        .and_then(Dec::half)
                        .map(Val::Num)
                        .unwrap_or(Val::Num(*lo))
                }
            }
            (Some(lo), None) => {
                if *loc {
                    Val::Num(*lo)
                } else {
                    Val::Num(lo.plus(Dec::int(1)).unwrap_or(*lo))
                }
            }
            (None, Some(hi)) => {
                if *hic {
                    Val::Num(*hi)
                } else {
                    Val::Num(hi.minus(Dec::int(1)).unwrap_or(*hi))
                }
            }
            (None, None) => Val::Num(Dec::int(0)),
        },
    }
}

fn meet(a: &Atom, b: &Atom) -> Option<Val> {
    use std::cmp::Ordering::*;
    match (a, b) {
        (Atom::Any, _) => Some(example(b)),
        (_, Atom::Any) => Some(example(a)),
        (Atom::Str(x), Atom::Str(y)) => (x == y).then(|| Val::Str(x.clone())),
        (Atom::Bool(x), Atom::Bool(y)) => (x == y).then_some(Val::Bool(*x)),
        (Atom::Iv(alo, aloc, ahi, ahic), Atom::Iv(blo, bloc, bhi, bhic)) => {
            let (mut lo, mut loc) = (*alo, *aloc);
            if let Some(bl) = blo {
                let take = match lo {
                    None => true,
                    Some(l) => bl.cmp_dec(&l) == Greater || (bl.cmp_dec(&l) == Equal && !bloc),
                };
                if take {
                    lo = Some(*bl);
                    loc = *bloc;
                }
            }
            let (mut hi, mut hic) = (*ahi, *ahic);
            if let Some(bh) = bhi {
                let take = match hi {
                    None => true,
                    Some(h) => bh.cmp_dec(&h) == Less || (bh.cmp_dec(&h) == Equal && !bhic),
                };
                if take {
                    hi = Some(*bh);
                    hic = *bhic;
                }
            }
            if let (Some(l), Some(h)) = (lo, hi)
                && (l.cmp_dec(&h) == Greater || (l.cmp_dec(&h) == Equal && !(loc && hic)))
            {
                return None;
            }
            Some(example(&Atom::Iv(lo, loc, hi, hic)))
        }
        _ => None,
    }
}

fn representative(cells: &[Vec<Option<Vec<Atom>>>], col: usize, typ: &str) -> Val {
    for row in cells {
        let Some(atoms) = &row[col] else { continue };
        for a in atoms {
            if !matches!(a, Atom::Any) {
                return example(a);
            }
        }
    }
    match typ {
        "number" => Val::Num(Dec::int(0)),
        "boolean" => Val::Bool(true),
        _ => Val::Null,
    }
}

fn show(v: &Val) -> String {
    match v {
        Val::Null | Val::Any => "null".into(),
        Val::Str(s) => format!("\"{s}\""),
        Val::Bool(b) => if *b { "true" } else { "false" }.into(),
        Val::Num(d) => d.to_text(),
    }
}

fn overlaps(d: &Decision) -> Vec<Check> {
    let cells: Vec<Vec<Option<Vec<Atom>>>> = d
        .rules
        .iter()
        .map(|r| r.inputs.iter().map(|c| atoms(c)).collect())
        .collect();
    let mut out = Vec::new();
    for i in 0..d.rules.len() {
        for j in i + 1..d.rules.len() {
            let mut ex = Vec::new();
            let mut ok = true;
            for col in 0..d.inputs.len() {
                let (Some(Some(a)), Some(Some(b))) = (cells[i].get(col), cells[j].get(col)) else {
                    ok = false;
                    break;
                };
                let mut found = None;
                'outer: for x in a {
                    for y in b {
                        found = meet(x, y);
                        if found.is_some() {
                            break 'outer;
                        }
                    }
                }
                let Some(mut v) = found else {
                    ok = false;
                    break;
                };
                if matches!(v, Val::Any) {
                    v = representative(&cells, col, &d.inputs[col].1);
                }
                ex.push(format!("{}={}", d.inputs[col].0, show(&v)));
            }
            if ok {
                out.push((
                    "D010",
                    d.rules[j].line,
                    vec![
                        ("a", (i + 1).to_string()),
                        ("b", (j + 1).to_string()),
                        ("example", ex.join(", ")),
                    ],
                ));
            }
        }
    }
    out
}

fn el(name: &str, attrs: &[(&str, &str)]) -> XElem {
    let mut e = XElem::new(DMN_NS, name);
    for (k, v) in attrs {
        e.set(k, *v);
    }
    e
}

fn with_text(name: &str, text: &str) -> XElem {
    let mut e = XElem::new(DMN_NS, name);
    e.text = Some(text.to_string());
    e
}

/// DMN 1.3 XML text of one decision table (deployable to Camunda 8).
pub fn export_dmn(d: &Decision, doc: Option<&Document>) -> String {
    let did = d.base.id();
    let name = d
        .base
        .label
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or(did);
    let ns = doc
        .and_then(|doc| doc.attrs.get("namespace"))
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| "http://camunda.org/schema/1.0/dmn".into());
    let mut root = el(
        "definitions",
        &[
            ("id", &format!("{did}-definitions")),
            ("name", name),
            ("namespace", &ns),
            ("exporter", "tok"),
        ],
    );
    root.set_ns(MODELER_NS, "executionPlatform", "Camunda Cloud");
    root.set_ns(MODELER_NS, "executionPlatformVersion", "8.10.0");
    let mut dec = el("decision", &[("id", did), ("name", name)]);
    let mut table = el(
        "decisionTable",
        &[
            ("id", &format!("{did}-table")),
            ("hitPolicy", &d.hit().to_uppercase()),
        ],
    );
    for (k, (expr, typ)) in d.inputs.iter().enumerate() {
        let k = k + 1;
        let mut inp = el(
            "input",
            &[("id", &format!("{did}-input-{k}")), ("label", expr)],
        );
        let mut ie = el(
            "inputExpression",
            &[("id", &format!("{did}-input-{k}-expr")), ("typeRef", typ)],
        );
        ie.push(with_text("text", expr));
        inp.push(ie);
        table.push(inp);
    }
    for (k, (name, typ)) in d.outputs.iter().enumerate() {
        let k = k + 1;
        table.push(el(
            "output",
            &[
                ("id", &format!("{did}-output-{k}")),
                ("label", name),
                ("name", name),
                ("typeRef", typ),
            ],
        ));
    }
    for (r, rule) in d.rules.iter().enumerate() {
        let r = r + 1;
        let mut rel = el("rule", &[("id", &format!("{did}-rule-{r}"))]);
        if let Some(a) = rule.annotation.as_deref().filter(|a| !a.is_empty()) {
            rel.push(with_text("description", a));
        }
        for (k, cell) in rule.inputs.iter().enumerate() {
            let mut e = el(
                "inputEntry",
                &[("id", &format!("{did}-rule-{r}-in-{}", k + 1))],
            );
            e.push(with_text("text", cell));
            rel.push(e);
        }
        for (k, cell) in rule.outputs.iter().enumerate() {
            let mut e = el(
                "outputEntry",
                &[("id", &format!("{did}-rule-{r}-out-{}", k + 1))],
            );
            e.push(with_text("text", if cell == "-" { "" } else { cell }));
            rel.push(e);
        }
        table.push(rel);
    }
    dec.push(table);
    root.push(dec);
    root.indent(0);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}\n",
        root.to_string_root()
    )
}

// ------------------------------------------------------------------ evaluation

fn decision_error(message: String) -> PyError {
    PyError::new("tok.dmn.DecisionError", message)
}

/// Evaluate a decision table against a context of variables (`dmn.evaluate_decision`).
///
/// Returns, as plain values: for the hit policies unique, first, any and priority a dict
/// {output name: value} or `None` when no rule matches; for collect a list of such dicts.
/// A violated hit policy (several rules under unique, different outputs under any) is the
/// error `tok.dmn.DecisionError`; FEEL errors and the exceptions of Python's decimal and
/// datetime arithmetic are returned the same way. `priority` behaves like `first`.
pub fn evaluate_decision(d: &Decision, ctx: &Context) -> Result<PyValue, PyError> {
    let mut values = Vec::with_capacity(d.inputs.len());
    for (expr, _) in &d.inputs {
        let node = feel::parse_expr(expr).map_err(feel_error)?;
        values.push(fe::evaluate(&node, ctx)?);
    }
    let mut matched: Vec<(usize, Vec<(String, PyValue)>)> = Vec::new();
    for (idx, rule) in d.rules.iter().enumerate() {
        let mut ok = true;
        for (cell, v) in rule.inputs.iter().zip(&values) {
            let unary = feel::parse_unary_tests(cell).map_err(feel_error)?;
            if fe::test(&unary, v, ctx)? != Some(true) {
                ok = false;
                break;
            }
        }
        if ok {
            let mut out: Vec<(String, PyValue)> = Vec::new();
            for ((name, _), cell) in d.outputs.iter().zip(&rule.outputs) {
                let value = if strip(cell) == "-" {
                    PyValue::None
                } else {
                    fe::to_python(&fe::eval_text(cell, ctx)?)?
                };
                match out.iter_mut().find(|(k, _)| k == name) {
                    Some(slot) => slot.1 = value,
                    None => out.push((name.clone(), value)),
                }
            }
            matched.push((idx + 1, out));
        }
    }
    let hit = d.hit();
    if hit == "collect" {
        return Ok(PyValue::List(
            matched.into_iter().map(|(_, o)| PyValue::Dict(o)).collect(),
        ));
    }
    if matched.is_empty() {
        return Ok(PyValue::None);
    }
    if hit == "unique" && matched.len() > 1 {
        let ids: Vec<String> = matched.iter().map(|(i, _)| i.to_string()).collect();
        return Err(decision_error(format!(
            "rules {} all match (hit policy unique)",
            ids.join(", ")
        )));
    }
    if hit == "any" {
        let first = PyValue::Dict(matched[0].1.clone());
        for (i, o) in &matched[1..] {
            if !PyValue::Dict(o.clone()).py_eq(&first) {
                return Err(decision_error(format!(
                    "rules {} and {i} match with different outputs (hit policy any)",
                    matched[0].0
                )));
            }
        }
    }
    Ok(PyValue::Dict(matched.swap_remove(0).1))
}

/// For a decision with one output, unwrap {name: value} to the value (what Camunda stores
/// in `resultVariable`); lists of such dicts become lists of values (`dmn.single_result`).
pub fn single_result(d: &Decision, result: PyValue) -> PyValue {
    if d.outputs.len() != 1 {
        return result;
    }
    let name = &d.outputs[0].0;
    let unwrap = |pairs: Vec<(String, PyValue)>| {
        pairs
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
            .unwrap_or(PyValue::None)
    };
    match result {
        PyValue::Dict(pairs) => unwrap(pairs),
        PyValue::List(items) => PyValue::List(
            items
                .into_iter()
                .map(|r| match r {
                    PyValue::Dict(pairs) => unwrap(pairs),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}
