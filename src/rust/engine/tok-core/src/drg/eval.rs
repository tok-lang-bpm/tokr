//! Evaluation of decisions of language 0.7 (DMN 1.5 §8, §10.3.2.9, §10.4): a decision
//! with the decisions, input data and business knowledge models it requires.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{Kind, bindings, kind_of, names, params, requires, type_of, types, var_name};
use crate::feel::FeelError;
use crate::feel3::eval::temporal_literal;
use crate::feel3::parse::{Expr, Names, Tests, Type, parse_expression, parse_tests};
use crate::feel3::{Body, Env, Eval, Func, Map, Param, Types, Value, compare, equal};
use crate::model::{Decision, Document};
use crate::pyutil::strip;

/// What a model needs while it evaluates: names for the parser, parsed texts, the names
/// of the business knowledge models and the problems found (violated hit policies).
pub struct Shared {
    pub names: Names,
    exprs: RefCell<HashMap<String, Result<Rc<Expr>, FeelError>>>,
    tests: RefCell<HashMap<String, Result<Rc<Tests>, FeelError>>>,
    bkm_names: HashMap<String, String>,
    /// Allowed values of the DMN types by identifier and name (the output values of a
    /// table whose output column has such a type).
    allowed: HashMap<String, String>,
    pub problems: RefCell<Vec<String>>,
}

impl Shared {
    fn expr(&self, text: &str) -> Option<Rc<Expr>> {
        if let Some(r) = self.exprs.borrow().get(text) {
            return r.clone().ok();
        }
        let r = parse_expression(text, &self.names).map(Rc::new);
        self.exprs.borrow_mut().insert(text.to_string(), r.clone());
        r.ok()
    }
    fn tests(&self, text: &str) -> Option<Rc<Tests>> {
        if let Some(r) = self.tests.borrow().get(text) {
            return r.clone().ok();
        }
        let r = parse_tests(text, &self.names).map(Rc::new);
        self.tests.borrow_mut().insert(text.to_string(), r.clone());
        r.ok()
    }
    fn problem(&self, p: String) {
        self.problems.borrow_mut().push(p);
    }
}

/// A document prepared for evaluation.
pub struct Model<'d> {
    pub doc: &'d Document,
    pub shared: Rc<Shared>,
    pub types: Types,
}

/// The body of a business knowledge model or of a function definition: a box.
struct BoxBody {
    logic: Decision,
    shared: Rc<Shared>,
}

impl Body for BoxBody {
    fn call(&self, ev: &Eval, env: &Rc<Env>) -> Value {
        eval_logic(ev, &self.shared, &self.logic, env, true)
    }
}

fn text_value(ev: &Eval, sh: &Shared, text: &str, env: &Rc<Env>) -> Value {
    match sh.expr(text) {
        Some(e) => ev.eval(&e, env),
        None => Value::Null,
    }
}

/// The value of the logic of a box (a decision, a business knowledge model, an entry, a
/// result or an element). `in_function`: the parameters of the box are already bound.
pub fn eval_logic(
    ev: &Eval,
    sh: &Rc<Shared>,
    b: &Decision,
    env: &Rc<Env>,
    in_function: bool,
) -> Value {
    if !in_function && b.base.kind != "bkm" && b.base.attrs.contains_key("params") {
        return function_value(sh, b, env);
    }
    let a = &b.base.attrs;
    if let Some(e) = a.get("expr") {
        return text_value(ev, sh, e, env);
    }
    if let Some(bkm) = a.get("invoke") {
        let fname = sh
            .bkm_names
            .get(bkm)
            .cloned()
            .unwrap_or_else(|| bkm.clone());
        let Some(Value::Func(f)) = env.lookup(&fname).cloned() else {
            return Value::Null;
        };
        let pairs = a
            .get("with")
            .map(|w| bindings(w).unwrap_or_default())
            .unwrap_or_default();
        let named: Vec<(String, Value)> = pairs
            .into_iter()
            .map(|(p, e)| (p, text_value(ev, sh, &e, env)))
            .collect();
        return call_named(ev, &f, named);
    }
    if b.has_table() {
        if b.relation {
            return relation(ev, sh, b, env);
        }
        return table(ev, sh, b, env);
    }
    if !b.lines.is_empty() {
        if b.lines.iter().all(|l| l.base.kind == "element") {
            return Value::list(
                b.lines
                    .iter()
                    .map(|l| typed(ev, l, eval_logic(ev, sh, l, env, false)))
                    .collect(),
            );
        }
        let mut scope = env.clone();
        let mut m = Map::new();
        for l in &b.lines {
            let v = typed(ev, l, eval_logic(ev, sh, l, &scope, false));
            if l.base.kind == "result" {
                return v;
            }
            let name = super::normalize(l.base.id());
            m.insert(name.clone(), v.clone());
            let mut vars = Map::new();
            vars.insert(name, v);
            scope = Env::child(&scope, vars);
        }
        return Value::context(m);
    }
    Value::Null
}

fn typed(ev: &Eval, l: &Decision, v: Value) -> Value {
    match l.base.attr("type") {
        Some(t) if !l.base.attrs.contains_key("params") => {
            ev.coerce(v, &Type::from_ref(t)).unwrap_or(Value::Null)
        }
        _ => v,
    }
}

fn call_named(ev: &Eval, f: &Func, named: Vec<(String, Value)>) -> Value {
    let params: Vec<&Param> = match f {
        Func::Lambda { params, .. } | Func::Boxed { params, .. } => params.iter().collect(),
        Func::Builtin(_) => return Value::Null,
    };
    let args: Vec<Value> = params
        .iter()
        .map(|p| {
            named
                .iter()
                .find(|(n, _)| *n == p.name)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Null)
        })
        .collect();
    ev.call(f, args)
}

fn param_list(text: Option<&str>) -> Vec<Param> {
    params(text.unwrap_or(""))
        .unwrap_or_default()
        .into_iter()
        .map(|(name, t)| Param {
            name,
            ty: t.map(|t| Type::from_ref(&t)),
        })
        .collect()
}

fn function_value(sh: &Rc<Shared>, b: &Decision, env: &Rc<Env>) -> Value {
    Value::Func(Rc::new(Func::Boxed {
        params: param_list(b.base.attr("params")),
        body: Rc::new(BoxBody {
            logic: b.clone(),
            shared: sh.clone(),
        }),
        env: env.clone(),
        result: b.base.attr("type").map(Type::from_ref),
    }))
}

fn relation(ev: &Eval, sh: &Shared, b: &Decision, env: &Rc<Env>) -> Value {
    let mut rows = Vec::new();
    for r in &b.rules {
        let mut m = Map::new();
        for ((name, typ), cell) in b.inputs.iter().zip(&r.inputs) {
            let v = text_value(ev, sh, cell, env);
            let v = ev.coerce(v, &Type::from_ref(typ)).unwrap_or(Value::Null);
            m.insert(super::normalize(name), v);
        }
        rows.push(Value::context(m));
    }
    Value::list(rows)
}

fn priority(values: &[Value], outs: &[Value]) -> Vec<usize> {
    outs.iter()
        .map(|o| {
            values
                .iter()
                .position(|v| equal(v, o) == Some(true))
                .unwrap_or(usize::MAX)
        })
        .collect()
}

fn value_lists(ev: &Eval, sh: &Shared, b: &Decision, env: &Rc<Env>) -> Vec<Vec<Value>> {
    b.output_values
        .iter()
        .zip(&b.outputs)
        .map(
            |(v, (_, typ))| match v.as_ref().or_else(|| sh.allowed.get(typ)) {
                Some(text) => {
                    // output values are unary tests; their values are the order of priority
                    match sh.tests(text).as_deref() {
                        Some(Tests::Some(ts)) => ts.iter().map(|t| ev.eval(t, env)).collect(),
                        _ => Vec::new(),
                    }
                }
                None => Vec::new(),
            },
        )
        .collect()
}

/// A decision table (DMN 1.5 §8.3).
fn table(ev: &Eval, sh: &Shared, b: &Decision, env: &Rc<Env>) -> Value {
    let id = b.base.id().to_string();
    let inputs: Vec<Value> = b
        .inputs
        .iter()
        .map(|(e, t)| {
            let v = text_value(ev, sh, e, env);
            if t == "any" || t == "Any" {
                v
            } else {
                ev.coerce(v.clone(), &Type::from_ref(t)).unwrap_or(v)
            }
        })
        .collect();
    let mut matched: Vec<(usize, Vec<Value>)> = Vec::new();
    for (k, rule) in b.rules.iter().enumerate() {
        let mut ok = true;
        for (cell, v) in rule.inputs.iter().zip(&inputs) {
            let tests = match sh.tests(cell) {
                Some(t) => t,
                None => {
                    ok = false;
                    break;
                }
            };
            if !ev.test(&tests, v, env) {
                ok = false;
                break;
            }
        }
        if ok {
            let outs: Vec<Value> = rule
                .outputs
                .iter()
                .zip(&b.outputs)
                .map(|(cell, (_, t))| {
                    if strip(cell) == "-" || strip(cell).is_empty() {
                        Value::Null
                    } else {
                        let v = text_value(ev, sh, cell, env);
                        ev.coerce(v.clone(), &Type::from_ref(t))
                            .unwrap_or(Value::Null)
                    }
                })
                .collect();
            matched.push((k + 1, outs));
        }
    }
    let shape = |outs: Vec<Value>| -> Value {
        if b.outputs.len() == 1 {
            outs.into_iter().next().unwrap_or(Value::Null)
        } else {
            let mut m = Map::new();
            for ((name, _), v) in b.outputs.iter().zip(outs) {
                m.insert(super::normalize(name), v);
            }
            Value::context(m)
        }
    };
    let hit = b.hit();
    if matched.is_empty()
        && hit != "unique"
        && let Some(d) = defaults(ev, sh, b, env)
        && ["collect", "rule-order", "output-order"].contains(&hit)
    {
        if b.base.attrs.contains_key("aggregation") {
            return d.into_iter().next().unwrap_or(Value::Null);
        }
        return Value::list(vec![shape(d)]);
    }
    match hit {
        "collect" => {
            if let Some(agg) = b.base.attr("aggregation") {
                let vals: Vec<Value> = matched
                    .into_iter()
                    .map(|(_, o)| o.into_iter().next().unwrap_or(Value::Null))
                    .filter(|v| !v.is_null())
                    .collect();
                return aggregate(agg, &vals);
            }
            Value::list(matched.into_iter().map(|(_, o)| shape(o)).collect())
        }
        "rule-order" => Value::list(matched.into_iter().map(|(_, o)| shape(o)).collect()),
        "output-order" => {
            let lists = value_lists(ev, sh, b, env);
            let mut ranked: Vec<(Vec<usize>, usize, Vec<Value>)> = matched
                .into_iter()
                .map(|(k, o)| {
                    let p = lists
                        .iter()
                        .zip(&o)
                        .map(|(l, v)| priority(l, std::slice::from_ref(v))[0])
                        .collect();
                    (p, k, o)
                })
                .collect();
            ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            Value::list(ranked.into_iter().map(|(_, _, o)| shape(o)).collect())
        }
        _ => {
            if matched.is_empty() {
                return defaults(ev, sh, b, env).map(shape).unwrap_or(Value::Null);
            }
            match hit {
                "unique" if matched.len() > 1 => {
                    let ids: Vec<String> = matched.iter().map(|(i, _)| i.to_string()).collect();
                    sh.problem(format!(
                        "decision '{id}': rules {} all match (hit policy unique)",
                        ids.join(", ")
                    ));
                    Value::Null
                }
                "any" => {
                    let first = &matched[0].1;
                    for (i, o) in &matched[1..] {
                        let same = o.iter().zip(first).all(|(x, y)| equal(x, y) == Some(true));
                        if !same {
                            sh.problem(format!(
                                "decision '{id}': rules {} and {i} match with different outputs (hit policy any)",
                                matched[0].0
                            ));
                            return Value::Null;
                        }
                    }
                    shape(matched.swap_remove(0).1)
                }
                "priority" => {
                    let lists = value_lists(ev, sh, b, env);
                    let best = matched
                        .into_iter()
                        .map(|(k, o)| {
                            let p: Vec<usize> = lists
                                .iter()
                                .zip(&o)
                                .map(|(l, v)| priority(l, std::slice::from_ref(v))[0])
                                .collect();
                            (p, k, o)
                        })
                        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                    best.map(|(_, _, o)| shape(o)).unwrap_or(Value::Null)
                }
                _ => shape(matched.swap_remove(0).1),
            }
        }
    }
}

/// The default output entries of a table (`default=`): the value of its only output, or
/// `output = FEEL; …` for several outputs.
fn defaults(ev: &Eval, sh: &Shared, b: &Decision, env: &Rc<Env>) -> Option<Vec<Value>> {
    let text = b.base.attr("default")?;
    if b.outputs.len() == 1 {
        return Some(vec![text_value(ev, sh, text, env)]);
    }
    let pairs = bindings(text).ok()?;
    Some(
        b.outputs
            .iter()
            .map(|(name, _)| {
                pairs
                    .iter()
                    .find(|(n, _)| n == &super::normalize(name))
                    .map(|(_, e)| text_value(ev, sh, e, env))
                    .unwrap_or(Value::Null)
            })
            .collect(),
    )
}

fn aggregate(agg: &str, vals: &[Value]) -> Value {
    match agg {
        "count" => Value::int(vals.len() as i64),
        "sum" => {
            if vals.is_empty() {
                return Value::Null;
            }
            let mut t = crate::feel3::Num::zero();
            for v in vals {
                let Value::Num(n) = v else {
                    return Value::Null;
                };
                match t.add(n) {
                    Some(x) => t = x,
                    None => return Value::Null,
                }
            }
            Value::Num(t)
        }
        "min" | "max" => {
            let mut best: Option<Value> = None;
            for v in vals {
                best = Some(match best {
                    None => v.clone(),
                    Some(b) => match compare(v, &b) {
                        Some(o)
                            if (agg == "min") == (o == std::cmp::Ordering::Less)
                                && o != std::cmp::Ordering::Equal =>
                        {
                            v.clone()
                        }
                        Some(_) => b,
                        None => return Value::Null,
                    },
                });
            }
            best.unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

impl<'d> Model<'d> {
    pub fn new(doc: &'d Document) -> Model<'d> {
        let bkm_names = doc
            .bkms
            .values()
            .map(|b| (b.base.id().to_string(), var_name(&b.base)))
            .collect();
        Model {
            doc,
            shared: Rc::new(Shared {
                names: names(doc),
                exprs: RefCell::new(HashMap::new()),
                tests: RefCell::new(HashMap::new()),
                bkm_names,
                allowed: doc
                    .items
                    .values()
                    .filter_map(|it| it.attr("allowed").map(|a| (it, a.to_string())))
                    .flat_map(|(it, a)| [(it.id().to_string(), a.clone()), (var_name(it), a)])
                    .collect(),
                problems: RefCell::new(Vec::new()),
            }),
            types: types(doc),
        }
    }

    /// The function value of a business knowledge model, with the models it requires.
    fn bkm_function(&self, ev: &Eval, id: &str, depth: usize) -> Value {
        let Some(b) = self.doc.bkms.get(id) else {
            return Value::Null;
        };
        let env =
            Env::root(self.required(ev, &b.base, &Map::new(), &mut HashMap::new(), depth + 1));
        Value::Func(Rc::new(Func::Boxed {
            params: param_list(b.base.attr("params"))
                .into_iter()
                .map(|p| Param {
                    ty: p.ty.map(|t| self.resolve(t)),
                    ..p
                })
                .collect(),
            body: Rc::new(BoxBody {
                logic: b.clone(),
                shared: self.shared.clone(),
            }),
            env,
            result: b.base.attr("type").map(|t| type_of(self.doc, t)),
        }))
    }

    fn resolve(&self, t: Type) -> Type {
        match t {
            Type::Named(n) => match self.doc.items.get(&n) {
                Some(item) => Type::Named(var_name(item)),
                None => Type::Named(n),
            },
            other => other,
        }
    }

    /// The variables a decision or business knowledge model sees: its requirements.
    fn required(
        &self,
        ev: &Eval,
        b: &crate::model::Base,
        inputs: &Map,
        memo: &mut HashMap<String, Value>,
        depth: usize,
    ) -> Map {
        let mut vars = Map::new();
        if depth > 64 {
            return vars;
        }
        let mut reqs = requires(b);
        if let Some(d) = self
            .doc
            .decisions
            .get(b.id())
            .or_else(|| self.doc.bkms.get(b.id()))
        {
            super::invoked(d, &mut reqs);
        }
        for r in reqs {
            match kind_of(self.doc, &r) {
                Some(Kind::InputData) => {
                    let Some(ib) = self.doc.inputdata.get(&r) else {
                        continue;
                    };
                    let name = var_name(ib);
                    let v = inputs.get(&name).cloned().unwrap_or(Value::Null);
                    let v = match ib.attr("type") {
                        Some(t) => {
                            let ty = type_of(self.doc, t);
                            let v = match (&v, &ty) {
                                (
                                    Value::Str(s),
                                    Type::Date
                                    | Type::Time
                                    | Type::DateTime
                                    | Type::DtDuration
                                    | Type::YmDuration,
                                ) => temporal_literal(s),
                                _ => v,
                            };
                            ev.coerce(v, &ty).unwrap_or(Value::Null)
                        }
                        None => v,
                    };
                    vars.insert(name, v);
                }
                Some(Kind::Decision) => {
                    let Some(db) = self.doc.decisions.get(&r) else {
                        continue;
                    };
                    let v = self.decision_inner(ev, &r, inputs, memo, depth + 1);
                    vars.insert(var_name(&db.base), v);
                }
                Some(Kind::Bkm) => {
                    let Some(bb) = self.doc.bkms.get(&r) else {
                        continue;
                    };
                    vars.insert(var_name(&bb.base), self.bkm_function(ev, &r, depth));
                }
                _ => {}
            }
        }
        vars
    }

    fn decision_inner(
        &self,
        ev: &Eval,
        id: &str,
        inputs: &Map,
        memo: &mut HashMap<String, Value>,
        depth: usize,
    ) -> Value {
        if let Some(v) = memo.get(id) {
            return v.clone();
        }
        let Some(d) = self.doc.decisions.get(id) else {
            return Value::Null;
        };
        let vars = self.required(ev, &d.base, inputs, memo, depth);
        let env = Env::root(vars);
        let v = eval_logic(ev, &self.shared, d, &env, false);
        let v = match d.base.attr("type") {
            Some(t) => ev.coerce(v, &type_of(self.doc, t)).unwrap_or(Value::Null),
            None => v,
        };
        memo.insert(id.to_string(), v.clone());
        v
    }

    /// Evaluate a decision for the given input data (by FEEL name). Returns the value and
    /// the violated hit policies.
    pub fn decision(&self, id: &str, inputs: &Map) -> (Value, Vec<String>) {
        let ev = Eval::new(&self.types);
        self.shared.problems.borrow_mut().clear();
        let v = self.decision_inner(&ev, id, inputs, &mut HashMap::new(), 0);
        (v, self.shared.problems.borrow().clone())
    }
}

/// Evaluate one decision of a 0.7 document.
pub fn evaluate(doc: &Document, id: &str, inputs: &Map) -> (Value, Vec<String>) {
    Model::new(doc).decision(id, inputs)
}
